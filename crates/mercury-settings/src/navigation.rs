// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Navigation settings: how a ⌘-click opens a link and whether quitting with several tabs asks.

use serde::{Deserialize, Serialize};

/// Navigation settings.
///
/// A file written before these existed has no `navigation` object and gets the defaults; keys
/// inside it that are missing get their defaults, and keys this version does not know are
/// ignored. Nothing here needs validating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavigationSettings {
    /// Whether a ⌘-click (Ctrl-click off macOS) or middle-click on a link opens it in a new tab.
    /// On by default.
    pub command_click_opens_tab: bool,
    /// Whether a tab opened that way becomes the active tab: with this on, ⌘-click opens in
    /// front and ⇧⌘-click in the background. Off by default.
    pub new_tab_becomes_active: bool,
    /// Whether quitting with several tabs open asks first. On by default.
    pub warn_before_quitting: bool,
}

impl Default for NavigationSettings {
    fn default() -> Self {
        Self {
            command_click_opens_tab: true,
            new_tab_becomes_active: false,
            warn_before_quitting: true,
        }
    }
}
