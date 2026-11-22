// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The accessibility tree of `RingBands` (SPEC §5, brief §3.1), read through Slint's testing
//! backend: a `list` for the ring, one `tab-list` per label band (siblings before the tabs, D10),
//! one `tab` per segment with the label Rust gave it, one selected tab, and nothing else.
//!
//! The arrays are built here from `RingModel` (labels are made up from the row titles) so that the
//! tests do not depend on `ring_bands::data`.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{self, Node, assert_bounds, reading_order};
use i_slint_backend_testing::AccessibleRole;
use mercury_ui::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
use mercury_ui::ring_geometry::slint_models::RingModel;
use mercury_ui::ring_geometry::{Point, RingSpec};
use mercury_ui::{GroupRow, RingBandsPreview, TabRow};
use slint::{ModelRc, SharedString, VecModel};

fn model_rc<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

/// What `RingBandsData::new` will produce, from `RingModel`: the shown groups and tabs in ring
/// order, the segments, the lifted shapes, made-up labels and the label bands.
fn fill(window: &RingBandsPreview, groups: &[GroupRow], tabs: &[TabRow]) -> Vec<String> {
    let model = RingModel::new(RingSpec::hub(Point::new(258.0, 258.0)), groups, tabs);
    let shown_tabs: Vec<TabRow> = model
        .tab_rows
        .iter()
        .map(|&row| tabs[row].clone())
        .collect();
    let shown_groups: Vec<GroupRow> = groups
        .iter()
        .filter(|group| shown_tabs.iter().any(|tab| tab.group == group.id))
        .cloned()
        .collect();
    let lifted: Vec<_> = (0..shown_tabs.len())
        .filter_map(|flat| model.lifted_segment_for_slint(flat, 10.0))
        .collect();
    let labels: Vec<String> = shown_tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| format!("{}, tab {} of {}", tab.title, i + 1, shown_tabs.len()))
        .collect();
    window.set_ring_size(516.0);
    window.set_groups(model_rc(shown_groups));
    window.set_tabs(model_rc(shown_tabs));
    window.set_segments(model_rc(model.segments_for_slint()));
    window.set_lifted_segments(model_rc(lifted));
    window.set_label_bands(model_rc(model.label_bands_for_slint()));
    window.set_segment_labels(model_rc(
        labels.iter().map(SharedString::from).collect::<Vec<_>>(),
    ));
    labels
}

fn scene(groups: &[GroupRow], tabs: &[TabRow]) -> (RingBandsPreview, Vec<String>) {
    a11y::init();
    let window = RingBandsPreview::new().unwrap();
    let labels = fill(&window, groups, tabs);
    a11y::open_window(&window, 516.0, 516.0);
    (window, labels)
}

fn with_role(nodes: &[Node], role: AccessibleRole) -> Vec<&Node> {
    nodes.iter().filter(|node| node.role == role).collect()
}

#[test]
fn the_mockup_ring_has_22_tabs_6_tab_lists_and_one_list() {
    let (window, labels) = scene(&mockup_groups(false), &mockup_tabs(false));
    let nodes = reading_order(&window);

    let tabs = with_role(&nodes, AccessibleRole::Tab);
    assert_eq!(tabs.len(), 22);
    for (tab, label) in tabs.iter().zip(&labels) {
        assert!(!tab.label.is_empty());
        assert_eq!(&tab.label, label);
    }

    let tab_lists = with_role(&nodes, AccessibleRole::TabList);
    assert_eq!(tab_lists.len(), 6);

    let lists = with_role(&nodes, AccessibleRole::List);
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].label, "Tab ring");
}

#[test]
fn exactly_one_tab_is_selected_and_every_tab_is_selectable() {
    let (window, _) = scene(&mockup_groups(false), &mockup_tabs(false));
    let nodes = reading_order(&window);
    let tabs = with_role(&nodes, AccessibleRole::Tab);
    assert!(tabs.iter().all(|tab| tab.selected.is_some()));
    let selected: Vec<_> = tabs
        .iter()
        .filter(|tab| tab.selected == Some(true))
        .collect();
    assert_eq!(selected.len(), 1);
    assert!(selected[0].label.starts_with("Servo layout notes"));
}

#[test]
fn a_tab_list_names_its_group_its_count_and_its_key() {
    let (window, _) = scene(&mockup_groups(false), &mockup_tabs(false));
    let nodes = reading_order(&window);
    let labels: Vec<&str> = with_role(&nodes, AccessibleRole::TabList)
        .iter()
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "Research, tab group, 5 tabs, key Option A",
            "Lisbon trip, tab group, 4 tabs, key Option S",
            "Reading, tab group, 6 tabs, key Option D",
            "Shopping, tab group, 3 tabs, key Option F",
            "Work, tab group, 2 tabs, key Option G",
            "Loose, tab group, 2 tabs, key Option H",
        ]
    );
}

#[test]
fn a_group_of_one_tab_reads_1_tab_and_no_key_part_without_a_key() {
    let mut groups = mockup_groups(false);
    groups.truncate(1);
    groups[0].count = 1;
    groups[0].key_label = "".into();
    let mut tabs = mockup_tabs(false);
    tabs.truncate(1);
    let (window, _) = scene(&groups, &tabs);
    let nodes = reading_order(&window);
    let tab_lists = with_role(&nodes, AccessibleRole::TabList);
    assert_eq!(tab_lists.len(), 1);
    assert_eq!(tab_lists[0].label, "Research, tab group, 1 tab");
}

#[test]
fn the_tab_lists_come_before_the_tabs() {
    let (window, _) = scene(&mockup_groups(false), &mockup_tabs(false));
    let nodes = reading_order(&window);
    let last_list = nodes
        .iter()
        .rposition(|node| node.role == AccessibleRole::TabList)
        .unwrap();
    let first_tab = nodes
        .iter()
        .position(|node| node.role == AccessibleRole::Tab)
        .unwrap();
    assert!(last_list < first_tab);
    assert_eq!(nodes[0].role, AccessibleRole::List);
}

#[test]
fn decoration_has_no_role() {
    let (window, _) = scene(&mockup_groups(false), &mockup_tabs(false));
    let nodes = reading_order(&window);
    assert_eq!(
        nodes.len(),
        1 + 6 + 22,
        "only the list, the tab-lists and the tabs"
    );
}

#[test]
fn a_host_can_rename_the_list() {
    let (window, _) = scene(&mockup_groups(false), &mockup_tabs(false));
    window.set_accessible_name("Tab dial".into());
    let nodes = reading_order(&window);
    assert_eq!(with_role(&nodes, AccessibleRole::List)[0].label, "Tab dial");
}

#[test]
fn the_bounds_are_small_boxes_inside_the_window() {
    let (window, _) = scene(&mockup_groups(false), &mockup_tabs(false));
    let nodes = reading_order(&window);
    let small: Vec<Node> = nodes
        .iter()
        .filter(|node| node.role != AccessibleRole::List)
        .cloned()
        .collect();
    assert_bounds(&small, (516.0, 516.0));
    for node in &small {
        let rect = node.rect();
        assert!(rect.width <= 24.5 && rect.height <= 24.5, "{rect:?}");
    }
}

#[test]
fn three_tabs() {
    let (groups, tabs) = generated(1, 3, false);
    let (window, labels) = scene(&groups, &tabs);
    let nodes = reading_order(&window);
    let found = with_role(&nodes, AccessibleRole::Tab);
    assert_eq!(found.len(), 3);
    assert_eq!(with_role(&nodes, AccessibleRole::TabList).len(), 1);
    for (tab, label) in found.iter().zip(&labels) {
        assert_eq!(&tab.label, label);
    }
}

#[test]
fn sixty_tabs_have_sixty_distinct_labels() {
    let (groups, tabs) = generated(5, 60, false);
    let (window, labels) = scene(&groups, &tabs);
    let nodes = reading_order(&window);
    let found = with_role(&nodes, AccessibleRole::Tab);
    assert_eq!(found.len(), 60);
    let mut seen: Vec<&str> = found.iter().map(|tab| tab.label.as_str()).collect();
    assert!(seen.iter().all(|label| !label.is_empty()));
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 60);
    for (tab, label) in found.iter().zip(&labels) {
        assert_eq!(&tab.label, label);
    }
    assert_eq!(with_role(&nodes, AccessibleRole::TabList).len(), 5);
}

#[test]
fn the_default_action_of_a_tab_activates_it_with_its_id() {
    let tabs = mockup_tabs(false);
    let (window, _) = scene(&mockup_groups(false), &tabs);
    let activated = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&activated);
    window.on_tab_activated(move |id| sink.borrow_mut().push(id));

    let nodes = reading_order(&window);
    let found = with_role(&nodes, AccessibleRole::Tab);
    found[4].press();
    found[21].press();
    assert_eq!(*activated.borrow(), vec![tabs[4].id, tabs[21].id]);
}
