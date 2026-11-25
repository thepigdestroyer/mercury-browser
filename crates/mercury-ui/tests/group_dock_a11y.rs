// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The group dock as an assistive technology reads it (SPEC §5, brief D10), through Slint's testing
//! backend: a `tab-list` "Group dock", the folded pills as `tab`s, the current group's pill as a
//! `tab-list` whose chips are `tab`s, the two buttons, the capsule as one button, and the default
//! actions of each.

mod common;
mod group_dock_common;

use common::a11y::{self, Node, node_in, reading_order};
use group_dock_common::Host;
use i_slint_backend_testing::AccessibleRole;
use mercury_ui::group_dock::DockOptions;
use mercury_ui::ring_bands::fixtures::{mockup_groups, mockup_tabs};
use mercury_ui::{GroupRow, TabRow};

fn host() -> Host {
    a11y::init();
    let host = Host::new(
        mockup_groups(false),
        mockup_tabs(false),
        DockOptions::default(),
    );
    host.open();
    host
}

fn pill_label(group: &GroupRow) -> String {
    let mut label = format!(
        "{}, tab group, {} {}",
        group.name,
        group.count,
        if group.count == 1 { "tab" } else { "tabs" }
    );
    if !group.key_label.is_empty() {
        label.push_str(&format!(", key Option {}", group.key_label));
    }
    if group.any_loading {
        label.push_str(", loading");
    }
    if group.any_audible {
        label.push_str(", playing sound");
    }
    if group.any_unread {
        label.push_str(", unread");
    }
    label
}

fn chip_label(tab: &TabRow, group: &GroupRow, index: usize, count: usize) -> String {
    let mut label = format!(
        "{}, {}, tab {} of {}",
        tab.title,
        group.name,
        index + 1,
        count
    );
    if tab.active {
        label.push_str(", current tab");
    }
    if tab.unread {
        label.push_str(", unread");
    }
    if tab.audible {
        label.push_str(", playing sound");
    }
    if tab.muted {
        label.push_str(", muted");
    }
    if tab.crashed {
        label.push_str(", crashed");
    }
    if tab.capture {
        label.push_str(", using camera or microphone");
    }
    if tab.loading {
        label.push_str(", loading");
    }
    label
}

fn nodes(host: &Host) -> Vec<Node> {
    reading_order(&host.window)
}

#[test]
fn the_dock_is_a_tab_list_with_a_pill_for_every_group() {
    let host = host();
    let nodes = nodes(&host);
    node_in(&nodes, AccessibleRole::TabList, "Group dock");
    for group in host.groups.iter().filter(|group| !group.active) {
        let pill = node_in(&nodes, AccessibleRole::Tab, &pill_label(group));
        assert_eq!(pill.selected, Some(false), "{}", group.name);
    }
    let current = host
        .groups
        .iter()
        .find(|group| group.active)
        .expect("a current group");
    node_in(
        &nodes,
        AccessibleRole::TabList,
        &format!("{}, current group", pill_label(current)),
    );
}

#[test]
fn the_current_pill_holds_its_chips_as_tabs_and_one_is_selected() {
    let host = host();
    let nodes = nodes(&host);
    let current = host
        .groups
        .iter()
        .find(|group| group.active)
        .expect("a current group");
    let tabs: Vec<&TabRow> = host
        .tabs
        .iter()
        .filter(|tab| tab.group == current.id)
        .collect();
    assert!(
        tabs.len() >= 2,
        "the fixture's current group has several tabs"
    );
    for (index, tab) in tabs.iter().enumerate() {
        let chip = node_in(
            &nodes,
            AccessibleRole::Tab,
            &chip_label(tab, current, index, tabs.len()),
        );
        assert_eq!(chip.selected, Some(tab.active), "{}", tab.title);
    }
    let selected = nodes
        .iter()
        .filter(|node| node.role == AccessibleRole::Tab && node.selected == Some(true))
        .count();
    assert_eq!(
        selected,
        1,
        "exactly one tab of the dock is selected:\n{}",
        a11y::dump(&nodes)
    );
}

#[test]
fn the_buttons_are_named_and_everything_else_has_no_role() {
    let host = host();
    let nodes = nodes(&host);
    node_in(
        &nodes,
        AccessibleRole::Button,
        "Shrink the group dock, Control Command B",
    );
    let current = host
        .groups
        .iter()
        .find(|group| group.active)
        .expect("a current group");
    node_in(
        &nodes,
        AccessibleRole::Button,
        &format!("New tab in {}", current.name),
    );
    for node in &nodes {
        assert!(
            matches!(
                node.role,
                AccessibleRole::TabList | AccessibleRole::Tab | AccessibleRole::Button
            ),
            "decoration must have no role, found {} {:?}",
            a11y::role_name(node.role),
            node.label
        );
        assert!(!node.label.is_empty(), "every role of the dock is labelled");
    }
}

#[test]
fn the_default_actions_do_what_a_click_does() {
    let host = host();
    let nodes = nodes(&host);
    let folded = host
        .groups
        .iter()
        .find(|group| !group.active)
        .expect("a folded group");
    node_in(&nodes, AccessibleRole::Tab, &pill_label(folded)).press();
    assert_eq!(host.take_log(), [format!("pill:{}", folded.id)]);

    let current = host
        .groups
        .iter()
        .find(|group| group.active)
        .expect("a current group");
    let tabs: Vec<&TabRow> = host
        .tabs
        .iter()
        .filter(|tab| tab.group == current.id)
        .collect();
    let second = tabs[1];
    node_in(
        &nodes,
        AccessibleRole::Tab,
        &chip_label(second, current, 1, tabs.len()),
    )
    .press();
    assert_eq!(host.take_log(), [format!("tab:{}", second.id)]);

    node_in(
        &nodes,
        AccessibleRole::Button,
        &format!("New tab in {}", current.name),
    )
    .press();
    assert_eq!(host.take_log(), [format!("new:{}", current.id)]);

    node_in(
        &nodes,
        AccessibleRole::Button,
        "Shrink the group dock, Control Command B",
    )
    .press();
    assert_eq!(host.take_log(), ["shrunk:true", "shrink"]);
    assert!(host.window.get_shrunk());
}

#[test]
fn the_shrunk_dock_is_one_button_that_restores_the_dock() {
    let host = host();
    host.window.set_shrunk(true);
    host.tick();
    let nodes = nodes(&host);
    let capsule = node_in(
        &nodes,
        AccessibleRole::Button,
        "Show the group dock, 22 tabs in 6 groups",
    );
    assert!(
        !nodes.iter().any(|node| node.label == "Group dock"),
        "the dock itself is hidden while shrunk:\n{}",
        a11y::dump(&nodes)
    );
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.role == AccessibleRole::Button)
            .count(),
        1,
        "the capsule is the only control while shrunk:\n{}",
        a11y::dump(&nodes)
    );
    capsule.press();
    assert_eq!(host.take_log(), ["shrunk:false", "expand"]);
    assert!(!host.window.get_shrunk());
    host.tick();
    node_in(&self::nodes(&host), AccessibleRole::TabList, "Group dock");
}
