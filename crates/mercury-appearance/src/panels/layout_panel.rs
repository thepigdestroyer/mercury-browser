// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Appearance section's pure half: what each control shows for the stored
//! [`AppearanceSettings`], and what a switch or a choice does to them (brief decisions D4 to D8).
//!
//! No Slint and no I/O. [`layout_panel`] turns the stored settings into [`LayoutPanel`], whose plain
//! fields the window copies into its controls; [`set_switch`] and [`set_choice`] are what the
//! window's reports do to the settings. Every value they can write is one
//! `Settings::validated` accepts without a repair (`tests/layout_panel.rs` walks them all).

use mercury_settings::{AppearanceSettings, DialCorner, DockStart};

use crate::resolve::Layout;

/// The note of D4: the interface behind the controls is not in yet. Shown while
/// `interface_live` is `false`.
const INTERFACE_NOTE: &str = "These choices are saved, but the tab-group interface they control \
                              arrives in a later version. Until then they change nothing you see.";

/// The note under the Group dock switch while the Mercury layout is chosen (D5).
const MERCURY_DOCK_NOTE: &str =
    "Bottom of the window. × shrinks it to a capsule; it never hides by itself.";

/// The note under the Group dock switch while the Classic layout is chosen (D5).
const CLASSIC_DOCK_NOTE: &str =
    "Optional in Classic: a dock under the tab strip. Off unless you turn it on.";

/// The sizes offered for the memory of Orbit's previews, in megabytes (D7).
const SNAPSHOT_CAPS_MB: [u16; 6] = [32, 64, 128, 256, 512, 1024];

/// The two places the dock can start, in the order the choice lists them.
const DOCK_STARTS: [DockStart; 2] = [DockStart::Open, DockStart::Shrunk];

/// The four corners of the dial, in the order the choice lists them.
const DIAL_CORNERS: [DialCorner; 4] = [
    DialCorner::TopLeft,
    DialCorner::TopRight,
    DialCorner::BottomLeft,
    DialCorner::BottomRight,
];

/// The layout the stored id names; the default for an id this version does not know (settings
/// that went through `Settings::validated` never have one).
fn layout_of(settings: &AppearanceSettings) -> Layout {
    Layout::from_id(&settings.layout).unwrap_or_default()
}

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
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|control| control.name() == name)
    }

    /// The name the Slint side uses for this switch.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dock => "dock",
            Self::Ring => "ring",
            Self::Orbit => "orbit",
            Self::OrbitPinch => "orbit-pinch",
            Self::Keys => "keys",
            Self::Dial => "dial",
        }
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
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.name() == name)
    }

    /// The name the Slint side uses for this choice.
    pub fn name(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::DockStarts => "dock-starts",
            Self::DialCorner => "dial-corner",
            Self::SnapshotCap => "snapshot-cap",
        }
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
pub fn snapshot_cap_choices(current_mb: u16) -> Vec<u16> {
    let mut choices = SNAPSHOT_CAPS_MB.to_vec();
    if !choices.contains(&current_mb) {
        choices.push(current_mb);
        choices.sort_unstable();
    }
    choices
}

/// Applies a switch. `Dock` writes the field of the layout that is chosen (D5) and nothing else
/// changes; a dependent control keeps its value (D6).
pub fn set_switch(settings: &mut AppearanceSettings, control: Control, on: bool) {
    let layout = layout_of(settings);
    let groups = &mut settings.tab_groups;
    match control {
        Control::Dock => match layout {
            Layout::Mercury => groups.dock = on,
            Layout::Classic => groups.dock_in_classic = on,
        },
        Control::Ring => groups.ring = on,
        Control::Orbit => groups.orbit = on,
        Control::OrbitPinch => groups.orbit_pinch = on,
        Control::Keys => groups.keys.enabled = on,
        Control::Dial => groups.dial = on,
    }
}

/// Applies a list choice. Returns `false`, and changes nothing, for an index outside the list.
pub fn set_choice(settings: &mut AppearanceSettings, choice: Choice, index: usize) -> bool {
    match choice {
        Choice::Layout => {
            let Some(layout) = Layout::ALL.get(index) else {
                return false;
            };
            layout.id().clone_into(&mut settings.layout);
        }
        Choice::DockStarts => {
            let Some(&start) = DOCK_STARTS.get(index) else {
                return false;
            };
            settings.tab_groups.dock_starts = start;
        }
        Choice::DialCorner => {
            let Some(&corner) = DIAL_CORNERS.get(index) else {
                return false;
            };
            settings.tab_groups.dial_corner = corner;
        }
        Choice::SnapshotCap => {
            let choices = snapshot_cap_choices(settings.tab_groups.snapshot_cap_mb);
            let Some(&megabytes) = choices.get(index) else {
                return false;
            };
            settings.tab_groups.snapshot_cap_mb = megabytes;
        }
    }
    true
}

/// What the section shows for `settings`. `interface_live` is
/// `mercury_ui::settings_sections::TAB_GROUP_INTERFACE_LIVE` (D4): when it is `false`,
/// [`LayoutPanel::note`] holds the note of D4, otherwise it is empty.
pub fn layout_panel(settings: &AppearanceSettings, interface_live: bool) -> LayoutPanel {
    let layout = layout_of(settings);
    let groups = &settings.tab_groups;
    let (dock, dock_note) = match layout {
        Layout::Mercury => (groups.dock, MERCURY_DOCK_NOTE),
        Layout::Classic => (groups.dock_in_classic, CLASSIC_DOCK_NOTE),
    };
    let caps = snapshot_cap_choices(groups.snapshot_cap_mb);
    LayoutPanel {
        layout_index: Layout::ALL
            .iter()
            .position(|&candidate| candidate == layout)
            .unwrap_or_default(),
        dock,
        dock_note: dock_note.to_owned(),
        dock_starts_index: DOCK_STARTS
            .iter()
            .position(|&start| start == groups.dock_starts)
            .unwrap_or_default(),
        ring: groups.ring,
        orbit: groups.orbit,
        orbit_pinch: groups.orbit_pinch,
        orbit_pinch_enabled: groups.orbit,
        keys: groups.keys.enabled,
        dial: groups.dial,
        dial_corner_index: DIAL_CORNERS
            .iter()
            .position(|&corner| corner == groups.dial_corner)
            .unwrap_or_default(),
        dial_corner_enabled: groups.dial,
        snapshot_cap_index: caps
            .iter()
            .position(|&megabytes| megabytes == groups.snapshot_cap_mb)
            .unwrap_or_default(),
        snapshot_cap_choices: caps
            .iter()
            .map(|megabytes| format!("{megabytes} MB"))
            .collect(),
        note: if interface_live {
            String::new()
        } else {
            INTERFACE_NOTE.to_owned()
        },
    }
}
