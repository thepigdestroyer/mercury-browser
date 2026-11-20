// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The viewer model that ties layout, cache, selection and find together for one open document.
//!
//! [`ViewerModel`] decides **what the viewer draws** (which pages are where, which tiles to show)
//! and **what to ask the helper process for next** ([`ViewerModel::plan`]). It has no I/O, no
//! clock, no threads and no Slint: the shell (item 4, `apps/mercury-desktop/src/pdf/`) feeds it
//! the viewport, the scroll position, the pointer and the helper's answers, and reads back what
//! to draw. It is tested without a window.
//!
//! # The loop the shell runs
//!
//! 1. After **any** change (viewport, zoom, fit, rotation, scroll, go to page, an answer from the
//!    helper, a press), call [`ViewerModel::plan`] and carry it out, **cancels first**: cancel the
//!    helper requests for `plan.cancel` (a request is named by its [`TileRequest`], so the shell
//!    keeps a map from request to the client's `RequestId`), then send `plan.requests` in the
//!    order given with the priority given, then `request_page_text` for `plan.page_text` and
//!    `request_links` for `plan.links`. The model marks everything it returned as pending and does
//!    not return it again.
//! 2. When a tile arrives call [`ViewerModel::tile_arrived`] (or [`ViewerModel::tile_failed`]
//!    for an error); when text or links arrive call [`ViewerModel::set_page_text`] /
//!    [`ViewerModel::set_links`] (**also on an error, with empty text or no links**, or the page
//!    stays pending); when a find answer arrives call [`ViewerModel::find_hits`]. An answer for
//!    something the model no longer wants is harmless. Then redraw and call `plan` again.
//! 3. To redraw, read [`ViewerModel::visible_tiles`] (draw in the order returned: placeholders
//!    first, exact tiles over them), [`ViewerModel::page_rects`] (the white paper under them),
//!    [`ViewerModel::selection_rects`] and [`ViewerModel::find_rects`] (highlights over them) and
//!    [`ViewerModel::content_size`] (the size of the scrolling content).
//! 4. After a helper restart ("Try again") call [`ViewerModel::forget_pending`] so nothing waits
//!    for an answer that will not come.
//!
//! # Coordinates
//!
//! **Content space** is logical pixels, origin at the top-left of the scrolling content, which is
//! what a `Flickable`'s viewport holds: [`ViewerModel::content_size`] is its size. Pointer
//! positions, links, highlights and placed tiles are all in content space (so a `TouchArea`
//! inside the `Flickable` gives the model what it needs, with no scroll arithmetic by the shell).
//! The scroll position is the content-space position of the viewport's top-left corner. Positions
//! are `f32` at the interface and `f64` inside; a document longer than about 16 million logical
//! pixels (20,000 pages at 100 %) loses sub-pixel precision at the far end at the interface only.
//! **Page space** is the engine's: points, y down, the page's own `/Rotate` applied; the view
//! rotation is applied by the model.
//!
//! # Public surface
//!
//! Types: [`Viewport`], [`Fit`] (re-exported from `layout`), [`PlacedTile`], [`Plan`], and the
//! priorities [`PRIORITY_VISIBLE`], [`PRIORITY_NEAR`] and [`PRIORITY_PLACEHOLDER`] that `plan`
//! uses (higher is sooner; the client queues by them). `find::FindProgress` is what
//! [`ViewerModel::find_status`] returns.
//!
//! Construction and view state:
//! - `new(info)` the document; `title()` its title; `page_count()`.
//! - `set_viewport(Viewport)`; `viewport()`. Width, height and the device pixel ratio. A new
//!   width (or ratio) re-lays out the pages; the scroll position is kept, except that a change of
//!   scale keeps the point at the viewport's centre where it was.
//! - `set_zoom_percent(u16)` (30 to 500, 100 = the fit); `zoom_percent()`; `set_fit(Fit)`;
//!   `fit()`; `set_rotation(Rotation)`; `rotation()`. `set_fit` changes only what 100 % means:
//!   for a "Fit width" or "Fit page" button the shell also calls `set_zoom_percent(100)` (the tab
//!   zoom is the zoom percent, decision 9). `Fit::Page` fits the page that was current when the
//!   fit was set, or when the viewport or rotation last changed, or when `go_to_page` last ran,
//!   never one that merely scrolled into view (scrolling must not zoom).
//! - `scroll_x()`, `scroll_y()`, `set_scroll_y(f32)`, `set_scroll_x(f32)`, `set_scroll(x, y)`,
//!   `scroll_by(dy)`: positions are clamped to the content. `content_size() -> (width, height)`.
//! - `go_to_page(page, y_points)`: scrolls so that the page (or the point `y_points` on it, in
//!   page space) is 8 px below the top of the viewport, and makes it the current page.
//!   `current_page()`: the page the page indicator shows: the first page of which at least half
//!   shows (half of its height, or half of the viewport's when it is taller than that), else the
//!   page with the most visible height (the earlier on a tie). `go_to_page` sets it directly (so
//!   "go to page 5" reads 5 even at 30 % zoom, where 6, 7 and 8 show too) and the next scroll
//!   computes it again.
//!
//! Drawing:
//! - `visible_tiles() -> Vec<PlacedTile>`: for each page that shows in the viewport, each tile of
//!   the grid that shows is the exact tile if the cache has it; if any of them is missing, the
//!   page's low-resolution placeholder is drawn under them (one `PlacedTile` covering the whole
//!   page, `is_placeholder`), and with neither the paper shows.
//! - `page_rects() -> Vec<(page, x, y, width, height)>`: the pages that show, as paper.
//!   `page_rect(page)` is one page's rectangle (any page).
//! - `selection_rects()` and `find_rects()`: `(x, y, width, height)` (and, for find, whether it
//!   is the current hit) of the highlights that intersect the viewport, merged per line.
//!
//! Diagnostics: `cache_usage() -> CacheUsage` (tiles, placeholders and text held, in bytes or
//! characters).
//!
//! Requests: `plan() -> Plan`, `tile_arrived(&TileRequest, Tile)`, `tile_failed(&TileRequest)`,
//! `set_page_text(page, PageText)`, `set_links(page, Vec<Link>)`, `forget_pending()`. A tile is a
//! rectangle of the 512 x 512 device-pixel grid of a page at the zoom's device scale, a whole
//! number of thousandths of a pixel per point, rounded down (so the cache key and the request
//! agree, and a page fitted to the width is never a pixel wider than the window). A page's size
//! in pixels is the engine's own `PageSize::pixel_size`, and the pages, their tiles and the
//! margins all sit on whole device pixels. The request order is: tiles that show, nearest the
//! centre of the viewport first; then tiles in
//! the margin of one viewport height above and below, nearest first; then page placeholders,
//! nearest first. Nothing is requested twice; whatever is no longer wanted (scrolled far away, or
//! at an old zoom or rotation) is cancelled; a tile that failed is not asked for again until the
//! zoom changes. Page text is asked for only for the pages the selection needs (the page under a
//! press, the page under the pointer, and the pages between, at most 256 from the first, the
//! copy limit) and links only for pages that show. Memory is bounded by the constants in
//! `cache::budget`: tiles 96 MiB, placeholders 16 MiB, text 2 million characters, never evicting
//! what is on screen.
//!
//! Pointer: `pointer_down(x, y, clicks)`, `pointer_drag(x, y)`, `pointer_up()`, all in content
//! space; `clicks` is the number of clicks in a row (1 caret, 2 word, 3 line). A press whose
//! page's text has not arrived is resolved when it does. `clear_selection()`,
//! `has_selection()`, `selected_text() -> Option<String>` (pages joined by `\n`, `\r\n` turned
//! into `\n`, cut at 1 MiB), `selection_complete()` (false while text of a page of the selection
//! is still on its way). `link_at(x, y) -> Option<LinkTarget>`: the link under a point of a page
//! whose links have arrived, the smallest if several contain it; the shell navigates (a
//! `LinkTarget::Page` goes to `go_to_page`, a `Uri` is an ordinary navigation if `http(s)`).
//!
//! Find: `find_start(query, options) -> u64` replaces any find and returns the model's id for it
//! (an empty query is a finished find with no hits: send nothing to the helper);
//! `find_bind(id)` tells the model the id the *client* returned for the search it then started
//! (`client.find(query, options, model.current_page())`), which is the id its `Found` events
//! carry; `find_hits(find, page, hits, finished) -> bool` (false: stale, ignored);
//! `find_next()` and `find_previous()` (wrap around; true if there is a hit) scroll the hit into
//! view; so does the first hit that arrives; `find_clear()`; `find_status() -> FindProgress`
//! (current counted from 1, total, finished: "searching" while not finished);
//! `find_query()`, `find_options()`, `find_is_active()`.

use std::collections::HashSet;
use std::sync::Arc;

use crate::cache::{
    ByteLru, FailedTiles, PageStore, PendingTiles, PlaceholderKey, TileKey, budget,
};
use crate::find::{FindProgress, FindState, HitsOutcome};
pub use crate::layout::{Fit, Viewport};
use crate::layout::{
    Geometry, Layout, MARGIN, PxRect, TILE_SIDE, clamp_percent, device_milli, placeholder_request,
};
use crate::selection::{IndexedText, PagePoint, Selection};
use crate::types::{
    DocumentInfo, FindOptions, Link, LinkTarget, PageText, Rect, Rotation, SearchHit, Tile,
    TileRequest,
};

/// The priority of a tile that shows in the viewport.
pub const PRIORITY_VISIBLE: u8 = 200;
/// The priority of a tile in the margin of one viewport above and below.
pub const PRIORITY_NEAR: u8 = 100;
/// The priority of a page placeholder.
pub const PRIORITY_PLACEHOLDER: u8 = 10;

/// A tile (or a page placeholder) to draw: where, and its pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedTile {
    /// Content space, logical pixels.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub page: u32,
    pub pixels: Arc<Tile>,
    /// True for a low-resolution picture of the whole page, to be drawn under the exact tiles.
    pub is_placeholder: bool,
}

/// How much memory the caches hold (for diagnostics and tests; the limits are in
/// [`budget`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub tiles: usize,
    pub tile_bytes: usize,
    pub placeholders: usize,
    pub placeholder_bytes: usize,
    pub text_chars: usize,
}

/// What to ask the helper process for next.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// Tile requests to send, in the order to send them, each with its priority.
    pub requests: Vec<(TileRequest, u8)>,
    /// Requests sent earlier that are no longer wanted: cancel them before sending `requests`.
    pub cancel: Vec<TileRequest>,
    /// Pages whose text to ask for.
    pub page_text: Vec<u32>,
    /// Pages whose links to ask for.
    pub links: Vec<u32>,
}

impl Plan {
    /// Whether there is nothing to do.
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
            && self.cancel.is_empty()
            && self.page_text.is_empty()
            && self.links.is_empty()
    }
}

/// The point at the viewport's centre, kept across a change of scale: a page and a point of its
/// page space (which may lie off the page, in a gap).
struct Anchor {
    page: usize,
    x: f32,
    y: f32,
}

struct WantedTile {
    key: TileKey,
    request: TileRequest,
    visible: bool,
    distance: f64,
}

pub struct ViewerModel {
    title: Option<String>,
    layout: Layout,
    viewport: Viewport,
    percent: u16,
    fit: Fit,
    fit_page: u32,
    rotation: Rotation,
    scroll_x: f64,
    scroll_y: f64,
    current: u32,
    tiles: ByteLru<TileKey>,
    placeholders: ByteLru<PlaceholderKey>,
    pending: PendingTiles,
    failed: FailedTiles,
    texts: PageStore<IndexedText>,
    text_pending: HashSet<u32>,
    links: PageStore<Vec<Link>>,
    links_pending: HashSet<u32>,
    selection: Selection,
    find: FindState,
}

fn tile_bytes(width: u32, height: u32) -> usize {
    usize::try_from(u64::from(width) * u64::from(height) * 4).unwrap_or(usize::MAX)
}

/// Whether the tile of `key` has pixels in the viewport at the layout's current zoom.
fn tile_on_screen(layout: &Layout, viewport: &PxRect, key: &TileKey) -> bool {
    if key.scale_milli != layout.scale_milli() || key.rotation != layout.rotation() {
        return false;
    }
    let page = key.page as usize;
    let (Some((x, y, width, height)), Some((origin_x, origin_y))) = (
        layout.tile_rect(page, key.col, key.row),
        layout.page_origin_px(page),
    ) else {
        return false;
    };
    let (x, y) = (origin_x + i64::from(x), origin_y + i64::from(y));
    PxRect {
        x0: x,
        y0: y,
        x1: x + i64::from(width),
        y1: y + i64::from(height),
    }
    .intersects(viewport)
}

fn placeholder_on_screen(layout: &Layout, viewport: &PxRect, key: &PlaceholderKey) -> bool {
    key.1 == layout.rotation()
        && layout
            .page_rect_px(key.0 as usize)
            .is_some_and(|rect| rect.intersects(viewport))
}

impl ViewerModel {
    /// A model of the document, scrolled to the top at 100 % of fit width in an 800 x 600
    /// viewport until [`Self::set_viewport`] says otherwise.
    pub fn new(info: DocumentInfo) -> Self {
        let mut model = Self {
            title: info.title,
            layout: Layout::new(info.pages),
            viewport: Viewport::default(),
            percent: 100,
            fit: Fit::Width,
            fit_page: 0,
            rotation: Rotation::Deg0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            current: 0,
            tiles: ByteLru::new(budget::TILE_BYTES),
            placeholders: ByteLru::new(budget::PLACEHOLDER_BYTES),
            pending: PendingTiles::default(),
            failed: FailedTiles::default(),
            texts: PageStore::new(budget::TEXT_CHARS),
            text_pending: HashSet::new(),
            links: PageStore::new(budget::LINK_ENTRIES),
            links_pending: HashSet::new(),
            selection: Selection::new(),
            find: FindState::new(),
        };
        model.relayout(None);
        model
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn page_count(&self) -> u32 {
        self.layout.page_count()
    }

    // ----- view state ---------------------------------------------------------------------

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        let viewport = viewport.sanitized();
        if viewport == self.viewport {
            return;
        }
        let anchor = self.centre_anchor();
        self.viewport = viewport;
        self.fit_page = self.current;
        self.relayout(anchor);
    }

    pub fn zoom_percent(&self) -> u16 {
        self.percent
    }

    /// Sets the zoom, in percent of the fit, clamped to 30..=500; the point at the viewport's
    /// centre stays where it is.
    pub fn set_zoom_percent(&mut self, percent: u16) {
        let anchor = self.centre_anchor();
        self.percent = clamp_percent(percent);
        self.relayout(anchor);
    }

    pub fn fit(&self) -> Fit {
        self.fit
    }

    pub fn set_fit(&mut self, fit: Fit) {
        let anchor = self.centre_anchor();
        self.fit = fit;
        self.fit_page = self.current;
        self.relayout(anchor);
    }

    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    pub fn set_rotation(&mut self, rotation: Rotation) {
        let anchor = self.centre_anchor();
        self.rotation = rotation;
        self.fit_page = self.current;
        self.relayout(anchor);
    }

    /// The device scale of the zoom, in device pixels per point: a whole number of thousandths.
    pub fn device_scale(&self) -> f32 {
        self.layout.device_scale()
    }

    pub fn scroll_x(&self) -> f32 {
        self.scroll_x as f32
    }

    pub fn scroll_y(&self) -> f32 {
        self.scroll_y as f32
    }

    pub fn set_scroll_y(&mut self, y: f32) {
        self.set_scroll(self.scroll_x as f32, y);
    }

    pub fn set_scroll_x(&mut self, x: f32) {
        self.set_scroll(x, self.scroll_y as f32);
    }

    /// Scrolls to a position of content space (clamped to the content).
    pub fn set_scroll(&mut self, x: f32, y: f32) {
        if x.is_finite() {
            self.scroll_x = f64::from(x);
        }
        if y.is_finite() {
            self.scroll_y = f64::from(y);
        }
        self.clamp_scroll();
        self.update_current();
    }

    pub fn scroll_by(&mut self, dy: f32) {
        if dy.is_finite() {
            self.scroll_y += f64::from(dy);
            self.clamp_scroll();
            self.update_current();
        }
    }

    /// The size of the scrolling content in logical pixels.
    pub fn content_size(&self) -> (f32, f32) {
        self.layout.content_size()
    }

    pub fn current_page(&self) -> u32 {
        self.current
    }

    /// Scrolls to a page, or to the point `y_points` on it (page space), 8 px below the top of
    /// the viewport, and makes it the current page. With `Fit::Page` the zoom refits to that page.
    pub fn go_to_page(&mut self, page: u32, y_points: Option<f32>) {
        let count = self.layout.page_count();
        if count == 0 {
            return;
        }
        let page = page.min(count - 1);
        if self.fit == Fit::Page {
            self.fit_page = page;
            self.relayout(None);
        }
        let index = page as usize;
        let mut top = self
            .layout
            .page_origin_px(index)
            .map_or(0.0, |(_, y)| y as f64 / self.layout.scale_factor());
        if let Some(y) = y_points.filter(|y| y.is_finite())
            && let Some((_, content_y)) = self.layout.page_point_to_content(index, 0.0, y)
        {
            top = content_y;
        }
        self.scroll_y = top - f64::from(MARGIN);
        self.clamp_scroll();
        self.current = page;
    }

    /// One page's rectangle in content space: x, y, width, height.
    pub fn page_rect(&self, page: u32) -> Option<(f32, f32, f32, f32)> {
        self.layout.page_rect(page as usize)
    }

    // ----- geometry internals -------------------------------------------------------------

    fn viewport_px(&self) -> PxRect {
        let factor = self.layout.scale_factor();
        let (width, height) = (
            f64::from(self.viewport.width),
            f64::from(self.viewport.height),
        );
        PxRect {
            x0: (self.scroll_x * factor).floor() as i64,
            y0: (self.scroll_y * factor).floor() as i64,
            x1: ((self.scroll_x + width) * factor).ceil() as i64,
            y1: ((self.scroll_y + height) * factor).ceil() as i64,
        }
    }

    fn target_geometry(&self) -> Geometry {
        let base = self.layout.fit_scale(
            self.fit,
            self.fit_page as usize,
            self.rotation,
            self.viewport,
        );
        let logical = base * f32::from(self.percent) / 100.0;
        Geometry {
            scale_milli: device_milli(logical, self.viewport.scale_factor),
            scale_factor: self.viewport.scale_factor,
            rotation: self.rotation,
            viewport_width: self.viewport.width,
        }
    }

    fn centre_anchor(&self) -> Option<Anchor> {
        let factor = self.layout.scale_factor();
        let x = self.scroll_x + f64::from(self.viewport.width) / 2.0;
        let y = self.scroll_y + f64::from(self.viewport.height) / 2.0;
        let page = self.layout.nearest_page(y * factor)?;
        let (px, py) = self.layout.content_point_to_page(page, x, y)?;
        Some(Anchor { page, x: px, y: py })
    }

    fn restore_anchor(&mut self, anchor: &Anchor) {
        if let Some((x, y)) = self
            .layout
            .page_point_to_content(anchor.page, anchor.x, anchor.y)
        {
            self.scroll_x = x - f64::from(self.viewport.width) / 2.0;
            self.scroll_y = y - f64::from(self.viewport.height) / 2.0;
        }
    }

    /// Lays the pages out again for the current settings. When the scale changes and an `anchor`
    /// (taken before the change: [`Self::centre_anchor`]) is given, that point goes back to the
    /// centre of the viewport.
    fn relayout(&mut self, anchor: Option<Anchor>) {
        let target = self.target_geometry();
        let old = self.layout.geometry();
        if target != old {
            let rescaled = target.scale_milli != old.scale_milli
                || target.scale_factor != old.scale_factor
                || target.rotation != old.rotation;
            self.layout.set_geometry(target);
            if rescaled {
                self.failed.clear();
                if let Some(anchor) = anchor {
                    self.restore_anchor(&anchor);
                }
            }
        }
        self.clamp_scroll();
        self.update_current();
    }

    fn clamp_scroll(&mut self) {
        let factor = self.layout.scale_factor();
        let (width, height) = self.layout.content_size_px();
        let max_x = (width as f64 / factor - f64::from(self.viewport.width)).max(0.0);
        let max_y = (height as f64 / factor - f64::from(self.viewport.height)).max(0.0);
        self.scroll_x = self.scroll_x.clamp(0.0, max_x);
        self.scroll_y = self.scroll_y.clamp(0.0, max_y);
    }

    /// The page the indicator shows: the first page of which at least half shows (half of its
    /// height, or half of the viewport's if the page is taller than that), else the page with
    /// the most visible height (the earlier on a tie). A jump with `go_to_page` sets it directly
    /// and the next scroll recomputes it.
    fn update_current(&mut self) {
        let viewport = self.viewport_px();
        let range = self.layout.visible_pages(viewport.y0, viewport.y1);
        if range.is_empty() {
            return;
        }
        let viewport_height = viewport.y1 - viewport.y0;
        let mut best = range.start;
        let mut best_overlap = i64::MIN;
        for page in range {
            let Some(rect) = self.layout.page_rect_px(page) else {
                continue;
            };
            let overlap = rect.y1.min(viewport.y1) - rect.y0.max(viewport.y0);
            let needed = ((rect.y1 - rect.y0).min(viewport_height) / 2).max(1);
            if overlap >= needed {
                best = page;
                break;
            }
            if overlap > best_overlap {
                best = page;
                best_overlap = overlap;
            }
        }
        self.current = u32::try_from(best).unwrap_or(0);
    }

    // ----- what to ask for ----------------------------------------------------------------

    fn wanted_region(&self) -> PxRect {
        let viewport = self.viewport_px();
        let height = viewport.y1 - viewport.y0;
        PxRect {
            x0: viewport.x0,
            y0: (viewport.y0 - height).max(0),
            x1: viewport.x1,
            y1: viewport.y1 + height,
        }
    }

    fn wanted_tiles(&self) -> Vec<WantedTile> {
        let viewport = self.viewport_px();
        let region = self.wanted_region();
        let (centre_x, centre_y) = viewport.centre();
        let mut wanted = Vec::new();
        for page in self.layout.visible_pages(region.y0, region.y1) {
            let (Some((cols, rows)), Some((origin_x, origin_y))) = (
                self.layout.cells_in(page, region),
                self.layout.page_origin_px(page),
            ) else {
                continue;
            };
            for row in rows {
                for col in cols.clone() {
                    let (Some((x, y, width, height)), Some(request)) = (
                        self.layout.tile_rect(page, col, row),
                        self.layout.tile_request(page, col, row),
                    ) else {
                        continue;
                    };
                    let cell = PxRect {
                        x0: origin_x + i64::from(x),
                        y0: origin_y + i64::from(y),
                        x1: origin_x + i64::from(x + width),
                        y1: origin_y + i64::from(y + height),
                    };
                    let (cell_x, cell_y) = cell.centre();
                    wanted.push(WantedTile {
                        key: TileKey::from_request(&request),
                        request,
                        visible: cell.intersects(&viewport),
                        distance: (cell_x - centre_x).powi(2) + (cell_y - centre_y).powi(2),
                    });
                }
            }
        }
        wanted.sort_by(|a, b| {
            b.visible
                .cmp(&a.visible)
                .then_with(|| a.distance.total_cmp(&b.distance))
                .then_with(|| {
                    (a.key.page, a.key.row, a.key.col).cmp(&(b.key.page, b.key.row, b.key.col))
                })
        });
        // Everything that shows is wanted. What is asked for beyond is limited to a share of the
        // budget, so that the margin never evicts what it was fetched for.
        let limit = budget::TILE_BYTES / 100 * budget::PREFETCH_SHARE_PERCENT;
        let mut bytes = 0_usize;
        let mut kept = Vec::with_capacity(wanted.len());
        for tile in wanted {
            let cost = tile_bytes(tile.request.width, tile.request.height);
            if !tile.visible && bytes.saturating_add(cost) > limit {
                break;
            }
            bytes = bytes.saturating_add(cost);
            kept.push(tile);
        }
        kept
    }

    fn wanted_placeholders(&self) -> Vec<(TileRequest, bool, f64)> {
        let viewport = self.viewport_px();
        let region = self.wanted_region();
        let (_, centre_y) = viewport.centre();
        let mut wanted: Vec<(TileRequest, bool, f64)> = Vec::new();
        for page in self.layout.visible_pages(region.y0, region.y1) {
            let (Some(size), Some(rect), Ok(index)) = (
                self.layout.page_size(page),
                self.layout.page_rect_px(page),
                u32::try_from(page),
            ) else {
                continue;
            };
            let Some(request) = placeholder_request(size, index, self.rotation) else {
                continue;
            };
            let middle = (rect.y0 + rect.y1) as f64 / 2.0;
            wanted.push((
                request,
                rect.intersects(&viewport),
                (middle - centre_y).abs(),
            ));
        }
        wanted.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.2.total_cmp(&b.2))
                .then_with(|| a.0.page.cmp(&b.0.page))
        });
        let limit = budget::PLACEHOLDER_BYTES / 100 * budget::PREFETCH_SHARE_PERCENT;
        let mut bytes = 0_usize;
        let mut kept = Vec::with_capacity(wanted.len());
        for entry in wanted {
            let cost = tile_bytes(entry.0.width, entry.0.height);
            if !entry.1 && bytes.saturating_add(cost) > limit {
                break;
            }
            bytes = bytes.saturating_add(cost);
            kept.push(entry);
        }
        kept
    }

    /// What to ask the helper for next, given everything that has changed. See the module
    /// documentation for how to carry it out.
    pub fn plan(&mut self) -> Plan {
        let mut plan = Plan::default();
        let tiles = self.wanted_tiles();
        let placeholders = self.wanted_placeholders();

        let mut wanted: HashSet<TileKey> = tiles.iter().map(|tile| tile.key).collect();
        wanted.extend(
            placeholders
                .iter()
                .map(|(request, _, _)| TileKey::from_request(request)),
        );

        // Cancel what is pending and no longer wanted: scrolled away, or at an old zoom.
        plan.cancel = self.pending.take_unwanted(&wanted);

        // Request what is missing, visible first and nearest the centre first.
        for tile in &tiles {
            if self.tiles.touch(&tile.key)
                || self.pending.contains(&tile.key)
                || self.failed.contains(&tile.key)
            {
                continue;
            }
            self.pending.insert(tile.key, tile.request);
            let priority = if tile.visible {
                PRIORITY_VISIBLE
            } else {
                PRIORITY_NEAR
            };
            plan.requests.push((tile.request, priority));
        }
        for (request, _, _) in &placeholders {
            let key = TileKey::from_request(request);
            let slot = (request.page, request.rotation);
            if self.placeholders.touch(&slot) {
                continue;
            }
            // A page that is one tile at this very scale is its own placeholder.
            let same = self
                .tiles
                .peek(&key)
                .filter(|tile| tile.width == request.width && tile.height == request.height)
                .cloned();
            if let Some(tile) = same {
                self.placeholders.insert(slot, tile);
                continue;
            }
            if self.pending.contains(&key) || self.failed.contains(&key) {
                continue;
            }
            self.pending.insert(key, *request);
            plan.requests.push((*request, PRIORITY_PLACEHOLDER));
        }

        // Text for the pages the selection needs, links for the pages that show.
        for page in self.selection.pages_needed(self.layout.page_count()) {
            if self.texts.contains(page) {
                self.texts.touch(page);
            } else if self.text_pending.insert(page) {
                plan.page_text.push(page);
            }
        }
        let viewport = self.viewport_px();
        for page in self.layout.visible_pages(viewport.y0, viewport.y1) {
            let Ok(page) = u32::try_from(page) else {
                continue;
            };
            if self.links.contains(page) {
                self.links.touch(page);
            } else if self.links_pending.insert(page) {
                plan.links.push(page);
            }
        }
        plan
    }

    /// What the caches hold now.
    pub fn cache_usage(&self) -> CacheUsage {
        CacheUsage {
            tiles: self.tiles.len(),
            tile_bytes: self.tiles.bytes(),
            placeholders: self.placeholders.len(),
            placeholder_bytes: self.placeholders.bytes(),
            text_chars: self.texts.cost(),
        }
    }

    /// Forgets what was asked for and not yet answered (and what failed), for the shell to call
    /// when the helper is restarted: the model then asks for it again at the next `plan`.
    pub fn forget_pending(&mut self) {
        self.pending.clear();
        self.failed.clear();
        self.text_pending.clear();
        self.links_pending.clear();
    }

    fn remember_failure(&mut self, key: TileKey) {
        self.failed.remember(key);
    }

    fn evict_tiles(&mut self) {
        let viewport = self.viewport_px();
        let layout = &self.layout;
        self.tiles
            .evict_to_budget(|key| tile_on_screen(layout, &viewport, key));
    }

    fn evict_placeholders(&mut self) {
        let viewport = self.viewport_px();
        let layout = &self.layout;
        self.placeholders
            .evict_to_budget(|key| placeholder_on_screen(layout, &viewport, key));
    }

    /// A tile the helper rendered. A tile nobody wants any more (an old zoom, a page that does
    /// not exist) is dropped; a tile whose pixels do not match its request counts as failed.
    pub fn tile_arrived(&mut self, request: &TileRequest, tile: Tile) {
        let key = TileKey::from_request(request);
        self.pending.remove(&key);
        if !tile.is_consistent() || tile.width != request.width || tile.height != request.height {
            self.remember_failure(key);
            return;
        }
        let index = request.page as usize;
        if !request.x.is_multiple_of(TILE_SIDE) || !request.y.is_multiple_of(TILE_SIDE) {
            return;
        }
        let Some(size) = self.layout.page_size(index) else {
            return;
        };
        let tile = Arc::new(tile);
        let current = key.scale_milli == self.layout.scale_milli()
            && key.rotation == self.layout.rotation()
            && self.layout.tile_rect(index, key.col, key.row)
                == Some((request.x, request.y, request.width, request.height));
        if current {
            self.tiles.insert(key, Arc::clone(&tile));
            self.evict_tiles();
        }
        let placeholder =
            placeholder_request(size, request.page, request.rotation).is_some_and(|expected| {
                TileKey::from_request(&expected) == key
                    && (expected.width, expected.height) == (request.width, request.height)
            });
        if placeholder {
            self.placeholders
                .insert((request.page, request.rotation), tile);
            self.evict_placeholders();
        }
    }

    /// A tile request the helper could not answer: it is not asked for again until the zoom or
    /// rotation changes (or [`Self::forget_pending`]).
    pub fn tile_failed(&mut self, request: &TileRequest) {
        let key = TileKey::from_request(request);
        self.pending.remove(&key);
        self.remember_failure(key);
    }

    /// The text of a page. Also call it, with empty text, when the helper could not read it.
    pub fn set_page_text(&mut self, page: u32, text: PageText) {
        self.text_pending.remove(&page);
        if page >= self.layout.page_count() {
            return;
        }
        let cost = text.chars.len() + 1;
        self.texts.insert(page, IndexedText::new(text), cost);
        self.selection.update(&self.texts);
        let keep: HashSet<u32> = self
            .selection
            .pages_needed(self.layout.page_count())
            .into_iter()
            .collect();
        self.texts.evict(|page| keep.contains(&page));
    }

    /// The links of a page. Also call it, with none, when the helper could not read them.
    pub fn set_links(&mut self, page: u32, links: Vec<Link>) {
        self.links_pending.remove(&page);
        if page >= self.layout.page_count() {
            return;
        }
        let cost = links.len() + 1;
        self.links.insert(page, links, cost);
        let viewport = self.viewport_px();
        let showing = self.layout.visible_pages(viewport.y0, viewport.y1);
        self.links.evict(|page| showing.contains(&(page as usize)));
    }

    // ----- what to draw -------------------------------------------------------------------

    /// The tiles to draw, in the order to draw them (see the module documentation).
    pub fn visible_tiles(&self) -> Vec<PlacedTile> {
        let viewport = self.viewport_px();
        let factor = self.layout.scale_factor();
        let mut under = Vec::new();
        let mut exact = Vec::new();
        for page in self.layout.visible_pages(viewport.y0, viewport.y1) {
            let (Some((cols, rows)), Some((origin_x, origin_y)), Ok(index)) = (
                self.layout.cells_in(page, viewport),
                self.layout.page_origin_px(page),
                u32::try_from(page),
            ) else {
                continue;
            };
            let mut missing = false;
            for row in rows {
                for col in cols.clone() {
                    let key = TileKey {
                        page: index,
                        scale_milli: self.layout.scale_milli(),
                        rotation: self.layout.rotation(),
                        col,
                        row,
                    };
                    let (Some(tile), Some((x, y, _, _))) =
                        (self.tiles.peek(&key), self.layout.tile_rect(page, col, row))
                    else {
                        missing = true;
                        continue;
                    };
                    exact.push(PlacedTile {
                        x: ((origin_x + i64::from(x)) as f64 / factor) as f32,
                        y: ((origin_y + i64::from(y)) as f64 / factor) as f32,
                        width: (f64::from(tile.width) / factor) as f32,
                        height: (f64::from(tile.height) / factor) as f32,
                        page: index,
                        pixels: Arc::clone(tile),
                        is_placeholder: false,
                    });
                }
            }
            if missing
                && let Some(pixels) = self.placeholders.peek(&(index, self.layout.rotation()))
                && let Some((x, y, width, height)) = self.layout.page_rect(page)
            {
                under.push(PlacedTile {
                    x,
                    y,
                    width,
                    height,
                    page: index,
                    pixels: Arc::clone(pixels),
                    is_placeholder: true,
                });
            }
        }
        under.extend(exact);
        under
    }

    /// The pages that show in the viewport, as paper: page index, x, y, width, height.
    pub fn page_rects(&self) -> Vec<(u32, f32, f32, f32, f32)> {
        let viewport = self.viewport_px();
        self.layout
            .visible_pages(viewport.y0, viewport.y1)
            .filter_map(|page| {
                let (x, y, width, height) = self.layout.page_rect(page)?;
                Some((u32::try_from(page).ok()?, x, y, width, height))
            })
            .collect()
    }

    fn viewport_rect(&self) -> (f64, f64, f64, f64) {
        (
            self.scroll_x,
            self.scroll_y,
            f64::from(self.viewport.width),
            f64::from(self.viewport.height),
        )
    }

    fn shows(&self, rect: (f32, f32, f32, f32)) -> bool {
        let (vx, vy, vw, vh) = self.viewport_rect();
        let (x, y, w, h) = (
            f64::from(rect.0),
            f64::from(rect.1),
            f64::from(rect.2),
            f64::from(rect.3),
        );
        x < vx + vw && vx < x + w && y < vy + vh && vy < y + h
    }

    // ----- pointer, selection, links ------------------------------------------------------

    fn page_point(&self, x: f32, y: f32) -> Option<PagePoint> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let factor = self.layout.scale_factor();
        let page = self.layout.nearest_page(f64::from(y) * factor)?;
        let (px, py) = self
            .layout
            .content_point_to_page(page, f64::from(x), f64::from(y))?;
        Some(PagePoint {
            page: u32::try_from(page).ok()?,
            x: px,
            y: py,
        })
    }

    /// A press at a point of content space; `clicks` is the number of clicks in a row.
    pub fn pointer_down(&mut self, x: f32, y: f32, clicks: u8) {
        match self.page_point(x, y) {
            Some(point) => {
                self.selection.press(point, clicks);
                self.selection.update(&self.texts);
            }
            None => self.selection.clear(),
        }
    }

    /// The pointer moved with the button down.
    pub fn pointer_drag(&mut self, x: f32, y: f32) {
        if !self.selection.is_held() {
            return;
        }
        if let Some(point) = self.page_point(x, y) {
            self.selection.drag(point);
            self.selection.update(&self.texts);
        }
    }

    pub fn pointer_up(&mut self) {
        self.selection.release();
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn has_selection(&self) -> bool {
        self.selection.range().is_some()
    }

    /// The selected text, for Copy.
    pub fn selected_text(&self) -> Option<String> {
        self.selection.text(&self.texts)
    }

    /// Whether the text of every page of the selection has arrived (Copy before then would miss
    /// pages).
    pub fn selection_complete(&self) -> bool {
        self.selection.is_complete(&self.texts)
    }

    /// The highlight rectangles of the selection that show: x, y, width, height, one per line.
    pub fn selection_rects(&self) -> Vec<(f32, f32, f32, f32)> {
        if !self.has_selection() {
            return Vec::new();
        }
        let viewport = self.viewport_px();
        let mut rects = Vec::new();
        for page in self.layout.visible_pages(viewport.y0, viewport.y1) {
            let Ok(index) = u32::try_from(page) else {
                continue;
            };
            for rect in self.selection.rects_on_page(index, &self.texts) {
                if let Some(placed) = self.layout.page_rect_to_content(page, &rect)
                    && self.shows(placed)
                {
                    rects.push(placed);
                }
            }
        }
        rects
    }

    /// The link under a point of content space, if the point is on a page whose links have
    /// arrived; the smallest if several contain it.
    pub fn link_at(&self, x: f32, y: f32) -> Option<LinkTarget> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let factor = self.layout.scale_factor();
        let page = self
            .layout
            .page_at_point_px(f64::from(x) * factor, f64::from(y) * factor)?;
        let links = self.links.get(u32::try_from(page).ok()?)?;
        let (px, py) = self
            .layout
            .content_point_to_page(page, f64::from(x), f64::from(y))?;
        links
            .iter()
            .filter(|link| link.rect.contains(px, py))
            .min_by(|a, b| {
                let area = |rect: &Rect| rect.width() * rect.height();
                area(&a.rect).total_cmp(&area(&b.rect))
            })
            .map(|link| link.target.clone())
    }

    // ----- find ---------------------------------------------------------------------------

    /// Starts a find, replacing the old one; returns the model's id for it.
    pub fn find_start(&mut self, query: &str, options: FindOptions) -> u64 {
        self.find.start(query, options)
    }

    /// Takes the id the client returned for the search just started as the one to expect.
    pub fn find_bind(&mut self, id: u64) {
        self.find.adopt_id(id);
    }

    /// An answer of the helper's find: the hits on one page. False when it was for another
    /// find than the current one and was ignored.
    pub fn find_hits(
        &mut self,
        find: u64,
        page: u32,
        hits: Vec<SearchHit>,
        finished: bool,
    ) -> bool {
        // Hits on a page the document does not have are dropped (the answer still counts).
        let hits = if page < self.layout.page_count() {
            hits
        } else {
            Vec::new()
        };
        match self.find.hits(find, page, hits, finished) {
            HitsOutcome::Stale => false,
            HitsOutcome::Accepted { current_changed } => {
                if current_changed {
                    self.reveal_current_hit();
                }
                true
            }
        }
    }

    /// Moves to the next hit (wrapping) and scrolls it into view; false when there is none.
    pub fn find_next(&mut self) -> bool {
        let moved = self.find.select_next().is_some();
        if moved {
            self.reveal_current_hit();
        }
        moved
    }

    /// Moves to the previous hit (wrapping) and scrolls it into view; false when there is none.
    pub fn find_previous(&mut self) -> bool {
        let moved = self.find.select_previous().is_some();
        if moved {
            self.reveal_current_hit();
        }
        moved
    }

    pub fn find_clear(&mut self) {
        self.find.clear();
    }

    pub fn find_status(&self) -> FindProgress {
        self.find.progress()
    }

    pub fn find_query(&self) -> &str {
        self.find.query()
    }

    pub fn find_options(&self) -> FindOptions {
        self.find.options()
    }

    pub fn find_is_active(&self) -> bool {
        self.find.is_active()
    }

    /// The highlight rectangles of the find that show: x, y, width, height, and whether it is
    /// the current hit.
    pub fn find_rects(&self) -> Vec<(f32, f32, f32, f32, bool)> {
        if !self.find.is_active() {
            return Vec::new();
        }
        let viewport = self.viewport_px();
        let mut rects = Vec::new();
        for page in self.layout.visible_pages(viewport.y0, viewport.y1) {
            let Ok(index) = u32::try_from(page) else {
                continue;
            };
            for hit in self.find.hits_on_page(index) {
                let current = self.find.is_current(hit);
                for rect in &hit.rects {
                    if let Some(placed) = self.layout.page_rect_to_content(page, rect)
                        && self.shows(placed)
                    {
                        rects.push((placed.0, placed.1, placed.2, placed.3, current));
                    }
                }
            }
        }
        rects
    }

    /// Scrolls the current hit into view if it is not entirely in it (centred), and keeps the
    /// scroll where it is otherwise.
    fn reveal_current_hit(&mut self) {
        let Some(hit) = self.find.current() else {
            return;
        };
        let (page, rect) = (hit.page as usize, hit.rects.first().copied());
        let Some(rect) = rect else {
            return;
        };
        let Some((x, y, width, height)) = self.layout.page_rect_to_content(page, &rect) else {
            return;
        };
        let (x, y, width, height) = (
            f64::from(x),
            f64::from(y),
            f64::from(width),
            f64::from(height),
        );
        let (view_x, view_y, view_width, view_height) = self.viewport_rect();
        let margin = f64::from(MARGIN);
        if y < view_y + margin || y + height > view_y + view_height - margin {
            self.scroll_y = y + height / 2.0 - view_height / 2.0;
        }
        if x < view_x || x + width > view_x + view_width {
            self.scroll_x = x + width / 2.0 - view_width / 2.0;
        }
        self.clamp_scroll();
        self.update_current();
    }
}
