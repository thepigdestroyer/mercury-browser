// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The Tab groups section, read the way an assistive technology reads it (lane `settings-ui`,
//! brief item 3 and decision D17). The section is filled from
//! `mercury_appearance::panels::key_panel` through the real bridge, as the desktop fills it; each
//! state's reading order is checked against the labelling rule and the bounds VoiceOver is given,
//! and frozen in `tests/a11y-snapshots/settings-tab-groups-*.txt` (the section only). Every
//! callback is checked to reach `connect`'s handler as the right `PanelAction`, and the key cap is
//! typed into the way a keyboard does it: a letter, Backspace, a digit, and the keys that must
//! reach the window (Cmd+W, Escape, Tab).
//!
//! Group colours come from `PRESETS` by position and no test says which preset is first (D15).
//!
//! `cargo test --release -p mercury-ui --features test-support --test a11y_settings_tab_groups`;
//! bless the golden files with `MERCURY_A11Y_BLESS=1` in front of it.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use common::a11y::{self, Node};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use mercury_appearance::PRESETS;
use mercury_appearance::keymap::LetterKey;
use mercury_appearance::panels::key_panel::{
    GroupInput, KeyPanel, KeyboardFacts, PanelAction, PanelView,
};
use mercury_settings::{GroupKeySettings, KeyCapture};
use mercury_ui::settings_sections::section;
use mercury_ui::settings_sections::tab_groups;
use mercury_ui::{SettingsWindow, TabGroupsUi};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};

use AccessibleRole as Role;

/// Tall enough that the whole section is on screen, at the width of the settings tests of
/// `a11y_windows.rs`.
const WIDE: (f32, f32) = (900.0, 1700.0);

/// The narrowest settings window (`min-width: 560px`): a 200 px sidebar leaves 360 px for the
/// section, 312 px inside the page's 24 px padding.
const NARROW: (f32, f32) = (560.0, 2400.0);

/// The note of D4, which the window shows while the interface is not live.
const NOTE: &str = "These choices are saved, but the tab-group interface they control arrives in a \
                    later version. Until then they change nothing you see.";

type Actions = Rc<RefCell<Vec<PanelAction>>>;

fn key(letter: char) -> LetterKey {
    LetterKey::from_char(letter).expect("a letter")
}

/// A group: its colour is the preset at `colour` in the list, whatever that is.
fn group(id: u64, name: &str, colour: usize, tabs: usize, letter: Option<char>) -> GroupInput {
    GroupInput {
        id,
        name: name.to_owned(),
        colour: PRESETS[colour % PRESETS.len()].id.to_owned(),
        tabs,
        key: letter.map(key),
    }
}

/// The groups of the design's mockup: Shopping has no key yet (the test types S into it).
fn mockup_groups() -> Vec<GroupInput> {
    vec![
        group(1, "Research", 0, 5, Some('A')),
        group(2, "Lisbon trip", 1, 4, Some('S')),
        group(3, "Reading", 2, 6, Some('D')),
        group(4, "Shopping", 3, 3, None),
        group(5, "Work", 4, 2, Some('E')),
    ]
}

fn panel_of(
    groups: &[GroupInput],
    loose: usize,
    keys: &GroupKeySettings,
    interface_live: bool,
) -> KeyPanel {
    let mut panel = KeyPanel::new();
    panel.refresh(groups, loose, keys, &KeyboardFacts::us(), interface_live);
    panel
}

/// The mockup, with S typed into Shopping: a conflict with Lisbon trip, and Work's accent key.
fn mockup_view() -> PanelView {
    let mut panel = panel_of(&mockup_groups(), 2, &GroupKeySettings::default(), true);
    assert_eq!(panel.key_typed(4, "s"), None, "S is Lisbon trip's");
    panel.view()
}

fn plain_view() -> PanelView {
    let groups: Vec<GroupInput> = mockup_groups()
        .into_iter()
        .filter(|group| group.name != "Work")
        .collect();
    panel_of(&groups, 0, &GroupKeySettings::default(), true).view()
}

fn window_with(view: &PanelView, size: (f32, f32)) -> (SettingsWindow, Actions) {
    a11y::init();
    let window = SettingsWindow::new().expect("a window");
    window.set_section(section::TAB_GROUPS);
    window.set_theme_index(0);
    window.set_show_bookmarks_bar(true);
    tab_groups::fill(&window, view);
    let actions: Actions = Rc::default();
    tab_groups::connect(&window, {
        let actions = Rc::clone(&actions);
        move |action| actions.borrow_mut().push(action)
    });
    a11y::open_window(&window, size.0, size.1);
    // Showing the section reported that it opened; the tests look at what the user does next.
    actions.borrow_mut().clear();
    (window, actions)
}

fn taken(actions: &Actions) -> Vec<PanelAction> {
    actions.borrow().clone()
}

fn section(window: &SettingsWindow) -> ElementHandle {
    ElementHandle::find_by_element_type_name(window, "TabGroupsSection")
        .next()
        .expect("the Tab groups section is shown")
}

fn read(window: &SettingsWindow) -> Vec<Node> {
    a11y::subtree(&section(window))
}

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

fn texts(nodes: &[Node]) -> Vec<&str> {
    labels(nodes, Role::Text)
}

// ---------------------------------------------------------------------------------------------
// Reading order
// ---------------------------------------------------------------------------------------------

#[test]
fn the_mockup_reads_title_table_conflict_note_loose_and_the_radios_in_order() {
    let (window, _) = window_with(&mockup_view(), WIDE);

    let nodes = freeze(&window, "settings-tab-groups-mockup");

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
        ("text", "Group keys"),
        ("switch", "Show keys beside names"),
        ("text", "Research"),
        ("text-input", "Key for Research"),
        ("text", "Lisbon trip"),
        ("text-input", "Key for Lisbon trip"),
        ("text", "Shopping"),
        ("text-input", "Key for Shopping"),
        ("button", "Swap keys for Shopping"),
        ("button", "Choose another key for Shopping"),
        ("text", "Work"),
        ("text-input", "Key for Work"),
        ("text", "Loose"),
        ("text", "When ⌥ switches groups"),
        (
            "radio-button",
            "Outside text fields, and anywhere after holding ⌥ for 0.4 s (recommended)",
        ),
        ("radio-button", "Only outside text fields"),
        (
            "radio-button",
            "Always: ⌥ + letter never types a character in Mercury",
        ),
        ("combobox", "Hold time"),
        ("button", "Reset to A, S, D, F, G…"),
    ];
    let places: Vec<usize> = expected
        .iter()
        .map(|(role, label)| position(role, label))
        .collect();
    assert!(
        places.windows(2).all(|pair| pair[0] < pair[1]),
        "the section is read in this order: {expected:?}, but at {places:?} of {order:?}"
    );
}

#[test]
fn a_plain_table_has_no_conflict_and_the_key_caps_say_their_letters() {
    let (window, _) = window_with(&plain_view(), WIDE);

    let nodes = freeze(&window, "settings-tab-groups-plain");

    assert!(labels(&nodes, Role::Button) == ["Reset to A, S, D, F, G…"]);
    let research = find(&nodes, Role::TextInput, "Key for Research");
    assert_eq!(research.value, "A");
    assert!(research.description.is_empty());
    assert_eq!(find(&nodes, Role::TextInput, "Key for Shopping").value, "");
}

#[test]
fn the_conflict_and_the_accent_key_say_what_is_wrong_in_the_key_caps_description() {
    let (window, _) = window_with(&mockup_view(), WIDE);
    let nodes = read(&window);

    let shopping = find(&nodes, Role::TextInput, "Key for Shopping");
    assert_eq!(shopping.description, "S is already Lisbon trip's key.");
    let work = find(&nodes, Role::TextInput, "Key for Work");
    assert_eq!(work.value, "E");
    assert_eq!(
        work.description,
        "⌥E is an accent key on the U.S. layout: it switches groups only outside text fields, or after the 0.4 s hold."
    );
    // The message is the description and is not read a second time as a text.
    assert!(!texts(&nodes).contains(&"S is already Lisbon trip's key."));
    // The buttons carry the message too, so a user who lands on one hears why it is there.
    assert_eq!(
        find(&nodes, Role::Button, "Swap keys for Shopping").description,
        "S is already Lisbon trip's key."
    );
}

#[test]
fn an_empty_window_says_so_and_a_window_with_groups_lists_its_loose_tabs() {
    let empty = panel_of(&[], 3, &GroupKeySettings::default(), false).view();
    let (window, _) = window_with(&empty, WIDE);

    let nodes = freeze(&window, "settings-tab-groups-empty");

    assert!(texts(&nodes).contains(
        &"No tab groups in this window yet. Make one from the tab strip once the tab-group interface is in."
    ));
    assert!(labels(&nodes, Role::TextInput).is_empty());
    assert!(
        !texts(&nodes).contains(&"Loose"),
        "no groups, no Loose line"
    );

    // Once the interface is live the sentence no longer promises it.
    let live = panel_of(&[], 0, &GroupKeySettings::default(), true).view();
    let (window, _) = window_with(&live, WIDE);
    assert!(
        texts(&read(&window))
            .contains(&"No tab groups in this window yet. Make one from the tab strip.")
    );

    // The Loose line is the count of tabs in no group, singular for one.
    let one_loose = panel_of(&mockup_groups(), 1, &GroupKeySettings::default(), true).view();
    let (window, _) = window_with(&one_loose, WIDE);
    assert!(texts(&read(&window)).contains(&"Loose"));
    assert!(texts(&read(&window)).contains(&"1 tab"));
}

#[test]
fn keys_switched_off_are_said_above_the_table_which_stays_editable() {
    let keys = GroupKeySettings {
        enabled: false,
        ..GroupKeySettings::default()
    };
    let view = panel_of(&mockup_groups(), 0, &keys, true).view();
    let (window, _) = window_with(&view, WIDE);

    let nodes = freeze(&window, "settings-tab-groups-keys-off");

    assert!(texts(&nodes).contains(&"Group keys are off. Turn them on in Appearance."));
    assert_eq!(labels(&nodes, Role::TextInput).len(), 5);
}

#[test]
fn the_always_choice_is_selected_and_its_dead_key_note_changes_and_the_hold_time_greys() {
    let keys = GroupKeySettings {
        capture: KeyCapture::Always,
        ..GroupKeySettings::default()
    };
    let view = panel_of(&mockup_groups(), 0, &keys, true).view();
    let (window, _) = window_with(&view, WIDE);

    let nodes = freeze(&window, "settings-tab-groups-capture-always");

    let always = find(
        &nodes,
        Role::RadioButton,
        "Always: ⌥ + letter never types a character in Mercury",
    );
    assert_eq!(always.checked, Some(true));
    assert_eq!(always.item, Some((2, 3)));
    assert_eq!(
        find(&nodes, Role::TextInput, "Key for Work").description,
        "⌥E is an accent key on the U.S. layout. With Always chosen it cannot start an accent in Mercury."
    );
    let hold = find(&nodes, Role::Combobox, "Hold time");
    assert!(
        !hold.enabled,
        "the hold time matters only for the first choice"
    );
    assert_eq!(hold.value, "0.4 s", "and it keeps its value");
}

#[test]
fn a_refused_key_says_why_in_red_and_offers_no_button() {
    let mut panel = panel_of(&mockup_groups(), 0, &GroupKeySettings::default(), true);
    assert_eq!(panel.key_typed(4, "7"), None);
    let (window, _) = window_with(&panel.view(), WIDE);

    let nodes = freeze(&window, "settings-tab-groups-refused");

    assert_eq!(
        find(&nodes, Role::TextInput, "Key for Shopping").description,
        "Type a letter from A to Z."
    );
    assert_eq!(labels(&nodes, Role::Button), ["Reset to A, S, D, F, G…"]);
}

#[test]
fn the_note_of_d4_is_read_first_and_only_while_the_interface_is_not_live() {
    let view = panel_of(&mockup_groups(), 0, &GroupKeySettings::default(), false).view();
    let (window, _) = window_with(&view, WIDE);

    let nodes = freeze(&window, "settings-tab-groups-note-shown");

    assert_eq!(nodes[0].role, Role::Text);
    assert_eq!(nodes[0].label, NOTE, "above everything else");
    assert_eq!(nodes[1].label, "Group keys");
    let (live, _) = window_with(&plain_view(), WIDE);
    assert!(
        read(&live).iter().all(|node| node.label != NOTE),
        "no note once the interface is live"
    );
}

#[test]
fn every_control_has_a_label_and_the_roles_are_the_ones_the_design_asks_for() {
    let (window, _) = window_with(&mockup_view(), WIDE);
    let nodes = read(&window);

    a11y::assert_labelled(&nodes, "the Tab groups section");
    assert_eq!(
        labels(&nodes, Role::TextInput),
        [
            "Key for Research",
            "Key for Lisbon trip",
            "Key for Reading",
            "Key for Shopping",
            "Key for Work"
        ]
    );
    assert_eq!(labels(&nodes, Role::Switch), ["Show keys beside names"]);
    assert_eq!(labels(&nodes, Role::RadioButton).len(), 3);
    assert_eq!(labels(&nodes, Role::Combobox), ["Hold time"]);
    for role in [Role::Image, Role::Checkbox] {
        assert!(labels(&nodes, role).is_empty(), "{role:?}");
    }
    let radios: Vec<_> = nodes
        .iter()
        .filter(|node| node.role == Role::RadioButton)
        .map(|node| node.item)
        .collect();
    assert_eq!(radios, [Some((0, 3)), Some((1, 3)), Some((2, 3))]);
}

// ---------------------------------------------------------------------------------------------
// Filling
// ---------------------------------------------------------------------------------------------

#[test]
fn fill_puts_every_value_of_the_view_in_the_global_and_a_second_fill_replaces_them() {
    let (window, _) = window_with(&mockup_view(), WIDE);
    let view = mockup_view();
    let ui = window.global::<TabGroupsUi>();

    let rows: Vec<_> = ui.get_rows().iter().collect();
    assert_eq!(rows.len(), view.rows.len());
    for (ui_row, row) in rows.iter().zip(&view.rows) {
        assert_eq!(u64::try_from(ui_row.id).ok(), Some(row.id));
        assert_eq!(ui_row.name, row.name.as_str());
        assert_eq!(ui_row.tabs, row.tabs_text.as_str());
        assert_eq!(ui_row.letter, row.letter.as_str());
        assert_eq!(ui_row.types, row.types.as_str());
        assert_eq!(ui_row.message, row.message.as_str());
        assert_eq!(
            ui_row.colour,
            slint::Color::from_argb_u8(row.colour.a, row.colour.r, row.colour.g, row.colour.b)
        );
    }
    let states: Vec<i32> = rows.iter().map(|row| row.state).collect();
    assert_eq!(
        states,
        [0, 0, 0, 2, 1],
        "plain, plain, plain, conflict, dead key"
    );
    assert_eq!(ui.get_loose_tabs(), 2);
    assert!(!ui.get_empty());
    assert!(ui.get_keys_enabled());
    assert_eq!(ui.get_types_header(), view.types_header.as_str());
    assert!(ui.get_show_beside_names());
    assert_eq!(ui.get_capture_index(), 0);
    assert_eq!(ui.get_hold_text(), "0.4 s");
    let hold: Vec<String> = ui
        .get_hold_choices()
        .iter()
        .map(|text| text.to_string())
        .collect();
    assert_eq!(hold, view.hold_choices);
    assert_eq!(ui.get_hold_index(), 1);
    assert_eq!(ui.get_reset_label(), "Reset to A, S, D, F, G…");
    assert_eq!(ui.get_reset_enabled(), view.reset_enabled);
    assert_eq!(ui.get_note(), "");

    // Filled again with another view, nothing of the first one is left.
    tab_groups::fill(
        &window,
        &panel_of(&[], 0, &GroupKeySettings::default(), false).view(),
    );
    assert_eq!(ui.get_rows().row_count(), 0);
    assert!(ui.get_empty());
    assert_eq!(ui.get_note(), NOTE);
}

#[test]
fn a_row_whose_id_does_not_fit_slints_int_is_skipped_not_wrapped() {
    let big = u64::try_from(i32::MAX).expect("positive") + 1;
    let view = panel_of(
        &[
            group(7, "Fits", 0, 1, None),
            group(big, "Too big", 1, 1, None),
        ],
        0,
        &GroupKeySettings::default(),
        true,
    )
    .view();
    let (window, _) = window_with(&view, WIDE);

    let ui = window.global::<TabGroupsUi>();
    let names: Vec<String> = ui
        .get_rows()
        .iter()
        .map(|row| row.name.to_string())
        .collect();
    assert_eq!(names, ["Fits"]);
}

#[test]
fn the_end_to_end_chain_from_the_panel_to_the_window_and_back_swaps_the_keys() {
    let mut panel = panel_of(&mockup_groups(), 0, &GroupKeySettings::default(), true);
    let (window, actions) = window_with(&panel.view(), WIDE);

    // The user types S into Shopping: the handler is told, runs the panel, which finds the conflict.
    let cap = find(&read(&window), Role::TextInput, "Key for Shopping")
        .handle
        .clone();
    cap.mock_single_click(PointerEventButton::Left);
    type_keys(&window, &["s"]);
    let PanelAction::KeyTyped { group, text } = taken(&actions).remove(0) else {
        panic!("a typed key");
    };
    assert_eq!((group, text.as_str()), (4, "s"));
    assert_eq!(panel.key_typed(group, &text), None);
    tab_groups::fill(&window, &panel.view());
    let shown = read(&window);
    assert_eq!(
        find(&shown, Role::TextInput, "Key for Shopping").description,
        "S is already Lisbon trip's key."
    );

    // Swap is pressed: the handler is told, and the panel promises the Change.
    press(&window, Role::Button, "Swap keys for Shopping");
    assert_eq!(taken(&actions).pop(), Some(PanelAction::Swap { group: 4 }));
    assert!(panel.swap(4).is_some());
}

// ---------------------------------------------------------------------------------------------
// Pressing and typing
// ---------------------------------------------------------------------------------------------

#[test]
fn each_callback_reaches_the_handler_as_the_right_action() {
    let (window, actions) = window_with(&mockup_view(), WIDE);

    press(&window, Role::Button, "Swap keys for Shopping");
    press(&window, Role::Button, "Choose another key for Shopping");
    press(&window, Role::Switch, "Show keys beside names");
    press(&window, Role::RadioButton, "Only outside text fields");
    press(
        &window,
        Role::RadioButton,
        "Always: ⌥ + letter never types a character in Mercury",
    );

    assert_eq!(
        taken(&actions),
        [
            PanelAction::Swap { group: 4 },
            PanelAction::ChooseAnother { group: 4 },
            PanelAction::ShowBesideNames(false),
            PanelAction::Capture(1),
            PanelAction::Capture(2),
        ]
    );

    let ui = window.global::<TabGroupsUi>();
    ui.invoke_chosen("hold".into(), 3);
    ui.invoke_toggled("show-beside-names".into(), true);
    ui.invoke_reset_keys();
    // Unknown names and negative ids or indices reach nobody.
    ui.invoke_chosen("palette".into(), 0);
    ui.invoke_chosen("hold".into(), -1);
    ui.invoke_toggled("ring".into(), true);
    ui.invoke_swap(-4);
    ui.invoke_choose_another(-1);
    ui.invoke_key_typed(-9, "a".into());
    assert_eq!(
        &taken(&actions)[5..],
        [
            PanelAction::Hold(3),
            PanelAction::ShowBesideNames(true),
            PanelAction::Reset
        ]
    );
}

#[test]
fn pressing_the_chosen_radio_again_reports_nothing_and_the_list_reports_its_index() {
    let (window, actions) = window_with(&mockup_view(), WIDE);

    press(
        &window,
        Role::RadioButton,
        "Outside text fields, and anywhere after holding ⌥ for 0.4 s (recommended)",
    );

    assert!(
        taken(&actions).is_empty(),
        "the chosen one again is no change"
    );
}

#[test]
fn the_section_reports_opened_once_when_it_appears_and_again_on_each_visit() {
    a11y::init();
    let window = SettingsWindow::new().expect("a window");
    let opened = Rc::new(Cell::new(0_usize));
    tab_groups::connect(&window, {
        let opened = Rc::clone(&opened);
        move |action| {
            if action == PanelAction::Opened {
                opened.set(opened.get() + 1);
            }
        }
    });
    a11y::open_window(&window, WIDE.0, WIDE.1);
    assert_eq!(opened.get(), 0, "another section is shown");

    window.set_section(section::TAB_GROUPS);
    a11y::open_window(&window, WIDE.0, WIDE.1);
    assert_eq!(opened.get(), 1);

    window.set_section(section::APPEARANCE);
    let _ = a11y::reading_order(&window);
    window.set_section(section::TAB_GROUPS);
    let _ = a11y::reading_order(&window);
    assert_eq!(opened.get(), 2, "each visit refreshes the table");
}

/// Presses and releases each key in turn on the window.
fn type_keys(window: &SettingsWindow, keys: &[&str]) {
    for key in keys {
        let text = SharedString::from(*key);
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }
}

fn key_code(key: Key) -> String {
    SharedString::from(key).to_string()
}

/// Holds `modifier`, types `text`, lets go.
fn type_with(window: &SettingsWindow, modifier: Key, text: &str) {
    let held = SharedString::from(key_code(modifier));
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: held.clone() });
    type_keys(window, &[text]);
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: held });
}

/// A window on the mockup with the first key cap focused, and what the window did when asked to close.
fn focused_cap() -> (SettingsWindow, Actions, Rc<Cell<usize>>) {
    let (window, actions) = window_with(&mockup_view(), WIDE);
    let closed = Rc::new(Cell::new(0_usize));
    window.on_close_settings({
        let closed = Rc::clone(&closed);
        move || closed.set(closed.get() + 1)
    });
    find(&read(&window), Role::TextInput, "Key for Research")
        .handle
        .mock_single_click(PointerEventButton::Left);
    (window, actions, closed)
}

#[test]
fn typing_into_the_key_cap_reports_a_letter_backspace_and_what_is_not_a_letter() {
    let (window, actions, closed) = focused_cap();

    type_keys(&window, &["q"]);
    type_keys(&window, &["Q"]);
    type_keys(&window, &[&key_code(Key::Backspace)]);
    type_keys(&window, &[&key_code(Key::Delete)]);
    type_keys(&window, &["7"]);
    type_keys(&window, &[" "]);
    type_keys(&window, &["é"]);

    let typed = |text: &str| PanelAction::KeyTyped {
        group: 1,
        text: text.to_owned(),
    };
    assert_eq!(
        taken(&actions),
        [
            typed("q"),
            typed("Q"),
            typed(""),
            typed(""),
            typed("7"),
            typed(" "),
            typed("é")
        ]
    );
    assert_eq!(closed.get(), 0);
}

#[test]
fn the_keys_that_are_not_characters_and_the_shortcuts_reach_the_window_not_the_key_cap() {
    let (window, actions, closed) = focused_cap();

    // Modifiers alone, arrows and function keys are not typed.
    for key in [
        Key::Shift,
        Key::Alt,
        Key::UpArrow,
        Key::LeftArrow,
        Key::F1,
        Key::Home,
    ] {
        type_keys(&window, &[&key_code(key)]);
    }
    assert!(taken(&actions).is_empty(), "{:?}", taken(&actions));

    // Command (Slint's `control` on macOS) with a letter is a shortcut, not a typed key, and the
    // same for Option, which is the key group keys are made with.
    type_with(&window, Key::Control, "q");
    assert!(taken(&actions).is_empty());
    type_with(&window, Key::Alt, "a");
    assert!(taken(&actions).is_empty());
    // Cmd+Q asked the window to quit and nothing was typed; it did not close the window.
    assert_eq!(closed.get(), 0);

    // Cmd+W closes the window although the key cap has the focus.
    type_with(&window, Key::Control, "w");
    assert_eq!(closed.get(), 1, "Cmd+W reached the window");
    assert!(taken(&actions).is_empty());

    // Escape too.
    type_keys(&window, &[&key_code(Key::Escape)]);
    assert_eq!(closed.get(), 2, "Escape reached the window");
    assert!(taken(&actions).is_empty());
}

#[test]
fn tab_leaves_the_key_cap_for_the_next_control_and_return_types_nothing() {
    let (window, actions, _) = focused_cap();

    type_keys(&window, &["\n"]);
    assert!(taken(&actions).is_empty(), "Return is not a character");

    type_keys(&window, &["\t"]);
    type_keys(&window, &["z"]);
    let typed = taken(&actions);
    assert_eq!(
        typed,
        [PanelAction::KeyTyped {
            group: 2,
            text: "z".to_owned()
        }],
        "the focus moved on to the next group's key cap"
    );
}

#[test]
fn an_assistive_technology_that_sets_the_value_types_it() {
    let (window, actions) = window_with(&mockup_view(), WIDE);

    find(&read(&window), Role::TextInput, "Key for Reading")
        .handle
        .set_accessible_value("g");

    assert_eq!(
        taken(&actions),
        [PanelAction::KeyTyped {
            group: 3,
            text: "g".to_owned()
        }]
    );
}

#[test]
fn a_click_in_the_reported_rectangle_does_what_the_default_action_does() {
    let (window, actions) = window_with(&mockup_view(), WIDE);
    let all = read(&window);

    find(&all, Role::Switch, "Show keys beside names")
        .handle
        .mock_single_click(PointerEventButton::Left);
    find(&all, Role::RadioButton, "Only outside text fields")
        .handle
        .mock_single_click(PointerEventButton::Left);
    find(&all, Role::Button, "Choose another key for Shopping")
        .handle
        .mock_single_click(PointerEventButton::Left);

    assert_eq!(
        taken(&actions),
        [
            PanelAction::ShowBesideNames(false),
            PanelAction::Capture(1),
            PanelAction::ChooseAnother { group: 4 },
        ]
    );
}

#[test]
fn reset_is_greyed_when_there_is_nothing_to_reset_and_then_does_nothing() {
    // Every group already has its sequence letter, in order.
    let groups = vec![
        group(1, "One", 0, 1, Some('A')),
        group(2, "Two", 1, 1, Some('S')),
    ];
    let view = panel_of(&groups, 0, &GroupKeySettings::default(), true).view();
    assert!(!view.reset_enabled);
    let (window, actions) = window_with(&view, WIDE);

    assert!(!find(&read(&window), Role::Button, "Reset to A, S, D, F, G…").enabled);
    press(&window, Role::Button, "Reset to A, S, D, F, G…");
    assert!(
        taken(&actions).is_empty(),
        "a disabled button refuses its own action"
    );

    // With something to reset it works.
    let (window, actions) = window_with(&mockup_view(), WIDE);
    press(&window, Role::Button, "Reset to A, S, D, F, G…");
    assert_eq!(taken(&actions), [PanelAction::Reset]);
}

// ---------------------------------------------------------------------------------------------
// Narrow and wide windows, and many groups
// ---------------------------------------------------------------------------------------------

#[test]
fn at_560_pixels_nothing_leaves_the_column_and_the_types_column_drops_below_the_name() {
    for view in [mockup_view(), plain_view()] {
        let (window, _) = window_with(&view, NARROW);
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
        // Every key cap is still there and reachable.
        assert!(!labels(&nodes, Role::TextInput).is_empty());
    }
}

#[test]
fn at_900_pixels_the_key_caps_line_up_in_one_column_with_the_types_column_beside_them() {
    let (window, _) = window_with(&plain_view(), WIDE);
    let nodes = read(&window);

    let caps: Vec<a11y::Rect> = nodes
        .iter()
        .filter(|node| node.role == Role::TextInput)
        .map(Node::rect)
        .collect();
    assert_eq!(caps.len(), 4);
    for cap in &caps {
        assert!((cap.x - caps[0].x).abs() < a11y::TOLERANCE, "{caps:?}");
        assert!(
            cap.width >= 40.0 && cap.height >= 30.0,
            "a key cap is a target: {cap:?}"
        );
    }
    for pair in caps.windows(2) {
        assert!(
            pair[1].y > pair[0].bottom() - a11y::TOLERANCE,
            "one row below the other"
        );
    }
}

#[test]
fn a_window_of_64_groups_renders_every_row_and_the_controls_below_them() {
    let groups: Vec<GroupInput> = (1..=64_u64)
        .map(|id| {
            group(
                id,
                &format!("Group number {id} with a rather long name to be elided"),
                usize::try_from(id).expect("small"),
                usize::try_from(id).expect("small"),
                None,
            )
        })
        .collect();
    let view = panel_of(&groups, 5, &GroupKeySettings::default(), true).view();

    for size in [(900.0, 6000.0), (560.0, 9000.0)] {
        let (window, _) = window_with(&view, size);
        let nodes = read(&window);
        assert_eq!(labels(&nodes, Role::TextInput).len(), 64);
        let last_cap = nodes
            .iter()
            .rfind(|node| node.role == Role::TextInput)
            .expect("a key cap")
            .rect();
        let hold = find(&nodes, Role::Combobox, "Hold time").rect();
        assert!(
            hold.y > last_cap.bottom(),
            "the capture choices come after the table ({last_cap:?}, {hold:?})"
        );
        a11y::assert_bounds(&nodes, a11y::window_size(&window));
    }
}

#[test]
fn the_show_keys_switch_sits_beside_the_title_when_wide_and_under_the_note_when_narrow() {
    let (wide, _) = window_with(&plain_view(), WIDE);
    let nodes = read(&wide);
    let title = find(&nodes, Role::Text, "Group keys").rect();
    let switch = find(&nodes, Role::Switch, "Show keys beside names").rect();
    assert!(
        (switch.y + switch.height / 2.0 - (title.y + title.height / 2.0)).abs() < 12.0,
        "on the title's line ({title:?}, {switch:?})"
    );
    assert!(switch.x > title.right());

    let (narrow, _) = window_with(&plain_view(), NARROW);
    let nodes = read(&narrow);
    let title = find(&nodes, Role::Text, "Group keys").rect();
    let switch = find(&nodes, Role::Switch, "Show keys beside names").rect();
    assert!(switch.y > title.bottom(), "stacked ({title:?}, {switch:?})");
}

// ---------------------------------------------------------------------------------------------
// The desktop's loop: every edit refills the section, and the keyboard focus must survive it
// ---------------------------------------------------------------------------------------------

/// A window whose handler does what the desktop's does: it runs the panel, applies the change it
/// asks for to a plain model of the groups, refreshes the panel and fills the section again.
fn refilling_window(
    groups: Vec<GroupInput>,
) -> (SettingsWindow, Actions, Rc<RefCell<Vec<GroupInput>>>) {
    a11y::init();
    let window = SettingsWindow::new().expect("a window");
    window.set_section(section::TAB_GROUPS);
    let groups = Rc::new(RefCell::new(groups));
    let panel = Rc::new(RefCell::new(KeyPanel::new()));
    let actions: Actions = Rc::default();
    let keys = GroupKeySettings::default();
    let facts = KeyboardFacts::us();
    let refill = {
        let (groups, panel, weak) = (Rc::clone(&groups), Rc::clone(&panel), window.as_weak());
        move || {
            panel
                .borrow_mut()
                .refresh(&groups.borrow(), 0, &keys, &facts, true);
            if let Some(window) = weak.upgrade() {
                tab_groups::fill(&window, &panel.borrow().view());
            }
        }
    };
    refill();
    tab_groups::connect(&window, {
        let (groups, panel, actions) = (Rc::clone(&groups), Rc::clone(&panel), Rc::clone(&actions));
        move |action| {
            actions.borrow_mut().push(action.clone());
            let change = match &action {
                PanelAction::KeyTyped { group, text } => panel.borrow_mut().key_typed(*group, text),
                PanelAction::Swap { group } => panel.borrow_mut().swap(*group),
                PanelAction::ChooseAnother { group } => {
                    panel.borrow_mut().choose_another(*group);
                    None
                }
                _ => None,
            };
            apply_change(&mut groups.borrow_mut(), change);
            refill();
        }
    });
    a11y::open_window(&window, WIDE.0, WIDE.1);
    actions.borrow_mut().clear();
    (window, actions, groups)
}

fn apply_change(
    groups: &mut [GroupInput],
    change: Option<mercury_appearance::panels::key_panel::Change>,
) {
    use mercury_appearance::panels::key_panel::Change;
    match change {
        Some(Change::SetKey { group, key }) => {
            if let Some(found) = groups.iter_mut().find(|candidate| candidate.id == group) {
                found.key = key;
            }
        }
        Some(Change::Swap { group, other }) => {
            let keys = |id: u64| {
                groups
                    .iter()
                    .find(|candidate| candidate.id == id)
                    .map(|g| g.key)
            };
            if let (Some(mine), Some(theirs)) = (keys(group), keys(other)) {
                for found in groups.iter_mut() {
                    if found.id == group {
                        found.key = theirs;
                    } else if found.id == other {
                        found.key = mine;
                    }
                }
            }
        }
        None => {}
    }
}

fn key_of(groups: &Rc<RefCell<Vec<GroupInput>>>, id: u64) -> Option<LetterKey> {
    groups
        .borrow()
        .iter()
        .find(|group| group.id == id)
        .and_then(|group| group.key)
}

fn click_cap(window: &SettingsWindow, name: &str) {
    find(&read(window), Role::TextInput, &format!("Key for {name}"))
        .handle
        .mock_single_click(PointerEventButton::Left);
}

fn typed(group: u64, text: &str) -> PanelAction {
    PanelAction::KeyTyped {
        group,
        text: text.to_owned(),
    }
}

#[test]
fn typing_two_keys_in_a_row_keeps_the_focus_in_the_same_cap() {
    let (window, actions, groups) = refilling_window(mockup_groups());

    click_cap(&window, "Research");
    type_keys(&window, &["q"]);
    type_keys(&window, &["w"]);

    // Both keys reached the same cap: the section was refilled after the first and kept it focused.
    assert_eq!(taken(&actions), [typed(1, "q"), typed(1, "w")]);
    assert_eq!(key_of(&groups, 1), Some(key('W')));
    assert_eq!(
        find(&read(&window), Role::TextInput, "Key for Research").value,
        "W"
    );
}

#[test]
fn a_conflict_keeps_the_cap_focused_and_choose_another_leaves_the_focus_in_it() {
    let (window, actions, groups) = refilling_window(mockup_groups());

    click_cap(&window, "Shopping");
    type_keys(&window, &["s"]);
    // The conflict appeared (the row changed in place); typing again still lands in the cap.
    assert_eq!(taken(&actions), [typed(4, "s")]);
    assert_eq!(
        find(&read(&window), Role::TextInput, "Key for Shopping").description,
        "S is already Lisbon trip's key."
    );

    press(&window, Role::Button, "Choose another key for Shopping");
    assert_eq!(
        taken(&actions).pop(),
        Some(PanelAction::ChooseAnother { group: 4 })
    );
    assert!(
        read(&window)
            .iter()
            .all(|node| node.role != Role::Button || node.label.starts_with("Reset")),
        "the conflict is gone with its buttons"
    );

    // The button that was pressed has gone; the focus is in the cap, where the user types again.
    type_keys(&window, &["x"]);
    assert_eq!(taken(&actions).pop(), Some(typed(4, "x")));
    assert_eq!(key_of(&groups, 4), Some(key('X')));
}

#[test]
fn swap_leaves_the_focus_in_the_cap_of_the_group_that_took_the_letter() {
    let (window, actions, groups) = refilling_window(mockup_groups());

    click_cap(&window, "Shopping");
    type_keys(&window, &["s"]);
    press(&window, Role::Button, "Swap keys for Shopping");

    assert_eq!(taken(&actions).pop(), Some(PanelAction::Swap { group: 4 }));
    assert_eq!(
        key_of(&groups, 4),
        Some(key('S')),
        "Shopping took the letter"
    );
    assert_eq!(
        key_of(&groups, 2),
        None,
        "Lisbon trip had none to give back"
    );
    type_keys(&window, &["k"]);
    assert_eq!(
        taken(&actions).pop(),
        Some(typed(4, "k")),
        "the focus is in Shopping's cap"
    );
    assert_eq!(key_of(&groups, 4), Some(key('K')));
}

#[test]
fn a_conflict_cap_shows_the_letter_that_was_typed_in_it() {
    let (window, _, _) = refilling_window(mockup_groups());

    click_cap(&window, "Shopping");
    type_keys(&window, &["s"]);

    let ui = window.global::<TabGroupsUi>();
    let shopping = ui.get_rows().iter().find(|row| row.id == 4).expect("a row");
    assert_eq!(shopping.attempted, "S");
    assert_eq!(shopping.letter, "", "the stored key is still none");
    let others: Vec<String> = ui
        .get_rows()
        .iter()
        .filter(|row| row.id != 4)
        .map(|row| row.attempted.to_string())
        .collect();
    assert!(others.iter().all(String::is_empty));
}

#[test]
fn filling_with_groups_that_came_went_and_moved_ends_with_exactly_the_rows_of_the_view() {
    let (window, _) = window_with(&mockup_view(), WIDE);
    let ids = |window: &SettingsWindow| -> Vec<i32> {
        window
            .global::<TabGroupsUi>()
            .get_rows()
            .iter()
            .map(|row| row.id)
            .collect()
    };
    assert_eq!(ids(&window), [1, 2, 3, 4, 5]);

    let groups = mockup_groups();
    for order in [
        vec![5_usize, 3],       // fewer
        vec![3, 5, 1, 0, 2, 4], // moved: indices into a copy
        vec![],                 // none
        vec![0, 1, 2, 3, 4],    // all again
    ] {
        let mut chosen: Vec<GroupInput> = order
            .iter()
            .filter_map(|&at| groups.get(at % groups.len().max(1)).cloned())
            .collect();
        chosen.dedup_by_key(|group| group.id);
        let mut seen = std::collections::HashSet::new();
        chosen.retain(|group| seen.insert(group.id));
        let view = panel_of(&chosen, 0, &GroupKeySettings::default(), true).view();
        tab_groups::fill(&window, &view);
        assert_eq!(
            ids(&window),
            chosen
                .iter()
                .map(|group| i32::try_from(group.id).expect("small"))
                .collect::<Vec<_>>(),
            "{order:?}"
        );
        let names: Vec<String> = window
            .global::<TabGroupsUi>()
            .get_rows()
            .iter()
            .map(|row| row.name.to_string())
            .collect();
        assert_eq!(
            names,
            chosen
                .iter()
                .map(|group| group.name.clone())
                .collect::<Vec<_>>()
        );
    }
}
