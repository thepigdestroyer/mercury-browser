// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What an assistive technology is given by the hub ring (SPEC §5 "Ring popover", brief §7
//! item 4): the hub button as a labelled button, the pinned beads as buttons labelled "Pinned tab,
//! ...", and (from `RingBands`) the "Tab ring" list with its tab lists and tabs. Read through
//! Slint's testing backend with the 1000 x 640 window of the `ring-open` scene.
#![cfg(feature = "test-support")]

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use common::a11y;
use i_slint_backend_testing::AccessibleRole as Role;
use mercury_ui::hub_ring::{HubRing, HubRingHost, HubRingInput, fixtures, hub_button};
use mercury_ui::{RingPopoverPreview, ring_bands};

#[derive(Default)]
struct Recorder {
    calls: RefCell<Vec<String>>,
}

impl HubRingHost for Recorder {
    fn activate_tab(&self, tab: i32) {
        self.calls.borrow_mut().push(format!("activate {tab}"));
    }
    fn close_tab(&self, _tab: i32) {}
    fn toggle_mute(&self, _tab: i32) {}
    fn toggle_pause(&self, _tab: i32) {}
    fn move_tab(&self, _tab: i32, _group: i32, _index: usize) {}
    fn open_orbit(&self) {}
    fn search_web(&self, _query: &str) {}
}

struct Rig {
    window: RingPopoverPreview,
    _ring: HubRing,
    host: Rc<Recorder>,
    input: HubRingInput,
}

fn rig_with(input: HubRingInput) -> Rig {
    a11y::init();
    let window = RingPopoverPreview::new().expect("the preview window can be created");
    window.set_show_chrome(true);
    a11y::open_window(&window, 1000.0, 640.0);
    let host = Rc::new(Recorder::default());
    let ring = HubRing::install(&window, host.clone(), ring_bands::metrics());
    ring.set_input(input.clone());
    ring.opened();
    Rig {
        window,
        _ring: ring,
        host,
        input,
    }
}

fn rig() -> Rig {
    rig_with(fixtures::mockup_input(false))
}

#[test]
fn the_hub_button_is_a_button_labelled_with_the_group_and_tab_counts() {
    let rig = rig();
    let nodes = a11y::reading_order(&rig.window);
    let label = hub_button::label(&rig.input.groups);
    assert!(
        label.starts_with("Tab groups, 6 groups"),
        "the label names the groups: {label:?}"
    );
    let button = a11y::node_in(&nodes, Role::Button, &label);
    assert!(button.enabled);

    let clicks = Rc::new(Cell::new(0));
    let counted = clicks.clone();
    rig.window
        .on_hub_clicked(move || counted.set(counted.get() + 1));
    button.press();
    assert_eq!(clicks.get(), 1, "the default action opens the popover");
}

#[test]
fn the_three_pinned_tabs_are_buttons_labelled_pinned_tab() {
    let rig = rig();
    let nodes = a11y::reading_order(&rig.window);
    for title in ["Mail", "Maps", "Calendar"] {
        let bead = a11y::node_in(&nodes, Role::Button, &format!("Pinned tab, {title}"));
        assert!(bead.enabled);
    }
    let beads = nodes
        .iter()
        .filter(|node| node.role == Role::Button && node.label.starts_with("Pinned tab, "))
        .count();
    assert_eq!(beads, 3);
}

#[test]
fn pressing_a_bead_through_the_accessibility_action_shows_its_tab() {
    let rig = rig();
    let nodes = a11y::reading_order(&rig.window);
    a11y::node_in(&nodes, Role::Button, "Pinned tab, Maps").press();
    assert_eq!(rig.host.calls.borrow().as_slice(), ["activate 201"]);
}

#[test]
fn more_pinned_tabs_than_fit_leave_one_chip_that_says_how_many_are_behind_it() {
    let mut input = fixtures::mockup_input(false);
    let template = fixtures::pinned_tabs();
    for extra in 0..5 {
        let mut tab = template[extra % 3].clone();
        tab.id = 300 + extra as i32;
        tab.title = format!("Pinned extra {extra}").into();
        input.tabs.push(tab);
    }
    let rig = rig_with(input);
    let nodes = a11y::reading_order(&rig.window);
    let drawn = nodes
        .iter()
        .filter(|node| node.role == Role::Button && node.label.starts_with("Pinned tab, "))
        .count();
    let chips: Vec<&a11y::Node> = nodes
        .iter()
        .filter(|node| node.role == Role::Button && node.label.ends_with(" more pinned tabs"))
        .collect();
    assert_eq!(chips.len(), 1, "one chip: {}", a11y::dump(&nodes));
    let behind: usize = chips[0]
        .label
        .split(' ')
        .next()
        .and_then(|number| number.parse().ok())
        .expect("the chip's label starts with the number");
    assert_eq!(
        drawn + behind,
        8,
        "every pinned tab is a bead or behind the chip"
    );
}

#[test]
fn nothing_interactive_in_the_popover_is_unlabelled() {
    let rig = rig();
    let nodes = a11y::reading_order(&rig.window);
    a11y::assert_labelled(&nodes, "the hub ring at rest");
}

// The list, the tab lists and the tabs are RingBands' own accessibility tree.
#[test]
fn the_ring_is_one_list_of_six_tab_lists_holding_the_22_tabs() {
    let rig = rig();
    let nodes = a11y::reading_order(&rig.window);
    a11y::node_in(&nodes, Role::List, "Tab ring");
    let lists = nodes.iter().filter(|n| n.role == Role::TabList).count();
    assert_eq!(lists, 6, "one tab list per group:\n{}", a11y::dump(&nodes));
    let tabs: Vec<&a11y::Node> = nodes.iter().filter(|n| n.role == Role::Tab).collect();
    assert_eq!(tabs.len(), 22, "{}", a11y::dump(&nodes));
    assert!(tabs.iter().all(|tab| !tab.label.is_empty()));
    let selected = nodes.iter().filter(|n| n.selected == Some(true)).count();
    assert_eq!(
        selected,
        1,
        "exactly one selected tab:\n{}",
        a11y::dump(&nodes)
    );
}
