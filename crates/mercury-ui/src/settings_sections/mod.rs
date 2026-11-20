// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Slint side of the settings window's Appearance and Tab groups sections: the bridges that put
//! a `mercury_appearance::panels` view into the section's global (`fill`) and turn what the user
//! did into an action (`connect`). The logic is in `mercury-appearance`; this crate only copies
//! values (lane `settings-ui`, brief section 7).

pub mod appearance;
pub mod tab_groups;

/// Whether the tab-group interface (the dock, the ring, Orbit, the corner dial, the ⌥ group keys
/// and the two layouts) is in the browser. While it is `false` both sections show the note of
/// brief decision D4: the choices are saved but change nothing you can see.
///
/// Lane `layout-switch` (lane 14) sets it to `true` when the interface is in.
pub const TAB_GROUP_INTERFACE_LIVE: bool = false;
