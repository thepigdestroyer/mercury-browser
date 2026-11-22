// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The pinned tabs as beads in the reserve at 12 o'clock (SPEC §4.3, brief D3 and D4). Pure
//! geometry: where each bead sits, how many fit, the "+N" chip. The mockup (`ring-open`): beads of
//! diameter 20, centred on the radius-180 circle, 20.1 px apart, symmetric about 12 o'clock.
//!
//! # Capacity
//!
//! The reserve is an arc of `reserve_deg` degrees centred on 12 o'clock. Its length at `radius` is
//! `arc = reserve_deg.to_radians() * radius + BEAD_SIZE - BEAD_PITCH` (the extra
//! `BEAD_SIZE - BEAD_PITCH` is the 0.1 px of slack between two beads that touch: `n` beads need
//! `(n - 1) * BEAD_PITCH` between the first and the last centre, plus the bead itself, so `n` beads
//! occupy `n * BEAD_PITCH - 0.1`). A margin of 2 px is kept, so
//! `capacity = floor((arc - 2) / BEAD_PITCH)`, never below 0. At radius 180 the capacity steps to
//! 1, 2, 3 and 4 at reserves of 7.07, 13.46, 19.86 and 26.26 degrees; the mockup's 24 degrees
//! gives 3, and 0 degrees gives 0.
//!
//! # Positions
//!
//! Centres lie on the circle of `radius` round the ring's centre, mirrored about 12 o'clock, and
//! neighbouring centres are exactly [`BEAD_PITCH`] apart measured along the **chord** (a straight
//! line), so two beads never overlap; the angle between two neighbours is
//! `2 * asin(BEAD_PITCH / (2 * radius))` (11.17 degrees at 180). On the mockup this is
//! (317.94, 163.12), (338, 162), (358.06, 163.12) round the centre (338, 342).
//! [`BeadSpot::x`] and [`BeadSpot::y`] are the **top-left corner** of the bead's `size` square, like
//! a Slint element's `x` and `y`; [`BeadSpot::centre`] is the middle.

use slint::{Color, Image, SharedString};

use crate::{BeadRow, TabRow};

/// Bead diameter in px.
pub const BEAD_SIZE: f32 = 20.0;
/// Centre-to-centre distance of two neighbouring beads (mockup).
pub const BEAD_PITCH: f32 = 20.1;
/// Room kept free on the reserve's arc.
const ARC_MARGIN: f32 = 2.0;

/// One pinned tab, from its row.
#[derive(Clone, Debug, PartialEq)]
pub struct PinnedBead {
    pub tab: i32,
    pub title: String,
    /// The first letter of the host (else of the title), upper case.
    pub letter: String,
    /// The tab's `shade`.
    pub mark: Color,
    pub favicon: Image,
    pub has_favicon: bool,
}

/// The first alphanumeric character of `text`, upper-cased to one character.
fn initial(text: &str) -> Option<char> {
    let c = text.chars().find(|c| c.is_alphanumeric())?;
    Some(c.to_uppercase().next().unwrap_or(c))
}

impl PinnedBead {
    /// From a row (`pinned` is not checked). The letter is the first letter or digit of the host,
    /// else of the title, else "?".
    pub fn from_row(row: &TabRow) -> Self {
        let letter = initial(row.host.as_str())
            .or_else(|| initial(row.title.as_str()))
            .unwrap_or('?');
        Self {
            tab: row.id,
            title: row.title.to_string(),
            letter: letter.to_string(),
            mark: row.shade,
            favicon: row.favicon.clone(),
            has_favicon: row.favicon.size().width > 0,
        }
    }
}

/// Where one drawn bead sits (ring-local) and what it stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeadSpot {
    /// The pinned tab; for the "+N" chip the first hidden one. [`layout`] has no ids and gives the
    /// index of the pinned tab instead; [`layout_for`] gives the tab id.
    pub tab: i32,
    /// The top-left corner of the bead's box.
    pub x: f32,
    pub y: f32,
    pub size: f32,
    /// > 0: this is the chip standing for `more` beads.
    pub more: usize,
}

impl BeadSpot {
    /// The middle of the bead.
    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.size / 2.0, self.y + self.size / 2.0)
    }
}

/// The bead layout of one ring.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BeadLayout {
    pub spots: Vec<BeadSpot>,
    /// How many beads fit on the reserve (0: none; the centre footer names the pinned tabs).
    pub capacity: usize,
    /// Pinned tabs not drawn as a bead of their own (behind the chip, or all of them at capacity 0).
    pub hidden: usize,
}

/// How many beads fit on a reserve of `reserve_deg` degrees at `radius` (see the module docs).
fn capacity(reserve_deg: f32, radius: f32) -> usize {
    if !reserve_deg.is_finite() || !radius.is_finite() || reserve_deg <= 0.0 || radius <= 0.0 {
        return 0;
    }
    let arc = reserve_deg.min(360.0).to_radians() * radius + BEAD_SIZE - BEAD_PITCH;
    let fit = ((arc - ARC_MARGIN) / BEAD_PITCH).floor();
    if fit >= 1.0 { fit as usize } else { 0 }
}

/// The spots for `drawn` slots (the last being a chip when `chip_more > 0`), first slot's `tab`
/// given by `tab_of(slot)`.
fn spots(
    drawn: usize,
    chip_more: usize,
    radius: f32,
    centre: (f32, f32),
    tab_of: &dyn Fn(usize) -> i32,
) -> Vec<BeadSpot> {
    let step_deg = 2.0
        * (BEAD_PITCH / (2.0 * radius))
            .clamp(0.0, 1.0)
            .asin()
            .to_degrees();
    (0..drawn)
        .map(|slot| {
            let from_middle = slot as f32 - (drawn as f32 - 1.0) / 2.0;
            let rad = (from_middle * step_deg).to_radians();
            let cx = centre.0 + radius * rad.sin();
            let cy = centre.1 - radius * rad.cos();
            BeadSpot {
                tab: tab_of(slot),
                x: cx - BEAD_SIZE / 2.0,
                y: cy - BEAD_SIZE / 2.0,
                size: BEAD_SIZE,
                more: if chip_more > 0 && slot + 1 == drawn {
                    chip_more
                } else {
                    0
                },
            }
        })
        .collect()
}

fn layout_with(
    count: usize,
    reserve_deg: f32,
    radius: f32,
    centre: (f32, f32),
    tab_of: &dyn Fn(usize) -> i32,
) -> BeadLayout {
    let capacity = capacity(reserve_deg, radius);
    if capacity == 0 {
        return BeadLayout {
            spots: Vec::new(),
            capacity,
            hidden: count,
        };
    }
    if count <= capacity {
        return BeadLayout {
            spots: spots(count, 0, radius, centre, tab_of),
            capacity,
            hidden: 0,
        };
    }
    // Over capacity: `capacity - 1` beads, then the chip for the rest (a single chip when only
    // one slot fits).
    let more = count - (capacity - 1);
    BeadLayout {
        spots: spots(capacity, more, radius, centre, tab_of),
        capacity,
        hidden: more,
    }
}

/// Lays out `count` beads in a reserve of `reserve_deg` degrees centred on 12 o'clock on the circle
/// of `radius` round `centre`. `capacity` = beads of [`BEAD_PITCH`] that fit in the arc (24 degrees
/// at 180 gives 3); over capacity the last slot is the chip. Spot `tab` is the index of the pinned
/// tab (the chip: the index of the first hidden one); use [`layout_for`] for tab ids.
pub fn layout(count: usize, reserve_deg: f32, radius: f32, centre: (f32, f32)) -> BeadLayout {
    layout_with(count, reserve_deg, radius, centre, &|slot| slot as i32)
}

/// [`layout`] for these pinned tabs: the spots' `tab` are the real tab ids, and the chip's is the
/// first hidden bead's.
pub fn layout_for(
    pinned: &[PinnedBead],
    reserve_deg: f32,
    radius: f32,
    centre: (f32, f32),
) -> BeadLayout {
    layout_with(pinned.len(), reserve_deg, radius, centre, &|slot| {
        pinned.get(slot).map_or(-1, |bead| bead.tab)
    })
}

/// The Slint rows for a layout: `pinned[i]` is the i-th pinned tab; `focused` is the focused bead's
/// tab id; `dimmed(tab)` is true for a bead that does not match the query.
///
/// The i-th plain spot is `pinned[i]`; the chip (the last spot, `more > 0`) stands for
/// `pinned[i..]`: its `tab` is the first of them, its `letter` is "+N", it is `focused` when the
/// focus is on any of them and `dimmed` when all of them are. `x` and `y` are the spot's top-left
/// corner.
pub fn rows(
    layout: &BeadLayout,
    pinned: &[PinnedBead],
    focused: Option<i32>,
    dimmed: &dyn Fn(i32) -> bool,
) -> Vec<BeadRow> {
    layout
        .spots
        .iter()
        .enumerate()
        .filter_map(|(slot, spot)| {
            if spot.more > 0 {
                let hidden = pinned.get(slot..)?;
                let first = hidden.first()?;
                return Some(BeadRow {
                    tab: first.tab,
                    title: SharedString::from(format!("{} more pinned tabs", spot.more)),
                    letter: SharedString::from(format!("+{}", spot.more)),
                    mark: Color::default(),
                    favicon: Image::default(),
                    has_favicon: false,
                    x: spot.x,
                    y: spot.y,
                    size: spot.size,
                    focused: focused.is_some_and(|id| hidden.iter().any(|bead| bead.tab == id)),
                    dimmed: hidden.iter().all(|bead| dimmed(bead.tab)),
                    more: i32::try_from(spot.more).unwrap_or(i32::MAX),
                });
            }
            let bead = pinned.get(slot)?;
            Some(BeadRow {
                tab: bead.tab,
                title: SharedString::from(bead.title.as_str()),
                letter: SharedString::from(bead.letter.as_str()),
                mark: bead.mark,
                favicon: bead.favicon.clone(),
                has_favicon: bead.has_favicon,
                x: spot.x,
                y: spot.y,
                size: spot.size,
                focused: focused == Some(bead.tab),
                dimmed: dimmed(bead.tab),
                more: 0,
            })
        })
        .collect()
}
