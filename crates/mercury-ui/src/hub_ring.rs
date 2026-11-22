// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Everything behind the hub ring popover except the ring itself (SPEC
//! `design/ui-final/SPEC.md` §4.3 and §5; lane brief `.superpowers/sdd/lane-hub-ring-brief.md`):
//! the search filter, the centre disc's state, the key navigation, the pinned beads, the hub
//! button's donut and the [`HubRing`] controller that fills the `HubRingBridge` global
//! (`ui/groups/hub-ring-bridge.slint`) and calls the browser through [`HubRingHost`].
//!
//! Pure logic first: [`search`], [`centre`], [`nav`], [`beads`] and [`hub_button`] use no window
//! and are unit-tested alone. The ring's own data comes from [`crate::ring_bands`].
//!
//! The public items of this file and of the submodules are the contract the desktop adapter and
//! the layout-switch lane build on; only additions are allowed, through the lane lead.
//!
//! # How a host uses it
//!
//! 1. Put `RingPopover` and `HubButton` (`ui/groups/hub-ring.slint`) in a window; nothing is
//!    forwarded, they read the `HubRingBridge` global.
//! 2. `HubRing::install(&window, host, ring_bands::metrics())` once.
//! 3. `ring.set_input(input)` on every change of tabs, groups, theme or window size, and
//!    `ring.opened()` when the popover opens. The popover's `opened()` and `closed()` bridge
//!    callbacks fire from `RingPopover` itself; a host that shows the content without a popup
//!    must call `closed` and then `opened` after a close request, or the ring ignores keys.
//! 4. The window-level ⌥ hook (which sees the physical key; ⌥S arrives as "ß" in the popover)
//!    calls [`HubRing::option_key`] with the upper-case letter or digit while the ring is open.

use slint::Image;

use crate::{GroupRow, TabRow};

pub mod beads;
pub mod centre;
mod controller;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
pub mod hub_button;
pub mod nav;
pub mod search;

pub use controller::HubRing;

/// The id of the pseudo group of tabs that are in no group (SPEC §3.1 "Loose"). Negative, so it
/// can never be a real `GroupId` (a `u64` up to `i32::MAX`) and is not the ring's "none" (-1).
pub const LOOSE_GROUP_ID: i32 = -2;

/// The sound state of one tab (only tabs that play sound or are muted have an entry).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabMedia {
    /// A `TabRow.id`.
    pub tab: i32,
    pub playing: bool,
    pub paused: bool,
    /// The media session has a pause command for this tab (D13: no Pause button without it).
    pub can_pause: bool,
    pub muted: bool,
    /// (position, duration) in seconds, when the media session knows them.
    pub position: Option<(u32, u32)>,
}

/// Everything the controller needs, built by the host (the desktop crate) or by `fixtures`.
#[derive(Clone, Debug, Default)]
pub struct HubRingInput {
    /// Every group that has a tab, in the window's order; loose tabs belong to a group row with
    /// id [`LOOSE_GROUP_ID`].
    pub groups: Vec<GroupRow>,
    /// EVERY tab of the window, pinned ones included. `matches_query` is overwritten by the
    /// controller.
    pub tabs: Vec<TabRow>,
    pub media: Vec<TabMedia>,
    /// Snapshots by tab id; a tab without one gets the placeholder card.
    pub previews: Vec<(i32, Image)>,
    /// (group id, tab id): the tab that a group's key switches to (its last active tab). A group
    /// without an entry uses its first tab.
    pub last_active: Vec<(i32, i32)>,
    pub dark: bool,
    /// The colours of the ring's key letters and counts. `None` reads the window's live
    /// `ChromePalette` (`RingColours::from_palette`) every time the input is set, so the ring
    /// follows the theme layer; a host sets `Some` only to override it.
    pub colours: Option<crate::ring_bands::RingColours>,
    /// Orbit exists: "See everything" is drawn (D9).
    pub orbit_available: bool,
    pub reduce_motion: bool,
    /// How long ⌥ must be down before the key look starts (settings `hold_ms`, 400).
    pub hold_ms: u32,
}

/// What the browser does for the ring; the desktop implements it, the tests record the calls.
/// Every method is called on the UI thread and must not block.
pub trait HubRingHost {
    /// Show the tab (the popover closes itself afterwards).
    fn activate_tab(&self, tab: i32);
    /// Close the tab (the existing close path, including the rule for pinned tabs).
    fn close_tab(&self, tab: i32);
    fn toggle_mute(&self, tab: i32);
    fn toggle_pause(&self, tab: i32);
    /// Move `tab` into `group` (a `GroupRow.id`, possibly [`LOOSE_GROUP_ID`]) at `index` (the index
    /// in that group's shown tabs, as `RingDropTarget.index`).
    fn move_tab(&self, tab: i32, group: i32, index: usize);
    /// "See everything": open Orbit.
    fn open_orbit(&self);
    /// "Search the web for ...": a new tab with the default search engine.
    fn search_web(&self, query: &str);
}
