// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The pinned tabs as beads in the reserve at 12 o'clock (SPEC §4.3, brief D3 and D4). Pure
//! geometry: where each bead sits, how many fit, the "+N" chip. The mockup (`ring-open`): beads of
//! diameter 20, centred on the radius-180 circle, 20.1 px apart, symmetric about 12 o'clock.
//!
//! STUB in the scaffold commit (types and signatures are the contract; item 2 `nav-beads` writes
//! the bodies).

use slint::{Color, Image};

use crate::{BeadRow, TabRow};

/// Bead diameter in px.
pub const BEAD_SIZE: f32 = 20.0;
/// Centre-to-centre distance of two neighbouring beads (mockup).
pub const BEAD_PITCH: f32 = 20.1;

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

impl PinnedBead {
    /// From a row (`pinned` is not checked).
    pub fn from_row(row: &TabRow) -> Self {
        Self {
            tab: row.id,
            title: row.title.to_string(),
            letter: String::new(),
            mark: row.shade,
            favicon: row.favicon.clone(),
            has_favicon: false,
        }
    }
}

/// Where one drawn bead sits (ring-local) and what it stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeadSpot {
    /// The pinned tab; for the "+N" chip the first hidden one.
    pub tab: i32,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    /// > 0: this is the chip standing for `more` beads.
    pub more: usize,
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

/// Lays out `count` beads in a reserve of `reserve_deg` degrees centred on 12 o'clock on the circle
/// of `radius` round `centre`. `capacity` = beads of [`BEAD_PITCH`] that fit in the arc (24 degrees
/// at 180 gives 3); over capacity the last slot is the chip.
pub fn layout(_count: usize, _reserve_deg: f32, _radius: f32, _centre: (f32, f32)) -> BeadLayout {
    BeadLayout::default()
}

/// The Slint rows for a layout: `pinned[i]` is the i-th pinned tab; `focused` is the focused bead's
/// tab id; `dimmed(tab)` is true for a bead that does not match the query.
pub fn rows(
    _layout: &BeadLayout,
    _pinned: &[PinnedBead],
    _focused: Option<i32>,
    _dimmed: &dyn Fn(i32) -> bool,
) -> Vec<BeadRow> {
    Vec::new()
}
