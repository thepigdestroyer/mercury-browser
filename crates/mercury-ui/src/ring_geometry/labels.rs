// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Curved group labels (SPEC §4.9 `label_glyphs`): Slint has no text on a path, so a label is a
//! row of single glyphs, each centred on the label band and rotated to follow it. This file
//! decides which parts of the label fit (shortening), which way it reads (flipping on the lower
//! half) and where every glyph goes. It knows no colours (a glyph carries a [`GlyphRole`]) and no
//! Slint types.
//!
//! # Rules (re-derived from the owner-approved mockup, see the lane brief §2)
//!
//! - Parts, in reading order: the group name (12 px, weight 600), the key ("⌥" and the letter,
//!   11 px, weight 700) and the tab count (digits, 11 px, weight 400). The parts are separated by
//!   two spaces, which advance the text but emit no glyph. The two spaces are measured in the
//!   size and weight of the part they follow.
//! - `room` is the arc length at `RingSpec::label_radius()` over the group's label extent, minus a
//!   12 px margin. A candidate fits when its total advance is at most `room`.
//! - Rest tiers with a key: [`LabelFit::Full`] (name, key, count), [`LabelFit::WithoutCount`],
//!   [`LabelFit::KeyOnly`] ("⌥" and the letter alone at 12 px, weight 700), [`LabelFit::Hidden`].
//!   The mockup draws "⌥K" even when it overflows and overlaps its neighbour; Mercury hides the
//!   label instead (brief D5: the group's accessible name still carries the key).
//! - Rest tiers without a key: `Full` (name and count), `WithoutCount` (name),
//!   [`LabelFit::EllipsisName`] (as many characters as fit, then "…", at least one character),
//!   `Hidden`.
//! - [`LabelStyle::Held`] (the ⌥ key is down, shortcuts show): the key letter alone at 15 px,
//!   weight 800, then the name shortened with "…" to `room - 30`. The fit is `Full` when the name
//!   is whole, `EllipsisName` when it was shortened, `KeyOnly` when not even one character fits,
//!   `Hidden` when the key itself does not fit. There is no count. A group without a key reads
//!   as at rest.
//! - Flipping: a label whose middle satisfies `cos(theta) < -0.25` reads counter-clockwise (so
//!   it is upright on the lower half of the ring). The first glyph then sits at the larger angle
//!   and the rotation is `theta - 180`. Otherwise it reads clockwise with rotation `theta`. The
//!   threshold is the mockup generator's (`sin(mid) > 0.25`), not the geometric half: the group
//!   at 259 degrees stays unflipped in the approved mockup.
//! - The text is centred on the label extent and every glyph centre lies on the label radius.
//!
//! # Combining marks (no grapheme dependency)
//!
//! A combining mark (U+0300 to U+036F), a zero width joiner or non-joiner (U+200C, U+200D) or a
//! variation selector (U+FE00 to U+FE0F) is appended to the text of the glyph before it and adds
//! no advance. A mark with nothing to attach to (at the start of a part, after a space) is
//! dropped. No new dependency is used; this is a small subset of grapheme clusters, enough for
//! accents and for an emoji followed by its variation selector.
//!
//! # Metrics
//!
//! Advances come from a [`GlyphMetrics`] trait object. [`ApproxMetrics`], a per-character-class
//! table, is the only implementation here: Slint 1.17.1 does not expose font metrics to Rust (brief
//! D3). A measured implementation (the real font's advances, cached per size) can replace it in
//! `ring-bands` without touching this file.

use super::{GlyphPlacement, GlyphRole, GroupArc, RingSpec, point_on_circle};

/// Source of glyph advances. `advance` is the horizontal advance of `ch` in logical pixels at
/// `size` px and a CSS-like `weight` (400 regular, 600 semibold, 700 bold, 800 extra bold). Results
/// that are negative or not finite are treated as zero by this module.
pub trait GlyphMetrics {
    fn advance(&self, ch: char, size: f32, weight: u16) -> f32;
}

/// The default [`GlyphMetrics`]: a table of width factors (per em) by character class, close to
/// the mockup's flat estimate (`chars * size * 0.585` at weight 600 and above, `0.545` below) on
/// real names, but telling `i` from `m`. An estimate by design; see the module docs.
///
/// | class | characters | em, weight < 600 | em, weight >= 600 |
/// | --- | --- | --- | --- |
/// | zero | combining marks, joiners, variation selectors, control characters | 0 | 0 |
/// | full | CJK, Hangul, kana, emoji and pictographs | 1.00 | 1.00 |
/// | narrow | `i j l I t f r ' . , : ;` and white space | 0.41 | 0.44 |
/// | wide | `m w M W @ …` | 0.85 | 0.92 |
/// | digit | `0`-`9` | 0.56 | 0.60 |
/// | capital | any uppercase letter | 0.66 | 0.72 |
/// | other | everything else (lower case, "⌥", punctuation) | 0.59 | 0.64 |
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApproxMetrics;

/// Character classes of [`ApproxMetrics`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GlyphClass {
    Zero,
    Full,
    Narrow,
    Wide,
    Digit,
    Capital,
    Other,
}

impl GlyphClass {
    fn of(ch: char) -> Self {
        if is_attached(ch) || ch.is_control() {
            Self::Zero
        } else if is_full_width(ch) {
            Self::Full
        } else if ch.is_whitespace()
            || matches!(ch, 'i' | 'j' | 'l' | 'I' | 't' | 'f' | 'r')
            || matches!(ch, '\'' | '.' | ',' | ':' | ';')
        {
            Self::Narrow
        } else if matches!(ch, 'm' | 'w' | 'M' | 'W' | '@' | '…') {
            Self::Wide
        } else if ch.is_ascii_digit() {
            Self::Digit
        } else if ch.is_uppercase() {
            Self::Capital
        } else {
            Self::Other
        }
    }

    /// Width in em at a regular (`bold` false) or semibold-and-heavier weight.
    fn em(self, bold: bool) -> f32 {
        let (regular, heavy) = match self {
            Self::Zero => (0.0, 0.0),
            Self::Full => (1.0, 1.0),
            Self::Narrow => (0.41, 0.44),
            Self::Wide => (0.85, 0.92),
            Self::Digit => (0.56, 0.60),
            Self::Capital => (0.66, 0.72),
            Self::Other => (0.59, 0.64),
        };
        if bold { heavy } else { regular }
    }
}

impl GlyphMetrics for ApproxMetrics {
    fn advance(&self, ch: char, size: f32, weight: u16) -> f32 {
        GlyphClass::of(ch).em(weight >= 600) * sane(size)
    }
}

/// Whether the key letter is drawn in the mockup's wide, upright style or small and quiet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelStyle {
    /// The normal state: name, key and count, shortened by tiers.
    Rest,
    /// The ⌥ key is held: the key letter is large and the name is shortened to make room.
    Held,
}

/// What to write on one group's label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelRequest<'a> {
    /// The group's name (at most 64 bytes, SPEC §3.1; longer names do not panic).
    pub name: &'a str,
    /// The group's key letter, already upper-cased (`GroupRow.key-label`); `None` for a group
    /// without a key. A white space or control character counts as no key.
    pub key: Option<char>,
    /// The number of tabs in the group; drawn as digits (0 is drawn as "0").
    pub count: usize,
    pub style: LabelStyle,
}

/// Which tier of the shortening rules the label ended up in (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelFit {
    /// Everything this style shows fits: name, key and count at rest; key and name when held.
    Full,
    /// The count was dropped.
    WithoutCount,
    /// Only the key ("⌥" and the letter at rest, the letter alone when held).
    KeyOnly,
    /// The name was cut and ends with "…" (keyless at rest, or held).
    EllipsisName,
    /// Nothing fits: no glyphs.
    Hidden,
}

/// The glyphs of one label, in reading order, and how they were fitted.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelLayout {
    /// One placement per visible character, first character first (spaces and the two-space
    /// separators advance the text but emit nothing). Empty when `fit` is `Hidden`.
    pub glyphs: Vec<GlyphPlacement>,
    pub fit: LabelFit,
    /// The label reads counter-clockwise (it sits on the lower half): the first glyph is at the
    /// larger angle and the rotation of each glyph is its angle minus 180 degrees.
    pub flipped: bool,
}

/// Name size and weight (mockup: 12 px, 600).
const NAME_SIZE: f32 = 12.0;
const NAME_WEIGHT: u16 = 600;
/// "⌥" and key letter beside the name (11 px, 700).
const KEY_SIZE: f32 = 11.0;
const KEY_WEIGHT: u16 = 700;
/// Tab count digits (11 px, 400).
const COUNT_SIZE: f32 = 11.0;
const COUNT_WEIGHT: u16 = 400;
/// The key alone, once the name no longer fits (12 px, 700).
const KEY_ONLY_SIZE: f32 = 12.0;
const KEY_ONLY_WEIGHT: u16 = 700;
/// The key letter while ⌥ is held (15 px, 800).
const HELD_KEY_SIZE: f32 = 15.0;
const HELD_KEY_WEIGHT: u16 = 800;
/// Arc length kept free of text at the ends of the label extent, in total.
const EDGE_MARGIN: f32 = 12.0;
/// What the held key letter and its gap are assumed to take from the name's room.
const HELD_RESERVE: f32 = 30.0;
/// Flip when `cos(theta) < FLIP_COS` (the mockup's `sin(mid) > 0.25`).
const FLIP_COS: f32 = -0.25;
/// The option-key symbol that precedes the key letter.
const KEY_SYMBOL: char = '⌥';
const ELLIPSIS: char = '…';

/// Whether a label whose middle is at `mid_deg` (degrees clockwise from 12 o'clock) reads
/// counter-clockwise so that it is upright on the lower half: `cos(theta) < -0.25`, that is
/// `mid_deg` in about (104.48, 255.52) for angles in `[0, 360)`. Any angle is accepted; NaN is
/// not flipped.
pub fn is_flipped(mid_deg: f32) -> bool {
    mid_deg.to_radians().cos() < FLIP_COS
}

/// Lays the label of one group on its label band.
///
/// `group` supplies the label extent (`label_start_deg..label_end_deg`), `spec` the centre and
/// `label_radius()`, `req` what to write and `metrics` the advances. Never panics: an empty name,
/// an empty or reversed extent, a zero count or non-finite numbers give fewer glyphs or none
/// ([`LabelFit::Hidden`]).
pub fn label_glyphs(
    group: &GroupArc,
    spec: &RingSpec,
    req: &LabelRequest<'_>,
    metrics: &dyn GlyphMetrics,
) -> LabelLayout {
    let flipped = is_flipped((group.label_start_deg + group.label_end_deg) / 2.0);
    let radius = sane(spec.label_radius());
    let extent_deg = sane(group.label_end_deg - group.label_start_deg);
    let arc_len = extent_deg.to_radians() * radius;
    let room = arc_len - EDGE_MARGIN;
    let hidden = LabelLayout {
        glyphs: Vec::new(),
        fit: LabelFit::Hidden,
        flipped,
    };
    if room <= 0.0 {
        return hidden;
    }
    let Some((runs, fit)) = choose(req, room, metrics) else {
        return hidden;
    };

    // The text is centred on the extent: it starts `(arc_len - total) / 2` along the arc from
    // the end the text reads from (the start when unflipped, the end when flipped).
    let mut pos = (arc_len - total_width(&runs)) / 2.0;
    let mut glyphs = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        for atom in &run.atoms {
            let centre_along = pos + atom.advance / 2.0;
            pos += atom.advance;
            if !atom.ink {
                continue;
            }
            let along_deg = (centre_along / radius).to_degrees();
            let theta = if flipped {
                group.label_end_deg - along_deg
            } else {
                group.label_start_deg + along_deg
            };
            glyphs.push(GlyphPlacement {
                text: atom.text.clone(),
                centre: point_on_circle(spec.centre, radius, theta),
                angle_deg: if flipped { theta - 180.0 } else { theta },
                size: run.size,
                weight: run.weight,
                role: run.role,
            });
        }
        if index + 1 < runs.len() {
            pos += run.gap;
        }
    }
    LabelLayout {
        glyphs,
        fit,
        flipped,
    }
}

/// `x` if it is a finite positive number, else 0 (keeps NaN and negative sizes out of the maths).
fn sane(x: f32) -> f32 {
    if x.is_finite() && x > 0.0 { x } else { 0.0 }
}

/// Whether `ch` attaches to the glyph before it and adds no advance (see the module docs).
fn is_attached(ch: char) -> bool {
    matches!(ch as u32, 0x0300..=0x036F | 0x200C..=0x200D | 0xFE00..=0xFE0F)
}

/// Characters one em wide: CJK, Hangul, kana, fullwidth forms, emoji and pictographs.
fn is_full_width(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1100..=0x11FF
            | 0x2600..=0x27BF
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F000..=0x1FAFF
            | 0x20000..=0x3FFFF
    )
}

/// A key letter that can be drawn: not white space, not a control or attaching character.
fn usable_key(key: char) -> bool {
    !key.is_whitespace() && !key.is_control() && !is_attached(key)
}

/// One character cluster of a run: a base character with the marks attached to it.
#[derive(Clone, Debug)]
struct Atom {
    text: String,
    advance: f32,
    /// Whether the atom is drawn (false for white space: it advances but emits no glyph).
    ink: bool,
}

/// A part of the label (name, key or count) in one size, weight and role.
#[derive(Clone, Debug)]
struct Run {
    atoms: Vec<Atom>,
    size: f32,
    weight: u16,
    role: GlyphRole,
    /// The two spaces that follow this run when another run follows it.
    gap: f32,
}

impl Run {
    /// Splits `text` into atoms and measures them. Control characters are dropped, as are marks
    /// with nothing to attach to.
    fn shape(
        text: &str,
        size: f32,
        weight: u16,
        role: GlyphRole,
        metrics: &dyn GlyphMetrics,
    ) -> Self {
        let mut atoms: Vec<Atom> = Vec::new();
        for ch in text.chars() {
            if is_attached(ch) {
                if let Some(base) = atoms.last_mut().filter(|atom| atom.ink) {
                    base.text.push(ch);
                }
            } else if !ch.is_control() {
                atoms.push(Atom {
                    text: ch.to_string(),
                    advance: sane(metrics.advance(ch, size, weight)),
                    ink: !ch.is_whitespace(),
                });
            }
        }
        Self {
            atoms,
            size,
            weight,
            role,
            gap: 2.0 * sane(metrics.advance(' ', size, weight)),
        }
    }

    /// A run with nothing to draw (empty text, or white space only) is left out of a label.
    fn has_ink(&self) -> bool {
        self.atoms.iter().any(|atom| atom.ink)
    }

    fn width(&self) -> f32 {
        self.atoms.iter().map(|atom| atom.advance).sum()
    }
}

/// The runs that have something to draw.
fn compose(runs: impl IntoIterator<Item = Run>) -> Vec<Run> {
    runs.into_iter().filter(Run::has_ink).collect()
}

/// The advance of a whole label: its runs and the separators between them.
fn total_width(runs: &[Run]) -> f32 {
    let separators: f32 = runs.iter().rev().skip(1).map(|run| run.gap).sum();
    separators + runs.iter().map(Run::width).sum::<f32>()
}

fn fits(runs: &[Run], room: f32) -> bool {
    !runs.is_empty() && total_width(runs) <= room
}

/// The first tier of `req`'s style whose candidate fits `room`.
fn choose(
    req: &LabelRequest<'_>,
    room: f32,
    metrics: &dyn GlyphMetrics,
) -> Option<(Vec<Run>, LabelFit)> {
    let name = Run::shape(req.name, NAME_SIZE, NAME_WEIGHT, GlyphRole::Name, metrics);
    let key = req.key.filter(|key| usable_key(*key));
    match (req.style, key) {
        (LabelStyle::Held, Some(key)) => held(name, key, room, metrics),
        (_, Some(key)) => with_key(req, name, key, room, metrics),
        (_, None) => keyless(req, name, room, metrics),
    }
}

/// Rest tiers of a group with a key: `Full`, `WithoutCount`, `KeyOnly`.
fn with_key(
    req: &LabelRequest<'_>,
    name: Run,
    key: char,
    room: f32,
    metrics: &dyn GlyphMetrics,
) -> Option<(Vec<Run>, LabelFit)> {
    let key_text = format!("{KEY_SYMBOL}{key}");
    let key_run = Run::shape(&key_text, KEY_SIZE, KEY_WEIGHT, GlyphRole::Key, metrics);
    let count = count_run(req.count, metrics);
    let tiers = [
        (
            compose([name.clone(), key_run.clone(), count]),
            LabelFit::Full,
        ),
        (compose([name, key_run]), LabelFit::WithoutCount),
        (
            compose([Run::shape(
                &key_text,
                KEY_ONLY_SIZE,
                KEY_ONLY_WEIGHT,
                GlyphRole::Key,
                metrics,
            )]),
            LabelFit::KeyOnly,
        ),
    ];
    tiers.into_iter().find(|(runs, _)| fits(runs, room))
}

/// Rest tiers of a group without a key: `Full`, `WithoutCount`, `EllipsisName`.
fn keyless(
    req: &LabelRequest<'_>,
    name: Run,
    room: f32,
    metrics: &dyn GlyphMetrics,
) -> Option<(Vec<Run>, LabelFit)> {
    let full = compose([name.clone(), count_run(req.count, metrics)]);
    if fits(&full, room) {
        return Some((full, LabelFit::Full));
    }
    let name_only = compose([name.clone()]);
    if fits(&name_only, room) {
        return Some((name_only, LabelFit::WithoutCount));
    }
    let cut = compose(ellipsise(&name, room, metrics));
    fits(&cut, room).then_some((cut, LabelFit::EllipsisName))
}

/// The held-key label: the key letter, then the name shortened to `room - 30`.
fn held(
    name: Run,
    key: char,
    room: f32,
    metrics: &dyn GlyphMetrics,
) -> Option<(Vec<Run>, LabelFit)> {
    let key_run = Run::shape(
        &key.to_string(),
        HELD_KEY_SIZE,
        HELD_KEY_WEIGHT,
        GlyphRole::Key,
        metrics,
    );
    if !fits(std::slice::from_ref(&key_run), room) {
        return None;
    }
    // The name gets `room - 30`, and never more than what is left after the key and its gap.
    let budget = (room - HELD_RESERVE).min(room - key_run.width() - key_run.gap);
    if name.has_ink() {
        if name.width() <= budget {
            return Some((compose([key_run, name]), LabelFit::Full));
        }
        if let Some(cut) = ellipsise(&name, budget, metrics) {
            return Some((compose([key_run, cut]), LabelFit::EllipsisName));
        }
    }
    Some((vec![key_run], LabelFit::KeyOnly))
}

fn count_run(count: usize, metrics: &dyn GlyphMetrics) -> Run {
    Run::shape(
        &count.to_string(),
        COUNT_SIZE,
        COUNT_WEIGHT,
        GlyphRole::Count,
        metrics,
    )
}

/// `run` cut after as many atoms as leave room for "…" within `budget`, with the "…" appended
/// (trailing white space before it is dropped). `None` when not even one character and the "…"
/// fit. Cuts fall between atoms, so a mark is never separated from its base.
fn ellipsise(run: &Run, budget: f32, metrics: &dyn GlyphMetrics) -> Option<Run> {
    if !budget.is_finite() {
        return None;
    }
    let ellipsis = Run::shape(
        &ELLIPSIS.to_string(),
        run.size,
        run.weight,
        run.role,
        metrics,
    );
    let ellipsis_width = ellipsis.width();
    let mut used = 0.0;
    let mut keep = 0;
    for (index, atom) in run.atoms.iter().enumerate() {
        if used + atom.advance + ellipsis_width > budget {
            break;
        }
        used += atom.advance;
        keep = index + 1;
    }
    while keep > 0 && run.atoms.get(keep - 1).is_some_and(|atom| !atom.ink) {
        keep -= 1;
    }
    if keep == 0 {
        return None;
    }
    let mut atoms: Vec<Atom> = run.atoms.iter().take(keep).cloned().collect();
    atoms.extend(ellipsis.atoms);
    Some(Run {
        atoms,
        size: run.size,
        weight: run.weight,
        role: run.role,
        gap: run.gap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is 10 px wide, so the tier and cut points are exact.
    struct Fixed;

    impl GlyphMetrics for Fixed {
        fn advance(&self, _: char, _: f32, _: u16) -> f32 {
            10.0
        }
    }

    /// Metrics that return the same (possibly nonsensical) number for every character.
    struct Constant(f32);

    impl GlyphMetrics for Constant {
        fn advance(&self, _: char, _: f32, _: u16) -> f32 {
            self.0
        }
    }

    /// The hub ring's default: label radius 209, centre (258, 258).
    fn spec() -> RingSpec {
        RingSpec::default()
    }

    fn radius() -> f32 {
        spec().label_radius()
    }

    fn assert_close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    /// A group whose label extent is exactly `label_start..label_end`.
    fn group(label_start: f32, label_end: f32) -> GroupArc {
        GroupArc {
            group: 0,
            first_tab: 0,
            tab_count: 3,
            start_deg: label_start - 0.4,
            end_deg: label_end + 0.4,
            label_start_deg: label_start,
            label_end_deg: label_end,
        }
    }

    /// A group centred on `mid` whose label `room` (arc length minus the brief's 12 px margin,
    /// written out here so the constant is pinned) is `room`.
    fn group_with_room(mid: f32, room: f32) -> GroupArc {
        let extent = ((room + 12.0) / radius()).to_degrees();
        group(mid - extent / 2.0, mid + extent / 2.0)
    }

    /// A wide group (60 degrees, 190 px of room at the hub ring) centred on `mid`.
    fn wide_group(mid: f32) -> GroupArc {
        group(mid - 30.0, mid + 30.0)
    }

    fn request(name: &str, key: Option<char>, count: usize) -> LabelRequest<'_> {
        LabelRequest {
            name,
            key,
            count,
            style: LabelStyle::Rest,
        }
    }

    fn held_request(name: &str, key: Option<char>, count: usize) -> LabelRequest<'_> {
        LabelRequest {
            style: LabelStyle::Held,
            ..request(name, key, count)
        }
    }

    fn lay_out(group: &GroupArc, req: &LabelRequest<'_>) -> LabelLayout {
        label_glyphs(group, &spec(), req, &ApproxMetrics)
    }

    fn lay_out_fixed(group: &GroupArc, req: &LabelRequest<'_>) -> LabelLayout {
        label_glyphs(group, &spec(), req, &Fixed)
    }

    /// The glyph texts joined: spaces emit nothing, so "Lisbon trip" reads "Lisbontrip".
    fn text_of(layout: &LabelLayout) -> String {
        layout
            .glyphs
            .iter()
            .map(|glyph| glyph.text.as_str())
            .collect()
    }

    fn texts_with_role(layout: &LabelLayout, role: GlyphRole) -> String {
        layout
            .glyphs
            .iter()
            .filter(|glyph| glyph.role == role)
            .map(|glyph| glyph.text.as_str())
            .collect()
    }

    /// The sum of the table's advances of `text`.
    fn width(text: &str, size: f32, weight: u16) -> f32 {
        text.chars()
            .map(|ch| ApproxMetrics.advance(ch, size, weight))
            .sum()
    }

    fn adv(ch: char, size: f32, weight: u16) -> f32 {
        ApproxMetrics.advance(ch, size, weight)
    }

    /// The advance of the glyph's first character at the glyph's size and weight.
    fn advance_of(glyph: &GlyphPlacement) -> f32 {
        let ch = glyph.text.chars().next().unwrap();
        adv(ch, glyph.size, glyph.weight)
    }

    /// Where a glyph centre sits, in degrees clockwise from 12 o'clock, in `[0, 360)`.
    fn theta_of(glyph: &GlyphPlacement) -> f32 {
        let centre = spec().centre;
        let dx = glyph.centre.x - centre.x;
        let dy = glyph.centre.y - centre.y;
        dx.atan2(-dy).to_degrees().rem_euclid(360.0)
    }

    fn distance_to_centre(glyph: &GlyphPlacement) -> f32 {
        let centre = spec().centre;
        (glyph.centre.x - centre.x).hypot(glyph.centre.y - centre.y)
    }

    // --- flipping ---

    #[test]
    fn flip_threshold_is_cos_minus_one_quarter() {
        let threshold = (-0.25_f32).acos().to_degrees();
        assert_close(threshold, 104.4775, 1e-3);
        assert!(!is_flipped(threshold - 0.01));
        assert!(is_flipped(threshold + 0.01));
        let upper = 360.0 - threshold;
        assert_close(upper, 255.5225, 1e-3);
        assert!(is_flipped(upper - 0.01));
        assert!(!is_flipped(upper + 0.01));
    }

    #[test]
    fn flip_covers_the_lower_part_and_accepts_any_angle() {
        assert!(!is_flipped(0.0));
        assert!(!is_flipped(90.0));
        assert!(is_flipped(180.0));
        assert!(!is_flipped(270.0));
        assert!(!is_flipped(359.9));
        assert!(
            is_flipped(180.0 + 360.0),
            "unwrapped extents flip like their fold"
        );
        assert!(is_flipped(-180.0));
        assert!(!is_flipped(f32::NAN));
        assert!(!is_flipped(f32::INFINITY));
    }

    #[test]
    fn the_three_mockup_groups_flip_as_in_the_mockup() {
        // The 22-tab ring (groups of 5, 4, 6, 3, 2, 2 tabs, pitch 13.909 degrees): Lisbon trip
        // is the second group, Reading the third and Shopping the fourth. Their label extents are
        // the group extents inset by 0.4 degrees.
        let lisbon = group(89.445, 144.282);
        let reading = group(150.082, 232.736);
        let shopping = group(238.536, 279.464);
        let mid = |g: &GroupArc| (g.label_start_deg + g.label_end_deg) / 2.0;
        assert_close(mid(&lisbon), 116.9, 0.15);
        assert_close(mid(&reading), 191.5, 0.15);
        assert_close(mid(&shopping), 259.0, 0.15);
        let lisbon_label = lay_out(&lisbon, &request("Lisbon trip", Some('L'), 4));
        let reading_label = lay_out(&reading, &request("Reading", Some('R'), 6));
        let shopping_label = lay_out(&shopping, &request("Shopping", Some('S'), 3));
        assert!(lisbon_label.flipped);
        assert!(reading_label.flipped);
        assert!(!shopping_label.flipped);
        assert_eq!(lisbon_label.fit, LabelFit::Full);
        assert_eq!(reading_label.fit, LabelFit::Full);
        assert_eq!(shopping_label.fit, LabelFit::Full);
        assert_eq!(text_of(&lisbon_label), "Lisbontrip⌥L4");
        assert_eq!(text_of(&shopping_label), "Shopping⌥S3");
    }

    // --- order, rotation, radius ---

    #[test]
    fn an_unflipped_label_reads_clockwise_and_rotates_with_theta() {
        let layout = lay_out(&wide_group(30.0), &request("Research", None, 5));
        assert!(!layout.flipped);
        assert_eq!(layout.fit, LabelFit::Full);
        assert_eq!(text_of(&layout), "Research5");
        let thetas: Vec<f32> = layout.glyphs.iter().map(theta_of).collect();
        assert!(
            thetas.windows(2).all(|pair| pair[0] < pair[1]),
            "{thetas:?}"
        );
        for (glyph, theta) in layout.glyphs.iter().zip(&thetas) {
            assert_close(glyph.angle_deg, *theta, 0.01);
        }
        // Everything lies inside the label extent.
        assert!(thetas[0] > 0.0 && thetas[thetas.len() - 1] < 60.0);
    }

    #[test]
    fn a_flipped_label_reads_counter_clockwise_and_rotates_with_theta_minus_180() {
        let layout = lay_out(&wide_group(190.0), &request("Research", None, 5));
        assert!(layout.flipped);
        assert_eq!(text_of(&layout), "Research5");
        let thetas: Vec<f32> = layout.glyphs.iter().map(theta_of).collect();
        assert!(
            thetas.windows(2).all(|pair| pair[0] > pair[1]),
            "{thetas:?}"
        );
        assert_eq!(
            layout.glyphs[0].text, "R",
            "the first glyph sits at the larger angle"
        );
        for (glyph, theta) in layout.glyphs.iter().zip(&thetas) {
            assert_close(glyph.angle_deg, theta - 180.0, 0.01);
        }
        assert!(thetas[0] < 220.0 && thetas[thetas.len() - 1] > 160.0);
    }

    #[test]
    fn every_glyph_centre_is_on_the_label_radius() {
        for mid in [10.0, 90.0, 120.0, 190.0, 259.0, 330.0] {
            let layout = lay_out(&wide_group(mid), &request("Lisbon trip", Some('L'), 12));
            assert!(!layout.glyphs.is_empty());
            for glyph in &layout.glyphs {
                assert_close(distance_to_centre(glyph), radius(), 1e-3);
            }
        }
    }

    #[test]
    fn glyph_centres_are_spaced_by_the_half_advances() {
        // Unflipped and flipped, name and count: two glyphs are (adv_a + adv_b) / 2 apart along
        // the arc, with the two separator spaces between the name and the count.
        for mid in [30.0, 190.0] {
            let layout = lay_out(&wide_group(mid), &request("Lisbon trip", None, 7));
            assert_eq!(text_of(&layout), "Lisbontrip7");
            let thetas: Vec<f32> = layout.glyphs.iter().map(theta_of).collect();
            let step = |index: usize| (thetas[index] - thetas[index + 1]).abs();
            let in_degrees = |px: f32| (px / radius()).to_degrees();
            // 'L' and 'i'.
            assert_close(
                step(0),
                in_degrees((adv('L', 12.0, 600) + adv('i', 12.0, 600)) / 2.0),
                1e-3,
            );
            // 'n' and 't' with the space between them.
            assert_close(
                step(5),
                in_degrees(
                    adv('n', 12.0, 600) / 2.0 + adv(' ', 12.0, 600) + adv('t', 12.0, 600) / 2.0,
                ),
                1e-3,
            );
            // 'p' and the count digit, with two spaces at the name's size between them.
            assert_close(
                step(9),
                in_degrees(
                    adv('p', 12.0, 600) / 2.0
                        + 2.0 * adv(' ', 12.0, 600)
                        + adv('7', 11.0, 400) / 2.0,
                ),
                1e-3,
            );
        }
    }

    #[test]
    fn the_text_is_centred_on_the_label_extent() {
        for (start, end) in [(10.0, 80.0), (170.0, 230.0)] {
            let layout = lay_out(&group(start, end), &request("Research", Some('R'), 12));
            let first = layout.glyphs.first().unwrap();
            let last = layout.glyphs.last().unwrap();
            let half = |glyph: &GlyphPlacement| (advance_of(glyph) / 2.0 / radius()).to_degrees();
            let (left, right) = if layout.flipped {
                (theta_of(last) - half(last), theta_of(first) + half(first))
            } else {
                (theta_of(first) - half(first), theta_of(last) + half(last))
            };
            assert_close((left + right) / 2.0, (start + end) / 2.0, 1e-3);
        }
    }

    #[test]
    fn unwrapped_extents_beyond_360_keep_the_raw_angles() {
        let layout = lay_out(&group(350.0, 410.0), &request("Reading", None, 3));
        assert!(!layout.flipped, "the middle (380) folds to 20 degrees");
        assert_eq!(text_of(&layout), "Reading3");
        for glyph in &layout.glyphs {
            assert_close(distance_to_centre(glyph), radius(), 1e-3);
            assert!((350.0..=410.0).contains(&glyph.angle_deg));
            let folded = glyph.angle_deg.rem_euclid(360.0);
            let seen = theta_of(glyph);
            let apart = (folded - seen).abs();
            assert!(apart < 0.01 || (360.0 - apart) < 0.01, "{folded} vs {seen}");
        }
    }

    // --- tiers ---

    #[test]
    fn each_tier_with_a_key_is_reached_by_shrinking_the_arc() {
        let req = request("Lisbon trip", Some('L'), 12);
        let name = width("Lisbon trip", 12.0, 600);
        let name_gap = 2.0 * adv(' ', 12.0, 600);
        let key = width("⌥L", 11.0, 700);
        let key_gap = 2.0 * adv(' ', 11.0, 700);
        let count = width("12", 11.0, 400);
        let full = name + name_gap + key + key_gap + count;
        let without_count = name + name_gap + key;
        let key_only = width("⌥L", 12.0, 700);

        let layout = lay_out(&group_with_room(30.0, full + 0.5), &req);
        assert_eq!(layout.fit, LabelFit::Full);
        assert_eq!(text_of(&layout), "Lisbontrip⌥L12");
        assert_eq!(texts_with_role(&layout, GlyphRole::Name), "Lisbontrip");
        assert_eq!(texts_with_role(&layout, GlyphRole::Key), "⌥L");
        assert_eq!(texts_with_role(&layout, GlyphRole::Count), "12");

        let layout = lay_out(&group_with_room(30.0, full - 0.5), &req);
        assert_eq!(layout.fit, LabelFit::WithoutCount);
        assert_eq!(text_of(&layout), "Lisbontrip⌥L");

        let layout = lay_out(&group_with_room(30.0, without_count - 0.5), &req);
        assert_eq!(layout.fit, LabelFit::KeyOnly);
        assert_eq!(text_of(&layout), "⌥L");
        assert!(layout.glyphs.iter().all(|glyph| glyph.size == 12.0
            && glyph.weight == 700
            && glyph.role == GlyphRole::Key));

        let layout = lay_out(&group_with_room(30.0, key_only + 0.5), &req);
        assert_eq!(layout.fit, LabelFit::KeyOnly);

        let layout = lay_out(&group_with_room(30.0, key_only - 0.5), &req);
        assert_eq!(layout.fit, LabelFit::Hidden);
        assert!(layout.glyphs.is_empty());
    }

    #[test]
    fn parts_carry_the_mockup_sizes_weights_and_roles() {
        let layout = lay_out(&wide_group(30.0), &request("Shopping", Some('S'), 3));
        assert_eq!(layout.fit, LabelFit::Full);
        let look = |role| {
            let found: Vec<(f32, u16)> = layout
                .glyphs
                .iter()
                .filter(|glyph| glyph.role == role)
                .map(|glyph| (glyph.size, glyph.weight))
                .collect();
            assert!(!found.is_empty());
            assert!(found.iter().all(|entry| *entry == found[0]));
            found[0]
        };
        assert_eq!(look(GlyphRole::Name), (12.0, 600));
        assert_eq!(look(GlyphRole::Key), (11.0, 700));
        assert_eq!(look(GlyphRole::Count), (11.0, 400));
    }

    #[test]
    fn keyless_tiers_are_full_without_count_ellipsis_and_hidden() {
        let req = request("Shopping", None, 7);
        let name = width("Shopping", 12.0, 600);
        let count = width("7", 11.0, 400);
        let gap = 2.0 * adv(' ', 12.0, 600);

        let layout = lay_out(&group_with_room(30.0, name + gap + count + 0.5), &req);
        assert_eq!(layout.fit, LabelFit::Full);
        assert_eq!(text_of(&layout), "Shopping7");

        let layout = lay_out(&group_with_room(30.0, name + gap + count - 0.5), &req);
        assert_eq!(layout.fit, LabelFit::WithoutCount);
        assert_eq!(text_of(&layout), "Shopping");

        let layout = lay_out(&group_with_room(30.0, name - 0.5), &req);
        assert_eq!(layout.fit, LabelFit::EllipsisName);
        let shown = text_of(&layout);
        assert!(
            shown.ends_with('…') && shown.chars().count() >= 2,
            "{shown}"
        );
        assert!("Shopping".starts_with(shown.trim_end_matches('…')));
        assert!(
            layout
                .glyphs
                .iter()
                .all(|glyph| glyph.role == GlyphRole::Name)
        );

        // The smallest cut is one character and the ellipsis.
        let minimum = adv('S', 12.0, 600) + adv('…', 12.0, 600);
        let layout = lay_out(&group_with_room(30.0, minimum + 0.5), &req);
        assert_eq!(layout.fit, LabelFit::EllipsisName);
        assert_eq!(text_of(&layout), "S…");
        let layout = lay_out(&group_with_room(30.0, minimum - 0.5), &req);
        assert_eq!(layout.fit, LabelFit::Hidden);
        assert!(layout.glyphs.is_empty());
    }

    #[test]
    fn an_ellipsis_cut_keeps_as_many_characters_as_fit() {
        let name = "Research";
        let room = 40.0;
        let layout = lay_out(&group_with_room(30.0, room), &request(name, None, 1));
        assert_eq!(layout.fit, LabelFit::EllipsisName);
        let shown = text_of(&layout);
        let kept = shown.trim_end_matches('…');
        assert!(name.starts_with(kept));
        assert!(width(&shown, 12.0, 600) <= room);
        let one_more: String = name.chars().take(kept.chars().count() + 1).collect();
        assert!(width(&one_more, 12.0, 600) + adv('…', 12.0, 600) > room);
    }

    #[test]
    fn an_ellipsis_cut_drops_the_white_space_before_the_ellipsis() {
        // "ab cd" is 50 wide: a cut at 45.5 keeps "ab " (three characters before the ellipsis)
        // and drops the space.
        let layout = lay_out_fixed(&group_with_room(30.0, 45.5), &request("ab cd", None, 1));
        assert_eq!(layout.fit, LabelFit::EllipsisName);
        assert_eq!(text_of(&layout), "ab…");
    }

    #[test]
    fn exact_tier_arithmetic_with_fixed_metrics() {
        // "abcd" = 40, two spaces = 20, count "5" = 10.
        let req = request("abcd", None, 5);
        let at = |room: f32| lay_out_fixed(&group_with_room(30.0, room), &req);
        assert_eq!(at(70.5).fit, LabelFit::Full);
        assert_eq!(at(69.5).fit, LabelFit::WithoutCount);
        assert_eq!(at(40.5).fit, LabelFit::WithoutCount);
        assert_eq!(
            text_of(&at(35.0)),
            "ab…",
            "10 per character: two and the ellipsis"
        );
        assert_eq!(text_of(&at(25.0)), "a…");
        assert_eq!(at(15.0).fit, LabelFit::Hidden);
        // With a key: "abcd" 40 + 20 + "⌥K" 20 (+ 20 + "5" 10).
        let req = request("abcd", Some('K'), 5);
        let at = |room: f32| lay_out_fixed(&group_with_room(30.0, room), &req);
        assert_eq!(at(110.5).fit, LabelFit::Full);
        assert_eq!(at(109.5).fit, LabelFit::WithoutCount);
        assert_eq!(at(79.5).fit, LabelFit::KeyOnly);
        assert_eq!(at(19.5).fit, LabelFit::Hidden);
    }

    // --- held ---

    #[test]
    fn held_shows_the_large_key_letter_then_the_name() {
        let layout = lay_out(&wide_group(30.0), &held_request("Research", Some('R'), 5));
        assert_eq!(layout.fit, LabelFit::Full);
        assert_eq!(
            text_of(&layout),
            "RResearch",
            "no count and no option symbol"
        );
        let key = &layout.glyphs[0];
        assert_eq!(
            (key.size, key.weight, key.role),
            (15.0, 800, GlyphRole::Key)
        );
        for glyph in &layout.glyphs[1..] {
            assert_eq!(
                (glyph.size, glyph.weight, glyph.role),
                (12.0, 600, GlyphRole::Name)
            );
        }
    }

    #[test]
    fn held_shortens_the_name_to_room_minus_30() {
        let req = held_request("Research", Some('K'), 5);
        let name = width("Research", 12.0, 600);
        // The whole name fits at room = name + 30.
        let layout = lay_out(&group_with_room(30.0, name + 30.5), &req);
        assert_eq!(layout.fit, LabelFit::Full);
        assert_eq!(text_of(&layout), "KResearch");
        // A little less and it is cut.
        let room = name + 29.5;
        let layout = lay_out(&group_with_room(30.0, room), &req);
        assert_eq!(layout.fit, LabelFit::EllipsisName);
        let shown = text_of(&layout);
        assert!(shown.starts_with('K') && shown.ends_with('…'), "{shown}");
        let cut = &shown['K'.len_utf8()..];
        assert!(width(cut, 12.0, 600) <= room - 30.0);
        let kept = cut.trim_end_matches('…');
        let one_more: String = "Research".chars().take(kept.chars().count() + 1).collect();
        assert!(width(&one_more, 12.0, 600) + adv('…', 12.0, 600) > room - 30.0);
        // The whole label never exceeds the room.
        let total = width("K", 15.0, 800) + 2.0 * adv(' ', 15.0, 800) + width(cut, 12.0, 600);
        assert!(total <= room);
    }

    #[test]
    fn held_falls_back_to_the_key_alone_and_then_hides() {
        let req = held_request("Research", Some('K'), 5);
        let key = width("K", 15.0, 800);
        // No room for one character and the ellipsis after the 30 px.
        let layout = lay_out(&group_with_room(30.0, 40.0), &req);
        assert_eq!(layout.fit, LabelFit::KeyOnly);
        assert_eq!(text_of(&layout), "K");
        assert_eq!(layout.glyphs[0].size, 15.0);
        let layout = lay_out(&group_with_room(30.0, key + 0.5), &req);
        assert_eq!(layout.fit, LabelFit::KeyOnly);
        let layout = lay_out(&group_with_room(30.0, key - 0.5), &req);
        assert_eq!(layout.fit, LabelFit::Hidden);
        assert!(layout.glyphs.is_empty());
    }

    #[test]
    fn held_with_an_empty_name_is_the_key_alone_and_without_a_key_it_reads_as_at_rest() {
        let layout = lay_out(&wide_group(30.0), &held_request("", Some('Q'), 4));
        assert_eq!(layout.fit, LabelFit::KeyOnly);
        assert_eq!(text_of(&layout), "Q");
        let target = group_with_room(200.0, 60.0);
        assert_eq!(
            lay_out(&target, &held_request("Reading list", None, 4)),
            lay_out(&target, &request("Reading list", None, 4)),
        );
    }

    // --- counts, names, keys ---

    #[test]
    fn counts_are_drawn_as_digits() {
        for (count, expected) in [(0, "0"), (5, "5"), (10, "10"), (1000, "1000")] {
            let layout = lay_out(&wide_group(30.0), &request("A", None, count));
            assert_eq!(layout.fit, LabelFit::Full);
            assert_eq!(texts_with_role(&layout, GlyphRole::Count), expected);
            assert_eq!(text_of(&layout), format!("A{expected}"));
            assert!(
                layout
                    .glyphs
                    .iter()
                    .filter(|glyph| glyph.role == GlyphRole::Count)
                    .all(|glyph| glyph.size == 11.0 && glyph.weight == 400)
            );
        }
    }

    #[test]
    fn counts_with_more_digits_take_more_room() {
        let one = width("5", 11.0, 400);
        let four = width("1000", 11.0, 400);
        assert_close(four, 4.0 * one, 1e-4);
        // A room that holds "5" but not "1000" drops the count of the larger number only.
        let name = width("Home", 12.0, 600);
        let gap = 2.0 * adv(' ', 12.0, 600);
        let room = name + gap + one + 0.5;
        assert_eq!(
            lay_out(&group_with_room(30.0, room), &request("Home", None, 5)).fit,
            LabelFit::Full
        );
        assert_eq!(
            lay_out(&group_with_room(30.0, room), &request("Home", None, 1000)).fit,
            LabelFit::WithoutCount
        );
    }

    #[test]
    fn a_name_of_64_bytes_lays_out_at_every_size() {
        let ascii = "n".repeat(64);
        let accented = "é".repeat(32);
        let cjk = format!("{}ab", "日".repeat(20)); // 62 bytes
        for name in [ascii, accented, cjk] {
            assert!(name.len() <= 64);
            for extent in [1.0, 20.0, 60.0, 120.0, 300.0, 355.0] {
                for key in [None, Some('N')] {
                    let layout = lay_out(&group(100.0, 100.0 + extent), &request(&name, key, 64));
                    assert!(layout.glyphs.len() <= name.chars().count() + 4);
                    for glyph in &layout.glyphs {
                        assert_close(distance_to_centre(glyph), radius(), 1e-3);
                    }
                }
            }
        }
        // 64 letters fit a 355-degree label whole (name, two spaces, "64")...
        let name = "n".repeat(64);
        let layout = lay_out(&group(2.0, 357.0), &request(&name, None, 64));
        assert_eq!(layout.fit, LabelFit::Full);
        assert_eq!(layout.glyphs.len(), 64 + 2);
        // ... and are cut on a short one.
        let layout = lay_out(&wide_group(30.0), &request(&name, None, 64));
        assert_eq!(layout.fit, LabelFit::EllipsisName);
    }

    #[test]
    fn an_empty_name_still_labels_the_count_or_the_key() {
        let layout = lay_out(&wide_group(30.0), &request("", None, 3));
        assert_eq!(layout.fit, LabelFit::Full);
        assert_eq!(text_of(&layout), "3");
        let layout = lay_out(&wide_group(30.0), &request("", Some('K'), 3));
        assert_eq!(layout.fit, LabelFit::Full);
        assert_eq!(text_of(&layout), "⌥K3");
        // Keyless, nothing but the count to show and no room for it: nothing at all.
        let digit = width("3", 11.0, 400);
        let layout = lay_out(&group_with_room(30.0, digit - 0.5), &request("", None, 3));
        assert_eq!(layout.fit, LabelFit::Hidden);
        // White space alone is no name.
        let layout = lay_out(&wide_group(30.0), &request("   ", None, 3));
        assert_eq!(text_of(&layout), "3");
        let layout = lay_out(&wide_group(30.0), &request("", None, 0));
        assert_eq!(text_of(&layout), "0");
    }

    #[test]
    fn a_key_that_cannot_be_drawn_counts_as_no_key() {
        for key in [' ', '\t', '\u{301}'] {
            assert_eq!(
                lay_out(&wide_group(30.0), &request("Home", Some(key), 2)),
                lay_out(&wide_group(30.0), &request("Home", None, 2)),
                "key {key:?}"
            );
        }
        // A lower-case key is drawn as given.
        let layout = lay_out(&wide_group(30.0), &request("Home", Some('k'), 2));
        assert_eq!(texts_with_role(&layout, GlyphRole::Key), "⌥k");
    }

    // --- degenerate input ---

    #[test]
    fn a_zero_length_arc_is_hidden() {
        let layout = lay_out(&group(40.0, 40.0), &request("Home", Some('H'), 2));
        assert_eq!(layout.fit, LabelFit::Hidden);
        assert!(layout.glyphs.is_empty());
        assert!(!layout.flipped);
        // A reversed extent, an arc shorter than the 12 px margin, and a group with no tabs.
        for target in [group(80.0, 40.0), group(40.0, 42.0), group(0.0, 0.0)] {
            assert_eq!(
                lay_out(&target, &request("Home", Some('H'), 2)).fit,
                LabelFit::Hidden
            );
        }
        let empty = GroupArc {
            tab_count: 0,
            ..group(0.0, 0.0)
        };
        assert_eq!(
            lay_out(&empty, &request("Home", None, 0)).fit,
            LabelFit::Hidden
        );
    }

    #[test]
    fn hidden_still_reports_the_reading_direction() {
        let layout = lay_out(&group(190.0, 190.0), &request("Home", None, 2));
        assert_eq!(layout.fit, LabelFit::Hidden);
        assert!(layout.flipped);
    }

    #[test]
    fn non_finite_input_never_panics_or_yields_nan() {
        let bad_groups = [
            group(f32::NAN, 40.0),
            group(0.0, f32::NAN),
            group(0.0, f32::INFINITY),
            group(f32::NEG_INFINITY, 0.0),
        ];
        for target in &bad_groups {
            let layout = lay_out(target, &request("Home", Some('H'), 2));
            assert_eq!(layout.fit, LabelFit::Hidden);
            assert!(layout.glyphs.is_empty());
        }
        let backwards = RingSpec {
            radius: -500.0,
            ..spec()
        };
        let layout = label_glyphs(
            &wide_group(30.0),
            &backwards,
            &request("Home", None, 2),
            &ApproxMetrics,
        );
        assert_eq!(layout.fit, LabelFit::Hidden);
    }

    #[test]
    fn broken_metrics_are_treated_as_zero_width() {
        for value in [f32::NAN, -5.0, f32::INFINITY, 0.0] {
            let layout = label_glyphs(
                &wide_group(30.0),
                &spec(),
                &request("Home", Some('H'), 2),
                &Constant(value),
            );
            assert_eq!(layout.fit, LabelFit::Full);
            assert_eq!(layout.glyphs.len(), 4 + 2 + 1);
            for glyph in &layout.glyphs {
                assert!(glyph.centre.x.is_finite() && glyph.centre.y.is_finite());
                assert!(glyph.angle_deg.is_finite());
            }
        }
    }

    #[test]
    fn layout_is_deterministic() {
        let target = group_with_room(123.0, 77.0);
        let req = request("Lisbon trip", Some('L'), 12);
        assert_eq!(lay_out(&target, &req), lay_out(&target, &req));
    }

    // --- combining marks ---

    #[test]
    fn a_combining_mark_attaches_to_the_previous_glyph_without_advance() {
        let plain = lay_out(&wide_group(30.0), &request("ea", None, 1));
        let accented = lay_out(&wide_group(30.0), &request("e\u{301}a", None, 1));
        assert_eq!(accented.glyphs.len(), 3, "e and its mark, a, the count");
        assert_eq!(accented.glyphs[0].text, "e\u{301}");
        assert_eq!(accented.glyphs[1].text, "a");
        for (with_mark, without) in accented.glyphs.iter().zip(&plain.glyphs) {
            assert_close(with_mark.centre.x, without.centre.x, 1e-4);
            assert_close(with_mark.centre.y, without.centre.y, 1e-4);
        }
        // Two marks on one base, and an emoji followed by its variation selector.
        let layout = lay_out(&wide_group(30.0), &request("e\u{301}\u{302}x", None, 1));
        assert_eq!(layout.glyphs[0].text, "e\u{301}\u{302}");
        let layout = lay_out(&wide_group(30.0), &request("\u{2764}\u{FE0F}", None, 1));
        assert_eq!(layout.glyphs[0].text, "\u{2764}\u{FE0F}");
        assert_eq!(layout.glyphs.len(), 2);
    }

    #[test]
    fn a_mark_with_nothing_to_attach_to_is_dropped() {
        let layout = lay_out(&wide_group(30.0), &request("\u{301}ab", None, 1));
        assert_eq!(text_of(&layout), "ab1");
        let layout = lay_out(&wide_group(30.0), &request("a \u{301}b", None, 1));
        assert_eq!(text_of(&layout), "ab1");
    }

    #[test]
    fn a_cut_never_separates_a_mark_from_its_base() {
        let name = "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}";
        for room in [14.0, 20.0, 30.0, 40.0, 50.0] {
            let layout = lay_out(&group_with_room(30.0, room), &request(name, None, 1));
            for glyph in &layout.glyphs {
                let mut chars = glyph.text.chars();
                let first = chars.next().unwrap();
                assert!(
                    first == 'e' || first == '…' || first == '1',
                    "{:?}",
                    glyph.text
                );
                if first == 'e' {
                    assert_eq!(chars.as_str(), "\u{301}");
                }
            }
        }
    }

    // --- the metrics table ---

    #[test]
    fn the_metrics_table_values_are_pinned() {
        let em = |ch: char, weight: u16| adv(ch, 100.0, weight) / 100.0;
        for (ch, regular, heavy) in [
            ('i', 0.41, 0.44),
            (' ', 0.41, 0.44),
            ('.', 0.41, 0.44),
            ('t', 0.41, 0.44),
            ('m', 0.85, 0.92),
            ('W', 0.85, 0.92),
            ('@', 0.85, 0.92),
            ('…', 0.85, 0.92),
            ('7', 0.56, 0.60),
            ('K', 0.66, 0.72),
            ('É', 0.66, 0.72),
            ('o', 0.59, 0.64),
            ('⌥', 0.59, 0.64),
            ('日', 1.0, 1.0),
            ('한', 1.0, 1.0),
            ('😀', 1.0, 1.0),
            ('\u{3000}', 1.0, 1.0),
        ] {
            assert_close(em(ch, 400), regular, 1e-4);
            assert_close(em(ch, 599), regular, 1e-4);
            assert_close(em(ch, 600), heavy, 1e-4);
            assert_close(em(ch, 800), heavy, 1e-4);
        }
    }

    #[test]
    fn marks_joiners_and_controls_have_no_advance() {
        for ch in [
            '\u{300}', '\u{36F}', '\u{200C}', '\u{200D}', '\u{FE0F}', '\n', '\t', '\u{7}',
        ] {
            assert_eq!(adv(ch, 12.0, 600), 0.0, "{ch:?}");
        }
        // The neighbours of the mark range are ordinary characters.
        assert!(adv('\u{2FF}', 12.0, 600) > 0.0);
        assert!(adv('\u{370}', 12.0, 600) > 0.0);
    }

    #[test]
    fn heavier_weights_are_never_narrower_and_size_scales_linearly() {
        for ch in ['i', 'm', '7', 'K', 'o', '日', ' ', '…'] {
            assert!(adv(ch, 12.0, 700) >= adv(ch, 12.0, 400), "{ch}");
            assert_close(adv(ch, 24.0, 600), 2.0 * adv(ch, 12.0, 600), 1e-4);
        }
        assert_eq!(adv('a', 0.0, 600), 0.0);
        assert_eq!(adv('a', -3.0, 600), 0.0);
        assert_eq!(adv('a', f32::NAN, 600), 0.0);
    }

    #[test]
    fn the_table_stays_within_12_percent_of_the_mockups_flat_estimate() {
        let flat = |text: &str, size: f32, weight: u16| {
            let per_char = if weight >= 600 { 0.585 } else { 0.545 };
            text.chars().count() as f32 * size * per_char
        };
        for name in ["Research", "Lisbon trip", "Shopping", "Reading"] {
            let estimate = flat(name, 12.0, 600);
            let table = width(name, 12.0, 600);
            let error = (table - estimate).abs() / estimate;
            assert!(
                error < 0.12,
                "{name}: table {table}, flat {estimate}, error {error}"
            );
        }
        for digits in ["5", "10", "1000"] {
            let estimate = flat(digits, 11.0, 400);
            let table = width(digits, 11.0, 400);
            assert!((table - estimate).abs() / estimate < 0.12, "{digits}");
        }
        // Unlike the flat estimate, the table tells narrow letters from wide ones.
        assert!(width("Illiit", 12.0, 600) < width("Mmwwmm", 12.0, 600) / 1.8);
    }

    #[test]
    fn the_glyph_metrics_trait_works_through_a_trait_object() {
        let metrics: &dyn GlyphMetrics = &ApproxMetrics;
        let layout = label_glyphs(
            &wide_group(30.0),
            &spec(),
            &request("Home", Some('H'), 2),
            metrics,
        );
        assert_eq!(layout.fit, LabelFit::Full);
    }
}
