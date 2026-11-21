// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `AppearanceUi` and the browser: `fill` shows a [`LayoutPanel`], `connect` reports what the
//! user did as an [`AppearanceAction`].
//!
//! Only copies values, in both directions: what each control shows is decided by
//! `mercury_appearance::panels::layout_panel`, and what a report does to the settings is decided
//! there too. The one thing this file knows is that the window's indices are `int` and the
//! panel's are `usize`.

use std::rc::Rc;

use mercury_appearance::panels::layout_panel::{AppearanceAction, Choice, Control, LayoutPanel};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{AppearanceUi, SettingsWindow};

/// A list index as the window's `int`. The lists are a handful of items long; a value that
/// does not fit (which cannot happen) selects the first item.
fn to_int(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(0)
}

/// Puts every value of `panel` into the window's `AppearanceUi` global.
pub fn fill(window: &SettingsWindow, panel: &LayoutPanel) {
    let ui = window.global::<AppearanceUi>();
    ui.set_layout_index(to_int(panel.layout_index));
    ui.set_dock(panel.dock);
    ui.set_dock_note(SharedString::from(panel.dock_note.as_str()));
    ui.set_dock_starts_index(to_int(panel.dock_starts_index));
    ui.set_ring(panel.ring);
    ui.set_orbit(panel.orbit);
    ui.set_orbit_pinch(panel.orbit_pinch);
    ui.set_orbit_pinch_enabled(panel.orbit_pinch_enabled);
    ui.set_keys(panel.keys);
    ui.set_dial(panel.dial);
    ui.set_dial_corner_index(to_int(panel.dial_corner_index));
    ui.set_dial_corner_enabled(panel.dial_corner_enabled);
    ui.set_snapshot_cap_choices(ModelRc::new(VecModel::from(
        panel
            .snapshot_cap_choices
            .iter()
            .map(|text| SharedString::from(text.as_str()))
            .collect::<Vec<_>>(),
    )));
    ui.set_snapshot_cap_index(to_int(panel.snapshot_cap_index));
    ui.set_note(SharedString::from(panel.note.as_str()));
}

/// Calls `on_action` for every switch and choice the user changes in the Appearance section. Call
/// it once per window.
///
/// A name this version does not know, or a negative index, is dropped: nothing in the window
/// sends one.
pub fn connect(window: &SettingsWindow, on_action: impl Fn(AppearanceAction) + 'static) {
    let on_action = Rc::new(on_action);
    let ui = window.global::<AppearanceUi>();
    ui.on_toggled({
        let on_action = Rc::clone(&on_action);
        move |name, on| {
            if let Some(control) = Control::from_name(&name) {
                on_action(AppearanceAction::Switch(control, on));
            }
        }
    });
    ui.on_chosen(move |name, index| {
        if let (Some(choice), Ok(index)) = (Choice::from_name(&name), usize::try_from(index)) {
            on_action(AppearanceAction::Choice(choice, index));
        }
    });
}
