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

/// The settings window's sections by their place in the sidebar (`SettingsWindow.section`). The
/// order is `settings-window.slint`'s `sections` list and its `if root.section == N` blocks; a
/// test (`a11y_windows.rs`) checks that each constant here selects the section of its name, so
/// code that opens a section by number says which one it means and a new section cannot
/// silently move the others.
pub mod section {
    /// General.
    pub const GENERAL: i32 = 0;
    /// Appearance.
    pub const APPEARANCE: i32 = 1;
    /// Tab groups.
    pub const TAB_GROUPS: i32 = 2;
    /// Privacy and security.
    pub const PRIVACY: i32 = 3;
    /// Downloads.
    pub const DOWNLOADS: i32 = 4;
    /// Passwords.
    pub const PASSWORDS: i32 = 5;
    /// Advanced.
    pub const ADVANCED: i32 = 6;

    /// The sidebar's names, indexed by the constants above.
    pub const NAMES: [&str; 7] = [
        "General",
        "Appearance",
        "Tab groups",
        "Privacy and security",
        "Downloads",
        "Passwords",
        "Advanced",
    ];
}
