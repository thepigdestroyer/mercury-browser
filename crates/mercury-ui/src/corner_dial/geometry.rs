// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Where the dial sits and which corner a release snaps to (SPEC §4.5): the snapping maths.
//! STUB in the scaffold commit: item 1 `geometry` of lane corner-dial writes the real bodies. The
//! signatures are the pinned contract.

use mercury_settings::DialCorner;

use crate::ring_geometry::{Point, RingSpec};

use super::{DIAL_DIAMETER, DIAL_MARGIN, FAN_BOX};

/// The overlay area the dial lives in (the page area: below the toolbar, above the dock's band),
/// in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialGeometry {
    pub width: f32,
    pub height: f32,
}

impl DialGeometry {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Distance from the dial's centre to the area's edges: `DIAL_DIAMETER / 2 + DIAL_MARGIN` (74),
    /// never more than half the area.
    pub fn inset(&self) -> f32 {
        DIAL_DIAMETER / 2.0 + DIAL_MARGIN
    }

    /// The resting centre of the dial in `corner`.
    pub fn centre(&self, _corner: DialCorner) -> Point {
        Point::new(0.0, 0.0)
    }

    /// The corner a release at `release` snaps to: the nearest resting centre; a tie keeps `current`.
    pub fn snap(&self, _release: Point, current: DialCorner) -> DialCorner {
        current
    }

    /// `p` moved to the nearest point where the whole dial is inside the area.
    pub fn clamp_drag(&self, p: Point) -> Point {
        p
    }

    /// Whether the whole fan of `corner` (`FAN_REACH` from the centre, its 98 degrees only) lies
    /// inside the area.
    pub fn fan_fits(&self, _corner: DialCorner) -> bool {
        false
    }
}

/// Where the fan of `corner` starts, in degrees clockwise from 12 o'clock: bottom-right 267 (the
/// mockup), bottom-left 357, top-left 87, top-right 177 (the same shape turned a quarter turn).
pub fn fan_start_deg(_corner: DialCorner) -> f32 {
    0.0
}

/// `RingSpec::dial_fan` centred in the `RingBands` box (`FAN_BOX / 2`) and started per corner.
pub fn fan_spec(_corner: DialCorner) -> RingSpec {
    RingSpec::dial_fan(Point::new(FAN_BOX / 2.0, FAN_BOX / 2.0), 0.0)
}
