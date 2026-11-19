// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The viewer model's page layout and tile grid, through the public API only.

use mercury_pdf::layout::{
    Fit, GAP, Geometry, Layout, MARGIN, PxRect, TILE_SIDE, Viewport, clamp_percent, device_milli,
    placeholder_request,
};
use mercury_pdf::types::{PageSize, Rotation, limits};

/// A small seeded generator (xorshift64*), so the property checks are reproducible.
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

fn geometry(milli: u32, factor: f32, rotation: Rotation, width: f32) -> Geometry {
    Geometry {
        scale_milli: milli,
        scale_factor: factor,
        rotation,
        viewport_width: width,
    }
}

fn layout_at(pages: &[(f32, f32)], geometry: Geometry) -> Layout {
    let mut layout = Layout::new(
        pages
            .iter()
            .map(|(width, height)| PageSize::new(*width, *height))
            .collect(),
    );
    layout.set_geometry(geometry);
    layout
}

#[test]
fn layout_one_page_is_centred_with_margins() {
    // 600 x 800 points in a 616-pixel-wide window at a scale of exactly 1.
    let layout = layout_at(
        &[(600.0, 800.0)],
        geometry(1000, 1.0, Rotation::Deg0, 616.0),
    );
    assert_eq!(layout.page_rect(0), Some((8.0, 8.0, 600.0, 800.0)));
    assert_eq!(layout.content_size(), (616.0, 816.0));
    assert_eq!(layout.page_count(), 1);
    assert_eq!(layout.page_rect(1), None);
}

#[test]
fn layout_pages_of_mixed_sizes_stack_with_gaps_and_centre() {
    let layout = layout_at(
        &[(600.0, 800.0), (300.0, 400.0), (600.0, 200.0)],
        geometry(1000, 1.0, Rotation::Deg0, 616.0),
    );
    assert_eq!(layout.page_rect(0), Some((8.0, 8.0, 600.0, 800.0)));
    // The narrow page is centred: (616 - 300) / 2.
    assert_eq!(layout.page_rect(1), Some((158.0, 816.0, 300.0, 400.0)));
    assert_eq!(layout.page_rect(2), Some((8.0, 1224.0, 600.0, 200.0)));
    // margin + 800 + gap + 400 + gap + 200 + margin
    assert_eq!(layout.content_size(), (616.0, 1432.0));
    assert_eq!(GAP, 8.0);
    assert_eq!(MARGIN, 8.0);
}

#[test]
fn layout_content_is_as_wide_as_the_viewport_or_the_widest_page() {
    let wide = layout_at(
        &[(600.0, 800.0)],
        geometry(2000, 1.0, Rotation::Deg0, 616.0),
    );
    // 1200 pixels of page + two margins; the page starts at the margin and the window scrolls.
    assert_eq!(wide.content_size().0, 1216.0);
    assert_eq!(wide.page_rect(0).map(|rect| rect.0), Some(8.0));
    let narrow = layout_at(&[(600.0, 800.0)], geometry(500, 1.0, Rotation::Deg0, 616.0));
    assert_eq!(narrow.content_size().0, 616.0);
    assert_eq!(narrow.page_rect(0), Some((158.0, 8.0, 300.0, 400.0)));
}

#[test]
fn layout_a_quarter_turn_swaps_the_sides_of_every_page() {
    let layout = layout_at(
        &[(600.0, 800.0), (300.0, 400.0)],
        geometry(1000, 1.0, Rotation::Deg90, 816.0),
    );
    assert_eq!(layout.page_rect(0), Some((8.0, 8.0, 800.0, 600.0)));
    assert_eq!(layout.page_rect(1), Some((208.0, 616.0, 400.0, 300.0)));
    let half_turn = layout_at(
        &[(600.0, 800.0)],
        geometry(1000, 1.0, Rotation::Deg180, 616.0),
    );
    assert_eq!(half_turn.page_rect(0), Some((8.0, 8.0, 600.0, 800.0)));
}

#[test]
fn layout_fit_width_uses_the_widest_page_as_displayed() {
    let layout = layout_at(
        &[(600.0, 800.0), (300.0, 400.0)],
        geometry(1000, 1.0, Rotation::Deg0, 616.0),
    );
    let viewport = Viewport {
        width: 616.0,
        height: 616.0,
        scale_factor: 1.0,
    };
    assert_eq!(
        layout.fit_scale(Fit::Width, 0, Rotation::Deg0, viewport),
        1.0
    );
    // Turned a quarter, the widest side is the tallest page's height: 800.
    assert_eq!(
        layout.fit_scale(Fit::Width, 0, Rotation::Deg90, viewport),
        0.75
    );
}

#[test]
fn layout_fit_page_makes_the_chosen_page_fit_both_ways() {
    let layout = layout_at(
        &[(600.0, 800.0), (800.0, 300.0)],
        geometry(1000, 1.0, Rotation::Deg0, 616.0),
    );
    let viewport = Viewport {
        width: 616.0,
        height: 616.0,
        scale_factor: 1.0,
    };
    // Portrait page: the height is the tight side (600 / 800).
    assert_eq!(
        layout.fit_scale(Fit::Page, 0, Rotation::Deg0, viewport),
        0.75
    );
    // Landscape page: the width is (600 / 800).
    assert_eq!(
        layout.fit_scale(Fit::Page, 1, Rotation::Deg0, viewport),
        0.75
    );
    // Turned, the landscape page is as tall as 800: still 0.75; the portrait one is now wide.
    assert_eq!(
        layout.fit_scale(Fit::Page, 0, Rotation::Deg90, viewport),
        0.75
    );
    // A page that does not exist falls back to the first.
    assert_eq!(
        layout.fit_scale(Fit::Page, 9, Rotation::Deg0, viewport),
        0.75
    );
    // A window smaller than its margins still gives a usable scale.
    let tiny = Viewport {
        width: 4.0,
        height: 4.0,
        scale_factor: 1.0,
    };
    assert!(layout.fit_scale(Fit::Width, 0, Rotation::Deg0, tiny) > 0.0);
}

#[test]
fn layout_percent_and_device_scale_are_clamped() {
    for (percent, expected) in [(0, 30), (30, 30), (100, 100), (500, 500), (501, 500)] {
        assert_eq!(clamp_percent(percent), expected, "{percent}");
    }
    assert_eq!(device_milli(1.0, 1.0), 1000);
    assert_eq!(device_milli(1.2810, 2.0), 2562);
    assert_eq!(
        device_milli(0.001, 1.0),
        50,
        "never below the engine's 0.05"
    );
    assert_eq!(device_milli(1000.0, 2.0), 16_000, "never above 16");
    assert_eq!(device_milli(f32::NAN, 1.0), 1000);
    assert_eq!(device_milli(f32::INFINITY, 1.0), 1000);
}

#[test]
fn layout_a_retina_screen_has_the_same_logical_layout_on_a_finer_grid() {
    let plain = layout_at(
        &[(600.0, 800.0)],
        geometry(1000, 1.0, Rotation::Deg0, 616.0),
    );
    let retina = layout_at(
        &[(600.0, 800.0)],
        geometry(2000, 2.0, Rotation::Deg0, 616.0),
    );
    assert_eq!(retina.page_rect(0), plain.page_rect(0));
    assert_eq!(retina.content_size(), plain.content_size());
    assert_eq!(retina.page_pixels(0), Some((1200, 1600)));
    assert_eq!(retina.page_origin_px(0), Some((16, 16)));
    assert_eq!(retina.logical_scale(), 1.0);
    assert_eq!(retina.device_scale(), 2.0);
}

#[test]
fn layout_tiles_are_a_grid_of_512_with_smaller_edges() {
    let layout = layout_at(
        &[(600.0, 800.0)],
        geometry(2000, 2.0, Rotation::Deg0, 616.0),
    );
    // 1200 x 1600 pixels: 3 columns (512, 512, 176) and 4 rows (512 x 3, 64).
    assert_eq!(layout.grid(0), Some((3, 4)));
    assert_eq!(layout.tile_rect(0, 0, 0), Some((0, 0, 512, 512)));
    assert_eq!(layout.tile_rect(0, 2, 3), Some((1024, 1536, 176, 64)));
    assert_eq!(layout.tile_rect(0, 3, 0), None);
    assert_eq!(layout.tile_rect(0, 0, 4), None);
    assert_eq!(layout.tile_rect(1, 0, 0), None);
    let request = layout.tile_request(0, 2, 3).expect("a tile of the grid");
    assert_eq!(request.scale, 2.0);
    assert_eq!(
        (
            request.page,
            request.x,
            request.y,
            request.width,
            request.height
        ),
        (0, 1024, 1536, 176, 64)
    );
    // The request is valid for the engine's own check.
    assert_eq!(request.validate(&[PageSize::new(600.0, 800.0)]), Ok(()));
    assert_eq!(TILE_SIDE, 512);
}

#[test]
fn layout_cells_in_a_region_follow_the_grid() {
    let layout = layout_at(
        &[(600.0, 800.0)],
        geometry(1000, 1.0, Rotation::Deg0, 616.0),
    );
    // The page is at (8, 8), 600 x 800: 2 columns, 2 rows.
    let region = |x0, y0, x1, y1| PxRect { x0, y0, x1, y1 };
    assert_eq!(
        layout.cells_in(0, region(0, 0, 616, 100)),
        Some((0..2, 0..1))
    );
    assert_eq!(layout.cells_in(0, region(8, 8, 9, 9)), Some((0..1, 0..1)));
    // The last pixel of the first column is x = 519 (8 + 511), the next one starts the second.
    assert_eq!(
        layout.cells_in(0, region(519, 8, 520, 9)),
        Some((0..1, 0..1))
    );
    assert_eq!(
        layout.cells_in(0, region(520, 8, 521, 9)),
        Some((1..2, 0..1))
    );
    assert_eq!(
        layout.cells_in(0, region(0, 500, 616, 700)),
        Some((0..2, 0..2))
    );
    // Misses: above, below, left of the page, and an empty region.
    assert_eq!(layout.cells_in(0, region(0, 0, 616, 8)), None);
    assert_eq!(layout.cells_in(0, region(0, 808, 616, 900)), None);
    assert_eq!(layout.cells_in(0, region(0, 0, 8, 900)), None);
    assert_eq!(layout.cells_in(0, region(100, 100, 100, 200)), None);
}

#[test]
fn layout_twenty_thousand_pages_find_their_pages_by_binary_search() {
    let pages: Vec<(f32, f32)> = (0..limits::MAX_PAGES)
        .map(|index| (600.0, 700.0 + (index % 7) as f32 * 25.0))
        .collect();
    let layout = layout_at(&pages, geometry(1000, 1.0, Rotation::Deg0, 616.0));
    assert_eq!(layout.page_count(), 20_000);
    let (_, content_height) = layout.content_size();
    assert!(content_height > 14_000_000.0, "{content_height}");
    // Every page is found at its own top and bottom row, and nothing else is.
    for page in [0_usize, 1, 7, 999, 10_000, 19_998, 19_999] {
        let rect = layout.page_rect_px(page).expect("a page");
        assert_eq!(layout.visible_pages(rect.y0, rect.y0 + 1), page..page + 1);
        assert_eq!(layout.visible_pages(rect.y1 - 1, rect.y1), page..page + 1);
        assert_eq!(layout.nearest_page(rect.y0 as f64), Some(page));
        // The gap after a page: the first half is that page, the second half the next.
        if page + 1 < 20_000 {
            assert_eq!(layout.nearest_page(rect.y1 as f64 + 1.0), Some(page));
            assert_eq!(layout.nearest_page(rect.y1 as f64 + 7.0), Some(page + 1));
            assert_eq!(
                layout.visible_pages(rect.y1, rect.y1 + 8),
                page + 1..page + 1
            );
        }
    }
    // A viewport of 900 pixels shows a handful of pages, wherever it is.
    for top in [0_i64, 123_456, 7_000_000, 14_000_000] {
        let range = layout.visible_pages(top, top + 900);
        assert!((1..=3).contains(&range.len()), "{range:?}");
    }
    assert_eq!(layout.visible_pages(i64::MAX - 1, i64::MAX), 20_000..20_000);
}

#[test]
fn layout_visible_pages_agree_with_a_linear_scan() {
    let mut rng = Rng(0x0DDB_1A5E_5BAD_5EED);
    for _ in 0..40 {
        let count = rng.between(1, 60) as usize;
        let pages: Vec<(f32, f32)> = (0..count)
            .map(|_| (rng.between(50, 900) as f32, rng.between(50, 1200) as f32))
            .collect();
        let milli = rng.between(50, 4000) as u32;
        let factor = [1.0, 1.25, 1.5, 2.0][rng.below(4) as usize];
        let rotation = Rotation::from_quarter_turns(rng.below(4) as i32);
        let layout = layout_at(&pages, geometry(milli, factor, rotation, 700.0));
        let (_, height) = layout.content_size_px();
        for _ in 0..50 {
            let y0 = rng.below(height + 50) as i64 - 25;
            let y1 = y0 + rng.between(1, 2000) as i64;
            let expected: Vec<usize> = (0..count)
                .filter(|&page| {
                    let rect = layout.page_rect_px(page).expect("a page");
                    rect.y0 < y1 && y0 < rect.y1
                })
                .collect();
            let range = layout.visible_pages(y0, y1);
            assert_eq!(range.clone().collect::<Vec<_>>(), expected, "{y0}..{y1}");
        }
    }
}

#[test]
fn layout_pages_never_overlap_and_keep_their_gaps() {
    let mut rng = Rng(42);
    for _ in 0..30 {
        let count = rng.between(2, 40) as usize;
        let pages: Vec<(f32, f32)> = (0..count)
            .map(|_| (rng.between(20, 1000) as f32, rng.between(20, 1000) as f32))
            .collect();
        let factor = [1.0, 1.5, 2.0][rng.below(3) as usize];
        let layout = layout_at(
            &pages,
            geometry(rng.between(50, 3000) as u32, factor, Rotation::Deg0, 640.0),
        );
        let gap = (f64::from(GAP) * f64::from(factor)).round() as i64;
        for page in 1..count {
            let above = layout.page_rect_px(page - 1).expect("a page");
            let below = layout.page_rect_px(page).expect("a page");
            assert_eq!(below.y0 - above.y1, gap, "page {page}");
        }
        let (content_width, content_height) = layout.content_size_px();
        let first = layout.page_rect_px(0).expect("a page");
        let last = layout.page_rect_px(count - 1).expect("a page");
        let margin = (f64::from(MARGIN) * f64::from(factor)).round() as i64;
        assert_eq!(first.y0, margin);
        assert_eq!(content_height as i64, last.y1 + margin);
        // Every page is centred: the same space on its left and right (to a pixel), and the
        // widest one has the margin.
        let mut tightest = i64::MAX;
        for page in 0..count {
            let rect = layout.page_rect_px(page).expect("a page");
            let (left, right) = (rect.x0, content_width as i64 - rect.x1);
            assert!(left >= 0 && right >= 0);
            assert!(
                (left - right).abs() <= 1,
                "page {page}: {left} against {right}"
            );
            tightest = tightest.min(left);
        }
        assert!(tightest >= margin, "{tightest} < {margin}");
    }
}

#[test]
fn layout_tiles_of_a_region_are_disjoint_inside_the_page_and_cover_it_exactly() {
    let mut rng = Rng(7);
    for _ in 0..200 {
        let count = rng.between(1, 8) as usize;
        let pages: Vec<(f32, f32)> = (0..count)
            .map(|_| (rng.between(30, 1500) as f32, rng.between(30, 1500) as f32))
            .collect();
        let factor = [1.0, 1.25, 2.0][rng.below(3) as usize];
        let rotation = Rotation::from_quarter_turns(rng.below(4) as i32);
        let layout = layout_at(
            &pages,
            geometry(rng.between(50, 4000) as u32, factor, rotation, 600.0),
        );
        let (width, height) = layout.content_size_px();
        let x0 = rng.below(width) as i64;
        let y0 = rng.below(height) as i64;
        let region = PxRect {
            x0,
            y0,
            x1: x0 + rng.between(1, 3000) as i64,
            y1: y0 + rng.between(1, 3000) as i64,
        };
        for page in layout.visible_pages(region.y0, region.y1) {
            let page_rect = layout.page_rect_px(page).expect("a page");
            let Some((cols, rows)) = layout.cells_in(page, region) else {
                // The region misses the page sideways.
                assert!(!page_rect.intersects(&region), "page {page} {region:?}");
                continue;
            };
            let tiles: Vec<(i64, i64, i64, i64)> = rows
                .clone()
                .flat_map(|row| cols.clone().map(move |col| (col, row)))
                .map(|(col, row)| {
                    let (x, y, w, h) = layout.tile_rect(page, col, row).expect("a tile");
                    (
                        i64::from(x),
                        i64::from(y),
                        i64::from(x + w),
                        i64::from(y + h),
                    )
                })
                .collect();
            let (page_w, page_h) = layout.page_pixels(page).expect("a page");
            // Inside the page, and each one touches the region.
            for &(tx0, ty0, tx1, ty1) in &tiles {
                assert!(tx0 >= 0 && ty0 >= 0);
                assert!(tx1 <= i64::from(page_w) && ty1 <= i64::from(page_h));
                let absolute = PxRect {
                    x0: page_rect.x0 + tx0,
                    y0: page_rect.y0 + ty0,
                    x1: page_rect.x0 + tx1,
                    y1: page_rect.y0 + ty1,
                };
                assert!(
                    absolute.intersects(&region),
                    "a tile the region does not need"
                );
            }
            // No two overlap.
            for (index, a) in tiles.iter().enumerate() {
                for b in &tiles[index + 1..] {
                    assert!(
                        a.2 <= b.0 || b.2 <= a.0 || a.3 <= b.1 || b.3 <= a.1,
                        "{a:?} overlaps {b:?}"
                    );
                }
            }
            // Together they hold every pixel of the visible part of the page: the union's area is
            // the sum (disjoint), and it contains the visible rectangle.
            let visible = PxRect {
                x0: (region.x0 - page_rect.x0).max(0),
                y0: (region.y0 - page_rect.y0).max(0),
                x1: (region.x1 - page_rect.x0).min(i64::from(page_w)),
                y1: (region.y1 - page_rect.y0).min(i64::from(page_h)),
            };
            let union_x0 = tiles.iter().map(|t| t.0).min().expect("a tile");
            let union_y0 = tiles.iter().map(|t| t.1).min().expect("a tile");
            let union_x1 = tiles.iter().map(|t| t.2).max().expect("a tile");
            let union_y1 = tiles.iter().map(|t| t.3).max().expect("a tile");
            let area: i64 = tiles.iter().map(|t| (t.2 - t.0) * (t.3 - t.1)).sum();
            assert_eq!(
                area,
                (union_x1 - union_x0) * (union_y1 - union_y0),
                "holes in the grid"
            );
            assert!(union_x0 <= visible.x0 && union_y0 <= visible.y0);
            assert!(union_x1 >= visible.x1 && union_y1 >= visible.y1);
        }
    }
}

#[test]
fn layout_placeholders_are_small_whole_pages() {
    let letter = PageSize::new(612.0, 792.0);
    let request = placeholder_request(letter, 3, Rotation::Deg0).expect("a placeholder");
    assert_eq!(request.page, 3);
    assert_eq!(request.scale, 0.261);
    assert_eq!((request.x, request.y), (0, 0));
    assert_eq!((request.width, request.height), (160, 207));
    assert_eq!(request.validate(&[letter; 4]), Ok(()));

    let turned = placeholder_request(letter, 0, Rotation::Deg90).expect("a placeholder");
    assert_eq!((turned.width, turned.height), (160, 124));
    assert_eq!(turned.validate(&[letter]), Ok(()));

    // Every placeholder obeys the width limit and the engine's own checks.
    let mut rng = Rng(99);
    for _ in 0..500 {
        let size = PageSize::new(
            rng.between(10, 3000) as f32 / 3.0,
            rng.between(10, 20_000) as f32 / 3.0,
        );
        let rotation = Rotation::from_quarter_turns(rng.below(4) as i32);
        if let Some(request) = placeholder_request(size, 0, rotation) {
            assert!(request.width <= 160, "{size:?} {request:?}");
            assert!(request.validate(&[size]).is_ok(), "{size:?} {request:?}");
            assert_eq!(request.scale, (request.scale * 1000.0).round() / 1000.0);
        }
    }
    // A page so large that the smallest scale already makes it wider than 160 pixels has none.
    assert_eq!(
        placeholder_request(PageSize::new(14_400.0, 14_400.0), 0, Rotation::Deg0),
        None
    );
}

#[test]
fn layout_points_convert_between_page_space_and_content_space() {
    // One 612 x 792 page at 1 device pixel per point in an 800-wide window: origin (8, 8) when
    // turned a quarter (792 wide, so the content is 808 wide), (94, 8) otherwise.
    let pages = [(612.0, 792.0)];
    let cases = [
        // rotation, page-space point, expected content point
        (Rotation::Deg0, (0.0, 0.0), (94.0, 8.0)),
        (Rotation::Deg0, (612.0, 792.0), (706.0, 800.0)),
        // The page's top edge becomes the right edge.
        (Rotation::Deg90, (0.0, 0.0), (800.0, 8.0)),
        (Rotation::Deg90, (612.0, 792.0), (8.0, 620.0)),
        (Rotation::Deg180, (0.0, 0.0), (706.0, 800.0)),
        // The page's top edge becomes the left edge.
        (Rotation::Deg270, (0.0, 0.0), (8.0, 620.0)),
        (Rotation::Deg270, (612.0, 0.0), (8.0, 8.0)),
    ];
    for (rotation, page_point, expected) in cases {
        let layout = layout_at(&pages, geometry(1000, 1.0, rotation, 800.0));
        let (x, y) = layout
            .page_point_to_content(0, page_point.0, page_point.1)
            .expect("on the page");
        assert!(
            (x - expected.0).abs() < 1e-9 && (y - expected.1).abs() < 1e-9,
            "{rotation:?} {page_point:?}: {x},{y} != {expected:?}"
        );
    }
}

#[test]
fn layout_a_point_survives_the_round_trip_in_every_rotation() {
    let mut rng = Rng(2026);
    for rotation in [
        Rotation::Deg0,
        Rotation::Deg90,
        Rotation::Deg180,
        Rotation::Deg270,
    ] {
        let layout = layout_at(
            &[(612.0, 792.0), (300.5, 400.25)],
            geometry(1337, 1.5, rotation, 700.0),
        );
        for page in 0..2 {
            for _ in 0..100 {
                let x = rng.below(8000) as f32 / 10.0 - 100.0;
                let y = rng.below(10_000) as f32 / 10.0 - 100.0;
                let (cx, cy) = layout.page_point_to_content(page, x, y).expect("a page");
                let (bx, by) = layout.content_point_to_page(page, cx, cy).expect("a page");
                assert!(
                    (bx - x).abs() < 0.01 && (by - y).abs() < 0.01,
                    "{rotation:?} page {page}: {x},{y} -> {bx},{by}"
                );
            }
        }
    }
}

#[test]
fn layout_a_rectangle_maps_to_the_pixels_the_page_is_drawn_in() {
    let layout = layout_at(
        &[(612.0, 792.0)],
        geometry(1000, 1.0, Rotation::Deg90, 800.0),
    );
    // The whole page maps onto the page's own rectangle.
    let whole = mercury_pdf::types::Rect::new(0.0, 0.0, 612.0, 792.0);
    assert_eq!(
        layout.page_rect_to_content(0, &whole),
        layout
            .page_rect(0)
            .map(|rect| (rect.0, rect.1, rect.2, rect.3))
    );
    // A strip along the top of the page ends up along the right side of the turned page.
    let strip = mercury_pdf::types::Rect::new(0.0, 0.0, 612.0, 100.0);
    assert_eq!(
        layout.page_rect_to_content(0, &strip),
        Some((700.0, 8.0, 100.0, 612.0))
    );
}

#[test]
fn layout_hostile_page_sizes_cannot_break_the_arithmetic() {
    let layout = Layout::new(vec![
        PageSize::new(f32::NAN, 100.0),
        PageSize::new(0.0, -5.0),
        PageSize::new(f32::INFINITY, 1e30),
    ]);
    for page in 0..3 {
        let size = layout.page_size(page).expect("a page");
        assert!((1.0..=50_000.0).contains(&size.width), "{size:?}");
        assert!((1.0..=50_000.0).contains(&size.height), "{size:?}");
        assert!(layout.page_rect(page).is_some());
    }
    assert_eq!(
        Layout::new(Vec::new()).nearest_page(10.0),
        None,
        "an empty document has no pages"
    );
    assert_eq!(Layout::new(Vec::new()).visible_pages(0, 100), 0..0);
}

#[test]
fn layout_viewport_numbers_are_made_usable() {
    let fixed = Viewport {
        width: f32::NAN,
        height: -3.0,
        scale_factor: 0.0,
    }
    .sanitized();
    assert_eq!(
        (fixed.width, fixed.height, fixed.scale_factor),
        (800.0, 1.0, 1.0)
    );
    let clamped = Viewport {
        width: 500.0,
        height: 400.0,
        scale_factor: 40.0,
    }
    .sanitized();
    assert_eq!(clamped.scale_factor, 8.0);
}
