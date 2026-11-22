// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! "Switch to tab" rows (stub: the paste-and-tabs task fills it in).

use crate::tab::TabId;

/// How many "Switch to tab" rows the list shows at most.
pub const SWITCH_TO_TAB_ROWS: usize = 3;

/// An open tab as the matcher needs it. The caller passes only the tabs the window may show
/// (a private window passes its private tabs only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTab {
    /// The tab.
    pub id: TabId,
    /// Its title.
    pub title: String,
    /// Its address.
    pub url: String,
    /// Whether it is the tab the user is typing in.
    pub is_active: bool,
}

/// A tab to offer a "Switch to tab" row for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabMatch {
    /// The tab.
    pub tab: TabId,
    /// Whether the typed text is exactly the tab's address.
    pub exact_url: bool,
}

/// The tabs to offer for `typed`, best first, at most `limit`.
pub fn switch_to_tab_matches(_typed: &str, _tabs: &[OpenTab], _limit: usize) -> Vec<TabMatch> {
    Vec::new()
}
