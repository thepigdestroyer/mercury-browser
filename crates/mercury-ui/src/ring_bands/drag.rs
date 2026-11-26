// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The drop target of a segment drag: which group and index a released tab lands at and the accent
//! bar that marks the insertion point.

use crate::ring_geometry::hit::hit_test;
use crate::ring_geometry::paths::annulus_sector;
use crate::ring_geometry::slint_models::RingModel;
use crate::ring_geometry::{Hit, Point, normalise_deg, point_on_circle};
use crate::{ArcSeg, RingDropTarget};

/// Width of the accent bar, in px, and how far it reaches past the band on each side.
const BAR_WIDTH: f32 = 3.0;
const BAR_REACH: f32 = 4.0;

/// The slot under (`x`, `y`) for the tab `dragging_tab` (a `TabRow.id`), or an invalid target.
///
/// - Over a tab, the tab is split at the middle of its slot: the half the pointer is in decides
///   whether the tab lands before it (`index` = the tab's own index in its group) or after it
///   (`index` + 1). The bar stands on that edge of the slot.
/// - Over a group's label band, the tab is appended to that group (`index` = its tab count) and the
///   bar stands at the end of the group.
/// - Over the centre, the reserve, a group gap, beyond the ring, or at a non-finite point: `valid`
///   is false, `group` and `index` are -1 and the indicator is empty.
///
/// A drop on the dragged tab's own slot (`index` equal to its index, or one more) is still returned
/// as valid with that index: it would not move the tab, and the host may ignore it. The index
/// counts the dragged tab itself, as the ring shows it before the move.
pub(super) fn drop_target_for(
    model: &RingModel,
    x: f32,
    y: f32,
    dragging_tab: i32,
) -> RingDropTarget {
    target(model, x, y, dragging_tab).unwrap_or_else(invalid)
}

fn invalid() -> RingDropTarget {
    RingDropTarget {
        valid: false,
        group: -1,
        index: -1,
        indicator: ArcSeg::default(),
    }
}

fn target(model: &RingModel, x: f32, y: f32, dragging_tab: i32) -> Option<RingDropTarget> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let layout = &model.layout;
    let spec = &layout.spec;
    let pointer = Point::new(x, y);
    let (group, index, edge_deg) = match hit_test(layout, pointer, None) {
        Hit::Tab(flat) => {
            let arc = layout.tabs.get(flat)?;
            let (dx, dy) = (x - spec.centre.x, y - spec.centre.y);
            let pointer_offset = normalise_deg(dx.atan2(-dy).to_degrees() - spec.start_deg);
            let middle = (arc.slot_start_deg + arc.slot_end_deg) / 2.0 - spec.start_deg;
            let group = model.group_id(arc.group)?;
            if pointer_offset < middle {
                (group, arc.index_in_group, arc.slot_start_deg)
            } else {
                (group, arc.index_in_group + 1, arc.slot_end_deg)
            }
        }
        Hit::Group(g) => {
            let arc = layout.groups.get(g)?;
            (model.group_id(g)?, arc.tab_count, arc.end_deg)
        }
        _ => return None,
    };
    Some(RingDropTarget {
        valid: true,
        group,
        index: i32::try_from(index).unwrap_or(i32::MAX),
        indicator: indicator(model, edge_deg, dragging_tab),
    })
}

/// The accent bar across the tab band at `angle_deg`.
fn indicator(model: &RingModel, angle_deg: f32, dragging_tab: i32) -> ArcSeg {
    let spec = &model.layout.spec;
    let half_width_deg = (BAR_WIDTH / 2.0 / spec.radius.max(1.0)).to_degrees();
    let mid = point_on_circle(spec.centre, spec.radius, angle_deg);
    ArcSeg {
        tab: dragging_tab,
        commands: annulus_sector(
            spec.centre,
            spec.inner_radius() - BAR_REACH,
            spec.outer_radius() + BAR_REACH,
            angle_deg - half_width_deg,
            angle_deg + half_width_deg,
        )
        .into(),
        mid_x: mid.x,
        mid_y: mid.y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
    use crate::ring_geometry::RingSpec;

    const CENTRE: Point = Point::new(258.0, 258.0);

    fn mockup() -> RingModel {
        RingModel::new(
            RingSpec::hub(CENTRE),
            &mockup_groups(false),
            &mockup_tabs(false),
        )
    }

    fn at(radius: f32, deg: f32) -> (f32, f32) {
        let p = point_on_circle(CENTRE, radius, deg);
        (p.x, p.y)
    }

    fn quarter(model: &RingModel, flat: usize, second_half: bool) -> f32 {
        let arc = &model.layout.tabs[flat];
        let (a, b) = (arc.slot_start_deg, arc.slot_end_deg);
        if second_half {
            a + (b - a) * 0.75
        } else {
            a + (b - a) * 0.25
        }
    }

    fn drop_at(model: &RingModel, radius: f32, deg: f32) -> RingDropTarget {
        let (x, y) = at(radius, deg);
        drop_target_for(model, x, y, 105)
    }

    /// Where the indicator's anchor is, in degrees clockwise from 12 o'clock in `[0, 360)`.
    fn indicator_deg(target: &RingDropTarget) -> f32 {
        let (dx, dy) = (
            target.indicator.mid_x - CENTRE.x,
            target.indicator.mid_y - CENTRE.y,
        );
        normalise_deg(dx.atan2(-dy).to_degrees())
    }

    fn assert_at_deg(target: &RingDropTarget, want: f32) {
        let diff = (indicator_deg(target) - normalise_deg(want) + 540.0).rem_euclid(360.0) - 180.0;
        assert!(
            diff.abs() < 0.05,
            "bar at {} not {want}",
            indicator_deg(target)
        );
    }

    fn assert_invalid(target: &RingDropTarget) {
        assert!(!target.valid);
        assert_eq!((target.group, target.index), (-1, -1));
        assert!(target.indicator.commands.is_empty());
    }

    fn assert_bar(target: &RingDropTarget, dragging: i32) {
        assert_eq!(target.indicator.tab, dragging);
        let commands = target.indicator.commands.as_str();
        assert!(
            commands.starts_with('M') && commands.ends_with('Z'),
            "{commands}"
        );
    }

    #[test]
    fn before_the_first_tab_of_a_group() {
        let model = mockup();
        // Lisbon trip (group 2) starts at flat position 5.
        let target = drop_at(&model, 180.0, quarter(&model, 5, false));
        assert!(target.valid);
        assert_eq!((target.group, target.index), (2, 0));
        assert_bar(&target, 105);
        assert_at_deg(&target, model.layout.tabs[5].slot_start_deg);
        let first = drop_at(&model, 180.0, quarter(&model, 0, false));
        assert_eq!((first.group, first.index), (1, 0));
        assert_at_deg(&first, model.layout.groups[0].start_deg);
    }

    #[test]
    fn between_two_tabs_from_either_side() {
        let model = mockup();
        let from_left = drop_at(&model, 180.0, quarter(&model, 2, true));
        let from_right = drop_at(&model, 180.0, quarter(&model, 3, false));
        for target in [&from_left, &from_right] {
            assert!(target.valid);
            assert_eq!((target.group, target.index), (1, 3));
            assert_at_deg(target, model.layout.tabs[2].slot_end_deg);
            assert_bar(target, 105);
        }
        assert_eq!(from_left.indicator, from_right.indicator);
    }

    #[test]
    fn after_the_last_tab_of_a_group() {
        let model = mockup();
        let target = drop_at(&model, 180.0, quarter(&model, 4, true));
        assert_eq!((target.group, target.index), (1, 5));
        assert_at_deg(&target, model.layout.groups[0].end_deg);
        let last = model.layout.tabs.len() - 1;
        let target = drop_at(&model, 180.0, quarter(&model, last, true));
        assert_eq!((target.group, target.index), (6, 2));
        assert_at_deg(&target, model.layout.tabs[last].slot_end_deg);
    }

    #[test]
    fn the_whole_tab_band_counts_from_its_inner_to_its_outer_edge() {
        let model = mockup();
        let deg = quarter(&model, 7, true);
        for radius in [165.5, 180.0, 194.5] {
            let target = drop_at(&model, radius, deg);
            assert_eq!((target.group, target.index), (2, 3), "radius {radius}");
        }
    }

    #[test]
    fn a_label_band_appends_to_its_group() {
        let model = mockup();
        let label = model.layout.spec.label_radius();
        for (g, id, count) in [(0, 1, 5), (2, 3, 6), (5, 6, 2)] {
            let group = &model.layout.groups[g];
            let target = drop_at(&model, label, (group.start_deg + group.end_deg) / 2.0);
            assert!(target.valid, "group {g}");
            assert_eq!((target.group, target.index), (id, count));
            assert_at_deg(&target, group.end_deg);
            assert_bar(&target, 105);
        }
    }

    #[test]
    fn the_centre_the_reserve_a_gap_and_the_outside_are_invalid() {
        let model = mockup();
        assert_invalid(&drop_target_for(&model, CENTRE.x, CENTRE.y, 105));
        assert_invalid(&drop_at(&model, 100.0, 90.0));
        // The reserve at 12 o'clock.
        assert_invalid(&drop_at(&model, 180.0, 0.0));
        assert_invalid(&drop_at(&model, 180.0, 359.0));
        // The gap between Research and Lisbon trip.
        let gap = (model.layout.groups[0].end_deg + model.layout.groups[1].start_deg) / 2.0;
        assert_invalid(&drop_at(&model, 180.0, gap));
        assert_invalid(&drop_at(&model, 300.0, 90.0));
        assert_invalid(&drop_target_for(&model, -50.0, -50.0, 105));
    }

    #[test]
    fn a_pointer_that_is_not_a_number_is_invalid() {
        let model = mockup();
        for (x, y) in [
            (f32::NAN, 100.0),
            (100.0, f32::NAN),
            (f32::INFINITY, 100.0),
            (100.0, f32::NEG_INFINITY),
            (f32::NAN, f32::NAN),
        ] {
            assert_invalid(&drop_target_for(&model, x, y, 105));
        }
    }

    #[test]
    fn the_dragged_tabs_own_slot_and_the_one_after_it_are_valid_no_ops() {
        let model = mockup();
        // Tab 103 is index 2 of Research: before it is index 2, after it is index 3, and both
        // sit next to the tab itself.
        let (x, y) = at(180.0, quarter(&model, 2, false));
        let before = drop_target_for(&model, x, y, 103);
        let (x, y) = at(180.0, quarter(&model, 2, true));
        let after = drop_target_for(&model, x, y, 103);
        assert!(before.valid && after.valid);
        assert_eq!((before.group, before.index), (1, 2));
        assert_eq!((after.group, after.index), (1, 3));
        assert_eq!(before.indicator.tab, 103);
    }

    #[test]
    fn the_bar_is_a_3_px_wide_band_with_a_reach_on_both_sides() {
        let model = mockup();
        let target = drop_at(&model, 180.0, quarter(&model, 9, false));
        let spec = &model.layout.spec;
        let arc = &model.layout.tabs[9];
        let half = (1.5 / spec.radius).to_degrees();
        let want = annulus_sector(
            spec.centre,
            spec.inner_radius() - 4.0,
            spec.outer_radius() + 4.0,
            arc.slot_start_deg - half,
            arc.slot_start_deg + half,
        );
        assert_eq!(target.indicator.commands.as_str(), want);
        // The anchor is on the middle of the band at the edge.
        let anchor = point_on_circle(spec.centre, spec.radius, arc.slot_start_deg);
        assert!((target.indicator.mid_x - anchor.x).abs() < 1e-3);
        assert!((target.indicator.mid_y - anchor.y).abs() < 1e-3);
    }

    #[test]
    fn a_fan_that_starts_late_still_finds_its_slots() {
        let spec = RingSpec::dial_fan(Point::new(400.0, 400.0), 300.0);
        let (groups, tabs) = (mockup_groups(false), mockup_tabs(false));
        let model = RingModel::new(spec, &groups, &tabs);
        let centre = model.layout.spec.centre;
        for flat in [0, 11, 21] {
            let arc = &model.layout.tabs[flat];
            for (second, want) in [(false, 0), (true, 1)] {
                let deg = if second {
                    arc.slot_start_deg + (arc.slot_end_deg - arc.slot_start_deg) * 0.75
                } else {
                    arc.slot_start_deg + (arc.slot_end_deg - arc.slot_start_deg) * 0.25
                };
                let p = point_on_circle(centre, spec.radius, deg);
                let target = drop_target_for(&model, p.x, p.y, 105);
                assert!(target.valid, "flat {flat}");
                assert_eq!(
                    target.index,
                    arc.index_in_group as i32 + want,
                    "flat {flat} second half {second}"
                );
            }
        }
    }

    #[test]
    fn sixty_four_groups_of_one_tab_give_valid_targets() {
        let (groups, tabs) = generated(64, 64, false);
        let model = RingModel::new(RingSpec::hub(CENTRE), &groups, &tabs);
        for (flat, arc) in model.layout.tabs.iter().enumerate() {
            for second in [false, true] {
                let deg = quarter(&model, flat, second);
                let target = drop_at(&model, 180.0, deg);
                assert!(target.valid, "tab {flat}");
                assert_eq!(target.group, flat as i32 + 1);
                assert_eq!(target.index, i32::from(second));
                assert_at_deg(
                    &target,
                    if second {
                        arc.slot_end_deg
                    } else {
                        arc.slot_start_deg
                    },
                );
                assert_bar(&target, 105);
            }
        }
    }
}
