// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! From the stored [`AppearanceSettings`] to the values the interface needs.
//!
//! [`resolve`] reads the layout, the two palette ids and the tab-group controls and returns a
//! [`Resolved`]: a [`Layout`], the two [`Palette`]s that are in use, and a [`TabGroupPolicy`]
//! made of flags, a [`Duration`], a byte count and parsed letters. The interface asks this and
//! never sees a string id, a Slint type or a range to check.
//!
//! # What is kept and what is not
//!
//! `resolve` only reads. It never changes the settings it is given, so what the user stored stays
//! stored (D9): a palette id that is well formed but that this version does not have, for
//! example one a later Mercury wrote, is **kept in `settings.json`** and shown as `signal`
//! for now, and a downgrade followed by an upgrade does not lose the user's choice. Each such
//! fallback is reported as a [`Repair`] whose `field` is the setting, but it is a *note*: nothing
//! was written anywhere. The settings window can show it ("this palette is not in this version").
//!
//! The palettes a user cannot choose (`hermes`, `mercury-b2`, `graphite`, `midnight`) are treated
//! like unknown ones, so a hand-edited file cannot switch them on (D2).
//!
//! # Values that are out of range
//!
//! Loading through `SettingsStore` already repairs every stored value (see the module
//! documentation of `mercury_settings`' `appearance.rs`), so normally `resolve` gets clean
//! settings and returns no notes. It is still total: it first applies those same stored-value
//! rules (the one implementation in `mercury-settings`, not a copy) to what it is given, so a
//! caller that built an [`AppearanceSettings`] by hand gets the default for an out-of-range value
//! and a repair for it, once. Those repairs come first in the list; the notes about palettes that
//! this crate does not have come after them, light before dark.

use std::time::Duration;

use mercury_settings::{
    DialCorner, DockStart, KeyCapture, LAYOUT_CLASSIC, LAYOUT_MERCURY, Settings,
};

pub use mercury_settings::{AppearanceSettings, Repair};

use crate::colour::Colour;
use crate::keymap::{LetterKey, parse_sequence};
use crate::palette::{Palette, Role, Scheme};
use crate::palettes;

/// How the window is arranged.
///
/// The ids are the stored `appearance.layout` strings
/// ([`LAYOUT_IDS`](mercury_settings::LAYOUT_IDS)); `tests/resolve.rs` checks that the two lists
/// agree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Layout {
    /// The toolbar on top, the group dock at the bottom, the hub ring under the toolbar.
    #[default]
    Mercury,
    /// Tabs in the title-bar strip, as browsers have always had them. The group dock is optional.
    Classic,
}

impl Layout {
    /// Every layout, the default first.
    pub const ALL: [Self; 2] = [Self::Mercury, Self::Classic];

    /// The layout with this stored id (`"mercury"` or `"classic"`, exactly), or `None`.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            LAYOUT_MERCURY => Some(Self::Mercury),
            LAYOUT_CLASSIC => Some(Self::Classic),
            _ => None,
        }
    }

    /// The id stored in `settings.json`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Mercury => LAYOUT_MERCURY,
            Self::Classic => LAYOUT_CLASSIC,
        }
    }
}

/// How the ⌥ group keys behave, from `appearance.tab_groups.keys` (the hold time is in
/// [`TabGroupPolicy::key_hold`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPolicy {
    /// Whether ⌥ group keys work at all.
    pub enabled: bool,
    /// When ⌥ and a letter is taken as a group key instead of typing a character.
    pub capture: KeyCapture,
    /// Whether each group's key is shown beside its name.
    pub show_beside_names: bool,
    /// The letters given to new groups, in order: the stored sequence parsed by
    /// [`parse_sequence`]. Empty means new groups get no key.
    pub default_sequence: Vec<LetterKey>,
}

/// What the tab-group controls come to, from `appearance.tab_groups`.
///
/// The dock has two switches (one per layout); [`dock_visible`](Self::dock_visible) picks the one
/// that applies. The other flags are the same in both layouts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabGroupPolicy {
    /// The group dock in the Mercury layout.
    pub dock: bool,
    /// The group dock in the Classic layout (off unless the user turned it on).
    pub dock_in_classic: bool,
    /// Whether the dock starts open or shrunk into its capsule.
    pub dock_starts: DockStart,
    /// The hub ring under the toolbar.
    pub ring: bool,
    /// The Orbit overview.
    pub orbit: bool,
    /// Whether pinching in on a page at 100% zoom opens Orbit.
    pub orbit_pinch: bool,
    /// The corner dial.
    pub dial: bool,
    /// The corner the dial parks in.
    pub dial_corner: DialCorner,
    /// How long ⌥ is held before the group-key card appears.
    pub key_hold: Duration,
    /// The most memory Orbit's tab previews may use, in bytes (the stored megabytes are
    /// mebibytes: 1 MB is 1,048,576 bytes).
    pub snapshot_cap_bytes: u64,
    /// The ⌥ group keys.
    pub keys: KeyPolicy,
}

impl TabGroupPolicy {
    /// Whether the group dock is shown in `layout`: the dock switch in the Mercury layout, the
    /// separate Classic switch in Classic.
    pub const fn dock_visible(&self, layout: Layout) -> bool {
        match layout {
            Layout::Mercury => self.dock,
            Layout::Classic => self.dock_in_classic,
        }
    }
}

/// The look of the browser, ready for the interface.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// The window layout.
    pub layout: Layout,
    /// The palette whose **light** table is drawn in the light scheme.
    pub palette_light: &'static Palette,
    /// The palette whose **dark** table is drawn in the dark scheme.
    pub palette_dark: &'static Palette,
    /// The tab-group controls.
    pub tab_groups: TabGroupPolicy,
}

impl Resolved {
    /// The palette in use for `scheme`.
    pub fn palette(&self, scheme: Scheme) -> &'static Palette {
        match scheme {
            Scheme::Light => self.palette_light,
            Scheme::Dark => self.palette_dark,
        }
    }

    /// The colour of `role` in `scheme`: the light table of the light palette, or the dark table
    /// of the dark palette. Never missing.
    pub fn colour(&self, scheme: Scheme, role: Role) -> Colour {
        self.palette(scheme).colour(scheme, role)
    }

    /// Whether the group dock is shown, in this layout.
    pub const fn dock_visible(&self) -> bool {
        self.tab_groups.dock_visible(self.layout)
    }
}

/// Resolves stored appearance settings, and reports anything it had to fall back from.
///
/// Reads only: `settings` is not changed, and the notes about palettes are not written back
/// (see the [module documentation](self)). Settings that came through `SettingsStore` give no
/// repairs for anything but a palette this version lacks.
pub fn resolve(settings: &AppearanceSettings) -> (Resolved, Vec<Repair>) {
    let (valid, mut repairs) = apply_stored_rules(settings);

    let layout = Layout::from_id(&valid.layout).unwrap_or_else(|| {
        // Not reachable while `LAYOUT_IDS` and `Layout` agree (a test checks it), but a layout
        // added to one and not the other must not turn into a panic.
        repairs.push(Repair {
            field: "appearance.layout",
            reason: format!(
                "{:?} is not a layout this build can draw; using {:?}",
                valid.layout,
                Layout::default().id()
            ),
        });
        Layout::default()
    });

    let palette_light = choose_palette(
        "appearance.palette_light",
        &valid.palette_light,
        &mut repairs,
    );
    let palette_dark = choose_palette("appearance.palette_dark", &valid.palette_dark, &mut repairs);

    let groups = &valid.tab_groups;
    let keys = &groups.keys;
    let resolved = Resolved {
        layout,
        palette_light,
        palette_dark,
        tab_groups: TabGroupPolicy {
            dock: groups.dock,
            dock_in_classic: groups.dock_in_classic,
            dock_starts: groups.dock_starts,
            ring: groups.ring,
            orbit: groups.orbit,
            orbit_pinch: groups.orbit_pinch,
            dial: groups.dial,
            dial_corner: groups.dial_corner,
            key_hold: Duration::from_millis(u64::from(keys.hold_ms)),
            snapshot_cap_bytes: u64::from(groups.snapshot_cap_mb) * BYTES_PER_MB,
            keys: KeyPolicy {
                enabled: keys.enabled,
                capture: keys.capture,
                show_beside_names: keys.show_beside_names,
                default_sequence: parse_sequence(&keys.default_sequence),
            },
        },
    };
    (resolved, repairs)
}

/// A stored megabyte, in bytes: the preview cap counts mebibytes.
const BYTES_PER_MB: u64 = 1024 * 1024;

/// The settings after the stored-value rules of `mercury-settings`, with the repairs they made.
///
/// Goes through [`Settings::validated`], the one place the rules live, so the two cannot drift:
/// a default `Settings` with these appearance values is validated and its appearance taken
/// back. The other fields are defaults, which need no repair, so every repair is an appearance
/// one.
fn apply_stored_rules(settings: &AppearanceSettings) -> (AppearanceSettings, Vec<Repair>) {
    let candidate = Settings {
        appearance: settings.clone(),
        ..Settings::default()
    };
    let (valid, repairs) = candidate.validated();
    (valid.appearance, repairs)
}

/// The palette that `id` names, or the default with a note when this version has no selectable
/// palette of that id.
fn choose_palette(field: &'static str, id: &str, notes: &mut Vec<Repair>) -> &'static Palette {
    if let Some(palette) = palettes::find_selectable(id) {
        return palette;
    }
    let fallback = palettes::default_palette();
    let why = if palettes::find(id).is_some() {
        "is a palette that cannot be chosen"
    } else {
        "is not a palette this version has"
    };
    // `id` is at most 64 bytes of a-z, 0-9 and "-" here: the stored-value rules have run.
    notes.push(Repair {
        field,
        reason: format!(
            "{id:?} {why}; showing {:?} for now (the setting is kept as it is)",
            fallback.id
        ),
    });
    fallback
}
