// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The hub button's donut (SPEC §4.3): one arc per group, in the group's colour, sized by its
//! tab count, in the button's own 30 px box; and its accessible label. Pure.
//!
//! The ring is centred at (15, 15) with a middle radius of 11.5 and a width of 5 (outer radius 14,
//! inner 9). The arcs start at 12 o'clock and run clockwise. With two or more groups each arc is
//! followed by a gap of [`GAP_DEG`] degrees (less when there are so many groups that the arcs
//! would shrink below a quarter of the circle in all: the gap is then `270 / groups`), and the
//! rest of the circle is shared in proportion to the tab counts. One group is the full ring (one
//! shape, written by `annulus_sector` as two half arcs); no group with a tab is one ring in the
//! fallback colour. The path text comes from [`crate::ring_geometry::paths`] only.

use crate::ring_geometry::{Point, paths};
use crate::{DonutArc, GroupRow};

/// The button's edge in px.
pub const BUTTON_SIZE: f32 = 30.0;
/// The gap between two arcs, in degrees.
pub const GAP_DEG: f32 = 4.0;
const CENTRE: Point = Point::new(BUTTON_SIZE / 2.0, BUTTON_SIZE / 2.0);
const RING_RADIUS: f32 = 11.5;
const RING_WIDTH: f32 = 5.0;
/// All the gaps together never take more than this many degrees.
const MAX_GAPS_DEG: f32 = 270.0;

fn ring(start_deg: f32, end_deg: f32) -> String {
    paths::annulus_sector(
        CENTRE,
        RING_RADIUS - RING_WIDTH / 2.0,
        RING_RADIUS + RING_WIDTH / 2.0,
        start_deg,
        end_deg,
    )
}

/// The arcs of the donut: one group = a full ring, none = one grey ring in `fallback`.
pub fn donut(groups: &[GroupRow], fallback: slint::Color) -> Vec<DonutArc> {
    let shown: Vec<&GroupRow> = groups.iter().filter(|group| group.count > 0).collect();
    let total: f32 = shown.iter().map(|group| group.count as f32).sum();
    match shown.as_slice() {
        [] => vec![DonutArc {
            commands: ring(0.0, 360.0).into(),
            colour: fallback,
        }],
        [only] => vec![DonutArc {
            commands: ring(0.0, 360.0).into(),
            colour: only.colour,
        }],
        _ => {
            let gap = GAP_DEG.min(MAX_GAPS_DEG / shown.len() as f32);
            let share = 360.0 - gap * shown.len() as f32;
            let mut start = 0.0;
            shown
                .iter()
                .map(|group| {
                    let sweep = share * group.count as f32 / total;
                    let arc = DonutArc {
                        commands: ring(start, start + sweep).into(),
                        colour: group.colour,
                    };
                    start += sweep + gap;
                    arc
                })
                .collect()
        }
    }
}

/// "Tab groups, 6 groups, 22 tabs" ("1 group", "1 tab"; no groups: "Tab groups, no tabs").
pub fn label(groups: &[GroupRow]) -> String {
    label_with_pinned(groups, 0)
}

/// [`label`] for a window that also has `pinned` pinned tabs (which belong to no group), so that
/// the label counts the same tabs as the search field's placeholder: "Tab groups, 6 groups, 25
/// tabs" is 22 tabs in groups plus 3 pinned. With pinned tabs only: "Tab groups, no groups, 3
/// tabs"; with nothing: "Tab groups, no tabs".
pub fn label_with_pinned(groups: &[GroupRow], pinned: usize) -> String {
    let shown = groups.iter().filter(|group| group.count > 0).count();
    let in_groups: usize = groups
        .iter()
        .filter(|group| group.count > 0)
        .map(|group| usize::try_from(group.count).unwrap_or(0))
        .sum();
    let tabs = in_groups.saturating_add(pinned);
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    if tabs == 0 {
        "Tab groups, no tabs".to_owned()
    } else if shown == 0 {
        format!("Tab groups, no groups, {tabs} tab{}", plural(tabs))
    } else {
        format!(
            "Tab groups, {shown} group{}, {tabs} tab{}",
            plural(shown),
            plural(tabs)
        )
    }
}
