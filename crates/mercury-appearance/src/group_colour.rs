// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Tab-group colours: the 14 presets, user-picked `#rrggbb` colours, the swatch the interface
//! draws for a group, and the shade of each tab inside a group.
//!
//! **A group colour is an identification mark, not text.** The group's name is always written
//! next to it, so the colour is not required to reach 3:1 against the chrome (amber measures
//! 2.3:1 on paper). What is guaranteed is what is drawn *on* a colour: [`Swatch`] gives a `tint`
//! that the chrome's ink still reads on at 4.5:1 and a `deep` that white reads on at 4.5:1.
//!
//! Colour ids are strings kept in `session.json`. A preset's id (`"blue"`) is lower-case ASCII
//! and part of the file format forever; a custom colour's id is its `#rrggbb` in lower case.
//!
//! The ids and their order are one contract with `GroupPreset` in `talaria-core/src/groups.rs`,
//! which writes and reads the same strings: the two lists must be equal, in this order. This
//! crate does not depend on `talaria-core`, so `tests/group_colours.rs` pins this side as a
//! literal list, and whatever crate links both asserts that the two agree.

use crate::colour::Colour;
use crate::oklab::Oklab;

/// The smallest contrast ratio at which text counts as readable (WCAG 2.2 AA, normal text).
pub const MIN_TEXT_CONTRAST: f64 = 4.5;

/// The strongest a [`Swatch::tint`] ever gets: the share of the group colour in the mix with
/// the chrome surface. Without a cap, a tint on the light surface would reach the full group
/// colour for most presets, because dark ink reads on any mid-tone, and a "tint" that is the
/// colour itself is not a tint. 0.4 is a clear wash that is still unmistakably the colour.
pub const TINT_STRENGTH_MAX: f64 = 0.4;

/// One of the 14 colours Mercury offers for a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Preset {
    /// The id kept in `session.json`: lower-case ASCII, never changed or reused.
    pub id: &'static str,
    /// The English name shown in the colour picker.
    pub name: &'static str,
    /// The colour itself, drawn for the group's dot, ring and label.
    pub base: Colour,
}

/// How many presets there are.
pub const PRESET_COUNT: usize = 14;

/// The 14 group colours, in the order [`auto_colour`] hands them out.
///
/// The first seven of the design spec (blue, amber, violet, rose, green, teal, slate) keep their
/// exact values and ids; seven more fill the gaps between them. Every pair is at least 0.08
/// apart in OKLab: the closest are green and teal at 0.086, then blue and violet at 0.102 and
/// teal and slate at 0.104; every other pair is 0.13 or more.
///
/// The order is chosen, not alphabetical: it is a farthest-point order that starts from blue, so
/// the first groups in a window get the colours that are furthest apart (the first eleven stay at
/// least 0.13 apart; the spec's own order would put violet third, 0.10 from blue). Slate is last
/// because it is the colour of the "loose tabs" pseudo-group, and no real group should get it
/// before the other 13 are in use. A picker that wants a rainbow can sort by [`crate::oklab::hue`].
pub static PRESETS: [Preset; PRESET_COUNT] = [
    Preset {
        id: "blue",
        name: "Blue",
        base: Colour::hex(0x2f6fd6),
    },
    Preset {
        id: "amber",
        name: "Amber",
        base: Colour::hex(0xe0912a),
    },
    Preset {
        id: "rose",
        name: "Rose",
        base: Colour::hex(0xd9486e),
    },
    Preset {
        id: "mint",
        name: "Mint",
        base: Colour::hex(0x3ccaba),
    },
    Preset {
        id: "orchid",
        name: "Orchid",
        base: Colour::hex(0xe08af3),
    },
    Preset {
        id: "brown",
        name: "Brown",
        base: Colour::hex(0x9a5505),
    },
    Preset {
        id: "green",
        name: "Green",
        base: Colour::hex(0x2e9e62),
    },
    Preset {
        id: "lime",
        name: "Lime",
        base: Colour::hex(0x91c43e),
    },
    Preset {
        id: "sky",
        name: "Sky",
        base: Colour::hex(0x77b5fb),
    },
    Preset {
        id: "plum",
        name: "Plum",
        base: Colour::hex(0x954a7e),
    },
    Preset {
        id: "salmon",
        name: "Salmon",
        base: Colour::hex(0xf3909d),
    },
    Preset {
        id: "violet",
        name: "Violet",
        base: Colour::hex(0x7e5bd0),
    },
    Preset {
        id: "teal",
        name: "Teal",
        base: Colour::hex(0x169c9c),
    },
    Preset {
        id: "slate",
        name: "Slate",
        base: Colour::hex(0x7c8088),
    },
];

/// The preset with this exact id (lower case), if there is one.
pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

/// The colour of a group: one of the presets, or a colour the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupColour {
    /// One of [`PRESETS`].
    Preset(&'static Preset),
    /// A colour picked by the user. Opaque: alpha is never stored or shown.
    Custom(Colour),
}

impl GroupColour {
    /// A user-picked colour. Alpha is dropped: a group colour is always opaque.
    pub fn custom(colour: Colour) -> Self {
        Self::Custom(opaque(colour))
    }

    /// Reads a stored id: a preset's id (exactly, in lower case) or `#rrggbb` in either letter
    /// case. `#rgb`, alpha forms, other names (including `Blue` and `BLUE`) and anything else
    /// are `None`; the caller then falls back to [`auto_colour`] or to slate.
    pub fn parse(id: &str) -> Option<Self> {
        if let Some(preset) = preset(id) {
            return Some(Self::Preset(preset));
        }
        let digits = id.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        Colour::parse(id).map(Self::Custom)
    }

    /// The id to store: the preset's id, or `#rrggbb` in lower case.
    pub fn id(&self) -> String {
        match self {
            Self::Preset(preset) => preset.id.to_owned(),
            Self::Custom(colour) => format!("#{:02x}{:02x}{:02x}", colour.r, colour.g, colour.b),
        }
    }

    /// The colour itself.
    pub fn base(&self) -> Colour {
        match self {
            Self::Preset(preset) => preset.base,
            Self::Custom(colour) => *colour,
        }
    }

    /// The swatch to draw on `surface` (the chrome background) with `ink` (the chrome's text).
    /// See [`Swatch::new`].
    pub fn swatch(&self, surface: Colour, ink: Colour) -> Swatch {
        Swatch::new(self.base(), surface, ink)
    }
}

/// [`GroupColour::parse`] as a free function.
pub fn parse(id: &str) -> Option<GroupColour> {
    GroupColour::parse(id)
}

/// The colours the interface draws for one group, for `GroupRow` in the design spec: the mark,
/// a wash to put text on, and a deep fill to put white text on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swatch {
    /// The group colour itself, for dots, rings and strokes. It is an identification mark and
    /// need not reach 3:1 against the chrome.
    pub colour: Colour,
    /// The colour mixed into the chrome surface as far as [`TINT_STRENGTH_MAX`] and legibility
    /// allow: the chrome's ink reads on it at 4.5:1 or better.
    pub tint: Colour,
    /// The colour darkened just until white reads on it at 4.5:1 or better; the colour itself
    /// when it already does.
    pub deep: Colour,
    /// What to draw on [`deep`](Self::deep): white.
    pub on_deep: Colour,
}

impl Swatch {
    /// Builds the swatch of `base` for a chrome with this `surface` and `ink`.
    ///
    /// The guarantees hold when `ink` already reads on `surface` at 4.5:1, which the chrome
    /// palettes enforce; when it does not, `tint` is the surface itself. Mixing is the straight
    /// sRGB interpolation of [`Colour::mix`]; alpha is ignored.
    pub fn new(base: Colour, surface: Colour, ink: Colour) -> Self {
        let base = opaque(base);
        let steps = legible_tint_steps(base, surface, ink, TINT_STEPS_MAX);
        Self {
            colour: base,
            tint: tint_at(base, surface, steps),
            deep: deep_of(base),
            on_deep: Colour::WHITE,
        }
    }
}

/// How much of `base` the strongest legible tint on `surface` holds, from 0 (the surface) to 1
/// (the colour itself), in steps of 1/255. This is the limit [`Swatch::tint`] is capped from:
/// one step more would put `ink` under 4.5:1. [`TINT_STRENGTH_MAX`] is not applied.
pub fn tint_strength_limit(base: Colour, surface: Colour, ink: Colour) -> f64 {
    f64::from(legible_tint_steps(opaque(base), surface, ink, 255)) / 255.0
}

/// [`TINT_STRENGTH_MAX`] in 1/255 steps.
const TINT_STEPS_MAX: u32 = (TINT_STRENGTH_MAX * 255.0 + 0.5) as u32;

/// The largest tint strength, in 1/255 steps up to `max`, such that `ink` reads at 4.5:1 on every
/// step from 0 to it (so the answer does not depend on the contrast being monotonic).
fn legible_tint_steps(base: Colour, surface: Colour, ink: Colour, max: u32) -> u32 {
    let mut reached = 0;
    for step in 1..=max {
        if ink.contrast_ratio(tint_at(base, surface, step)) < MIN_TEXT_CONTRAST {
            break;
        }
        reached = step;
    }
    reached
}

/// `surface` with `step`/255 of `base` mixed in.
fn tint_at(base: Colour, surface: Colour, step: u32) -> Colour {
    surface.mix(base, f64::from(step) / 255.0)
}

/// `base` darkened by the smallest 1/255 step at which white reads on it at 4.5:1. Black always
/// does, so the search ends.
fn deep_of(base: Colour) -> Colour {
    (0..=255_u32)
        .map(|step| base.darker(f64::from(step) / 255.0))
        .find(|candidate| Colour::WHITE.contrast_ratio(*candidate) >= MIN_TEXT_CONTRAST)
        .unwrap_or(Colour::BLACK)
}

/// `colour` with alpha 255.
fn opaque(colour: Colour) -> Colour {
    Colour::rgb(colour.r, colour.g, colour.b)
}

/// How many different shades a group's tabs cycle through.
pub const SHADE_CYCLE: usize = 6;

/// The OKLab distance between neighbouring rungs of the shade ladder. Neighbouring tabs differ
/// by two rungs or more (0.04, twice the smallest difference a person notices, less the rounding
/// to 8-bit colours: no preset measures under 0.037), and the outermost shade is three rungs
/// (0.06) from the group colour: far enough to see, near enough that every preset's shades stay
/// closer to their own colour than to any other preset's.
pub const SHADE_STEP: f64 = 0.02;

/// OKLab lightness from which a base counts as light: its ladder reaches further up than down.
const LIGHT_BASE: f64 = 0.66;

/// The ladder positions (in rungs: positive toward white, negative toward black) that slots 0 to
/// 5 of the cycle take, indexed by how many rungs of the ladder lie below the colour. Every row
/// visits six consecutive rungs once, starts at the colour itself and keeps every step between
/// slots, including the wrap from slot 5 back to slot 0, at two rungs or more.
const SHADE_ORDER: [[i8; SHADE_CYCLE]; 6] = [
    [0, 2, 4, 1, 5, 3],
    [0, 2, 4, -1, 1, 3],
    [0, 2, -1, 1, -2, 3],
    [0, -2, 1, -1, 2, -3],
    [0, -2, -4, 1, -1, -3],
    [0, -2, -4, -1, -5, -3],
];

/// The shade of the tab at `index` in its group, from the group colour `base`: SPEC section 1's
/// "mixed toward black or white", as a repeating cycle of six ([`SHADE_CYCLE`]) so a 60-tab group
/// does not produce 60 near-equal colours. Index 0 is `base`.
///
/// Rungs are [`SHADE_STEP`] apart in OKLab, so the steps look equal whatever the colour; each is
/// found by mixing toward white or black until the distance is reached. A light colour (OKLab
/// lightness 0.66 or more) gets more rungs lighter than darker, and the other way round for a
/// dark one, so there is always room. A colour with no room on one side (white, or near black)
/// gets all its rungs on the other. Near black, 8-bit colours are too coarse to give six
/// distinct shades, and some repeat.
pub fn tab_shade(base: Colour, index: usize) -> Colour {
    let lab = Oklab::from_colour(base);
    let rungs_below = rungs_below(lab);
    let position = SHADE_ORDER[rungs_below][index % SHADE_CYCLE];
    if position == 0 {
        return base;
    }
    let toward = if position > 0 {
        Colour::WHITE
    } else {
        Colour::BLACK
    };
    shade_toward(
        base,
        lab,
        toward,
        f64::from(position.unsigned_abs()) * SHADE_STEP,
    )
}

/// How many of the ladder's five rungs besides the colour itself lie toward black (0 to 5).
fn rungs_below(lab: Oklab) -> usize {
    let rungs = |end: Colour| (lab.distance(Oklab::from_colour(end)) / SHADE_STEP).floor() as usize;
    let room_below = rungs(Colour::BLACK).min(5);
    let room_above = rungs(Colour::WHITE).min(5);
    let preferred = if lab.l >= LIGHT_BASE { 2 } else { 3 };
    // At least `5 - room_above` rungs must go below, at most `room_below`; black plus white are
    // a full OKLab unit apart, so the two bounds never cross, but stay in range regardless.
    preferred.max(5 - room_above).min(room_below)
}

/// `base` mixed toward `toward` until it is `distance` from `base` in OKLab, or all the way when
/// it never gets that far. Alpha is kept.
fn shade_toward(base: Colour, from: Oklab, toward: Colour, distance: f64) -> Colour {
    let mixed = |t: f64| base.mix(toward, t);
    let apart = |t: f64| from.distance(Oklab::from_colour(mixed(t)));
    if apart(1.0) < distance {
        return mixed(1.0);
    }
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..24 {
        let middle = (low + high) / 2.0;
        if apart(middle) < distance {
            low = middle;
        } else {
            high = middle;
        }
    }
    mixed(high)
}

/// The preset a new group gets when the user did not pick one: the first of [`PRESETS`] that no
/// id in `used` names, or, when all 14 are in use, the one used the fewest times (the first, in
/// [`PRESETS`] order, among equals). Ids that are not presets (custom colours, unknown text) are
/// ignored.
pub fn auto_colour(used: &[&str]) -> &'static Preset {
    let mut counts = [0_usize; PRESET_COUNT];
    for id in used {
        if let Some(index) = PRESETS.iter().position(|preset| preset.id == *id) {
            counts[index] += 1;
        }
    }
    let mut chosen = 0;
    for (index, count) in counts.iter().enumerate() {
        if *count < counts[chosen] {
            chosen = index;
        }
    }
    &PRESETS[chosen]
}
