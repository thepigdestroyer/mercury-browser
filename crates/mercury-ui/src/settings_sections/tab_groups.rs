// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `TabGroupsUi` and the browser: `fill` shows a [`PanelView`], `connect` reports what the user did
//! as a [`PanelAction`].
//!
//! Skeleton by the lane lead: signatures frozen; item 3 `tab-groups-section` writes the bodies. A
//! row whose id does not fit Slint's `int` is skipped, never wrapped.

use mercury_appearance::panels::key_panel::{PanelAction, PanelView};

use crate::SettingsWindow;

/// Puts every value of `view` into the window's `TabGroupsUi` global.
pub fn fill(_window: &SettingsWindow, _view: &PanelView) {
    todo!("tab-groups-section")
}

/// Calls `on_action` for everything the user does in the Tab groups section, including
/// [`PanelAction::Opened`] when the section appears. Call it once per window.
pub fn connect(_window: &SettingsWindow, _on_action: impl Fn(PanelAction) + 'static) {
    todo!("tab-groups-section")
}
