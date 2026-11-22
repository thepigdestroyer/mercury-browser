// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Where the dial sits and which corner a release snaps to (SPEC §4.5): the snapping maths.
//!
//! Everything here is total: a width or height that is negative or NaN counts as 0, an infinite one
//! as the largest finite value, so no function returns NaN or a negative inset for any area.
//!
//! - **Position.** The dial's centre rests [`DialGeometry::inset`] (74 px: the dial's radius 36 plus
//!   the 38 px margin of the mockup) from the area's edges, but never more than half the area. An
//!   area under 148 px in a direction therefore pulls the centres of opposite corners together; at
//!   148 px and below they meet in the middle, and none crosses another.
//! - **Snapping.** A release goes to the corner whose resting centre is nearest by Euclidean
//!   distance. A tie (two or more centres within `1e-4` px of the nearest) keeps `current`, so a
//!   release exactly on an axis of the area changes nothing; a release that is NaN or infinite also
//!   keeps `current`. On an area of 148 px or more the four centres form a rectangle, so the nearest
//!   centre is the quadrant of the area's middle that the release lies in.
//! - **Dragging.** [`DialGeometry::clamp_drag`] keeps the dial's centre within `[36, width - 36]` by
//!   `[36, height - 36]`, so the 72 px disc never leaves the area; where the area is under 72 px in a
//!   direction the centre is the middle of that direction. A NaN coordinate becomes the middle of
//!   the area in that direction; an infinite one is clamped to the edge like any far point.
//! - **Fan.** [`fan_start_deg`] turns the bottom-right fan (the mockup's) a quarter turn per
//!   corner, so the four fans are congruent and the tabs run clockwise in each.

use mercury_settings::DialCorner;

use crate::ring_geometry::{Point, RingSpec, point_on_circle};

use super::{DIAL_DIAMETER, DIAL_MARGIN, FAN_BOX, FAN_REACH};

/// Two resting centres that are this close (in px) to the nearest one are a tie.
const SNAP_TIE: f64 = 1e-4;

/// The angle the fan covers (`RingSpec::dial_fan`), in degrees.
const FAN_SPAN_DEG: f32 = 98.0;

/// The overlay area the dial lives in (the page area: below the toolbar, above the dock's band),
/// in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialGeometry {
    pub width: f32,
    pub height: f32,
}

/// `v` as a length: NaN and negative values are 0, infinity is the largest finite value.
fn length(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, f32::MAX)
    }
}

impl DialGeometry {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    fn size(&self) -> (f32, f32) {
        (length(self.width), length(self.height))
    }

    /// Distance from the dial's centre to the area's edges: `DIAL_DIAMETER / 2 + DIAL_MARGIN` (74),
    /// never more than half the area.
    pub fn inset(&self) -> f32 {
        let (w, h) = self.size();
        (DIAL_DIAMETER / 2.0 + DIAL_MARGIN)
            .min(w / 2.0)
            .min(h / 2.0)
    }

    /// The resting centre of the dial in `corner`.
    pub fn centre(&self, corner: DialCorner) -> Point {
        let (w, h) = self.size();
        let inset = self.inset();
        let (left, top) = (inset, inset);
        let (right, bottom) = (w - inset, h - inset);
        match corner {
            DialCorner::TopLeft => Point::new(left, top),
            DialCorner::TopRight => Point::new(right, top),
            DialCorner::BottomLeft => Point::new(left, bottom),
            DialCorner::BottomRight => Point::new(right, bottom),
        }
    }

    /// The corner a release at `release` snaps to: the nearest resting centre. When two or more are
    /// equally near (within `SNAP_TIE`: a release on an axis, or centres that coincide in a small
    /// area) `current` is kept if it is one of them, else the first of them in the order top-left,
    /// top-right, bottom-left, bottom-right.
    pub fn snap(&self, release: Point, current: DialCorner) -> DialCorner {
        if !release.x.is_finite() || !release.y.is_finite() {
            return current;
        }
        let distance = |corner: DialCorner| {
            let c = self.centre(corner);
            (f64::from(release.x) - f64::from(c.x)).hypot(f64::from(release.y) - f64::from(c.y))
        };
        let distances = CORNERS.map(|corner| (corner, distance(corner)));
        let best = distances
            .iter()
            .map(|&(_, d)| d)
            .fold(f64::INFINITY, f64::min);
        let mut near = distances
            .iter()
            .filter(|&&(_, d)| d - best <= SNAP_TIE)
            .map(|&(corner, _)| corner);
        let first = near.next().unwrap_or(current);
        if first == current || near.any(|corner| corner == current) {
            current
        } else {
            first
        }
    }

    /// `p` moved to the nearest point where the whole dial is inside the area.
    pub fn clamp_drag(&self, p: Point) -> Point {
        let (w, h) = self.size();
        Point::new(clamp_axis(p.x, w / 2.0, w), clamp_axis(p.y, h / 2.0, h))
    }

    /// Whether the whole fan of `corner` (`FAN_REACH` from the centre, its 98 degrees only) lies
    /// inside the area.
    ///
    /// The arc's extremes are its two end points and the axis directions (0, 90, 180 and 270
    /// degrees) that fall inside its span; the axis points are exact (`centre ± FAN_REACH`).
    pub fn fan_fits(&self, corner: DialCorner) -> bool {
        let (w, h) = self.size();
        let centre = self.centre(corner);
        let start = fan_start_deg(corner);
        let end = start + FAN_SPAN_DEG;
        let inside = |p: Point| p.x >= 0.0 && p.x <= w && p.y >= 0.0 && p.y <= h;
        if !inside(point_on_circle(centre, FAN_REACH, start))
            || !inside(point_on_circle(centre, FAN_REACH, end))
        {
            return false;
        }
        (0..=6).all(|quarter| {
            let axis = quarter as f32 * 90.0;
            if axis < start || axis > end {
                return true;
            }
            let p = match quarter % 4 {
                0 => Point::new(centre.x, centre.y - FAN_REACH),
                1 => Point::new(centre.x + FAN_REACH, centre.y),
                2 => Point::new(centre.x, centre.y + FAN_REACH),
                _ => Point::new(centre.x - FAN_REACH, centre.y),
            };
            inside(p)
        })
    }
}

const CORNERS: [DialCorner; 4] = [
    DialCorner::TopLeft,
    DialCorner::TopRight,
    DialCorner::BottomLeft,
    DialCorner::BottomRight,
];

/// `v` kept within `[DIAL_DIAMETER / 2, extent - DIAL_DIAMETER / 2]`; `middle` where that is empty
/// or `v` is NaN.
fn clamp_axis(v: f32, middle: f32, extent: f32) -> f32 {
    let (lo, hi) = (DIAL_DIAMETER / 2.0, extent - DIAL_DIAMETER / 2.0);
    if v.is_nan() || hi < lo {
        middle
    } else {
        v.clamp(lo, hi)
    }
}

/// Where the fan of `corner` starts, in degrees clockwise from 12 o'clock: bottom-right 267 (the
/// mockup), bottom-left 357, top-left 87, top-right 177 (the same shape turned a quarter turn).
pub fn fan_start_deg(corner: DialCorner) -> f32 {
    match corner {
        DialCorner::BottomRight => 267.0,
        DialCorner::BottomLeft => 357.0,
        DialCorner::TopLeft => 87.0,
        DialCorner::TopRight => 177.0,
    }
}

/// `RingSpec::dial_fan` centred in the `RingBands` box (`FAN_BOX / 2`) and started per corner.
pub fn fan_spec(corner: DialCorner) -> RingSpec {
    RingSpec::dial_fan(
        Point::new(FAN_BOX / 2.0, FAN_BOX / 2.0),
        fan_start_deg(corner),
    )
}
