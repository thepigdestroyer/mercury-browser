// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The viewer model as the shell drives it: planning, zoom, scrolling, selection through the
//! pointer, links and find, through the public API only.

use std::collections::{BTreeMap, HashSet};

use mercury_pdf::types::{
    DocumentInfo, FindOptions, Link, LinkTarget, PageSize, PageText, Rect, Rotation, SearchHit,
    TextChar, Tile, TileRequest,
};
use mercury_pdf::viewer::{
    Fit, PRIORITY_NEAR, PRIORITY_PLACEHOLDER, PRIORITY_VISIBLE, Plan, ViewerModel, Viewport,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }
}

type Key = (u32, u32, u16, u32, u32);

fn key(request: &TileRequest) -> Key {
    (
        request.page,
        request.scale.to_bits(),
        request.rotation.degrees(),
        request.x,
        request.y,
    )
}

fn viewport(width: f32, height: f32, scale_factor: f32) -> Viewport {
    Viewport {
        width,
        height,
        scale_factor,
    }
}

fn info(pages: &[(f32, f32)]) -> DocumentInfo {
    DocumentInfo {
        pages: pages
            .iter()
            .map(|(width, height)| PageSize::new(*width, *height))
            .collect(),
        title: Some("A test".to_owned()),
    }
}

fn model(pages: &[(f32, f32)], viewport: Viewport) -> ViewerModel {
    let mut model = ViewerModel::new(info(pages));
    model.set_viewport(viewport);
    model
}

fn letter_pages(count: usize) -> Vec<(f32, f32)> {
    vec![(600.0, 800.0); count]
}

fn tile_for(request: &TileRequest) -> Tile {
    Tile {
        width: request.width,
        height: request.height,
        rgba: vec![0; (request.width * request.height * 4) as usize],
    }
}

/// What the shell does with the model: it carries a plan out, keeping the set of requests the
/// helper is working on, and checks every rule of the contract as it goes.
struct Shell {
    model: ViewerModel,
    pages: Vec<PageSize>,
    outstanding: BTreeMap<Key, TileRequest>,
    zombies: Vec<TileRequest>,
}

impl Shell {
    fn new(pages: &[(f32, f32)], viewport: Viewport) -> Self {
        Self {
            model: model(pages, viewport),
            pages: info(pages).pages,
            outstanding: BTreeMap::new(),
            zombies: Vec::new(),
        }
    }

    /// Plans and carries the plan out: cancels first, then requests.
    fn pump(&mut self) -> Plan {
        let plan = self.model.plan();
        for request in &plan.cancel {
            assert!(
                self.outstanding.remove(&key(request)).is_some(),
                "cancelled a request that is not outstanding: {request:?}"
            );
            self.zombies.push(*request);
        }
        for (request, priority) in &plan.requests {
            assert_eq!(request.validate(&self.pages), Ok(()), "{request:?}");
            assert!(
                self.outstanding.insert(key(request), *request).is_none(),
                "requested twice: {request:?}"
            );
            assert!(
                [PRIORITY_VISIBLE, PRIORITY_NEAR, PRIORITY_PLACEHOLDER].contains(priority),
                "{priority}"
            );
        }
        plan
    }

    fn deliver_all(&mut self) {
        let outstanding = std::mem::take(&mut self.outstanding);
        for request in outstanding.values() {
            self.model.tile_arrived(request, tile_for(request));
        }
    }

    /// Delivers about half of what is outstanding, and some of what was cancelled (a cancelled
    /// request may still answer).
    fn deliver_some(&mut self, rng: &mut Rng) {
        let keys: Vec<Key> = self.outstanding.keys().copied().collect();
        for one in keys {
            if rng.below(2) == 0
                && let Some(request) = self.outstanding.remove(&one)
            {
                self.model.tile_arrived(&request, tile_for(&request));
            }
        }
        let zombies = std::mem::take(&mut self.zombies);
        for request in zombies {
            if rng.below(3) == 0 {
                self.model.tile_arrived(&request, tile_for(&request));
            }
        }
    }

    /// Delivers everything until the model has nothing more to ask for.
    fn settle(&mut self) {
        for _ in 0..30 {
            let plan = self.pump();
            if plan.requests.is_empty() && self.outstanding.is_empty() {
                return;
            }
            self.deliver_all();
        }
        panic!("the model keeps asking for more");
    }
}

fn plain() -> Viewport {
    viewport(616.0, 600.0, 1.0)
}

fn summary(plan: &Plan) -> Vec<(u32, u32, u32, u8)> {
    plan.requests
        .iter()
        .map(|(request, priority)| (request.page, request.x, request.y, *priority))
        .collect()
}

#[test]
fn viewer_a_new_document_starts_at_the_top_at_fit_width() {
    let model = model(&letter_pages(3), plain());
    assert_eq!(model.title(), Some("A test"));
    assert_eq!(model.page_count(), 3);
    assert_eq!(model.zoom_percent(), 100);
    assert_eq!(model.fit(), Fit::Width);
    assert_eq!(model.rotation(), Rotation::Deg0);
    assert_eq!((model.scroll_x(), model.scroll_y()), (0.0, 0.0));
    assert_eq!(model.content_size(), (616.0, 2432.0));
    assert_eq!(model.current_page(), 0);
    assert_eq!(model.device_scale(), 1.0);
    // Only the first page shows: 8..808 of a 600-pixel viewport; the next starts at 816.
    assert_eq!(model.page_rects(), vec![(0, 8.0, 8.0, 600.0, 800.0)]);
    assert_eq!(model.page_rect(2), Some((8.0, 1624.0, 600.0, 800.0)));
    assert_eq!(model.page_rect(3), None);
    assert!(
        model.visible_tiles().is_empty(),
        "nothing has arrived: the paper shows"
    );
}

#[test]
fn viewer_the_first_plan_orders_visible_then_near_then_placeholders() {
    let mut model = model(&letter_pages(3), plain());
    let plan = model.plan();
    assert!(plan.cancel.is_empty());
    assert_eq!(
        summary(&plan),
        vec![
            // Page 0 shows: nearest the centre of the viewport first.
            (0, 0, 0, PRIORITY_VISIBLE),
            (0, 512, 0, PRIORITY_VISIBLE),
            (0, 0, 512, PRIORITY_VISIBLE),
            (0, 512, 512, PRIORITY_VISIBLE),
            // The margin of one viewport below reaches into page 1.
            (1, 0, 0, PRIORITY_NEAR),
            (1, 512, 0, PRIORITY_NEAR),
            // Placeholders last, the page that shows first.
            (0, 0, 0, PRIORITY_PLACEHOLDER),
            (1, 0, 0, PRIORITY_PLACEHOLDER),
        ]
    );
    let sizes: Vec<(u32, u32)> = plan
        .requests
        .iter()
        .map(|(request, _)| (request.width, request.height))
        .collect();
    assert_eq!(
        sizes,
        vec![
            (512, 512),
            (88, 512),
            (512, 288),
            (88, 288),
            (512, 512),
            (88, 512),
            (160, 213),
            (160, 213)
        ]
    );
    // The tiles are at the device scale; the placeholders at their own small one.
    assert_eq!(plan.requests[0].0.scale, 1.0);
    assert_eq!(plan.requests[6].0.scale, 0.266);
    // Links of the page that shows, no text (nobody pressed).
    assert_eq!(plan.links, vec![0]);
    assert!(plan.page_text.is_empty());
}

#[test]
fn viewer_nothing_is_requested_twice() {
    let mut shell = Shell::new(&letter_pages(3), plain());
    assert_eq!(shell.pump().requests.len(), 8);
    assert!(
        shell.pump().is_empty(),
        "the same plan again asks for nothing"
    );
    shell.model.set_scroll_y(0.0);
    shell.model.set_scroll_y(0.0);
    assert!(shell.pump().is_empty());
    // Delivered tiles are not asked for again either.
    shell.deliver_all();
    assert!(shell.pump().is_empty());
}

#[test]
fn viewer_a_scroll_cancels_what_scrolled_out_and_keeps_what_is_still_wanted() {
    let mut shell = Shell::new(&letter_pages(8), plain());
    let first = shell.pump();
    assert_eq!(first.requests.len(), 8);
    // Scroll down by three pages: nothing of page 0 and page 1 is wanted any more.
    shell.model.set_scroll_y(2500.0);
    let second = shell.pump();
    let cancelled: HashSet<(u32, u32, u32)> = second
        .cancel
        .iter()
        .map(|request| (request.page, request.x, request.y))
        .collect();
    // Page 0: 4 tiles and its placeholder; page 1: 2 tiles and its placeholder.
    assert_eq!(second.cancel.len(), 4 + 1 + 2 + 1);
    assert!(cancelled.contains(&(0, 0, 0)) && cancelled.contains(&(1, 512, 0)));
    // The new plan only asks for pages around the new position.
    assert!(
        second.requests.iter().all(|(request, _)| request.page >= 2),
        "{:?}",
        summary(&second)
    );
    // A small scroll that keeps most of the tiles in the margin cancels few, asks for few.
    shell.deliver_all();
    shell.settle();
    shell.model.scroll_by(100.0);
    let third = shell.pump();
    assert!(third.cancel.len() <= 4, "{}", third.cancel.len());
    assert!(third.requests.len() <= 6, "{}", third.requests.len());
}

#[test]
fn viewer_a_zoom_change_cancels_every_tile_of_the_old_zoom() {
    let mut shell = Shell::new(&letter_pages(3), plain());
    let first = shell.pump();
    let old_tiles: Vec<TileRequest> = first
        .requests
        .iter()
        .filter(|(_, priority)| *priority != PRIORITY_PLACEHOLDER)
        .map(|(request, _)| *request)
        .collect();
    assert_eq!(old_tiles.len(), 6);
    shell.model.set_zoom_percent(200);
    let second = shell.pump();
    for request in &old_tiles {
        assert!(
            second
                .cancel
                .iter()
                .any(|cancelled| key(cancelled) == key(request)),
            "{request:?} was not cancelled"
        );
    }
    // The placeholder of the page that still shows is zoom-independent: it stays wanted.
    assert!(
        !second
            .cancel
            .iter()
            .any(|request| request.page == 0 && request.scale == 0.266)
    );
    // Everything asked for now is at the new scale (2.0), except placeholders.
    for (request, priority) in &second.requests {
        if *priority != PRIORITY_PLACEHOLDER {
            assert_eq!(request.scale, 2.0);
        }
    }
    assert_eq!(shell.model.device_scale(), 2.0);
    // Rotating changes the tiles' orientation: all go again.
    shell.model.set_rotation(Rotation::Deg90);
    let third = shell.pump();
    assert!(!third.cancel.is_empty());
    assert!(
        third
            .requests
            .iter()
            .all(|(request, _)| request.rotation == Rotation::Deg90)
    );
}

#[test]
fn viewer_zoom_percent_is_clamped_to_the_zoom_range() {
    let mut model = model(&letter_pages(2), plain());
    for (asked, expected) in [(10, 30), (30, 30), (125, 125), (500, 500), (9999, 500)] {
        model.set_zoom_percent(asked);
        assert_eq!(model.zoom_percent(), expected, "{asked}");
    }
    model.set_zoom_percent(30);
    // 30 % of a fit that makes the page 600 wide: 180 pixels.
    assert_eq!(model.device_scale(), 0.3);
    assert_eq!(model.page_rects()[0].3, 180.0);
}

/// How far down its page the centre of the viewport is, in 0..1, and which page.
fn centre_of(model: &ViewerModel) -> (u32, f32) {
    let centre = model.scroll_y() + model.viewport().height / 2.0;
    for page in 0..model.page_count() {
        let (_, y, _, height) = model.page_rect(page).expect("a page");
        if centre < y + height {
            return (page, (centre - y) / height);
        }
    }
    (model.page_count() - 1, 1.0)
}

#[test]
fn viewer_zoom_keeps_the_point_at_the_centre_of_the_viewport() {
    let mut model = model(&letter_pages(20), plain());
    model.set_scroll_y(5000.0);
    let (page, fraction) = centre_of(&model);
    assert_eq!(page, 6);
    for percent in [50, 150, 200, 500, 30, 100, 333] {
        model.set_zoom_percent(percent);
        let (now_page, now_fraction) = centre_of(&model);
        assert_eq!(now_page, page, "{percent} %");
        assert!(
            (now_fraction - fraction).abs() < 0.002,
            "{percent} %: {now_fraction} != {fraction}"
        );
    }
}

#[test]
fn viewer_zoom_in_keeps_the_horizontal_centre_too() {
    let mut model = model(&letter_pages(3), plain());
    model.set_zoom_percent(300);
    // The page is 1800 wide in a content of 1816: the viewport's centre is the page's centre.
    let (width, _) = model.content_size();
    assert_eq!(width, 1816.0);
    assert_eq!(model.scroll_x(), (1816.0 - 616.0) / 2.0);
    // Scrolling sideways is bounded by the content; zooming out brings it back to the left edge.
    model.set_scroll_x(100_000.0);
    assert_eq!(model.scroll_x(), 1816.0 - 616.0);
    model.set_zoom_percent(100);
    assert_eq!(model.scroll_x(), 0.0);
}

#[test]
fn viewer_zoom_keeps_a_point_under_every_rotation() {
    for rotation in [
        Rotation::Deg0,
        Rotation::Deg90,
        Rotation::Deg180,
        Rotation::Deg270,
    ] {
        let mut model = model(&letter_pages(20), plain());
        model.set_rotation(rotation);
        model.set_scroll_y(5000.0);
        let before = centre_of(&model);
        model.set_zoom_percent(250);
        let after = centre_of(&model);
        assert_eq!(before.0, after.0, "{rotation:?}");
        assert!(
            (before.1 - after.1).abs() < 0.01,
            "{rotation:?} {before:?} {after:?}"
        );
    }
}

#[test]
fn viewer_a_new_viewport_relays_the_pages_out() {
    let mut model = model(&letter_pages(3), plain());
    // Wider window, fit width: the page grows to fit it.
    model.set_viewport(viewport(1016.0, 600.0, 1.0));
    // (1016 - 16) / 600 = 1.6667, kept in whole thousandths, rounded down: at 1.667 the page
    // would be 1001 pixels wide and the window would scroll sideways by one.
    assert_eq!(model.device_scale(), 1.666);
    assert_eq!(model.content_size().0, 1016.0);
    // Only the height changed: the scale and the scroll position stay.
    model.set_scroll_y(300.0);
    let scale = model.device_scale();
    model.set_viewport(viewport(1016.0, 900.0, 1.0));
    assert_eq!(model.device_scale(), scale);
    assert_eq!(model.scroll_y(), 300.0);
    // Garbage in the viewport is repaired, never a panic.
    model.set_viewport(viewport(f32::NAN, -1.0, 0.0));
    assert!(model.content_size().0 > 0.0);
}

#[test]
fn viewer_the_scroll_position_stays_inside_the_content() {
    let mut model = model(&letter_pages(3), plain());
    // Content is 2432 tall, the viewport 600.
    model.set_scroll_y(1.0e9);
    assert_eq!(model.scroll_y(), 1832.0);
    model.set_scroll_y(-50.0);
    assert_eq!(model.scroll_y(), 0.0);
    model.scroll_by(700.0);
    model.scroll_by(-200.0);
    assert_eq!(model.scroll_y(), 500.0);
    model.set_scroll_y(f32::NAN);
    model.scroll_by(f32::INFINITY);
    assert_eq!(model.scroll_y(), 500.0, "not a number: ignored");
    model.set_scroll_y(f32::MAX);
    assert_eq!(model.scroll_y(), 1832.0);
}

#[test]
fn viewer_fit_page_makes_the_current_page_fit_the_window() {
    let mut model = model(&letter_pages(3), viewport(616.0, 616.0, 1.0));
    model.set_fit(Fit::Page);
    // 600 x 800 into 600 x 600: scale 0.75, a page of 450 x 600 centred.
    assert_eq!(model.device_scale(), 0.75);
    assert_eq!(model.page_rect(0), Some((83.0, 8.0, 450.0, 600.0)));
    assert_eq!(
        model.content_size(),
        (616.0, 8.0 + 3.0 * 600.0 + 2.0 * 8.0 + 8.0)
    );
    // Scrolling does not zoom, even onto a page of another size.
    model.set_scroll_y(700.0);
    assert_eq!(model.device_scale(), 0.75);
    // Fit width again: back to the widest page.
    model.set_fit(Fit::Width);
    assert_eq!(model.device_scale(), 1.0);
}

#[test]
fn viewer_fit_page_follows_a_jump_to_another_page() {
    let mut model = model(
        &[(600.0, 800.0), (800.0, 300.0), (600.0, 800.0)],
        viewport(616.0, 616.0, 1.0),
    );
    model.set_fit(Fit::Page);
    assert_eq!(model.device_scale(), 0.75);
    // The landscape page: 800 wide into 600: 0.75 as well; a wider-than-high page, same fit.
    model.go_to_page(1, None);
    assert_eq!(model.current_page(), 1);
    assert_eq!(model.device_scale(), 0.75);
    // A shorter window makes the height the tight side for the portrait page.
    model.set_viewport(viewport(616.0, 316.0, 1.0));
    model.go_to_page(2, None);
    assert_eq!(
        model.device_scale(),
        0.375,
        "(316 - 16) / 800; the width would allow 1"
    );
}

#[test]
fn viewer_go_to_page_puts_the_page_at_the_top() {
    let mut model = model(&letter_pages(20), plain());
    model.go_to_page(5, None);
    assert_eq!(
        model.scroll_y(),
        4040.0,
        "page 5 starts at 4048, 8 below the top"
    );
    assert_eq!(model.current_page(), 5);
    model.go_to_page(5, Some(100.0));
    assert_eq!(model.scroll_y(), 4140.0);
    model.go_to_page(0, None);
    assert_eq!(model.scroll_y(), 0.0);
    assert_eq!(model.current_page(), 0);
    // Past the end: the last page, as far as the content scrolls.
    model.go_to_page(999, None);
    assert_eq!(model.current_page(), 19);
    assert_eq!(model.scroll_y(), 15_352.0);
    // A destination that is not a number is the top of the page.
    model.go_to_page(2, Some(f32::NAN));
    assert_eq!(model.scroll_y(), 1616.0, "page 2 starts at 1624");
}

#[test]
fn viewer_the_current_page_is_the_first_page_of_which_half_shows() {
    let mut model = model(&letter_pages(20), plain());
    assert_eq!(model.current_page(), 0);
    model.scroll_by(300.0);
    assert_eq!(
        model.current_page(),
        0,
        "page 0 still shows 508 of 600 rows"
    );
    model.scroll_by(300.0);
    assert_eq!(model.current_page(), 1, "page 1 shows 384, page 0 only 208");
    model.set_scroll_y(0.0);
    assert_eq!(model.current_page(), 0);
    // A tall window sees several pages whole: the current one stays while it is whole.
    let mut tall = self::model(&letter_pages(20), viewport(616.0, 2000.0, 1.0));
    tall.go_to_page(3, None);
    assert_eq!(tall.current_page(), 3);
    tall.scroll_by(40.0);
    assert_eq!(
        tall.current_page(),
        3,
        "page 3 is not whole any more, but more than half of it shows"
    );
    // Zoomed far out, going to page 5 reads 5 even though 6, 7 and 8 show as much.
    let mut small = self::model(&letter_pages(20), plain());
    small.set_zoom_percent(30);
    small.go_to_page(5, None);
    assert_eq!(small.current_page(), 5);
    small.scroll_by(1.0);
    assert_eq!(small.current_page(), 5);
}

#[test]
fn viewer_visible_tiles_fall_back_from_the_exact_tile_to_the_placeholder_to_nothing() {
    let mut model = model(&letter_pages(3), plain());
    let plan = model.plan();
    let tile = |index: usize| plan.requests[index].0;
    let placeholder = tile(6);
    assert_eq!(placeholder.scale, 0.266);
    // Nothing arrived: the paper shows.
    assert!(model.visible_tiles().is_empty());
    // The placeholder of page 0 arrives: it is the whole page, stretched.
    model.tile_arrived(&placeholder, tile_for(&placeholder));
    let shown = model.visible_tiles();
    assert_eq!(shown.len(), 1);
    assert!(shown[0].is_placeholder);
    assert_eq!(shown[0].page, 0);
    assert_eq!(
        (shown[0].x, shown[0].y, shown[0].width, shown[0].height),
        (8.0, 8.0, 600.0, 800.0)
    );
    assert_eq!((shown[0].pixels.width, shown[0].pixels.height), (160, 213));
    // One exact tile arrives: drawn over the placeholder, which stays under the rest.
    model.tile_arrived(&tile(0), tile_for(&tile(0)));
    let shown = model.visible_tiles();
    assert_eq!(shown.len(), 2);
    assert!(shown[0].is_placeholder && !shown[1].is_placeholder);
    assert_eq!(
        (shown[1].x, shown[1].y, shown[1].width, shown[1].height),
        (8.0, 8.0, 512.0, 512.0)
    );
    // The last exact tile of the page arrives: no placeholder is needed.
    for index in 1..4 {
        model.tile_arrived(&tile(index), tile_for(&tile(index)));
    }
    let shown = model.visible_tiles();
    assert_eq!(shown.len(), 4);
    assert!(shown.iter().all(|tile| !tile.is_placeholder));
    assert_eq!(
        shown
            .iter()
            .map(|tile| (tile.x, tile.y))
            .collect::<Vec<_>>(),
        vec![(8.0, 8.0), (520.0, 8.0), (8.0, 520.0), (520.0, 520.0)]
    );
}

#[test]
fn viewer_after_a_zoom_the_placeholder_shows_until_the_new_tiles_arrive() {
    let mut shell = Shell::new(&letter_pages(3), plain());
    shell.settle();
    assert!(
        shell
            .model
            .visible_tiles()
            .iter()
            .all(|tile| !tile.is_placeholder)
    );
    shell.model.set_zoom_percent(150);
    let shown = shell.model.visible_tiles();
    assert!(
        !shown.is_empty(),
        "the old tiles are not drawn but the placeholder is"
    );
    assert!(shown.iter().all(|tile| tile.is_placeholder));
    shell.settle();
    let shown = shell.model.visible_tiles();
    assert!(shown.iter().all(|tile| !tile.is_placeholder));
    assert!(!shown.is_empty());
}

#[test]
fn viewer_a_failed_tile_is_not_asked_for_again_until_the_zoom_changes() {
    let mut model = model(&letter_pages(3), plain());
    let plan = model.plan();
    let failed = plan.requests[1].0;
    model.tile_failed(&failed);
    assert!(model.plan().is_empty(), "no retry loop");
    model.set_scroll_y(1.0);
    model.set_scroll_y(0.0);
    assert!(
        model
            .plan()
            .requests
            .iter()
            .all(|(request, _)| (request.page, request.x, request.y) != (0, failed.x, failed.y))
    );
    // A new zoom starts afresh.
    model.set_zoom_percent(110);
    let again = model.plan();
    assert!(!again.requests.is_empty());
    // A tile that arrives with the wrong pixels counts as failed too.
    let broken = again.requests[0].0;
    model.tile_arrived(
        &broken,
        Tile {
            width: broken.width,
            height: broken.height,
            rgba: vec![0; 3],
        },
    );
    assert!(
        model
            .plan()
            .requests
            .iter()
            .all(|(request, _)| key(request) != key(&broken))
    );
    // Forgetting what was pending (a helper restart) asks for everything again.
    model.forget_pending();
    assert!(
        model
            .plan()
            .requests
            .iter()
            .any(|(request, _)| key(request) == key(&broken))
    );
}

#[test]
fn viewer_a_late_tile_for_an_old_zoom_is_dropped_but_a_late_current_one_is_kept() {
    let mut shell = Shell::new(&letter_pages(3), plain());
    let plan = shell.pump();
    let old = plan.requests[0].0;
    shell.model.set_zoom_percent(200);
    // The old tile answers anyway: nothing breaks and nothing of it is drawn.
    shell.model.tile_arrived(&old, tile_for(&old));
    assert!(shell.model.visible_tiles().is_empty());
    let new_plan = shell.pump();
    let wanted = new_plan.requests[0].0;
    assert_eq!(wanted.scale, 2.0);
    // Scrolled far away, it is cancelled; it answers late; scrolled back, it is not asked for
    // again, because the late answer was kept.
    shell.model.set_scroll_y(1.0e9);
    let far = shell.pump();
    assert!(
        far.cancel
            .iter()
            .any(|request| key(request) == key(&wanted))
    );
    shell.model.tile_arrived(&wanted, tile_for(&wanted));
    shell.model.set_scroll_y(0.0);
    let back = shell.pump();
    assert!(
        !back
            .requests
            .iter()
            .any(|(request, _)| key(request) == key(&wanted))
    );
}

#[test]
fn viewer_device_pixel_ratio_scales_tiles_not_the_layout() {
    let mut model = model(&letter_pages(2), viewport(616.0, 600.0, 2.0));
    assert_eq!(model.device_scale(), 2.0);
    assert_eq!(model.page_rect(0), Some((8.0, 8.0, 600.0, 800.0)));
    let plan = model.plan();
    // A page of 1200 x 1600 pixels: 3 x 4 tiles; the viewport shows 600 x 1200 pixels... of
    // device 1232 x 1200: 3 columns and 3 rows (0..1200 of 1600) show.
    let visible: Vec<_> = plan
        .requests
        .iter()
        .filter(|(_, priority)| *priority == PRIORITY_VISIBLE)
        .collect();
    assert_eq!(visible.len(), 9);
    assert!(visible.iter().all(|(request, _)| request.scale == 2.0));
    // The tile nearest the centre comes first; a 512-pixel tile is 256 logical pixels, and the
    // page starts 16 device pixels (8 logical) in.
    let nearest = plan.requests[0].0;
    assert_eq!((nearest.x, nearest.y), (512, 512));
    model.tile_arrived(&nearest, tile_for(&nearest));
    let shown = model.visible_tiles();
    assert_eq!(
        (shown[0].x, shown[0].y, shown[0].width, shown[0].height),
        (264.0, 264.0, 256.0, 256.0)
    );
}

#[test]
fn viewer_rotation_swaps_the_page_and_asks_for_rotated_tiles() {
    let mut model = model(&[(600.0, 800.0)], viewport(816.0, 616.0, 1.0));
    model.set_rotation(Rotation::Deg90);
    // Fit width: the widest page as displayed is 800 points, so the scale stays 1 here.
    assert_eq!(model.device_scale(), 1.0);
    assert_eq!(model.page_rect(0), Some((8.0, 8.0, 800.0, 600.0)));
    let plan = model.plan();
    assert!(
        plan.requests
            .iter()
            .all(|(request, _)| request.rotation == Rotation::Deg90)
    );
    // The rotated tiles are on the rotated page's grid: 800 x 600 pixels.
    let max_x = plan
        .requests
        .iter()
        .map(|(request, _)| request.x + request.width)
        .max();
    assert_eq!(max_x, Some(800));
}

#[test]
fn viewer_text_is_asked_for_only_where_the_selection_needs_it_and_once() {
    let mut model = model(&letter_pages(5), plain());
    assert!(model.plan().page_text.is_empty(), "nobody pressed");
    // A press on page 1 (it starts at 816): its text, once.
    model.set_scroll_y(1000.0);
    model.pointer_down(100.0, 1100.0, 1);
    assert_eq!(model.plan().page_text, vec![1]);
    assert!(model.plan().page_text.is_empty());
    // Dragging to page 3 asks for page 3 and the page between, not page 1 again.
    model.pointer_drag(100.0, 8.0 + 808.0 * 3.0 + 50.0);
    assert_eq!(model.plan().page_text, vec![3, 2]);
    // The text arrives; nothing is asked twice, even after the plan runs again.
    model.set_page_text(1, PageText::default());
    model.set_page_text(2, PageText::default());
    model.set_page_text(3, PageText::default());
    assert!(model.plan().page_text.is_empty());
    // A new press elsewhere asks for its page, which is new.
    model.pointer_down(100.0, 50.0, 1);
    assert_eq!(model.plan().page_text, vec![0]);
    // A press in the gap goes to the nearer page.
    model.pointer_down(100.0, 8.0 + 800.0 + 6.0, 1);
    assert_eq!(
        model.plan().page_text,
        Vec::<u32>::new(),
        "page 1 is already known"
    );
}

#[test]
fn viewer_links_are_asked_for_once_for_pages_that_show() {
    let mut model = model(&letter_pages(5), plain());
    assert_eq!(model.plan().links, vec![0]);
    assert!(model.plan().links.is_empty());
    model.set_links(0, Vec::new());
    model.set_scroll_y(1000.0);
    // Viewport 1000..1600: page 1 (816..1616) only.
    assert_eq!(model.plan().links, vec![1]);
    model.set_links(1, Vec::new());
    model.set_scroll_y(0.0);
    assert!(model.plan().links.is_empty(), "page 0 is already known");
    // A taller view of two pages asks for the one it has not seen.
    model.set_viewport(viewport(616.0, 1400.0, 1.0));
    assert_eq!(model.plan().links, Vec::<u32>::new());
    model.set_scroll_y(1000.0);
    assert_eq!(model.plan().links, vec![2]);
}

fn glyph(ch: char, x: f32, top: f32) -> TextChar {
    TextChar {
        ch,
        bounds: Rect::new(x, top, x + 6.0, top + 10.0),
    }
}

fn hello_line() -> PageText {
    PageText {
        chars: "Hello world"
            .chars()
            .enumerate()
            .map(|(index, ch)| glyph(ch, 10.0 + index as f32 * 6.0, 100.0))
            .collect(),
    }
}

#[test]
fn viewer_a_drag_selects_text_and_highlights_it_in_content_space() {
    let mut model = model(&letter_pages(3), plain());
    // Page 0 is at (8, 8) at scale 1: page space (10.5, 105) is content (18.5, 113).
    model.pointer_down(18.5, 113.0, 1);
    assert!(!model.has_selection());
    assert_eq!(model.plan().page_text, vec![0]);
    model.set_page_text(0, hello_line());
    model.pointer_drag(8.0 + 40.5, 113.0);
    model.pointer_up();
    assert_eq!(model.selected_text().as_deref(), Some("Hello"));
    assert!(model.has_selection());
    assert!(model.selection_complete());
    // "Hello" is page-space x 10..40, y 100..110.
    assert_eq!(model.selection_rects(), vec![(18.0, 108.0, 30.0, 10.0)]);
    // Scrolled so that the line is above the viewport, there is nothing to draw.
    model.set_scroll_y(300.0);
    assert!(model.selection_rects().is_empty());
    assert_eq!(model.selected_text().as_deref(), Some("Hello"));
    model.clear_selection();
    assert_eq!(model.selected_text(), None);
    assert!(model.selection_rects().is_empty());
}

#[test]
fn viewer_a_press_resolves_when_the_text_arrives_after_the_drag() {
    let mut model = model(&letter_pages(3), plain());
    model.pointer_down(18.5, 113.0, 1);
    model.pointer_drag(8.0 + 200.0, 113.0);
    model.pointer_up();
    assert_eq!(model.selected_text(), None, "no text yet");
    model.set_page_text(0, hello_line());
    assert_eq!(model.selected_text().as_deref(), Some("Hello world"));
}

#[test]
fn viewer_double_and_triple_clicks_select_a_word_and_a_line() {
    let mut model = model(&letter_pages(1), plain());
    model.set_page_text(0, hello_line());
    // 'w' of "world" is page-space x 46..52.
    model.pointer_down(8.0 + 48.0, 113.0, 2);
    model.pointer_up();
    assert_eq!(model.selected_text().as_deref(), Some("world"));
    model.pointer_down(8.0 + 48.0, 113.0, 3);
    model.pointer_up();
    assert_eq!(model.selected_text().as_deref(), Some("Hello world"));
}

#[test]
fn viewer_the_selection_follows_the_rotation() {
    let mut model = model(&letter_pages(1), plain());
    model.set_page_text(0, hello_line());
    model.pointer_down(8.0 + 10.5, 113.0, 1);
    model.pointer_drag(8.0 + 40.5, 113.0);
    model.pointer_up();
    assert_eq!(model.selection_rects(), vec![(18.0, 108.0, 30.0, 10.0)]);
    // A quarter turn: fit width now scales the 800-point side to the window (0.75), and the
    // selection moves with the text: the line is now vertical, near the right edge.
    model.set_rotation(Rotation::Deg90);
    let rects = model.selection_rects();
    assert_eq!(rects.len(), 1);
    let (x, y, width, height) = rects[0];
    // Page space x 10..40, y 100..110 becomes display x (800 - 110)..(800 - 100), y 10..40, at 0.75
    // from the page's origin (8, 8).
    for (got, expected) in [
        (x, 8.0 + 690.0 * 0.75),
        (y, 8.0 + 10.0 * 0.75),
        (width, 7.5),
        (height, 22.5),
    ] {
        assert!(
            (got - expected).abs() < 0.05,
            "{got} != {expected}: {rects:?}"
        );
    }
    // Pressing on that rotated line selects the same text.
    model.pointer_down(x + 3.0, y + 1.0, 3);
    model.pointer_up();
    assert_eq!(model.selected_text().as_deref(), Some("Hello world"));
}

#[test]
fn viewer_links_are_found_under_the_pointer_smallest_first() {
    let mut model = model(&letter_pages(3), plain());
    let outer = Link {
        rect: Rect::new(50.0, 50.0, 150.0, 70.0),
        target: LinkTarget::Uri("https://example.com/".to_owned()),
    };
    let inner = Link {
        rect: Rect::new(60.0, 55.0, 100.0, 65.0),
        target: LinkTarget::Page {
            page: 2,
            y: Some(40.0),
        },
    };
    assert_eq!(model.link_at(70.0, 70.0), None, "no links have arrived");
    model.set_links(0, vec![outer.clone(), inner.clone()]);
    // Page space (70, 60) is content (78, 68): inside both; the smaller one wins.
    assert_eq!(model.link_at(78.0, 68.0), Some(inner.target.clone()));
    assert_eq!(model.link_at(8.0 + 140.0, 68.0), Some(outer.target.clone()));
    assert_eq!(model.link_at(8.0 + 300.0, 8.0 + 300.0), None);
    assert_eq!(model.link_at(3.0, 68.0), None, "left of the page");
    assert_eq!(model.link_at(78.0, 812.0), None, "in the gap");
    assert_eq!(
        model.link_at(78.0, 816.0 + 68.0),
        None,
        "page 1 has no links yet"
    );
    assert_eq!(model.link_at(f32::NAN, 1.0), None);
    // The edges: the left and top count, the right and bottom do not.
    assert_eq!(
        model.link_at(8.0 + 50.0, 8.0 + 50.0),
        Some(outer.target.clone())
    );
    assert_eq!(model.link_at(8.0 + 150.0, 8.0 + 60.0), None);
}

fn hit(page: u32, rect: Rect) -> SearchHit {
    SearchHit {
        page,
        char_start: 0,
        char_len: 3,
        rects: vec![rect],
    }
}

#[test]
fn viewer_find_scrolls_to_the_first_hit_and_steps_through_the_rest() {
    let mut model = model(&letter_pages(20), plain());
    let mine = model.find_start("hello", FindOptions::default());
    assert_ne!(mine, 0);
    // The client's own id is what its answers carry.
    model.find_bind(42);
    assert!(model.find_is_active());
    assert_eq!(model.find_query(), "hello");
    assert_eq!(model.find_status().current, 0);
    assert!(model.find_status().searching());
    // A page far away has the first answer: the view goes there, the hit centred.
    let on_seven = vec![hit(7, Rect::new(10.0, 200.0, 50.0, 212.0))];
    assert!(model.find_hits(42, 7, on_seven, false));
    assert_eq!(model.scroll_y(), 5864.0 + 6.0 - 300.0);
    assert_eq!(model.current_page(), 7);
    assert_eq!(model.find_rects(), vec![(18.0, 5864.0, 40.0, 12.0, true)]);
    let status = model.find_status();
    assert_eq!(
        (status.current, status.total, status.finished),
        (1, 1, false)
    );
    // A hit on an earlier page arrives: counted before the current one, the view stays.
    let scroll = model.scroll_y();
    let on_three = vec![hit(3, Rect::new(10.0, 100.0, 50.0, 112.0))];
    assert!(model.find_hits(42, 3, on_three, true));
    assert_eq!(model.scroll_y(), scroll);
    let status = model.find_status();
    assert_eq!(
        (status.current, status.total, status.finished),
        (2, 2, true)
    );
    // Next wraps to page 3, previous goes back, each scrolling to the hit.
    assert!(model.find_next());
    assert_eq!(model.find_status().current, 1);
    assert_eq!(model.current_page(), 3);
    assert!(
        model.find_rects().iter().any(|rect| rect.4),
        "the current one is marked"
    );
    assert!(model.find_previous());
    assert_eq!(model.find_status().current, 2);
    assert_eq!(model.current_page(), 7);
    // A late answer of the first (the model's own) id is ignored.
    assert!(!model.find_hits(mine, 9, vec![hit(9, Rect::new(0.0, 0.0, 5.0, 5.0))], true));
    assert_eq!(model.find_status().total, 2);
    // Clearing removes the highlights and the answers still coming.
    model.find_clear();
    assert!(model.find_rects().is_empty());
    assert!(!model.find_is_active());
    assert!(!model.find_hits(42, 5, vec![hit(5, Rect::new(0.0, 0.0, 5.0, 5.0))], true));
    assert!(!model.find_next());
}

#[test]
fn viewer_find_highlights_every_hit_that_shows_and_marks_the_current_one() {
    let mut model = model(&letter_pages(3), plain());
    let id = model.find_start("a", FindOptions::default());
    let hits = [
        hit(0, Rect::new(10.0, 100.0, 50.0, 112.0)),
        SearchHit {
            char_start: 9,
            ..hit(0, Rect::new(10.0, 300.0, 50.0, 312.0))
        },
        // On page 1, below the viewport.
        hit(1, Rect::new(10.0, 10.0, 50.0, 22.0)),
    ];
    assert!(model.find_hits(id, 0, hits[..2].to_vec(), false));
    assert!(model.find_hits(id, 1, hits[2..].to_vec(), true));
    assert_eq!(model.find_status().total, 3);
    assert_eq!(
        model.find_rects(),
        vec![
            (18.0, 108.0, 40.0, 12.0, true),
            (18.0, 308.0, 40.0, 12.0, false)
        ]
    );
    assert!(model.find_next());
    assert_eq!(
        model.find_rects(),
        vec![
            (18.0, 108.0, 40.0, 12.0, false),
            (18.0, 308.0, 40.0, 12.0, true)
        ]
    );
    // A hit already in view does not scroll the view.
    assert_eq!(model.scroll_y(), 0.0);
    // The third is on the next page: the view moves to it.
    assert!(model.find_next());
    assert!(model.scroll_y() > 0.0);
    assert_eq!(model.current_page(), 1);
}

#[test]
fn viewer_an_empty_find_is_finished_with_no_hits() {
    let mut model = model(&letter_pages(2), plain());
    model.find_start("", FindOptions::default());
    let status = model.find_status();
    assert_eq!(
        (status.current, status.total, status.finished),
        (0, 0, true)
    );
    assert!(!model.find_next());
}

#[test]
fn viewer_twenty_thousand_pages_cost_what_three_cost() {
    let mut shell = Shell::new(&letter_pages(20_000), plain());
    assert_eq!(shell.model.page_count(), 20_000);
    let first = shell.pump();
    assert_eq!(first.requests.len(), 8, "the same plan as for three pages");
    shell.model.go_to_page(10_000, None);
    assert_eq!(shell.model.current_page(), 10_000);
    let middle = shell.pump();
    assert!(middle.requests.len() <= 40, "{}", middle.requests.len());
    assert!(
        middle
            .requests
            .iter()
            .all(|(request, _)| request.page >= 9_999)
    );
    assert_eq!(shell.model.page_rects().len(), 1);
    shell.model.set_scroll_y(f32::MAX);
    assert_eq!(shell.model.current_page(), 19_999);
    shell.pump();
    shell.deliver_all();
    shell.settle();
    shell.model.set_zoom_percent(30);
    assert!(shell.model.page_rects().len() >= 3);
    shell.pump();
}

#[test]
fn viewer_a_huge_page_and_a_tiny_page_ask_for_sane_tiles() {
    let pages = [(50_000.0, 50_000.0), (1.0, 1.0), (0.25, 0.25)];
    let mut shell = Shell::new(&pages, plain());
    shell.model.set_zoom_percent(500);
    let plan = shell.pump();
    assert!(plan.requests.len() < 60, "{}", plan.requests.len());
    shell.model.go_to_page(1, None);
    shell.pump();
    shell.model.go_to_page(2, None);
    shell.pump();
    shell.settle();
}

fn check_drawing(model: &ViewerModel, all_delivered: bool) {
    let view = model.viewport();
    let (left, top) = (f64::from(model.scroll_x()), f64::from(model.scroll_y()));
    let (right, bottom) = (left + f64::from(view.width), top + f64::from(view.height));
    let pages = model.page_rects();
    let shown = model.visible_tiles();
    let epsilon = 0.01;
    // Every placed tile belongs to a page that shows and lies inside its paper.
    for tile in &shown {
        let (_, px, py, pw, ph) = *pages
            .iter()
            .find(|page| page.0 == tile.page)
            .unwrap_or_else(|| panic!("a tile of page {} that does not show", tile.page));
        assert!(
            f64::from(tile.x) >= f64::from(px) - epsilon,
            "{tile:?} {pages:?}"
        );
        assert!(f64::from(tile.y) >= f64::from(py) - epsilon);
        assert!(f64::from(tile.x + tile.width) <= f64::from(px + pw) + epsilon);
        assert!(f64::from(tile.y + tile.height) <= f64::from(py + ph) + epsilon);
        if tile.is_placeholder {
            assert!((tile.width - pw).abs() < 0.01 && (tile.height - ph).abs() < 0.01);
        }
    }
    // Exact tiles of a page never overlap one another.
    let exact: Vec<_> = shown.iter().filter(|tile| !tile.is_placeholder).collect();
    for (index, a) in exact.iter().enumerate() {
        for b in &exact[index + 1..] {
            if a.page != b.page {
                continue;
            }
            let apart = a.x + a.width <= b.x + 0.01
                || b.x + b.width <= a.x + 0.01
                || a.y + a.height <= b.y + 0.01
                || b.y + b.height <= a.y + 0.01;
            assert!(apart, "{a:?} overlaps {b:?}");
        }
    }
    if !all_delivered {
        return;
    }
    // With everything delivered, the tiles cover exactly the part of each page that shows.
    for (page, px, py, pw, ph) in pages {
        let visible_w = (f64::from(px + pw).min(right) - f64::from(px).max(left)).max(0.0);
        let visible_h = (f64::from(py + ph).min(bottom) - f64::from(py).max(top)).max(0.0);
        if visible_w < 0.5 || visible_h < 0.5 {
            continue;
        }
        let covered: f64 = exact
            .iter()
            .filter(|tile| tile.page == page)
            .map(|tile| {
                let w = (f64::from(tile.x + tile.width).min(right) - f64::from(tile.x).max(left))
                    .max(0.0);
                let h = (f64::from(tile.y + tile.height).min(bottom) - f64::from(tile.y).max(top))
                    .max(0.0);
                w * h
            })
            .sum();
        let wanted = visible_w * visible_h;
        assert!(
            (covered - wanted).abs() <= 0.01 * wanted + 1.0,
            "page {page}: tiles cover {covered} of the {wanted} that shows"
        );
    }
}

#[test]
fn viewer_random_sessions_keep_every_rule_of_the_contract() {
    for seed in 1..=40_u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let count = rng.between(1, 40) as usize;
        let pages: Vec<(f32, f32)> = (0..count)
            .map(|_| (rng.between(100, 1200) as f32, rng.between(100, 1600) as f32))
            .collect();
        let factor = [1.0, 1.5, 2.0][rng.below(3) as usize];
        let mut shell = Shell::new(
            &pages,
            viewport(
                rng.between(300, 1400) as f32,
                rng.between(300, 1000) as f32,
                factor,
            ),
        );
        for step in 0..80 {
            match rng.below(10) {
                0 => {
                    let (_, height) = shell.model.content_size();
                    shell
                        .model
                        .set_scroll_y(rng.below(height as u64 + 1) as f32);
                }
                1 => shell.model.scroll_by(rng.between(0, 3000) as f32 - 1500.0),
                2 => shell.model.set_zoom_percent(rng.between(10, 600) as u16),
                3 => shell
                    .model
                    .set_rotation(Rotation::from_quarter_turns(rng.below(4) as i32)),
                4 => shell.model.set_fit(if rng.below(2) == 0 {
                    Fit::Width
                } else {
                    Fit::Page
                }),
                5 => shell.model.set_viewport(viewport(
                    rng.between(300, 1400) as f32,
                    rng.between(300, 1000) as f32,
                    [1.0, 1.5, 2.0][rng.below(3) as usize],
                )),
                6 => shell.model.go_to_page(
                    rng.below(count as u64 + 2) as u32,
                    (rng.below(2) == 0).then(|| rng.below(1000) as f32),
                ),
                7 | 8 => shell.deliver_some(&mut rng),
                _ => {
                    let (width, _) = shell.model.content_size();
                    shell.model.set_scroll_x(rng.below(width as u64 + 1) as f32);
                }
            }
            shell.pump();
            check_drawing(&shell.model, false);
            assert!(
                shell.model.current_page() < count as u32,
                "seed {seed} step {step}"
            );
        }
        shell.settle();
        check_drawing(&shell.model, true);
        assert!(
            shell.pump().is_empty(),
            "seed {seed}: settled but still planning"
        );
        assert!(shell.outstanding.is_empty());
    }
}

#[test]
fn viewer_at_high_zoom_only_the_columns_in_view_are_asked_for() {
    let mut shell = Shell::new(&[(600.0, 800.0)], plain());
    shell.model.set_zoom_percent(500);
    // 3000 x 4000 pixels: 6 columns. The viewport (616 wide) is over the middle of the page.
    let columns = |plan: &Plan| -> Vec<u32> {
        let mut columns: Vec<u32> = plan
            .requests
            .iter()
            .filter(|(_, priority)| *priority != PRIORITY_PLACEHOLDER)
            .map(|(request, _)| request.x / 512)
            .collect();
        columns.sort_unstable();
        columns.dedup();
        columns
    };
    let plan = shell.pump();
    let visible = columns(&plan);
    assert!(visible.len() <= 3, "{visible:?}");
    assert!(!visible.contains(&0) || !visible.contains(&5));
    // At the far right edge it is the last columns, and the old ones are cancelled.
    shell.model.set_scroll_x(1.0e9);
    let plan = shell.pump();
    let right = columns(&plan);
    assert!(right.contains(&5), "{right:?}");
    assert!(!right.contains(&0));
    assert!(!plan.cancel.is_empty());
    // What shows is drawn at the right place: the last column ends at the page's right edge.
    shell.settle();
    let shown = shell.model.visible_tiles();
    let edge = shown
        .iter()
        .filter(|tile| !tile.is_placeholder)
        .map(|tile| tile.x + tile.width)
        .fold(0.0_f32, f32::max);
    assert_eq!(edge, 3008.0);
    assert_eq!(shell.model.scroll_x(), shell.model.content_size().0 - 616.0);
}

#[test]
fn viewer_a_document_with_no_pages_does_nothing_and_does_not_panic() {
    let mut model = ViewerModel::new(DocumentInfo {
        pages: Vec::new(),
        title: None,
    });
    model.set_viewport(plain());
    model.set_zoom_percent(300);
    model.set_rotation(Rotation::Deg90);
    model.set_fit(Fit::Page);
    model.go_to_page(3, Some(10.0));
    model.set_scroll_y(100.0);
    model.pointer_down(10.0, 10.0, 1);
    model.pointer_drag(50.0, 50.0);
    model.pointer_up();
    assert_eq!(model.page_count(), 0);
    assert!(model.plan().requests.is_empty());
    assert!(model.visible_tiles().is_empty());
    assert!(model.page_rects().is_empty());
    assert_eq!(model.selected_text(), None);
    assert_eq!(model.link_at(10.0, 10.0), None);
    assert_eq!(model.scroll_y(), 0.0);
    let id = model.find_start("x", FindOptions::default());
    assert!(model.find_hits(id, 0, Vec::new(), true));
    assert_eq!(model.find_status().total, 0);
    assert!(!model.find_next());
}

#[test]
fn viewer_links_follow_the_view_rotation() {
    let mut model = model(&[(600.0, 800.0)], viewport(816.0, 616.0, 1.0));
    let target = LinkTarget::Uri("https://example.com/".to_owned());
    model.set_links(
        0,
        vec![Link {
            rect: Rect::new(50.0, 50.0, 150.0, 70.0),
            target: target.clone(),
        }],
    );
    // Turned a quarter, the page is 800 x 600 at (8, 8): page-space (x, y) is display
    // (800 - y, x), so the link covers display x 730..750, y 50..150.
    model.set_rotation(Rotation::Deg90);
    assert_eq!(model.link_at(8.0 + 740.0, 8.0 + 100.0), Some(target));
    assert_eq!(
        model.link_at(8.0 + 100.0, 8.0 + 60.0),
        None,
        "where it was unturned"
    );
}

#[test]
fn viewer_a_change_of_device_pixel_ratio_asks_for_the_tiles_again() {
    let mut shell = Shell::new(&letter_pages(2), viewport(616.0, 600.0, 1.0));
    shell.settle();
    shell.model.set_viewport(viewport(616.0, 600.0, 2.0));
    assert_eq!(shell.model.device_scale(), 2.0);
    let plan = shell.pump();
    assert!(!plan.cancel.is_empty() || !plan.requests.is_empty());
    assert!(
        plan.requests
            .iter()
            .filter(|(_, priority)| *priority != PRIORITY_PLACEHOLDER)
            .all(|(request, _)| request.scale == 2.0)
    );
    shell.settle();
    check_drawing(&shell.model, true);
}

#[test]
fn viewer_a_window_resize_keeps_the_point_at_the_centre_and_the_model_can_move_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<ViewerModel>();
    assert_send::<Plan>();

    let mut model = model(&letter_pages(20), plain());
    model.set_scroll_y(5000.0);
    let (page, fraction) = centre_of(&model);
    // A wider window at fit width makes the pages larger: the centre stays on the same place.
    model.set_viewport(viewport(1216.0, 700.0, 1.0));
    let (now_page, now_fraction) = centre_of(&model);
    assert_eq!(now_page, page);
    assert!(
        (now_fraction - fraction).abs() < 0.002,
        "{now_fraction} != {fraction}"
    );
    // A taller window at the same width does not move the top.
    let top = model.scroll_y();
    model.set_viewport(viewport(1216.0, 900.0, 1.0));
    assert_eq!(model.scroll_y(), top);
}

#[test]
fn viewer_hits_on_pages_that_do_not_exist_are_ignored() {
    let mut model = model(&letter_pages(2), plain());
    let id = model.find_start("a", FindOptions::default());
    assert!(model.find_hits(id, 9, vec![hit(9, Rect::new(0.0, 0.0, 5.0, 5.0))], true));
    let status = model.find_status();
    assert_eq!((status.total, status.finished), (0, true));
}
