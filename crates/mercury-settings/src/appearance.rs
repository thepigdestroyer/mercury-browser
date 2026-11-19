// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The stored shape of the layout, palette and tab-group settings (`appearance.layout`,
//! `appearance.palette_light`, `appearance.palette_dark`, `appearance.tab_groups`; SPEC §3.4).
//!
//! Two kinds of value, on purpose:
//!
//! - **Open-ended values are validated strings**, not enums: the layout and the palette ids. A
//!   file written by a later Mercury that knows more layouts or palettes must still load in this
//!   one, so an unknown value becomes the default with a [`Repair`] instead of
//!   making the whole file "damaged".
//! - **Closed sets are enums** ([`DockStart`], [`DialCorner`], [`KeyCapture`]). Adding a variant
//!   later is a change an older reader would misread, so it needs a
//!   [`SETTINGS_FORMAT_VERSION`](crate::SETTINGS_FORMAT_VERSION) bump (see `store.rs`).
//!
//! Which palettes exist is not known here: `mercury-appearance` owns that, and checks the ids
//! when it resolves them. This module checks only that an id is well formed.
//!
//! # What loading repairs
//!
//! `AppearanceSettings::validate` runs from [`Settings::validated`](crate::Settings::validated),
//! so it applies to every load and every save. A value it refuses is replaced by the default and
//! reported as one [`Repair`], never clamped (the convention of the page zoom). None of these
//! damages the file: a damaged file is one serde cannot read at all, such as a `dial_corner` that
//! names no corner, exactly as for `theme` today.
//!
//! | Field | Rule | Becomes |
//! | --- | --- | --- |
//! | `layout` | exactly one of [`LAYOUT_IDS`] | [`DEFAULT_LAYOUT`] |
//! | `palette_light`, `palette_dark` | well formed: 1 to [`MAX_PALETTE_ID_BYTES`] bytes, `a-z` or `0-9` first, then `a-z`, `0-9` or `-`; whether the palette exists is not checked | [`DEFAULT_PALETTE`] |
//! | `tab_groups.snapshot_cap_mb` | [`MIN_SNAPSHOT_CAP_MB`] to [`MAX_SNAPSHOT_CAP_MB`] | [`DEFAULT_SNAPSHOT_CAP_MB`] |
//! | `tab_groups.keys.hold_ms` | [`MIN_KEY_HOLD_MS`] to [`MAX_KEY_HOLD_MS`] | [`DEFAULT_KEY_HOLD_MS`] |
//! | `tab_groups.keys.default_sequence` | ASCII capitals, none twice, at most [`MAX_KEY_SEQUENCE_LEN`] | its ASCII letters in capitals, the first of each kept; [`DEFAULT_KEY_SEQUENCE`] when none is left; empty stays empty |

use serde::{Deserialize, Serialize};

use crate::settings::{AppearanceSettings, Repair, quoted};

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

impl AppearanceSettings {
    /// Repairs the layout, the palette ids and the tab-group settings, appending one [`Repair`]
    /// per field changed, in field order: `layout`, `palette_light`, `palette_dark`,
    /// `tab_groups.snapshot_cap_mb`, `tab_groups.keys.hold_ms`,
    /// `tab_groups.keys.default_sequence`. The rules are in the module documentation. The page
    /// zoom is checked by [`Settings::validated`](crate::Settings::validated) itself.
    pub(crate) fn validate(&mut self, repairs: &mut Vec<Repair>) {
        if !LAYOUT_IDS.contains(&self.layout.as_str()) {
            repairs.push(Repair {
                field: "appearance.layout",
                reason: format!(
                    "{} is not a layout this version knows ({LAYOUT_MERCURY:?} or \
                     {LAYOUT_CLASSIC:?}); using {DEFAULT_LAYOUT:?}",
                    quoted(&self.layout)
                ),
            });
            DEFAULT_LAYOUT.clone_into(&mut self.layout);
        }

        repair_palette_id("appearance.palette_light", &mut self.palette_light, repairs);
        repair_palette_id("appearance.palette_dark", &mut self.palette_dark, repairs);

        let groups = &mut self.tab_groups;

        let cap = groups.snapshot_cap_mb;
        if !(MIN_SNAPSHOT_CAP_MB..=MAX_SNAPSHOT_CAP_MB).contains(&cap) {
            repairs.push(Repair {
                field: "appearance.tab_groups.snapshot_cap_mb",
                reason: format!(
                    "{cap} is outside {MIN_SNAPSHOT_CAP_MB}-{MAX_SNAPSHOT_CAP_MB}; \
                     using {DEFAULT_SNAPSHOT_CAP_MB}"
                ),
            });
            groups.snapshot_cap_mb = DEFAULT_SNAPSHOT_CAP_MB;
        }

        let hold = groups.keys.hold_ms;
        if !(MIN_KEY_HOLD_MS..=MAX_KEY_HOLD_MS).contains(&hold) {
            repairs.push(Repair {
                field: "appearance.tab_groups.keys.hold_ms",
                reason: format!(
                    "{hold} is outside {MIN_KEY_HOLD_MS}-{MAX_KEY_HOLD_MS}; \
                     using {DEFAULT_KEY_HOLD_MS}"
                ),
            });
            groups.keys.hold_ms = DEFAULT_KEY_HOLD_MS;
        }

        let sequence = &groups.keys.default_sequence;
        let kept = key_letters(sequence);
        if kept != *sequence {
            let (reason, replacement) = if kept.is_empty() {
                (
                    format!(
                        "{} has no ASCII letter; using {DEFAULT_KEY_SEQUENCE:?}",
                        quoted(sequence)
                    ),
                    DEFAULT_KEY_SEQUENCE.to_owned(),
                )
            } else {
                (
                    format!(
                        "{} is not capital letters without repeats; using {}",
                        quoted(sequence),
                        quoted(&kept)
                    ),
                    kept,
                )
            };
            repairs.push(Repair {
                field: "appearance.tab_groups.keys.default_sequence",
                reason,
            });
            groups.keys.default_sequence = replacement;
        }
    }
}

/// Puts [`DEFAULT_PALETTE`] in place of `id` if it is not well formed, and reports it.
fn repair_palette_id(field: &'static str, id: &mut String, repairs: &mut Vec<Repair>) {
    if is_well_formed_palette_id(id) {
        return;
    }
    repairs.push(Repair {
        field,
        reason: format!(
            "{} is not a well formed palette id (1-{MAX_PALETTE_ID_BYTES} bytes of a-z, 0-9 and \
             \"-\", not starting with \"-\"); using {DEFAULT_PALETTE:?}",
            quoted(id)
        ),
    });
    DEFAULT_PALETTE.clone_into(id);
}

/// Whether `id` has the form of a palette id: 1 to [`MAX_PALETTE_ID_BYTES`] bytes, the first
/// `a-z` or `0-9`, the rest `a-z`, `0-9` or `-`. Whether a palette of that id exists is for
/// `mercury-appearance` to say.
fn is_well_formed_palette_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let Some((first, rest)) = bytes.split_first() else {
        return false;
    };
    bytes.len() <= MAX_PALETTE_ID_BYTES
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && rest
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// The ASCII letters of `text` in capitals, the first of each kept, at most
/// [`MAX_KEY_SEQUENCE_LEN`] of them. Works on bytes: every byte of a multi-byte character is
/// non-ASCII, so none of them is taken for a letter.
fn key_letters(text: &str) -> String {
    let mut seen = 0_u32;
    let mut kept = String::new();
    for byte in text.bytes().filter(u8::is_ascii_alphabetic) {
        let letter = byte.to_ascii_uppercase();
        let bit = 1_u32 << (letter - b'A');
        if seen & bit == 0 {
            seen |= bit;
            kept.push(char::from(letter));
            if kept.len() == MAX_KEY_SEQUENCE_LEN {
                break;
            }
        }
    }
    kept
}
