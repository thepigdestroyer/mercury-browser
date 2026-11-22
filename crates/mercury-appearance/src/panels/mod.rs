// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the settings window's Appearance and Tab groups sections show and do, as plain data.
//!
//! No Slint and no I/O: `mercury-ui` turns these values into Slint properties and reports what
//! the user did as the actions defined here; the desktop applies them. Keeping the logic here,
//! in a crate that builds in seconds, is the point (lane `settings-ui`,
//! `.superpowers/sdd/lane-settings-ui-brief.md`).
//!
//! - [`layout_panel`]: the layout cards and the tab-group switches of Appearance.
//! - [`key_panel`]: the table of the window's groups with their one-letter keys, the conflict
//!   and dead-key notes, and the capture mode.

pub mod key_panel;
pub mod layout_panel;

/// The note of D4: the tab-group interface behind the Appearance and Tab groups controls is not
/// in yet. Both sections show these words while `interface_live` is `false`, so they live here,
/// once.
pub const INTERFACE_NOTE: &str = "These choices are saved, but the tab-group interface they control \
                                  arrives in a later version. Until then they change nothing you see.";
