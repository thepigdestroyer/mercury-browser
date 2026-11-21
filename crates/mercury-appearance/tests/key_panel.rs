// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Tab groups key panel (lane `settings-ui`, item 2 `key-panel`, brief decisions D9 to D12
//! and D14): the rows, the typed-character rules of D11, the dead-key notes of D12 in every
//! capture mode, the one pending conflict, Swap and Choose another, Reset, the capture and hold
//! choices, and a seeded random walk against a plain model of the controller.

use std::collections::{BTreeSet, HashSet};

use mercury_appearance::group_colour::preset;
use mercury_appearance::panels::key_panel::{
    Change, GroupInput, KeyPanel, KeyRow, KeyboardFacts, PanelView, RowState, hold_choices,
    set_capture, set_hold,
};
use mercury_appearance::{Colour, KeyMap, LetterKey, OptionChar, PRESETS, parse_sequence};
use mercury_settings::{GroupKeySettings, KeyCapture, Settings};

const CAPTURES: [KeyCapture; 3] = [
    KeyCapture::OutsideTextOrHold,
    KeyCapture::OutsideText,
    KeyCapture::Always,
];

const REFUSED: &str = "Type a letter from A to Z.";
const KEYS_OFF: &str = "Group keys are off. Turn them on in Appearance.";
const D4_NOTE: &str = "These choices are saved, but the tab-group interface they control arrives in a later version. Until then they change nothing you see.";

// ---- helpers ------------------------------------------------------------------------------

fn key(letter: char) -> LetterKey {
    LetterKey::from_char(letter).expect("a letter")
}

fn group(id: u64, name: &str, letter: Option<char>) -> GroupInput {
    GroupInput {
        id,
        name: name.to_owned(),
        colour: PRESETS[0].id.to_owned(),
        tabs: 3,
        key: letter.map(key),
    }
}

fn keys_with(capture: KeyCapture, hold_ms: u16) -> GroupKeySettings {
    GroupKeySettings {
        capture,
        hold_ms,
        ..GroupKeySettings::default()
    }
}

/// A panel refreshed with the given groups, the default key settings and the U.S. facts.
fn panel_of(groups: &[GroupInput]) -> KeyPanel {
    panel_with(groups, &GroupKeySettings::default())
}

fn panel_with(groups: &[GroupInput], keys: &GroupKeySettings) -> KeyPanel {
    let mut panel = KeyPanel::new();
    panel.refresh(groups, 0, keys, &KeyboardFacts::us(), true);
    panel
}

/// The mockup: Research A, Lisbon trip S, Reading D, Shopping (no key yet), Work E.
fn mockup() -> Vec<GroupInput> {
    vec![
        group(1, "Research", Some('A')),
        group(2, "Lisbon trip", Some('S')),
        group(3, "Reading", Some('D')),
        group(4, "Shopping", None),
        group(5, "Work", Some('E')),
    ]
}

fn row(view: &PanelView, id: u64) -> &KeyRow {
    view.rows
        .iter()
        .find(|row| row.id == id)
        .expect("a row for the group")
}

fn letters(view: &PanelView) -> Vec<String> {
    view.rows.iter().map(|row| row.letter.clone()).collect()
}

fn states(view: &PanelView) -> Vec<RowState> {
    view.rows.iter().map(|row| row.state).collect()
}

/// A plain model of the controller's keys: ring order, strict about what it accepts, so that a
/// `Change` the panel should never emit fails the test instead of being quietly repaired.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Model {
    groups: Vec<(u64, Option<LetterKey>)>,
}

impl Model {
    fn from(groups: &[GroupInput]) -> Self {
        Self {
            groups: groups.iter().map(|g| (g.id, g.key)).collect(),
        }
    }

    fn key_of(&self, id: u64) -> Option<LetterKey> {
        self.groups.iter().find(|g| g.0 == id).and_then(|g| g.1)
    }

    fn holder_of(&self, letter: LetterKey) -> Option<u64> {
        self.groups
            .iter()
            .find(|g| g.1 == Some(letter))
            .map(|g| g.0)
    }

    fn apply(&mut self, change: Change) {
        match change {
            Change::SetKey { group, key } => {
                if let Some(letter) = key {
                    assert_eq!(
                        self.holder_of(letter),
                        None,
                        "SetKey must name a free letter: {change:?} on {self:?}"
                    );
                }
                let entry = self
                    .groups
                    .iter_mut()
                    .find(|g| g.0 == group)
                    .expect("SetKey names a group of the window");
                entry.1 = key;
            }
            Change::Swap { group, other } => {
                assert_ne!(group, other, "a group is never swapped with itself");
                let first = self.groups.iter().position(|g| g.0 == group);
                let second = self.groups.iter().position(|g| g.0 == other);
                let (first, second) = first.zip(second).expect("both groups are in the window");
                let old = self.groups[first].1;
                self.groups[first].1 = self.groups[second].1;
                self.groups[second].1 = old;
            }
        }
    }

    fn apply_all(&mut self, changes: &[Change]) {
        for &change in changes {
            self.apply(change);
        }
    }

    fn inputs(&self, names: &dyn Fn(u64) -> String) -> Vec<GroupInput> {
        self.groups
            .iter()
            .map(|&(id, key)| GroupInput {
                id,
                name: names(id),
                colour: PRESETS[(id as usize) % PRESETS.len()].id.to_owned(),
                tabs: (id as usize) % 7,
                key,
            })
            .collect()
    }
}

/// What `KeyMap::reset_to_sequence` gives these groups.
fn reset_target(model: &Model, sequence: &str) -> Vec<(u64, Option<LetterKey>)> {
    let names: Vec<(u64, Option<&str>)> = model
        .groups
        .iter()
        .map(|&(id, key)| (id, key.map(LetterKey::code_name)))
        .collect();
    let (mut map, issues) = KeyMap::restore(names);
    assert!(issues.is_empty(), "{issues:?}");
    map.reset_to_sequence(&parse_sequence(sequence));
    map.entries().to_vec()
}

// ---- D11: what a typed character does -------------------------------------------------------

#[test]
fn a_free_letter_sets_the_key_in_every_capture_mode() {
    for capture in CAPTURES {
        let mut panel = panel_with(&mockup(), &keys_with(capture, 400));
        assert_eq!(
            panel.key_typed(4, "K"),
            Some(Change::SetKey {
                group: 4,
                key: Some(LetterKey::K)
            }),
            "{capture:?}"
        );
        // The panel is a pure function of its last refresh: the keys have not moved.
        assert_eq!(row(&panel.view(), 4).letter, "");
        assert_eq!(row(&panel.view(), 4).state, RowState::Plain);
    }
}

#[test]
fn a_lower_case_letter_is_the_same_as_a_capital() {
    for capture in CAPTURES {
        let mut panel = panel_with(&mockup(), &keys_with(capture, 400));
        assert_eq!(
            panel.key_typed(4, "k"),
            Some(Change::SetKey {
                group: 4,
                key: Some(LetterKey::K)
            })
        );
        assert_eq!(
            panel.key_typed(1, "z"),
            Some(Change::SetKey {
                group: 1,
                key: Some(LetterKey::Z)
            })
        );
    }
}

#[test]
fn every_ascii_letter_in_both_cases_names_its_key() {
    let mut panel = panel_of(&[group(1, "Only", None)]);
    for letter in LetterKey::ALL {
        for text in [
            letter.letter().to_string(),
            letter.letter().to_ascii_lowercase().to_string(),
        ] {
            assert_eq!(
                panel.key_typed(1, &text),
                Some(Change::SetKey {
                    group: 1,
                    key: Some(letter)
                }),
                "{text:?}"
            );
        }
    }
}

#[test]
fn typing_the_letter_the_group_already_has_changes_nothing_in_either_case() {
    for capture in CAPTURES {
        let mut panel = panel_with(&mockup(), &keys_with(capture, 400));
        assert_eq!(panel.key_typed(1, "A"), None);
        assert_eq!(panel.key_typed(1, "a"), None);
        let view = panel.view();
        assert_eq!(row(&view, 1).state, RowState::Plain);
        assert_eq!(row(&view, 1).message, "");
    }
}

#[test]
fn an_empty_text_takes_the_key_away() {
    for capture in CAPTURES {
        let mut panel = panel_with(&mockup(), &keys_with(capture, 400));
        assert_eq!(
            panel.key_typed(1, ""),
            Some(Change::SetKey {
                group: 1,
                key: None
            })
        );
    }
}

#[test]
fn backspace_on_a_group_without_a_key_changes_nothing() {
    for capture in CAPTURES {
        let mut panel = panel_with(&mockup(), &keys_with(capture, 400));
        assert_eq!(panel.key_typed(4, ""), None);
        let view = panel.view();
        assert_eq!(
            states(&view),
            vec![
                RowState::Plain,
                RowState::Plain,
                RowState::Plain,
                RowState::Plain,
                RowState::DeadKey
            ]
        );
        assert_eq!(row(&view, 4).message, "");
    }
}

#[test]
fn a_letter_another_group_holds_puts_the_row_in_conflict_and_changes_nothing() {
    for capture in CAPTURES {
        let mut panel = panel_with(&mockup(), &keys_with(capture, 400));
        assert_eq!(panel.key_typed(4, "S"), None);
        let view = panel.view();
        let shopping = row(&view, 4);
        assert_eq!(shopping.state, RowState::Conflict);
        assert_eq!(shopping.message, "S is already Lisbon trip's key.");
        // No letter is ever shown twice: the red cap keeps the stored key (here none).
        assert_eq!(shopping.letter, "");
        assert_eq!(row(&view, 2).letter, "S");
        assert_eq!(row(&view, 2).state, RowState::Plain);
    }
}

#[test]
fn a_lower_case_letter_another_group_holds_is_a_conflict_too() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(4, "s"), None);
    let view = panel.view();
    assert_eq!(row(&view, 4).state, RowState::Conflict);
    assert_eq!(row(&view, 4).message, "S is already Lisbon trip's key.");
}

#[test]
fn a_group_with_a_key_in_conflict_keeps_showing_its_own_key() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(3, "S"), None);
    let view = panel.view();
    assert_eq!(row(&view, 3).state, RowState::Conflict);
    assert_eq!(row(&view, 3).letter, "D");
    assert_eq!(row(&view, 3).types, "\u{2202}");
    assert_eq!(row(&view, 3).message, "S is already Lisbon trip's key.");
}

#[test]
fn text_that_is_not_one_ascii_letter_is_refused_and_leaves_the_key_alone() {
    let refused = [
        "1",
        "0",
        " ",
        "-",
        "?",
        "\u{e9}",
        "\u{df}",
        "\u{ff21}",
        "ab",
        "AB",
        "A ",
        " A",
        "A1",
        "\n",
        "\t",
        "\u{0}",
        "A\u{301}",
        "\u{1f600}",
        "\u{2325}",
        "KeyA",
    ];
    for capture in CAPTURES {
        for text in refused {
            let mut panel = panel_with(&mockup(), &keys_with(capture, 400));
            assert_eq!(panel.key_typed(1, text), None, "{text:?}");
            let view = panel.view();
            assert_eq!(row(&view, 1).state, RowState::Refused, "{text:?}");
            assert_eq!(row(&view, 1).message, REFUSED, "{text:?}");
            assert_eq!(
                row(&view, 1).letter,
                "A",
                "{text:?}: the key is not touched"
            );
            // Nothing else moved.
            assert_eq!(letters(&view), vec!["A", "S", "D", "", "E"], "{text:?}");
        }
    }
}

#[test]
fn a_refusal_on_a_group_without_a_key_works_too() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(4, "7"), None);
    let view = panel.view();
    assert_eq!(row(&view, 4).state, RowState::Refused);
    assert_eq!(row(&view, 4).message, REFUSED);
    assert_eq!(row(&view, 4).letter, "");
}

#[test]
fn a_huge_typed_text_is_refused_without_reading_it_all() {
    let mut panel = panel_of(&mockup());
    let long = "A".repeat(1_000_000);
    assert_eq!(panel.key_typed(1, &long), None);
    assert_eq!(row(&panel.view(), 1).state, RowState::Refused);
}

#[test]
fn a_key_typed_for_a_group_not_in_the_list_does_nothing_and_does_not_panic() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(99, "K"), None);
    assert_eq!(panel.key_typed(99, ""), None);
    assert_eq!(panel.key_typed(99, "7"), None);
    assert_eq!(panel.key_typed(u64::MAX, "S"), None);
    assert_eq!(panel.swap(99), None);
    panel.choose_another(99);
    let before = panel_of(&mockup()).view();
    assert_eq!(
        panel.view(),
        before,
        "an unknown group leaves the panel as it was"
    );

    // And it does not disturb a pending conflict on a real group.
    assert_eq!(panel.key_typed(4, "S"), None);
    assert_eq!(panel.key_typed(99, "K"), None);
    assert_eq!(row(&panel.view(), 4).state, RowState::Conflict);
}

#[test]
fn an_empty_panel_ignores_every_action() {
    let mut panel = KeyPanel::new();
    assert_eq!(panel.key_typed(1, "A"), None);
    assert_eq!(panel.swap(1), None);
    panel.choose_another(1);
    assert_eq!(panel.reset_keys(), vec![]);
    let view = panel.view();
    assert!(view.empty);
    assert!(view.rows.is_empty());
}

// ---- the conflict: Swap, Choose another, what clears it ------------------------------------

#[test]
fn swap_yields_the_change_that_was_promised_and_clears_the_conflict() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(3, "S"), None);
    assert_eq!(panel.swap(3), Some(Change::Swap { group: 3, other: 2 }));
    assert_eq!(states(&panel.view())[2], RowState::Plain);
    assert_eq!(
        panel.swap(3),
        None,
        "the conflict is gone: nothing left to swap"
    );

    // The promise: Reading gets S, Lisbon trip gets Reading's old D.
    let mut model = Model::from(&mockup());
    model.apply(Change::Swap { group: 3, other: 2 });
    assert_eq!(model.key_of(3), Some(LetterKey::S));
    assert_eq!(model.key_of(2), Some(LetterKey::D));
}

#[test]
fn swap_from_a_group_with_no_key_leaves_the_holder_without_one() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(4, "S"), None);
    assert_eq!(panel.swap(4), Some(Change::Swap { group: 4, other: 2 }));
    let mut model = Model::from(&mockup());
    model.apply(Change::Swap { group: 4, other: 2 });
    assert_eq!(model.key_of(4), Some(LetterKey::S));
    assert_eq!(model.key_of(2), None);
}

#[test]
fn swap_without_a_conflict_is_none() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.swap(1), None);
    assert_eq!(panel.swap(4), None);
    // A conflict on another group is not this group's conflict, and it stays.
    assert_eq!(panel.key_typed(4, "S"), None);
    assert_eq!(panel.swap(1), None);
    assert_eq!(row(&panel.view(), 4).state, RowState::Conflict);
}

#[test]
fn choose_another_dismisses_the_conflict_and_keeps_the_old_key() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(3, "S"), None);
    panel.choose_another(3);
    let view = panel.view();
    assert_eq!(row(&view, 3).state, RowState::Plain);
    assert_eq!(row(&view, 3).message, "");
    assert_eq!(row(&view, 3).letter, "D");
    assert_eq!(panel.swap(3), None);
}

#[test]
fn choose_another_on_a_different_group_leaves_the_conflict() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(3, "S"), None);
    panel.choose_another(1);
    panel.choose_another(4);
    assert_eq!(row(&panel.view(), 3).state, RowState::Conflict);
}

#[test]
fn another_edit_replaces_the_pending_conflict() {
    // A different group gets a free letter.
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(3, "S"), None);
    assert!(panel.key_typed(4, "K").is_some());
    assert!(
        states(&panel.view())
            .iter()
            .all(|s| *s != RowState::Conflict)
    );
    assert_eq!(panel.swap(3), None);

    // A different group is refused: the refusal replaces the conflict.
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(3, "S"), None);
    assert_eq!(panel.key_typed(1, "9"), None);
    let view = panel.view();
    assert_eq!(row(&view, 3).state, RowState::Plain);
    assert_eq!(row(&view, 1).state, RowState::Refused);

    // A different group conflicts: the new conflict replaces the old one.
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(3, "S"), None);
    assert_eq!(panel.key_typed(4, "A"), None);
    let view = panel.view();
    assert_eq!(row(&view, 3).state, RowState::Plain);
    assert_eq!(row(&view, 4).state, RowState::Conflict);
    assert_eq!(row(&view, 4).message, "A is already Research's key.");
    assert_eq!(panel.swap(3), None);
    assert_eq!(panel.swap(4), Some(Change::Swap { group: 4, other: 1 }));

    // A different group's key is cleared, or typed again unchanged.
    for edit in ["", "A"] {
        let mut panel = panel_of(&mockup());
        assert_eq!(panel.key_typed(3, "S"), None);
        let _ = panel.key_typed(1, edit);
        assert_eq!(row(&panel.view(), 3).state, RowState::Plain, "{edit:?}");
    }

    // Reset is an edit too.
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(3, "S"), None);
    let _ = panel.reset_keys();
    assert_eq!(row(&panel.view(), 3).state, RowState::Plain);

    // The same group typing a new letter, or its own key back, replaces its conflict.
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(3, "S"), None);
    assert!(panel.key_typed(3, "K").is_some());
    assert_eq!(row(&panel.view(), 3).state, RowState::Plain);
    assert_eq!(panel.key_typed(3, "S"), None);
    assert_eq!(panel.key_typed(3, "D"), None);
    assert_eq!(row(&panel.view(), 3).state, RowState::Plain);
}

#[test]
fn a_refusal_is_replaced_by_the_next_edit_and_by_nothing_else() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(1, "!"), None);
    assert_eq!(row(&panel.view(), 1).state, RowState::Refused);
    // Another group, a free letter.
    assert!(panel.key_typed(4, "K").is_some());
    assert_eq!(row(&panel.view(), 1).state, RowState::Plain);
    // The same group, a good letter.
    assert_eq!(panel.key_typed(1, "!"), None);
    assert!(panel.key_typed(1, "Q").is_some());
    assert_eq!(row(&panel.view(), 1).state, RowState::Plain);
    // A refresh that leaves the group in place keeps the message.
    assert_eq!(panel.key_typed(1, "!"), None);
    let keys = keys_with(KeyCapture::Always, 400);
    panel.refresh(&mockup(), 2, &keys, &KeyboardFacts::us(), true);
    assert_eq!(row(&panel.view(), 1).state, RowState::Refused);
    // A refresh without the group drops it.
    panel.refresh(&mockup()[1..], 2, &keys, &KeyboardFacts::us(), true);
    assert!(
        states(&panel.view())
            .iter()
            .all(|s| *s != RowState::Refused)
    );
    panel.refresh(&mockup(), 2, &keys, &KeyboardFacts::us(), true);
    assert_eq!(row(&panel.view(), 1).state, RowState::Plain);
}

#[test]
fn a_refresh_that_leaves_the_conflict_true_keeps_it() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(4, "S"), None);

    // More tabs, a renamed bystander, a new group at the end, a new colour, other loose tabs.
    let mut groups = mockup();
    groups[0].tabs = 99;
    groups[0].name = "Research 2".to_owned();
    groups[3].colour = "#102030".to_owned();
    groups.push(group(6, "Late", Some('Q')));
    panel.refresh(
        &groups,
        5,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    let view = panel.view();
    assert_eq!(row(&view, 4).state, RowState::Conflict);
    assert_eq!(row(&view, 4).message, "S is already Lisbon trip's key.");

    // A new capture mode and hold do not touch a conflict's message.
    panel.refresh(
        &groups,
        5,
        &keys_with(KeyCapture::Always, 800),
        &KeyboardFacts::us(),
        false,
    );
    assert_eq!(row(&panel.view(), 4).state, RowState::Conflict);

    // The holder renamed: the message follows the stored name.
    groups[1].name = "Lisbon".to_owned();
    panel.refresh(
        &groups,
        5,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    assert_eq!(row(&panel.view(), 4).message, "S is already Lisbon's key.");

    // Swap still gives the promised change.
    assert_eq!(panel.swap(4), Some(Change::Swap { group: 4, other: 2 }));
}

#[test]
fn a_refresh_that_makes_the_conflict_untrue_drops_it() {
    let base = mockup();

    let cases: Vec<(&str, Vec<GroupInput>)> = {
        // The holder was closed.
        let closed: Vec<GroupInput> = base.iter().filter(|g| g.id != 2).cloned().collect();
        // The letter was freed.
        let mut freed = base.clone();
        freed[1].key = None;
        // The holder changed: another group now has S.
        let mut changed = base.clone();
        changed[1].key = Some(LetterKey::K);
        changed[2].key = Some(LetterKey::S);
        // The group itself now has the letter.
        let mut itself = base.clone();
        itself[1].key = None;
        itself[3].key = Some(LetterKey::S);
        // The group itself is gone.
        let gone: Vec<GroupInput> = base.iter().filter(|g| g.id != 4).cloned().collect();
        // Every group is gone.
        vec![
            ("the holder was closed", closed),
            ("the letter was freed", freed),
            ("the holder changed", changed),
            ("the group now holds the letter", itself),
            ("the group was closed", gone),
            ("no groups", Vec::new()),
        ]
    };
    for (what, groups) in cases {
        let mut panel = panel_of(&base);
        assert_eq!(panel.key_typed(4, "S"), None);
        panel.refresh(
            &groups,
            0,
            &GroupKeySettings::default(),
            &KeyboardFacts::us(),
            true,
        );
        let view = panel.view();
        assert!(
            states(&view).iter().all(|s| *s != RowState::Conflict),
            "{what}"
        );
        assert!(
            view.rows
                .iter()
                .all(|r| r.message.is_empty() || r.state == RowState::DeadKey),
            "{what}"
        );
        assert_eq!(panel.swap(4), None, "{what}");

        // It does not come back when the world becomes true again.
        panel.refresh(
            &base,
            0,
            &GroupKeySettings::default(),
            &KeyboardFacts::us(),
            true,
        );
        assert!(
            states(&panel.view())
                .iter()
                .all(|s| *s != RowState::Conflict),
            "{what}: came back"
        );
    }
}

#[test]
fn the_conflict_names_the_holder_as_stored() {
    let mut groups = mockup();
    groups[1].name = "<b>Lisbon</b> & \"co\"".to_owned();
    let mut panel = panel_of(&groups);
    assert_eq!(panel.key_typed(4, "S"), None);
    assert_eq!(
        row(&panel.view(), 4).message,
        "S is already <b>Lisbon</b> & \"co\"'s key."
    );
    assert_eq!(row(&panel.view(), 2).name, "<b>Lisbon</b> & \"co\"");
}

#[test]
fn a_holder_without_a_name_is_called_an_unnamed_group() {
    let mut groups = mockup();
    groups[1].name = String::new();
    let mut panel = panel_of(&groups);
    assert_eq!(panel.key_typed(4, "S"), None);
    assert_eq!(
        row(&panel.view(), 4).message,
        "S is already an unnamed group's key."
    );
    groups[1].name = "   ".to_owned();
    panel.refresh(
        &groups,
        0,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    assert_eq!(
        row(&panel.view(), 4).message,
        "S is already an unnamed group's key."
    );
}

// ---- D12: dead keys ---------------------------------------------------------------------------

#[test]
fn e_i_n_and_u_are_dead_on_the_us_table_and_the_others_are_not() {
    let groups: Vec<GroupInput> = LetterKey::ALL
        .iter()
        .enumerate()
        .map(|(i, k)| group(i as u64 + 1, &format!("G{k}"), Some(k.letter())))
        .collect();
    let panel = panel_of(&groups);
    let view = panel.view();
    for row in &view.rows {
        let dead = ['E', 'I', 'N', 'U'].contains(&row.letter.chars().next().expect("a letter"));
        assert_eq!(
            row.state,
            if dead {
                RowState::DeadKey
            } else {
                RowState::Plain
            },
            "{}",
            row.letter
        );
        assert_eq!(row.message.is_empty(), !dead, "{}", row.letter);
        assert_eq!(row.types.contains("(accent key)"), dead, "{}", row.letter);
    }
}

#[test]
fn the_dead_key_note_of_e_reads_as_the_brief_says_in_each_capture_mode() {
    let groups = [group(1, "Work", Some('E'))];
    let cases = [
        (
            KeyCapture::OutsideTextOrHold,
            "\u{2325}E is an accent key on the U.S. layout: it switches groups only outside text fields, or after the 0.4 s hold.",
        ),
        (
            KeyCapture::OutsideText,
            "\u{2325}E is an accent key on the U.S. layout: it switches groups only outside text fields.",
        ),
        (
            KeyCapture::Always,
            "\u{2325}E is an accent key on the U.S. layout. With Always chosen it cannot start an accent in Mercury.",
        ),
    ];
    for (capture, note) in cases {
        let panel = panel_with(&groups, &keys_with(capture, 400));
        let view = panel.view();
        assert_eq!(view.rows[0].state, RowState::DeadKey, "{capture:?}");
        assert_eq!(view.rows[0].message, note, "{capture:?}");
        assert_eq!(view.rows[0].types, "\u{b4} (accent key)");
    }
}

#[test]
fn the_dead_key_note_quotes_the_chosen_hold_and_follows_the_capture_mode_at_once() {
    let groups = [group(1, "Work", Some('I'))];
    let mut panel = KeyPanel::new();
    for (hold, text) in [
        (250, "0.25 s"),
        (400, "0.4 s"),
        (600, "0.6 s"),
        (800, "0.8 s"),
        (1000, "1 s"),
        (333, "0.333 s"),
        (270, "0.27 s"),
        (1, "0.001 s"),
    ] {
        panel.refresh(
            &groups,
            0,
            &keys_with(KeyCapture::OutsideTextOrHold, hold),
            &KeyboardFacts::us(),
            true,
        );
        assert_eq!(
            panel.view().rows[0].message,
            format!(
                "\u{2325}I is an accent key on the U.S. layout: it switches groups only outside text fields, or after the {text} hold."
            )
        );
        assert_eq!(panel.view().hold_text, text);
    }
    // The hold is not in the other two notes; a refresh with another mode changes the text.
    panel.refresh(
        &groups,
        0,
        &keys_with(KeyCapture::OutsideText, 600),
        &KeyboardFacts::us(),
        true,
    );
    assert!(!panel.view().rows[0].message.contains("0.6"));
    assert!(
        panel.view().rows[0]
            .message
            .ends_with("only outside text fields.")
    );
    panel.refresh(
        &groups,
        0,
        &keys_with(KeyCapture::Always, 600),
        &KeyboardFacts::us(),
        true,
    );
    assert!(
        panel.view().rows[0]
            .message
            .ends_with("cannot start an accent in Mercury.")
    );
}

#[test]
fn n_and_u_have_their_own_accents_in_the_types_column() {
    let groups = [
        group(1, "N", Some('N')),
        group(2, "U", Some('U')),
        group(3, "I", Some('I')),
    ];
    let view = panel_of(&groups).view();
    assert_eq!(view.rows[0].types, "\u{2dc} (accent key)");
    assert_eq!(view.rows[1].types, "\u{a8} (accent key)");
    assert_eq!(view.rows[2].types, "\u{2c6} (accent key)");
}

#[test]
fn the_types_column_shows_the_character_or_nothing() {
    let view = panel_of(&mockup()).view();
    assert_eq!(row(&view, 1).types, "\u{e5}"); // A: å
    assert_eq!(row(&view, 2).types, "\u{df}"); // S: ß
    assert_eq!(row(&view, 3).types, "\u{2202}"); // D: ∂
    assert_eq!(row(&view, 4).types, ""); // no key
    assert_eq!(row(&view, 5).types, "\u{b4} (accent key)"); // E
}

#[test]
fn learnt_facts_change_what_is_dead_and_what_is_typed() {
    let groups = [
        group(1, "Work", Some('E')),
        group(2, "Alpha", Some('A')),
        group(3, "Sigma", Some('S')),
    ];
    // On this keyboard E types é and A starts a grave accent.
    let facts = KeyboardFacts::us().with_learnt([
        (LetterKey::E, OptionChar::Char('\u{e9}')),
        (LetterKey::A, OptionChar::Dead('`')),
    ]);
    assert!(facts.is_learnt());
    let mut panel = KeyPanel::new();
    panel.refresh(&groups, 0, &GroupKeySettings::default(), &facts, true);
    let view = panel.view();

    // E stopped being dead.
    assert_eq!(row(&view, 1).state, RowState::Plain);
    assert_eq!(row(&view, 1).message, "");
    assert_eq!(row(&view, 1).types, "\u{e9}");
    // A started to be.
    assert_eq!(row(&view, 2).state, RowState::DeadKey);
    assert_eq!(row(&view, 2).types, "` (accent key)");
    assert_eq!(
        row(&view, 2).message,
        "\u{2325}A is an accent key on this keyboard: it switches groups only outside text fields, or after the 0.4 s hold."
    );
    // S was not mentioned: the table's answer stands.
    assert_eq!(row(&view, 3).state, RowState::Plain);
    assert_eq!(row(&view, 3).types, "\u{df}");

    // The same panel, back to the U.S. facts: the answers move back.
    panel.refresh(
        &groups,
        0,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    let view = panel.view();
    assert_eq!(row(&view, 1).state, RowState::DeadKey);
    assert_eq!(row(&view, 2).state, RowState::Plain);
    assert_eq!(row(&view, 2).types, "\u{e5}");
}

#[test]
fn learnt_facts_that_agree_with_the_table_still_count_as_learnt() {
    let facts = KeyboardFacts::us().with_learnt([(LetterKey::E, OptionChar::Dead('\u{b4}'))]);
    let mut panel = KeyPanel::new();
    panel.refresh(
        &[group(1, "Work", Some('E'))],
        0,
        &GroupKeySettings::default(),
        &facts,
        true,
    );
    let view = panel.view();
    assert_eq!(view.types_header, "\u{2325} + KEY TYPES ON THIS KEYBOARD");
    assert!(view.rows[0].message.contains("on this keyboard"));
}

#[test]
fn the_types_header_says_u_s_layout_until_something_is_learnt() {
    let mut panel = KeyPanel::new();
    assert_eq!(
        panel.view().types_header,
        "\u{2325} + KEY TYPES (U.S. LAYOUT)"
    );
    panel.refresh(
        &mockup(),
        0,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    assert_eq!(
        panel.view().types_header,
        "\u{2325} + KEY TYPES (U.S. LAYOUT)"
    );
    let learnt = KeyboardFacts::us().with_learnt([(LetterKey::A, OptionChar::Char('\u{e5}'))]);
    panel.refresh(&mockup(), 0, &GroupKeySettings::default(), &learnt, true);
    assert_eq!(
        panel.view().types_header,
        "\u{2325} + KEY TYPES ON THIS KEYBOARD"
    );
    // With nothing to learn from (an empty list) the facts are still marked learnt: that is
    // `KeyboardFacts`' own rule, and the header follows it.
    let none = KeyboardFacts::us().with_learnt([]);
    panel.refresh(&mockup(), 0, &GroupKeySettings::default(), &none, true);
    assert_eq!(
        panel.view().types_header,
        "\u{2325} + KEY TYPES ON THIS KEYBOARD"
    );
}

#[test]
fn a_dead_key_row_in_conflict_shows_the_conflict_not_the_note() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.key_typed(5, "S"), None);
    let view = panel.view();
    assert_eq!(row(&view, 5).state, RowState::Conflict);
    assert_eq!(row(&view, 5).message, "S is already Lisbon trip's key.");
    assert_eq!(row(&view, 5).types, "\u{b4} (accent key)");
    panel.choose_another(5);
    assert_eq!(row(&panel.view(), 5).state, RowState::DeadKey);
}

#[test]
fn typing_a_dead_key_is_allowed() {
    let mut panel = panel_of(&mockup());
    assert_eq!(
        panel.key_typed(4, "n"),
        Some(Change::SetKey {
            group: 4,
            key: Some(LetterKey::N)
        })
    );
}

// ---- rows, loose tabs, empty state, notes --------------------------------------------------

#[test]
fn the_rows_follow_the_input_order_and_say_how_many_tabs() {
    let mut groups = mockup();
    groups[0].tabs = 1;
    groups[1].tabs = 0;
    groups[2].tabs = 5;
    let view = panel_of(&groups).view();
    assert_eq!(
        view.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(
        view.rows
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Research", "Lisbon trip", "Reading", "Shopping", "Work"]
    );
    assert_eq!(row(&view, 1).tabs_text, "1 tab");
    assert_eq!(row(&view, 2).tabs_text, "0 tabs");
    assert_eq!(row(&view, 3).tabs_text, "5 tabs");
    assert_eq!(row(&view, 4).tabs_text, "3 tabs");
    assert_eq!(letters(&view), vec!["A", "S", "D", "", "E"]);
}

#[test]
fn colours_are_resolved_and_an_unreadable_one_is_slate() {
    let slate = preset("slate").expect("slate is a preset").base;
    let mut groups = mockup();
    groups[0].colour = PRESETS[0].id.to_owned();
    groups[1].colour = "#102030".to_owned();
    groups[2].colour = "#AABBCC".to_owned();
    groups[3].colour = "chartreuse".to_owned();
    groups[4].colour = String::new();
    let view = panel_of(&groups).view();
    assert_eq!(row(&view, 1).colour, PRESETS[0].base);
    assert_eq!(row(&view, 2).colour, Colour::rgb(0x10, 0x20, 0x30));
    assert_eq!(row(&view, 3).colour, Colour::rgb(0xaa, 0xbb, 0xcc));
    assert_eq!(row(&view, 4).colour, slate);
    assert_eq!(row(&view, 5).colour, slate);

    // The other unreadable forms: short hex, alpha forms, capitals, trailing text.
    for text in [
        "#fff",
        "#11223344",
        "BLUE",
        "#12345g",
        "#1234567",
        "#",
        "  #102030",
        "\u{0}",
    ] {
        let mut groups = mockup();
        groups[0].colour = text.to_owned();
        let view = panel_of(&groups).view();
        assert_eq!(row(&view, 1).colour, slate, "{text:?}");
    }
    // Every preset id resolves to its own colour.
    for preset in PRESETS.iter() {
        let mut groups = mockup();
        groups[0].colour = preset.id.to_owned();
        assert_eq!(row(&panel_of(&groups).view(), 1).colour, preset.base);
    }
}

#[test]
fn the_loose_count_is_handed_on() {
    let mut panel = KeyPanel::new();
    panel.refresh(
        &mockup(),
        2,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    assert_eq!(panel.view().loose_tabs, 2);
    assert!(!panel.view().empty);
    panel.refresh(
        &mockup(),
        0,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    assert_eq!(panel.view().loose_tabs, 0);
    panel.refresh(
        &mockup(),
        usize::MAX,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    assert_eq!(panel.view().loose_tabs, usize::MAX);
    // Loose is information: it never becomes a row and never takes a key.
    assert_eq!(panel.view().rows.len(), 5);
}

#[test]
fn no_groups_is_the_empty_state() {
    let mut panel = KeyPanel::new();
    panel.refresh(
        &[],
        7,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    let view = panel.view();
    assert!(view.empty);
    assert!(view.rows.is_empty());
    assert_eq!(
        view.loose_tabs, 7,
        "the count is handed on; the window shows Loose only with a group"
    );
    assert!(!view.reset_enabled, "nothing to reset");
    // A panel that was never refreshed is the empty state too.
    let fresh = KeyPanel::new().view();
    assert!(fresh.empty);
    assert_eq!(fresh.loose_tabs, 0);
    assert!(fresh.rows.is_empty());
    // The settings controls still read from the defaults.
    assert!(fresh.keys_enabled);
    assert_eq!(fresh.hold_text, "0.4 s");
    assert_eq!(fresh.capture_index, 0);
    assert!(fresh.show_beside_names);
    assert_eq!(fresh.reset_label, "Reset to A, S, D, F, G\u{2026}");
}

#[test]
fn the_note_of_the_missing_interface_is_empty_exactly_when_it_is_live() {
    let mut panel = KeyPanel::new();
    panel.refresh(
        &mockup(),
        0,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        false,
    );
    assert_eq!(panel.view().note, D4_NOTE);
    panel.refresh(
        &mockup(),
        0,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    assert_eq!(panel.view().note, "");
    // Before any refresh the interface is not assumed live.
    assert_eq!(KeyPanel::new().view().note, D4_NOTE);
}

#[test]
fn group_keys_off_gives_its_note_and_leaves_the_table_editable() {
    let keys = GroupKeySettings {
        enabled: false,
        ..GroupKeySettings::default()
    };
    let mut panel = panel_with(&mockup(), &keys);
    let view = panel.view();
    assert!(!view.keys_enabled);
    assert_eq!(view.keys_off_note, KEYS_OFF);
    assert_eq!(view.rows.len(), 5);
    // Still editable.
    assert_eq!(
        panel.key_typed(4, "k"),
        Some(Change::SetKey {
            group: 4,
            key: Some(LetterKey::K)
        })
    );
    assert_eq!(panel.key_typed(4, "S"), None);
    assert_eq!(row(&panel.view(), 4).state, RowState::Conflict);
    // On again: no note.
    panel.refresh(
        &mockup(),
        0,
        &GroupKeySettings::default(),
        &KeyboardFacts::us(),
        true,
    );
    let view = panel.view();
    assert!(view.keys_enabled);
    assert_eq!(view.keys_off_note, "");
}

#[test]
fn show_beside_names_follows_the_setting() {
    let mut keys = GroupKeySettings::default();
    assert!(panel_with(&mockup(), &keys).view().show_beside_names);
    keys.show_beside_names = false;
    assert!(!panel_with(&mockup(), &keys).view().show_beside_names);
}

// ---- capture and hold choices -----------------------------------------------------------------

#[test]
fn the_view_reports_the_capture_mode_as_an_index() {
    for (capture, index) in [
        (KeyCapture::OutsideTextOrHold, 0),
        (KeyCapture::OutsideText, 1),
        (KeyCapture::Always, 2),
    ] {
        assert_eq!(
            panel_with(&mockup(), &keys_with(capture, 400))
                .view()
                .capture_index,
            index
        );
    }
}

#[test]
fn set_capture_chooses_a_mode_and_refuses_another_index() {
    let mut keys = GroupKeySettings::default();
    assert!(set_capture(&mut keys, 2));
    assert_eq!(keys.capture, KeyCapture::Always);
    assert!(set_capture(&mut keys, 1));
    assert_eq!(keys.capture, KeyCapture::OutsideText);
    assert!(set_capture(&mut keys, 0));
    assert_eq!(keys.capture, KeyCapture::OutsideTextOrHold);

    let before = keys.clone();
    assert!(!set_capture(&mut keys, 3));
    assert!(!set_capture(&mut keys, 26));
    assert!(!set_capture(&mut keys, usize::MAX));
    assert_eq!(keys, before);
}

#[test]
fn set_capture_touches_nothing_but_the_capture() {
    let mut keys = GroupKeySettings {
        enabled: false,
        capture: KeyCapture::OutsideText,
        hold_ms: 777,
        show_beside_names: false,
        default_sequence: "QWE".to_owned(),
    };
    assert!(set_capture(&mut keys, 2));
    assert_eq!(
        keys,
        GroupKeySettings {
            enabled: false,
            capture: KeyCapture::Always,
            hold_ms: 777,
            show_beside_names: false,
            default_sequence: "QWE".to_owned(),
        }
    );
}

#[test]
fn the_hold_choices_are_the_five_times_plus_an_odd_stored_one_in_order() {
    let five = vec![250, 400, 600, 800, 1000];
    for current in five.iter().copied() {
        assert_eq!(hold_choices(current), five, "{current}");
    }
    assert_eq!(hold_choices(333), vec![250, 333, 400, 600, 800, 1000]);
    assert_eq!(hold_choices(251), vec![250, 251, 400, 600, 800, 1000]);
    assert_eq!(hold_choices(999), vec![250, 400, 600, 800, 999, 1000]);
    // A value the settings would repair is still listed as stored: the list never lies about
    // what is stored, and `Settings::validated` is what keeps such a value out of the window.
    assert_eq!(hold_choices(0), vec![0, 250, 400, 600, 800, 1000]);
    assert_eq!(hold_choices(1001), vec![250, 400, 600, 800, 1000, 1001]);
    assert_eq!(
        hold_choices(u16::MAX),
        vec![250, 400, 600, 800, 1000, u16::MAX]
    );
}

#[test]
fn the_view_lists_the_hold_choices_as_text_and_selects_the_stored_one() {
    let view = panel_with(&mockup(), &keys_with(KeyCapture::OutsideTextOrHold, 400)).view();
    assert_eq!(
        view.hold_choices,
        vec!["0.25 s", "0.4 s", "0.6 s", "0.8 s", "1 s"]
    );
    assert_eq!(view.hold_index, 1);
    assert_eq!(view.hold_text, "0.4 s");

    for (hold, index) in [(250, 0), (400, 1), (600, 2), (800, 3), (1000, 4)] {
        let view = panel_with(&mockup(), &keys_with(KeyCapture::Always, hold)).view();
        assert_eq!(view.hold_index, index, "{hold}");
    }

    let view = panel_with(&mockup(), &keys_with(KeyCapture::Always, 333)).view();
    assert_eq!(
        view.hold_choices,
        vec!["0.25 s", "0.333 s", "0.4 s", "0.6 s", "0.8 s", "1 s"]
    );
    assert_eq!(view.hold_index, 1);
    assert_eq!(view.hold_text, "0.333 s");
    assert_eq!(view.hold_choices[view.hold_index], view.hold_text);
}

#[test]
fn set_hold_chooses_an_item_of_the_list_and_refuses_an_index_outside_it() {
    let mut keys = GroupKeySettings::default();
    assert!(set_hold(&mut keys, 0));
    assert_eq!(keys.hold_ms, 250);
    assert!(set_hold(&mut keys, 4));
    assert_eq!(keys.hold_ms, 1000);
    assert!(set_hold(&mut keys, 2));
    assert_eq!(keys.hold_ms, 600);

    let before = keys.clone();
    assert!(!set_hold(&mut keys, 5));
    assert!(!set_hold(&mut keys, 100));
    assert!(!set_hold(&mut keys, usize::MAX));
    assert_eq!(keys, before);
}

#[test]
fn set_hold_counts_items_of_the_list_that_includes_an_odd_stored_value() {
    // 333 is item 1 of [250, 333, 400, 600, 800, 1000]: six items, so index 5 exists.
    let mut keys = GroupKeySettings {
        hold_ms: 333,
        ..GroupKeySettings::default()
    };
    assert!(set_hold(&mut keys, 1), "the stored value itself");
    assert_eq!(keys.hold_ms, 333);
    assert!(set_hold(&mut keys, 2));
    assert_eq!(keys.hold_ms, 400);

    let mut keys = GroupKeySettings {
        hold_ms: 333,
        ..GroupKeySettings::default()
    };
    assert!(set_hold(&mut keys, 5));
    assert_eq!(keys.hold_ms, 1000);

    let mut keys = GroupKeySettings {
        hold_ms: 333,
        ..GroupKeySettings::default()
    };
    assert!(!set_hold(&mut keys, 6));
    assert_eq!(keys.hold_ms, 333);
    assert!(set_hold(&mut keys, 0));
    assert_eq!(keys.hold_ms, 250);
}

#[test]
fn every_choice_the_window_offers_passes_validation_without_a_repair() {
    for stored in [250, 333, 400, 999, 1000] {
        let count = hold_choices(stored).len();
        for index in 0..count {
            for capture in 0..3 {
                let mut settings = Settings::default();
                settings.appearance.tab_groups.keys.hold_ms = stored;
                assert!(set_hold(&mut settings.appearance.tab_groups.keys, index));
                assert!(set_capture(
                    &mut settings.appearance.tab_groups.keys,
                    capture
                ));
                let before = settings.clone();
                let (validated, repairs) = settings.validated();
                assert!(
                    repairs.is_empty(),
                    "{stored} {index} {capture}: {repairs:?}"
                );
                assert_eq!(validated, before);
            }
        }
    }
}

// ---- reset --------------------------------------------------------------------------------------

#[test]
fn the_reset_label_is_built_from_the_first_five_letters_of_the_stored_sequence() {
    let label = |sequence: &str| {
        let keys = GroupKeySettings {
            default_sequence: sequence.to_owned(),
            ..GroupKeySettings::default()
        };
        panel_with(&mockup(), &keys).view().reset_label
    };
    assert_eq!(label("ASDFGHJKL"), "Reset to A, S, D, F, G\u{2026}");
    assert_eq!(label("QWERTYUIOP"), "Reset to Q, W, E, R, T\u{2026}");
    assert_eq!(label("ZXCVBN"), "Reset to Z, X, C, V, B\u{2026}");
    // Five letters or fewer: nothing more to say, so no ellipsis.
    assert_eq!(label("ASDFG"), "Reset to A, S, D, F, G");
    assert_eq!(label("ASD"), "Reset to A, S, D");
    assert_eq!(label("A"), "Reset to A");
    assert_eq!(label(""), "Reset to no keys");
    // Read the way the settings read it: letters in either case, repeats once, the rest skipped.
    assert_eq!(label("a-s-d-f-g-h"), "Reset to A, S, D, F, G\u{2026}");
    assert_eq!(label("AASSDDFFGG"), "Reset to A, S, D, F, G");
    assert_eq!(label("1234"), "Reset to no keys");
}

#[test]
fn reset_gives_the_groups_the_sequence_in_ring_order() {
    let groups = [
        group(1, "One", Some('Z')),
        group(2, "Two", Some('Y')),
        group(3, "Three", None),
    ];
    let mut panel = panel_of(&groups);
    assert!(panel.view().reset_enabled);
    let changes = panel.reset_keys();
    let mut model = Model::from(&groups);
    model.apply_all(&changes);
    assert_eq!(
        model.groups,
        vec![
            (1, Some(LetterKey::A)),
            (2, Some(LetterKey::S)),
            (3, Some(LetterKey::D))
        ]
    );
}

#[test]
fn reset_with_fewer_letters_than_groups_takes_the_keys_of_the_rest() {
    let groups = [
        group(1, "One", Some('Z')),
        group(2, "Two", Some('Y')),
        group(3, "Three", Some('X')),
        group(4, "Four", Some('W')),
    ];
    let keys = GroupKeySettings {
        default_sequence: "QW".to_owned(),
        ..GroupKeySettings::default()
    };
    let mut panel = panel_with(&groups, &keys);
    let changes = panel.reset_keys();
    let mut model = Model::from(&groups);
    model.apply_all(&changes);
    assert_eq!(
        model.groups,
        vec![
            (1, Some(LetterKey::Q)),
            (2, Some(LetterKey::W)),
            (3, None),
            (4, None)
        ]
    );
}

#[test]
fn reset_with_an_empty_sequence_takes_every_key_away() {
    let keys = GroupKeySettings {
        default_sequence: String::new(),
        ..GroupKeySettings::default()
    };
    let mut panel = panel_with(&mockup(), &keys);
    assert!(panel.view().reset_enabled);
    let changes = panel.reset_keys();
    assert_eq!(changes.len(), 4, "the four groups that had a key");
    let mut model = Model::from(&mockup());
    model.apply_all(&changes);
    assert!(model.groups.iter().all(|g| g.1.is_none()));

    // Keyless groups and an empty sequence: nothing to do.
    let groups = [group(1, "One", None), group(2, "Two", None)];
    let mut panel = panel_with(&groups, &keys);
    assert!(!panel.view().reset_enabled);
    assert_eq!(panel.reset_keys(), vec![]);
}

#[test]
fn reset_when_the_keys_already_are_the_sequence_changes_nothing() {
    let groups = [
        group(1, "One", Some('A')),
        group(2, "Two", Some('S')),
        group(3, "Three", Some('D')),
    ];
    let mut panel = panel_of(&groups);
    assert!(!panel.view().reset_enabled);
    assert_eq!(panel.reset_keys(), vec![]);

    // More groups than letters, the extra ones keyless: still already there.
    let keys = GroupKeySettings {
        default_sequence: "AS".to_owned(),
        ..GroupKeySettings::default()
    };
    let groups = [
        group(1, "One", Some('A')),
        group(2, "Two", Some('S')),
        group(3, "Three", None),
    ];
    let mut panel = panel_with(&groups, &keys);
    assert!(!panel.view().reset_enabled);
    assert_eq!(panel.reset_keys(), vec![]);
}

#[test]
fn reset_changes_apply_one_after_the_other_without_ever_naming_a_taken_letter() {
    // Every letter the reset wants is held by a later group: the changes must still go in order.
    let groups = [
        group(1, "One", Some('D')),
        group(2, "Two", Some('A')),
        group(3, "Three", Some('S')),
    ];
    let mut panel = panel_of(&groups);
    let changes = panel.reset_keys();
    assert!(!changes.is_empty());
    let mut model = Model::from(&groups);
    model.apply_all(&changes); // the model fails the test on a taken letter
    assert_eq!(
        model.groups,
        reset_target(&Model::from(&groups), "ASDFGHJKL")
    );

    // Rotations, reversals and gaps, every one of them.
    let mut rng = Rng::new(7);
    for _ in 0..300 {
        let count = 1 + rng.below(12);
        let mut pool: Vec<LetterKey> = LetterKey::ALL.to_vec();
        let groups: Vec<GroupInput> = (0..count)
            .map(|i| {
                let letter = if rng.below(5) == 0 || pool.is_empty() {
                    None
                } else {
                    let at = rng.below(pool.len());
                    Some(pool.swap_remove(at).letter())
                };
                group(i as u64 + 1, &format!("G{i}"), letter)
            })
            .collect();
        let sequence: String = {
            let mut all: Vec<char> = LetterKey::ALL.iter().map(|k| k.letter()).collect();
            let length = rng.below(15);
            let mut text = String::new();
            for _ in 0..length {
                let at = rng.below(all.len());
                text.push(all.swap_remove(at));
            }
            text
        };
        let keys = GroupKeySettings {
            default_sequence: sequence.clone(),
            ..GroupKeySettings::default()
        };
        let mut panel = panel_with(&groups, &keys);
        let changes = panel.reset_keys();
        let mut model = Model::from(&groups);
        model.apply_all(&changes);
        assert_eq!(
            model.groups,
            reset_target(&Model::from(&groups), &sequence),
            "{groups:?} {sequence:?}"
        );
        assert_eq!(
            panel.view().reset_enabled,
            !changes.is_empty(),
            "{groups:?} {sequence:?}"
        );
        // Asked again, the same answer: reset does not remember anything.
        assert_eq!(panel.reset_keys(), changes);
    }
}

#[test]
fn reset_makes_the_same_changes_before_and_after_it_is_asked_for_the_enabled_flag() {
    let mut panel = panel_of(&mockup());
    let enabled = panel.view().reset_enabled;
    let changes = panel.reset_keys();
    assert_eq!(enabled, !changes.is_empty());
    assert!(enabled);
}

// ---- limits: 64 groups, big ids, odd names, repeated keys -----------------------------------

#[test]
fn sixty_four_groups_make_sixty_four_rows() {
    let groups: Vec<GroupInput> = (0..64)
        .map(|i| {
            group(
                i + 1,
                &format!("Group {}", i + 1),
                LetterKey::from_index(i as usize).map(LetterKey::letter),
            )
        })
        .collect();
    let mut panel = panel_of(&groups);
    let view = panel.view();
    assert_eq!(view.rows.len(), 64);
    assert!(view.rows[..26].iter().all(|r| r.letter.len() == 1));
    assert!(view.rows[26..].iter().all(|r| r.letter.is_empty()));
    // A letter typed for the 64th: free ones do not exist, so it conflicts with the holder.
    assert_eq!(panel.key_typed(64, "z"), None);
    let view = panel.view();
    assert_eq!(row(&view, 64).state, RowState::Conflict);
    assert_eq!(row(&view, 64).message, "Z is already Group 26's key.");
    assert_eq!(
        panel.swap(64),
        Some(Change::Swap {
            group: 64,
            other: 26
        })
    );
    // Reset: nine letters for 64 groups.
    let changes = panel.reset_keys();
    let mut model = Model::from(&groups);
    model.apply_all(&changes);
    assert_eq!(
        model.groups,
        reset_target(&Model::from(&groups), "ASDFGHJKL")
    );
    assert_eq!(model.groups.iter().filter(|g| g.1.is_some()).count(), 9);
}

#[test]
fn the_largest_id_works_everywhere() {
    let big = i32::MAX as u64;
    let groups = [
        group(big, "Last", Some('A')),
        group(big - 1, "Before", None),
        group(0, "Zero", Some('S')),
    ];
    let mut panel = panel_of(&groups);
    assert_eq!(
        panel.key_typed(big - 1, "k"),
        Some(Change::SetKey {
            group: big - 1,
            key: Some(LetterKey::K)
        })
    );
    assert_eq!(panel.key_typed(big - 1, "a"), None);
    assert_eq!(
        panel.swap(big - 1),
        Some(Change::Swap {
            group: big - 1,
            other: big
        })
    );
    assert_eq!(panel.key_typed(big, "s"), None);
    assert_eq!(
        panel.swap(big),
        Some(Change::Swap {
            group: big,
            other: 0
        })
    );
    assert_eq!(row(&panel.view(), big).id, big);
    assert_eq!(panel.key_typed(u64::MAX, "q"), None);
}

#[test]
fn names_empty_or_of_sixty_four_bytes_are_handed_on_as_stored() {
    let long = "x".repeat(64);
    let accents = "\u{e9}".repeat(32); // 64 bytes
    let wide = "\u{1f600}".repeat(16); // 64 bytes
    let groups = [
        group(1, "", Some('A')),
        group(2, &long, Some('S')),
        group(3, &accents, Some('D')),
        group(4, &wide, None),
    ];
    let mut panel = panel_of(&groups);
    let view = panel.view();
    assert_eq!(view.rows[0].name, "");
    assert_eq!(view.rows[1].name, long);
    assert_eq!(view.rows[2].name, accents);
    assert_eq!(view.rows[3].name, wide);
    assert_eq!(panel.key_typed(4, "s"), None);
    assert_eq!(
        row(&panel.view(), 4).message,
        format!("S is already {long}'s key.")
    );
    assert_eq!(panel.key_typed(4, "d"), None);
    assert_eq!(
        row(&panel.view(), 4).message,
        format!("D is already {accents}'s key.")
    );
    assert_eq!(panel.key_typed(4, "a"), None);
    assert_eq!(
        row(&panel.view(), 4).message,
        "A is already an unnamed group's key."
    );
}

#[test]
fn two_groups_with_the_same_key_never_show_one_letter_twice() {
    let groups = [
        group(1, "First", Some('S')),
        group(2, "Second", Some('S')),
        group(3, "Third", Some('S')),
        group(4, "Fourth", Some('E')),
        group(5, "Fifth", Some('E')),
    ];
    let mut panel = panel_of(&groups);
    let view = panel.view();
    // Ring order decides, as in `KeyMap::restore`: the earlier group keeps the letter.
    assert_eq!(letters(&view), vec!["S", "", "", "E", ""]);
    assert_eq!(row(&view, 2).types, "");
    assert_eq!(row(&view, 2).state, RowState::Plain);
    assert_eq!(row(&view, 5).state, RowState::Plain);
    assert_eq!(row(&view, 4).state, RowState::DeadKey);
    // Typing the letter into the one that lost it conflicts with the one that kept it.
    assert_eq!(panel.key_typed(2, "s"), None);
    assert_eq!(row(&panel.view(), 2).message, "S is already First's key.");
    assert_eq!(panel.swap(2), Some(Change::Swap { group: 2, other: 1 }));
    // Reset works from what is shown.
    let changes = panel.reset_keys();
    assert!(!changes.is_empty());
}

#[test]
fn a_group_id_given_twice_is_one_row() {
    let groups = [
        group(1, "First", Some('A')),
        group(2, "Second", Some('S')),
        group(1, "Again", Some('D')),
    ];
    let mut panel = panel_of(&groups);
    let view = panel.view();
    assert_eq!(view.rows.len(), 2);
    assert_eq!(row(&view, 1).name, "First");
    assert_eq!(letters(&view), vec!["A", "S"]);
    // The dropped entry's letter is free: it is not held by anyone.
    assert_eq!(
        panel.key_typed(2, "d"),
        Some(Change::SetKey {
            group: 2,
            key: Some(LetterKey::D)
        })
    );
}

#[test]
fn view_is_pure_and_repeatable() {
    let mut panel = panel_of(&mockup());
    assert_eq!(panel.view(), panel.view());
    assert_eq!(panel.key_typed(4, "S"), None);
    assert_eq!(panel.view(), panel.view());
}

// ---- the random walk -------------------------------------------------------------------------------

/// A small seeded generator (SplitMix64), so the walk is the same on every machine.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() % bound as u64) as usize
        }
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

/// What the plain model says is pending: the same rules as the brief, written out again.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pending {
    Conflict {
        group: u64,
        wanted: LetterKey,
        holder: u64,
    },
    Refused {
        group: u64,
    },
}

struct World {
    model: Model,
    pending: Option<Pending>,
    names: Vec<(u64, String)>,
    next_id: u64,
    keys: GroupKeySettings,
    facts: KeyboardFacts,
    loose: usize,
    live: bool,
    panel: KeyPanel,
}

const TYPED: [&str; 16] = [
    "a", "b", "e", "s", "z", "A", "S", "N", "", "", "1", " ", "ab", "\u{e9}", "q", "d",
];

impl World {
    fn new(seed: u64, groups: usize, rng: &mut Rng) -> Self {
        let mut pool = LetterKey::ALL.to_vec();
        let mut model = Model { groups: Vec::new() };
        let mut names = Vec::new();
        for i in 0..groups {
            let id = i as u64 + 1;
            let key = (!rng.chance(3) && !pool.is_empty()).then(|| {
                let at = rng.below(pool.len());
                pool.swap_remove(at)
            });
            model.groups.push((id, key));
            names.push((id, format!("Group {id} of {seed}")));
        }
        Self {
            model,
            pending: None,
            names,
            next_id: groups as u64 + 1,
            keys: GroupKeySettings::default(),
            facts: KeyboardFacts::us(),
            loose: 0,
            live: false,
            panel: KeyPanel::new(),
        }
    }

    fn name(&self, id: u64) -> String {
        self.names
            .iter()
            .find(|n| n.0 == id)
            .map(|n| n.1.clone())
            .unwrap_or_default()
    }

    fn inputs(&self) -> Vec<GroupInput> {
        let names = self.names.clone();
        self.model.inputs(&move |id| {
            names
                .iter()
                .find(|n| n.0 == id)
                .map(|n| n.1.clone())
                .unwrap_or_default()
        })
    }

    /// The model's rule for a refresh: a pending state that is no longer true is dropped.
    fn settle_pending(&mut self) {
        let still_true = match &self.pending {
            None => true,
            Some(Pending::Refused { group }) => self.model.groups.iter().any(|g| g.0 == *group),
            Some(Pending::Conflict {
                group,
                wanted,
                holder,
            }) => {
                self.model.groups.iter().any(|g| g.0 == *group)
                    && self.model.holder_of(*wanted) == Some(*holder)
            }
        };
        if !still_true {
            self.pending = None;
        }
    }

    fn refresh(&mut self) {
        let inputs = self.inputs();
        self.panel
            .refresh(&inputs, self.loose, &self.keys, &self.facts, self.live);
        self.settle_pending();
        self.check();
    }

    /// Every promise of the view, against the plain model.
    fn check(&self) {
        let view = self.panel.view();
        // The rows are the groups of the model in ring order, with its keys.
        assert_eq!(
            view.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            self.model.groups.iter().map(|g| g.0).collect::<Vec<_>>()
        );
        assert_eq!(view.empty, self.model.groups.is_empty());
        for (row, &(id, held)) in view.rows.iter().zip(&self.model.groups) {
            assert_eq!(
                row.letter,
                held.map_or(String::new(), |k| k.letter().to_string()),
                "the letter of group {id}"
            );
            assert_eq!(row.name, self.name(id));
        }
        // Keys stay unique and no letter is shown twice.
        let shown: Vec<&str> = view
            .rows
            .iter()
            .map(|r| r.letter.as_str())
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(shown.len(), shown.iter().collect::<HashSet<_>>().len());
        let held: Vec<LetterKey> = self.model.groups.iter().filter_map(|g| g.1).collect();
        assert_eq!(held.len(), held.iter().collect::<HashSet<_>>().len());

        // At most one row carries a conflict or a refusal, and it is the model's pending one.
        let pending_rows: Vec<&_> = view
            .rows
            .iter()
            .filter(|r| matches!(r.state, RowState::Conflict | RowState::Refused))
            .collect();
        match &self.pending {
            None => assert!(pending_rows.is_empty(), "{view:#?}"),
            Some(Pending::Refused { group }) => {
                assert_eq!(pending_rows.len(), 1);
                assert_eq!(pending_rows[0].id, *group);
                assert_eq!(pending_rows[0].state, RowState::Refused);
                assert_eq!(pending_rows[0].message, REFUSED);
            }
            Some(Pending::Conflict {
                group,
                wanted,
                holder,
            }) => {
                assert_eq!(pending_rows.len(), 1);
                assert_eq!(pending_rows[0].id, *group);
                assert_eq!(pending_rows[0].state, RowState::Conflict);
                assert_eq!(
                    pending_rows[0].message,
                    format!("{wanted} is already {}'s key.", self.name(*holder))
                );
                assert_eq!(self.model.holder_of(*wanted), Some(*holder));
            }
        }
        // The other rows are plain, or amber exactly when their key is dead.
        for row in &view.rows {
            if matches!(row.state, RowState::Conflict | RowState::Refused) {
                continue;
            }
            let dead = row
                .letter
                .chars()
                .next()
                .and_then(LetterKey::from_char)
                .is_some_and(|k| self.facts.dead_keys().accent(k).is_some());
            assert_eq!(
                row.state,
                if dead {
                    RowState::DeadKey
                } else {
                    RowState::Plain
                }
            );
            assert_eq!(row.message.is_empty(), !dead);
        }
        // The settings controls follow the settings.
        assert_eq!(view.keys_enabled, self.keys.enabled);
        assert_eq!(view.keys_off_note.is_empty(), self.keys.enabled);
        assert_eq!(view.note.is_empty(), self.live);
        assert_eq!(view.loose_tabs, self.loose);
        assert_eq!(view.hold_choices[view.hold_index], view.hold_text);
        let choices = hold_choices(self.keys.hold_ms);
        assert_eq!(view.hold_choices.len(), choices.len());
        assert_eq!(choices[view.hold_index], self.keys.hold_ms);
        // Reset is enabled exactly when it would change something.
        let target = reset_target(&self.model, &self.keys.default_sequence);
        assert_eq!(view.reset_enabled, target != self.model.groups);
    }

    fn pick_group(&self, rng: &mut Rng) -> u64 {
        if self.model.groups.is_empty() || rng.chance(12) {
            // Sometimes a group that is not there.
            return self.next_id + 100 + rng.below(5) as u64;
        }
        self.model.groups[rng.below(self.model.groups.len())].0
    }

    fn step(&mut self, rng: &mut Rng) {
        match rng.below(100) {
            // Type into a key cap.
            0..=34 => {
                let group = self.pick_group(rng);
                let text = TYPED[rng.below(TYPED.len())];
                self.type_key(group, text);
            }
            // Swap.
            35..=49 => {
                let group = if let Some(Pending::Conflict { group, .. }) = &self.pending {
                    if rng.chance(4) {
                        self.pick_group(rng)
                    } else {
                        *group
                    }
                } else {
                    self.pick_group(rng)
                };
                self.swap(group);
            }
            // Choose another.
            50..=57 => {
                let group = if let Some(Pending::Conflict { group, .. }) = &self.pending {
                    if rng.chance(4) {
                        self.pick_group(rng)
                    } else {
                        *group
                    }
                } else {
                    self.pick_group(rng)
                };
                self.panel.choose_another(group);
                if matches!(&self.pending, Some(Pending::Conflict { group: g, .. }) if *g == group)
                {
                    self.pending = None;
                }
                self.check();
            }
            // Reset.
            58..=65 => self.reset(),
            // A refresh with nothing changed: a pending state that is still true survives.
            66..=71 => self.refresh(),
            // The world changes under the panel: close, add, reorder, rekey, rename.
            72..=95 => self.external(rng),
            // Settings and facts.
            _ => self.settings(rng),
        }
    }

    fn type_key(&mut self, group: u64, text: &str) {
        let known = self.model.groups.iter().any(|g| g.0 == group);
        let change = self.panel.key_typed(group, text);
        if !known {
            assert_eq!(change, None);
            self.check();
            return;
        }
        self.pending = None;
        let old = self.model.key_of(group);
        let typed = {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (None, _) => Some(None),
                (Some(c), None) => LetterKey::from_char(c).map(Some),
                _ => None,
            }
        };
        match typed {
            None => {
                assert_eq!(change, None);
                self.pending = Some(Pending::Refused { group });
            }
            Some(want) if want == old => assert_eq!(change, None),
            Some(None) => assert_eq!(
                change,
                Some(Change::SetKey { group, key: None }),
                "Backspace on a group with a key"
            ),
            Some(Some(letter)) => match self.model.holder_of(letter) {
                Some(holder) => {
                    assert_eq!(change, None);
                    self.pending = Some(Pending::Conflict {
                        group,
                        wanted: letter,
                        holder,
                    });
                }
                None => assert_eq!(
                    change,
                    Some(Change::SetKey {
                        group,
                        key: Some(letter)
                    })
                ),
            },
        }
        if let Some(change) = change {
            self.model.apply(change);
            // The application refreshes after an edit of its own.
            self.refresh();
        } else {
            self.check();
        }
    }

    fn swap(&mut self, group: u64) {
        let change = self.panel.swap(group);
        match self.pending.clone() {
            Some(Pending::Conflict {
                group: g,
                wanted,
                holder,
            }) if g == group => {
                assert_eq!(
                    change,
                    Some(Change::Swap {
                        group,
                        other: holder
                    })
                );
                let old = self.model.key_of(group);
                let holder_before = self.model.holder_of(wanted);
                assert_eq!(holder_before, Some(holder));
                self.model.apply(Change::Swap {
                    group,
                    other: holder,
                });
                // Exactly the keys the message promised.
                assert_eq!(self.model.key_of(group), Some(wanted));
                assert_eq!(self.model.key_of(holder), old);
                self.pending = None;
                self.refresh();
            }
            _ => {
                assert_eq!(change, None);
                self.check();
            }
        }
    }

    fn reset(&mut self) {
        let before = self.model.clone();
        let changes = self.panel.reset_keys();
        self.pending = None;
        self.model.apply_all(&changes);
        let target = reset_target(&before, &self.keys.default_sequence);
        assert_eq!(self.model.groups, target);
        assert_eq!(changes.is_empty(), before.groups == target);
        self.refresh();
    }

    fn external(&mut self, rng: &mut Rng) {
        match rng.below(7) {
            // Close a group.
            0 if !self.model.groups.is_empty() => {
                let at = rng.below(self.model.groups.len());
                self.model.groups.remove(at);
            }
            // Add a group at the end, with a free letter or none.
            1 if self.model.groups.len() < 64 => {
                let free: Vec<LetterKey> = LetterKey::ALL
                    .into_iter()
                    .filter(|k| self.model.holder_of(*k).is_none())
                    .collect();
                let key = (!free.is_empty() && !rng.chance(3)).then(|| free[rng.below(free.len())]);
                let id = if rng.chance(10) {
                    i32::MAX as u64 - rng.below(3) as u64
                } else {
                    self.next_id
                };
                if self.model.groups.iter().all(|g| g.0 != id) {
                    self.model.groups.push((id, key));
                    self.names.push((id, format!("New {id}")));
                }
                self.next_id += 1;
            }
            // Move a group.
            2 if self.model.groups.len() > 1 => {
                let from = rng.below(self.model.groups.len());
                let entry = self.model.groups.remove(from);
                let to = rng.below(self.model.groups.len() + 1);
                self.model.groups.insert(to, entry);
            }
            // Something else sets a key (to a free letter or to none).
            3 if !self.model.groups.is_empty() => {
                let at = rng.below(self.model.groups.len());
                let id = self.model.groups[at].0;
                let free: Vec<LetterKey> = LetterKey::ALL
                    .into_iter()
                    .filter(|k| self.model.holder_of(*k).is_none())
                    .collect();
                let key = (!free.is_empty() && !rng.chance(3)).then(|| free[rng.below(free.len())]);
                self.model.apply(Change::SetKey { group: id, key });
            }
            // Something else swaps two groups' keys.
            4 if self.model.groups.len() > 1 => {
                let a = self.model.groups[rng.below(self.model.groups.len())].0;
                let b = self.model.groups[rng.below(self.model.groups.len())].0;
                if a != b {
                    self.model.apply(Change::Swap { group: a, other: b });
                }
            }
            // A rename.
            5 if !self.model.groups.is_empty() => {
                let at = rng.below(self.model.groups.len());
                let id = self.model.groups[at].0;
                let name = format!("Renamed {id} #{}", rng.below(1000));
                self.names.retain(|n| n.0 != id);
                self.names.push((id, name));
            }
            // Loose tabs.
            _ => self.loose = rng.below(40),
        }
        self.refresh();
    }

    fn settings(&mut self, rng: &mut Rng) {
        match rng.below(6) {
            0 => {
                let index = rng.below(4);
                let ok = set_capture(&mut self.keys, index);
                assert_eq!(ok, index < 3);
            }
            1 => {
                let index = rng.below(8);
                let before = self.keys.hold_ms;
                let ok = set_hold(&mut self.keys, index);
                assert_eq!(ok, index < hold_choices(before).len());
            }
            2 => self.keys.enabled = !self.keys.enabled,
            3 => self.live = !self.live,
            4 => {
                self.facts = if rng.chance(2) {
                    KeyboardFacts::us()
                } else {
                    KeyboardFacts::us().with_learnt([
                        (LetterKey::E, OptionChar::Char('\u{e9}')),
                        (LetterKey::A, OptionChar::Dead('`')),
                    ])
                };
            }
            _ => {
                let sequences = [
                    "ASDFGHJKL",
                    "",
                    "QWERTY",
                    "ZYXWVUTSRQPONMLKJIHGFEDCBA",
                    "AS",
                    "E",
                ];
                self.keys.default_sequence = sequences[rng.below(sequences.len())].to_owned();
            }
        }
        self.refresh();
    }
}

fn walk(seed: u64, groups: usize, steps: usize) {
    let mut rng = Rng::new(seed);
    let mut world = World::new(seed, groups, &mut rng);
    world.refresh();
    let mut seen_conflict = false;
    let mut seen_swap = false;
    let mut seen_refused = false;
    let mut seen_survive = false;
    for _ in 0..steps {
        let before = world.pending.clone();
        world.step(&mut rng);
        seen_conflict |= matches!(world.pending, Some(Pending::Conflict { .. }));
        seen_refused |= matches!(world.pending, Some(Pending::Refused { .. }));
        seen_survive |= before.is_some() && before == world.pending;
        seen_swap |= before.is_some() && world.pending.is_none();
    }
    // The walk is not a tour of the empty state: it met the interesting cases.
    assert!(seen_conflict, "seed {seed}: no conflict met");
    assert!(seen_refused, "seed {seed}: no refusal met");
    assert!(seen_swap, "seed {seed}: no pending state resolved");
    assert!(
        seen_survive,
        "seed {seed}: no pending state survived a step"
    );
}

#[test]
fn a_seeded_random_walk_of_edits_swaps_refreshes_and_resets_matches_the_model() {
    walk(1, 6, 800);
    walk(2, 12, 800);
    walk(3, 3, 800);
    walk(0x5eed, 26, 800);
}

#[test]
fn a_random_walk_over_sixty_four_groups_matches_the_model() {
    walk(64, 64, 600);
}

#[test]
fn every_swap_of_the_walk_ends_with_unique_keys() {
    // The strict model panics on a taken letter and `check` on a repeated one: run more seeds.
    for seed in 100..120 {
        walk(seed, 8, 520);
    }
}

#[test]
fn the_ids_a_walk_uses_are_all_different() {
    let mut rng = Rng::new(9);
    let world = World::new(9, 20, &mut rng);
    let ids: BTreeSet<u64> = world.model.groups.iter().map(|g| g.0).collect();
    assert_eq!(ids.len(), 20);
}
