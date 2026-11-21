// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The card's models for Slint: the one file of the card's Rust side that names generated Slint
//! types (lane brief §6 item 3). The rings come from [`crate::ring_geometry`]
//! (`RingSpec::planet`, `RingModel`); the layout and words from the sibling modules.
//!
//! [`build_card`] is total: no panic and no `NaN` for no groups, no tabs, 26 groups, 27 labelled
//! groups, 500 tabs in one group, or any name (a name is never cut by bytes; Slint elides it).
//!
//! Conventions the `KeySwitcher` component relies on:
//!
//! - `KeyTile.x` and `y` are from the tile area's top-left (the card's `tiles-left` and
//!   `tiles-top`), as the frozen note of `KeyTile` says; `CardLayout::positions` are from the
//!   card's top-left.
//! - Every ring path is ring-local to a square box of `KeyMetrics.ring-size` that is **centred on
//!   the ring's centre** (`ring-cx`, `ring-cy`): the box's top-left in the tile is
//!   `(ring-cx - ring-size / 2, ring-cy - ring-size / 2)`.
//! - `mark-x` and `mark-y` are the **centre** of the active tab's dot, in tile coordinates.

use std::collections::{HashMap, HashSet};

use slint::{ModelRc, VecModel};

use crate::key_switcher::layout::{CardLayout, MAX_TILES, TileMetrics, card_layout};
use crate::key_switcher::text::{
    HintMode, chip_label, count_text, empty_text, hint_primary, hint_secondary, tile_label,
};
use crate::ring_geometry::slint_models::RingModel;
use crate::ring_geometry::{Point, RingSpec};
use crate::{GroupRow, KeyChip, KeyMetrics, KeyRingSeg, KeyTile, TabRow};

/// The most segments one tile's ring draws. A group with more tabs shows its first 39 and the
/// active tab as the 40th when it lies beyond.
pub const MAX_RING_TABS: usize = 40;

/// The most tab chips: the digits reach only 1 to 9.
const MAX_CHIPS: usize = 9;

/// How far from the ring's centre the active tab's dot sits, as a share of the ring's radius: the
/// mockup has it 10.5 px out on a ring of radius 20.
const MARK_REACH: f32 = 0.525;

/// What the card is built from.
#[derive(Clone, Copy, Debug)]
pub struct CardInput<'a> {
    /// The window's groups in ring order; a group whose `key-label` is empty is not a tile.
    pub groups: &'a [GroupRow],
    /// Every tab of those groups (`TabRow.group` is a `GroupRow.id`).
    pub tabs: &'a [TabRow],
    /// The hold setting in milliseconds, for the second hint.
    pub hold_ms: u16,
    /// The capture mode, for the second hint.
    pub mode: HintMode,
    /// The chosen group's `GroupRow.id`, if any.
    pub selected_group: Option<i32>,
    /// The chosen tab as a digit `1..=9`, if any.
    pub selected_tab: Option<u8>,
}

/// Everything the `KeySwitcher` component shows, ready to set.
#[derive(Clone, Debug)]
pub struct CardModel {
    /// One tile per keyed group, in order, at most 26.
    pub tiles: Vec<KeyTile>,
    /// The numbered tab chips of the chosen group (at most 9), empty without a chosen group.
    pub chips: Vec<KeyChip>,
    /// The tile measures for the component.
    pub metrics: KeyMetrics,
    /// The layout the model was built with.
    pub layout: CardLayout,
    /// The first hint.
    pub hint_primary: String,
    /// The second hint.
    pub hint_secondary: String,
    /// The text for an empty tile area.
    pub empty_text: String,
    /// The chosen group's `id`, or -1.
    pub selected_group: i32,
    /// The chosen digit, or -1.
    pub selected_tab: i32,
    /// The chosen group's name, or empty.
    pub chosen_name: String,
}

/// The positions (in the tab list sorted by `index-in-group`) of the tabs a tile's ring draws: all
/// of them up to [`MAX_RING_TABS`]; with more, the first 39, and as the 40th the active tab when it
/// lies beyond them (else the 40th tab). `active` is the position of the first active tab.
fn ring_tab_positions(len: usize, active: Option<usize>) -> Vec<usize> {
    if len <= MAX_RING_TABS {
        return (0..len).collect();
    }
    let last = active
        .filter(|&position| (MAX_RING_TABS - 1..len).contains(&position))
        .unwrap_or(MAX_RING_TABS - 1);
    let mut positions: Vec<usize> = (0..MAX_RING_TABS - 1).collect();
    positions.push(last);
    positions
}

/// Where the active tab's dot sits in the tile: [`MARK_REACH`] of the ring's radius from the ring's
/// centre, towards `mid` (the middle of the active segment, ring-local). A `mid` on the centre
/// or not a number puts the dot straight above the centre.
fn mark_position(metrics: &TileMetrics, mid: (f32, f32)) -> (f32, f32) {
    let half = metrics.ring_size / 2.0;
    let (dx, dy) = (mid.0 - half, mid.1 - half);
    let length = dx.hypot(dy);
    let (ux, uy) = if length.is_finite() && length > 1e-3 {
        (dx / length, dy / length)
    } else {
        (0.0, -1.0)
    };
    let reach = metrics.ring_radius * MARK_REACH;
    (metrics.ring_cx + ux * reach, metrics.ring_cy + uy * reach)
}

/// The tabs of every group, each list sorted by `index-in-group` (equal values keep their order).
fn tabs_by_group(tabs: &[TabRow]) -> HashMap<i32, Vec<&TabRow>> {
    let mut by_group: HashMap<i32, Vec<&TabRow>> = HashMap::new();
    for tab in tabs {
        by_group.entry(tab.group).or_default().push(tab);
    }
    for list in by_group.values_mut() {
        list.sort_by_key(|tab| tab.index_in_group);
    }
    by_group
}

/// One group's tile: its ring (the planet ring of the ring geometry, in the box that is centred on
/// the ring's centre), the dot of the active tab and the words. `tabs` are the group's tabs in
/// order; `position` is the tile's top-left from the card's top-left.
fn build_tile(
    group: &GroupRow,
    tabs: &[&TabRow],
    position: (f32, f32),
    layout: &CardLayout,
) -> KeyTile {
    let metrics = &layout.metrics;
    let half = metrics.ring_size / 2.0;
    let active = tabs.iter().position(|tab| tab.active);
    let ring_rows: Vec<TabRow> = ring_tab_positions(tabs.len(), active)
        .into_iter()
        .map(|index| tabs[index].clone())
        .collect();
    let spec = RingSpec::planet(
        Point::new(half, half),
        metrics.ring_radius,
        metrics.ring_band,
    );
    let ring = RingModel::new(spec, std::slice::from_ref(group), &ring_rows);
    let arcs = ring.segments_for_slint();
    // One arc per tab in ring order: `ring.tab_rows[flat]` is the row (of `ring_rows`) of arc `flat`.
    let row_of = |flat: usize| ring.tab_rows.get(flat).and_then(|&row| ring_rows.get(row));
    let segments: Vec<KeyRingSeg> = arcs
        .iter()
        .enumerate()
        .map(|(flat, arc)| KeyRingSeg {
            commands: arc.commands.clone(),
            shade: row_of(flat).map(|tab| tab.shade).unwrap_or_default(),
        })
        .collect();
    let mark = arcs
        .iter()
        .enumerate()
        .find(|(flat, _)| row_of(*flat).is_some_and(|tab| tab.active))
        .map(|(_, arc)| mark_position(metrics, (arc.mid_x, arc.mid_y)));
    let (mark_x, mark_y) = mark.unwrap_or_default();
    let count = usize::try_from(group.count).unwrap_or(0);
    KeyTile {
        group: group.id,
        x: position.0 - layout.tiles_left,
        y: position.1 - layout.tiles_top,
        key_label: group.key_label.clone(),
        name: group.name.clone(),
        count_text: count_text(count).into(),
        accessible_label: tile_label(&group.key_label, &group.name, count).into(),
        tint: group.tint,
        segments: ModelRc::new(VecModel::from(segments)),
        has_mark: mark.is_some(),
        mark_x,
        mark_y,
    }
}

/// The measures the component needs, as the Slint struct.
fn key_metrics(metrics: &TileMetrics) -> KeyMetrics {
    KeyMetrics {
        tile_width: metrics.tile_width,
        tile_height: metrics.tile_height,
        ring_size: metrics.ring_size,
        ring_cx: metrics.ring_cx,
        ring_cy: metrics.ring_cy,
        halo_outer: metrics.halo_outer,
        halo_width: metrics.halo_width,
        key_size: metrics.key_size,
        key_y: metrics.key_y,
        key_radius: metrics.key_radius,
        key_font: metrics.key_font,
        name_y: metrics.name_y,
        name_font: metrics.name_font,
        count_y: metrics.count_y,
        count_font: metrics.count_font,
    }
}

/// Builds the card's models from the rows.
///
/// - **Tiles** are the groups whose `key-label` is not empty, in the order given, at most 26. Two
///   groups with the same label (compared without regard to case) are one key: the first stays.
///   `KeyTile.group` is `GroupRow.id`; the count text and label use `GroupRow.count`.
/// - **Rings** hold one segment per tab (the tab's `shade`), ordered by `index-in-group`; at most
///   [`MAX_RING_TABS`] (see there). The active tab's dot is `has-mark`.
/// - **Chips** exist only while `selected_group` names a tile: the first 9 tabs of that group by
///   `index-in-group`, numbered from 1 in that order (the machine's `Target::Tab` index is that
///   position). `selected_tab` above the chip count (or without a chosen group) becomes -1.
/// - `chosen_name` is the chosen group's name; `selected_group` is -1 when it names no tile.
pub fn build_card(input: &CardInput) -> CardModel {
    let mut labels = HashSet::new();
    let keyed: Vec<&GroupRow> = input
        .groups
        .iter()
        .filter(|group| !group.key_label.is_empty())
        .filter(|group| labels.insert(group.key_label.to_lowercase()))
        .take(MAX_TILES)
        .collect();
    let layout = card_layout(keyed.len());
    let by_group = tabs_by_group(input.tabs);
    let tabs_of = |group: &GroupRow| by_group.get(&group.id).map_or(&[][..], Vec::as_slice);

    let tiles: Vec<KeyTile> = keyed
        .iter()
        .zip(&layout.positions)
        .map(|(group, &position)| build_tile(group, tabs_of(group), position, &layout))
        .collect();

    let chosen = input
        .selected_group
        .and_then(|id| keyed.iter().find(|group| group.id == id));
    let chips: Vec<KeyChip> = chosen
        .map(|group| {
            tabs_of(group)
                .iter()
                .take(MAX_CHIPS)
                .enumerate()
                .map(|(index, tab)| KeyChip {
                    index: index as i32 + 1,
                    title: tab.title.clone(),
                    shade: tab.shade,
                    accessible_label: chip_label(index + 1, &tab.title).into(),
                })
                .collect()
        })
        .unwrap_or_default();
    let selected_tab = input
        .selected_tab
        .map(usize::from)
        .filter(|digit| (1..=chips.len()).contains(digit))
        .map_or(-1, |digit| digit as i32);

    CardModel {
        metrics: key_metrics(&layout.metrics),
        tiles,
        chips,
        hint_primary: hint_primary().to_owned(),
        hint_secondary: hint_secondary(input.mode, input.hold_ms),
        empty_text: empty_text().to_owned(),
        selected_group: chosen.map_or(-1, |group| group.id),
        selected_tab,
        chosen_name: chosen.map_or_else(String::new, |group| group.name.to_string()),
        layout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key_switcher::layout::card_layout;

    fn tab(id: i32, group: i32, index_in_group: i32) -> TabRow {
        TabRow {
            id,
            group,
            index_in_group,
            ..Default::default()
        }
    }

    #[test]
    fn a_ring_with_up_to_40_tabs_draws_them_all() {
        assert_eq!(ring_tab_positions(0, None), Vec::<usize>::new());
        assert_eq!(ring_tab_positions(1, Some(0)), vec![0]);
        assert_eq!(ring_tab_positions(5, Some(2)), vec![0, 1, 2, 3, 4]);
        assert_eq!(ring_tab_positions(40, None), (0..40).collect::<Vec<_>>());
        assert_eq!(
            ring_tab_positions(40, Some(39)),
            (0..40).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_ring_with_more_tabs_draws_39_and_the_active_tab_as_the_40th_when_it_lies_beyond() {
        let first_39: Vec<usize> = (0..39).collect();
        let with = |last: usize| [first_39.clone(), vec![last]].concat();
        // No active tab, or one among the first 39: the 40th tab fills the last place.
        assert_eq!(ring_tab_positions(41, None), with(39));
        assert_eq!(ring_tab_positions(41, Some(0)), with(39));
        assert_eq!(ring_tab_positions(41, Some(38)), with(39));
        // The active tab is the 40th or beyond: it takes the last place.
        assert_eq!(ring_tab_positions(41, Some(39)), with(39));
        assert_eq!(ring_tab_positions(41, Some(40)), with(40));
        assert_eq!(ring_tab_positions(60, Some(50)), with(50));
        assert_eq!(ring_tab_positions(500, Some(499)), with(499));
    }

    #[test]
    fn the_ring_positions_are_ascending_unique_and_in_range() {
        for len in [0usize, 1, 39, 40, 41, 100, 500] {
            for active in [
                None,
                Some(0),
                Some(39),
                Some(len.saturating_sub(1)),
                Some(len + 5),
            ] {
                let positions = ring_tab_positions(len, active);
                assert!(positions.len() <= MAX_RING_TABS, "{len} {active:?}");
                assert_eq!(positions.len(), len.min(MAX_RING_TABS), "{len} {active:?}");
                assert!(
                    positions.windows(2).all(|pair| pair[0] < pair[1]),
                    "{len} {active:?}"
                );
                // An active position past the end is not a tab: it never gets drawn.
                assert!(positions.iter().all(|&p| p < len), "{len} {active:?}");
            }
        }
    }

    #[test]
    fn the_mark_is_about_half_the_ring_radius_from_the_centre_on_the_side_of_the_segment() {
        let metrics = card_layout(3).metrics;
        let half = metrics.ring_size / 2.0;
        // The segment's middle straight above, to the right, below and to the left of the centre.
        for (mid, direction) in [
            ((half, half - 20.0), (0.0, -1.0)),
            ((half + 20.0, half), (1.0, 0.0)),
            ((half, half + 20.0), (0.0, 1.0)),
            ((half - 20.0, half), (-1.0, 0.0)),
        ] {
            let (x, y) = mark_position(&metrics, mid);
            let (dx, dy) = (x - metrics.ring_cx, y - metrics.ring_cy);
            assert!((dx - direction.0 * 10.5).abs() < 0.01, "{mid:?}: dx {dx}");
            assert!((dy - direction.1 * 10.5).abs() < 0.01, "{mid:?}: dy {dy}");
        }
        // The mockup's first Research segment: mid angle 36 degrees, mark at (+6.2, -8.5).
        let rad = 36.0_f32.to_radians();
        let mid = (half + 20.0 * rad.sin(), half - 20.0 * rad.cos());
        let (x, y) = mark_position(&metrics, mid);
        assert!(
            (x - (54.0 + 6.2)).abs() < 0.1 && (y - (46.0 - 8.5)).abs() < 0.1,
            "{x} {y}"
        );
    }

    #[test]
    fn the_mark_keeps_its_distance_for_the_compact_ring_and_for_a_mid_on_the_centre() {
        let metrics = card_layout(12).metrics;
        let half = metrics.ring_size / 2.0;
        let (x, y) = mark_position(&metrics, (half + 14.0, half));
        assert!(
            (x - (metrics.ring_cx + 14.0 * 0.525)).abs() < 0.01
                && (y - metrics.ring_cy).abs() < 0.01
        );
        // A degenerate mid (on the centre, or not a number) still gives a finite point above it.
        for mid in [(half, half), (f32::NAN, 3.0), (f32::INFINITY, 0.0)] {
            let (x, y) = mark_position(&metrics, mid);
            assert!(x.is_finite() && y.is_finite(), "{mid:?}");
            assert!((x - metrics.ring_cx).abs() < 0.01);
            assert!((y - (metrics.ring_cy - 14.0 * 0.525)).abs() < 0.01);
        }
    }

    #[test]
    fn tabs_are_grouped_by_group_and_sorted_by_index_in_group_keeping_the_order_of_equals() {
        let tabs = [
            tab(10, 1, 2),
            tab(11, 2, 0),
            tab(12, 1, 0),
            tab(13, 1, 0),
            tab(14, 1, 1),
            tab(15, 3, -4),
            tab(16, 3, -5),
        ];
        let by_group = tabs_by_group(&tabs);
        let ids = |group: i32| -> Vec<i32> { by_group[&group].iter().map(|t| t.id).collect() };
        assert_eq!(ids(1), vec![12, 13, 14, 10]);
        assert_eq!(ids(2), vec![11]);
        assert_eq!(ids(3), vec![16, 15]);
        assert!(!by_group.contains_key(&4));
        assert!(tabs_by_group(&[]).is_empty());
    }
}
