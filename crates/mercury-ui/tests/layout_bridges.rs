// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The bridge globals of the layout-switch lane (`ui/layout/bridges.slint`) and the host
//! components that put the group dock and the corner dial on them (`ui/layout/hosts.slint`).
//! Rust only ever talks to the globals; the hosts bind every property and callback of the
//! dock and the dial to them. The first group of tests reaches the globals through `MainWindow`
//! (the window the app has); the second shows the hosts really forward, using `LayoutHostsPreview`,
//! a window that holds nothing but the two hosts.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use common::a11y;
use i_slint_backend_testing::{AccessibleRole as Role, mock_elapsed_time};
use mercury_ui::group_dock::{DockData, DockModels, DockOptions};
use mercury_ui::ring_bands::fixtures::{mockup_groups, mockup_tabs};
use mercury_ui::ring_geometry::ApproxMetrics;
use mercury_ui::{
    DialBridge, DialCornerKind, DialPlacement, DockBridge, DockHit, DockHitKind,
    LayoutHostsPreview, MainWindow, RingHit, RingHitKind,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};

const WIDTH: f32 = 1440.0;
const HEIGHT: f32 = 900.0;
/// The dock's band under the page (`DockGeometry.band-open`).
const BAND: f32 = 72.0;

fn main_window() -> MainWindow {
    a11y::init();
    MainWindow::new().expect("the main window can be created")
}

#[test]
fn dock_bridge_pure_callback_reaches_rust() {
    let window = main_window();
    let bridge = window.global::<DockBridge>();
    bridge.on_hit(|x, y| {
        assert_eq!((x, y), (1.0, 2.0));
        DockHit {
            kind: DockHitKind::Pill,
            group: 7,
            tab: -1,
        }
    });
    let hit = bridge.invoke_hit(1.0, 2.0);
    assert_eq!(hit.kind, DockHitKind::Pill);
    assert_eq!((hit.group, hit.tab), (7, -1));
}

#[test]
fn dial_bridge_pure_callbacks_reach_rust() {
    let window = main_window();
    let bridge = window.global::<DialBridge>();
    bridge.on_place(|corner, width, height| {
        assert_eq!(corner, DialCornerKind::BottomRight);
        DialPlacement {
            x: width - 74.0,
            y: height - 74.0,
        }
    });
    bridge.on_snap_corner(|x, y, _, _, _| {
        if x < 10.0 && y < 10.0 {
            DialCornerKind::TopLeft
        } else {
            DialCornerKind::BottomLeft
        }
    });
    bridge.on_clamp_drag(|x, y, width, _| DialPlacement { x: x.min(width), y });
    bridge.on_step_focus(|focused, step| focused + step);
    bridge.on_hit_test(|_, _, hovered| RingHit {
        kind: RingHitKind::Tab,
        id: hovered,
    });

    let placed = bridge.invoke_place(DialCornerKind::BottomRight, 1440.0, 900.0);
    assert_eq!((placed.x, placed.y), (1366.0, 826.0));
    assert_eq!(
        bridge.invoke_snap_corner(1.0, 1.0, 1440.0, 900.0, DialCornerKind::BottomRight),
        DialCornerKind::TopLeft
    );
    assert_eq!(
        bridge.invoke_snap_corner(700.0, 800.0, 1440.0, 900.0, DialCornerKind::BottomRight),
        DialCornerKind::BottomLeft
    );
    assert_eq!(
        bridge.invoke_clamp_drag(2000.0, 5.0, 1440.0, 900.0).x,
        1440.0
    );
    assert_eq!(bridge.invoke_step_focus(4, 1), 5);
    let hit = bridge.invoke_hit_test(0.0, 0.0, 9);
    assert_eq!((hit.kind, hit.id), (RingHitKind::Tab, 9));
}

#[test]
fn bridge_callbacks_are_plain_callbacks_with_handlers() {
    let window = main_window();
    let heard: Rc<RefCell<Vec<String>>> = Rc::default();

    let dock = window.global::<DockBridge>();
    let log = Rc::clone(&heard);
    dock.on_tab_activated(move |tab| log.borrow_mut().push(format!("dock tab {tab}")));
    let log = Rc::clone(&heard);
    dock.on_pill_activated(move |group| log.borrow_mut().push(format!("dock pill {group}")));
    let log = Rc::clone(&heard);
    dock.on_shrunk_changed(move |shrunk| log.borrow_mut().push(format!("shrunk {shrunk}")));
    dock.invoke_tab_activated(5);
    dock.invoke_pill_activated(3);
    dock.invoke_shrunk_changed(true);

    let dial = window.global::<DialBridge>();
    let log = Rc::clone(&heard);
    dial.on_tab_activated(move |tab| log.borrow_mut().push(format!("dial tab {tab}")));
    let log = Rc::clone(&heard);
    dial.on_drop(move |group, index| log.borrow_mut().push(format!("drop {group} {index}")));
    let log = Rc::clone(&heard);
    dial.on_corner_changed(move |corner| log.borrow_mut().push(format!("corner {corner:?}")));
    dial.invoke_tab_activated(8);
    dial.invoke_drop(2, 4);
    dial.invoke_corner_changed(DialCornerKind::TopRight);

    assert_eq!(
        *heard.borrow(),
        [
            "dock tab 5",
            "dock pill 3",
            "shrunk true",
            "dial tab 8",
            "drop 2 4",
            "corner TopRight",
        ]
    );
}

#[test]
fn bridge_state_properties_hold_what_rust_sets() {
    let window = main_window();
    let dock = window.global::<DockBridge>();
    dock.set_tooltip("Research · Notes".into());
    dock.set_held(true);
    dock.set_shrunk(true);
    dock.set_count_text("22".into());
    assert_eq!(dock.get_tooltip(), "Research · Notes");
    assert!(dock.get_held());
    assert!(dock.get_shrunk());
    assert_eq!(dock.get_count_text(), "22");

    let dial = window.global::<DialBridge>();
    dial.set_corner(DialCornerKind::TopLeft);
    dial.set_expanded(true);
    dial.set_suppressed(true);
    dial.set_dial_label("Tab dial, Research".into());
    assert_eq!(dial.get_corner(), DialCornerKind::TopLeft);
    assert!(dial.get_expanded());
    assert!(dial.get_suppressed());
    assert_eq!(dial.get_dial_label(), "Tab dial, Research");
}

/// A window holding the dial host over the page area and the dock host in the 72 px band.
fn hosts() -> LayoutHostsPreview {
    a11y::init();
    let window = LayoutHostsPreview::new().expect("the hosts window can be created");
    a11y::open_window(&window, WIDTH, HEIGHT);
    window
}

fn move_to(window: &LayoutHostsPreview, x: f32, y: f32) {
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(x, y),
    });
}

fn click(window: &LayoutHostsPreview, x: f32, y: f32) {
    move_to(window, x, y);
    let position = LogicalPosition::new(x, y);
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

fn dock_models() -> (DockModels, DockData) {
    let groups = mockup_groups(false);
    let tabs = mockup_tabs(false);
    let data = DockData::new(
        &groups,
        &tabs,
        &DockOptions {
            container_width: WIDTH,
            ..DockOptions::default()
        },
        &ApproxMetrics,
    );
    let models = DockModels::new();
    models.update(&data.frame(None), &data.capsule());
    (models, data)
}

#[test]
fn the_dock_host_shows_what_the_bridge_holds() {
    let window = hosts();
    let (models, data) = dock_models();
    let frame = data.frame(None);
    let bridge = window.global::<DockBridge>();
    bridge.set_pills(models.pills());
    bridge.set_slivers(models.slivers());
    bridge.set_dock_x(frame.dock_x);
    bridge.set_dock_width(frame.dock_width);
    bridge.set_dock_height(frame.dock_height);
    bridge.set_new_tab_x(frame.new_tab_x);
    bridge.set_shrink_x(frame.shrink_x);
    bridge.set_accessible_name("Group dock under test".into());

    let current = models
        .pills()
        .iter()
        .find(|pill| pill.expanded)
        .expect("one pill is expanded")
        .group
        .name;
    let nodes = a11y::reading_order(&window);
    let dock = a11y::node_in(&nodes, Role::TabList, "Group dock under test");
    assert!(dock.rect().width > 0.0);
    a11y::node_in(&nodes, Role::Button, &format!("New tab in {current}"));
}

#[test]
fn the_dock_host_sends_pointer_events_and_asks_the_bridge_what_is_hit() {
    let window = hosts();
    let bridge = window.global::<DockBridge>();
    let heard: Rc<RefCell<Vec<String>>> = Rc::default();
    let asked = Rc::new(Cell::new(0));

    let counter = Rc::clone(&asked);
    bridge.on_hit(move |_, _| {
        counter.set(counter.get() + 1);
        DockHit {
            kind: DockHitKind::Pill,
            group: 7,
            tab: -1,
        }
    });
    let log = Rc::clone(&heard);
    bridge.on_pointer_moved(move |_, _| log.borrow_mut().push("moved".into()));
    let log = Rc::clone(&heard);
    bridge.on_pointer_left(move || log.borrow_mut().push("left".into()));
    let log = Rc::clone(&heard);
    bridge.on_pill_activated(move |group| log.borrow_mut().push(format!("pill {group}")));

    click(&window, WIDTH / 2.0, HEIGHT - BAND / 2.0);
    assert!(asked.get() > 0, "the click asked the bridge's `hit`");
    assert!(heard.borrow().contains(&"moved".to_string()));
    assert!(
        heard.borrow().contains(&"pill 7".to_string()),
        "the hit the bridge returned decides what the click does: {:?}",
        heard.borrow()
    );

    move_to(&window, WIDTH / 2.0, 100.0);
    assert_eq!(heard.borrow().last().map(String::as_str), Some("left"));
}

#[test]
fn the_dock_host_shrink_button_writes_the_bridge() {
    let window = hosts();
    let (models, data) = dock_models();
    let frame = data.frame(None);
    let bridge = window.global::<DockBridge>();
    bridge.set_pills(models.pills());
    bridge.set_dock_x(frame.dock_x);
    bridge.set_dock_width(frame.dock_width);
    bridge.set_dock_height(frame.dock_height);
    bridge.set_new_tab_x(frame.new_tab_x);
    bridge.set_shrink_x(frame.shrink_x);
    let changes: Rc<RefCell<Vec<bool>>> = Rc::default();
    let log = Rc::clone(&changes);
    bridge.on_shrunk_changed(move |shrunk| log.borrow_mut().push(shrunk));

    let nodes = a11y::reading_order(&window);
    a11y::node_in(
        &nodes,
        Role::Button,
        "Shrink the group dock, Control Command B",
    )
    .press();
    assert!(bridge.get_shrunk(), "the bar's own state is the bridge's");
    assert_eq!(*changes.borrow(), [true]);

    let nodes = a11y::reading_order(&window);
    assert!(
        nodes
            .iter()
            .all(|node| node.label != "Shrink the group dock, Control Command B"),
        "a shrunk dock shows the capsule, not the bar"
    );
}

#[test]
fn the_dock_host_follows_a_shrunk_the_bridge_sets() {
    let window = hosts();
    let (models, data) = dock_models();
    let bridge = window.global::<DockBridge>();
    bridge.set_slivers(models.slivers());
    bridge.set_count_text("22".into());
    bridge.set_capsule_label("22 tabs in 6 groups".into());
    bridge.set_shrunk(true);
    let nodes = a11y::reading_order(&window);
    a11y::node_in(&nodes, Role::Button, "22 tabs in 6 groups");
    let _ = data;
}

#[test]
fn the_dial_host_sits_where_the_bridge_places_it_and_a_click_toggles_the_bridge() {
    let window = hosts();
    let bridge = window.global::<DialBridge>();
    bridge.on_place(|_, _, _| DialPlacement { x: 200.0, y: 300.0 });
    bridge.set_dial_label("Tab dial, Research".into());
    let opened: Rc<RefCell<Vec<bool>>> = Rc::default();
    let log = Rc::clone(&opened);
    bridge.on_toggled(move |open| log.borrow_mut().push(open));

    let nodes = a11y::reading_order(&window);
    let dial = a11y::node_in(&nodes, Role::Button, "Tab dial, Research");
    let rect = dial.rect();
    assert!(
        (rect.x + rect.width / 2.0 - 200.0).abs() < 1.0
            && (rect.y + rect.height / 2.0 - 300.0).abs() < 1.0,
        "the dial's centre is where `place` said: {rect:?}"
    );

    click(&window, 200.0, 300.0);
    assert_eq!(*opened.borrow(), [true]);
    assert!(bridge.get_expanded(), "`expanded` is the bridge's");

    bridge.set_expanded(false);
    click(&window, 200.0, 300.0);
    assert_eq!(*opened.borrow(), [true, true]);
}

#[test]
fn the_dial_host_is_hidden_while_the_bridge_says_suppressed() {
    let window = hosts();
    let bridge = window.global::<DialBridge>();
    bridge.on_place(|_, _, _| DialPlacement { x: 200.0, y: 300.0 });
    bridge.set_dial_label("Tab dial, Research".into());
    assert!(
        a11y::reading_order(&window)
            .iter()
            .any(|node| node.label == "Tab dial, Research")
    );
    bridge.set_suppressed(true);
    assert!(
        a11y::reading_order(&window)
            .iter()
            .all(|node| node.label != "Tab dial, Research"),
        "a suppressed dial is not in the accessibility tree"
    );
}

#[test]
fn the_dial_host_asks_the_bridge_for_the_ring_hit_while_the_fan_is_open() {
    let window = hosts();
    let bridge = window.global::<DialBridge>();
    bridge.on_place(|_, _, _| DialPlacement {
        x: 1366.0,
        y: 750.0,
    });
    let asked = Rc::new(Cell::new(0));
    let counter = Rc::clone(&asked);
    bridge.on_hit_test(move |_, _, _| {
        counter.set(counter.get() + 1);
        RingHit {
            kind: RingHitKind::Outside,
            id: -1,
        }
    });
    bridge.set_expanded(true);
    mock_elapsed_time(Duration::from_millis(400));
    for step in 0..6 {
        move_to(&window, 1200.0 - step as f32 * 30.0, 700.0);
    }
    assert!(asked.get() > 0, "the fan's hit test is the bridge's");
}
