// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `AppearanceUi` and the browser: `fill` shows a [`LayoutPanel`], `connect` reports what the
//! user did as an [`AppearanceAction`].
//!
//! Skeleton by the lane lead: signatures frozen; item 1 `appearance-section` writes the bodies.

use mercury_appearance::panels::layout_panel::{AppearanceAction, LayoutPanel};

use crate::SettingsWindow;

/// Puts every value of `panel` into the window's `AppearanceUi` global.
pub fn fill(_window: &SettingsWindow, _panel: &LayoutPanel) {
    todo!("appearance-section")
}

/// Calls `on_action` for every switch and choice the user changes in the Appearance section. Call
/// it once per window.
pub fn connect(_window: &SettingsWindow, _on_action: impl Fn(AppearanceAction) + 'static) {
    todo!("appearance-section")
}
