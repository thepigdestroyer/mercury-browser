// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The two Orbit layouts (SPEC §4.6): where the group planets sit at level 1 and where the tab
//! previews sit at level 2. Pure functions of a count and an ellipse; every position is
//! **relative to the ellipse's centre**, in logical pixels, y down. The Orbit lane adds the
//! centre of its stage.
//!
//! The ellipses are given by their **half-axes**: the mockup's level 1 is
//! `Ellipse { rx: 470.0, ry: 290.0 }` (the drawn ellipse is 940 px wide) and level 2 is
//! `Ellipse { rx: 520.0, ry: 292.0 }`.
//!
//! # Planets (level 1)
//!
//! Planet `i` of a ring of `m` sits at parametric angle `360 * i / m` clockwise from the top,
//! at `(rx sin t, -ry cos t)`; group 0 is at the top for every group count, and 5 groups give
//! the mockup's 0, 72, 144, 216 and 288 degrees. The planet's disc has the diameter
//! `scale * PLANET_DIAMETER`. For up to nine groups on the mockup ellipse the scale is 1.0 and
//! there is one ring. More groups first shrink the planets (down to what one ring holds), and
//! when inner rings let the planets stay larger, those are added:
//!
//! - **spacing** `D = scale * 170 + 16`: the least distance between two planet centres, so
//!   neighbouring discs keep 16 px between them;
//! - **rings**: ring `k` is the given ellipse scaled so that its smaller half-axis is
//!   `q - k * D`, where `q = min(rx, ry)`, i.e. by the factor `1 - k * D / q` (outer ring
//!   `k = 0`: factor 1.0). Two similar nested ellipses are at least the difference of their
//!   smaller half-axes apart, so planets of different rings never come closer than `D`. A ring
//!   exists only while its smaller half-axis is at least `SUN_KEEP_OUT + scale * 85`, so a disc
//!   never reaches into the sun's glow (this holds for the outer ring too: a small window
//!   shrinks the planets even for one group);
//! - **capacity** of a ring with smaller half-axis `b`: the most planets `m` with
//!   `2 b sin(pi / m) >= D` (the chord of the regular polygon, a lower bound of the distance
//!   between any two planets of the ring, whatever the ellipse's other half-axis);
//! - **scale**: the largest of 1.00, 0.99, ... 0.25 at which the rings together hold all
//!   groups. The scale never grows when groups are added. Beyond the capacity at 0.25 (73
//!   groups on the mockup ellipse, the product allows 64) the planets overlap, they are still
//!   placed on valid positions without NaN and without panics;
//! - **sharing out**: groups are shared over the rings in proportion to their capacity (largest
//!   remainder, ties to the outer ring), in index order from the outer ring in, so each ring
//!   reads clockwise by index. Inner rings start half a step round so that their planets sit
//!   between those of the ring outside.
//!
//! # Previews (level 2)
//!
//! Preview `i` of a ring of `m` sits at `360 * i / m` clockwise from the top. The first
//! `PREVIEWS_PER_RING` tabs (most recent first) form the outer ring on the ellipse itself; the
//! next up to `PREVIEWS_PER_RING` tabs form an inner ring on the ellipse scaled by
//! `INNER_PREVIEW_FACTOR`, turned half a step. At most `MAX_PREVIEWS` points are returned: the
//! Orbit lane shows "+N more" for `n_tabs - points.len()`. Cards of the inner ring are meant
//! to be drawn at the same factor and before the outer ring's: they then clear each other and
//! overlap an outer card by under 5 px at the worst (see the tests).

use super::types::{Ellipse, Point};
use std::f32::consts::PI;

/// Diameter of a planet's ring at scale 1.0 (the mockup's ring is about 167-170 px).
pub const PLANET_DIAMETER: f32 = 170.0;
/// Radius around the sun's centre that no planet disc may reach (the sun's glow).
pub const SUN_KEEP_OUT: f32 = 110.0;
/// Tab previews on one ellipse.
pub const PREVIEWS_PER_RING: usize = 12;
/// Tab previews shown in all (two ellipses); further tabs are "+N more".
pub const MAX_PREVIEWS: usize = 2 * PREVIEWS_PER_RING;
/// The smallest planet scale.
pub const MIN_PLANET_SCALE: f32 = 0.25;
/// Space kept between two neighbouring planet discs.
pub const PLANET_GAP: f32 = 16.0;
/// The inner preview ellipse relative to the outer one (and the size its cards are meant to have).
pub const INNER_PREVIEW_FACTOR: f32 = 0.58;

/// Where the planets sit, and how large they are drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanetLayout {
    /// The centre of every planet, in group order, relative to the ellipse's centre.
    pub centres: Vec<Point>,
    /// Multiplies `PLANET_DIAMETER` (and everything drawn on the planet): 1.0 for few groups.
    pub scale: f32,
}

/// The planets of `n_groups` groups on `ellipse` (half-axes). See the module documentation for
/// the rules; no groups give no centres and scale 1.0, one group sits at the top.
///
/// A NaN, infinite, zero or negative half-axis counts as 0 and puts every planet at the centre.
pub fn planet_layout(n_groups: usize, ellipse: Ellipse) -> PlanetLayout {
    let plan = planet_plan(n_groups, ellipse);
    PlanetLayout {
        centres: plan.centres(),
        scale: plan.scale,
    }
}

/// The tab previews of `n_tabs` tabs on `ellipse` (half-axes), most recent tab first, relative to
/// the ellipse's centre. At most `MAX_PREVIEWS` points; the first `PREVIEWS_PER_RING` are on the
/// ellipse, any further ones on the inner ellipse. See the module documentation.
pub fn preview_layout(n_tabs: usize, ellipse: Ellipse) -> Vec<Point> {
    let shown = n_tabs.min(MAX_PREVIEWS);
    let outer = shown.min(PREVIEWS_PER_RING);
    let inner = shown - outer;
    let (rx, ry) = (positive(ellipse.rx), positive(ellipse.ry));
    let mut points = Vec::with_capacity(shown);
    push_ring(&mut points, outer, rx, ry, 0.0);
    push_ring(
        &mut points,
        inner,
        rx * INNER_PREVIEW_FACTOR,
        ry * INNER_PREVIEW_FACTOR,
        0.5,
    );
    points
}

/// `count` points at `360 * (i + phase) / count` degrees clockwise from the top on the ellipse
/// with half-axes `rx` and `ry`.
fn push_ring(points: &mut Vec<Point>, count: usize, rx: f32, ry: f32, phase: f32) {
    for i in 0..count {
        let t = (360.0 * (i as f32 + phase) / count as f32).to_radians();
        points.push(Point::new(rx * t.sin(), -ry * t.cos()));
    }
}

/// Finite and above zero, else 0.
fn positive(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// One ring of planets after the sharing out.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlanetRing {
    /// 0 for the outer ring (the given ellipse), 1 for the next one in, ...
    index: usize,
    /// The ring's half-axes.
    half: Ellipse,
    /// Planets on this ring (at least 1).
    count: usize,
}

/// The whole decision of `planet_layout`: the scale and the rings, from the outside in.
#[derive(Clone, Debug, PartialEq)]
struct PlanetPlan {
    scale: f32,
    rings: Vec<PlanetRing>,
}

impl PlanetPlan {
    fn centres(&self) -> Vec<Point> {
        let total = self.rings.iter().map(|ring| ring.count).sum();
        let mut points = Vec::with_capacity(total);
        for ring in &self.rings {
            // Group 0 is at the top of the outer ring; an inner ring is turned half a step.
            let phase = if ring.index == 0 { 0.0 } else { 0.5 };
            push_ring(&mut points, ring.count, ring.half.rx, ring.half.ry, phase);
        }
        points
    }
}

/// A ring that fits at some scale: its smaller half-axis and how many planets it holds.
#[derive(Clone, Copy, Debug)]
struct RingSlot {
    reach: f32,
    capacity: usize,
}

fn planet_plan(n: usize, ellipse: Ellipse) -> PlanetPlan {
    if n == 0 {
        return PlanetPlan {
            scale: 1.0,
            rings: Vec::new(),
        };
    }
    let (rx, ry) = (positive(ellipse.rx), positive(ellipse.ry));
    let q = rx.min(ry);
    if q <= 0.0 {
        return PlanetPlan {
            scale: MIN_PLANET_SCALE,
            rings: vec![PlanetRing {
                index: 0,
                half: Ellipse { rx: 0.0, ry: 0.0 },
                count: n,
            }],
        };
    }
    let min_percent = (MIN_PLANET_SCALE * 100.0).round() as u32;
    let (scale, slots) = (min_percent..=100)
        .rev()
        .find_map(|percent| {
            let scale = percent as f32 / 100.0;
            let slots = ring_slots(q, scale);
            let capacity: usize = slots.iter().map(|slot| slot.capacity).sum();
            (capacity >= n).then_some((scale, slots))
        })
        // Fuller than the floor can hold: share out over the rings of the floor, overlapping.
        .unwrap_or_else(|| (MIN_PLANET_SCALE, ring_slots(q, MIN_PLANET_SCALE)));
    // A window too small even for the floor: one ring on the ellipse itself, in the keep-out.
    let slots = if slots.is_empty() {
        let spacing = scale * PLANET_DIAMETER + PLANET_GAP;
        vec![RingSlot {
            reach: q,
            capacity: ring_capacity(q, spacing),
        }]
    } else {
        slots
    };
    let capacities: Vec<usize> = slots.iter().map(|slot| slot.capacity).collect();
    let counts = share_out(n, &capacities);
    let rings = slots
        .iter()
        .zip(counts)
        .enumerate()
        .filter(|(_, (_, count))| *count > 0)
        .map(|(index, (slot, count))| {
            let factor = slot.reach / q;
            PlanetRing {
                index,
                half: Ellipse {
                    rx: rx * factor,
                    ry: ry * factor,
                },
                count,
            }
        })
        .collect();
    PlanetPlan { scale, rings }
}

/// The rings that fit at `scale`, outermost first: every ring whose smaller half-axis keeps the
/// whole disc outside the sun's keep-out. Empty when not even the given ellipse does (a window
/// too small for planets of this scale). `q` is the smaller half-axis of the given ellipse.
fn ring_slots(q: f32, scale: f32) -> Vec<RingSlot> {
    let spacing = scale * PLANET_DIAMETER + PLANET_GAP;
    let innermost = SUN_KEEP_OUT + scale * PLANET_DIAMETER / 2.0;
    let mut slots = Vec::new();
    let mut k = 0;
    loop {
        let reach = q - k as f32 * spacing;
        if reach < innermost {
            break;
        }
        slots.push(RingSlot {
            reach,
            capacity: ring_capacity(reach, spacing),
        });
        k += 1;
    }
    slots
}

/// The most planets a ring of smaller half-axis `reach` holds with their centres at least
/// `spacing` apart: the largest `m` with `2 * reach * sin(pi / m) >= spacing`, at least 1.
fn ring_capacity(reach: f32, spacing: f32) -> usize {
    let chord = |m: usize| 2.0 * reach * (PI / m as f32).sin();
    let ratio = spacing / (2.0 * reach);
    if !(ratio > 0.0 && ratio <= 1.0) {
        return 1;
    }
    let mut m = ((PI / ratio.asin()).floor() as usize).max(2);
    while m > 2 && chord(m) < spacing {
        m -= 1;
    }
    while chord(m + 1) >= spacing {
        m += 1;
    }
    m
}

/// Shares `n` over rings with the given capacities in proportion to them (largest remainder,
/// ties to the outer ring). `n` not above the total capacity never exceeds a ring's capacity.
/// The outer ring always gets at least one (group 0 is at the top).
fn share_out(n: usize, capacities: &[usize]) -> Vec<usize> {
    let total = capacities.iter().sum::<usize>().max(1) as u128;
    let exact = |capacity: usize| n as u128 * capacity as u128;
    let mut counts: Vec<usize> = capacities
        .iter()
        .map(|&capacity| (exact(capacity) / total) as usize)
        .collect();
    let rest = n - counts.iter().sum::<usize>();
    let mut by_remainder: Vec<usize> = (0..capacities.len()).collect();
    by_remainder.sort_by(|&a, &b| {
        (exact(capacities[b]) % total)
            .cmp(&(exact(capacities[a]) % total))
            .then(a.cmp(&b))
    });
    for &ring in by_remainder.iter().cycle().take(rest) {
        counts[ring] += 1;
    }
    match counts.iter().rposition(|&count| count > 0) {
        Some(donor) if counts[0] == 0 => {
            counts[donor] -= 1;
            counts[0] = 1;
        }
        _ => {}
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring_geometry::normalise_deg;

    const EPS: f32 = 1e-2;
    /// Level 1 of the mockup (half-axes).
    const MOCKUP: Ellipse = Ellipse {
        rx: 470.0,
        ry: 290.0,
    };
    /// Level 2 of the mockup (half-axes).
    const LEVEL_2: Ellipse = Ellipse {
        rx: 520.0,
        ry: 292.0,
    };
    const CARD: (f32, f32) = (190.0, 128.0);

    fn distance(a: Point, b: Point) -> f32 {
        (a.x - b.x).hypot(a.y - b.y)
    }

    fn close(a: Point, b: Point) -> bool {
        distance(a, b) <= EPS
    }

    /// The angle of `point` on an ellipse with half-axes `half`, degrees clockwise from the top.
    fn parametric_deg(point: Point, half: Ellipse) -> f32 {
        normalise_deg((point.x / half.rx).atan2(-point.y / half.ry).to_degrees())
    }

    fn angle_gap(a: f32, b: f32) -> f32 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    /// Most planets the floor scale holds on `ellipse`.
    fn floor_capacity(ellipse: Ellipse) -> usize {
        ring_slots(ellipse.rx.min(ellipse.ry), MIN_PLANET_SCALE)
            .iter()
            .map(|slot| slot.capacity)
            .sum()
    }

    /// Checks every property of a planet layout that must hold when the groups fit.
    fn check_planets(n: usize, ellipse: Ellipse) {
        let layout = planet_layout(n, ellipse);
        let plan = planet_plan(n, ellipse);
        let ctx = format!("n = {n}, ellipse {}x{}", ellipse.rx, ellipse.ry);
        assert_eq!(layout.centres.len(), n, "{ctx}");
        assert_eq!(layout.centres, plan.centres(), "{ctx}");
        assert!(
            (MIN_PLANET_SCALE..=1.0).contains(&layout.scale),
            "{ctx}: scale {}",
            layout.scale
        );
        let radius = layout.scale * PLANET_DIAMETER / 2.0;
        for (i, c) in layout.centres.iter().enumerate() {
            assert!(c.x.is_finite() && c.y.is_finite(), "{ctx}: planet {i}");
            // Inside the ellipse's bounding box.
            assert!(c.x.abs() <= ellipse.rx + EPS, "{ctx}: planet {i} x {}", c.x);
            assert!(c.y.abs() <= ellipse.ry + EPS, "{ctx}: planet {i} y {}", c.y);
            // Outside the sun's keep-out, and the whole disc too.
            let from_sun = c.x.hypot(c.y);
            assert!(from_sun >= SUN_KEEP_OUT, "{ctx}: planet {i} at {from_sun}");
            assert!(
                from_sun >= SUN_KEEP_OUT + radius - EPS,
                "{ctx}: planet {i} disc reaches the sun ({from_sun})"
            );
        }
        // No two discs overlap, and the 16 px gap is kept.
        for i in 0..n {
            for j in (i + 1)..n {
                let gap = distance(layout.centres[i], layout.centres[j]) - 2.0 * radius;
                assert!(
                    gap >= PLANET_GAP - EPS,
                    "{ctx}: planets {i} and {j}, gap {gap}"
                );
            }
        }
        // Each ring reads clockwise by index, evenly spaced; group 0 is at the top.
        let mut first = 0;
        for ring in &plan.rings {
            let phase = if ring.index == 0 { 0.0 } else { 0.5 };
            for j in 0..ring.count {
                let wanted = normalise_deg(360.0 * (j as f32 + phase) / ring.count as f32);
                let got = parametric_deg(layout.centres[first + j], ring.half);
                assert!(
                    angle_gap(got, wanted) < EPS,
                    "{ctx}: ring {} planet {j}: {got} instead of {wanted}",
                    ring.index
                );
            }
            first += ring.count;
        }
        assert_eq!(first, n, "{ctx}");
        assert_eq!(plan.rings.first().map(|r| r.index), Some(0), "{ctx}");
        assert!(
            close(layout.centres[0], Point::new(0.0, -ellipse.ry)),
            "{ctx}"
        );
        // Deterministic.
        assert_eq!(layout, planet_layout(n, ellipse), "{ctx}");
    }

    // ---- planet_layout ----

    #[test]
    fn the_constants_are_the_mockups() {
        assert_eq!(PLANET_DIAMETER, 170.0);
        assert_eq!(SUN_KEEP_OUT, 110.0);
        assert_eq!(PREVIEWS_PER_RING, 12);
        assert_eq!(MAX_PREVIEWS, 24);
    }

    #[test]
    fn no_groups_give_no_planets_and_scale_one() {
        let layout = planet_layout(0, MOCKUP);
        assert!(layout.centres.is_empty());
        assert_eq!(layout.scale, 1.0);
    }

    #[test]
    fn one_group_sits_at_the_top() {
        let layout = planet_layout(1, MOCKUP);
        assert_eq!(layout.centres, vec![Point::new(0.0, -290.0)]);
        assert_eq!(layout.scale, 1.0);
        check_planets(1, MOCKUP);
    }

    #[test]
    fn five_groups_reproduce_the_mockup() {
        let layout = planet_layout(5, MOCKUP);
        assert_eq!(layout.scale, 1.0);
        for (i, centre) in layout.centres.iter().enumerate() {
            let t = (72.0 * i as f32).to_radians();
            let expected = Point::new(470.0 * t.sin(), -290.0 * t.cos());
            assert!(close(*centre, expected), "planet {i}: {centre:?}");
        }
        assert_eq!(layout.centres[0], Point::new(0.0, -290.0));
        // Numbers read off the mockup: the second planet is to the right and above the middle.
        assert!(close(layout.centres[1], Point::new(446.997, -89.615)));
        assert!(close(layout.centres[2], Point::new(276.259, 234.615)));
        check_planets(5, MOCKUP);
    }

    #[test]
    fn up_to_eight_groups_keep_scale_one_on_one_ellipse() {
        for n in 1..=8 {
            let layout = planet_layout(n, MOCKUP);
            assert_eq!(layout.scale, 1.0, "n = {n}");
            assert_eq!(planet_plan(n, MOCKUP).rings.len(), 1, "n = {n}");
            for c in &layout.centres {
                let on = (c.x / 470.0).powi(2) + (c.y / 290.0).powi(2);
                assert!(
                    (on - 1.0).abs() < 1e-4,
                    "n = {n}: {c:?} is not on the ellipse"
                );
            }
            check_planets(n, MOCKUP);
        }
    }

    #[test]
    fn the_properties_hold_for_the_counts_of_the_brief() {
        for n in [1, 5, 6, 12, 13, 24, 64] {
            check_planets(n, MOCKUP);
        }
    }

    #[test]
    fn the_properties_hold_for_every_count_up_to_the_floor_capacity() {
        let capacity = floor_capacity(MOCKUP);
        assert!(
            capacity >= 64,
            "the product allows 64 groups, the floor holds {capacity}"
        );
        for n in 0..=capacity {
            check_planets_or_empty(n, MOCKUP);
        }
    }

    fn check_planets_or_empty(n: usize, ellipse: Ellipse) {
        if n == 0 {
            assert!(planet_layout(0, ellipse).centres.is_empty());
        } else {
            check_planets(n, ellipse);
        }
    }

    #[test]
    fn other_ellipses_work_too() {
        for ellipse in [
            Ellipse {
                rx: 360.0,
                ry: 225.0,
            },
            Ellipse {
                rx: 640.0,
                ry: 400.0,
            },
            Ellipse {
                rx: 300.0,
                ry: 450.0,
            },
            Ellipse {
                rx: 470.0,
                ry: 470.0,
            },
        ] {
            let capacity = floor_capacity(ellipse);
            assert!(capacity >= 20, "{ellipse:?}: {capacity}");
            for n in 1..=capacity {
                check_planets(n, ellipse);
            }
        }
    }

    #[test]
    fn group_zero_is_at_the_top_for_every_count() {
        for n in 1..=150 {
            let layout = planet_layout(n, MOCKUP);
            assert!(close(layout.centres[0], Point::new(0.0, -290.0)), "n = {n}");
        }
    }

    #[test]
    fn the_scale_never_grows_with_more_groups_and_never_goes_below_the_floor() {
        let mut last = f32::INFINITY;
        for n in 0..=150 {
            let scale = planet_layout(n, MOCKUP).scale;
            assert!(scale <= last, "n = {n}: {scale} after {last}");
            assert!(scale >= MIN_PLANET_SCALE, "n = {n}");
            last = scale;
        }
        assert!(planet_layout(24, MOCKUP).scale > MIN_PLANET_SCALE);
        assert!(planet_layout(64, MOCKUP).scale > MIN_PLANET_SCALE);
        assert_eq!(planet_layout(150, MOCKUP).scale, MIN_PLANET_SCALE);
    }

    #[test]
    fn many_planets_use_inner_rings_and_keep_them_in_order() {
        assert_eq!(planet_plan(12, MOCKUP).rings.len(), 1);
        assert_eq!(planet_plan(13, MOCKUP).rings.len(), 1);
        let two = planet_plan(24, MOCKUP);
        assert_eq!(two.rings.len(), 2);
        assert_eq!(two.rings.iter().map(|r| r.count).sum::<usize>(), 24);
        assert!(two.rings[0].count > two.rings[1].count);
        // The inner ring is the given ellipse scaled by 1 - D / q.
        let spacing = two.scale * PLANET_DIAMETER + PLANET_GAP;
        let factor = 1.0 - spacing / 290.0;
        assert!((two.rings[1].half.ry - 290.0 * factor).abs() < EPS);
        assert!((two.rings[1].half.rx - 470.0 * factor).abs() < EPS);
        let many = planet_plan(64, MOCKUP);
        assert!(many.rings.len() >= 3, "{many:?}");
        assert_eq!(many.rings[0].half, MOCKUP);
        for pair in many.rings.windows(2) {
            assert!(pair[1].half.ry < pair[0].half.ry);
        }
    }

    #[test]
    fn a_crowd_beyond_the_floor_is_still_placed_without_nan() {
        for n in [200, 500] {
            let layout = planet_layout(n, MOCKUP);
            assert_eq!(layout.centres.len(), n);
            assert_eq!(layout.scale, MIN_PLANET_SCALE);
            for c in &layout.centres {
                assert!(c.x.is_finite() && c.y.is_finite());
                assert!(c.x.abs() <= 470.0 + EPS && c.y.abs() <= 290.0 + EPS);
                assert!(c.x.hypot(c.y) >= SUN_KEEP_OUT);
            }
        }
    }

    #[test]
    fn a_small_window_shrinks_the_planets_so_that_they_clear_the_sun() {
        let small = Ellipse {
            rx: 300.0,
            ry: 150.0,
        };
        // One planet of scale 1.0 would reach into the sun's keep-out: 150 < 110 + 85.
        let one = planet_layout(1, small);
        assert!(one.scale < 1.0 && one.scale >= MIN_PLANET_SCALE, "{one:?}");
        for n in 1..=floor_capacity(small) {
            check_planets(n, small);
        }
        // Not even the floor scale clears the sun: one ring on the ellipse, nothing breaks.
        let tiny = Ellipse { rx: 90.0, ry: 90.0 };
        let layout = planet_layout(4, tiny);
        assert_eq!(layout.centres.len(), 4);
        assert_eq!(layout.scale, MIN_PLANET_SCALE);
        assert!(
            layout
                .centres
                .iter()
                .all(|c| c.x.is_finite() && c.y.is_finite())
        );
    }

    #[test]
    fn a_bad_ellipse_puts_everything_at_the_centre() {
        for ellipse in [
            Ellipse { rx: 0.0, ry: 0.0 },
            Ellipse {
                rx: -5.0,
                ry: 290.0,
            },
            Ellipse {
                rx: f32::NAN,
                ry: 290.0,
            },
            Ellipse {
                rx: 470.0,
                ry: f32::INFINITY,
            },
        ] {
            let layout = planet_layout(3, ellipse);
            assert_eq!(layout.centres.len(), 3);
            for c in &layout.centres {
                assert!(c.x.is_finite() && c.y.is_finite());
            }
            assert!(layout.scale.is_finite());
        }
        assert!(
            planet_layout(0, Ellipse { rx: 0.0, ry: 0.0 })
                .centres
                .is_empty()
        );
    }

    #[test]
    fn sharing_out_never_exceeds_a_capacity_and_keeps_the_outer_ring_busy() {
        for capacities in [vec![9], vec![14, 8], vec![28, 22, 15], vec![1, 1, 1]] {
            let total: usize = capacities.iter().sum();
            for n in 1..=total {
                let counts = share_out(n, &capacities);
                assert_eq!(counts.iter().sum::<usize>(), n, "{capacities:?} n = {n}");
                for (count, capacity) in counts.iter().zip(&capacities) {
                    assert!(count <= capacity, "{capacities:?} n = {n}: {counts:?}");
                }
                assert!(counts[0] >= 1, "{capacities:?} n = {n}");
            }
        }
        // A ring with the largest remainder loses its planet to the outer ring when the outer
        // ring would be left empty.
        assert_eq!(share_out(1, &[1, 5]), vec![1, 0]);
    }

    #[test]
    fn ring_capacity_matches_the_chord_rule() {
        // 9 planets on the mockup's outer ring at scale 1.0 (spacing 186), not 10.
        assert_eq!(ring_capacity(290.0, 186.0), 9);
        // Nothing fits twice when the spacing is more than the ring's diameter.
        assert_eq!(ring_capacity(50.0, 200.0), 1);
        assert_eq!(ring_capacity(100.0, 200.0), 2);
        assert_eq!(ring_capacity(0.0, 10.0), 1);
        for (reach, spacing) in [(290.0, 59.0), (173.0, 58.5), (190.0, 120.0)] {
            let m = ring_capacity(reach, spacing);
            let chord = |m: usize| 2.0 * reach * (PI / m as f32).sin();
            assert!(chord(m) >= spacing);
            assert!(chord(m + 1) < spacing);
        }
    }

    // ---- preview_layout ----

    /// Positions of `count` previews on the ellipse with the given factor and phase.
    fn expected_ring(count: usize, ellipse: Ellipse, factor: f32, phase: f32) -> Vec<Point> {
        (0..count)
            .map(|i| {
                let t = (360.0 * (i as f32 + phase) / count as f32).to_radians();
                Point::new(
                    ellipse.rx * factor * t.sin(),
                    -ellipse.ry * factor * t.cos(),
                )
            })
            .collect()
    }

    fn assert_points(actual: &[Point], expected: &[Point]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!(close(*a, *e), "preview {i}: {a:?} vs {e:?}");
        }
    }

    #[test]
    fn no_tabs_no_previews() {
        assert!(preview_layout(0, LEVEL_2).is_empty());
    }

    #[test]
    fn one_tab_is_at_the_top() {
        assert_eq!(preview_layout(1, LEVEL_2), vec![Point::new(0.0, -292.0)]);
    }

    #[test]
    fn up_to_twelve_tabs_share_one_ellipse() {
        for n in [2, 5, 11, 12] {
            let previews = preview_layout(n, LEVEL_2);
            assert_points(&previews, &expected_ring(n, LEVEL_2, 1.0, 0.0));
            assert_eq!(previews[0], Point::new(0.0, -292.0), "n = {n}");
        }
    }

    #[test]
    fn eleven_tabs_are_spread_over_the_whole_ellipse() {
        let previews = preview_layout(11, LEVEL_2);
        for (i, p) in previews.iter().enumerate() {
            let got = parametric_deg(*p, LEVEL_2);
            assert!(angle_gap(got, 360.0 * i as f32 / 11.0) < EPS, "preview {i}");
        }
    }

    #[test]
    fn the_thirteenth_tab_goes_on_the_inner_ellipse_at_the_bottom() {
        let previews = preview_layout(13, LEVEL_2);
        assert_eq!(previews.len(), 13);
        assert_points(&previews[..12], &preview_layout(12, LEVEL_2));
        // One tab on the inner ellipse, turned half of its full step: at 6 o'clock.
        assert!(close(previews[12], Point::new(0.0, 292.0 * 0.58)));
    }

    #[test]
    fn twenty_four_tabs_fill_two_rings_offset_by_half_a_step() {
        let previews = preview_layout(24, LEVEL_2);
        assert_eq!(previews.len(), 24);
        assert_points(&previews[..12], &expected_ring(12, LEVEL_2, 1.0, 0.0));
        assert_points(&previews[12..], &expected_ring(12, LEVEL_2, 0.58, 0.5));
        // The inner ring's first preview is at 15 degrees, between 0 and 30 of the outer ring.
        let first_inner = parametric_deg(
            previews[12],
            Ellipse {
                rx: 520.0 * 0.58,
                ry: 292.0 * 0.58,
            },
        );
        assert!(angle_gap(first_inner, 15.0) < EPS);
    }

    #[test]
    fn more_than_twenty_four_tabs_show_twenty_four_previews() {
        let twenty_four = preview_layout(24, LEVEL_2);
        for n in [25, 100, 10_000] {
            let previews = preview_layout(n, LEVEL_2);
            assert_eq!(previews.len(), MAX_PREVIEWS, "n = {n}");
            assert_eq!(previews, twenty_four, "n = {n}");
            assert_eq!(n - previews.len(), n - 24, "the '+N more' count");
        }
    }

    #[test]
    fn the_first_twelve_previews_do_not_move_when_more_tabs_are_added() {
        let twelve = preview_layout(12, LEVEL_2);
        for n in 13..=30 {
            assert_eq!(preview_layout(n, LEVEL_2)[..12], twelve[..], "n = {n}");
        }
    }

    #[test]
    fn previews_stay_inside_the_ellipse_and_are_deterministic() {
        for n in 0..=40 {
            let previews = preview_layout(n, LEVEL_2);
            assert_eq!(previews.len(), n.min(MAX_PREVIEWS));
            for p in &previews {
                assert!(p.x.abs() <= 520.0 + EPS && p.y.abs() <= 292.0 + EPS);
                let on = (p.x / 520.0).powi(2) + (p.y / 292.0).powi(2);
                assert!(on <= 1.0 + 1e-4, "n = {n}: {p:?}");
            }
            assert_eq!(previews, preview_layout(n, LEVEL_2));
        }
    }

    /// How deep two axis-aligned cards, `(width, height)` each, centred on `a` and `b`, run into
    /// each other: the smaller of the horizontal and vertical overlap, 0 when they are apart.
    fn overlap_depth(a: Point, a_size: (f32, f32), b: Point, b_size: (f32, f32)) -> f32 {
        let across = (a_size.0 + b_size.0) / 2.0 - (a.x - b.x).abs();
        let down = (a_size.1 + b_size.1) / 2.0 - (a.y - b.y).abs();
        across.min(down).max(0.0)
    }

    /// The cards overlap by more than a hair.
    fn cards_overlap(a: Point, a_size: (f32, f32), b: Point, b_size: (f32, f32)) -> bool {
        overlap_depth(a, a_size, b, b_size) > EPS
    }

    #[test]
    fn cards_of_one_ring_never_overlap_on_the_mockup_ellipse() {
        for n in 1..=PREVIEWS_PER_RING {
            let previews = preview_layout(n, LEVEL_2);
            for i in 0..n {
                for j in (i + 1)..n {
                    assert!(
                        !cards_overlap(previews[i], CARD, previews[j], CARD),
                        "n = {n}: cards {i} and {j} overlap ({:?}, {:?})",
                        previews[i],
                        previews[j]
                    );
                }
            }
        }
    }

    #[test]
    fn inner_cards_at_the_inner_factor_clear_each_other_and_barely_touch_the_outer_ones() {
        let inner_card = (CARD.0 * INNER_PREVIEW_FACTOR, CARD.1 * INNER_PREVIEW_FACTOR);
        let mut deepest_cross = 0.0_f32;
        for n in (PREVIEWS_PER_RING + 1)..=MAX_PREVIEWS {
            let previews = preview_layout(n, LEVEL_2);
            let size_of = |i: usize| {
                if i < PREVIEWS_PER_RING {
                    CARD
                } else {
                    inner_card
                }
            };
            for i in 0..n {
                for j in (i + 1)..n {
                    let depth = overlap_depth(previews[i], size_of(i), previews[j], size_of(j));
                    if (i < PREVIEWS_PER_RING) == (j < PREVIEWS_PER_RING) {
                        // Two cards of one ring.
                        assert!(
                            depth <= EPS,
                            "n = {n}: cards {i} and {j} overlap by {depth} ({:?}, {:?})",
                            previews[i],
                            previews[j]
                        );
                    } else {
                        deepest_cross = deepest_cross.max(depth);
                    }
                }
            }
        }
        // An outer and an inner card can overlap by a few pixels where their edges meet (the
        // worst of the 12 cases is under 5 px deep); the Orbit lane draws the inner ring first,
        // so the outer cards stay whole.
        assert!(
            deepest_cross < 5.0,
            "cards of the two rings overlap by {deepest_cross}"
        );
    }

    #[test]
    fn a_bad_ellipse_puts_every_preview_at_the_centre() {
        let previews = preview_layout(
            5,
            Ellipse {
                rx: f32::NAN,
                ry: -3.0,
            },
        );
        assert_eq!(previews.len(), 5);
        assert!(previews.iter().all(|p| p.x == 0.0 && p.y == 0.0));
    }

    /// The numbers quoted in the module documentation and the lane report, on the mockup ellipse:
    /// `(groups, scale, planets per ring from the outside in, ring factors)`.
    #[test]
    fn the_documented_examples() {
        assert_eq!(floor_capacity(MOCKUP), 73);
        let examples: [(usize, f32, &[usize], &[f32]); 8] = [
            (9, 1.00, &[9], &[1.0]),
            (10, 0.96, &[10], &[1.0]),
            (12, 0.78, &[12], &[1.0]),
            (14, 0.66, &[14], &[1.0]),
            (15, 0.64, &[10, 5], &[1.0, 0.570]),
            (24, 0.59, &[15, 9], &[1.0, 0.599]),
            (48, 0.34, &[22, 16, 10], &[1.0, 0.746, 0.491]),
            (64, 0.28, &[27, 22, 15], &[1.0, 0.781, 0.561]),
        ];
        for (n, scale, counts, factors) in examples {
            let plan = planet_plan(n, MOCKUP);
            assert!((plan.scale - scale).abs() < 1e-4, "n = {n}: {}", plan.scale);
            let got: Vec<usize> = plan.rings.iter().map(|ring| ring.count).collect();
            assert_eq!(got, counts, "n = {n}");
            for (ring, factor) in plan.rings.iter().zip(factors) {
                assert!(
                    (ring.half.ry / 290.0 - factor).abs() < 1e-3,
                    "n = {n}: factor {}",
                    ring.half.ry / 290.0
                );
            }
        }
    }
}
