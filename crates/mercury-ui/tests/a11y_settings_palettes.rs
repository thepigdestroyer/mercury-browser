// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The palette picker of the Appearance section, read the way an assistive technology reads it
//! (lane polish-1). The cards come from `palettes::selectable()` through the real bridge, so the
//! tests name no palette: they hold for the list the data has today and for one with Ember in it.
//! Each card is found by role and label and pressed through its default action, which must report
//! `Choice::Palette` with the card's place in the list, and nothing for the chosen card; the
//! keyboard path (Tab, then Space or Return) is the layout cards' own.
//!
//! `cargo test --release -p mercury-ui --features test-support --test a11y_settings_palettes`;
//! bless the golden files with `MERCURY_A11Y_BLESS=1` in front of it.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{self, Node};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use mercury_appearance::palettes::selectable;
use mercury_appearance::panels::layout_panel::{AppearanceAction, Choice, layout_panel, set_choice};
use mercury_appearance::panels::palette_panel::{palette_panel, set_palette};
use mercury_settings::AppearanceSettings;
use mercury_ui::SettingsWindow;
use mercury_ui::settings_sections::appearance;
use mercury_ui::settings_sections::section;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, SharedString};

use AccessibleRole as Role;

const WIDE: (f32, f32) = (900.0, 2600.0);
const NARROW: (f32, f32) = (560.0, 3200.0);

type Actions = Rc<RefCell<Vec<AppearanceAction>>>;

/// The settings window on the Appearance section, filled as the desktop fills it (layout panel
/// and palette picker), and its handler recording every action.
fn picker_window(settings: &AppearanceSettings, size: (f32, f32)) -> (SettingsWindow, Actions) {
    a11y::init();
    let window = SettingsWindow::new().expect("a window");
    window.set_section(section::APPEARANCE);
    window.set_theme_index(0);
    window.set_show_bookmarks_bar(true);
    appearance::fill(&window, &layout_panel(settings, true));
    appearance::fill_palettes(&window, &palette_panel(settings));
    let actions: Actions = Rc::default();
    appearance::connect(&window, {
        let actions = Rc::clone(&actions);
        move |action| actions.borrow_mut().push(action)
    });
    a11y::open_window(&window, size.0, size.1);
    (window, actions)
}

fn names() -> Vec<&'static str> {
    selectable().map(|palette| palette.name).collect()
}

fn section_of(window: &SettingsWindow) -> ElementHandle {
    ElementHandle::find_by_element_type_name(window, "AppearanceSection")
        .next()
        .expect("the Appearance section is shown")
}

fn read(window: &SettingsWindow) -> Vec<Node> {
    a11y::subtree(&section_of(window))
}

fn taken(actions: &Actions) -> Vec<AppearanceAction> {
    actions.borrow().clone()
}

/// The palette cards among the section's radio buttons: the first `selectable()` of them.
fn cards(nodes: &[Node]) -> Vec<&Node> {
    nodes
        .iter()
        .filter(|node| node.role == Role::RadioButton)
        .take(names().len())
        .collect()
}

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

/// Tab stops before the first palette card: the window's own focus scope, the seven sidebar tabs,
/// the Theme list and the two check boxes above the section.
const TABS_TO_FIRST_CARD: usize = 7 + 1 + 2 + 1;

#[test]
fn the_section_lists_one_radio_per_selectable_palette_in_the_data_order() {
    let (window, _) = picker_window(&AppearanceSettings::default(), WIDE);
    let nodes = read(&window);

    let shown: Vec<&str> = cards(&nodes)
        .iter()
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(shown, names());
    a11y::assert_labelled(&nodes, "the Appearance section with its palettes");
}

#[test]
fn the_group_is_read_title_then_note_then_the_cards_then_layout() {
    let (window, _) = picker_window(&AppearanceSettings::default(), WIDE);
    let nodes = freeze(&window, "settings-appearance-palettes");

    assert_eq!(nodes[0].role, Role::Text);
    assert_eq!(nodes[0].label, "Palette");
    assert_eq!(nodes[1].role, Role::Text);
    assert!(
        nodes[1].label.contains("light and a dark look"),
        "{:?}",
        nodes[1].label
    );
    let first_card = nodes
        .iter()
        .position(|node| node.role == Role::RadioButton)
        .expect("cards");
    assert_eq!(first_card, 2);
    let layout = nodes
        .iter()
        .position(|node| node.label == "Layout")
        .expect("the Layout title");
    assert_eq!(layout, 2 + names().len());
}

fn freeze(window: &SettingsWindow, snapshot: &str) -> Vec<Node> {
    let nodes = read(window);
    a11y::assert_labelled(&nodes, snapshot);
    a11y::assert_bounds(&nodes, a11y::window_size(window));
    a11y::assert_snapshot(snapshot, &nodes);
    nodes
}

#[test]
fn each_card_says_its_place_and_the_default_says_so() {
    let (window, _) = picker_window(&AppearanceSettings::default(), WIDE);
    let nodes = read(&window);
    let count = names().len();

    for (index, node) in cards(&nodes).into_iter().enumerate() {
        assert_eq!(node.item, Some((index, count)), "{}", node.label);
        let default = index == 0;
        assert_eq!(
            node.description,
            if default {
                "Default. Shown in light and dark."
            } else {
                "Shown in light and dark."
            },
            "{}",
            node.label
        );
    }
}

#[test]
fn the_drawings_and_captions_are_hidden_from_assistive_technology() {
    let (window, _) = picker_window(&AppearanceSettings::default(), WIDE);
    let nodes = read(&window);

    for role in [Role::Image, Role::Button, Role::Checkbox] {
        assert!(
            !nodes.iter().any(|node| node.role == role),
            "{role:?} is exposed"
        );
    }
    for caption in ["Light", "Dark", "Default"] {
        assert!(
            !nodes.iter().any(|node| node.label == caption),
            "the caption {caption:?} is read on its own"
        );
    }
}

#[test]
fn the_chosen_palette_is_the_checked_card() {
    for index in 0..names().len() {
        let mut settings = AppearanceSettings::default();
        assert!(set_palette(&mut settings, index));
        let (window, _) = picker_window(&settings, WIDE);
        let nodes = read(&window);
        let checked: Vec<bool> = cards(&nodes)
            .iter()
            .map(|node| node.checked == Some(true))
            .collect();
        let expected: Vec<bool> = (0..names().len()).map(|i| i == index).collect();
        assert_eq!(checked, expected, "palette {index} chosen");
    }
}

#[test]
fn a_hand_edited_pair_checks_no_card_and_reads_the_note() {
    let mut settings = AppearanceSettings::default();
    assert!(set_choice(&mut settings, Choice::Palette, names().len() - 1));
    settings.palette_dark = AppearanceSettings::default().palette_dark;
    let panel = palette_panel(&settings);
    assert_eq!(panel.selected, None);

    let (window, _) = picker_window(&settings, WIDE);
    let nodes = read(&window);

    assert!(cards(&nodes).iter().all(|node| node.checked == Some(false)));
    assert!(
        nodes.iter().any(|node| node.label == panel.note),
        "the note {:?} is read",
        panel.note
    );
}

#[test]
fn pressing_a_card_reports_its_index_and_the_chosen_card_reports_nothing() {
    let (window, actions) = picker_window(&AppearanceSettings::default(), WIDE);
    let labels = names();

    a11y::node_in(&read(&window), Role::RadioButton, labels[0]).press();
    assert!(taken(&actions).is_empty(), "the chosen palette again");

    a11y::node_in(&read(&window), Role::RadioButton, labels[1]).press();
    assert_eq!(
        taken(&actions),
        [AppearanceAction::Choice(Choice::Palette, 1)]
    );
    let after = read(&window);
    let checked: Vec<bool> = cards(&after)
        .iter()
        .map(|node| node.checked == Some(true))
        .collect();
    assert_eq!(checked.iter().filter(|on| **on).count(), 1);
    assert!(checked[1], "the card follows the press");

    a11y::node_in(&read(&window), Role::RadioButton, labels[1]).press();
    assert_eq!(taken(&actions).len(), 1, "the new choice again");
}

#[test]
fn every_card_reports_its_own_place() {
    for index in 1..names().len() {
        let (window, actions) = picker_window(&AppearanceSettings::default(), WIDE);
        let label = names()[index];
        a11y::node_in(&read(&window), Role::RadioButton, label).press();
        assert_eq!(
            taken(&actions),
            [AppearanceAction::Choice(Choice::Palette, index)],
            "{label}"
        );
    }
}

#[test]
fn a_negative_or_unknown_palette_report_reaches_nobody() {
    let (window, actions) = picker_window(&AppearanceSettings::default(), WIDE);
    let ui = window.global::<mercury_ui::AppearanceUi>();
    ui.invoke_chosen("palette".into(), -1);
    ui.invoke_chosen("palettes".into(), 0);
    assert!(taken(&actions).is_empty());
    // A index beyond the list reaches the handler (the desktop's `set_choice` refuses it).
    ui.invoke_chosen("palette".into(), 99);
    assert_eq!(
        taken(&actions),
        [AppearanceAction::Choice(Choice::Palette, 99)]
    );
}

#[test]
fn tab_reaches_the_cards_in_order_and_space_or_return_chooses_the_focused_one() {
    const TAB: &str = "\t";

    // The first stop is the first card (the chosen one: nothing to report), the second the next.
    let (window, actions) = picker_window(&AppearanceSettings::default(), WIDE);
    press_keys(&window, &[TAB; TABS_TO_FIRST_CARD]);
    press_keys(&window, &[" "]);
    assert!(
        taken(&actions).is_empty(),
        "Space on the chosen card is no change"
    );
    press_keys(&window, &[TAB, " "]);
    assert_eq!(
        taken(&actions),
        [AppearanceAction::Choice(Choice::Palette, 1)]
    );

    let (window, actions) = picker_window(&AppearanceSettings::default(), WIDE);
    press_keys(&window, &[TAB; TABS_TO_FIRST_CARD + 1]);
    press_keys(&window, &["\n"]);
    assert_eq!(
        taken(&actions),
        [AppearanceAction::Choice(Choice::Palette, 1)],
        "Return chooses too"
    );

    // After the last palette card the next stop is the first layout card.
    let (window, actions) = picker_window(&AppearanceSettings::default(), WIDE);
    press_keys(&window, &vec![TAB; TABS_TO_FIRST_CARD + names().len() + 1]);
    press_keys(&window, &[" "]);
    assert_eq!(
        taken(&actions),
        [AppearanceAction::Choice(Choice::Layout, 1)],
        "Classic is the second layout card"
    );
}

#[test]
fn at_the_narrowest_window_every_card_is_inside_it() {
    let (window, _) = picker_window(&AppearanceSettings::default(), NARROW);
    let nodes = freeze(&window, "settings-appearance-palettes-narrow");
    let section = a11y::rect_of(&section_of(&window));
    for node in cards(&nodes) {
        let rect = node.rect();
        assert!(
            section.contains(&rect),
            "{} at {rect:?} is outside the section {section:?}",
            node.label
        );
        assert!(rect.width >= 200.0, "{} is {} wide", node.label, rect.width);
    }
}

#[test]
fn without_palettes_the_group_is_not_drawn() {
    a11y::init();
    let window = SettingsWindow::new().expect("a window");
    window.set_section(section::APPEARANCE);
    appearance::fill(&window, &layout_panel(&AppearanceSettings::default(), true));
    a11y::open_window(&window, WIDE.0, WIDE.1);
    let nodes = read(&window);
    assert!(!nodes.iter().any(|node| node.label == "Palette"));
    assert_eq!(nodes[0].label, "Layout");
}
