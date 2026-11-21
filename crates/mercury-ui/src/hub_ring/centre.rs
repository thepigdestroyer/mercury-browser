// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the centre disc shows (SPEC §4.3 table, the four `ring-*` mockups): a pure function from
//! the ring's state to the strings and the mode of the `HubRingBridge`. No window.
//!
//! STUB in the scaffold commit (types and signature are the contract; item 1 `search` writes the
//! body).

use super::TabMedia;
use crate::CentreMode;

/// The tab the centre is about (the hovered one, else the focused one, else the active one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentreFocus {
    pub tab: i32,
    pub title: String,
    pub host: String,
    pub group_name: String,
    /// "D" or empty.
    pub key_label: String,
    /// 1-based among the shown tabs of its group; 0 for a pinned tab.
    pub index_in_group: usize,
    pub pinned: bool,
    pub media: Option<TabMedia>,
}

/// The group picked while ⌥ is held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldGroup {
    pub name: String,
    pub key_label: String,
    /// Shown tabs of the group; the help says "press 1–N" with N = min(9, this).
    pub tab_count: usize,
}

/// The inputs of [`view`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CentreInput {
    /// The search query has terms.
    pub query_active: bool,
    /// ⌥ is held (the key look).
    pub held: bool,
    /// The group chosen by its letter while held.
    pub held_group: Option<HeldGroup>,
    /// Every tab of the window, pinned included ("Search 25 tabs").
    pub total_tabs: usize,
    pub focus: Option<CentreFocus>,
    /// Matches beyond the two result rows.
    pub more_matches: usize,
    /// Pinned tabs that have no bead on the ring (reserve 0): the footer names them.
    pub pinned_collapsed: usize,
}

/// The centre's lines. `mode` is `Held` while held, else `Searching` while a query is active, else
/// `Audio` when the focused tab plays sound or is muted, else `Rest`.
#[derive(Clone, Debug, PartialEq)]
pub struct CentreView {
    pub mode: CentreMode,
    /// "Essay on maps"; empty when there is no tab.
    pub title: String,
    /// "Reading · ⌥D 2 · essays.example.net"; a loose tab "Loose · host"; a pinned tab "Pinned · host".
    pub subtitle: String,
    /// "Playing · 12:40 of 48:00", "Playing", "Paused" or "Muted" (audio mode).
    pub play_state: String,
    pub can_pause: bool,
    pub paused: bool,
    pub muted: bool,
    /// "Search 22 tabs" ("Search 1 tab").
    pub field_placeholder: String,
    /// "⌥ S" (held mode; "⌥" alone while no group is chosen).
    pub held_key: String,
    pub held_name: String,
    /// "press 1–4 for a tab" (the third line, "release ⌥ to switch", is drawn by Slint).
    pub held_help: String,
    /// "Hold ⌥ to see group keys", "↑ ↓ choose · esc clears", "+3 more lit in the ring",
    /// "2 pinned tabs".
    pub footer: String,
}

/// The centre's content for `input`.
pub fn view(_input: &CentreInput) -> CentreView {
    CentreView {
        mode: CentreMode::Rest,
        title: String::new(),
        subtitle: String::new(),
        play_state: String::new(),
        can_pause: false,
        paused: false,
        muted: false,
        field_placeholder: String::new(),
        held_key: String::new(),
        held_name: String::new(),
        held_help: String::new(),
        footer: String::new(),
    }
}
