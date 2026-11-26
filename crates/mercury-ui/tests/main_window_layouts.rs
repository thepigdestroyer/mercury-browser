// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The main window's two trees (layout-switch plan, task 5): the tab row, the toolbar, the hub
//! button, the address field, the page area and the dock band, for the Mercury and the Classic
//! layout. Rust pushes the layout through the `MainWindow` properties (`push_plan` in the desktop
//! app); these tests set the same properties directly.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use common::a11y::{self, Rect, TOLERANCE, node_in, reading_order};
use i_slint_backend_testing::AccessibleRole as Role;
use mercury_appearance::Metrics;
use mercury_settings::DialCorner;
use mercury_ui::corner_dial::{DIAL_DIAMETER, DIAL_MARGIN, DialGeometry};
use mercury_ui::ring_bands::fixtures;
use mercury_ui::ring_geometry::ApproxMetrics;
use mercury_ui::window_layout::{WindowFeed, WindowRows};
use mercury_ui::{HubRingBridge, Look, MainWindow, WindowLayout, apply_look};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};

const WIDTH: f32 = 1180.0;
const HEIGHT: f32 = 760.0;
const DOCK_OPEN: f32 = 72.0;
const DOCK_SHRUNK: f32 = 40.0;
const HUB_LABEL: &str = "Tab groups, 6 groups, 22 tabs";

/// A window with the layout sizes preset (tab row 46, toolbar 52) shown at `width` x `height`.
fn layout_window(width: f32, height: f32) -> MainWindow {
    a11y::init();
    let window = MainWindow::new().unwrap();
    apply_look(&window, &Look::hermes_roomy().with_metrics(Metrics::LAYOUT));
    a11y::open_window(&window, width, height);
    window
}

/// The Mercury tree: no tab row, the hub button, the dock at its open height.
fn show_mercury(window: &MainWindow) {
    window.set_layout(WindowLayout::Mercury);
    window.set_tab_row_shown(false);
    window.set_hub_shown(true);
    window.set_dock_shown(true);
    window.set_dock_band(DOCK_OPEN);
    window.set_dial_shown(false);
    window.set_window_controls_inset(0.0);
    window.set_bookmarks_bar_shown(false);
}

/// The Classic tree: the tab row above the toolbar, no dock, no hub button.
fn show_classic(window: &MainWindow) {
    window.set_layout(WindowLayout::Classic);
    window.set_tab_row_shown(true);
    window.set_hub_shown(false);
    window.set_dock_shown(false);
    window.set_dock_band(0.0);
    window.set_dial_shown(false);
    window.set_window_controls_inset(0.0);
    window.set_bookmarks_bar_shown(false);
}

/// Rows for the dock and the dial, and the feed that puts them on the bridges.
fn fill(window: &MainWindow, area_height: f32) -> WindowFeed {
    let feed = WindowFeed::install(window, Box::new(ApproxMetrics));
    let rows = WindowRows {
        groups: fixtures::mockup_groups(false),
        tabs: fixtures::mockup_tabs(false),
        container_width: WIDTH,
        area: (WIDTH, area_height),
        ..WindowRows::default()
    };
    feed.set_rows(window, &rows);
    feed
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= TOLERANCE
}

fn rect(window: &MainWindow, role: Role, label: &str) -> Rect {
    node_in(&reading_order(window), role, label).rect()
}

fn page_bottom(window: &MainWindow) -> f32 {
    window.get_controls_page_y() + window.get_web_view_height()
}

#[test]
fn mercury_has_one_row_of_52() {
    let window = layout_window(WIDTH, HEIGHT);
    show_mercury(&window);
    let page_y = window.get_controls_page_y();
    assert!(
        close(page_y, 52.0),
        "Mercury has one toolbar row of 52 px above the page, not {page_y}"
    );
}

#[test]
fn classic_has_46_plus_52() {
    let window = layout_window(WIDTH, HEIGHT);
    show_classic(&window);
    let page_y = window.get_controls_page_y();
    assert!(
        close(page_y, 98.0),
        "Classic has the 46 px tab row and the 52 px toolbar above the page, not {page_y}"
    );
}

#[test]
fn the_page_area_height_is_window_minus_rows_and_band() {
    let window = layout_window(800.0, 600.0);
    show_mercury(&window);

    let open = window.get_web_view_height();
    assert!(close(open, 600.0 - 52.0 - DOCK_OPEN), "open dock: {open}");

    window.set_dock_band(DOCK_SHRUNK);
    let shrunk = window.get_web_view_height();
    assert!(
        close(shrunk, 600.0 - 52.0 - DOCK_SHRUNK),
        "shrunk dock: {shrunk}"
    );

    window.set_dock_shown(false);
    window.set_dock_band(0.0);
    let no_dock = window.get_web_view_height();
    assert!(close(no_dock, 600.0 - 52.0), "no dock: {no_dock}");

    window.set_dock_shown(true);
    window.set_dock_band(DOCK_OPEN);
    window.set_chrome_hidden(true);
    let hidden = window.get_web_view_height();
    assert!(close(hidden, 600.0), "chrome hidden: {hidden}");
}

#[test]
fn an_unpushed_window_is_roomy_classic() {
    a11y::init();
    let window = MainWindow::new().unwrap();
    window.set_bookmarks_bar_shown(false);
    a11y::open_window(&window, WIDTH, HEIGHT);
    let page_y = window.get_controls_page_y();
    assert!(
        close(page_y, 108.0),
        "a window nobody pushed to is the roomy Classic tree: 48 + 60, not {page_y}"
    );
}

#[test]
fn the_inset_moves_the_first_control_by_exactly_the_inset() {
    let window = layout_window(WIDTH, HEIGHT);

    show_mercury(&window);
    let before = rect(&window, Role::Button, "Back").x;
    window.set_window_controls_inset(78.0);
    let after = rect(&window, Role::Button, "Back").x;
    assert!(
        close(after - before, 78.0),
        "Mercury: the toolbar pads its left edge by the inset; Back moved {}",
        after - before
    );

    show_classic(&window);
    let before = rect(&window, Role::TabList, "Tabs").x;
    window.set_window_controls_inset(78.0);
    let after = rect(&window, Role::TabList, "Tabs").x;
    assert!(
        close(after - before, 78.0),
        "Classic: the tab row pads its left edge by the inset; the strip moved {}",
        after - before
    );
}

#[test]
fn the_address_field_keeps_its_ids_across_layouts() {
    let window = layout_window(WIDTH, HEIGHT);
    show_classic(&window);
    window.invoke_focus_address_bar();
    window.set_address("example.com".into());
    assert!(
        window.get_address_has_focus(),
        "the address field has focus"
    );

    show_mercury(&window);
    assert_eq!(
        window.get_address().as_str(),
        "example.com",
        "the typed text stays"
    );
    assert!(
        window.get_address_has_focus(),
        "the same address field keeps the focus across the layout change"
    );

    show_classic(&window);
    assert_eq!(
        window.get_address().as_str(),
        "example.com",
        "and back again"
    );
    assert!(window.get_address_has_focus());
}

#[test]
fn the_address_field_stays_between_240_and_640_in_mercury() {
    let window = layout_window(600.0, HEIGHT);
    show_mercury(&window);
    for width in [600.0, 1180.0, 3440.0] {
        a11y::open_window(&window, width, HEIGHT);
        let address = rect(&window, Role::Search, "Address bar").width;
        assert!(
            (240.0 - TOLERANCE..=640.0 + TOLERANCE).contains(&address),
            "at {width} px the address field is {address} px wide"
        );
        if width == 3440.0 {
            assert!(close(address, 640.0), "the widest window caps it at 640 px");
        }
    }
}

#[test]
fn a_click_outside_the_dial_reaches_the_page() {
    let window = layout_window(WIDTH, HEIGHT);
    show_mercury(&window);
    window.set_is_new_tab(false);
    window.set_web_view_visible(true);
    window.set_dial_shown(true);
    let top = window.get_controls_page_y();
    let height = window.get_web_view_height();
    let _feed = fill(&window, height);

    let centre = DialGeometry::new(WIDTH, height).centre(DialCorner::default());
    let received: Rc<RefCell<Vec<(f32, f32)>>> = Rc::default();
    let sink = received.clone();
    window.on_web_view_pointer_moved(move |x, y| sink.borrow_mut().push((x, y)));

    let mut expected = Vec::new();
    for i in 0..10 {
        for j in 0..10 {
            let x = WIDTH * (i as f32 + 0.5) / 10.0;
            let y = top + height * (j as f32 + 0.5) / 10.0;
            let (dx, dy) = (x - centre.x, y - top - centre.y);
            if (dx * dx + dy * dy).sqrt() < DIAL_DIAMETER + DIAL_MARGIN {
                continue;
            }
            window.window().dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(x, y),
            });
            expected.push((x, y - top));
        }
    }

    assert!(
        expected.len() > 50,
        "the grid samples the page: {}",
        expected.len()
    );
    let received = received.borrow();
    for (x, y) in expected {
        assert!(
            received
                .iter()
                .any(|(rx, ry)| (rx - x).abs() <= 0.5 && (ry - y).abs() <= 0.5),
            "the pointer at ({x}, {y}) of the page did not reach the page"
        );
    }
}

#[test]
fn the_dock_never_overlaps_the_page() {
    let window = layout_window(WIDTH, HEIGHT);
    show_mercury(&window);
    let _feed = fill(&window, window.get_web_view_height());

    let dock = rect(&window, Role::TabList, "Group dock");
    assert!(
        dock.y >= page_bottom(&window) - TOLERANCE,
        "the dock starts at {} but the page ends at {}",
        dock.y,
        page_bottom(&window)
    );
    assert!(
        dock.bottom() <= HEIGHT + TOLERANCE,
        "the dock stays in the window"
    );
}

#[test]
fn narrow_600_keeps_every_row_inside_the_window() {
    let window_rect = Rect::new(0.0, 0.0, 600.0, HEIGHT);

    let window = layout_window(600.0, HEIGHT);
    show_mercury(&window);
    let _feed = fill(&window, window.get_web_view_height());
    let toolbar = rect(&window, Role::Navigation, "Toolbar");
    assert!(
        window_rect.contains(&toolbar),
        "Mercury toolbar {toolbar:?}"
    );
    let dock = rect(&window, Role::TabList, "Group dock");
    assert!(window_rect.contains(&dock), "Mercury dock {dock:?}");

    show_classic(&window);
    let toolbar = rect(&window, Role::Navigation, "Toolbar");
    assert!(
        window_rect.contains(&toolbar),
        "Classic toolbar {toolbar:?}"
    );
    let strip = rect(&window, Role::TabList, "Tabs");
    assert!(window_rect.contains(&strip), "Classic tab row {strip:?}");
}

#[test]
fn hub_button_is_a_button_labelled_by_the_bridge() {
    let window = layout_window(WIDTH, HEIGHT);
    show_mercury(&window);
    window
        .global::<HubRingBridge>()
        .set_hub_label(HUB_LABEL.into());
    let nodes = reading_order(&window);
    let hub = node_in(&nodes, Role::Button, HUB_LABEL);
    assert!(hub.enabled, "the hub button is enabled");
}

#[test]
fn show_tab_ring_opens_the_popover() {
    let window = layout_window(WIDTH, HEIGHT);
    show_mercury(&window);
    window
        .global::<HubRingBridge>()
        .set_hub_label(HUB_LABEL.into());
    let opened = Rc::new(Cell::new(0));
    let counter = opened.clone();
    window
        .global::<HubRingBridge>()
        .on_opened(move || counter.set(counter.get() + 1));

    window.invoke_show_tab_ring();
    assert!(
        opened.get() > 0,
        "show-tab-ring opens the popover, which reports opened()"
    );
}
