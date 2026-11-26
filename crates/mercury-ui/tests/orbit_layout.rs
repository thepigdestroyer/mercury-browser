// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Orbit stage layout (brief §6 item 2, D12): the mockup's five planets, their sizes, clearances
//! from the title bar, the footer and the sun for every window size and group count, the preview
//! cards of level 2, the level-3 card, and Orbit's own hit test. Needs `--features test-support`
//! for the fixtures.
#![cfg(feature = "test-support")]

use mercury_ui::orbit_view::fixtures::{generated_input, orbit_input, orbit_tabs};
use mercury_ui::orbit_view::layout::{OrbitLayout, Rect, Side, TooltipSide};
use mercury_ui::orbit_view::{
    LOOSE_GROUP_ID, Level, OrbitInput, Stage, TITLE_BAR_HEIGHT, Target, View,
};
use mercury_ui::ring_geometry::Point;

const MOCKUP_CENTRES: [(f32, f32); 5] = [
    (720.0, 196.0),
    (1160.0, 392.0),
    (990.0, 706.0),
    (450.0, 706.0),
    (280.0, 392.0),
];

const SIZES: [(f32, f32); 6] = [
    (900.0, 600.0),
    (1280.0, 800.0),
    (1440.0, 900.0),
    (1920.0, 1080.0),
    (2560.0, 1440.0),
    (3440.0, 1440.0),
];

fn view(level: Level, group: Option<i32>, tab: Option<i32>) -> View {
    View {
        level,
        group,
        tab,
        ..View::default()
    }
}

fn level1(stage: Stage, input: &OrbitInput) -> OrbitLayout {
    OrbitLayout::new(stage, input, &View::default())
}

fn level2(stage: Stage, input: &OrbitInput, group: i32) -> OrbitLayout {
    OrbitLayout::new(stage, input, &view(Level::Group, Some(group), None))
}

fn level3(stage: Stage, input: &OrbitInput, group: i32, tab: i32) -> OrbitLayout {
    OrbitLayout::new(stage, input, &view(Level::Tab, Some(group), Some(tab)))
}

fn dist(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// How deep two rectangles run into each other: the smaller of the horizontal and the vertical
/// overlap, 0 when they are apart.
fn depth(a: Rect, b: Rect) -> f32 {
    let across = a.right().min(b.right()) - a.x.max(b.x);
    let down = a.bottom().min(b.bottom()) - a.y.max(b.y);
    across.min(down).max(0.0)
}

fn grown(rect: Rect, by: f32) -> Rect {
    Rect::new(
        rect.x - by,
        rect.y - by,
        rect.w + 2.0 * by,
        rect.h + 2.0 * by,
    )
}

// ---- level 1: the planets ----

#[test]
fn the_five_planets_are_within_16_px_of_the_mockup() {
    let layout = level1(Stage::MOCKUP, &orbit_input(false));
    assert_eq!(layout.planets.len(), 5);
    for (cell, (x, y)) in layout.planets.iter().zip(MOCKUP_CENTRES) {
        let off = dist(cell.centre, Point::new(x, y));
        assert!(
            off <= 16.01,
            "group {}: {off} px from the mockup",
            cell.group
        );
    }
}

#[test]
fn the_planet_sizes_are_40_plus_2_per_tab_exactly() {
    let layout = level1(Stage::MOCKUP, &orbit_input(false));
    let radii: Vec<f32> = layout.planets.iter().map(|p| p.spec.radius).collect();
    assert_eq!(radii, vec![60.0, 56.0, 58.0, 54.0, 52.0]);
    for cell in &layout.planets {
        assert_eq!(cell.scale, 1.0);
        assert_eq!(cell.spec.band_width, 16.0);
        assert_eq!(cell.radius, cell.spec.radius);
    }
    // The label band is 2 px outside the tab band and 22 px wide: Research's outer radius is 92
    // and the halo is 11 px beyond it (103 in the mockup).
    assert_eq!(layout.planets[0].diameter, 184.0);
    assert_eq!(layout.planets[0].halo_radius, 103.0);
}

#[test]
fn a_planets_ring_spec_is_ring_local_and_its_box_holds_the_ring() {
    let layout = level1(Stage::MOCKUP, &orbit_input(false));
    for cell in &layout.planets {
        let half = cell.ring_size / 2.0;
        assert_eq!((cell.spec.centre.x, cell.spec.centre.y), (half, half));
        assert!(half >= cell.spec.label_outer_radius() + cell.spec.lift);
        assert!(cell.ring_size < 2.0 * (cell.spec.label_outer_radius() + 12.0));
    }
}

#[test]
fn the_sun_and_the_new_group_ring_are_where_the_mockup_has_them() {
    let layout = level1(Stage::MOCKUP, &orbit_input(false));
    assert_eq!(layout.sun.centre, Point::new(720.0, 470.0));
    assert_eq!(layout.sun.radius, 92.0);
    assert_eq!(layout.sun.opacity, 1.0);
    assert!((layout.sun.glow_radius - 133.4).abs() < 0.01);
    assert_eq!(layout.new_group.centre, Point::new(1250.0, 800.0));
    assert_eq!(layout.new_group.radius, 50.0);
    assert!(layout.new_group.visible);
    for cell in &layout.planets {
        assert!(
            dist(cell.centre, layout.new_group.centre)
                >= cell.halo_radius + layout.new_group.radius,
            "group {} touches the New group ring",
            cell.group
        );
    }
}

#[test]
fn the_title_bar_and_the_footer_are_on_the_stage() {
    let layout = level1(Stage::MOCKUP, &orbit_input(false));
    assert_eq!(
        layout.title_bar,
        Rect::new(0.0, 0.0, 1440.0, TITLE_BAR_HEIGHT)
    );
    assert_eq!(layout.footer.right(), 1440.0);
    assert_eq!(layout.footer.bottom(), 892.0);
    assert_eq!(layout.footer.y, 852.0);
}

#[test]
fn no_planet_reaches_the_title_bar_the_footer_or_the_sun() {
    for (w, h) in SIZES {
        for n in [1_usize, 5, 12, 24, 64] {
            let input = generated_input(n, n * 6, false);
            let layout = level1(Stage::new(w, h), &input);
            let ctx = format!("{w}x{h}, {n} groups");
            assert_eq!(layout.planets.len(), n, "{ctx}");
            for cell in &layout.planets {
                let r = cell.halo_radius;
                assert!(
                    cell.centre.y - r >= TITLE_BAR_HEIGHT,
                    "{ctx}: title bar, {cell:?}"
                );
                assert!(
                    cell.centre.y + r <= layout.footer.y,
                    "{ctx}: footer, {cell:?}"
                );
                assert!(
                    cell.centre.x - r >= 0.0 && cell.centre.x + r <= w,
                    "{ctx}: sides"
                );
                let gap = dist(cell.centre, layout.sun.centre) - cell.diameter / 2.0;
                assert!(gap >= layout.sun.radius, "{ctx}: sun, gap {gap}, {cell:?}");
            }
        }
    }
}

#[test]
fn the_planets_shrink_to_fit_a_small_window_and_never_grow_in_a_big_one() {
    let input = orbit_input(false);
    let small = level1(Stage::new(900.0, 600.0), &input);
    let big = level1(Stage::new(3440.0, 1440.0), &input);
    assert!(
        small.fit < 1.0,
        "the 900 x 600 window scales the ellipse down"
    );
    assert_eq!(big.fit, 1.0);
    assert_eq!(big.ellipse.rx, 470.0);
    assert_eq!(big.ellipse.ry, 290.0);
    assert!(small.ellipse.rx < 470.0 && small.ellipse.ry < 290.0);
    for cell in &big.planets {
        assert_eq!(cell.scale, 1.0);
    }
}

#[test]
fn rebuilding_at_another_size_keeps_the_planet_order() {
    let input = orbit_input(false);
    let order = |w, h| -> Vec<i32> {
        level1(Stage::new(w, h), &input)
            .planets
            .iter()
            .map(|p| p.group)
            .collect()
    };
    let wanted: Vec<i32> = input.groups.iter().map(|g| g.id).collect();
    for (w, h) in SIZES {
        assert_eq!(order(w, h), wanted, "{w}x{h}");
    }
}

#[test]
fn the_tooltip_sits_to_the_right_unless_it_would_leave_the_window() {
    let layout = level1(Stage::MOCKUP, &orbit_input(false));
    let top = &layout.planets[0];
    assert_eq!(top.tooltip_side, TooltipSide::Right);
    assert!(top.tooltip.x >= top.centre.x + top.halo_radius);
    let right = &layout.planets[1];
    assert_ne!(
        right.tooltip_side,
        TooltipSide::Right,
        "the planet at x 1160 has no room on its right"
    );
    if right.tooltip_side == TooltipSide::Left {
        assert!(right.tooltip.right() <= right.centre.x - right.halo_radius + 0.01);
    }
    assert_eq!(
        right.tooltip_side,
        TooltipSide::Above,
        "its left side is the sun's, so the tooltip goes above"
    );
    assert!(right.tooltip.bottom() <= right.centre.y - right.halo_radius + 0.01);
    for (i, cell) in layout.planets.iter().enumerate() {
        assert_eq!(
            crowding(&layout, i),
            0.0,
            "the mockup's tooltips are clear: {cell:?}"
        );
    }
    for cell in &layout.planets {
        assert!(
            cell.tooltip.x >= 0.0 && cell.tooltip.right() <= 1440.0,
            "{cell:?}"
        );
        assert!(cell.tooltip.y >= TITLE_BAR_HEIGHT && cell.tooltip.bottom() <= 900.0);
    }
}

/// How far the rectangle runs into the circle (0 when they do not touch).
fn into_circle(rect: Rect, centre: Point, radius: f32) -> f32 {
    let nearest = Point::new(
        centre.x.clamp(rect.x, rect.right()),
        centre.y.clamp(rect.y, rect.bottom()),
    );
    (radius - dist(nearest, centre)).max(0.0)
}

/// What a tooltip runs into: the other planets' halos and the sun.
fn crowding(layout: &OrbitLayout, index: usize) -> f32 {
    crowding_of(layout, index, layout.planets[index].tooltip)
}

/// The same for a rectangle that stands for planet `index`'s tooltip.
fn crowding_of(layout: &OrbitLayout, index: usize, tooltip: Rect) -> f32 {
    let planets: f32 = layout
        .planets
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, p)| into_circle(tooltip, p.centre, p.halo_radius))
        .sum();
    planets + into_circle(tooltip, layout.sun.centre, layout.sun.radius)
}

fn inside(rect: Rect, stage: Stage) -> bool {
    rect.x >= 8.0
        && rect.right() <= stage.width - 8.0
        && rect.y >= TITLE_BAR_HEIGHT
        && rect.bottom() <= stage.height - 8.0
}

/// (planets whose tooltip touches a neighbour or the sun, planets whose tooltip covers their own
/// halo) for `n` groups on `stage`; every tooltip must be inside the stage.
fn tooltip_trouble(stage: Stage, n: usize) -> (usize, usize) {
    let layout = level1(stage, &generated_input(n, n * 6, false));
    let (mut crowded, mut covering) = (0, 0);
    for (i, cell) in layout.planets.iter().enumerate() {
        assert!(
            inside(cell.tooltip, stage),
            "{}x{}, {n} groups: {cell:?}",
            stage.width,
            stage.height
        );
        crowded += usize::from(crowding(&layout, i) > 0.0);
        covering += usize::from(into_circle(cell.tooltip, cell.centre, cell.halo_radius) > 0.0);
    }
    println!(
        "{}x{}, {n} groups: {crowded} tooltips touch a neighbour or the sun, {covering} their own halo",
        stage.width, stage.height
    );
    (crowded, covering)
}

#[test]
fn a_tooltip_touches_no_other_planet_and_not_the_sun_at_5_and_12_groups() {
    for (w, h) in [(1440.0, 900.0), (1920.0, 1080.0), (2560.0, 1440.0)] {
        for n in [5_usize, 12] {
            assert_eq!(tooltip_trouble(Stage::new(w, h), n), (0, 0), "{w}x{h}, {n}");
        }
    }
    // A small window with 12 planets may have none of the four sides free for a planet or two.
    assert_eq!(tooltip_trouble(Stage::new(1280.0, 800.0), 5), (0, 0));
    let (crowded, covering) = tooltip_trouble(Stage::new(1280.0, 800.0), 12);
    assert!(crowded <= 3 && covering == 0, "{crowded}, {covering}");
}

/// The tooltip the first version drew: right of the halo, left when that leaves the window.
fn right_or_left(cell: &mercury_ui::orbit_view::layout::PlanetCell, stage: Stage) -> Rect {
    let w = cell.tooltip.w;
    let mut x = cell.centre.x + cell.halo_radius + 8.0;
    if x + w > stage.width - 8.0 {
        x = cell.centre.x - cell.halo_radius - 8.0 - w;
    }
    x = x.min(stage.width - 8.0 - w).max(8.0);
    let y = (cell.centre.y - 16.0)
        .min(stage.height - 8.0 - 32.0)
        .max(TITLE_BAR_HEIGHT + 8.0);
    Rect::new(x, y, w, 32.0)
}

#[test]
fn at_24_groups_the_four_sides_run_into_neighbours_less_than_right_or_left_did() {
    for (w, h) in [(1440.0, 900.0), (1920.0, 1080.0), (2560.0, 1440.0)] {
        let stage = Stage::new(w, h);
        let layout = level1(stage, &generated_input(24, 24 * 6, false));
        let (now, before): (f32, f32) = layout
            .planets
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                (
                    crowding_of(&layout, i, cell.tooltip),
                    crowding_of(&layout, i, right_or_left(cell, stage)),
                )
            })
            .fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
        let (crowded, covering) = tooltip_trouble(stage, 24);
        println!("{w}x{h}, 24 groups: depth {now:.0} px now, {before:.0} px with right or left");
        assert!(now < before, "{w}x{h}: {now} against {before}");
        assert_eq!(covering, 0, "{w}x{h}: a tooltip covers its own planet");
        assert!(crowded < 24, "{w}x{h}: {crowded} of 24 touch a neighbour");
    }
}

#[test]
fn loose_and_pinned_tabs_make_one_last_planet() {
    let mut tabs = orbit_tabs();
    tabs[0].pinned = true;
    let input = OrbitInput::new(
        &mercury_ui::orbit_view::fixtures::orbit_groups(false),
        &tabs,
        "Personal",
        "Window 1",
        101,
        false,
    );
    let layout = level1(Stage::MOCKUP, &input);
    assert_eq!(layout.planets.len(), 6);
    let last = layout.planets.last().expect("the pseudo planet");
    assert_eq!(last.group, LOOSE_GROUP_ID);
    assert_eq!(last.tabs, 1);
    assert_eq!(layout.planets[0].tabs, 9, "the pinned tab left Research");
    assert_eq!(level1(Stage::MOCKUP, &orbit_input(false)).planets.len(), 5);
}

#[test]
fn levels_two_and_three_slide_the_planets_out_and_hide_the_shown_one() {
    let input = orbit_input(false);
    let layout = level2(Stage::MOCKUP, &input, 1);
    let centre = layout.sun.centre;
    for cell in &layout.planets {
        assert_eq!(cell.opacity, 0.0, "group {}", cell.group);
        if cell.group == 1 {
            assert_eq!(cell.shown, centre);
            assert!(!cell.dimmed);
        } else {
            assert!(cell.dimmed);
            assert!(dist(cell.shown, centre) > dist(cell.centre, centre));
        }
    }
    assert_eq!(layout.sun.opacity, 0.0);
    assert!(!layout.new_group.visible);
    let first = level1(Stage::MOCKUP, &input);
    for cell in &first.planets {
        assert_eq!(cell.opacity, 1.0);
        assert_eq!(cell.shown, cell.centre);
        assert!(!cell.dimmed);
    }
}

#[test]
fn a_level_that_lost_its_group_or_tab_falls_back_one_level() {
    let input = orbit_input(false);
    let stage = Stage::MOCKUP;
    let gone_group = OrbitLayout::new(stage, &input, &view(Level::Tab, Some(99), Some(104)));
    assert_eq!(gone_group.level, Level::Groups);
    assert!(gone_group.centre.is_none() && gone_group.card.is_none());
    let gone_tab = OrbitLayout::new(stage, &input, &view(Level::Tab, Some(1), Some(999)));
    assert_eq!(gone_tab.level, Level::Group);
    let other_group_tab = OrbitLayout::new(stage, &input, &view(Level::Tab, Some(1), Some(115)));
    assert_eq!(other_group_tab.level, Level::Group, "tab 115 is in Reading");
    let no_group = OrbitLayout::new(stage, &input, &view(Level::Group, None, None));
    assert_eq!(no_group.level, Level::Groups);
}

// ---- level 2: the group ----

#[test]
fn ten_cards_at_level_2_do_not_overlap() {
    let layout = level2(Stage::MOCKUP, &orbit_input(false), 1);
    assert_eq!(layout.previews.len(), 10);
    assert_eq!(layout.more, 0);
    for (i, a) in layout.previews.iter().enumerate() {
        for b in &layout.previews[i + 1..] {
            // The 2 px outline is part of a card.
            assert!(
                depth(grown(a.rect, 2.0), grown(b.rect, 2.0)) <= 0.01,
                "tabs {} and {} overlap",
                a.tab,
                b.tab
            );
        }
    }
    let first = &layout.previews[0];
    assert_eq!((first.rect.w, first.rect.h), (184.0, 124.0));
}

#[test]
fn twenty_four_cards_in_two_rings_overlap_by_under_5_px_and_the_inner_ring_is_drawn_first() {
    let input = generated_input(1, 24, false);
    let layout = level2(Stage::MOCKUP, &input, 1);
    assert_eq!(layout.previews.len(), 24);
    assert_eq!(layout.more, 0);
    let rings: Vec<usize> = layout.previews.iter().map(|p| p.ring).collect();
    assert_eq!(
        rings[..12],
        [1; 12],
        "the inner ring is first in draw order"
    );
    assert_eq!(rings[12..], [0; 12]);
    let mut deepest = 0.0_f32;
    for (i, a) in layout.previews.iter().enumerate() {
        for b in &layout.previews[i + 1..] {
            let d = depth(a.rect, b.rect);
            if a.ring == b.ring {
                assert!(d <= 0.01, "two cards of ring {} overlap by {d}", a.ring);
            } else {
                deepest = deepest.max(d);
            }
        }
    }
    println!("deepest overlap of the two rings: {deepest} px");
    assert!(deepest < 5.0, "the two rings overlap by {deepest} px");
    let outer = layout.previews.last().expect("outer card");
    let inner = &layout.previews[0];
    assert!(
        (inner.rect.w / outer.rect.w - 0.58).abs() < 1e-3,
        "inner cards are drawn at 0.58"
    );
}

#[test]
fn forty_tabs_give_24_cards_and_16_more() {
    let input = generated_input(1, 40, false);
    let layout = level2(Stage::MOCKUP, &input, 1);
    assert_eq!(layout.previews.len(), 24);
    assert_eq!(layout.more, 16);
}

#[test]
fn the_active_tab_comes_first_then_strip_order() {
    let mut input = orbit_input(false);
    input.active_tab = 114;
    let order = OrbitLayout::preview_order(&input, 2);
    assert_eq!(order, vec![114, 111, 112, 113, 115, 116, 117, 118]);
    // An active tab of another group changes nothing.
    assert_eq!(
        OrbitLayout::preview_order(&orbit_input(false), 2),
        vec![111, 112, 113, 114, 115, 116, 117, 118]
    );
    let layout = level2(Stage::MOCKUP, &input, 2);
    let by_order = |n: usize| layout.previews.iter().find(|p| p.order == n).map(|p| p.tab);
    assert_eq!(by_order(0), Some(114));
    assert_eq!(by_order(1), Some(111));
    let top = layout
        .previews
        .iter()
        .find(|p| p.order == 0)
        .expect("first card");
    assert!(
        layout
            .previews
            .iter()
            .all(|p| p.rect.centre().y >= top.rect.centre().y)
    );
    assert!(
        (top.rect.centre().x - 720.0).abs() < 0.01,
        "the first card is at the top"
    );
}

#[test]
fn the_cards_stay_inside_the_stage_at_900_by_600() {
    for tabs in [10_usize, 24] {
        let input = generated_input(1, tabs, false);
        let stage = Stage::new(900.0, 600.0);
        let layout = level2(stage, &input, 1);
        let ring = layout.centre.as_ref().expect("centre ring");
        for card in &layout.previews {
            let r = grown(card.rect, 2.0);
            assert!(r.x >= 0.0 && r.right() <= 900.0, "{tabs} tabs: {r:?}");
            assert!(
                r.y >= TITLE_BAR_HEIGHT && r.bottom() <= 546.0,
                "{tabs} tabs: {r:?}"
            );
            // The ring is not under a card (an inner card's corner may brush the label band's
            // rim by a pixel or two).
            let nearest = Point::new(
                ring.centre.x.clamp(card.rect.x, card.rect.right()),
                ring.centre.y.clamp(card.rect.y, card.rect.bottom()),
            );
            assert!(
                dist(nearest, ring.centre) >= ring.spec.label_outer_radius() - 3.0,
                "{tabs} tabs: card {} touches the ring",
                card.tab
            );
        }
        assert!(
            layout.previews[0].rect.w < 184.0,
            "the cards scale down with the window"
        );
    }
}

#[test]
fn the_centre_ring_has_radius_92_and_the_stage_centre() {
    let layout = level2(Stage::MOCKUP, &orbit_input(false), 1);
    let ring = layout.centre.as_ref().expect("centre ring");
    assert_eq!(ring.group, 1);
    assert_eq!(ring.centre, Point::new(720.0, 470.0));
    assert_eq!(ring.spec.radius, 92.0);
    assert_eq!(ring.scale, 1.0);
    let half = ring.ring_size / 2.0;
    assert_eq!((ring.spec.centre.x, ring.spec.centre.y), (half, half));
    assert!(half >= ring.spec.label_outer_radius());
}

#[test]
fn each_spoke_runs_from_the_ring_to_the_edge_of_its_card() {
    let layout = level2(Stage::MOCKUP, &orbit_input(false), 1);
    let ring = layout.centre.as_ref().expect("centre ring");
    for card in &layout.previews {
        let from = dist(card.spoke_from, ring.centre);
        assert!(
            (from - ring.spec.label_outer_radius()).abs() < 0.01,
            "tab {}: the spoke starts {from} from the centre",
            card.tab
        );
        let to = card.spoke_to;
        let on_edge = ((to.x - card.rect.x).abs() < 0.01
            || (to.x - card.rect.right()).abs() < 0.01
            || (to.y - card.rect.y).abs() < 0.01
            || (to.y - card.rect.bottom()).abs() < 0.01)
            && to.x >= card.rect.x - 0.01
            && to.x <= card.rect.right() + 0.01
            && to.y >= card.rect.y - 0.01
            && to.y <= card.rect.bottom() + 0.01;
        assert!(
            on_edge,
            "tab {}: the spoke ends at {to:?}, not on {:?}",
            card.tab, card.rect
        );
        assert!(
            dist(to, ring.centre) > from,
            "tab {}: the spoke points outward",
            card.tab
        );
    }
}

#[test]
fn the_other_groups_are_chips_along_the_bottom() {
    let layout = level2(Stage::MOCKUP, &orbit_input(false), 1);
    let ids: Vec<i32> = layout.chips.iter().map(|c| c.group).collect();
    assert_eq!(ids, vec![2, 3, 4, 5]);
    assert_eq!(layout.other_groups_label.x, 30.0);
    assert_eq!(layout.chips[0].rect.x, 128.0);
    for chip in &layout.chips {
        assert_eq!(chip.rect.h, 24.0);
        assert_eq!(chip.rect.centre().y, 858.0, "chip row at y 858");
        assert!(chip.rect.w > 40.0);
    }
    for pair in layout.chips.windows(2) {
        assert!(
            pair[1].rect.x >= pair[0].rect.right() + 20.0,
            "chips are apart"
        );
    }
    // The key hints take the right of the row: the chips stop before them.
    let last = layout.chips.last().expect("chip");
    assert!(last.rect.right() < 1440.0 - 500.0);
    // Level 1 and 3 have no chips.
    assert!(level1(Stage::MOCKUP, &orbit_input(false)).chips.is_empty());
    assert!(
        level3(Stage::MOCKUP, &orbit_input(false), 1, 104)
            .chips
            .is_empty()
    );
}

#[test]
fn many_groups_cut_the_chip_row_off_before_the_key_hints() {
    let input = generated_input(64, 200, false);
    let layout = level2(Stage::MOCKUP, &input, 1);
    assert!(!layout.chips.is_empty() && layout.chips.len() < 63);
    assert!(
        layout
            .chips
            .iter()
            .all(|c| c.rect.right() <= 1440.0 - 500.0)
    );
}

// ---- level 3: the tab ----

#[test]
fn the_level_3_card_is_68_percent_of_the_window() {
    let layout = level3(Stage::MOCKUP, &orbit_input(false), 1, 104);
    let card = layout.card.as_ref().expect("card");
    assert_eq!(card.tab, 104);
    assert!((card.rect.w - 980.0).abs() <= 1.0, "width {}", card.rect.w);
    assert!((card.rect.h - 660.0).abs() <= 1.0, "height {}", card.rect.h);
    assert!((card.rect.x - 230.0).abs() <= 1.0, "x {}", card.rect.x);
    assert!((card.rect.y - 110.0).abs() <= 1.0, "y {}", card.rect.y);
    assert!((card.rect.centre().x - 720.0).abs() < 0.01);
    for (w, h) in SIZES {
        let layout = level3(Stage::new(w, h), &orbit_input(false), 1, 104);
        let card = layout.card.as_ref().expect("card");
        assert!(card.rect.w <= 0.68 * w + 0.01, "{w}x{h}");
        assert!(card.rect.x >= 0.0 && card.rect.right() <= w, "{w}x{h}");
        assert!(
            card.rect.y >= TITLE_BAR_HEIGHT && card.rect.bottom() <= h,
            "{w}x{h}"
        );
        if w <= 1440.0 {
            assert!((card.rect.w - 0.68 * w).abs() <= 1.0, "{w}x{h}");
        }
    }
}

#[test]
fn the_huge_ring_of_level_3_is_a_bigger_spec_with_outer_radius_560() {
    let layout = level3(Stage::MOCKUP, &orbit_input(false), 1, 104);
    let ring = layout.centre.as_ref().expect("centre ring");
    assert_eq!(ring.group, 1);
    assert_eq!(ring.centre, Point::new(720.0, 470.0));
    assert_eq!(ring.spec.outer_radius(), 560.0);
    assert!(ring.ring_size / 2.0 >= ring.spec.label_outer_radius());
    let small = level3(Stage::new(900.0, 600.0), &orbit_input(false), 1, 104);
    let small_ring = small.centre.as_ref().expect("centre ring");
    assert!(
        small_ring.spec.outer_radius() < 560.0,
        "the ring scales with a small window"
    );
    assert!(layout.previews.is_empty());
}

#[test]
fn the_neighbours_are_half_off_the_stage_and_stop_at_the_ends_of_the_group() {
    let input = orbit_input(false);
    let layout = level3(Stage::MOCKUP, &input, 1, 104);
    let ids: Vec<i32> = layout.neighbours.iter().map(|n| n.tab).collect();
    assert_eq!(ids, vec![103, 105]);
    let (left, right) = (&layout.neighbours[0], &layout.neighbours[1]);
    assert_eq!(left.side, Side::Left);
    assert_eq!(right.side, Side::Right);
    assert!(
        left.rect.x < 0.0 && left.rect.right() > 100.0,
        "{:?}",
        left.rect
    );
    assert!(
        right.rect.right() > 1440.0 && right.rect.x < 1340.0,
        "{:?}",
        right.rect
    );
    assert!((left.rect.w - 420.0).abs() <= 1.0 && (left.rect.h - 300.0).abs() <= 1.0);
    let card = layout.card.as_ref().expect("card");
    assert!(left.rect.right() <= card.rect.x && right.rect.x >= card.rect.right());

    let first = level3(Stage::MOCKUP, &input, 1, 101);
    assert_eq!(
        first.neighbours.iter().map(|n| n.tab).collect::<Vec<_>>(),
        vec![102]
    );
    assert_eq!(first.neighbours[0].side, Side::Right);
    let last = level3(Stage::MOCKUP, &input, 1, 110);
    assert_eq!(
        last.neighbours.iter().map(|n| n.tab).collect::<Vec<_>>(),
        vec![109]
    );
    assert_eq!(last.neighbours[0].side, Side::Left);
    let lone = generated_input(1, 1, false);
    assert!(level3(Stage::MOCKUP, &lone, 1, 1000).neighbours.is_empty());
}

// ---- the hit test ----

fn outward(from: Point, centre: Point, by: f32) -> (f32, f32) {
    let d = dist(from, centre);
    (
        from.x + (from.x - centre.x) / d * by,
        from.y + (from.y - centre.y) / d * by,
    )
}

#[test]
fn hit_at_level_1_tells_planets_sun_new_group_and_gaps_apart() {
    let layout = level1(Stage::MOCKUP, &orbit_input(false));
    for cell in &layout.planets {
        assert_eq!(
            layout.hit(cell.centre.x, cell.centre.y),
            Target::Planet(cell.group)
        );
        // Just inside the halo, away from the sun.
        let (x, y) = outward(cell.centre, layout.sun.centre, cell.halo_radius - 1.0);
        assert_eq!(
            layout.hit(x, y),
            Target::Planet(cell.group),
            "inside the halo"
        );
        let (x, y) = outward(cell.centre, layout.sun.centre, cell.halo_radius + 2.0);
        assert_eq!(
            layout.hit(x, y),
            Target::Backdrop,
            "just outside the halo of {}",
            cell.group
        );
    }
    assert_eq!(layout.hit(720.0, 470.0), Target::Sun);
    assert_eq!(layout.hit(720.0 + 91.0, 470.0), Target::Sun);
    assert_eq!(layout.hit(1250.0, 800.0), Target::NewGroup);
    assert_eq!(layout.hit(1250.0 + 49.0, 800.0), Target::NewGroup);
    assert_eq!(layout.hit(1250.0 + 51.0, 800.0), Target::Backdrop);
    // The gap between two planets, the corners and the title bar.
    assert_eq!(layout.hit(940.0, 250.0), Target::Backdrop);
    assert_eq!(layout.hit(5.0, 60.0), Target::Backdrop);
    assert_eq!(layout.hit(720.0, 10.0), Target::Backdrop);
}

#[test]
fn a_hidden_new_group_ring_is_not_hit_at_level_2() {
    let layout = level2(Stage::MOCKUP, &orbit_input(false), 1);
    assert_eq!(layout.hit(1250.0, 800.0), Target::Backdrop);
    // Nor does the faded sun.
    assert_eq!(layout.hit(720.0, 470.0), Target::Backdrop);
    // A hidden planet's halo is not hit: this point is in the halo of group 2 and on no card.
    let planet = &layout.planets[1];
    assert_eq!(
        layout.hit(planet.centre.x, planet.centre.y - 90.0),
        Target::Backdrop
    );
}

#[test]
fn hit_at_level_2_finds_cards_and_chips_and_leaves_the_ring_to_ring_bands() {
    let layout = level2(Stage::MOCKUP, &orbit_input(false), 1);
    for card in &layout.previews {
        let c = card.rect.centre();
        assert_eq!(layout.hit(c.x, c.y), Target::Preview(card.tab));
        assert_eq!(
            layout.hit(card.rect.x + 1.0, card.rect.y + 1.0),
            Target::Preview(card.tab)
        );
        assert_eq!(layout.hit(card.rect.x - 1.0, c.y), Target::Backdrop);
    }
    for chip in &layout.chips {
        let c = chip.rect.centre();
        assert_eq!(layout.hit(c.x, c.y), Target::Chip(chip.group));
    }
    // The hole, the ring band and empty space are Orbit's "backdrop"; the ring is ring_at's.
    assert_eq!(layout.hit(720.0, 470.0), Target::Backdrop);
    assert_eq!(layout.hit(720.0, 470.0 - 92.0), Target::Backdrop);
    assert_eq!(layout.hit(10.0, 100.0), Target::Backdrop);
}

#[test]
fn where_two_cards_overlap_the_one_drawn_last_takes_the_hit() {
    let input = generated_input(1, 24, false);
    let layout = level2(Stage::MOCKUP, &input, 1);
    // At the mockup's size the two rings may not touch at all; any pair that does is checked.
    for inner in layout.previews.iter().filter(|p| p.ring == 1) {
        for outer in layout.previews.iter().filter(|p| p.ring == 0) {
            if depth(inner.rect, outer.rect) <= 0.5 {
                continue;
            }
            let x0 = inner.rect.x.max(outer.rect.x);
            let x1 = inner.rect.right().min(outer.rect.right());
            let y0 = inner.rect.y.max(outer.rect.y);
            let y1 = inner.rect.bottom().min(outer.rect.bottom());
            assert_eq!(
                layout.hit((x0 + x1) / 2.0, (y0 + y1) / 2.0),
                Target::Preview(outer.tab),
                "inner {} under outer {}",
                inner.tab,
                outer.tab
            );
        }
    }
    for card in &layout.previews {
        let c = card.rect.centre();
        let hit = layout.hit(c.x, c.y);
        assert!(matches!(hit, Target::Preview(_)), "{hit:?}");
    }
}

#[test]
fn hit_at_level_3_finds_the_card_and_the_visible_part_of_a_neighbour() {
    let layout = level3(Stage::MOCKUP, &orbit_input(false), 1, 104);
    let card = layout.card.as_ref().expect("card");
    let c = card.rect.centre();
    assert_eq!(layout.hit(c.x, c.y), Target::Card);
    assert_eq!(layout.hit(100.0, 400.0), Target::Neighbour(103));
    assert_eq!(layout.hit(1340.0, 400.0), Target::Neighbour(105));
    assert_eq!(layout.hit(720.0, 30.0), Target::Backdrop);
    assert_eq!(layout.hit(720.0, 850.0), Target::Backdrop);
    assert_eq!(
        layout.hit(card.rect.x - 1.0, c.y),
        Target::Backdrop,
        "between a neighbour and the card"
    );
}

#[test]
fn ring_at_gives_the_planet_or_the_centre_ring_and_ring_local_coordinates() {
    let input = orbit_input(false);
    let layout = level1(Stage::MOCKUP, &input);
    let cell = &layout.planets[0];
    let half = cell.ring_size / 2.0;
    let at = layout
        .ring_at(cell.centre.x, cell.centre.y - cell.radius)
        .expect("the band of the first planet");
    assert_eq!(at.group, 1);
    assert!(!at.centre);
    assert!(
        (at.x - half).abs() < 0.01 && (at.y - (half - cell.radius)).abs() < 0.01,
        "{at:?}"
    );
    assert!(layout.ring_at(940.0, 250.0).is_none());
    assert!(
        layout.ring_at(720.0, 470.0).is_none(),
        "the sun is not a ring"
    );

    let two = level2(Stage::MOCKUP, &input, 1);
    let ring = two.centre.as_ref().expect("centre ring");
    let at = two
        .ring_at(720.0, 470.0 - 92.0)
        .expect("the centre ring's band");
    assert_eq!(at.group, 1);
    assert!(at.centre);
    assert!((at.x - ring.ring_size / 2.0).abs() < 0.01);
    assert!(two.ring_at(720.0, 10.0).is_none());
    assert!(
        two.ring_at(cell.centre.x, cell.centre.y).is_none(),
        "hidden planets are not rings"
    );

    let three = level3(Stage::MOCKUP, &input, 1, 104);
    assert!(
        three.ring_at(720.0, 470.0 - 553.0).is_none(),
        "the level-3 ring is not interactive"
    );
}
