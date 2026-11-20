// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Appearance section's pure half: what each control shows for the stored
//! [`AppearanceSettings`], and what a switch or a choice does to them (brief decisions D4 to D8).
//!
//! Skeleton by the lane lead: the types and signatures are frozen; item 1 `appearance-section`
//! writes the bodies (every `todo!` below) and the tests in `tests/layout_panel.rs`.

use mercury_settings::AppearanceSettings;

/// A switch of the Appearance section. [`name`](Self::name) is the string the Slint side sends in
/// `AppearanceUi.toggled`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    /// "Group dock": `dock` under the Mercury layout, `dock_in_classic` under Classic (D5).
    Dock,
    /// "Ring from the toolbar hub": `ring`.
    Ring,
    /// "Orbit overview": `orbit`.
    Orbit,
    /// "Pinch to open Orbit": `orbit_pinch`.
    OrbitPinch,
    /// "Group keys (⌥ + letter)": `keys.enabled`.
    Keys,
    /// "Corner dial": `dial`.
    Dial,
}

impl Control {
    /// Every switch, in the order the section draws them.
    pub const ALL: [Self; 6] = [
        Self::Dock,
        Self::Ring,
        Self::Orbit,
        Self::OrbitPinch,
        Self::Keys,
        Self::Dial,
    ];

    /// The switch the Slint side named, or `None` for a name this version does not know:
    /// "dock", "ring", "orbit", "orbit-pinch", "keys", "dial".
    pub fn from_name(_name: &str) -> Option<Self> {
        todo!("appearance-section")
    }

    /// The name the Slint side uses for this switch.
    pub fn name(self) -> &'static str {
        todo!("appearance-section")
    }
}

/// A list choice of the Appearance section. [`name`](Self::name) is the string the Slint side
/// sends in `AppearanceUi.chosen`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Choice {
    /// The layout card: index 0 Mercury, 1 Classic.
    Layout,
    /// Where the dock starts: 0 open, 1 shrunk.
    DockStarts,
    /// The corner of the dial: 0 top left, 1 top right, 2 bottom left, 3 bottom right.
    DialCorner,
    /// The memory for Orbit previews: an index into [`snapshot_cap_choices`].
    SnapshotCap,
}

impl Choice {
    /// Every choice, in the order the section draws them.
    pub const ALL: [Self; 4] = [
        Self::Layout,
        Self::DockStarts,
        Self::DialCorner,
        Self::SnapshotCap,
    ];

    /// The choice the Slint side named, or `None`: "layout", "dock-starts", "dial-corner",
    /// "snapshot-cap".
    pub fn from_name(_name: &str) -> Option<Self> {
        todo!("appearance-section")
    }

    /// The name the Slint side uses for this choice.
    pub fn name(self) -> &'static str {
        todo!("appearance-section")
    }
}

/// What the user did in the Appearance section, as `mercury-ui` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppearanceAction {
    /// A switch was turned on or off.
    Switch(Control, bool),
    /// An item of a list was chosen.
    Choice(Choice, usize),
}

/// What every control of the section shows. Plain fields, so the Slint side only copies them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutPanel {
    /// The chosen layout card: 0 Mercury, 1 Classic.
    pub layout_index: usize,
    /// The Group dock switch, for the chosen layout (D5).
    pub dock: bool,
    /// The note under the Group dock switch, which names the layout (D5).
    pub dock_note: String,
    /// Where the dock starts: 0 open, 1 shrunk.
    pub dock_starts_index: usize,
    /// "Ring from the toolbar hub".
    pub ring: bool,
    /// "Orbit overview".
    pub orbit: bool,
    /// "Pinch to open Orbit": the stored value, even while Orbit is off (D6).
    pub orbit_pinch: bool,
    /// Whether the pinch switch can be used: only while Orbit is on (D6).
    pub orbit_pinch_enabled: bool,
    /// "Group keys (⌥ + letter)".
    pub keys: bool,
    /// "Corner dial".
    pub dial: bool,
    /// The dial's corner: 0 top left, 1 top right, 2 bottom left, 3 bottom right.
    pub dial_corner_index: usize,
    /// Whether the corner can be chosen: only while the dial is on (D6).
    pub dial_corner_enabled: bool,
    /// The memory choices as text ("32 MB" to "1024 MB", D7).
    pub snapshot_cap_choices: Vec<String>,
    /// The chosen item of `snapshot_cap_choices`.
    pub snapshot_cap_index: usize,
    /// The one note of D4: empty when the interface is live.
    pub note: String,
}

/// The memory choices in megabytes: 32, 64, 128, 256, 512 and 1024, plus `current_mb` when it is
/// another value (a hand-edited file), in order (D7).
pub fn snapshot_cap_choices(_current_mb: u16) -> Vec<u16> {
    todo!("appearance-section")
}

/// Applies a switch. `Dock` writes the field of the layout that is chosen (D5) and nothing else
/// changes; a dependent control keeps its value (D6).
pub fn set_switch(_settings: &mut AppearanceSettings, _control: Control, _on: bool) {
    todo!("appearance-section")
}

/// Applies a list choice. Returns `false`, and changes nothing, for an index outside the list.
pub fn set_choice(_settings: &mut AppearanceSettings, _choice: Choice, _index: usize) -> bool {
    todo!("appearance-section")
}

/// What the section shows for `settings`. `interface_live` is
/// `mercury_ui::settings_sections::TAB_GROUP_INTERFACE_LIVE` (D4): when it is `false`,
/// [`LayoutPanel::note`] holds the note of D4, otherwise it is empty.
pub fn layout_panel(_settings: &AppearanceSettings, _interface_live: bool) -> LayoutPanel {
    todo!("appearance-section")
}
