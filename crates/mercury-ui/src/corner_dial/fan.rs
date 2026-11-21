// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The data of the dial's fan: `RingBandsData` with the corner's `RingSpec`, where its box sits and
//! the hover caption. STUB in the scaffold commit: item 1 `geometry` of lane corner-dial writes the
//! real bodies. The signatures are the pinned contract.

use mercury_settings::DialCorner;

use crate::ring_bands::{RingBandsData, RingOptions};
use crate::ring_geometry::{GlyphMetrics, Point};
use crate::{DialCaption, GroupRow, TabRow};

use super::geometry::{DialGeometry, fan_spec};

/// Everything the fan of one corner needs.
#[derive(Clone, Debug, PartialEq)]
pub struct DialFan {
    pub corner: DialCorner,
    pub data: RingBandsData,
    /// The top-left corner of the `RingBands` box in the overlay area (the dial's centre minus
    /// `FAN_BOX / 2` in both directions).
    pub origin: Point,
}

impl DialFan {
    pub fn new(
        geometry: &DialGeometry,
        corner: DialCorner,
        groups: &[GroupRow],
        tabs: &[TabRow],
        opts: &RingOptions,
        metrics: &dyn GlyphMetrics,
    ) -> Self {
        let _ = geometry;
        Self {
            corner,
            data: RingBandsData::new(fan_spec(corner), groups, tabs, opts, metrics),
            origin: Point::new(0.0, 0.0),
        }
    }

    /// "Group · Title" at the hovered segment, 24 px toward the dial, clamped into the fan's box;
    /// `None` when no shown tab has the id `hovered_tab`.
    pub fn caption(&self, _hovered_tab: i32) -> Option<DialCaption> {
        None
    }
}
