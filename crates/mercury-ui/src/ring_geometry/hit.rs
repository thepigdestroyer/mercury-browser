// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What a pointer position is over: a tab, a group's label, the centre, the pinned beads' reserve
//! or nothing (SPEC §4.2: Slint arcs are not clickable shapes, so the ring's `TouchArea` is one
//! square and Rust turns the pointer position into an angle and a radius and then into a tab).
//!
//! Hit-testing reads the same [`RingLayout`] as the path builder, so what is drawn and what is hit
//! cannot drift apart. It runs on every pointer move: it does one binary search and no allocation.
//!
//! # The rules ([`hit_test_polar`] pins each one in a test)
//!
//! With `r` the distance from the ring's centre and `theta` the angle in degrees clockwise from
//! 12 o'clock (`atan2(dx, -dy)`), and `spec` the layout's effective [`RingSpec`](super::RingSpec):
//!
//! 1. A NaN or infinite coordinate, or a negative radius, is [`Hit::Outside`].
//! 2. `r < inner_radius` is [`Hit::Centre`]: the centre disc and the empty ring around it.
//! 3. **Tab zone**, `inner_radius <= r <= outer_radius + label_gap / 2` (up to the *middle* of the
//!    gap to the label band, so no dead strip lies between the two bands):
//!    - the tab whose **slot** contains `theta` is [`Hit::Tab`]. Slots, not drawn extents: two
//!      neighbours of a group leave no dead zone. Angle intervals are half-open `[start, end)`: a
//!      boundary angle belongs to the next tab, and the end of a group belongs to nothing;
//!    - a group gap is [`Hit::Outside`];
//!    - the reserve is [`Hit::Reserve`]: on a full ring it is centred on `start_deg` (half of it on
//!      each side of 12 o'clock for the default), on a fan with a reserve it is the half at each
//!      end of the span. A tab always wins over the reserve where the two would overlap by a
//!      rounding error.
//! 4. The **lifted** tab (the one the pointer hovers, passed in by the caller) stays hit out to
//!    `outer_radius + spec.lift`, over its own slot: the pointer does not flicker between the
//!    lifted segment and the label band under it.
//! 5. **Label zone**, up to and including `label_outer_radius`: `theta` inside a group's *label*
//!    extent (`[label_start, label_end)`, narrower than the group by the label inset) is
//!    [`Hit::Group`]; everywhere else, including a group with no tabs, is [`Hit::Outside`].
//! 6. Beyond `label_outer_radius`, and everywhere outside a fan's span, is [`Hit::Outside`].
//!
//! Angles are compared as offsets from the ring's start, in `[0, 360)`, so a ring that starts late
//! (extents above 360) needs no special case: `theta` and `theta + 360` are the same offset.
//! Both sides of every comparison are computed from the same stored `f32` by the same
//! subtraction (see `RingLayout::stored_offset`), which is why the exact boundary angles in the
//! tests are exact.

use super::types::{Hit, Point, RingLayout};

/// What is at `point` (ring-local, the same box as the layout's centre): the pointer position of
/// the ring's `TouchArea`. `lifted` is the flat index of the currently hovered tab, if any.
///
/// See the module documentation for the rules. A NaN or infinite coordinate is `Outside`.
pub fn hit_test(layout: &RingLayout, point: Point, lifted: Option<usize>) -> Hit {
    if !point.x.is_finite() || !point.y.is_finite() {
        return Hit::Outside;
    }
    let centre = layout.spec.centre;
    let dx = point.x - centre.x;
    let dy = point.y - centre.y;
    let radius = dx.hypot(dy);
    // `atan2(0, -0)` is 180 degrees, so the exact centre is spelled out; it is `Centre` anyway.
    let theta = if radius > 0.0 {
        dx.atan2(-dy).to_degrees()
    } else {
        0.0
    };
    hit_test_polar(layout, radius, theta, lifted)
}

/// The same as [`hit_test`] for a position that is already a radius and an angle (degrees
/// clockwise from 12 o'clock, any finite value: it is folded modulo 360). It exists so that the
/// exact boundary angles of a layout can be tested without a round trip through `atan2`, and for
/// callers that know the angle already (a keyboard walking the ring, the dial's drag).
pub fn hit_test_polar(
    layout: &RingLayout,
    radius: f32,
    theta_deg: f32,
    lifted: Option<usize>,
) -> Hit {
    if !radius.is_finite() || !theta_deg.is_finite() || radius < 0.0 {
        return Hit::Outside;
    }
    let spec = &layout.spec;
    if radius < spec.inner_radius() {
        return Hit::Centre;
    }
    let offset = layout.offset_of(theta_deg);
    let full = spec.span_deg >= 360.0;
    if !full && offset >= spec.span_deg {
        return Hit::Outside;
    }

    let outer = spec.outer_radius();
    let tab_zone_end = outer + spec.label_gap / 2.0;
    let slot = layout.tab_at_angle(theta_deg);

    if radius <= tab_zone_end {
        return match slot {
            Some(tab) => Hit::Tab(tab),
            None if in_reserve(layout, offset) => Hit::Reserve,
            None => Hit::Outside,
        };
    }
    if let Some(tab) = slot
        && lifted == Some(tab)
        && radius <= outer + spec.lift
    {
        return Hit::Tab(tab);
    }
    if radius <= spec.label_outer_radius() {
        return match group_label_at(layout, offset) {
            Some(group) => Hit::Group(group),
            None => Hit::Outside,
        };
    }
    Hit::Outside
}

/// Whether `offset` (from the ring's start, inside the span) is in the reserve.
fn in_reserve(layout: &RingLayout, offset: f32) -> bool {
    let spec = &layout.spec;
    let half = spec.reserve_deg / 2.0;
    if half <= 0.0 {
        return false;
    }
    let span = spec.span_deg.min(360.0);
    offset < half || offset >= span - half
}

/// The group whose label extent contains `offset`. Groups without tabs have an empty extent and
/// are never found.
fn group_label_at(layout: &RingLayout, offset: f32) -> Option<usize> {
    layout
        .groups
        .iter()
        .find(|group| {
            group.tab_count > 0
                && layout.stored_offset(group.label_start_deg) <= offset
                && offset < layout.stored_offset(group.label_end_deg)
        })
        .map(|group| group.group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring_geometry::{RingSpec, point_on_circle};

    const MOCKUP_GROUPS: [usize; 6] = [5, 4, 6, 3, 2, 2];
    const MOCKUP_CENTRE: Point = Point::new(338.0, 342.0);

    /// The 22-tab hub ring of the mockup: inner 165, outer 195, tab zone to 196, label band
    /// 197..221 (centre line 209), lift to 205.
    fn mockup() -> RingLayout {
        RingLayout::new(RingSpec::hub(MOCKUP_CENTRE), &MOCKUP_GROUPS)
    }

    fn polar(layout: &RingLayout, radius: f32, theta: f32) -> Hit {
        hit_test_polar(layout, radius, theta, None)
    }

    fn mid(arc_start: f32, arc_end: f32) -> f32 {
        (arc_start + arc_end) / 2.0
    }

    fn tab_mid(layout: &RingLayout, tab: usize) -> f32 {
        mid(
            layout.tabs[tab].slot_start_deg,
            layout.tabs[tab].slot_end_deg,
        )
    }

    fn last_tab_of_group(layout: &RingLayout, group: usize) -> usize {
        layout.groups[group].first_tab + layout.groups[group].tab_count - 1
    }

    // ----- radii --------------------------------------------------------------------------------

    #[test]
    fn the_exact_centre_and_the_empty_disc_are_the_centre() {
        let layout = mockup();
        assert_eq!(
            hit_test(&layout, MOCKUP_CENTRE, None),
            Hit::Centre,
            "the exact centre"
        );
        assert_eq!(polar(&layout, 0.0, 0.0), Hit::Centre);
        assert_eq!(polar(&layout, 0.0, 123.0), Hit::Centre);
        assert_eq!(polar(&layout, 100.0, tab_mid(&layout, 3)), Hit::Centre);
        // A point left of the centre, inside the inner radius.
        assert_eq!(
            hit_test(&layout, Point::new(338.0 - 100.0, 342.0), None),
            Hit::Centre
        );
    }

    #[test]
    fn the_tab_band_starts_exactly_at_the_inner_radius() {
        let layout = mockup();
        let inner = layout.spec.inner_radius();
        assert_eq!(inner, 165.0);
        let theta = tab_mid(&layout, 3);
        assert_eq!(polar(&layout, inner, theta), Hit::Tab(3));
        assert_eq!(polar(&layout, inner - 1e-3, theta), Hit::Centre);
        assert_eq!(polar(&layout, inner + 1e-3, theta), Hit::Tab(3));
        // By point: straight up from the centre at exactly the inner radius is the reserve (12
        // o'clock), straight right of it is the tab at 90 degrees.
        let right = Point::new(MOCKUP_CENTRE.x + inner, MOCKUP_CENTRE.y);
        assert!(matches!(hit_test(&layout, right, None), Hit::Tab(_)));
        let up = Point::new(MOCKUP_CENTRE.x, MOCKUP_CENTRE.y - inner);
        assert_eq!(hit_test(&layout, up, None), Hit::Reserve);
    }

    #[test]
    fn the_tab_zone_ends_in_the_middle_of_the_gap_to_the_label_band() {
        let layout = mockup();
        let spec = layout.spec;
        let theta = tab_mid(&layout, 0);
        assert_eq!(spec.outer_radius(), 195.0);
        assert_eq!(polar(&layout, spec.outer_radius(), theta), Hit::Tab(0));
        // The middle of the 2 px gap still belongs to the tab ...
        let gap_middle = spec.outer_radius() + spec.label_gap / 2.0;
        assert_eq!(gap_middle, 196.0);
        assert_eq!(polar(&layout, gap_middle, theta), Hit::Tab(0));
        // ... and just beyond it the label band's group takes over.
        assert_eq!(polar(&layout, gap_middle + 1e-3, theta), Hit::Group(0));
        assert_eq!(
            polar(&layout, spec.label_inner_radius(), theta),
            Hit::Group(0)
        );
    }

    #[test]
    fn the_label_zone_ends_exactly_at_the_label_outer_radius() {
        let layout = mockup();
        let spec = layout.spec;
        let theta = tab_mid(&layout, 6);
        assert_eq!(spec.label_outer_radius(), 221.0);
        assert_eq!(polar(&layout, spec.label_radius(), theta), Hit::Group(1));
        assert_eq!(
            polar(&layout, spec.label_outer_radius(), theta),
            Hit::Group(1)
        );
        assert_eq!(
            polar(&layout, spec.label_outer_radius() + 1e-3, theta),
            Hit::Outside
        );
        assert_eq!(polar(&layout, 500.0, theta), Hit::Outside);
        assert_eq!(polar(&layout, 1.0e9, theta), Hit::Outside);
    }

    // ----- angles: slots, groups, the reserve ---------------------------------------------------

    #[test]
    fn slot_boundaries_belong_to_the_next_tab() {
        let layout = mockup();
        let radius = layout.spec.radius;
        for arc in &layout.tabs {
            // Stored values on both sides: the start of a slot is in it ...
            assert_eq!(
                polar(&layout, radius, arc.slot_start_deg),
                Hit::Tab(arc.tab),
                "start of tab {}",
                arc.tab
            );
            // ... and the end of a slot is the next tab's start, or nothing at a group's end.
            let expected = if arc.tab == last_tab_of_group(&layout, arc.group) {
                Hit::Outside
            } else {
                Hit::Tab(arc.tab + 1)
            };
            assert_eq!(
                polar(&layout, radius, arc.slot_end_deg),
                expected,
                "end of tab {}",
                arc.tab
            );
            // A hair inside the slot, on both ends.
            assert_eq!(
                polar(&layout, radius, arc.slot_start_deg + 1e-3),
                Hit::Tab(arc.tab)
            );
            assert_eq!(
                polar(&layout, radius, arc.slot_end_deg - 1e-3),
                Hit::Tab(arc.tab)
            );
        }
    }

    #[test]
    fn a_slot_is_hit_over_its_whole_width_including_the_drawn_gap() {
        // The segment is drawn 0.6 degrees inside its slot at each end; the pointer is not
        // punished for that: the whole slot is the tab.
        let layout = mockup();
        let arc = layout.tabs[2];
        assert!(arc.start_deg > arc.slot_start_deg);
        let in_gap = mid(arc.slot_start_deg, arc.start_deg);
        assert_eq!(polar(&layout, 180.0, in_gap), Hit::Tab(2));
        let in_gap = mid(arc.end_deg, arc.slot_end_deg);
        assert_eq!(polar(&layout, 180.0, in_gap), Hit::Tab(2));
        // Between tabs 2 and 3 there is no dead zone: the two slots touch.
        assert_eq!(polar(&layout, 180.0, arc.slot_end_deg - 1e-3), Hit::Tab(2));
        assert_eq!(polar(&layout, 180.0, arc.slot_end_deg + 1e-3), Hit::Tab(3));
    }

    #[test]
    fn a_group_gap_is_outside_in_both_bands() {
        let layout = mockup();
        for group in 0..layout.groups.len() - 1 {
            let end = layout.groups[group].end_deg;
            let next = layout.groups[group + 1].start_deg;
            assert!((next - end - 5.0).abs() < 1e-3);
            let middle = mid(end, next);
            assert_eq!(polar(&layout, 180.0, middle), Hit::Outside, "tab band");
            assert_eq!(polar(&layout, 209.0, middle), Hit::Outside, "label band");
            // The end of the group is already in the gap, the start of the next is in the group.
            assert_eq!(polar(&layout, 180.0, end), Hit::Outside);
            assert!(matches!(polar(&layout, 180.0, next), Hit::Tab(_)));
        }
    }

    #[test]
    fn the_reserve_is_centred_on_twelve_o_clock() {
        let layout = mockup();
        // Reserve 24: 12 degrees on each side of the start; then half a group gap of 2.5.
        assert_eq!(polar(&layout, 180.0, 0.0), Hit::Reserve);
        assert_eq!(polar(&layout, 180.0, 5.0), Hit::Reserve);
        assert_eq!(polar(&layout, 180.0, 11.999), Hit::Reserve);
        assert_eq!(polar(&layout, 180.0, 12.0), Hit::Outside, "the half gap");
        assert_eq!(polar(&layout, 180.0, 14.0), Hit::Outside);
        assert_eq!(
            polar(&layout, 180.0, layout.tabs[0].slot_start_deg),
            Hit::Tab(0)
        );
        // The other side: the last group ends at 345.5, then 2.5 of gap, then the reserve.
        let last = layout.groups.last().unwrap();
        assert_eq!(polar(&layout, 180.0, last.end_deg), Hit::Outside);
        assert_eq!(polar(&layout, 180.0, 347.9), Hit::Outside);
        assert_eq!(polar(&layout, 180.0, 348.0), Hit::Reserve);
        assert_eq!(polar(&layout, 180.0, 359.999), Hit::Reserve);
        assert_eq!(polar(&layout, 180.0, 360.0), Hit::Reserve, "360 is 0");
        assert_eq!(polar(&layout, 180.0, -3.0), Hit::Reserve);
        // The reserve exists in the tab band only: the label band over it is outside, and
        // inside the inner radius it is still the centre.
        assert_eq!(polar(&layout, 209.0, 0.0), Hit::Outside);
        assert_eq!(polar(&layout, 100.0, 0.0), Hit::Centre);
        assert_eq!(polar(&layout, 300.0, 0.0), Hit::Outside);
        // The point straight above the centre, at the band's middle, by pixels.
        let up = Point::new(MOCKUP_CENTRE.x, MOCKUP_CENTRE.y - 180.0);
        assert_eq!(hit_test(&layout, up, None), Hit::Reserve);
    }

    #[test]
    fn label_extents_are_half_open_and_narrower_than_the_group() {
        let layout = mockup();
        for group in &layout.groups {
            let g = group.group;
            let radius = layout.spec.label_radius();
            assert_eq!(
                polar(&layout, radius, group.label_start_deg),
                Hit::Group(g),
                "label start of group {g}"
            );
            assert_eq!(
                polar(&layout, radius, group.label_end_deg),
                Hit::Outside,
                "label end of group {g}"
            );
            assert_eq!(
                polar(&layout, radius, group.label_end_deg - 1e-3),
                Hit::Group(g)
            );
            assert_eq!(
                polar(&layout, radius, group.label_start_deg - 1e-3),
                Hit::Outside,
                "the label inset"
            );
            // The same angles one band down belong to the tabs.
            assert!(matches!(
                polar(&layout, 180.0, group.label_start_deg),
                Hit::Tab(_)
            ));
        }
        // The label inset is 0.4 degrees, as the mockup draws it.
        let group = layout.groups[0];
        assert!((group.label_start_deg - group.start_deg - 0.4).abs() < 1e-5);
    }

    #[test]
    fn every_group_label_is_hit_in_its_middle() {
        let layout = mockup();
        for group in &layout.groups {
            let theta = mid(group.label_start_deg, group.label_end_deg);
            let at = point_on_circle(MOCKUP_CENTRE, layout.spec.label_radius(), theta);
            assert_eq!(hit_test(&layout, at, None), Hit::Group(group.group));
        }
    }

    // ----- by point: the round trip through atan2 -----------------------------------------------

    #[test]
    fn points_on_a_tabs_centre_line_hit_that_tab() {
        let layout = mockup();
        for arc in &layout.tabs {
            let theta = mid(arc.start_deg, arc.end_deg);
            let at = point_on_circle(MOCKUP_CENTRE, layout.spec.radius, theta);
            assert_eq!(hit_test(&layout, at, None), Hit::Tab(arc.tab));
        }
    }

    #[test]
    fn sweeping_every_slot_returns_that_tab() {
        // 360 samples inside every slot of the 22-tab layout, by polar and by pixel.
        let layout = mockup();
        let radius = layout.spec.radius;
        for arc in &layout.tabs {
            for sample in 0..360 {
                let theta = arc.slot_start_deg + (sample as f32 + 0.5) / 360.0 * layout.pitch_deg;
                assert_eq!(
                    polar(&layout, radius, theta),
                    Hit::Tab(arc.tab),
                    "tab {} sample {sample} at {theta}",
                    arc.tab
                );
                let at = point_on_circle(MOCKUP_CENTRE, radius, theta);
                assert_eq!(
                    hit_test(&layout, at, None),
                    Hit::Tab(arc.tab),
                    "tab {} sample {sample} by pixel",
                    arc.tab
                );
            }
        }
    }

    #[test]
    fn the_whole_circle_is_covered_in_order() {
        // Walking round the tab band in small steps meets reserve, tabs in order, gaps,
        // and nothing else; no tab is skipped and none appears twice.
        let layout = mockup();
        let mut seen = Vec::new();
        for step in 0..7200 {
            let theta = step as f32 * 0.05;
            match polar(&layout, 190.0, theta) {
                Hit::Tab(tab) => {
                    if seen.last() != Some(&tab) {
                        seen.push(tab);
                    }
                }
                Hit::Reserve | Hit::Outside => {}
                other => panic!("unexpected {other:?} at {theta}"),
            }
        }
        let expected: Vec<usize> = (0..22).collect();
        assert_eq!(seen, expected);
    }

    // ----- fans, late starts, special layouts ---------------------------------------------------

    #[test]
    fn a_fan_hits_only_inside_its_span() {
        let centre = Point::new(400.0, 400.0);
        let spec = RingSpec::dial_fan(centre, 200.0);
        let layout = RingLayout::new(spec, &MOCKUP_GROUPS);
        let (band, label) = (340.0, spec.label_radius());
        assert_eq!(polar(&layout, band, 199.99), Hit::Outside, "before the fan");
        assert_eq!(polar(&layout, band, 200.0), Hit::Tab(0), "the first slot");
        assert_eq!(polar(&layout, band, 297.99), Hit::Tab(21), "the last slot");
        assert_eq!(
            polar(&layout, band, 298.0),
            Hit::Outside,
            "the end of the span"
        );
        assert_eq!(polar(&layout, band, 90.0), Hit::Outside, "the other side");
        // No reserve in this fan: nothing is `Reserve` anywhere.
        for step in 0..3600 {
            let hit = polar(&layout, band, step as f32 * 0.1);
            assert_ne!(hit, Hit::Reserve);
        }
        // Labels: the same rules, no group outside the span.
        let first = layout.groups[0];
        assert_eq!(polar(&layout, label, first.label_start_deg), Hit::Group(0));
        assert_eq!(
            polar(&layout, label, first.label_start_deg - 1e-3),
            Hit::Outside
        );
        assert_eq!(polar(&layout, label, 150.0), Hit::Outside);
        // The gap between two groups of a fan, 3.5 degrees.
        let gap = mid(layout.groups[2].end_deg, layout.groups[3].start_deg);
        assert_eq!(polar(&layout, band, gap), Hit::Outside);
        // By pixel: the fan opens down-right, the centre being its corner.
        let theta = tab_mid(&layout, 10);
        let at = point_on_circle(centre, band, theta);
        assert_eq!(hit_test(&layout, at, None), Hit::Tab(10));
    }

    #[test]
    fn a_fan_with_a_reserve_has_half_of_it_at_each_end() {
        let mut spec = RingSpec::dial_fan(Point::new(0.0, 0.0), 100.0);
        spec.reserve_deg = 10.0;
        let layout = RingLayout::new(spec, &[4, 4]);
        let band = layout.spec.radius;
        assert_eq!(polar(&layout, band, 100.0), Hit::Reserve);
        assert_eq!(polar(&layout, band, 104.999), Hit::Reserve);
        assert_eq!(polar(&layout, band, 105.0), Hit::Tab(0));
        assert_eq!(polar(&layout, band, 192.999), Hit::Tab(7));
        assert_eq!(polar(&layout, band, 193.0), Hit::Reserve);
        assert_eq!(polar(&layout, band, 197.999), Hit::Reserve);
        assert_eq!(polar(&layout, band, 198.0), Hit::Outside, "past the span");
    }

    #[test]
    fn a_ring_that_starts_late_wraps_through_360() {
        let spec = RingSpec {
            start_deg: 300.0,
            ..RingSpec::default()
        };
        let layout = RingLayout::new(spec, &[3, 3]);
        assert!(layout.groups[1].end_deg > 360.0);
        let band = 180.0;
        // The first slot starts at 314.5 and runs through 360 to 368.8 (= 8.8).
        assert_eq!(polar(&layout, band, 314.5), Hit::Tab(0));
        assert_eq!(polar(&layout, band, 359.0), Hit::Tab(0));
        assert_eq!(polar(&layout, band, 0.0), Hit::Tab(0));
        assert_eq!(polar(&layout, band, 5.0), Hit::Tab(0));
        assert_eq!(polar(&layout, band, 10.0), Hit::Tab(1));
        // The reserve is centred on the start, 300: 12 degrees each side, as offsets.
        assert_eq!(polar(&layout, band, 300.0), Hit::Reserve);
        assert_eq!(polar(&layout, band, 288.5), Hit::Reserve);
        assert_eq!(polar(&layout, band, 311.9), Hit::Reserve);
        // The last tab ends at 645.5 = 285.5.
        assert_eq!(polar(&layout, band, 285.0), Hit::Tab(5));
        assert_eq!(polar(&layout, band, 285.6), Hit::Outside);
        // Exact stored boundaries above 360, both spellings of the angle.
        for arc in &layout.tabs {
            assert_eq!(polar(&layout, band, arc.slot_start_deg), Hit::Tab(arc.tab));
            let theta = mid(arc.slot_start_deg, arc.slot_end_deg);
            assert_eq!(polar(&layout, band, theta), Hit::Tab(arc.tab));
            assert_eq!(polar(&layout, band, theta - 360.0), Hit::Tab(arc.tab));
            assert_eq!(polar(&layout, band, theta + 360.0), Hit::Tab(arc.tab));
        }
        // By pixel, across the 12 o'clock line.
        let at = point_on_circle(Point::new(258.0, 258.0), band, 2.0);
        assert_eq!(hit_test(&layout, at, None), Hit::Tab(0));
        // The labels wrap the same way.
        let group = layout.groups[0];
        assert!(group.label_end_deg > 360.0 - 1.0);
        let theta = mid(group.label_start_deg, group.label_end_deg);
        assert_eq!(polar(&layout, 209.0, theta), Hit::Group(0));
        assert_eq!(polar(&layout, 209.0, theta - 360.0), Hit::Group(0));
    }

    #[test]
    fn a_one_tab_planet_ring_is_hit_all_round() {
        let spec = RingSpec::planet(Point::new(100.0, 100.0), 42.0, 16.0);
        let layout = RingLayout::new(spec, &[1]);
        assert!((layout.tabs[0].end_deg - layout.tabs[0].start_deg - 356.8).abs() < 1e-3);
        for step in 0..720 {
            let theta = step as f32 * 0.5;
            assert_eq!(polar(&layout, 42.0, theta), Hit::Tab(0), "at {theta}");
        }
        // Its label band covers the circle except for the 0.4 degree insets.
        let radius = layout.spec.label_radius();
        assert_eq!(polar(&layout, radius, 180.0), Hit::Group(0));
        assert_eq!(polar(&layout, radius, 0.2), Hit::Outside);
        assert_eq!(polar(&layout, radius, 359.8), Hit::Outside);
        assert_eq!(polar(&layout, radius, 1.0), Hit::Group(0));
        // The planet's inner radius is 34: the disc inside is the centre.
        assert_eq!(polar(&layout, 33.9, 10.0), Hit::Centre);
    }

    #[test]
    fn a_one_tab_hub_ring_keeps_its_reserve_and_gap() {
        let layout = RingLayout::new(RingSpec::default(), &[1]);
        assert_eq!(polar(&layout, 180.0, 0.0), Hit::Reserve);
        assert_eq!(polar(&layout, 180.0, 13.0), Hit::Outside);
        assert_eq!(polar(&layout, 180.0, 14.5), Hit::Tab(0));
        assert_eq!(polar(&layout, 180.0, 180.0), Hit::Tab(0));
        assert_eq!(polar(&layout, 180.0, 345.4), Hit::Tab(0));
        assert_eq!(polar(&layout, 180.0, 345.5), Hit::Outside);
        assert_eq!(polar(&layout, 180.0, 350.0), Hit::Reserve);
    }

    #[test]
    fn sixty_four_groups_are_all_reachable() {
        let sizes = vec![1usize; 64];
        let layout = RingLayout::new(RingSpec::default(), &sizes);
        assert!((layout.pitch_deg - 2.0).abs() < 1e-4);
        let label = layout.spec.label_radius();
        for group in 0..64 {
            let arc = layout.tabs[group];
            let theta = mid(arc.slot_start_deg, arc.slot_end_deg);
            assert_eq!(polar(&layout, 180.0, theta), Hit::Tab(group));
            assert_eq!(polar(&layout, label, theta), Hit::Group(group));
            // The gap after the group is outside, in both bands.
            if group + 1 < 64 {
                let end = layout.groups[group].end_deg;
                assert_eq!(polar(&layout, 180.0, end), Hit::Outside);
                assert_eq!(polar(&layout, label, end), Hit::Outside);
            }
        }
    }

    #[test]
    fn six_groups_with_uneven_sizes() {
        let sizes = [3usize, 1, 4, 1, 5, 9];
        let layout = RingLayout::new(RingSpec::default(), &sizes);
        for arc in &layout.tabs {
            let theta = mid(arc.slot_start_deg, arc.slot_end_deg);
            assert_eq!(polar(&layout, 180.0, theta), Hit::Tab(arc.tab));
            assert_eq!(polar(&layout, 209.0, theta), Hit::Group(arc.group));
        }
    }

    #[test]
    fn five_hundred_tabs_are_still_distinct() {
        let layout = RingLayout::new(RingSpec::default(), &[500]);
        for arc in &layout.tabs {
            let theta = mid(arc.slot_start_deg, arc.slot_end_deg);
            assert_eq!(polar(&layout, 180.0, theta), Hit::Tab(arc.tab));
        }
        // No reserve any more: the whole circle is tabs.
        assert!(matches!(polar(&layout, 180.0, 0.0), Hit::Tab(0)));
        assert!(matches!(polar(&layout, 180.0, 359.9), Hit::Tab(499)));
    }

    // ----- the lifted tab -----------------------------------------------------------------------

    #[test]
    fn the_lifted_tab_stays_hit_out_to_the_lift() {
        let layout = mockup();
        let spec = layout.spec;
        let theta = tab_mid(&layout, 3);
        let seven_out = spec.outer_radius() + 7.0;
        // Without the lift, 7 px outside the band is the label band over group 0 ...
        assert_eq!(polar(&layout, seven_out, theta), Hit::Group(0));
        assert_eq!(
            hit_test_polar(&layout, seven_out, theta, None),
            Hit::Group(0)
        );
        // ... with tab 3 lifted it is the tab ...
        assert_eq!(
            hit_test_polar(&layout, seven_out, theta, Some(3)),
            Hit::Tab(3)
        );
        // ... and another lifted tab changes nothing here.
        assert_eq!(
            hit_test_polar(&layout, seven_out, theta, Some(4)),
            Hit::Group(0)
        );
        assert_eq!(
            hit_test_polar(&layout, seven_out, theta, Some(999)),
            Hit::Group(0)
        );
    }

    #[test]
    fn the_lift_ends_exactly_at_outer_plus_lift() {
        let layout = mockup();
        let spec = layout.spec;
        assert_eq!(spec.lift, 10.0);
        let theta = tab_mid(&layout, 8);
        let top = spec.outer_radius() + spec.lift;
        assert_eq!(top, 205.0);
        assert_eq!(hit_test_polar(&layout, top, theta, Some(8)), Hit::Tab(8));
        assert_eq!(
            hit_test_polar(&layout, top + 1e-3, theta, Some(8)),
            Hit::Group(1)
        );
        // Just outside the tab zone, inside the lift.
        assert_eq!(
            hit_test_polar(&layout, 196.0 + 1e-3, theta, Some(8)),
            Hit::Tab(8)
        );
        // The lift extends over the lifted tab's own slot only: its neighbour is not lifted.
        let neighbour = tab_mid(&layout, 7);
        assert_eq!(
            hit_test_polar(&layout, 200.0, neighbour, Some(8)),
            Hit::Group(1)
        );
        // Over a group gap the lift reaches nothing.
        let gap = mid(layout.groups[1].end_deg, layout.groups[2].start_deg);
        assert_eq!(hit_test_polar(&layout, 200.0, gap, Some(8)), Hit::Outside);
        // And it never reaches inwards: the inner radius still makes the centre.
        assert_eq!(
            hit_test_polar(&layout, spec.inner_radius() - 1.0, theta, Some(8)),
            Hit::Centre
        );
        // By pixel, the sticky lift as the pointer drifts out of the band.
        let drifting = point_on_circle(MOCKUP_CENTRE, 202.0, theta);
        assert_eq!(hit_test(&layout, drifting, Some(8)), Hit::Tab(8));
        assert_eq!(hit_test(&layout, drifting, None), Hit::Group(1));
    }

    // ----- hostile input ------------------------------------------------------------------------

    #[test]
    fn nan_and_infinite_inputs_are_outside() {
        let layout = mockup();
        let c = MOCKUP_CENTRE;
        for point in [
            Point::new(f32::NAN, c.y),
            Point::new(c.x, f32::NAN),
            Point::new(f32::NAN, f32::NAN),
            Point::new(f32::INFINITY, c.y),
            Point::new(c.x, f32::NEG_INFINITY),
            Point::new(f32::INFINITY, f32::INFINITY),
        ] {
            assert_eq!(hit_test(&layout, point, None), Hit::Outside, "{point:?}");
            assert_eq!(hit_test(&layout, point, Some(0)), Hit::Outside);
        }
        assert_eq!(polar(&layout, f32::NAN, 10.0), Hit::Outside);
        assert_eq!(polar(&layout, 180.0, f32::NAN), Hit::Outside);
        assert_eq!(polar(&layout, f32::INFINITY, 10.0), Hit::Outside);
        assert_eq!(polar(&layout, 180.0, f32::INFINITY), Hit::Outside);
        assert_eq!(polar(&layout, -5.0, 10.0), Hit::Outside);
        // A huge but finite point is simply far away.
        assert_eq!(
            hit_test(&layout, Point::new(1.0e30, -1.0e30), None),
            Hit::Outside
        );
    }

    #[test]
    fn empty_rings_are_hit_nowhere() {
        // No groups at all: the centre and (on a full ring) the reserve are all there is.
        let layout = RingLayout::new(RingSpec::default(), &[]);
        assert_eq!(polar(&layout, 50.0, 0.0), Hit::Centre);
        assert_eq!(polar(&layout, 180.0, 0.0), Hit::Reserve);
        assert_eq!(polar(&layout, 180.0, 90.0), Hit::Outside);
        assert_eq!(polar(&layout, 209.0, 90.0), Hit::Outside);
        // Groups with no tabs are never hit either.
        let layout = RingLayout::new(RingSpec::default(), &[0, 0]);
        for step in 0..720 {
            let theta = step as f32 * 0.5;
            for radius in [180.0, 196.5, 209.0] {
                assert!(
                    matches!(polar(&layout, radius, theta), Hit::Reserve | Hit::Outside),
                    "{radius} {theta}"
                );
            }
        }
    }

    #[test]
    fn a_group_without_tabs_is_never_hit_and_takes_no_angle() {
        let sizes = [3usize, 0, 2];
        let layout = RingLayout::new(RingSpec::default(), &sizes);
        let mut tabs = Vec::new();
        let mut groups = Vec::new();
        for step in 0..1440 {
            let theta = step as f32 * 0.25;
            for (radius, lifted) in [
                (180.0, None),
                (202.0, Some(1)),
                (209.0, None),
                (205.0, Some(4)),
            ] {
                match hit_test_polar(&layout, radius, theta, lifted) {
                    Hit::Tab(tab) => {
                        assert!(tab < 5);
                        tabs.push(tab);
                    }
                    Hit::Group(group) => {
                        assert!(group == 0 || group == 2, "group {group} has no tabs");
                        groups.push(group);
                    }
                    Hit::Centre | Hit::Reserve | Hit::Outside => {}
                }
            }
        }
        tabs.dedup();
        groups.dedup();
        assert_eq!(tabs, [0, 1, 2, 3, 4]);
        assert_eq!(groups, [0, 2]);
    }

    #[test]
    fn any_layout_and_any_input_answers_without_panicking() {
        let specs = [
            RingSpec::hub(MOCKUP_CENTRE),
            RingSpec::dial_fan(Point::new(0.0, 0.0), 340.0),
            RingSpec::planet(Point::new(50.0, 50.0), 54.0, 16.0),
            RingSpec {
                start_deg: 300.0,
                reserve_deg: 40.0,
                ..RingSpec::default()
            },
            RingSpec {
                span_deg: 180.0,
                start_deg: 90.0,
                reserve_deg: 20.0,
                ..RingSpec::default()
            },
        ];
        let size_sets: [&[usize]; 7] = [
            &[],
            &[0],
            &[1],
            &MOCKUP_GROUPS,
            &[500],
            &[10; 64],
            &[1, 0, 2, 0, 0, 7],
        ];
        for spec in specs {
            for sizes in size_sets {
                let layout = RingLayout::new(spec, sizes);
                let total = layout.tabs.len();
                for step in 0..1080 {
                    let theta = -90.0 + step as f32 * 0.37;
                    for radius in [
                        0.0, 30.0, 164.9, 165.0, 180.0, 196.0, 200.0, 209.0, 221.0, 400.0,
                    ] {
                        for lifted in [None, Some(0), Some(total.saturating_sub(1))] {
                            match hit_test_polar(&layout, radius, theta, lifted) {
                                Hit::Tab(tab) => assert!(tab < total),
                                Hit::Group(group) => {
                                    assert!(layout.groups[group].tab_count > 0)
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
}
