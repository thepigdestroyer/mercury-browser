// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Where every tab and group sits on a ring (angles only; `paths.rs` turns them into shapes).
//!
//! The rule is the one of the mockup generator (`design/ui-concepts/_source/gen.swift`,
//! `ringMid`/`ring`), re-derived here: every tab gets the same angle (`pitch`), the groups are
//! separated by `group_gap_deg`, and `reserve_deg` stays free at 12 o'clock. The numbers are
//! pinned to the SVG of `ring-open-light` (22 tabs, centre (338, 342)) by the golden tests below.
//!
//! # What `RingLayout::spec` holds
//!
//! `layout.spec` is the spec the ring was **actually laid out with**, not necessarily the one
//! passed in: garbage is cleaned up (non-finite numbers, negative widths, `span_deg` outside
//! `0..=360`, `start_deg` folded into `[0, 360)` so no extent is ever negative) and the angular
//! gaps (`reserve_deg`, `group_gap_deg`, `tab_gap_deg`) are the *effective* ones after the
//! crowded-ring policy below. Radii and every presentation number are untouched. Hit-testing
//! reads the effective gaps, so it always agrees with what is drawn. Keep your own copy of the
//! requested spec and call `RingLayout::new` again when the tabs change; never lay out from
//! `layout.spec`, or a ring that was once crowded would stay squeezed.
//!
//! # Crowded rings (D9 of the lane brief)
//!
//! A ring with hundreds of tabs is legal but cannot keep its gaps. When the pitch would fall
//! below [`MIN_PITCH_DEG`] (2 degrees) the layout gives up space in a fixed order, only as far as
//! it needs:
//!
//! 1. the group gaps shrink, down to [`MIN_GROUP_GAP_DEG`] (1 degree);
//! 2. then the reserve shrinks, down to 0;
//! 3. then the tab gap: it does not change the slot angle (the pitch counts slots, the gap is
//!    cut out of each slot), so it is the drawn segment's inset rule that gives: at a pitch of
//!    4 degrees or less the inset is at most 0.35 degrees (the mockup's `sg`), and never more than
//!    40 percent of the slot, so a segment stays visible at any pitch;
//! 4. finally the group gaps shrink to 0, so the pitch is `span / tabs`.
//!
//! No input makes a width negative, a number NaN or the extents non-monotonic.

use super::types::{GroupArc, Point, RingLayout, RingSpec, TabArc, normalise_deg};

/// The slot angle below which a crowded ring starts to give up gaps.
pub const MIN_PITCH_DEG: f32 = 2.0;
/// The smallest group gap that the first stage of the crowded-ring policy shrinks to.
pub const MIN_GROUP_GAP_DEG: f32 = 1.0;
/// From this slot angle up a segment is inset by half the tab gap; below it the mockup's smaller
/// inset applies (`gen.swift`: `per > 4 ? segGap / 2 : min(0.35, per * 0.12)`).
const ROOMY_PITCH_DEG: f32 = 4.0;
/// The largest share of a slot that the two insets together may take away.
const MAX_INSET_SHARE: f32 = 0.4;
/// The label band keeps at least half of a group's angle, however small the group is.
const MAX_LABEL_INSET_SHARE: f32 = 0.25;

/// Presets for the three places that draw a ring (SPEC §4.3, §4.5, §4.6).
impl RingSpec {
    /// The hub ring of the popover (SPEC §4.3): the default spec with its centre moved.
    /// Radius 180, band 30, label band 24, reserve 24, group gap 5, tab gap 1.2 degrees.
    pub fn hub(centre: Point) -> Self {
        Self {
            centre,
            ..Self::default()
        }
    }

    /// The corner dial's fan (SPEC §4.5): radius 340, band 28, a span of 98 degrees starting at
    /// `start_deg`, no reserve, group gap 3.5, tab gap 0.5 degrees (the numbers of the mockup's
    /// dial generator). Being a fan, it has no half gap at its ends.
    pub fn dial_fan(centre: Point, start_deg: f32) -> Self {
        Self {
            centre,
            radius: 340.0,
            band_width: 28.0,
            start_deg,
            span_deg: 98.0,
            reserve_deg: 0.0,
            group_gap_deg: 3.5,
            tab_gap_deg: 0.5,
            ..Self::default()
        }
    }

    /// One Orbit planet ring (SPEC §4.6): the tabs of one group on a full circle.
    ///
    /// The numbers were measured on `orbit-1-overview` and read from the generator's
    /// `planetRing`: every tab slot is `360 / n` degrees (no reserve, **no group gap**: a planet
    /// ring holds one group), each segment is inset by 1.6 degrees at both ends (tab gap 3.2),
    /// the first slot starts at 12 o'clock, and the label band sits 2 px outside the tab band and
    /// is 22 px wide. The mockup draws the rings with `radius = 40 + 2 * tabs` and a 16 px band.
    /// The lift is half the band, at most 10 px.
    pub fn planet(centre: Point, radius: f32, band_width: f32) -> Self {
        Self {
            centre,
            radius,
            band_width,
            label_gap: 2.0,
            label_width: 22.0,
            start_deg: 0.0,
            span_deg: 360.0,
            reserve_deg: 0.0,
            group_gap_deg: 0.0,
            tab_gap_deg: 3.2,
            lift: (band_width / 2.0).clamp(0.0, 10.0),
            ..Self::default()
        }
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// The spec with garbage removed: finite numbers, no negative widths or gaps, `span_deg` in
/// `0..=360`, the reserve no larger than the span and `start_deg` in `[0, 360)`.
fn sanitised(spec: RingSpec) -> RingSpec {
    let fallback = RingSpec::default();
    let span = finite_or(spec.span_deg, fallback.span_deg).clamp(0.0, 360.0);
    RingSpec {
        centre: Point::new(
            finite_or(spec.centre.x, fallback.centre.x),
            finite_or(spec.centre.y, fallback.centre.y),
        ),
        radius: finite_or(spec.radius, fallback.radius).max(0.0),
        band_width: finite_or(spec.band_width, fallback.band_width).max(0.0),
        label_gap: finite_or(spec.label_gap, fallback.label_gap).max(0.0),
        label_width: finite_or(spec.label_width, fallback.label_width).max(0.0),
        start_deg: normalise_deg(finite_or(spec.start_deg, 0.0)),
        span_deg: span,
        reserve_deg: finite_or(spec.reserve_deg, 0.0).clamp(0.0, span),
        group_gap_deg: finite_or(spec.group_gap_deg, 0.0).max(0.0),
        tab_gap_deg: finite_or(spec.tab_gap_deg, 0.0).max(0.0),
        label_inset_deg: finite_or(spec.label_inset_deg, 0.0).max(0.0),
        lift: finite_or(spec.lift, 0.0).max(0.0),
    }
}

/// Stages 1, 2 and 4 of the crowded-ring policy: the reserve and the group gap that keep the
/// pitch at [`MIN_PITCH_DEG`] if that is possible, giving up the group gap down to
/// [`MIN_GROUP_GAP_DEG`] first, then the reserve, then the rest of the group gap. Stage 3 (the
/// tab gap) is the inset rule of [`tab_inset`].
fn fit_gaps(span: f32, reserve: f32, group_gap: f32, gaps: f32, total: usize) -> (f32, f32) {
    if total == 0 {
        return (reserve, group_gap);
    }
    let need = MIN_PITCH_DEG * total as f32;
    let room = |reserve: f32, group_gap: f32| span - reserve - group_gap * gaps;
    let (mut reserve, mut group_gap) = (reserve, group_gap);
    // 1. The group gaps, down to one degree (never raised: a spec asking for less keeps it).
    if gaps > 0.0 && room(reserve, group_gap) < need {
        let floor = group_gap.min(MIN_GROUP_GAP_DEG);
        group_gap = ((span - reserve - need) / gaps).max(floor).min(group_gap);
    }
    // 2. The reserve, down to nothing.
    if room(reserve, group_gap) < need {
        reserve = (span - group_gap * gaps - need).max(0.0).min(reserve);
    }
    // 4. The rest of the group gaps (stage 3, the tab gap, does not change the pitch).
    if gaps > 0.0 && room(reserve, group_gap) < need {
        group_gap = ((span - reserve - need) / gaps).max(0.0).min(group_gap);
    }
    (reserve, group_gap)
}

/// How far a drawn segment is cut off at each end of its slot (the mockup's `sg`): half the tab
/// gap in a roomy ring, a small fixed amount in a tight one, and never more than 40 percent of
/// the slot.
fn tab_inset(pitch: f32, tab_gap: f32) -> f32 {
    let wanted = if pitch > ROOMY_PITCH_DEG {
        tab_gap / 2.0
    } else {
        (pitch * 0.12).min(0.35)
    };
    wanted.min(pitch * MAX_INSET_SHARE).max(0.0)
}

impl RingLayout {
    /// Lays out one ring. `group_sizes[g]` is the number of tabs of group `g`, in ring order.
    ///
    /// - `pitch = (span - reserve - group_gap * G) / tabs` where `G` is the number of groups with
    ///   tabs on a full ring and one fewer on a fan (a group with no tabs takes no angle and no
    ///   gap, and gets an empty extent where it would have started).
    /// - The first slot starts at `start + reserve / 2`, plus half a group gap on a full ring.
    /// - Every slot of a group ends exactly where the next one starts (the same stored `f32`),
    ///   so hit-testing has no one-ulp holes; the last slot ends where the group ends.
    /// - A segment is its slot inset by `tab_inset` at both ends.
    /// - A group's label extent is its extent inset by `label_inset_deg`, or by a quarter of the
    ///   group's angle when that is smaller, so it never turns inside out.
    ///
    /// Crowded rings degrade as described in the module documentation; `self.spec` holds the
    /// effective spec.
    pub fn new(spec: RingSpec, group_sizes: &[usize]) -> Self {
        let mut spec = sanitised(spec);
        let total = group_sizes
            .iter()
            .fold(0usize, |sum, &count| sum.saturating_add(count));
        let live = group_sizes.iter().filter(|&&count| count > 0).count();
        let full = spec.span_deg >= 360.0;
        let gaps = if full { live } else { live.saturating_sub(1) } as f32;

        let (reserve, group_gap) = fit_gaps(
            spec.span_deg,
            spec.reserve_deg,
            spec.group_gap_deg,
            gaps,
            total,
        );
        let free = (spec.span_deg - reserve - group_gap * gaps).max(0.0);
        let pitch = if total == 0 { 0.0 } else { free / total as f32 };
        let inset = tab_inset(pitch, spec.tab_gap_deg);
        spec.reserve_deg = reserve;
        spec.group_gap_deg = group_gap;
        if total > 0 {
            spec.tab_gap_deg = inset * 2.0;
        }

        let mut angle = spec.start_deg + reserve / 2.0 + if full { group_gap / 2.0 } else { 0.0 };
        let mut tabs = Vec::with_capacity(total.min(4096));
        let mut groups = Vec::with_capacity(group_sizes.len());
        for (group, &count) in group_sizes.iter().enumerate() {
            let first_tab = tabs.len();
            let group_start = angle;
            // One expression for every edge: the end of slot `k` is bit for bit the start of
            // slot `k + 1`.
            let edge = |k: usize| group_start + k as f32 * pitch;
            for index_in_group in 0..count {
                let slot_start = edge(index_in_group);
                let slot_end = edge(index_in_group + 1);
                tabs.push(TabArc {
                    tab: first_tab + index_in_group,
                    group,
                    index_in_group,
                    slot_start_deg: slot_start,
                    slot_end_deg: slot_end,
                    start_deg: slot_start + inset,
                    end_deg: slot_end - inset,
                });
            }
            let group_end = edge(count);
            let label_inset = spec
                .label_inset_deg
                .min((group_end - group_start) * MAX_LABEL_INSET_SHARE);
            groups.push(GroupArc {
                group,
                first_tab,
                tab_count: count,
                start_deg: group_start,
                end_deg: group_end,
                label_start_deg: group_start + label_inset,
                label_end_deg: group_end - label_inset,
            });
            if count > 0 {
                angle = group_end + group_gap;
            }
        }
        Self {
            spec,
            pitch_deg: pitch,
            tabs,
            groups,
        }
    }

    /// The group of the tab with flat index `tab`.
    pub fn group_of_tab(&self, tab: usize) -> Option<usize> {
        self.tabs.get(tab).map(|arc| arc.group)
    }

    /// `deg` as an offset from the ring's start, in `[0, 360)`: the one place where the wrap at
    /// 360 degrees is handled. Extents that run past 360 (a ring that starts late) are compared
    /// through [`Self::stored_offset`], which needs no wrap.
    pub(super) fn offset_of(&self, deg: f32) -> f32 {
        normalise_deg(deg - self.spec.start_deg)
    }

    /// A stored (unwrapped) angle of this layout as an offset from the ring's start. Use this and
    /// [`Self::offset_of`] together, so both sides of a comparison come from the same subtraction.
    pub(super) fn stored_offset(&self, stored_deg: f32) -> f32 {
        stored_deg - self.spec.start_deg
    }

    /// The tab whose **slot** contains `deg` (any finite angle; it is folded modulo 360). Slots,
    /// not drawn extents, so two neighbours of a group leave no dead zone between them. Slots are
    /// half-open, `[start, end)`: a boundary angle belongs to the next tab, and the end of a group
    /// belongs to none. `None` in a gap, in the reserve, outside a fan, or when `deg` is not
    /// finite.
    pub fn tab_at_angle(&self, deg: f32) -> Option<usize> {
        if !deg.is_finite() {
            return None;
        }
        let offset = self.offset_of(deg);
        let starting_before = self
            .tabs
            .partition_point(|arc| self.stored_offset(arc.slot_start_deg) <= offset);
        let arc = self.tabs.get(starting_before.checked_sub(1)?)?;
        (offset < self.stored_offset(arc.slot_end_deg)).then_some(arc.tab)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring_geometry::point_on_circle;

    /// The hub ring of the mockup `ring-open`: 22 tabs in groups of 5, 4, 6, 3, 2 and 2.
    const MOCKUP_GROUPS: [usize; 6] = [5, 4, 6, 3, 2, 2];
    /// Where the mockup's SVG puts the ring's centre.
    const MOCKUP_CENTRE: Point = Point::new(338.0, 342.0);

    fn mockup() -> RingLayout {
        RingLayout::new(RingSpec::hub(MOCKUP_CENTRE), &MOCKUP_GROUPS)
    }

    fn assert_near(actual: Point, x: f32, y: f32, tolerance: f32, what: &str) {
        assert!(
            (actual.x - x).abs() <= tolerance && (actual.y - y).abs() <= tolerance,
            "{what}: got ({:.3}, {:.3}), the mockup draws ({x}, {y})",
            actual.x,
            actual.y
        );
    }

    /// What every layout must satisfy, whatever the input (the crowded-ring promise).
    fn assert_valid(layout: &RingLayout, sizes: &[usize], what: &str) {
        let spec = &layout.spec;
        let total: usize = sizes.iter().sum();
        // Up to 64 groups of f32 additions: a few thousandths of a degree of drift is allowed.
        let eps = 5e-3;
        assert!(
            layout.pitch_deg.is_finite() && layout.pitch_deg >= 0.0,
            "{what}: pitch"
        );
        for value in [
            spec.radius,
            spec.band_width,
            spec.label_gap,
            spec.label_width,
            spec.start_deg,
            spec.span_deg,
            spec.reserve_deg,
            spec.group_gap_deg,
            spec.tab_gap_deg,
            spec.label_inset_deg,
            spec.lift,
        ] {
            assert!(value.is_finite() && value >= 0.0, "{what}: spec {spec:?}");
        }
        assert!((0.0..360.0).contains(&spec.start_deg), "{what}: start");
        assert!(spec.reserve_deg <= spec.span_deg, "{what}: reserve");
        assert_eq!(layout.tabs.len(), total, "{what}: tab count");
        assert_eq!(layout.groups.len(), sizes.len(), "{what}: group count");
        let low = spec.start_deg - eps;
        let high = spec.start_deg + spec.span_deg + eps;
        let mut previous_end = low;
        for (flat, arc) in layout.tabs.iter().enumerate() {
            assert_eq!(arc.tab, flat, "{what}: flat index");
            for angle in [
                arc.slot_start_deg,
                arc.start_deg,
                arc.end_deg,
                arc.slot_end_deg,
            ] {
                assert!(angle.is_finite(), "{what}: tab {flat} not finite");
            }
            assert!(
                arc.slot_start_deg >= previous_end - eps,
                "{what}: tab {flat} overlaps"
            );
            assert!(
                arc.slot_start_deg <= arc.start_deg
                    && arc.start_deg <= arc.end_deg
                    && arc.end_deg <= arc.slot_end_deg,
                "{what}: tab {flat} inside out: {arc:?}"
            );
            assert!(
                arc.slot_end_deg <= high,
                "{what}: tab {flat} beyond the span"
            );
            assert!(
                arc.slot_end_deg - arc.slot_start_deg <= 360.0,
                "{what}: tab {flat} too wide"
            );
            previous_end = arc.slot_end_deg;
        }
        let mut seen = 0;
        let mut previous_group_end = low;
        for (index, group) in layout.groups.iter().enumerate() {
            assert_eq!(group.group, index, "{what}: group index");
            assert_eq!(
                group.tab_count, sizes[index],
                "{what}: group {index} tab count"
            );
            assert_eq!(group.first_tab, seen, "{what}: group {index} first tab");
            seen += group.tab_count;
            assert!(
                group.start_deg <= group.label_start_deg
                    && group.label_start_deg <= group.label_end_deg
                    && group.label_end_deg <= group.end_deg,
                "{what}: group {index} label inside out: {group:?}"
            );
            assert!(
                group.start_deg >= previous_group_end - eps,
                "{what}: group {index} overlaps"
            );
            previous_group_end = group.end_deg;
            if group.tab_count == 0 {
                assert_eq!(
                    group.start_deg, group.end_deg,
                    "{what}: empty group has an angle"
                );
            } else {
                let first = &layout.tabs[group.first_tab];
                let last = &layout.tabs[group.first_tab + group.tab_count - 1];
                assert_eq!(
                    first.slot_start_deg, group.start_deg,
                    "{what}: group {index} start"
                );
                assert_eq!(
                    last.slot_end_deg, group.end_deg,
                    "{what}: group {index} end"
                );
                assert!(
                    group.end_deg - group.start_deg <= 360.0,
                    "{what}: group {index} too wide"
                );
            }
        }
        // Slots of one group touch; groups keep their gap.
        for pair in layout.tabs.windows(2) {
            if pair[0].group == pair[1].group {
                assert_eq!(
                    pair[0].slot_end_deg, pair[1].slot_start_deg,
                    "{what}: slots touch"
                );
            } else {
                let gap = pair[1].slot_start_deg - pair[0].slot_end_deg;
                assert!(gap >= spec.group_gap_deg - eps, "{what}: group gap {gap}");
            }
        }
    }

    // ----- golden numbers of the mockup ---------------------------------------------------------

    #[test]
    fn tab_starts_match_the_mockup_svg() {
        // Centre-line points of ring-open-light.svg (`d='M x y A180 180 ...'`), within 0.3 px as
        // the lane brief asks; the numbers are in fact within 0.05 px.
        let layout = mockup();
        let centre = layout.spec.centre;
        assert_eq!(centre, MOCKUP_CENTRE);
        let radius = layout.spec.radius;
        let starts = [
            (0, 384.89, 168.21),
            (1, 425.29, 184.58),
            (2, 460.57, 210.18),
            (3, 488.66, 243.51),
            (5, 518.00, 340.89),
            (6, 512.99, 384.19),
            (9, 427.23, 498.33),
            (15, 184.14, 435.42),
            (18, 164.52, 294.01),
            (20, 218.22, 207.64),
            (21, 254.03, 182.79),
        ];
        for (tab, x, y) in starts {
            let at = point_on_circle(centre, radius, layout.tabs[tab].start_deg);
            assert_near(at, x, y, 0.3, &format!("start of tab {tab}"));
            assert_near(at, x, y, 0.05, &format!("start of tab {tab} (tight)"));
        }
    }

    #[test]
    fn tab_ends_match_the_mockup_svg() {
        let layout = mockup();
        let centre = layout.spec.centre;
        let ends = [
            (0, 421.97, 182.79),
            (1, 457.78, 207.64),
            (2, 486.57, 240.38),
            (4, 516.82, 321.45),
            (5, 513.83, 380.51),
            (21, 291.11, 168.21),
        ];
        for (tab, x, y) in ends {
            let at = point_on_circle(centre, layout.spec.radius, layout.tabs[tab].end_deg);
            assert_near(at, x, y, 0.3, &format!("end of tab {tab}"));
        }
    }

    #[test]
    fn label_bands_match_the_mockup_svg() {
        // `A209 209` paths of the label bands: radius 180 + 15 + 2 + 12 = 209.
        let layout = mockup();
        assert!((layout.spec.label_radius() - 209.0).abs() < 1e-4);
        let centre = layout.spec.centre;
        let group = |index: usize| &layout.groups[index];
        let cases = [
            (group(0).label_start_deg, 391.74, 140.03),
            (group(0).label_end_deg, 545.72, 318.87),
            (group(1).label_start_deg, 546.99, 339.98),
            (group(1).label_end_deg, 460.01, 511.69),
            (group(2).label_start_deg, 442.24, 523.15),
            (group(5).label_start_deg, 198.37, 186.48),
            (group(5).label_end_deg, 284.26, 140.03),
        ];
        for (angle, x, y) in cases {
            let at = point_on_circle(centre, layout.spec.label_radius(), angle);
            assert_near(at, x, y, 0.3, "label band end");
        }
    }

    #[test]
    fn the_first_segment_starts_where_the_mockup_draws_it() {
        let layout = mockup();
        // 12 (half the reserve) + 2.5 (half a group gap) + 0.6 (half a tab gap) = 15.1 degrees.
        assert!((layout.tabs[0].start_deg - 15.1).abs() < 1e-3);
        // Every segment is (306 / 22) - 1.2 = 12.709 degrees wide (measured: 12.707).
        let width = layout.tabs[0].end_deg - layout.tabs[0].start_deg;
        assert!((width - 12.709).abs() < 1e-2);
        assert!((layout.pitch_deg - 306.0 / 22.0).abs() < 1e-5);
        // The first tab of the amber group starts at theta 89.645 (the lane brief's number).
        assert!((layout.tabs[5].start_deg - 89.645).abs() < 1e-3);
    }

    #[test]
    fn the_ring_closes_on_the_reserve() {
        let layout = mockup();
        let last = layout.groups.last().unwrap();
        // The last group ends half a group gap and half the reserve before a full turn.
        assert!((last.end_deg + 2.5 + 12.0 - 360.0).abs() < 1e-3);
        assert_eq!(layout.tabs.len(), 22);
        // The mockup's spec is untouched: nothing was crowded.
        assert_eq!(layout.spec, RingSpec::hub(MOCKUP_CENTRE));
    }

    #[test]
    fn groups_hold_their_tabs_in_order() {
        let layout = mockup();
        let counts: Vec<usize> = layout.groups.iter().map(|g| g.tab_count).collect();
        assert_eq!(counts, MOCKUP_GROUPS);
        let firsts: Vec<usize> = layout.groups.iter().map(|g| g.first_tab).collect();
        assert_eq!(firsts, [0, 5, 9, 15, 18, 20]);
        assert_eq!(layout.group_of_tab(0), Some(0));
        assert_eq!(layout.group_of_tab(4), Some(0));
        assert_eq!(layout.group_of_tab(5), Some(1));
        assert_eq!(layout.group_of_tab(21), Some(5));
        assert_eq!(layout.group_of_tab(22), None);
        assert_eq!(layout.tabs[7].index_in_group, 2);
        assert_valid(&layout, &MOCKUP_GROUPS, "mockup");
    }

    // ----- presets ------------------------------------------------------------------------------

    #[test]
    fn the_hub_preset_is_the_default_with_a_centre() {
        let centre = Point::new(300.0, 301.0);
        let hub = RingSpec::hub(centre);
        assert_eq!(
            hub,
            RingSpec {
                centre,
                ..RingSpec::default()
            }
        );
        assert_eq!(hub.radius, 180.0);
        assert_eq!(hub.band_width, 30.0);
        assert_eq!(hub.span_deg, 360.0);
        assert_eq!(hub.reserve_deg, 24.0);
        assert_eq!(hub.group_gap_deg, 5.0);
        assert_eq!(hub.tab_gap_deg, 1.2);
    }

    #[test]
    fn the_dial_fan_preset_has_the_mockups_numbers() {
        let spec = RingSpec::dial_fan(Point::new(900.0, 700.0), 267.0);
        assert_eq!(spec.radius, 340.0);
        assert_eq!(spec.band_width, 28.0);
        assert_eq!(spec.span_deg, 98.0);
        assert_eq!(spec.start_deg, 267.0);
        assert_eq!(spec.reserve_deg, 0.0);
        assert_eq!(spec.group_gap_deg, 3.5);
        assert_eq!(spec.tab_gap_deg, 0.5);
        assert_eq!(spec.centre, Point::new(900.0, 700.0));
        assert_eq!(spec.inner_radius(), 326.0);
        assert_eq!(spec.outer_radius(), 354.0);
    }

    #[test]
    fn a_fan_has_no_half_gap_and_ends_on_its_span() {
        let spec = RingSpec::dial_fan(Point::new(0.0, 0.0), 200.0);
        let layout = RingLayout::new(spec, &MOCKUP_GROUPS);
        // 5 gaps for 6 groups: (98 - 17.5) / 22 per tab.
        assert!((layout.pitch_deg - 80.5 / 22.0).abs() < 1e-5);
        assert_eq!(layout.tabs[0].slot_start_deg, 200.0);
        assert!((layout.groups[5].end_deg - 298.0).abs() < 1e-3);
        assert!((layout.tabs[21].slot_end_deg - 298.0).abs() < 1e-3);
        // Between the groups: exactly one gap of 3.5 degrees.
        let gap = layout.groups[1].start_deg - layout.groups[0].end_deg;
        assert!((gap - 3.5).abs() < 1e-3);
        // The inset follows the mockup's rule for a pitch under 4: min(0.35, pitch * 0.12).
        assert!((layout.tabs[0].start_deg - 200.35).abs() < 1e-3);
        assert_valid(&layout, &MOCKUP_GROUPS, "fan");
    }

    #[test]
    fn a_fan_with_a_reserve_keeps_half_of_it_at_each_end() {
        let mut spec = RingSpec::dial_fan(Point::new(0.0, 0.0), 100.0);
        spec.reserve_deg = 10.0;
        let layout = RingLayout::new(spec, &[4, 4]);
        assert!((layout.tabs[0].slot_start_deg - 105.0).abs() < 1e-4);
        assert!((layout.tabs[7].slot_end_deg - 193.0).abs() < 1e-3);
        assert_valid(&layout, &[4, 4], "fan with reserve");
    }

    #[test]
    fn the_planet_preset_matches_the_orbit_mockup() {
        let spec = RingSpec::planet(Point::new(720.0, 196.0), 60.0, 16.0);
        assert_eq!(spec.reserve_deg, 0.0);
        assert_eq!(spec.group_gap_deg, 0.0);
        assert_eq!(spec.tab_gap_deg, 3.2);
        assert_eq!(spec.label_gap, 2.0);
        assert_eq!(spec.label_width, 22.0);
        assert_eq!(spec.start_deg, 0.0);
        assert_eq!(spec.span_deg, 360.0);
        // The label band's centre line is 21 px outside the ring's (`A81 81` on a 60 ring).
        assert!((spec.label_radius() - 81.0).abs() < 1e-4);
        assert_eq!(spec.lift, 8.0);
        assert_eq!(
            RingSpec::planet(Point::new(0.0, 0.0), 60.0, 40.0).lift,
            10.0
        );

        // planet 1 of orbit-1-overview-light.svg: 10 tabs, ring radius 60, band 16.
        let layout = RingLayout::new(spec, &[10]);
        assert!((layout.pitch_deg - 36.0).abs() < 1e-4);
        let at = |deg| point_on_circle(Point::new(720.0, 196.0), 60.0, deg);
        assert_near(
            at(layout.tabs[0].start_deg),
            721.68,
            136.02,
            0.02,
            "planet tab 0 start",
        );
        assert_near(
            at(layout.tabs[0].end_deg),
            753.90,
            146.49,
            0.02,
            "planet tab 0 end",
        );
        assert_near(
            at(layout.tabs[1].start_deg),
            756.61,
            148.46,
            0.02,
            "planet tab 1 start",
        );
        // planet 2: 8 tabs, ring radius 56, centre (1160, 392).
        let layout = RingLayout::new(
            RingSpec::planet(Point::new(1160.0, 392.0), 56.0, 16.0),
            &[8],
        );
        let at = |deg| point_on_circle(Point::new(1160.0, 392.0), 56.0, deg);
        assert_near(
            at(layout.tabs[0].start_deg),
            1161.56,
            336.02,
            0.02,
            "planet 2 tab 0 start",
        );
        assert_near(
            at(layout.tabs[0].end_deg),
            1198.48,
            351.31,
            0.02,
            "planet 2 tab 0 end",
        );
        assert_near(
            at(layout.tabs[1].start_deg),
            1200.69,
            353.52,
            0.02,
            "planet 2 tab 1 start",
        );
        assert_valid(&layout, &[8], "planet 2");
    }

    #[test]
    fn a_one_tab_planet_is_one_long_segment() {
        let layout = RingLayout::new(RingSpec::planet(Point::new(100.0, 100.0), 42.0, 16.0), &[1]);
        assert_eq!(layout.tabs.len(), 1);
        assert!((layout.pitch_deg - 360.0).abs() < 1e-4);
        let arc = layout.tabs[0];
        assert!((arc.start_deg - 1.6).abs() < 1e-4);
        assert!((arc.end_deg - 358.4).abs() < 1e-3);
        assert!((arc.end_deg - arc.start_deg - 356.8).abs() < 1e-3);
        assert_valid(&layout, &[1], "one-tab planet");
    }

    // ----- empty and small rings ----------------------------------------------------------------

    #[test]
    fn no_groups_and_no_tabs_give_an_empty_ring() {
        let layout = RingLayout::new(RingSpec::default(), &[]);
        assert!(layout.tabs.is_empty() && layout.groups.is_empty());
        assert_eq!(layout.pitch_deg, 0.0);
        assert_eq!(layout.spec, RingSpec::default());
        assert_valid(&layout, &[], "no groups");
        assert_eq!(layout.tab_at_angle(100.0), None);
    }

    #[test]
    fn groups_without_tabs_take_no_angle_and_no_gap() {
        let layout = RingLayout::new(RingSpec::default(), &[0, 0]);
        assert!(layout.tabs.is_empty());
        assert_eq!(layout.groups.len(), 2);
        assert_valid(&layout, &[0, 0], "two empty groups");
        // They sit where the first tab would have started, with no width.
        let start = 12.0 + 2.5;
        for group in &layout.groups {
            assert_eq!(group.tab_count, 0);
            assert!((group.start_deg - start).abs() < 1e-4);
            assert_eq!(group.start_deg, group.end_deg);
        }

        // An empty group between two others changes nothing about the angles of the others.
        let with_empty = RingLayout::new(RingSpec::default(), &[3, 0, 2]);
        let without = RingLayout::new(RingSpec::default(), &[3, 2]);
        assert_eq!(with_empty.tabs.len(), 5);
        assert_eq!(with_empty.pitch_deg, without.pitch_deg);
        for (a, b) in with_empty.tabs.iter().zip(&without.tabs) {
            assert_eq!(a.slot_start_deg, b.slot_start_deg);
            assert_eq!(a.start_deg, b.start_deg);
            assert_eq!(a.end_deg, b.end_deg);
        }
        let hole = with_empty.groups[1];
        assert_eq!(hole.start_deg, hole.end_deg);
        assert_eq!(hole.start_deg, with_empty.groups[2].start_deg);
        assert_valid(&with_empty, &[3, 0, 2], "empty group between");
    }

    #[test]
    fn one_group_closes_on_one_gap() {
        let layout = RingLayout::new(RingSpec::default(), &[7]);
        // (360 - 24 - 5) / 7 per tab: the single gap is split over both ends.
        assert!((layout.pitch_deg - 331.0 / 7.0).abs() < 1e-4);
        assert!((layout.groups[0].start_deg - 14.5).abs() < 1e-4);
        assert!((layout.groups[0].end_deg - 345.5).abs() < 1e-3);
        assert_valid(&layout, &[7], "one group");
    }

    #[test]
    fn one_tab_is_one_slot() {
        let layout = RingLayout::new(RingSpec::default(), &[1]);
        assert_eq!(layout.tabs.len(), 1);
        assert!((layout.pitch_deg - 331.0).abs() < 1e-3);
        assert_valid(&layout, &[1], "one tab");
    }

    #[test]
    fn six_groups_keep_one_gap_each() {
        let sizes = [3usize, 1, 4, 1, 5, 9];
        let layout = RingLayout::new(RingSpec::default(), &sizes);
        let total = 23.0;
        assert!((layout.pitch_deg - (360.0 - 24.0 - 30.0) / total).abs() < 1e-5);
        for pair in layout.groups.windows(2) {
            assert!((pair[1].start_deg - pair[0].end_deg - 5.0).abs() < 1e-3);
        }
        assert_eq!(layout.spec, RingSpec::default());
        assert_valid(&layout, &sizes, "six groups");
    }

    // ----- crowded rings ------------------------------------------------------------------------

    #[test]
    fn sixty_four_groups_of_one_tab_shrink_the_group_gap_only() {
        let sizes = vec![1usize; 64];
        let layout = RingLayout::new(RingSpec::default(), &sizes);
        // 64 gaps of 5 would leave 16 degrees for 64 tabs: stage 1 shrinks the gap to 3.25 so
        // that the pitch is exactly 2 degrees. The reserve stays.
        assert!(
            (layout.pitch_deg - MIN_PITCH_DEG).abs() < 1e-4,
            "{}",
            layout.pitch_deg
        );
        assert!((layout.spec.group_gap_deg - 3.25).abs() < 1e-4);
        assert_eq!(layout.spec.reserve_deg, 24.0);
        assert!(layout.spec.tab_gap_deg < 1.2);
        for arc in &layout.tabs {
            assert!(arc.end_deg - arc.start_deg > 1.0, "a segment stays visible");
        }
        assert_valid(&layout, &sizes, "64 x 1");
    }

    #[test]
    fn sixty_four_groups_of_ten_tabs_give_up_every_gap() {
        let sizes = vec![10usize; 64];
        let layout = RingLayout::new(RingSpec::default(), &sizes);
        // 640 tabs cannot reach 2 degrees each: the gaps and the reserve all go, the pitch is
        // span / tabs.
        assert_eq!(layout.spec.group_gap_deg, 0.0);
        assert_eq!(layout.spec.reserve_deg, 0.0);
        assert!((layout.pitch_deg - 360.0 / 640.0).abs() < 1e-6);
        assert_eq!(layout.tabs.len(), 640);
        assert_valid(&layout, &sizes, "64 x 10");
        // Still strictly increasing, no segment collapsed.
        for arc in &layout.tabs {
            assert!(arc.end_deg > arc.start_deg);
        }
    }

    #[test]
    fn five_hundred_tabs_in_one_group() {
        let layout = RingLayout::new(RingSpec::default(), &[500]);
        assert_eq!(layout.tabs.len(), 500);
        assert_eq!(layout.spec.reserve_deg, 0.0);
        assert_eq!(layout.spec.group_gap_deg, 0.0);
        assert!((layout.pitch_deg - 360.0 / 500.0).abs() < 1e-6);
        // The tab gap has shrunk to 24 percent of the slot at most.
        let arc = layout.tabs[250];
        let width = arc.end_deg - arc.start_deg;
        assert!(width > layout.pitch_deg * 0.7 && width < layout.pitch_deg);
        assert_valid(&layout, &[500], "500 tabs");
    }

    #[test]
    fn the_policy_gives_up_the_group_gap_then_the_reserve_then_the_rest_of_the_gap() {
        // 6 groups: 24 reserve, gap 5. Pitch 2 needs 2 * tabs degrees.
        let layout = |tabs: usize| {
            let mut sizes = vec![tabs / 6; 6];
            sizes[0] += tabs % 6;
            RingLayout::new(RingSpec::default(), &sizes)
        };
        // 120 tabs: 306 degrees free at the full gaps, 240 needed: nothing shrinks.
        let roomy = layout(120);
        assert_eq!(roomy.spec.group_gap_deg, 5.0);
        assert_eq!(roomy.spec.reserve_deg, 24.0);
        // 156 tabs need 312 degrees: only the group gap shrinks, (336 - 312) / 6 = 4.
        let tighter = layout(156);
        assert!((tighter.spec.group_gap_deg - 4.0).abs() < 1e-4);
        assert_eq!(tighter.spec.reserve_deg, 24.0);
        assert!((tighter.pitch_deg - 2.0).abs() < 1e-4);
        // 170 tabs need 340: the gap is at its floor of 1, the reserve gives up 360 - 6 - 340 = 14.
        let reserve_gives = layout(170);
        assert!((reserve_gives.spec.group_gap_deg - 1.0).abs() < 1e-6);
        assert!((reserve_gives.spec.reserve_deg - 14.0).abs() < 1e-3);
        assert!((reserve_gives.pitch_deg - 2.0).abs() < 1e-4);
        // 177 tabs need 354: the reserve is gone (360 - 6 - 354 = 0), the gap still 1.
        let reserve_gone = layout(177);
        assert!(reserve_gone.spec.reserve_deg.abs() < 1e-3);
        assert!((reserve_gone.spec.group_gap_deg - 1.0).abs() < 1e-6);
        // 180 tabs need 360: now the group gap gives up the last 0.5 degrees.
        let gap_gone = layout(180);
        assert_eq!(gap_gone.spec.reserve_deg, 0.0);
        assert!(gap_gone.spec.group_gap_deg.abs() < 1e-4);
        assert!((gap_gone.pitch_deg - 2.0).abs() < 1e-4);
        // 200 tabs cannot reach 2 degrees any more.
        let beyond = layout(200);
        assert_eq!(beyond.spec.group_gap_deg, 0.0);
        assert!((beyond.pitch_deg - 1.8).abs() < 1e-5);
        for l in [
            &roomy,
            &tighter,
            &reserve_gives,
            &reserve_gone,
            &gap_gone,
            &beyond,
        ] {
            let sizes: Vec<usize> = l.groups.iter().map(|g| g.tab_count).collect();
            assert_valid(l, &sizes, "policy ladder");
        }
    }

    #[test]
    fn a_spec_that_asks_for_less_keeps_it() {
        // A group gap of 0.5 is already under the floor: the policy never raises it, and as the
        // pitch of 2.56 degrees is roomy enough, it does not take it away either.
        let spec = RingSpec {
            group_gap_deg: 0.5,
            reserve_deg: 0.0,
            ..RingSpec::default()
        };
        let sizes = vec![2usize; 64];
        let layout = RingLayout::new(spec, &sizes);
        assert!(
            (layout.spec.group_gap_deg - 0.5).abs() < 1e-6,
            "{:?}",
            layout.spec
        );
        assert!((layout.pitch_deg - (360.0 - 32.0) / 128.0).abs() < 1e-5);
        assert_valid(&layout, &sizes, "small gap");
        // With 192 tabs the pitch cannot reach 2 degrees: the last stage takes the small gap away
        // too (it never goes to the floor of 1 degree, it was below it already).
        let sizes = vec![3usize; 64];
        let layout = RingLayout::new(spec, &sizes);
        assert_eq!(layout.spec.group_gap_deg, 0.0);
        assert_valid(&layout, &sizes, "small gap, crowded");
    }

    #[test]
    fn the_tab_gap_follows_the_mockups_inset_rule() {
        // Roomy: half the tab gap on each side.
        assert!((tab_inset(13.9, 1.2) - 0.6).abs() < 1e-6);
        // Pitch 4 or less: min(0.35, 12 percent of the pitch).
        assert!((tab_inset(4.0, 1.2) - 0.35).abs() < 1e-6);
        assert!((tab_inset(2.0, 1.2) - 0.24).abs() < 1e-6);
        assert!((tab_inset(0.72, 1.2) - 0.0864).abs() < 1e-6);
        // Never more than 40 percent of a slot, never negative or NaN.
        assert!((tab_inset(5.0, 40.0) - 2.0).abs() < 1e-6);
        assert_eq!(tab_inset(0.0, 1.2), 0.0);
        assert_eq!(tab_inset(10.0, -3.0), 0.0);
        assert_eq!(tab_inset(10.0, 0.0), 0.0);
        // The effective tab gap of a crowded ring says what is drawn.
        let layout = RingLayout::new(RingSpec::default(), &[500]);
        assert!((layout.spec.tab_gap_deg - 2.0 * 0.12 * layout.pitch_deg).abs() < 1e-6);
    }

    /// A named way to make the tab count of group `g`.
    type SizePattern = (&'static str, fn(usize) -> usize);

    // ----- fans, late starts and garbage --------------------------------------------------------

    #[test]
    fn a_ring_that_starts_late_has_extents_above_360() {
        let spec = RingSpec {
            start_deg: 300.0,
            ..RingSpec::default()
        };
        let layout = RingLayout::new(spec, &[3, 3]);
        assert_eq!(layout.spec.start_deg, 300.0);
        // 300 + 12 + 2.5 = 314.5 for the first slot; the last group ends at 300 + 360 - 14.5.
        assert!((layout.tabs[0].slot_start_deg - 314.5).abs() < 1e-3);
        assert!((layout.groups[1].end_deg - 645.5).abs() < 1e-2);
        assert!(layout.groups[1].end_deg > 360.0);
        assert!(layout.groups[1].end_deg - layout.tabs[0].slot_start_deg <= 360.0);
        assert_valid(&layout, &[3, 3], "late start");
    }

    #[test]
    fn a_negative_start_is_folded_into_a_full_turn() {
        let spec = RingSpec {
            start_deg: -25.0,
            ..RingSpec::default()
        };
        let layout = RingLayout::new(spec, &[4, 4]);
        assert_eq!(layout.spec.start_deg, 335.0);
        assert!(layout.tabs.iter().all(|arc| arc.slot_start_deg > 0.0));
        assert_valid(&layout, &[4, 4], "negative start");
    }

    #[test]
    fn garbage_specs_are_cleaned_up() {
        let spec = RingSpec {
            radius: f32::NAN,
            band_width: -4.0,
            span_deg: 720.0,
            reserve_deg: f32::INFINITY,
            group_gap_deg: -1.0,
            tab_gap_deg: f32::NAN,
            label_inset_deg: -2.0,
            start_deg: f32::NAN,
            centre: Point::new(f32::NAN, 5.0),
            ..RingSpec::default()
        };
        let sizes = [3usize, 0, 4];
        let layout = RingLayout::new(spec, &sizes);
        assert_valid(&layout, &sizes, "garbage");
        assert_eq!(layout.spec.span_deg, 360.0);
        assert_eq!(layout.spec.radius, RingSpec::default().radius);
        assert_eq!(layout.spec.band_width, 0.0);

        // A zero span lays every slot at zero width without panicking.
        let flat = RingSpec {
            span_deg: 0.0,
            ..RingSpec::default()
        };
        let layout = RingLayout::new(flat, &[2, 2]);
        assert_eq!(layout.pitch_deg, 0.0);
        assert_valid(&layout, &[2, 2], "zero span");
    }

    #[test]
    fn every_layout_stays_valid() {
        let late = RingSpec {
            start_deg: 340.0,
            ..RingSpec::default()
        };
        let half = RingSpec {
            span_deg: 180.0,
            start_deg: 90.0,
            reserve_deg: 20.0,
            ..RingSpec::default()
        };
        let specs = [
            ("hub", RingSpec::hub(MOCKUP_CENTRE)),
            ("fan", RingSpec::dial_fan(Point::new(500.0, 500.0), 267.0)),
            (
                "planet",
                RingSpec::planet(Point::new(200.0, 200.0), 60.0, 16.0),
            ),
            ("late", late),
            ("half", half),
        ];
        let patterns: [SizePattern; 4] = [
            ("ones", |_| 1),
            ("tens", |_| 10),
            ("mod7", |g| g % 7),
            ("holes", |g| if g % 3 == 0 { 0 } else { 1 + g % 5 }),
        ];
        for (name, spec) in specs {
            for groups in 0..=70usize {
                for (pattern_name, pattern) in patterns {
                    let sizes: Vec<usize> = (0..groups).map(pattern).collect();
                    let layout = RingLayout::new(spec, &sizes);
                    assert_valid(
                        &layout,
                        &sizes,
                        &format!("{name}, {groups} groups, {pattern_name}"),
                    );
                }
            }
        }
    }

    // ----- tab_at_angle -------------------------------------------------------------------------

    #[test]
    fn tab_at_angle_uses_half_open_slots() {
        let layout = mockup();
        for arc in &layout.tabs {
            assert_eq!(
                layout.tab_at_angle(arc.slot_start_deg),
                Some(arc.tab),
                "start of {}",
                arc.tab
            );
            let mid = (arc.slot_start_deg + arc.slot_end_deg) / 2.0;
            assert_eq!(layout.tab_at_angle(mid), Some(arc.tab));
            let group = &layout.groups[arc.group];
            let last_of_group = group.first_tab + group.tab_count - 1;
            if arc.tab == last_of_group {
                // The end of a group belongs to nothing: it is the group gap.
                assert_eq!(
                    layout.tab_at_angle(arc.slot_end_deg),
                    None,
                    "end of group {}",
                    arc.group
                );
            } else {
                assert_eq!(
                    layout.tab_at_angle(arc.slot_end_deg),
                    Some(arc.tab + 1),
                    "end of {}",
                    arc.tab
                );
            }
        }
        // The reserve, the gaps, a non-finite angle.
        assert_eq!(layout.tab_at_angle(0.0), None);
        assert_eq!(layout.tab_at_angle(13.0), None);
        assert_eq!(layout.tab_at_angle(f32::NAN), None);
        assert_eq!(layout.tab_at_angle(f32::INFINITY), None);
    }

    #[test]
    fn tab_at_angle_wraps_and_folds_modulo_360() {
        let spec = RingSpec {
            start_deg: 300.0,
            ..RingSpec::default()
        };
        let layout = RingLayout::new(spec, &[3, 3]);
        // The slot that begins at 314.5 and the ones past 360 are found at both spellings.
        assert_eq!(layout.tab_at_angle(320.0), Some(0));
        assert_eq!(layout.tab_at_angle(5.0), Some(0));
        assert_eq!(layout.tab_at_angle(365.0), Some(0));
        assert_eq!(layout.tab_at_angle(-355.0), Some(0));
        assert_eq!(
            layout.tab_at_angle(100.0 + 360.0),
            layout.tab_at_angle(100.0)
        );
        let late = layout.tabs[5];
        let mid = (late.slot_start_deg + late.slot_end_deg) / 2.0;
        assert_eq!(layout.tab_at_angle(mid), Some(5));
        assert_eq!(layout.tab_at_angle(mid - 360.0), Some(5));
    }
}
