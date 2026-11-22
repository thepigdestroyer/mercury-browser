// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The data behind `OrbitView` (brief §6 item 2): what `OrbitData::build` puts in each row, the
//! SPEC §5 wording of the labels, the persistent models of `OrbitModels` and the build time.
//! Needs `--features test-support` for the fixtures. Tests named `ring_bands_*` read the arrays that
//! the finished `ring_bands::RingBandsData` fills; they pass once that lane is merged.
#![cfg(feature = "test-support")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

use mercury_ui::orbit_view::data::{Options, OrbitData, OrbitModels, PlanetData};
use mercury_ui::orbit_view::fixtures::{
    generated_input, orbit_groups, orbit_input, orbit_tabs, skeleton_picture,
};
use mercury_ui::orbit_view::layout::OrbitLayout;
use mercury_ui::orbit_view::{
    DragView, DropTarget, LOOSE_GROUP_ID, Level, NewGroupPopover, OrbitInput, Source, Stage,
    Target, View,
};
use mercury_ui::ring_bands::{RingBandsData, RingOptions};
use mercury_ui::ring_geometry::{ApproxMetrics, point_on_circle};
use mercury_ui::{FlagKind, FlagPin};
use slint::{Color, Image, Model};

thread_local! {
    static PLATFORM_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Installs Slint's testing platform on this test thread, once. `OrbitData::build` measures the
/// ring labels through `ring_bands::metrics()`, which needs a platform; call this first in every
/// test that uses `build` (the `build_with` tests pass their own metrics).
fn platform() {
    PLATFORM_READY.with(|ready| {
        if !ready.replace(true) {
            i_slint_backend_testing::init_no_event_loop();
        }
    });
}

fn view(level: Level, group: Option<i32>, tab: Option<i32>) -> View {
    View {
        level,
        group,
        tab,
        ..View::default()
    }
}

fn level1() -> View {
    View::default()
}

fn level2(group: i32) -> View {
    view(Level::Group, Some(group), None)
}

fn level3(group: i32, tab: i32) -> View {
    view(Level::Tab, Some(group), Some(tab))
}

fn no_pictures(_: i32) -> Option<Image> {
    None
}

fn build_with(input: &OrbitInput, view: &View) -> OrbitData {
    build_at(Stage::MOCKUP, input, view)
}

fn build_at(stage: Stage, input: &OrbitInput, view: &View) -> OrbitData {
    OrbitData::build_with(
        stage,
        input,
        view,
        &no_pictures,
        &Options::default(),
        &ApproxMetrics,
    )
}

fn angle_of(x: f32, y: f32) -> f32 {
    (x.atan2(-y).to_degrees()).rem_euclid(360.0)
}

fn pins_kinds(pins: &[FlagPin]) -> Vec<FlagKind> {
    pins.iter().map(|p| p.kind).collect()
}

// ---- level 1 ----

#[test]
fn the_planet_rows_follow_the_groups() {
    let data = build_with(&orbit_input(false), &level1());
    let ids: Vec<i32> = data.planets.iter().map(|p| p.row.group).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    let names: Vec<String> = data
        .planets
        .iter()
        .map(|p| p.row.name.to_string())
        .collect();
    assert_eq!(
        names,
        ["Research", "Lisbon trip", "Reading", "Shopping", "Work"]
    );
    for (cell, planet) in data.layout.planets.iter().zip(&data.planets) {
        assert_eq!((planet.row.x, planet.row.y), (cell.shown.x, cell.shown.y));
        assert_eq!(planet.row.scale, cell.scale);
        assert_eq!(planet.row.ring_size, cell.ring_size);
        assert_eq!(planet.row.halo_radius, cell.halo_radius);
        assert_eq!(planet.row.opacity, 1.0);
        assert!(!planet.row.dimmed);
    }
    let many = build_with(&generated_input(64, 200, false), &level1());
    assert_eq!(many.planets.len(), 64);
}

#[test]
fn the_labels_use_the_spec_wording() {
    let data = build_with(&orbit_input(false), &level1());
    let label = |i: usize| data.planets[i].row.accessible_label.to_string();
    assert_eq!(label(0), "Research, tab group, 10 tabs, key Option A");
    assert_eq!(label(1), "Lisbon trip, tab group, 8 tabs, key Option S");
    assert_eq!(label(4), "Work, tab group, 6 tabs, key Option G");
    assert_eq!(
        data.planets[0].row.tooltip,
        "Research · click or pinch in to zoom"
    );
    assert_eq!(data.planets[0].row.hole_text, "2 min ago");
    assert_eq!(data.planets[2].row.hole_text, "now");
    // A group with one tab and no key.
    let mut tabs = orbit_tabs();
    tabs[0].pinned = true;
    let input = OrbitInput::new(
        &orbit_groups(false),
        &tabs,
        "Personal",
        "Window 1",
        102,
        false,
    );
    let data = build_with(&input, &level1());
    let loose = data.planets.last().expect("pseudo planet");
    assert_eq!(loose.row.group, LOOSE_GROUP_ID);
    assert_eq!(loose.row.accessible_label, "Loose tabs, tab group, 1 tab");
    assert_eq!(loose.row.hole_text, "");
}

#[test]
fn the_tooltip_rectangle_comes_from_the_layout() {
    let data = build_with(&orbit_input(false), &level1());
    for (cell, planet) in data.layout.planets.iter().zip(&data.planets) {
        assert_eq!(
            (planet.row.tooltip_x, planet.row.tooltip_y),
            (cell.tooltip.x, cell.tooltip.y)
        );
    }
}

#[test]
fn hover_and_focus_light_a_planet() {
    let mut v = level1();
    v.hover = Some(Target::Planet(2));
    v.focus = Some(Target::Planet(4));
    let data = build_with(&orbit_input(false), &v);
    let lit: Vec<bool> = data.planets.iter().map(|p| p.row.lit).collect();
    assert_eq!(lit, vec![false, true, false, true, false]);
}

#[test]
fn the_sun_says_the_profile_and_the_window_with_all_tabs() {
    let data = build_with(&orbit_input(false), &level1());
    assert_eq!(data.sun.title, "Personal");
    assert_eq!(data.sun.subtitle, "Window 1 · 40 tabs");
    assert_eq!(
        (data.sun.x, data.sun.y, data.sun.radius),
        (720.0, 470.0, 92.0)
    );
    assert_eq!(data.sun.opacity, 1.0);
    // Loose and pinned tabs count too; one tab is "1 tab".
    let one = generated_input(1, 1, false);
    assert_eq!(build_with(&one, &level1()).sun.subtitle, "Window 1 · 1 tab");
    assert_eq!(build_with(&orbit_input(false), &level2(1)).sun.opacity, 0.0);
}

#[test]
fn the_new_group_ring_is_there_at_level_1_only() {
    let data = build_with(&orbit_input(false), &level1());
    assert!(data.new_group.visible);
    assert_eq!(
        (data.new_group.x, data.new_group.y, data.new_group.radius),
        (1250.0, 800.0, 50.0)
    );
    assert!(!data.new_group.lit);
    assert!(
        !build_with(&orbit_input(false), &level2(1))
            .new_group
            .visible
    );
}

#[test]
fn the_group_badges_sit_on_the_label_band_at_the_bottom() {
    let data = build_with(&orbit_input(false), &level1());
    let kinds = |i: usize| pins_kinds(&data.planets[i].badges);
    assert_eq!(kinds(0), vec![FlagKind::Spinner]);
    assert_eq!(kinds(1), vec![FlagKind::Unread]);
    assert_eq!(kinds(2), vec![FlagKind::Audible]);
    assert_eq!(kinds(3), vec![]);
    assert_eq!(kinds(4), vec![FlagKind::Unread]);
    for (i, planet) in data.planets.iter().enumerate() {
        let cell = &data.layout.planets[i];
        let half = cell.ring_size / 2.0;
        for pin in &planet.badges {
            let (dx, dy) = (pin.x - half, pin.y - half);
            assert!(
                (dx.hypot(dy) - cell.spec.label_radius()).abs() < 0.01,
                "group {}: the badge is on the label band's middle",
                planet.row.group
            );
            assert!(
                (angle_of(dx, dy) - 200.0).abs() < 0.1,
                "a single badge is at 200 degrees"
            );
            assert_eq!(pin.tab, -1, "a group badge belongs to no tab");
        }
    }
    let spinner = &data.planets[0].badges[0];
    assert_eq!(spinner.size, 18.0);
    assert_eq!(
        data.planets[1].badges[0].size, 9.0,
        "the unread dot is small"
    );
}

#[test]
fn three_badges_spread_over_180_plus_and_minus_20_degrees() {
    let mut groups = orbit_groups(false);
    groups[3].any_loading = true;
    groups[3].any_audible = true;
    groups[3].any_unread = true;
    let input = OrbitInput::new(&groups, &orbit_tabs(), "P", "W", 101, false);
    let data = build_with(&input, &level1());
    let planet = &data.planets[3];
    assert_eq!(
        pins_kinds(&planet.badges),
        vec![FlagKind::Spinner, FlagKind::Audible, FlagKind::Unread]
    );
    let half = data.layout.planets[3].ring_size / 2.0;
    let angles: Vec<f32> = planet
        .badges
        .iter()
        .map(|p| angle_of(p.x - half, p.y - half))
        .collect();
    for (got, want) in angles.iter().zip([200.0, 180.0, 160.0]) {
        assert!((got - want).abs() < 0.1, "{angles:?}");
    }
    groups[3].any_unread = false;
    let input = OrbitInput::new(&groups, &orbit_tabs(), "P", "W", 101, false);
    let two = build_with(&input, &level1());
    let half = two.layout.planets[3].ring_size / 2.0;
    let angles: Vec<f32> = two.planets[3]
        .badges
        .iter()
        .map(|p| angle_of(p.x - half, p.y - half))
        .collect();
    assert!(
        (angles[0] - 200.0).abs() < 0.1 && (angles[1] - 180.0).abs() < 0.1,
        "{angles:?}"
    );
}

#[test]
fn the_crumbs_follow_the_level() {
    let input = orbit_input(false);
    let crumbs = |v: &View| -> Vec<String> {
        build_with(&input, v)
            .crumbs
            .iter()
            .map(|c| c.to_string())
            .collect()
    };
    assert_eq!(crumbs(&level1()), ["Orbit"]);
    assert_eq!(crumbs(&level2(1)), ["Orbit", "Research"]);
    assert_eq!(
        crumbs(&level3(1, 104)),
        ["Orbit", "Research", "AccessKit guide"]
    );
}

#[test]
fn level_1_asks_for_no_pictures() {
    let asked = RefCell::new(Vec::new());
    let snapshots = |tab: i32| {
        asked.borrow_mut().push(tab);
        None
    };
    OrbitData::build_with(
        Stage::MOCKUP,
        &orbit_input(false),
        &level1(),
        &snapshots,
        &Options::default(),
        &ApproxMetrics,
    );
    assert!(asked.borrow().is_empty());
}

// ---- level 2 ----

#[test]
fn the_previews_carry_title_host_letter_and_shade() {
    let input = orbit_input(false);
    let data = build_with(&input, &level2(1));
    assert_eq!(data.previews.len(), 10);
    assert_eq!(data.more_previews, 0);
    let mut by_tab: HashMap<i32, &_> = HashMap::new();
    for card in &data.previews {
        by_tab.insert(card.row.tab, card);
    }
    let first = by_tab[&101];
    assert_eq!(first.row.title, "Servo layout notes");
    assert_eq!(first.row.host, "docs.example.org");
    assert_eq!(first.row.letter, "S");
    assert_eq!(
        first.row.shade,
        input.tab(101).map(|t| t.shade).unwrap_or_default()
    );
    assert!(first.row.current, "the active tab is the current card");
    assert_eq!(by_tab.values().filter(|c| c.row.current).count(), 1);
    assert!(first.row.draggable);
    assert!(!first.row.has_picture, "no snapshot: the placeholder");
    assert_eq!((first.row.x, first.row.y), {
        let cell = data
            .layout
            .previews
            .iter()
            .find(|p| p.tab == 101)
            .expect("cell");
        (cell.rect.centre().x, cell.rect.centre().y)
    });
}

#[test]
fn the_inner_ring_is_first_in_the_rows_too() {
    let input = generated_input(1, 30, false);
    let data = build_with(&input, &level2(1));
    assert_eq!(data.previews.len(), 24);
    assert_eq!(data.more_previews, 6);
    let widths: Vec<f32> = data.previews.iter().map(|p| p.row.width).collect();
    assert!(
        widths[..12].iter().all(|w| *w < widths[12]),
        "inner cards are smaller and first"
    );
    assert_eq!(data.layout.previews.len(), data.previews.len());
}

#[test]
fn a_snapshot_is_asked_for_once_per_shown_tab() {
    let asked = RefCell::new(Vec::new());
    let picture = skeleton_picture(false);
    let snapshots = |tab: i32| {
        asked.borrow_mut().push(tab);
        Some(picture.clone())
    };
    let build = |v: &View| {
        asked.borrow_mut().clear();
        let data = OrbitData::build_with(
            Stage::MOCKUP,
            &orbit_input(false),
            v,
            &snapshots,
            &Options::default(),
            &ApproxMetrics,
        );
        let mut tabs = asked.borrow().clone();
        tabs.sort_unstable();
        (data, tabs)
    };
    let (data, tabs) = build(&level2(1));
    assert_eq!(tabs, (101..=110).collect::<Vec<_>>());
    assert!(data.previews.iter().all(|p| p.row.has_picture));
    assert!(
        data.previews
            .iter()
            .all(|p| p.row.picture.size().width == 480)
    );
    let (data, tabs) = build(&level3(1, 104));
    assert_eq!(tabs, vec![103, 104, 105]);
    assert!(data.card.has_picture);
    assert!(data.neighbours.iter().all(|n| n.has_picture));
}

#[test]
fn a_card_is_lit_and_lifted_when_hovered_or_focused() {
    let mut v = level2(1);
    v.hover = Some(Target::Preview(102));
    v.focus = Some(Target::Preview(105));
    let data = build_with(&orbit_input(false), &v);
    for card in &data.previews {
        let lit = card.row.tab == 102 || card.row.tab == 105;
        assert_eq!(card.row.lit, lit, "tab {}", card.row.tab);
        assert_eq!(
            card.row.scale,
            if card.row.tab == 102 { 1.04 } else { 1.0 },
            "tab {}",
            card.row.tab
        );
    }
}

#[test]
fn a_pinned_tab_is_not_draggable() {
    let mut tabs = orbit_tabs();
    tabs[0].pinned = true;
    tabs[1].group = -1;
    let input = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 103, false);
    let data = build_with(&input, &level2(LOOSE_GROUP_ID));
    assert_eq!(data.previews.len(), 2);
    let drag = |tab: i32| {
        data.previews
            .iter()
            .find(|p| p.row.tab == tab)
            .map(|p| p.row.draggable)
    };
    assert_eq!(drag(101), Some(false), "pinned");
    assert_eq!(drag(102), Some(true), "loose");
}

#[test]
fn the_card_flags_follow_the_tab_states() {
    let data = build_with(&orbit_input(false), &level2(1));
    let card = data
        .previews
        .iter()
        .find(|c| c.row.tab == 102)
        .expect("WebRender overview");
    assert_eq!(pins_kinds(&card.flags), vec![FlagKind::Spinner]);
    assert_eq!(card.flags[0].tab, 102);
    assert!((card.flags[0].progress - 0.64).abs() < 1e-4);
    assert_eq!(card.flags[0].size, 20.0);
    // Card-local: 20 px from the right edge, 8 px from the top (the mockup's badge).
    assert_eq!(
        (card.flags[0].x, card.flags[0].y),
        (card.row.width - 20.0, 8.0)
    );
    let quiet = data
        .previews
        .iter()
        .find(|c| c.row.tab == 103)
        .expect("Slint reference");
    assert!(quiet.flags.is_empty());

    let lisbon = build_with(&orbit_input(false), &level2(2));
    let unread = lisbon
        .previews
        .iter()
        .find(|c| c.row.tab == 112)
        .expect("unread tab");
    assert_eq!(pins_kinds(&unread.flags), vec![FlagKind::Unread]);
    let reading = build_with(&orbit_input(false), &level2(3));
    let loud = reading
        .previews
        .iter()
        .find(|c| c.row.tab == 127)
        .expect("audible tab");
    assert_eq!(pins_kinds(&loud.flags), vec![FlagKind::Audible]);
}

#[test]
fn a_card_reads_as_title_group_position_and_states() {
    let data = build_with(&orbit_input(false), &level2(1));
    let label = |tab: i32| {
        data.previews
            .iter()
            .find(|c| c.row.tab == tab)
            .map(|c| c.row.accessible_label.to_string())
            .unwrap_or_default()
    };
    assert_eq!(label(101), "Servo layout notes, Research, tab 1 of 10");
    assert_eq!(
        label(102),
        "WebRender overview, Research, tab 2 of 10, loading 64 percent"
    );
    assert_eq!(label(110), "Style guide, Research, tab 10 of 10");
    let reading = build_with(&orbit_input(false), &level2(3));
    let loud = reading
        .previews
        .iter()
        .find(|c| c.row.tab == 127)
        .expect("audible tab");
    assert_eq!(
        loud.row.accessible_label,
        "Interview: Servo, Reading, tab 9 of 9, playing sound"
    );
    let mut tabs = orbit_tabs();
    tabs[0].pinned = true;
    tabs[0].unread = true;
    let input = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
    let loose = build_with(&input, &level2(LOOSE_GROUP_ID));
    assert_eq!(
        loose.previews[0].row.accessible_label,
        "Servo layout notes, Loose tabs, tab 1 of 1, unread, pinned"
    );
}

#[test]
fn the_centre_ring_says_how_many_tabs_and_when() {
    let data = build_with(&orbit_input(false), &level2(1));
    let centre = data.centre.as_ref().expect("centre ring");
    assert_eq!(centre.row.group, 1);
    assert_eq!(centre.row.name, "Research");
    assert_eq!(centre.row.hole_text, "10 tabs · 2 min ago");
    assert_eq!((centre.row.x, centre.row.y), (720.0, 470.0));
    assert_eq!(centre.row.opacity, 1.0);
    assert_eq!(
        centre.row.accessible_label,
        "Research, tab group, 10 tabs, key Option A"
    );
    assert!(build_with(&orbit_input(false), &level1()).centre.is_none());
    let lone = build_with(&generated_input(1, 1, false), &level2(1));
    assert_eq!(
        lone.centre.as_ref().map(|c| c.row.hole_text.to_string()),
        Some("1 tab · now".into())
    );
}

#[test]
fn the_chips_name_the_other_groups_with_their_keys() {
    let data = build_with(&orbit_input(false), &level2(1));
    assert_eq!(data.chips.len(), 4);
    let names: Vec<String> = data.chips.iter().map(|c| c.name.to_string()).collect();
    assert_eq!(names, ["Lisbon trip", "Reading", "Shopping", "Work"]);
    assert_eq!(data.chips[0].key_label, "⌥S");
    assert_eq!(data.chips[0].group, 2);
    assert_eq!(
        data.chips[0].accessible_label,
        "Lisbon trip, tab group, 8 tabs, key Option S"
    );
    let groups = orbit_groups(false);
    assert_eq!(data.chips[0].colour, groups[1].colour);
    for (row, cell) in data.chips.iter().zip(&data.layout.chips) {
        assert_eq!(
            (row.x, row.y, row.width, row.height),
            (cell.rect.x, cell.rect.y, cell.rect.w, cell.rect.h)
        );
    }
    assert!(build_with(&orbit_input(false), &level1()).chips.is_empty());
}

#[test]
fn the_spokes_are_the_layouts() {
    let data = build_with(&orbit_input(false), &level2(1));
    for (row, cell) in data.previews.iter().zip(&data.layout.previews) {
        assert_eq!(
            (row.row.spoke_x1, row.row.spoke_y1),
            (cell.spoke_from.x, cell.spoke_from.y)
        );
        assert_eq!(
            (row.row.spoke_x2, row.row.spoke_y2),
            (cell.spoke_to.x, cell.spoke_to.y)
        );
    }
}

// ---- level 3 ----

#[test]
fn the_level_3_card_says_group_and_position() {
    let data = build_with(&orbit_input(false), &level3(1, 104));
    let card = &data.card;
    assert!(card.visible);
    assert_eq!(card.tab, 104);
    assert_eq!(card.title, "AccessKit guide");
    assert_eq!(card.host, "a11y.example.net");
    assert_eq!(card.caption, "Research · tab 4 of 10");
    assert_eq!(card.letter, "A");
    assert_eq!(card.accessible_label, "Open AccessKit guide");
    assert!(!card.has_picture);
    let rect = data.layout.card.as_ref().expect("card cell").rect;
    assert_eq!(
        (card.x, card.y, card.width, card.height),
        (rect.x, rect.y, rect.w, rect.h)
    );
    assert_eq!(card.group_colour, orbit_groups(false)[0].colour);
    assert!(!build_with(&orbit_input(false), &level2(1)).card.visible);
    assert!(!build_with(&orbit_input(false), &level1()).card.visible);
}

#[test]
fn the_neighbours_are_the_previous_and_the_next_tab() {
    let data = build_with(&orbit_input(false), &level3(1, 104));
    let titles: Vec<String> = data
        .neighbours
        .iter()
        .map(|n| n.title.to_string())
        .collect();
    assert_eq!(titles, ["Slint reference", "Forum: window dragging"]);
    assert_eq!(data.neighbours[0].tab, 103);
    assert_eq!(data.neighbours[0].letter, "S");
    assert_eq!(
        data.neighbours[0].accessible_label,
        "Slint reference, previous tab"
    );
    assert_eq!(
        data.neighbours[1].accessible_label,
        "Forum: window dragging, next tab"
    );
    let first = build_with(&orbit_input(false), &level3(1, 101));
    assert_eq!(first.neighbours.len(), 1);
    assert!(
        build_with(&orbit_input(false), &level2(1))
            .neighbours
            .is_empty()
    );
}

// ---- drag and drop ----

fn dragging(source: Source, over: Option<DropTarget>) -> View {
    View {
        drag: Some(DragView { source, over }),
        ..level1()
    }
}

#[test]
fn a_tab_over_another_planet_lights_it_and_marks_the_slot_with_a_bar() {
    let data = build_with(
        &orbit_input(false),
        &dragging(
            Source::Tab(104),
            Some(DropTarget::Group {
                group: 2,
                slot: Some(3),
            }),
        ),
    );
    let lit: Vec<bool> = data.planets.iter().map(|p| p.row.drop_lit).collect();
    assert_eq!(lit, vec![false, true, false, false, false]);
    let target = &data.planets[1];
    assert!(target.row.has_drop_bar);
    let cell = &data.layout.planets[1];
    let half = cell.ring_size / 2.0;
    let wanted = point_on_circle(
        mercury_ui::ring_geometry::Point::new(half, half),
        cell.radius,
        135.0,
    );
    assert!((target.row.drop_bar.mid_x - wanted.x).abs() < 0.01);
    assert!((target.row.drop_bar.mid_y - wanted.y).abs() < 0.01);
    assert!(target.row.drop_bar.commands.starts_with('M'));
    assert!(target.row.drop_bar.commands.ends_with('Z'));
    // The source ring dashes its slot; the others do not.
    let dashed: Vec<i32> = data.planets.iter().map(|p| p.row.dragging_tab).collect();
    assert_eq!(dashed, vec![104, -1, -1, -1, -1]);
    assert!(data.planets.iter().filter(|p| p.row.has_drop_bar).count() == 1);
}

#[test]
fn a_halo_drop_with_no_slot_puts_the_bar_at_the_end() {
    let data = build_with(
        &orbit_input(false),
        &dragging(
            Source::Tab(104),
            Some(DropTarget::Group {
                group: 4,
                slot: None,
            }),
        ),
    );
    let target = &data.planets[3];
    assert!(target.row.drop_lit && target.row.has_drop_bar);
    // The end of a 7-tab ring is its start: 360 degrees, straight up.
    let cell = &data.layout.planets[3];
    let half = cell.ring_size / 2.0;
    assert!((target.row.drop_bar.mid_x - half).abs() < 0.01);
    assert!((target.row.drop_bar.mid_y - (half - cell.radius)).abs() < 0.01);
}

#[test]
fn a_tab_over_its_own_planet_does_not_light_it_and_its_own_slots_show_no_bar() {
    let own = |slot: Option<usize>| {
        build_with(
            &orbit_input(false),
            &dragging(Source::Tab(104), Some(DropTarget::Group { group: 1, slot })),
        )
    };
    // Tab 104 is the fourth tab (index 3): slots 3 and 4 leave it where it is.
    for slot in [3, 4] {
        let data = own(Some(slot));
        assert!(!data.planets[0].row.drop_lit);
        assert!(
            !data.planets[0].row.has_drop_bar,
            "slot {slot} changes nothing"
        );
    }
    let data = own(Some(7));
    assert!(!data.planets[0].row.drop_lit, "its own planet is not lit");
    assert!(
        data.planets[0].row.has_drop_bar,
        "but moving within the group shows the slot"
    );
}

#[test]
fn a_tab_over_empty_space_or_new_group_lights_the_new_group_ring() {
    let lit = |source, over| {
        build_with(&orbit_input(false), &dragging(source, over))
            .new_group
            .lit
    };
    assert!(lit(Source::Tab(104), Some(DropTarget::Empty)));
    assert!(lit(Source::Tab(104), Some(DropTarget::NewGroup)));
    assert!(!lit(Source::Tab(104), None));
    assert!(!lit(
        Source::Tab(104),
        Some(DropTarget::Group {
            group: 2,
            slot: None
        })
    ));
    assert!(
        !lit(Source::Group(2), Some(DropTarget::NewGroup)),
        "a group cannot make a group"
    );
    assert!(!build_with(&orbit_input(false), &level1()).new_group.lit);
}

#[test]
fn the_ghost_names_the_tab_and_the_dashed_line_starts_at_its_slot() {
    let data = build_with(
        &orbit_input(false),
        &dragging(Source::Tab(104), Some(DropTarget::NewGroup)),
    );
    let drag = &data.drag;
    assert!(drag.active);
    assert_eq!(drag.ghost_text, "AccessKit guide");
    assert_eq!(drag.ghost_colour, orbit_groups(false)[0].colour);
    assert!(drag.has_from);
    // The middle of the fourth of ten slots on Research's tab band.
    let cell = &data.layout.planets[0];
    let at = point_on_circle(cell.centre, cell.radius, 3.5 * 36.0);
    assert!((drag.from_x - at.x).abs() < 0.01 && (drag.from_y - at.y).abs() < 0.01);
    // Over "New group" the ghost starts on its ring.
    assert_eq!((drag.ghost_x, drag.ghost_y), (1250.0, 800.0));
    assert!(!build_with(&orbit_input(false), &level1()).drag.active);
}

#[test]
fn dragging_a_planet_names_the_group_and_starts_at_its_centre() {
    let data = build_with(
        &orbit_input(false),
        &dragging(
            Source::Group(3),
            Some(DropTarget::Group {
                group: 5,
                slot: None,
            }),
        ),
    );
    assert!(data.drag.active);
    assert_eq!(data.drag.ghost_text, "Reading");
    assert_eq!(data.drag.ghost_colour, orbit_groups(false)[2].colour);
    let cell = &data.layout.planets[2];
    assert_eq!(
        (data.drag.from_x, data.drag.from_y),
        (cell.centre.x, cell.centre.y)
    );
    assert!(data.planets[4].row.drop_lit);
    assert!(!data.planets[2].row.drop_lit, "a group onto itself");
    assert!(
        data.planets.iter().all(|p| !p.row.has_drop_bar),
        "a planet has no slot bar"
    );
    assert!(data.planets.iter().all(|p| p.row.dragging_tab == -1));
}

#[test]
fn a_tab_dragged_from_a_preview_starts_at_its_card() {
    let mut v = level2(1);
    v.drag = Some(DragView {
        source: Source::Tab(104),
        over: Some(DropTarget::Group {
            group: 1,
            slot: Some(7),
        }),
    });
    let data = build_with(&orbit_input(false), &v);
    let card = data
        .layout
        .previews
        .iter()
        .find(|p| p.tab == 104)
        .expect("card");
    assert_eq!(
        (data.drag.from_x, data.drag.from_y),
        (card.rect.centre().x, card.rect.centre().y)
    );
    let centre = data.centre.as_ref().expect("centre ring");
    assert_eq!(centre.row.dragging_tab, 104);
    assert!(
        centre.row.has_drop_bar,
        "dropping on the centre ring at slot 7 marks it"
    );
    assert!(!centre.row.drop_lit);
}

#[test]
fn the_popover_shows_the_name_and_the_chosen_dot() {
    let mut v = level1();
    v.popover = Some(NewGroupPopover {
        tab: 104,
        name: "Untitled group".into(),
        colour: "amber".into(),
    });
    let data = build_with(&orbit_input(false), &v);
    assert!(data.popover.visible);
    assert_eq!(data.popover.name, "Untitled group");
    assert_eq!(data.popover.selected, 1);
    // Above the New group ring and inside the stage.
    assert!(data.popover.x >= 0.0 && data.popover.x + 230.0 <= 1440.0);
    assert!(data.popover.y >= 48.0 && data.popover.y + 100.0 <= 800.0 - 50.0);
    assert!(!build_with(&orbit_input(false), &level1()).popover.visible);
    v.popover = Some(NewGroupPopover::default());
    assert_eq!(
        build_with(&orbit_input(false), &v).popover.selected,
        -1,
        "no colour chosen yet"
    );
}

// ---- the models ----

#[test]
fn update_keeps_every_model_identity_and_follows_the_row_counts() {
    let models = OrbitModels::new();
    let planets = models.planets();
    let previews = models.previews();
    let chips = models.chips();
    let neighbours = models.neighbours();
    let crumbs = models.crumbs();

    models.update(&build_with(&orbit_input(false), &level1()));
    assert_eq!(planets.row_count(), 5);
    assert_eq!(
        (
            previews.row_count(),
            chips.row_count(),
            neighbours.row_count()
        ),
        (0, 0, 0)
    );
    assert_eq!(crumbs.row_count(), 1);
    assert!(!models.has_centre());
    assert_eq!(models.sun().title, "Personal");
    assert!(models.new_group().visible);

    models.update(&build_with(&orbit_input(false), &level2(1)));
    assert_eq!(planets.row_count(), 5, "the planets stay (they slide out)");
    assert_eq!(previews.row_count(), 10);
    assert_eq!(chips.row_count(), 4);
    assert_eq!(crumbs.row_count(), 2);
    assert!(models.has_centre());
    assert_eq!(models.centre().name, "Research");
    assert_eq!(models.more_previews(), 0);
    assert!(!models.new_group().visible);

    models.update(&build_with(&orbit_input(false), &level3(1, 104)));
    assert_eq!(previews.row_count(), 0);
    assert_eq!(neighbours.row_count(), 2);
    assert!(models.card().visible);
    assert_eq!(models.card().caption, "Research · tab 4 of 10");

    models.update(&build_with(&generated_input(64, 200, false), &level1()));
    assert_eq!(planets.row_count(), 64);
    models.update(&build_with(&orbit_input(false), &level1()));
    assert_eq!(planets.row_count(), 5);
    assert!(!models.card().visible);
    assert!(
        models.planets() == planets,
        "the same ModelRc after every update"
    );
    assert!(models.previews() == previews && models.chips() == chips);
    assert!(models.neighbours() == neighbours && models.crumbs() == crumbs);
}

#[test]
fn a_planets_ring_models_keep_their_identity_while_the_planet_exists() {
    let models = OrbitModels::new();
    models.update(&build_with(&orbit_input(false), &level1()));
    let first = models.planets().row_data(0).expect("row 0");
    let last = models.planets().row_data(4).expect("row 4");
    assert!(
        first.segments != last.segments,
        "one set of models per planet"
    );

    models.update(&build_with(&generated_input(64, 200, false), &level1()));
    let again = models.planets().row_data(0).expect("row 0");
    assert!(again.segments == first.segments && again.tabs == first.tabs);
    assert!(again.groups == first.groups && again.flags == first.flags);
    assert!(again.label_glyphs == first.label_glyphs && again.badges == first.badges);
    assert!(
        again.lifted_segments == first.lifted_segments && again.label_bands == first.label_bands
    );
    assert!(again.segment_labels == first.segment_labels);

    models.update(&build_with(&orbit_input(false), &level1()));
    models.update(&build_with(&generated_input(64, 200, false), &level1()));
    let back = models.planets().row_data(0).expect("row 0");
    assert!(back.segments == first.segments);
}

#[test]
fn the_models_hold_the_six_colour_dots_of_the_popover() {
    let models = OrbitModels::new();
    let mut v = level1();
    v.popover = Some(NewGroupPopover {
        tab: 104,
        name: "".into(),
        colour: "violet".into(),
    });
    models.update(&build_with(&orbit_input(false), &v));
    let popover = models.popover();
    assert!(popover.visible);
    assert_eq!(popover.selected, 2);
    let names: Vec<String> = popover.colour_names.iter().map(|n| n.to_string()).collect();
    assert_eq!(names, ["blue", "amber", "violet", "rose", "green", "teal"]);
    let colours: Vec<Color> = popover.colours.iter().collect();
    assert_eq!(colours.len(), 6);
    assert_eq!(colours[1], Color::from_rgb_u8(0xE0, 0x91, 0x2A), "amber");
    assert_eq!(colours[5], Color::from_rgb_u8(0x16, 0x9C, 0x9C), "teal");
}

#[test]
fn the_pseudo_planet_appears_exactly_when_loose_or_pinned_tabs_exist() {
    let plain = build_with(&orbit_input(false), &level1());
    assert!(plain.planets.iter().all(|p| p.row.group != LOOSE_GROUP_ID));
    let mut tabs = orbit_tabs();
    tabs[3].group = -1;
    let loose = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
    let data = build_with(&loose, &level1());
    assert_eq!(data.planets.len(), 6);
    assert_eq!(data.planets[5].row.group, LOOSE_GROUP_ID);
    assert_eq!(data.planets[5].row.name, "Loose tabs");
    let mut tabs = orbit_tabs();
    tabs[3].pinned = true;
    let pinned = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
    assert_eq!(build_with(&pinned, &level1()).planets.len(), 6);
    let mut tabs = orbit_tabs();
    tabs[3].pinned = false;
    let none = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
    assert_eq!(build_with(&none, &level1()).planets.len(), 5);
}

#[test]
fn the_build_that_keeps_its_own_metrics_gives_the_same_rows() {
    platform();
    let input = orbit_input(false);
    let data = OrbitData::build(
        Stage::MOCKUP,
        &input,
        &level1(),
        &no_pictures,
        &Options::default(),
    );
    assert_eq!(data.planets.len(), 5);
    assert_eq!(
        data.layout,
        OrbitLayout::new(Stage::MOCKUP, &input, &level1())
    );
}

// ---- needs the finished ring-bands ----

#[test]
fn ring_bands_every_planet_ring_has_one_segment_per_tab() {
    let input = orbit_input(false);
    let data = build_with(&input, &level1());
    for (planet, group) in data.planets.iter().zip(&input.groups) {
        assert_eq!(
            planet.ring.segments.len() as i32,
            group.count,
            "{}",
            group.name
        );
        assert_eq!(planet.ring.tabs.len() as i32, group.count);
        assert_eq!(planet.ring.groups.len(), 1);
        assert_eq!(planet.ring.label_bands.len(), 1);
        assert!(!planet.ring.label_glyphs.is_empty(), "{}", group.name);
    }
}

#[test]
fn ring_bands_the_models_carry_the_ring_arrays() {
    let models = OrbitModels::new();
    models.update(&build_with(&orbit_input(false), &level1()));
    let research = models.planets().row_data(0).expect("Research");
    assert_eq!(research.segments.row_count(), 10);
    assert_eq!(research.tabs.row_count(), 10);
    assert_eq!(research.lifted_segments.row_count(), 10);
    assert_eq!(research.segment_labels.row_count(), 10);
    assert_eq!(research.label_bands.row_count(), 1);
    assert_eq!(research.groups.row_count(), 1);
    assert_eq!(research.badges.row_count(), 1, "Research has a loading tab");
    models.update(&build_with(&orbit_input(false), &level2(1)));
    assert_eq!(
        models.centre().segments.row_count(),
        10,
        "the centre ring is a ring too"
    );
}

// ---- the labels are at the top ----

/// Every label glyph is above the ring's centre, the label reads left to right and its middle is on
/// the vertical through the centre.
fn assert_label_on_top(planet: &PlanetData, ring_size: f32, what: &str) {
    let half = ring_size / 2.0;
    let glyphs = &planet.ring.label_glyphs;
    assert!(!glyphs.is_empty(), "{what}: no label");
    for glyph in glyphs {
        assert!(
            glyph.y < half,
            "{what}: glyph {:?} at y {} is below the centre {half}",
            glyph.text,
            glyph.y
        );
    }
    let (first, last) = (&glyphs[0], &glyphs[glyphs.len() - 1]);
    assert!(
        first.x < last.x,
        "{what}: the label does not read left to right"
    );
    assert!(
        ((first.x + last.x) / 2.0 - half).abs() < 4.0,
        "{what}: the label is not centred on the top"
    );
}

#[test]
fn every_planets_label_is_centred_at_the_top_of_its_ring() {
    let data = build_with(&orbit_input(false), &level1());
    for (planet, cell) in data.planets.iter().zip(&data.layout.planets) {
        assert_label_on_top(planet, cell.ring_size, planet.row.name.as_str());
    }
    // Crowded: many small planets keep the rule wherever a label is shown.
    let many = build_with(&generated_input(12, 72, false), &level1());
    for (planet, cell) in many.planets.iter().zip(&many.layout.planets) {
        if !planet.ring.label_glyphs.is_empty() {
            assert_label_on_top(planet, cell.ring_size, "12 groups");
        }
    }
}

#[test]
fn the_centre_ring_and_the_big_ring_have_their_label_on_top_too() {
    for view in [level2(1), level3(1, 104)] {
        let data = build_with(&orbit_input(false), &view);
        let centre = data.centre.as_ref().expect("centre ring");
        let ring_size = data.layout.centre.as_ref().expect("cell").ring_size;
        assert_label_on_top(centre, ring_size, &format!("{:?}", view.level));
    }
}

#[test]
fn only_the_label_glyphs_come_from_the_turned_build() {
    let input = orbit_input(false);
    let options = Options::default();
    let data = build_with(&input, &level1());
    for (planet, cell) in data.planets.iter().zip(&data.layout.planets) {
        let group = input.group(cell.group).expect("group");
        let tabs: Vec<_> = input.tabs_of(group.id).into_iter().cloned().collect();
        let ring_options = RingOptions {
            held: false,
            selected_group: None,
            flag_size: options.flag_size * cell.scale,
            colours: options.colours,
        };
        let plain = RingBandsData::new(
            cell.spec,
            std::slice::from_ref(group),
            &tabs,
            &ring_options,
            &ApproxMetrics,
        );
        let ctx = &group.name;
        assert_eq!(planet.ring.segments, plain.segments, "{ctx}");
        assert_eq!(planet.ring.lifted_segments, plain.lifted_segments, "{ctx}");
        assert_eq!(planet.ring.segment_labels, plain.segment_labels, "{ctx}");
        assert_eq!(planet.ring.label_bands, plain.label_bands, "{ctx}");
        assert_eq!(planet.ring.flags, plain.flags, "{ctx}");
        let ids = |tabs: &[mercury_ui::TabRow]| tabs.iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids(&planet.ring.tabs), ids(&plain.tabs), "{ctx}");
        assert_eq!(planet.ring.groups, plain.groups, "{ctx}");
        assert_eq!(planet.ring.model, plain.model, "{ctx}");
        // The same glyphs (text, size, weight, colour), in the same order, somewhere else.
        let (turned, normal) = (&planet.ring.label_glyphs, &plain.label_glyphs);
        assert_eq!(turned.len(), normal.len(), "{ctx}");
        for (a, b) in turned.iter().zip(normal) {
            assert_eq!(
                (&a.text, a.size, a.weight, a.colour),
                (&b.text, b.size, b.weight, b.colour),
                "{ctx}"
            );
        }
        assert_ne!(turned, normal, "{ctx}: the label moved");
        // The plain build has its label at the bottom, the one Orbit shows is above it.
        let half = cell.ring_size / 2.0;
        assert!(
            normal.iter().all(|g| g.y > half),
            "{ctx}: the normal label is at the bottom"
        );
    }
}

// ---- time ----

#[test]
fn sixty_four_groups_and_two_hundred_tabs_build_in_under_100_ms() {
    let input = generated_input(64, 200, false);
    let models = OrbitModels::new();
    // Warm the allocator and the code once; the number below is the second run.
    models.update(&build_with(&input, &level1()));
    let started = Instant::now();
    let data = build_with(&input, &level1());
    let built = started.elapsed();
    let started = Instant::now();
    models.update(&data);
    let updated = started.elapsed();
    println!("64 groups, 200 tabs: build {built:?}, update {updated:?}");
    assert_eq!(data.planets.len(), 64);
    assert!(built.as_millis() < 100, "build took {built:?}");
    assert!(updated.as_millis() < 100, "update took {updated:?}");
    // Level 2 of the biggest group.
    let started = Instant::now();
    let data = build_with(&input, &level2(1));
    println!("level 2: build {:?}", started.elapsed());
    assert!(started.elapsed().as_millis() < 100);
    assert_eq!(data.planets.len(), 64);
}
