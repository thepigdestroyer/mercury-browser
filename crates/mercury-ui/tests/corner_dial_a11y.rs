// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! What the corner dial tells an assistive technology (SPEC §5, "Corner dial"): a `button` with
//! the host's label, an expanded state, a default action that opens and closes the fan, the fan as
//! a list called "Tab dial" holding one `tab` per segment, nothing for the decoration, and nothing
//! at all while the dial is suppressed.
//!
//! Tests marked `[RingBands]` need the finished `RingBands` body and the data of
//! `RingBandsData::new`; until ring-bands lands they fail because the fan holds nothing.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::a11y::{self, Node};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use mercury_settings::DialCorner;
use mercury_ui::corner_dial::{
    DialFan, DialGeometry, corner_from_slint, corner_to_slint, dial_label,
};
use mercury_ui::ring_bands::{RingBandsModels, RingOptions, fixtures};
use mercury_ui::ring_geometry::{ApproxMetrics, Point};
use mercury_ui::{CornerDialPreview, DialCaption, DialPlacement};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

const LABEL: &str = "Tab dial, Research, page loading 65 percent";

struct Rig {
    window: CornerDialPreview,
    fan: DialFan,
    toggles: Rc<RefCell<Vec<bool>>>,
    _models: RingBandsModels,
}

impl Rig {
    fn new() -> Self {
        a11y::init();
        let window = CornerDialPreview::new().unwrap();
        let geometry = DialGeometry::new(1440.0, 900.0);
        let groups = fixtures::mockup_groups(false);
        let tabs = fixtures::mockup_tabs(false);
        let fan = DialFan::new(
            &geometry,
            DialCorner::BottomRight,
            &groups,
            &tabs,
            &RingOptions::default(),
            &ApproxMetrics,
        );
        let models = RingBandsModels::new();
        models.update(&fan.data);
        window.set_groups(models.groups());
        window.set_tabs(models.tabs());
        window.set_segments(models.segments());
        window.set_lifted_segments(models.lifted_segments());
        window.set_segment_labels(models.segment_labels());
        window.set_label_bands(models.label_bands());
        window.set_label_glyphs(models.label_glyphs());
        window.set_flags(models.flags());
        window.set_active_colour(groups[0].colour);
        window.set_dial_label(LABEL.into());
        window.on_place(|corner, width, height| {
            let centre = DialGeometry::new(width, height).centre(corner_from_slint(corner));
            DialPlacement {
                x: centre.x,
                y: centre.y,
            }
        });
        window.on_clamp_drag(|x, y, width, height| {
            let centre = DialGeometry::new(width, height).clamp_drag(Point::new(x, y));
            DialPlacement {
                x: centre.x,
                y: centre.y,
            }
        });
        window.on_snap_corner(|x, y, width, height, current| {
            corner_to_slint(
                DialGeometry::new(width, height)
                    .snap(Point::new(x, y), corner_from_slint(current)),
            )
        });
        let toggles = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::clone(&toggles);
        window.on_toggled(move |open| heard.borrow_mut().push(open));
        a11y::open_window(&window, 1440.0, 900.0);
        let rig = Self {
            window,
            fan,
            toggles,
            _models: models,
        };
        rig.frame();
        rig
    }

    fn frame(&self) {
        let _ = a11y::reading_order(&self.window);
    }

    fn advance(&self, milliseconds: u64) {
        self.frame();
        mock_elapsed_time(Duration::from_millis(milliseconds));
        slint::platform::update_timers_and_animations();
        self.frame();
    }

    fn nodes(&self) -> Vec<Node> {
        a11y::reading_order(&self.window)
    }

    fn dial(&self) -> ElementHandle {
        a11y::control(&self.window, AccessibleRole::Button, LABEL)
    }

    fn toggles(&self) -> Vec<bool> {
        std::mem::take(&mut *self.toggles.borrow_mut())
    }

    fn click(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        let window = self.window.window();
        window.dispatch_event(WindowEvent::PointerMoved { position });
        window.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        window.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        self.frame();
    }

    /// Opens the fan the way an assistive technology does and lets it finish opening.
    fn open(&self) {
        self.dial().invoke_accessible_default_action();
        self.advance(200);
        assert!(self.window.get_expanded());
        self.toggles();
    }
}

fn count(nodes: &[Node], role: AccessibleRole) -> usize {
    nodes.iter().filter(|node| node.role == role).count()
}

#[test]
fn the_dial_is_a_button_with_the_label_the_host_gives_it() {
    let rig = Rig::new();
    let dial = rig.dial();
    assert_eq!(dial.accessible_role(), Some(AccessibleRole::Button));
    assert_eq!(dial.accessible_label().unwrap(), LABEL);
    rig.window.set_dial_label("Tab dial, Reading".into());
    assert_eq!(
        a11y::control(&rig.window, AccessibleRole::Button, "Tab dial, Reading")
            .accessible_label()
            .unwrap(),
        "Tab dial, Reading",
        "the label follows the property"
    );
}

#[test]
fn the_label_of_state_dial_label_reads_as_the_spec_says() {
    let rig = Rig::new();
    let label = dial_label(Some("Research"), true, 0.65);
    assert_eq!(label, LABEL);
    rig.window.set_dial_label(label);
    assert_eq!(
        a11y::control(&rig.window, AccessibleRole::Button, LABEL)
            .accessible_label()
            .unwrap(),
        "Tab dial, Research, page loading 65 percent"
    );
}

#[test]
fn without_a_label_the_dial_is_still_called_tab_dial() {
    let rig = Rig::new();
    rig.window.set_dial_label("".into());
    let nodes = rig.nodes();
    let dial = a11y::node_in(&nodes, AccessibleRole::Button, "Tab dial");
    assert_eq!(dial.label, "Tab dial");
}

#[test]
fn the_dial_is_the_only_thing_read_while_closed_and_it_has_an_area() {
    let rig = Rig::new();
    // A favicon and a fallback letter are both on offer: whichever is drawn, none of it is read.
    rig.window.set_fallback_letter("D".into());
    rig.window.set_has_favicon(false);
    let nodes = rig.nodes();
    assert_eq!(a11y::dump(&nodes), format!("button | {LABEL} | | collapsed\n"));
    a11y::assert_labelled(&nodes, "the dial at rest");
    a11y::assert_bounds(&nodes, (1440.0, 900.0));
    let rect = nodes[0].rect();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (1330.0, 790.0, 72.0, 72.0),
        "72 px round, centred 74 px from the edges"
    );

    rig.window.set_has_favicon(true);
    rig.window.set_loading(true);
    rig.window.set_progress(0.65);
    rig.advance(200);
    let nodes = rig.nodes();
    assert_eq!(nodes.len(), 1, "the favicon and the ring are decoration:\n{}", a11y::dump(&nodes));
}

#[test]
fn the_dial_says_whether_the_fan_is_open() {
    let rig = Rig::new();
    let dial = rig.dial();
    assert_eq!(dial.accessible_expandable(), Some(true));
    assert_eq!(dial.accessible_expanded(), Some(false));
    rig.window.set_expanded(true);
    assert_eq!(dial.accessible_expanded(), Some(true));
    rig.window.set_expanded(false);
    assert_eq!(dial.accessible_expanded(), Some(false));
}

#[test]
fn the_default_action_toggles_expanded_and_fires_toggled() {
    let rig = Rig::new();
    let dial = rig.dial();
    dial.invoke_accessible_default_action();
    assert!(rig.window.get_expanded());
    assert_eq!(rig.toggles(), [true]);
    assert_eq!(dial.accessible_expanded(), Some(true));
    dial.invoke_accessible_default_action();
    assert!(!rig.window.get_expanded());
    assert_eq!(rig.toggles(), [false]);
}

#[test]
fn the_expand_action_opens_the_fan_and_does_nothing_when_it_is_open() {
    let rig = Rig::new();
    let dial = rig.dial();
    dial.invoke_accessible_expand_action();
    assert!(rig.window.get_expanded());
    assert_eq!(rig.toggles(), [true]);
    dial.invoke_accessible_expand_action();
    assert!(rig.window.get_expanded());
    assert!(rig.toggles().is_empty());
}

/// [RingBands]
#[test]
fn an_open_fan_is_a_list_called_tab_dial_with_one_tab_per_segment() {
    let rig = Rig::new();
    rig.open();
    let nodes = rig.nodes();
    let list = a11y::node_in(&nodes, AccessibleRole::List, "Tab dial");
    let inside = a11y::subtree(&list.handle);
    let tabs: Vec<&Node> = inside
        .iter()
        .filter(|node| node.role == AccessibleRole::Tab)
        .collect();
    assert_eq!(tabs.len(), 22, "the mockup's 22 tabs:\n{}", a11y::dump(&nodes));
    let mut labels: Vec<String> = tabs.iter().map(|tab| tab.label.clone()).collect();
    let mut wanted: Vec<String> = rig
        .fan
        .data
        .segment_labels
        .iter()
        .map(|label| label.to_string())
        .collect();
    labels.sort();
    wanted.sort();
    assert_eq!(labels, wanted, "each tab is named by segment-labels");
    assert!(labels.iter().all(|label| !label.is_empty()));
    // The dial is still there, after the fan (it is drawn above it), and is the only button.
    assert_eq!(count(&nodes, AccessibleRole::Button), 1);
    assert_eq!(count(&nodes, AccessibleRole::List), 1);
    a11y::assert_labelled(&nodes, "the dial fanned out");
    // Every tab lies inside the window (the fan of the bottom-right corner opens up and left).
    let tab_nodes: Vec<Node> = tabs.into_iter().cloned().collect();
    a11y::assert_bounds(&tab_nodes, (1440.0, 900.0));
}

#[test]
fn a_closed_fan_is_not_in_the_tree() {
    let rig = Rig::new();
    let nodes = rig.nodes();
    assert_eq!(count(&nodes, AccessibleRole::List), 0);
    assert_eq!(count(&nodes, AccessibleRole::Tab), 0);
    rig.open();
    rig.dial().invoke_accessible_default_action();
    rig.advance(200);
    let nodes = rig.nodes();
    assert_eq!(
        count(&nodes, AccessibleRole::List) + count(&nodes, AccessibleRole::Tab),
        0,
        "closed again:\n{}",
        a11y::dump(&nodes)
    );
}

#[test]
fn the_caption_and_the_scrim_are_not_read() {
    let rig = Rig::new();
    rig.open();
    rig.window.set_caption(DialCaption {
        text: "Reading · Essay on maps".into(),
        x: 500.0,
        y: 400.0,
    });
    rig.window.set_hovered_tab(103);
    rig.advance(10);
    let nodes = rig.nodes();
    assert!(
        nodes.iter().all(|node| !node.label.contains("Essay on maps")),
        "the caption is decoration (the hovered tab already has its label):\n{}",
        a11y::dump(&nodes)
    );
    assert!(
        nodes.iter().all(|node| node.role != AccessibleRole::Image),
        "{}",
        a11y::dump(&nodes)
    );
    assert!(
        nodes.iter().all(|node| node.label != "D"),
        "the letter on the dial is decoration"
    );
}

#[test]
fn suppressed_removes_the_dial_and_the_fan_from_the_tree_and_a_click_does_nothing() {
    let rig = Rig::new();
    rig.window.set_suppressed(true);
    rig.advance(10);
    assert!(rig.nodes().is_empty(), "{}", a11y::dump(&rig.nodes()));
    rig.click(1366.0, 826.0);
    assert!(!rig.window.get_expanded());
    assert!(rig.toggles().is_empty());

    // The fan too: open it first, then suppress.
    rig.window.set_suppressed(false);
    rig.advance(10);
    rig.open();
    assert!(count(&rig.nodes(), AccessibleRole::Button) == 1);
    rig.window.set_suppressed(true);
    rig.advance(200);
    assert!(rig.nodes().is_empty(), "{}", a11y::dump(&rig.nodes()));
    assert!(!rig.window.get_expanded());
}

#[test]
fn the_focus_ring_is_there_only_while_the_dial_has_the_keyboard_focus() {
    let rig = Rig::new();
    let ring = || ElementHandle::find_by_element_id(&rig.window, "CornerDial::focus-ring").count();
    assert_eq!(ring(), 0, "nothing is focused yet");
    // A press on the dial takes the focus (and the click opens the fan).
    rig.click(1366.0, 826.0);
    assert_eq!(ring(), 1, "the focused dial shows its ring");
    // The window losing the focus takes the ring away.
    rig.window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(false));
    assert_eq!(ring(), 0, "an inactive window shows no focus ring");
    rig.window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    assert_eq!(ring(), 1, "and it comes back with the window");
}
