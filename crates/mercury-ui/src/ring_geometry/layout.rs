// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Where every tab and group sits on a ring (angles only; `paths.rs` turns them into shapes).
//!
//! The rule is the one of the mockup generator (`design/ui-concepts/_source/gen.swift`,
//! `ringMid`/`ring`), re-derived here: every tab gets the same angle (`pitch`), the groups are
//! separated by `group_gap_deg`, and `reserve_deg` stays free at 12 o'clock.
//! A STARTER written by the lane lead; the work item "layout and hit" verifies it against the
//! mockup's numbers, adds the degradation policy for crowded rings and the `RingSpec` presets.

use super::types::{GroupArc, RingLayout, RingSpec, TabArc};

impl RingLayout {
    /// Lays out one ring. `group_sizes[g]` is the number of tabs of group `g`, in ring order.
    pub fn new(spec: RingSpec, group_sizes: &[usize]) -> Self {
        let total: usize = group_sizes.iter().sum();
        let full = spec.span_deg >= 360.0;
        let live = group_sizes.iter().filter(|&&n| n > 0).count();
        let gaps = if full { live } else { live.saturating_sub(1) } as f32;
        let free = (spec.span_deg - spec.reserve_deg - spec.group_gap_deg * gaps).max(0.0);
        let pitch = if total == 0 { 0.0 } else { free / total as f32 };
        // How far each segment is drawn inside its slot (the mockup's `sg`): half the tab gap,
        // or less when the slots are tiny.
        let inset = if pitch > 4.0 {
            spec.tab_gap_deg / 2.0
        } else {
            (pitch * 0.12).min(0.35)
        };

        let mut angle = spec.start_deg
            + spec.reserve_deg / 2.0
            + if full { spec.group_gap_deg / 2.0 } else { 0.0 };
        let mut tabs = Vec::with_capacity(total);
        let mut groups = Vec::with_capacity(group_sizes.len());
        for (group, &count) in group_sizes.iter().enumerate() {
            let group_start = angle;
            let first_tab = tabs.len();
            for index_in_group in 0..count {
                let slot_start = angle + index_in_group as f32 * pitch;
                let slot_end = slot_start + pitch;
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
            let group_end = group_start + count as f32 * pitch;
            groups.push(GroupArc {
                group,
                first_tab,
                tab_count: count,
                start_deg: group_start,
                end_deg: group_end,
                label_start_deg: (group_start + spec.label_inset_deg).min(group_end),
                label_end_deg: (group_end - spec.label_inset_deg).max(group_start),
            });
            if count > 0 {
                angle = group_end + spec.group_gap_deg;
            }
        }
        Self {
            spec,
            pitch_deg: pitch,
            tabs,
            groups,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hub ring of the mockup `ring-open`: 22 tabs in groups of 5, 4, 6, 3, 2 and 2.
    fn mockup() -> RingLayout {
        RingLayout::new(RingSpec::default(), &[5, 4, 6, 3, 2, 2])
    }

    #[test]
    fn the_first_segment_starts_where_the_mockup_draws_it() {
        let layout = mockup();
        // 12 (half the reserve) + 2.5 (half a group gap) + 0.6 (half a tab gap) = 15.1 degrees.
        assert!((layout.tabs[0].start_deg - 15.1).abs() < 1e-3);
        // Every segment is (306 / 22) - 1.2 = 12.709 degrees wide (measured: 12.707).
        let width = layout.tabs[0].end_deg - layout.tabs[0].start_deg;
        assert!((width - 12.709).abs() < 1e-2);
    }

    #[test]
    fn the_ring_closes_on_the_reserve() {
        let layout = mockup();
        let last = layout.groups.last().unwrap();
        // The last group ends half a group gap and half the reserve before a full turn.
        assert!((last.end_deg + 2.5 + 12.0 - 360.0).abs() < 1e-3);
        assert_eq!(layout.tabs.len(), 22);
    }
}
