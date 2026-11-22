// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The Appearance section's layout cards and tab-group controls, read the way an assistive
//! technology reads them (lane `settings-ui`, brief item 1 and decision D17). The section is
//! filled from `mercury_appearance::panels::layout_panel` through the real bridge, as the desktop
//! fills it; each state's reading order is checked against the labelling rule and the bounds
//! VoiceOver is given, and frozen in `tests/a11y-snapshots/settings-appearance-*.txt` (the
//! section only: the sidebar's own golden is `windows-settings-appearance.txt`). Every switch and
//! card is found by role and label and pressed through its default action, which must report
//! what a click reports, and nothing while the control is disabled.
//!
//! `cargo test --release -p mercury-ui --features test-support --test a11y_settings_appearance`;
//! bless the golden files with `MERCURY_A11Y_BLESS=1` in front of it.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{self, Node};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use mercury_appearance::panels::layout_panel::{
    AppearanceAction, Choice, Control, LayoutPanel, layout_panel, set_choice, set_switch,
};
use mercury_settings::AppearanceSettings;
use mercury_ui::settings_sections::appearance;
use mercury_ui::settings_sections::section;
use mercury_ui::{AppearanceUi, SettingsWindow};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};

use AccessibleRole as Role;

/// Tall enough that the whole section is on screen, at the width of the settings tests of
/// `a11y_windows.rs`.
const WIDE: (f32, f32) = (900.0, 2600.0);

/// The narrowest settings window (`min-width: 560px`): a 200 px sidebar leaves 360 px for the
/// section, 312 px inside the page's 24 px padding.
const NARROW: (f32, f32) = (560.0, 3200.0);

/// The note of D4, which the window shows while the interface is not live.
const NOTE: &str = "These choices are saved, but the tab-group interface they control arrives in a \
                    later version. Until then they change nothing you see.";

/// What `connect`'s handler was called with, in order.
type Actions = Rc<RefCell<Vec<AppearanceAction>>>;

/// The settings window on the Appearance section, filled from `settings` as the desktop fills
/// it, and its handler recording every action.
fn appearance_window(
    settings: &AppearanceSettings,
    interface_live: bool,
    size: (f32, f32),
) -> (SettingsWindow, Actions) {
    a11y::init();
    let window = SettingsWindow::new().expect("a window");
    window.set_section(section::APPEARANCE);
    window.set_theme_index(0);
    window.set_show_bookmarks_bar(true);
    appearance::fill(&window, &layout_panel(settings, interface_live));
    let actions: Actions = Rc::default();
    appearance::connect(&window, {
        let actions = Rc::clone(&actions);
        move |action| actions.borrow_mut().push(action)
    });
    a11y::open_window(&window, size.0, size.1);
    (window, actions)
}

fn defaults() -> (SettingsWindow, Actions) {
    appearance_window(&AppearanceSettings::default(), true, WIDE)
}

fn taken(actions: &Actions) -> Vec<AppearanceAction> {
    actions.borrow().clone()
}

/// The section's own element, so the tests see it and not the sidebar or the controls above it.
fn section(window: &SettingsWindow) -> ElementHandle {
    ElementHandle::find_by_element_type_name(window, "AppearanceSection")
        .next()
        .expect("the Appearance section is shown")
}

fn read(window: &SettingsWindow) -> Vec<Node> {
    a11y::subtree(&section(window))
}

/// Reads the section, checks the labelling rule and the bounds, and compares the reading order
/// with `tests/a11y-snapshots/<snapshot>.txt`.
fn freeze(window: &SettingsWindow, snapshot: &str) -> Vec<Node> {
    let nodes = read(window);
    a11y::assert_labelled(&nodes, snapshot);
    a11y::assert_bounds(&nodes, a11y::window_size(window));
    a11y::assert_snapshot(snapshot, &nodes);
    nodes
}

fn find<'a>(nodes: &'a [Node], role: Role, label: &str) -> &'a Node {
    a11y::node_in(nodes, role, label)
}

fn press(window: &SettingsWindow, role: Role, label: &str) {
    find(&read(window), role, label).press();
}

fn labels(nodes: &[Node], role: Role) -> Vec<&str> {
    nodes
        .iter()
        .filter(|node| node.role == role)
        .map(|node| node.label.as_str())
        .collect()
}

const SWITCHES: [(&str, Control); 6] = [
    ("Group dock", Control::Dock),
    ("Ring from the toolbar hub", Control::Ring),
    ("Orbit overview", Control::Orbit),
    ("Pinch to open Orbit", Control::OrbitPinch),
    ("Group keys (⌥ + letter)", Control::Keys),
    ("Corner dial", Control::Dial),
];

/// Settings with Classic chosen.
fn classic() -> AppearanceSettings {
    let mut settings = AppearanceSettings::default();
    assert!(set_choice(&mut settings, Choice::Layout, 1));
    settings
}

/// Settings with Orbit and the corner dial off (their pinch and corner dependents keep their values).
fn orbit_and_dial_off() -> AppearanceSettings {
    let mut settings = AppearanceSettings::default();
    assert!(set_choice(&mut settings, Choice::DialCorner, 0));
    set_switch(&mut settings, Control::Orbit, false);
    set_switch(&mut settings, Control::Dial, false);
    settings
}

// ---------------------------------------------------------------------------------------------
// Reading order
// ---------------------------------------------------------------------------------------------

#[test]
fn the_mercury_defaults_read_top_to_bottom_as_drawn() {
    let (window, _) = defaults();

    let nodes = freeze(&window, "settings-appearance-mercury-defaults");

    // Layout, then the six switches in the order of the design, the lists between them.
    let order: Vec<(&str, &str)> = nodes
        .iter()
        .map(|node| (node.role_name(), node.label.as_str()))
        .collect();
    let position = |role: &str, label: &str| {
        order
            .iter()
            .position(|entry| *entry == (role, label))
            .unwrap_or_else(|| panic!("no {role} {label:?} in {order:?}"))
    };
    let expected = [
        ("text", "Layout"),
        ("radio-button", "Mercury (default)"),
        ("radio-button", "Classic"),
        ("text", "Tab group controls"),
        ("switch", "Group dock"),
        ("combobox", "Dock starts"),
        ("switch", "Ring from the toolbar hub"),
        ("switch", "Orbit overview"),
        ("switch", "Pinch to open Orbit"),
        ("switch", "Group keys (⌥ + letter)"),
        ("switch", "Corner dial"),
        ("combobox", "Dial corner"),
        ("combobox", "Memory for Orbit previews"),
    ];
    let places: Vec<usize> = expected
        .iter()
        .map(|(role, label)| position(role, label))
        .collect();
    assert!(
        places.windows(2).all(|pair| pair[0] < pair[1]),
        "the controls are read in this order: {expected:?}, but at {places:?} of {order:?}"
    );
    // The closing line is last, and the live interface has no note above "Layout".
    let last = nodes.last().expect("the section reads something");
    assert_eq!(last.role, Role::Text);
    assert!(last.label.starts_with("Classic keeps every control above"));
    assert_eq!(nodes[0].label, "Layout");
}

#[test]
fn classic_chosen_selects_its_card_and_reads_the_classic_dock_note() {
    let (window, _) = appearance_window(&classic(), true, WIDE);

    let nodes = freeze(&window, "settings-appearance-classic");

    assert_eq!(
        find(&nodes, Role::RadioButton, "Classic").checked,
        Some(true)
    );
    assert_eq!(
        find(&nodes, Role::RadioButton, "Mercury (default)").checked,
        Some(false)
    );
    let dock = find(&nodes, Role::Switch, "Group dock");
    assert_eq!(
        dock.checked,
        Some(false),
        "Classic's dock is off by default"
    );
    assert_eq!(
        dock.description,
        "Optional in Classic: a dock under the tab strip. Off unless you turn it on."
    );
}

#[test]
fn orbit_and_the_dial_off_grey_their_dependents_and_keep_their_values() {
    let (window, _) = appearance_window(&orbit_and_dial_off(), true, WIDE);

    let nodes = freeze(&window, "settings-appearance-orbit-and-dial-off");

    let pinch = find(&nodes, Role::Switch, "Pinch to open Orbit");
    assert!(!pinch.enabled, "greyed while Orbit is off");
    assert_eq!(pinch.checked, Some(true), "and keeping its value");
    let corner = find(&nodes, Role::Combobox, "Dial corner");
    assert!(!corner.enabled, "greyed while the dial is off");
    assert_eq!(corner.value, "Top left", "and keeping its corner");
    for (label, _) in SWITCHES {
        let node = find(&nodes, Role::Switch, label);
        let greyed = label == "Pinch to open Orbit";
        assert_eq!(node.enabled, !greyed, "{label}");
    }
}

#[test]
fn the_note_of_d4_is_read_first_and_only_while_the_interface_is_not_live() {
    let (window, _) = appearance_window(&AppearanceSettings::default(), false, WIDE);

    let nodes = freeze(&window, "settings-appearance-note-shown");

    assert_eq!(nodes[0].role, Role::Text);
    assert_eq!(nodes[0].label, NOTE, "above everything else");
    assert_eq!(nodes[1].label, "Layout");

    let (live, _) = defaults();
    assert!(
        !read(&live).iter().any(|node| node.label == NOTE),
        "no note once the interface is live"
    );
}

#[test]
fn every_control_has_a_label_and_the_roles_are_the_ones_the_design_asks_for() {
    let (window, _) = defaults();
    let nodes = read(&window);

    a11y::assert_labelled(&nodes, "the Appearance section");
    assert_eq!(
        labels(&nodes, Role::RadioButton),
        ["Mercury (default)", "Classic"]
    );
    assert_eq!(
        labels(&nodes, Role::Switch),
        SWITCHES.map(|(label, _)| label)
    );
    assert_eq!(
        labels(&nodes, Role::Combobox),
        ["Dock starts", "Dial corner", "Memory for Orbit previews"]
    );
    // Nothing is exposed as a bare image, a button or a checkbox: the drawings and the marks are
    // hidden, and each control says everything in its label and description.
    for role in [Role::Image, Role::Button, Role::Checkbox] {
        assert!(labels(&nodes, role).is_empty(), "{role:?}");
    }
    // Cards carry their place in the group and their description; switches their note.
    let mercury = find(&nodes, Role::RadioButton, "Mercury (default)");
    assert_eq!(mercury.item, Some((0, 2)));
    assert_eq!(
        mercury.description,
        "Toolbar on top, group dock at the bottom"
    );
    let classic = find(&nodes, Role::RadioButton, "Classic");
    assert_eq!(classic.item, Some((1, 2)));
    assert_eq!(classic.description, "Tabs in the title strip, group labels");
    for (label, _) in SWITCHES {
        let node = find(&nodes, Role::Switch, label);
        assert!(!node.description.is_empty(), "{label} has a note to read");
        assert!(node.checked.is_some(), "{label} reports its state");
    }
    assert_eq!(
        find(&nodes, Role::Switch, "Group keys (⌥ + letter)").description,
        "Assign letters in Tab groups."
    );
    assert_eq!(
        find(&nodes, Role::Combobox, "Memory for Orbit previews").value,
        "128 MB"
    );
}

// ---------------------------------------------------------------------------------------------
// Pressing
// ---------------------------------------------------------------------------------------------

#[test]
fn pressing_each_switch_reports_it_with_its_new_state_and_again_to_turn_it_back() {
    let (window, actions) = defaults();

    for (label, _) in SWITCHES {
        press(&window, Role::Switch, label);
    }
    assert_eq!(
        taken(&actions),
        SWITCHES.map(|(_, control)| AppearanceAction::Switch(control, false))
    );
    // The switches follow the press (they are bound both ways to the global).
    let after = read(&window);
    for (label, _) in SWITCHES {
        assert_eq!(
            find(&after, Role::Switch, label).checked,
            Some(false),
            "{label}"
        );
    }

    actions.borrow_mut().clear();
    for (label, _) in SWITCHES {
        press(&window, Role::Switch, label);
    }
    assert_eq!(
        taken(&actions),
        SWITCHES.map(|(_, control)| AppearanceAction::Switch(control, true))
    );
}

#[test]
fn pressing_a_card_reports_the_layout_only_when_it_changes() {
    let (window, actions) = defaults();

    press(&window, Role::RadioButton, "Mercury (default)");
    assert!(
        taken(&actions).is_empty(),
        "the chosen layout again is no change"
    );

    press(&window, Role::RadioButton, "Classic");
    assert_eq!(
        taken(&actions),
        [AppearanceAction::Choice(Choice::Layout, 1)]
    );
    let after = read(&window);
    assert_eq!(
        find(&after, Role::RadioButton, "Classic").checked,
        Some(true)
    );
    assert_eq!(
        find(&after, Role::RadioButton, "Mercury (default)").checked,
        Some(false)
    );
    press(&window, Role::RadioButton, "Classic");
    assert_eq!(
        taken(&actions).len(),
        1,
        "the chosen card again reports nothing"
    );

    press(&window, Role::RadioButton, "Mercury (default)");
    assert_eq!(
        taken(&actions),
        [
            AppearanceAction::Choice(Choice::Layout, 1),
            AppearanceAction::Choice(Choice::Layout, 0)
        ]
    );
}

#[test]
fn the_lists_report_their_name_and_index_and_a_name_this_version_does_not_know_is_dropped() {
    let (window, actions) = defaults();
    let ui = window.global::<AppearanceUi>();

    ui.invoke_chosen("dock-starts".into(), 1);
    ui.invoke_chosen("dial-corner".into(), 2);
    ui.invoke_chosen("snapshot-cap".into(), 4);
    ui.invoke_chosen("layout".into(), 1);
    ui.invoke_toggled("orbit-pinch".into(), false);
    // Unknown names and a negative index reach nobody.
    ui.invoke_chosen("palette".into(), 0);
    ui.invoke_chosen("snapshot-cap".into(), -1);
    ui.invoke_toggled("show-home-button".into(), true);
    ui.invoke_toggled(SharedString::new(), true);

    assert_eq!(
        taken(&actions),
        [
            AppearanceAction::Choice(Choice::DockStarts, 1),
            AppearanceAction::Choice(Choice::DialCorner, 2),
            AppearanceAction::Choice(Choice::SnapshotCap, 4),
            AppearanceAction::Choice(Choice::Layout, 1),
            AppearanceAction::Switch(Control::OrbitPinch, false),
        ]
    );
}

#[test]
fn a_disabled_control_refuses_its_own_action_and_keeps_its_value() {
    let (window, actions) = appearance_window(&orbit_and_dial_off(), true, WIDE);

    // Pressed through the accessibility action (what VoiceOver does), the greyed pinch switch
    // does nothing: no report, no change of state.
    press(&window, Role::Switch, "Pinch to open Orbit");
    assert!(taken(&actions).is_empty());
    let after = read(&window);
    let pinch = find(&after, Role::Switch, "Pinch to open Orbit");
    assert_eq!(pinch.checked, Some(true));
    assert!(!pinch.enabled);

    // And a click on it too.
    pinch.handle.mock_single_click(PointerEventButton::Left);
    assert!(taken(&actions).is_empty());
    assert_eq!(
        find(&read(&window), Role::Switch, "Pinch to open Orbit").checked,
        Some(true)
    );

    // The lists under a switch that is off are disabled too; the model fills them again once
    // the switch is on (the desktop refills after each report).
    let dock_off = {
        let mut settings = AppearanceSettings::default();
        set_switch(&mut settings, Control::Dock, false);
        settings
    };
    let (window, _) = appearance_window(&dock_off, true, WIDE);
    let nodes = read(&window);
    assert!(!find(&nodes, Role::Combobox, "Dock starts").enabled);
    assert!(find(&nodes, Role::Combobox, "Memory for Orbit previews").enabled);
    // Switching the parent back on enables the pinch switch again, and it works.
    let (window, actions) = appearance_window(&AppearanceSettings::default(), true, WIDE);
    let nodes = read(&window);
    assert!(find(&nodes, Role::Switch, "Pinch to open Orbit").enabled);
    press(&window, Role::Switch, "Pinch to open Orbit");
    assert_eq!(
        taken(&actions),
        [AppearanceAction::Switch(Control::OrbitPinch, false)]
    );
}

#[test]
fn a_click_in_the_reported_rectangle_does_what_the_default_action_does() {
    let (window, actions) = defaults();
    let all = read(&window);

    find(&all, Role::Switch, "Ring from the toolbar hub")
        .handle
        .mock_single_click(PointerEventButton::Left);
    find(&all, Role::RadioButton, "Classic")
        .handle
        .mock_single_click(PointerEventButton::Left);

    assert_eq!(
        taken(&actions),
        [
            AppearanceAction::Switch(Control::Ring, false),
            AppearanceAction::Choice(Choice::Layout, 1)
        ]
    );
}

/// Presses and releases each key in turn on the window: Tab moves the focus, Space and Return act.
fn press_keys(window: &SettingsWindow, presses: &[&str]) {
    for key in presses {
        let text = SharedString::from(*key);
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }
}

/// Tab stops before the section: the window's own focus scope is focused first, then the seven
/// sidebar tabs, the Theme list and the two check boxes. The next stops are the cards, then the
/// controls in the order they are drawn.
const TABS_TO_FIRST_CARD: usize = 7 + 1 + 2 + 1;

#[test]
fn a_focused_card_is_chosen_with_space_and_a_focused_switch_with_space_or_return() {
    const TAB: &str = "\t";

    // Mercury is the first stop, Classic the second.
    let (window, actions) = defaults();
    press_keys(&window, &[TAB; TABS_TO_FIRST_CARD + 1]);
    press_keys(&window, &[" "]);
    assert_eq!(
        taken(&actions),
        [AppearanceAction::Choice(Choice::Layout, 1)]
    );

    // The Group dock switch is the third stop.
    let (window, actions) = defaults();
    press_keys(&window, &[TAB; TABS_TO_FIRST_CARD + 2]);
    press_keys(&window, &["\n"]);
    assert_eq!(
        taken(&actions),
        [AppearanceAction::Switch(Control::Dock, false)],
        "Return turned the dock off"
    );

    // With Orbit off its pinch switch is greyed and Tab skips it: Dock, Starts, Ring, Orbit,
    // then Group keys.
    let (window, actions) = appearance_window(&orbit_and_dial_off(), true, WIDE);
    press_keys(&window, &[TAB; TABS_TO_FIRST_CARD + 5]);
    press_keys(&window, &[" "]);
    press_keys(&window, &[TAB, " "]);
    assert_eq!(
        taken(&actions),
        [
            AppearanceAction::Switch(Control::Orbit, true),
            AppearanceAction::Switch(Control::Keys, false)
        ],
        "Tab went from Orbit to Group keys, past the greyed pinch switch"
    );
}
// ---------------------------------------------------------------------------------------------
// Filling
// ---------------------------------------------------------------------------------------------

/// What the global holds, as the panel it was filled from.
fn global_as_panel(window: &SettingsWindow) -> LayoutPanel {
    let ui = window.global::<AppearanceUi>();
    let index = |value: i32| usize::try_from(value).expect("a list index");
    LayoutPanel {
        layout_index: index(ui.get_layout_index()),
        dock: ui.get_dock(),
        dock_note: ui.get_dock_note().to_string(),
        dock_starts_index: index(ui.get_dock_starts_index()),
        ring: ui.get_ring(),
        orbit: ui.get_orbit(),
        orbit_pinch: ui.get_orbit_pinch(),
        orbit_pinch_enabled: ui.get_orbit_pinch_enabled(),
        keys: ui.get_keys(),
        dial: ui.get_dial(),
        dial_corner_index: index(ui.get_dial_corner_index()),
        dial_corner_enabled: ui.get_dial_corner_enabled(),
        snapshot_cap_choices: ui
            .get_snapshot_cap_choices()
            .iter()
            .map(|text| text.to_string())
            .collect(),
        snapshot_cap_index: index(ui.get_snapshot_cap_index()),
        note: ui.get_note().to_string(),
    }
}

#[test]
fn fill_puts_every_value_of_the_panel_in_the_global_and_a_second_fill_replaces_them() {
    let mut odd = classic();
    odd.tab_groups.snapshot_cap_mb = 200;
    set_switch(&mut odd, Control::Dock, true);
    for settings in [
        AppearanceSettings::default(),
        classic(),
        orbit_and_dial_off(),
        odd,
    ] {
        for live in [true, false] {
            let (window, _) = appearance_window(&settings, live, WIDE);
            assert_eq!(
                global_as_panel(&window),
                layout_panel(&settings, live),
                "{settings:?} live {live}"
            );
        }
    }

    // A section that was used and is then filled again shows the new settings, not its own.
    let (window, _) = defaults();
    press(&window, Role::Switch, "Group dock");
    press(&window, Role::RadioButton, "Classic");
    let panel = layout_panel(&AppearanceSettings::default(), true);
    appearance::fill(&window, &panel);
    let nodes = read(&window);
    assert_eq!(find(&nodes, Role::Switch, "Group dock").checked, Some(true));
    assert_eq!(
        find(&nodes, Role::RadioButton, "Mercury (default)").checked,
        Some(true)
    );
    assert_eq!(global_as_panel(&window), panel);
}

#[test]
fn the_odd_memory_value_is_listed_and_selected_and_the_dock_note_names_the_layout() {
    let mut settings = AppearanceSettings::default();
    settings.tab_groups.snapshot_cap_mb = 200;
    let (window, _) = appearance_window(&settings, true, WIDE);
    let all = read(&window);
    assert_eq!(
        find(&all, Role::Combobox, "Memory for Orbit previews").value,
        "200 MB"
    );
    assert_eq!(
        find(&all, Role::Switch, "Group dock").description,
        "Bottom of the window. × shrinks it to a capsule; it never hides by itself."
    );
    let ui = window.global::<AppearanceUi>();
    assert_eq!(ui.get_snapshot_cap_choices().row_count(), 7);
}

// ---------------------------------------------------------------------------------------------
// Narrow and wide windows
// ---------------------------------------------------------------------------------------------

#[test]
fn at_560_pixels_the_cards_stack_and_nothing_leaves_the_column() {
    let (window, _) = appearance_window(&AppearanceSettings::default(), false, NARROW);
    let nodes = read(&window);
    a11y::assert_bounds(&nodes, a11y::window_size(&window));

    // The page's column: 200 px of sidebar and 24 px of padding on each side.
    let column = a11y::Rect::new(224.0, 0.0, 560.0 - 224.0 - 24.0, NARROW.1);
    for node in &nodes {
        assert!(
            column.contains(&node.rect()),
            "{} {:?} leaves the 312 px column: {:?}",
            node.role_name(),
            node.label,
            node.rect()
        );
    }
    let mercury = find(&nodes, Role::RadioButton, "Mercury (default)").rect();
    let classic = find(&nodes, Role::RadioButton, "Classic").rect();
    assert!(
        classic.y >= mercury.bottom(),
        "stacked: Classic below Mercury ({mercury:?}, {classic:?})"
    );
    assert!(
        (classic.width - mercury.width).abs() < a11y::TOLERANCE,
        "each as wide as the column"
    );
    assert!(
        mercury.width > 300.0,
        "a stacked card takes the column: {mercury:?}"
    );
}

#[test]
fn at_900_pixels_the_cards_sit_side_by_side_at_the_same_height() {
    let (window, _) = defaults();
    let nodes = read(&window);

    let mercury = find(&nodes, Role::RadioButton, "Mercury (default)").rect();
    let classic = find(&nodes, Role::RadioButton, "Classic").rect();
    assert!(
        (classic.y - mercury.y).abs() < a11y::TOLERANCE,
        "one row ({mercury:?}, {classic:?})"
    );
    assert!(
        classic.x >= mercury.right(),
        "Classic to the right of Mercury"
    );
    assert!((classic.width - mercury.width).abs() < a11y::TOLERANCE);
}

#[test]
fn the_cards_switch_from_stacked_to_side_by_side_at_a_section_width_of_460_pixels() {
    // Window widths whose section (window - 200 sidebar - 48 padding) straddles 460.
    for (width, stacked) in [
        (560.0, true),
        (700.0, true),
        (707.0, true),
        (709.0, false),
        (780.0, false),
    ] {
        let (window, _) = appearance_window(&AppearanceSettings::default(), true, (width, 3000.0));
        let nodes = read(&window);
        let mercury = find(&nodes, Role::RadioButton, "Mercury (default)").rect();
        let classic = find(&nodes, Role::RadioButton, "Classic").rect();
        assert_eq!(
            classic.y > mercury.y + 1.0,
            stacked,
            "window {width}: {mercury:?} {classic:?}"
        );
    }
}
