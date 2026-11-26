// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! `WindowFeed` (layout-switch plan, task 4): the dock and dial bridges are filled once and only
//! when the rows, the plan or the pointer change.

mod common;

use std::time::Instant;

use mercury_settings::{AppearanceSettings, DialCorner};
use mercury_ui::corner_dial::{DialFan, DialGeometry, corner_from_slint, corner_to_slint};
use mercury_ui::ring_bands::RingOptions;
use mercury_ui::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
use mercury_ui::ring_geometry::ApproxMetrics;
use mercury_ui::window_layout::{
    ChromePlan, DialFace, Layout, Platform, WindowChrome, WindowFeed, WindowRows, plan,
};
use mercury_ui::{DialBridge, DialCornerKind, DockBridge, GroupRow, MainWindow, TabRow};
use slint::{ComponentHandle, Model};

const WIDTH: f32 = 1440.0;
const HEIGHT: f32 = 900.0;

fn main_window() -> MainWindow {
    common::a11y::init();
    MainWindow::new().expect("the main window can be created")
}

fn rows(groups: Vec<GroupRow>, tabs: Vec<TabRow>) -> WindowRows {
    WindowRows {
        groups,
        tabs,
        dark: true,
        container_width: WIDTH,
        area: (WIDTH, HEIGHT - 52.0 - 72.0),
        held: false,
        show_keys: false,
        reduce_motion: false,
        dial: DialFace::default(),
    }
}

fn mockup_rows() -> WindowRows {
    rows(mockup_groups(true), mockup_tabs(true))
}

fn feed(window: &MainWindow) -> WindowFeed {
    WindowFeed::install(window, Box::new(ApproxMetrics))
}

fn pill_widths(window: &MainWindow) -> Vec<f32> {
    window
        .global::<DockBridge>()
        .get_pills()
        .iter()
        .map(|pill| pill.width)
        .collect()
}

#[test]
fn install_sets_each_model_once() {
    let window = main_window();
    let feed = feed(&window);
    let held = window.global::<DockBridge>().get_pills();
    assert_eq!(
        held.row_count(),
        0,
        "nothing is shown before the first rows"
    );
    feed.set_rows(&window, &mockup_rows());
    assert_eq!(
        held.row_count(),
        6,
        "the model set once at install holds the six pills"
    );
    assert_eq!(window.global::<DockBridge>().get_pills().row_count(), 6);
}

#[test]
fn an_identical_second_set_rows_writes_no_row() {
    let window = main_window();
    let feed = feed(&window);
    feed.set_rows(&window, &mockup_rows());
    let before = pill_widths(&window);
    feed.set_rows(&window, &mockup_rows());
    assert_eq!(pill_widths(&window), before);
}

#[test]
fn dock_pills_follow_the_groups() {
    let window = main_window();
    let feed = feed(&window);
    feed.set_rows(&window, &mockup_rows());
    let dock = window.global::<DockBridge>();
    assert_eq!(dock.get_pills().row_count(), 6);
    assert_eq!(dock.get_count_text().as_str(), "22");
    assert_eq!(
        dock.get_capsule_label().as_str(),
        "Show the group dock, 22 tabs in 6 groups"
    );
}

#[test]
fn pointer_moved_magnifies_the_pill_under_the_pointer() {
    let window = main_window();
    let feed = feed(&window);
    feed.set_rows(&window, &mockup_rows());
    let rest = pill_widths(&window);
    let dock = window.global::<DockBridge>();
    let grew = (0..=40).any(|step| {
        let x = WIDTH * step as f32 / 40.0;
        dock.invoke_pointer_moved(x, 20.0);
        pill_widths(&window)
            .iter()
            .zip(&rest)
            .any(|(now, at_rest)| *now >= at_rest * 1.3)
    });
    assert!(grew, "a pointer over the dock magnifies a pill");
}

#[test]
fn pointer_left_restores_rest() {
    let window = main_window();
    let feed = feed(&window);
    feed.set_rows(&window, &mockup_rows());
    let rest = pill_widths(&window);
    let dock = window.global::<DockBridge>();
    dock.invoke_pointer_moved(dock.get_dock_x() + rest[0] / 2.0, 20.0);
    dock.invoke_pointer_left();
    assert_eq!(pill_widths(&window), rest);
}

#[test]
fn empty_rows_leave_every_bridge_empty_and_valid() {
    let window = main_window();
    let feed = feed(&window);
    feed.set_rows(&window, &rows(Vec::new(), Vec::new()));
    let dock = window.global::<DockBridge>();
    let dial = window.global::<DialBridge>();
    assert_eq!(dock.get_pills().row_count(), 0);
    assert_eq!(dock.get_slivers().row_count(), 0);
    assert_eq!(dial.get_segments().row_count(), 0);
    assert_eq!(dial.get_groups().row_count(), 0);
}

#[test]
fn two_hundred_tabs_build_under_a_second() {
    let window = main_window();
    let feed = feed(&window);
    let (groups, tabs) = generated(20, 200, true);
    let start = Instant::now();
    feed.set_rows(&window, &rows(groups, tabs));
    let elapsed = start.elapsed();
    println!("200 tabs built in {elapsed:?}");
    assert!(elapsed.as_secs_f32() < 1.0, "200 tabs took {elapsed:?}");
    assert_eq!(
        window.global::<DockBridge>().get_count_text().as_str(),
        "200"
    );
}

#[test]
fn dial_fan_is_built_only_while_expanded() {
    let window = main_window();
    let feed = feed(&window);
    feed.set_rows(&window, &mockup_rows());
    let dial = window.global::<DialBridge>();
    assert_eq!(
        dial.get_segments().row_count(),
        0,
        "a closed dial builds no fan"
    );
    dial.set_expanded(true);
    feed.set_rows(&window, &mockup_rows());
    assert!(
        dial.get_segments().row_count() > 0,
        "an open dial builds its fan"
    );
}

#[test]
fn dial_is_not_offered_when_the_fan_cannot_fit() {
    let window = main_window();
    let feed = feed(&window);
    let mut appearance = AppearanceSettings::default();
    appearance.tab_groups.dial = true;
    let chosen = plan(&appearance, &WindowChrome::default(), true, Platform::MacOs);
    let shown = ChromePlan {
        show_dial: true,
        ..chosen
    };
    feed.set_plan(&window, &shown);
    let small = DialGeometry::new(400.0, 300.0);
    assert!(!small.fan_fits(DialCorner::BottomRight));
    assert!(DialGeometry::new(WIDTH, HEIGHT).fan_fits(DialCorner::BottomRight));
}

#[test]
fn dial_place_matches_geometry() {
    let window = main_window();
    let _feed = feed(&window);
    let dial = window.global::<DialBridge>();
    let placed = dial.invoke_place(DialCornerKind::BottomRight, WIDTH, HEIGHT);
    let centre = DialGeometry::new(WIDTH, HEIGHT).centre(DialCorner::BottomRight);
    assert!(
        (placed.x - centre.x).abs() < 0.01,
        "{placed:?} vs {centre:?}"
    );
    assert!(
        (placed.y - centre.y).abs() < 0.01,
        "{placed:?} vs {centre:?}"
    );
}

#[test]
fn dial_snap_corner_matches_geometry() {
    let window = main_window();
    let _feed = feed(&window);
    let dial = window.global::<DialBridge>();
    let kind = dial.invoke_snap_corner(10.0, 10.0, WIDTH, HEIGHT, DialCornerKind::BottomRight);
    let expected = DialGeometry::new(WIDTH, HEIGHT).snap(
        mercury_ui::ring_geometry::Point::new(10.0, 10.0),
        DialCorner::BottomRight,
    );
    assert_eq!(corner_from_slint(kind), expected);
}

#[test]
fn hit_test_forwards_to_the_ring() {
    let window = main_window();
    let feed = feed(&window);
    let groups = mockup_groups(true);
    let tabs = mockup_tabs(true);
    feed.set_rows(&window, &rows(groups.clone(), tabs.clone()));
    window.global::<DialBridge>().set_expanded(true);
    feed.set_rows(&window, &rows(groups.clone(), tabs.clone()));
    let geometry = DialGeometry::new(WIDTH, HEIGHT);
    let fan = DialFan::new(
        &geometry,
        DialCorner::BottomRight,
        &groups,
        &tabs,
        &RingOptions::default(),
        &ApproxMetrics,
    );
    let segment = fan.data.segments.first().expect("the fan has a segment");
    let hit = window
        .global::<DialBridge>()
        .invoke_hit_test(segment.mid_x, segment.mid_y, -1);
    assert_eq!(hit.kind, mercury_ui::RingHitKind::Tab);
}

#[test]
fn set_plan_applies_the_dial_corner() {
    let window = main_window();
    let feed = feed(&window);
    let chosen = plan(
        &AppearanceSettings::default(),
        &WindowChrome::default(),
        true,
        Platform::MacOs,
    );
    feed.set_plan(&window, &chosen);
    let dial = window.global::<DialBridge>();
    assert_eq!(dial.get_corner(), corner_to_slint(chosen.dial_corner));
    assert_eq!(chosen.layout, Layout::Mercury);
}

#[test]
fn dock_state_changed_writes_the_shrunk_flag() {
    let window = main_window();
    let feed = feed(&window);
    feed.dock_state_changed(&window, true);
    assert!(window.global::<DockBridge>().get_shrunk());
    feed.dock_state_changed(&window, false);
    assert!(!window.global::<DockBridge>().get_shrunk());
}
