// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Where each page sits in the scrolling document at a zoom, and which tiles a viewport needs.
//!
//! Pages are stacked vertically, centred, with [`MARGIN`] around the stack and [`GAP`] between
//! pages (logical pixels). Everything is computed on the **device pixel grid**: a page is
//! [`PageSize::pixel_size`] device pixels at the zoom's scale, the margin and the gap are whole
//! device pixels, and a page's origin is a whole device pixel, so a tile lands on the pixels it
//! was rendered for and the grid is exact (no tile overlaps another, none leaves its page). The
//! logical size of anything is its device size divided by the scale factor.
//!
//! The offsets of the pages are a prefix sum, built once per zoom ([`Layout::set_geometry`],
//! linear in the page count and a few microseconds for the 20,000 pages the limit allows). Every
//! lookup after that (the page at a y position, the pages in a range) is a binary search, so a
//! 20,000-page document costs what a 3-page one costs while scrolling.
//!
//! The zoom is a *device scale* in whole thousandths of a device pixel per point
//! ([`device_milli`]); tile requests use exactly that scale, so a tile's cache key (scale in
//! thousandths) and its request agree.
//!
//! Page space (what the PDF engine reports: points, y down, the page's own `/Rotate` applied) and
//! the displayed page differ by the view's [`Rotation`]; [`Layout::page_point_to_content`] and
//! [`Layout::content_point_to_page`] convert between page space and content space.

use std::ops::Range;

use crate::cache::budget::PLACEHOLDER_MAX_WIDTH;
use crate::types::{PageSize, Rect, Rotation, TileRequest, limits};

/// The space around the stack of pages, in logical pixels.
pub const MARGIN: f32 = 8.0;
/// The space between two pages, in logical pixels.
pub const GAP: f32 = 8.0;
/// The side of a tile, in device pixels.
pub const TILE_SIDE: u32 = 512;
/// The zoom range, in percent of the fit.
pub const MIN_PERCENT: u16 = 30;
/// See [`MIN_PERCENT`].
pub const MAX_PERCENT: u16 = 500;
/// The largest page side the viewer lays out, in points; a hostile file's larger side is cut to it,
/// and a side that is zero, negative or not a number becomes one point, so that no arithmetic here
/// can overflow or divide by zero.
pub const MAX_PAGE_POINTS: f32 = 50_000.0;

const MIN_SCALE_MILLI: f64 = 50.0;
const MAX_SCALE_MILLI: f64 = 16_000.0;

/// What a zoom is relative to: the widest page, or the page being read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// 100 % makes the widest page as wide as the window (less the margins).
    #[default]
    Width,
    /// 100 % makes the current page fit the window, whole.
    Page,
}

/// The visible area: logical pixels and the device pixel ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    pub scale_factor: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        }
    }
}

impl Viewport {
    /// The same viewport with every number made usable: finite, at least one pixel, a scale
    /// factor between 0.25 and 8.
    pub fn sanitized(self) -> Self {
        let side = |value: f32, fallback: f32| {
            if value.is_finite() {
                value.max(1.0)
            } else {
                fallback
            }
        };
        let factor = if self.scale_factor.is_finite() && self.scale_factor > 0.0 {
            self.scale_factor.clamp(0.25, 8.0)
        } else {
            1.0
        };
        Self {
            width: side(self.width, 800.0),
            height: side(self.height, 600.0),
            scale_factor: factor,
        }
    }
}

/// A rectangle of content space in device pixels, half-open: `x0 <= x < x1`, `y0 <= y < y1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PxRect {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
}

impl PxRect {
    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    pub fn intersects(&self, other: &Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.x0 < other.x1
            && other.x0 < self.x1
            && self.y0 < other.y1
            && other.y0 < self.y1
    }

    pub fn centre(&self) -> (f64, f64) {
        (
            (self.x0 + self.x1) as f64 / 2.0,
            (self.y0 + self.y1) as f64 / 2.0,
        )
    }
}

/// What a layout depends on besides the pages: the device scale in thousandths, the device pixel
/// ratio, the view rotation and the viewport's width (which centres the pages).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub scale_milli: u32,
    pub scale_factor: f32,
    pub rotation: Rotation,
    pub viewport_width: f32,
}

/// The percent clamped to [`MIN_PERCENT`]..=[`MAX_PERCENT`].
pub fn clamp_percent(percent: u16) -> u16 {
    percent.clamp(MIN_PERCENT, MAX_PERCENT)
}

/// The device scale, in whole thousandths of a device pixel per point, of a zoom of
/// `logical_scale` logical pixels per point on a screen of `scale_factor`; kept within the
/// engine's range (0.05 to 16). It is rounded **down** (the part of a thousandth that single
/// precision noise adds is forgiven), so that a page fitted to the width is never one pixel
/// wider than the window, which would give "fit width" a scroll bar for a pixel.
pub fn device_milli(logical_scale: f32, scale_factor: f32) -> u32 {
    let milli = (f64::from(logical_scale) * f64::from(scale_factor) * 1000.0 + 0.001).floor();
    if milli.is_finite() {
        milli.clamp(MIN_SCALE_MILLI, MAX_SCALE_MILLI) as u32
    } else {
        1000
    }
}

fn sane(size: PageSize) -> PageSize {
    let side = |value: f32| {
        if value.is_finite() && value > 0.0 {
            value.min(MAX_PAGE_POINTS)
        } else {
            1.0
        }
    };
    PageSize::new(side(size.width), side(size.height))
}

/// The request for the low-resolution placeholder of a page: the whole page, at most
/// [`PLACEHOLDER_MAX_WIDTH`] pixels wide and one tile high, at a scale in whole thousandths.
/// `None` when the page is so large that even the smallest scale the engine accepts would be
/// wider than that.
pub fn placeholder_request(size: PageSize, page: u32, rotation: Rotation) -> Option<TileRequest> {
    let size = sane(size);
    let rotated = size.rotated(rotation);
    let ideal = (PLACEHOLDER_MAX_WIDTH as f32 / rotated.width)
        .min(limits::MAX_TILE_SIDE as f32 / rotated.height);
    let mut milli = ((ideal * 1000.0).floor() as u32).min(MAX_SCALE_MILLI as u32);
    loop {
        if f64::from(milli) < MIN_SCALE_MILLI {
            return None;
        }
        let scale = milli as f32 / 1000.0;
        let (width, height) = size.pixel_size(scale, rotation);
        if width <= PLACEHOLDER_MAX_WIDTH
            && height <= limits::MAX_TILE_SIDE
            && u64::from(width) * u64::from(height) <= u64::from(limits::MAX_TILE_PIXELS)
        {
            return Some(TileRequest {
                page,
                scale,
                rotation,
                x: 0,
                y: 0,
                width,
                height,
            });
        }
        milli = milli.checked_sub(1)?;
    }
}

/// Where the pages are at one zoom, rotation and viewport width.
pub struct Layout {
    pages: Vec<PageSize>,
    max_width: f32,
    max_height: f32,
    geometry: Geometry,
    sizes: Vec<(u32, u32)>,
    tops: Vec<u64>,
    margin_px: u64,
    gap_px: u64,
    content_w_px: u64,
    content_h_px: u64,
}

impl Layout {
    /// A layout of `pages` at a scale of 1 device pixel per point, unrotated, 800 logical pixels
    /// wide; call [`Self::set_geometry`] for the real one.
    pub fn new(pages: Vec<PageSize>) -> Self {
        let pages: Vec<PageSize> = pages.into_iter().map(sane).collect();
        let max_width = pages.iter().map(|page| page.width).fold(1.0, f32::max);
        let max_height = pages.iter().map(|page| page.height).fold(1.0, f32::max);
        let mut layout = Self {
            pages,
            max_width,
            max_height,
            geometry: Geometry {
                scale_milli: 1000,
                scale_factor: 1.0,
                rotation: Rotation::Deg0,
                viewport_width: 800.0,
            },
            sizes: Vec::new(),
            tops: Vec::new(),
            margin_px: 0,
            gap_px: 0,
            content_w_px: 0,
            content_h_px: 0,
        };
        layout.rebuild();
        layout
    }

    /// Changes the zoom, rotation, device pixel ratio or viewport width and rebuilds the offsets;
    /// false (and nothing done) when nothing changed.
    pub fn set_geometry(&mut self, geometry: Geometry) -> bool {
        if geometry == self.geometry {
            return false;
        }
        self.geometry = geometry;
        self.rebuild();
        true
    }

    fn rebuild(&mut self) {
        let scale = self.device_scale();
        let rotation = self.geometry.rotation;
        let factor = self.scale_factor();
        self.margin_px = (f64::from(MARGIN) * factor).round().max(1.0) as u64;
        self.gap_px = (f64::from(GAP) * factor).round().max(1.0) as u64;
        self.sizes.clear();
        self.tops.clear();
        let mut y = self.margin_px;
        let mut widest = 0_u64;
        let count = self.pages.len();
        for (index, page) in self.pages.iter().enumerate() {
            let (width, height) = page.pixel_size(scale, rotation);
            self.tops.push(y);
            self.sizes.push((width, height));
            widest = widest.max(u64::from(width));
            y += u64::from(height);
            if index + 1 < count {
                y += self.gap_px;
            }
        }
        self.content_h_px = y + self.margin_px;
        let viewport_px = (f64::from(self.geometry.viewport_width) * factor).floor() as u64;
        self.content_w_px = (widest + 2 * self.margin_px).max(viewport_px);
    }

    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    pub fn page_count(&self) -> u32 {
        u32::try_from(self.pages.len()).unwrap_or(u32::MAX)
    }

    /// A page's size in points before the view rotation (made sane: see [`MAX_PAGE_POINTS`]).
    pub fn page_size(&self, page: usize) -> Option<PageSize> {
        self.pages.get(page).copied()
    }

    /// The device scale (device pixels per point) of the current zoom: whole thousandths.
    pub fn device_scale(&self) -> f32 {
        self.geometry.scale_milli as f32 / 1000.0
    }

    pub fn scale_milli(&self) -> u32 {
        self.geometry.scale_milli
    }

    pub fn rotation(&self) -> Rotation {
        self.geometry.rotation
    }

    pub fn scale_factor(&self) -> f64 {
        f64::from(self.geometry.scale_factor)
    }

    /// Logical pixels per point.
    pub fn logical_scale(&self) -> f32 {
        (f64::from(self.device_scale()) / self.scale_factor()) as f32
    }

    /// The zoom to give `fit` at 100 %, in logical pixels per point: the widest page (as
    /// displayed, so rotation counts) as wide as the viewport less the margins, or the page
    /// `fit_page` whole in it.
    pub fn fit_scale(
        &self,
        fit: Fit,
        fit_page: usize,
        rotation: Rotation,
        viewport: Viewport,
    ) -> f32 {
        let available_width = (viewport.width - 2.0 * MARGIN).max(1.0);
        let available_height = (viewport.height - 2.0 * MARGIN).max(1.0);
        match fit {
            Fit::Width => {
                let widest = if rotation.swaps_axes() {
                    self.max_height
                } else {
                    self.max_width
                };
                available_width / widest
            }
            Fit::Page => {
                let size = self
                    .pages
                    .get(fit_page)
                    .or_else(|| self.pages.first())
                    .copied()
                    .unwrap_or(PageSize::new(612.0, 792.0))
                    .rotated(rotation);
                (available_width / size.width).min(available_height / size.height)
            }
        }
    }

    /// A page's size in device pixels as displayed (rotation applied).
    pub fn page_pixels(&self, page: usize) -> Option<(u32, u32)> {
        self.sizes.get(page).copied()
    }

    /// A page's top-left corner in content space, in device pixels.
    pub fn page_origin_px(&self, page: usize) -> Option<(i64, i64)> {
        let (width, _) = self.page_pixels(page)?;
        let left = self.content_w_px.saturating_sub(u64::from(width)) / 2;
        Some((left as i64, *self.tops.get(page)? as i64))
    }

    pub fn page_rect_px(&self, page: usize) -> Option<PxRect> {
        let (width, height) = self.page_pixels(page)?;
        let (x, y) = self.page_origin_px(page)?;
        Some(PxRect {
            x0: x,
            y0: y,
            x1: x + i64::from(width),
            y1: y + i64::from(height),
        })
    }

    /// A page's rectangle in content space: x, y, width, height in logical pixels.
    pub fn page_rect(&self, page: usize) -> Option<(f32, f32, f32, f32)> {
        let rect = self.page_rect_px(page)?;
        let factor = self.scale_factor();
        Some((
            (rect.x0 as f64 / factor) as f32,
            (rect.y0 as f64 / factor) as f32,
            ((rect.x1 - rect.x0) as f64 / factor) as f32,
            ((rect.y1 - rect.y0) as f64 / factor) as f32,
        ))
    }

    /// The size of the content in device pixels.
    pub fn content_size_px(&self) -> (u64, u64) {
        (self.content_w_px, self.content_h_px)
    }

    /// The size of the content in logical pixels: at least as wide as the pages with their
    /// margins and no wider than the viewport unless a page needs it.
    pub fn content_size(&self) -> (f32, f32) {
        let factor = self.scale_factor();
        (
            (self.content_w_px as f64 / factor) as f32,
            (self.content_h_px as f64 / factor) as f32,
        )
    }

    /// The page that contains the y position, or the nearer neighbour when it is in a gap or
    /// outside the stack (the first or last page). `None` only for a document with no pages.
    pub fn nearest_page(&self, y_px: f64) -> Option<usize> {
        let count = self.tops.len();
        if count == 0 || y_px < self.tops[0] as f64 {
            return (count > 0).then_some(0);
        }
        let after = self.tops.partition_point(|&top| top as f64 <= y_px);
        let page = after - 1;
        let bottom = self.tops[page] + u64::from(self.sizes[page].1);
        if y_px < bottom as f64 || page + 1 >= count {
            return Some(page);
        }
        let middle = bottom as f64 + self.gap_px as f64 / 2.0;
        Some(if y_px < middle { page } else { page + 1 })
    }

    /// The page under a point of content space (device pixels), if it is on a page.
    pub fn page_at_point_px(&self, x_px: f64, y_px: f64) -> Option<usize> {
        let page = self.nearest_page(y_px)?;
        let rect = self.page_rect_px(page)?;
        let inside = x_px >= rect.x0 as f64
            && x_px < rect.x1 as f64
            && y_px >= rect.y0 as f64
            && y_px < rect.y1 as f64;
        inside.then_some(page)
    }

    /// The pages that have any pixel in the rows `y0..y1` of content space.
    pub fn visible_pages(&self, y0: i64, y1: i64) -> Range<usize> {
        let count = self.tops.len();
        if count == 0 || y1 <= y0 {
            return 0..0;
        }
        let started = self.tops.partition_point(|&top| top as i64 <= y0);
        let first = match started {
            0 => 0,
            _ => {
                let page = started - 1;
                let bottom = self.tops[page] + u64::from(self.sizes[page].1);
                if bottom as i64 <= y0 { started } else { page }
            }
        };
        let end = self.tops.partition_point(|&top| (top as i64) < y1);
        first.min(end)..end
    }

    /// The number of tile columns and rows of a page.
    pub fn grid(&self, page: usize) -> Option<(u32, u32)> {
        let (width, height) = self.page_pixels(page)?;
        Some((width.div_ceil(TILE_SIDE), height.div_ceil(TILE_SIDE)))
    }

    /// A tile's rectangle inside its page, in device pixels: x, y, width, height. Edge tiles are
    /// smaller than [`TILE_SIDE`].
    pub fn tile_rect(&self, page: usize, col: u32, row: u32) -> Option<(u32, u32, u32, u32)> {
        let (width, height) = self.page_pixels(page)?;
        let (x, y) = (col.checked_mul(TILE_SIDE)?, row.checked_mul(TILE_SIDE)?);
        if x >= width || y >= height {
            return None;
        }
        Some((x, y, TILE_SIDE.min(width - x), TILE_SIDE.min(height - y)))
    }

    /// The request that renders one tile at the current zoom.
    pub fn tile_request(&self, page: usize, col: u32, row: u32) -> Option<TileRequest> {
        let (x, y, width, height) = self.tile_rect(page, col, row)?;
        Some(TileRequest {
            page: u32::try_from(page).ok()?,
            scale: self.device_scale(),
            rotation: self.geometry.rotation,
            x,
            y,
            width,
            height,
        })
    }

    /// The tiles of a page that a region of content space touches: the range of columns and the
    /// range of rows. `None` when the region misses the page.
    pub fn cells_in(&self, page: usize, region: PxRect) -> Option<(Range<u32>, Range<u32>)> {
        let (width, height) = self.page_pixels(page)?;
        let (origin_x, origin_y) = self.page_origin_px(page)?;
        let x0 = (region.x0 - origin_x).max(0);
        let x1 = (region.x1 - origin_x).min(i64::from(width));
        let y0 = (region.y0 - origin_y).max(0);
        let y1 = (region.y1 - origin_y).min(i64::from(height));
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let side = i64::from(TILE_SIDE);
        let cell = |low: i64, high: i64| (low / side) as u32..((high - 1) / side + 1) as u32;
        Some((cell(x0, x1), cell(y0, y1)))
    }

    /// Device pixels per point along each axis of the displayed page: the page's pixel size over
    /// its size in points (they differ from the scale by the rounding up of the pixel size).
    fn factors(&self, page: usize) -> Option<(f64, f64)> {
        let displayed = self.pages.get(page)?.rotated(self.geometry.rotation);
        let (width, height) = self.page_pixels(page)?;
        Some((
            f64::from(width) / f64::from(displayed.width),
            f64::from(height) / f64::from(displayed.height),
        ))
    }

    /// A point of page space (points) in content space (logical pixels).
    pub fn page_point_to_content(&self, page: usize, x: f32, y: f32) -> Option<(f64, f64)> {
        let size = *self.pages.get(page)?;
        let (width, height) = (f64::from(size.width), f64::from(size.height));
        let (x, y) = (f64::from(x), f64::from(y));
        let (dx, dy) = match self.geometry.rotation {
            Rotation::Deg0 => (x, y),
            Rotation::Deg90 => (height - y, x),
            Rotation::Deg180 => (width - x, height - y),
            Rotation::Deg270 => (y, width - x),
        };
        let (fx, fy) = self.factors(page)?;
        let (origin_x, origin_y) = self.page_origin_px(page)?;
        let factor = self.scale_factor();
        Some((
            (origin_x as f64 + dx * fx) / factor,
            (origin_y as f64 + dy * fy) / factor,
        ))
    }

    /// A point of content space (logical pixels) in the page space of `page` (points); the
    /// inverse of [`Self::page_point_to_content`], also for points off the page.
    pub fn content_point_to_page(&self, page: usize, x: f64, y: f64) -> Option<(f32, f32)> {
        let size = *self.pages.get(page)?;
        let (width, height) = (f64::from(size.width), f64::from(size.height));
        let (fx, fy) = self.factors(page)?;
        let (origin_x, origin_y) = self.page_origin_px(page)?;
        let factor = self.scale_factor();
        let dx = (x * factor - origin_x as f64) / fx;
        let dy = (y * factor - origin_y as f64) / fy;
        let (px, py) = match self.geometry.rotation {
            Rotation::Deg0 => (dx, dy),
            Rotation::Deg90 => (dy, height - dx),
            Rotation::Deg180 => (width - dx, height - dy),
            Rotation::Deg270 => (width - dy, dx),
        };
        Some((px as f32, py as f32))
    }

    /// A rectangle of page space in content space: x, y, width, height in logical pixels.
    pub fn page_rect_to_content(&self, page: usize, rect: &Rect) -> Option<(f32, f32, f32, f32)> {
        let (x0, y0) = self.page_point_to_content(page, rect.left, rect.top)?;
        let (x1, y1) = self.page_point_to_content(page, rect.right, rect.bottom)?;
        let (left, right) = (x0.min(x1), x0.max(x1));
        let (top, bottom) = (y0.min(y1), y0.max(y1));
        Some((
            left as f32,
            top as f32,
            (right - left) as f32,
            (bottom - top) as f32,
        ))
    }
}
