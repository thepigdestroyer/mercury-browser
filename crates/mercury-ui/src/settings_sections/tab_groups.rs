// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `TabGroupsUi` and the browser: `fill` shows a [`PanelView`], `connect` reports what the user did
//! as a [`PanelAction`].
//!
//! Only copies values, in both directions: what each row says is decided by
//! `mercury_appearance::panels::key_panel`. The two things this file knows are that Slint's ids and
//! indices are `int` (a row whose id does not fit is skipped, never wrapped) and which key
//! presses are one typed character.

use std::rc::Rc;

use mercury_appearance::panels::key_panel::{KeyRow, PanelAction, PanelView, RowState};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{KeyRowUi, SettingsWindow, TabGroupsUi};

/// A count or index as the window's `int`; saturates, which cannot happen for the lists shown.
fn to_int(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The row as the window draws it, or `None` when its id does not fit Slint's `int`.
fn row_ui(row: &KeyRow) -> Option<KeyRowUi> {
    let id = i32::try_from(row.id).ok()?;
    let colour = slint::Color::from_argb_u8(row.colour.a, row.colour.r, row.colour.g, row.colour.b);
    Some(KeyRowUi {
        id,
        name: SharedString::from(row.name.as_str()),
        colour,
        tabs: SharedString::from(row.tabs_text.as_str()),
        letter: SharedString::from(row.letter.as_str()),
        types: SharedString::from(row.types.as_str()),
        state: match row.state {
            RowState::Plain => 0,
            RowState::DeadKey => 1,
            RowState::Conflict => 2,
            RowState::Refused => 3,
        },
        message: SharedString::from(row.message.as_str()),
    })
}

/// Puts every value of `view` into the window's `TabGroupsUi` global.
pub fn fill(window: &SettingsWindow, view: &PanelView) {
    let ui = window.global::<TabGroupsUi>();
    ui.set_rows(ModelRc::new(VecModel::from(
        view.rows.iter().filter_map(row_ui).collect::<Vec<_>>(),
    )));
    ui.set_loose_tabs(to_int(view.loose_tabs));
    ui.set_empty(view.empty);
    ui.set_keys_enabled(view.keys_enabled);
    ui.set_keys_off_note(SharedString::from(view.keys_off_note.as_str()));
    ui.set_types_header(SharedString::from(view.types_header.as_str()));
    ui.set_show_beside_names(view.show_beside_names);
    ui.set_capture_index(to_int(view.capture_index));
    ui.set_hold_text(SharedString::from(view.hold_text.as_str()));
    ui.set_hold_choices(ModelRc::new(VecModel::from(
        view.hold_choices
            .iter()
            .map(|text| SharedString::from(text.as_str()))
            .collect::<Vec<_>>(),
    )));
    ui.set_hold_index(to_int(view.hold_index));
    ui.set_reset_label(SharedString::from(view.reset_label.as_str()));
    ui.set_reset_enabled(view.reset_enabled);
    ui.set_note(SharedString::from(view.note.as_str()));
}

/// Whether a key press is one typed character for a key cap: a single character that is not a
/// control character or one of Slint's private-use key codes (Tab, Return, Escape, arrows,
/// modifier keys, function keys). A space, a digit or a symbol is typed; the panel refuses it.
pub fn is_typed_character(text: &str) -> bool {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(only), None) => !only.is_control() && !is_private_use(only),
        _ => false,
    }
}

/// The Unicode private-use areas, where Slint's `Key` constants for arrows and function keys live.
fn is_private_use(character: char) -> bool {
    matches!(character, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{10FFFF}')
}

/// Calls `on_action` for everything the user does in the Tab groups section, including
/// [`PanelAction::Opened`] when the section appears. Call it once per window.
///
/// A name this version does not know, or a negative id or index, is dropped: nothing in the
/// window sends one.
pub fn connect(window: &SettingsWindow, on_action: impl Fn(PanelAction) + 'static) {
    let on_action = Rc::new(on_action);
    let ui = window.global::<TabGroupsUi>();
    ui.on_is_typed_character(|text| is_typed_character(&text));
    ui.on_opened({
        let on_action = Rc::clone(&on_action);
        move || on_action(PanelAction::Opened)
    });
    ui.on_key_typed({
        let on_action = Rc::clone(&on_action);
        move |id, text| {
            if let Ok(group) = u64::try_from(id) {
                on_action(PanelAction::KeyTyped {
                    group,
                    text: text.to_string(),
                });
            }
        }
    });
    ui.on_swap({
        let on_action = Rc::clone(&on_action);
        move |id| {
            if let Ok(group) = u64::try_from(id) {
                on_action(PanelAction::Swap { group });
            }
        }
    });
    ui.on_choose_another({
        let on_action = Rc::clone(&on_action);
        move |id| {
            if let Ok(group) = u64::try_from(id) {
                on_action(PanelAction::ChooseAnother { group });
            }
        }
    });
    ui.on_reset_keys({
        let on_action = Rc::clone(&on_action);
        move || on_action(PanelAction::Reset)
    });
    ui.on_toggled({
        let on_action = Rc::clone(&on_action);
        move |name, on| {
            if name == "show-beside-names" {
                on_action(PanelAction::ShowBesideNames(on));
            }
        }
    });
    ui.on_chosen(move |name, index| {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        match name.as_str() {
            "capture" => on_action(PanelAction::Capture(index)),
            "hold" => on_action(PanelAction::Hold(index)),
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_printable_character_is_typed() {
        for text in ["a", "A", "7", " ", "-", "é", "å"] {
            assert!(is_typed_character(text), "{text:?}");
        }
    }

    #[test]
    fn control_keys_and_slints_special_keys_are_not_typed() {
        for text in [
            "", "\t", "\n", "\r", "\u{1b}", "\u{8}", "\u{7f}", "\u{10}", "\u{11}", "\u{f700}",
            "\u{f701}", "\u{f704}", "ab", "a ",
        ] {
            assert!(!is_typed_character(text), "{text:?}");
        }
    }

    #[test]
    fn a_row_whose_id_does_not_fit_is_skipped() {
        use mercury_appearance::Colour;
        let row = |id| KeyRow {
            id,
            name: "x".into(),
            colour: Colour::rgb(1, 2, 3),
            tabs_text: "1 tab".into(),
            letter: String::new(),
            types: String::new(),
            state: RowState::Plain,
            message: String::new(),
        };
        assert!(row_ui(&row(u64::from(i32::MAX.unsigned_abs()))).is_some());
        assert!(row_ui(&row(u64::from(i32::MAX.unsigned_abs()) + 1)).is_none());
        assert_eq!(
            row_ui(&row(5)).map(|ui| ui.colour),
            Some(slint::Color::from_rgb_u8(1, 2, 3))
        );
    }
}
