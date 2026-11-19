// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The stored shape of the layout, palette and tab-group settings (`appearance.layout`,
//! `appearance.palette_light`, `appearance.palette_dark`, `appearance.tab_groups`; SPEC §3.4).
//!
//! Two kinds of value, on purpose:
//!
//! - **Open-ended values are validated strings**, not enums: the layout and the palette ids. A
//!   file written by a later Mercury that knows more layouts or palettes must still load in this
//!   one, so an unknown value becomes the default with a [`Repair`](crate::Repair) instead of
//!   making the whole file "damaged".
//! - **Closed sets are enums** ([`DockStart`], [`DialCorner`], [`KeyCapture`]). Adding a variant
//!   later is a change an older reader would misread, so it needs a
//!   [`SETTINGS_FORMAT_VERSION`](crate::SETTINGS_FORMAT_VERSION) bump (see `store.rs`).
//!
//! Which palettes exist is not known here: `mercury-appearance` owns that, and checks the ids
//! when it resolves them. This module checks only that an id is well formed.

use serde::{Deserialize, Serialize};

/// The id of the Mercury layout: the toolbar on top, the group dock at the bottom.
pub const LAYOUT_MERCURY: &str = "mercury";
/// The id of the Classic layout: tabs in the title-bar strip; the group dock is optional.
pub const LAYOUT_CLASSIC: &str = "classic";
/// Every layout id this version knows.
pub const LAYOUT_IDS: [&str; 2] = [LAYOUT_MERCURY, LAYOUT_CLASSIC];
/// The layout of a profile that does not name one.
pub const DEFAULT_LAYOUT: &str = LAYOUT_MERCURY;
/// The palette (for both colour schemes) of a profile that does not name one.
pub const DEFAULT_PALETTE: &str = "mercury-b2";
/// The longest palette id, in bytes.
pub const MAX_PALETTE_ID_BYTES: usize = 64;

/// The shortest hold of ⌥ before the group-key card appears, in milliseconds.
pub const MIN_KEY_HOLD_MS: u16 = 250;
/// The longest hold of ⌥ before the group-key card appears, in milliseconds.
pub const MAX_KEY_HOLD_MS: u16 = 1000;
/// The hold of ⌥ before the group-key card appears, when the user has not chosen one.
pub const DEFAULT_KEY_HOLD_MS: u16 = 400;

/// The smallest cap on the memory the tab previews of Orbit may use, in megabytes.
pub const MIN_SNAPSHOT_CAP_MB: u16 = 32;
/// The largest cap on the memory the tab previews of Orbit may use, in megabytes.
pub const MAX_SNAPSHOT_CAP_MB: u16 = 1024;
/// The cap on the memory the tab previews of Orbit may use, when the user has not chosen one.
pub const DEFAULT_SNAPSHOT_CAP_MB: u16 = 128;

/// The letters given to groups in creation order, when the user has not chosen others: the home
/// row, `A S D F G H J K L`.
pub const DEFAULT_KEY_SEQUENCE: &str = "ASDFGHJKL";
/// The most letters a key sequence can hold: one for each of A to Z.
pub const MAX_KEY_SEQUENCE_LEN: usize = 26;

/// Whether the group dock starts open or shrunk into its capsule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockStart {
    /// The full dock.
    #[default]
    Open,
    /// The small capsule.
    Shrunk,
}

/// The corner the corner dial parks in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialCorner {
    /// The top left.
    TopLeft,
    /// The top right.
    TopRight,
    /// The bottom left.
    BottomLeft,
    /// The bottom right.
    #[default]
    BottomRight,
}

/// When a held ⌥ and a letter is taken as a group key instead of typing a character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyCapture {
    /// Outside text fields at once; inside them only after the hold, through the key card.
    #[default]
    OutsideTextOrHold,
    /// Outside text fields only; inside them ⌥ always types.
    OutsideText,
    /// Everywhere. ⌥ characters (å, ß, ∂) cannot then be typed in Mercury.
    Always,
}

/// The ⌥ group keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupKeySettings {
    /// Whether ⌥ group keys work at all.
    pub enabled: bool,
    /// When ⌥ and a letter is captured.
    pub capture: KeyCapture,
    /// How long ⌥ is held before the key card appears
    /// ([`MIN_KEY_HOLD_MS`]..=[`MAX_KEY_HOLD_MS`]).
    pub hold_ms: u16,
    /// Whether each group's key is shown beside its name.
    pub show_beside_names: bool,
    /// The letters given to new groups, in creation order: ASCII capitals, none twice, at most
    /// [`MAX_KEY_SEQUENCE_LEN`]. Empty means new groups get no key.
    pub default_sequence: String,
}

impl Default for GroupKeySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            capture: KeyCapture::default(),
            hold_ms: DEFAULT_KEY_HOLD_MS,
            show_beside_names: true,
            default_sequence: DEFAULT_KEY_SEQUENCE.to_owned(),
        }
    }
}

/// The tab-group controls: the dock, the ring, Orbit, the corner dial and the ⌥ keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabGroupSettings {
    /// The group dock in the Mercury layout.
    pub dock: bool,
    /// The group dock in the Classic layout, where it is optional and off by default.
    pub dock_in_classic: bool,
    /// Whether the dock starts open or shrunk; a window may differ after the user shrinks it.
    pub dock_starts: DockStart,
    /// The hub ring under the toolbar.
    pub ring: bool,
    /// The Orbit overview (⌘E and "See everything").
    pub orbit: bool,
    /// Whether pinching in on a page at 100% zoom opens Orbit.
    pub orbit_pinch: bool,
    /// The corner dial.
    pub dial: bool,
    /// The corner the dial parks in; a window may differ after the user drags it.
    pub dial_corner: DialCorner,
    /// The cap on the memory used by Orbit's tab previews, in megabytes
    /// ([`MIN_SNAPSHOT_CAP_MB`]..=[`MAX_SNAPSHOT_CAP_MB`]).
    pub snapshot_cap_mb: u16,
    /// The ⌥ group keys.
    pub keys: GroupKeySettings,
}

impl Default for TabGroupSettings {
    fn default() -> Self {
        Self {
            dock: true,
            dock_in_classic: false,
            dock_starts: DockStart::default(),
            ring: true,
            orbit: true,
            orbit_pinch: true,
            dial: true,
            dial_corner: DialCorner::default(),
            snapshot_cap_mb: DEFAULT_SNAPSHOT_CAP_MB,
            keys: GroupKeySettings::default(),
        }
    }
}
