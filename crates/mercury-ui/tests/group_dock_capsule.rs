// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The shrunk dock's slivers (brief D14): one 6 px sliver per tab in group and tab order at rest,
//! narrower and then shared between tabs of one group when the capsule is full, the true count, and
//! the accessible label.

use mercury_ui::group_dock::{
    CAPSULE_WIDTH, CHEVRON_X, COUNT_WIDTH, COUNT_X, CapsuleFrame, MIN_SLIVER_WIDTH, SLIVER_GAP,
    SLIVER_LEFT, SLIVER_RIGHT, SLIVER_WIDTH, capsule_frame, capsule_label,
};
use mercury_ui::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
use mercury_ui::{GroupRow, TabRow};

const EPS: f32 = 1e-3;

fn group(id: i32) -> GroupRow {
    GroupRow {
        id,
        name: format!("Group {id}").into(),
        ..Default::default()
    }
}

fn tab(id: i32, group: i32, index_in_group: i32) -> TabRow {
    TabRow {
        id,
        group,
        index_in_group,
        ..Default::default()
    }
}

fn group_count(tabs: &[TabRow]) -> usize {
    let mut ids: Vec<i32> = tabs.iter().map(|tab| tab.group).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.len()
}

/// The properties of every capsule: inside the area, at least 2 px, no overlap, in order, the true
/// count, at least one sliver per group.
fn assert_sound(frame: &CapsuleFrame, tabs: &[TabRow], groups: usize) {
    assert_eq!(frame.tab_count, tabs.len());
    assert_eq!(frame.group_count, groups);
    assert_eq!(frame.count_text.as_str(), tabs.len().to_string());
    assert_eq!(
        frame.label.as_str(),
        capsule_label(tabs.len(), groups),
        "label"
    );
    assert!(frame.slivers.len() >= groups);
    assert!(frame.slivers.len() <= tabs.len());
    let mut previous_right = SLIVER_LEFT;
    for (i, sliver) in frame.slivers.iter().enumerate() {
        assert!(sliver.x >= SLIVER_LEFT - EPS, "{i}: x {}", sliver.x);
        assert!(
            sliver.x + sliver.width <= SLIVER_RIGHT + EPS,
            "{i}: right {}",
            sliver.x + sliver.width
        );
        assert!(
            sliver.width >= MIN_SLIVER_WIDTH - EPS,
            "{i}: {}",
            sliver.width
        );
        assert!(sliver.width <= SLIVER_WIDTH + EPS, "{i}: {}", sliver.width);
        assert!(
            sliver.x >= previous_right - EPS,
            "{i} overlaps its left neighbour"
        );
        previous_right = sliver.x + sliver.width;
    }
}

fn mockup() -> (Vec<GroupRow>, Vec<TabRow>) {
    (mockup_groups(false), mockup_tabs(false))
}

#[test]
fn the_areas_of_the_capsule_are_pinned() {
    assert_eq!(CAPSULE_WIDTH, 236.0);
    assert_eq!(SLIVER_LEFT, 14.0);
    assert_eq!(SLIVER_RIGHT, 176.0);
    assert_eq!(COUNT_X, 182.0);
    assert_eq!(COUNT_WIDTH, 26.0);
    assert_eq!(CHEVRON_X, 212.0);
    assert_eq!(SLIVER_WIDTH, 6.0);
    assert_eq!(SLIVER_GAP, 1.0);
    assert_eq!(MIN_SLIVER_WIDTH, 2.0);
    const { assert!(SLIVER_RIGHT <= COUNT_X) };
    const { assert!(COUNT_X + COUNT_WIDTH <= CHEVRON_X) };
    const { assert!(CHEVRON_X < CAPSULE_WIDTH) };
}

#[test]
fn the_mockup_has_22_slivers_of_6_px_with_a_gap() {
    let (groups, tabs) = mockup();
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 6);
    assert_eq!(frame.slivers.len(), 22);
    assert_eq!(frame.tab_count, 22);
    assert_eq!(frame.group_count, 6);
    assert_eq!(frame.count_text.as_str(), "22");
    assert_eq!(
        frame.label.as_str(),
        "Show the group dock, 22 tabs in 6 groups"
    );
    for (i, sliver) in frame.slivers.iter().enumerate() {
        assert_eq!(sliver.width, SLIVER_WIDTH, "{i}");
        assert!(
            (sliver.x - (SLIVER_LEFT + i as f32 * (SLIVER_WIDTH + SLIVER_GAP))).abs() < EPS,
            "{i}: x {}",
            sliver.x
        );
    }
    let last = &frame.slivers[21];
    assert!(last.x + last.width <= SLIVER_RIGHT);
}

#[test]
fn the_mockup_slivers_are_in_group_order_and_tab_shades() {
    let (groups, tabs) = mockup();
    let frame = capsule_frame(&groups, &tabs);
    // mockup_tabs lists the tabs in group order, then index order
    for (sliver, tab) in frame.slivers.iter().zip(&tabs) {
        assert_eq!(sliver.tab, tab.id);
        assert_eq!(sliver.shade, tab.shade);
        assert_eq!(sliver.active, tab.active);
    }
    assert_eq!(frame.slivers.iter().filter(|s| s.active).count(), 1);
}

#[test]
fn the_order_is_the_order_of_the_groups_and_then_the_index_in_the_group() {
    let groups = vec![group(7), group(3)];
    let tabs = vec![
        tab(30, 3, 0),
        tab(71, 7, 1),
        tab(31, 3, 1),
        tab(70, 7, 0),
        tab(72, 7, 2),
    ];
    let frame = capsule_frame(&groups, &tabs);
    let order: Vec<i32> = frame.slivers.iter().map(|s| s.tab).collect();
    assert_eq!(order, vec![70, 71, 72, 30, 31]);
    assert_eq!(frame.group_count, 2);
}

#[test]
fn tabs_with_the_same_index_keep_their_given_order() {
    let groups = vec![group(1)];
    let tabs = vec![tab(10, 1, 0), tab(11, 1, 0), tab(12, 1, 0)];
    let frame = capsule_frame(&groups, &tabs);
    let order: Vec<i32> = frame.slivers.iter().map(|s| s.tab).collect();
    assert_eq!(order, vec![10, 11, 12]);
}

#[test]
fn rows_of_unknown_groups_are_ignored() {
    let groups = vec![group(1)];
    let tabs = vec![tab(10, 1, 0), tab(20, 2, 0), tab(21, 2, 1)];
    let frame = capsule_frame(&groups, &tabs);
    assert_eq!(frame.slivers.len(), 1);
    assert_eq!(frame.tab_count, 1);
    assert_eq!(frame.group_count, 1);
    assert_eq!(frame.count_text.as_str(), "1");
}

#[test]
fn a_group_without_a_tab_is_not_counted() {
    let groups = vec![group(1), group(2), group(3)];
    let tabs = vec![tab(10, 1, 0), tab(30, 3, 0)];
    let frame = capsule_frame(&groups, &tabs);
    assert_eq!(frame.group_count, 2);
    assert_eq!(
        frame.label.as_str(),
        "Show the group dock, 2 tabs in 2 groups"
    );
}

#[test]
fn one_tab() {
    let groups = vec![group(1)];
    let tabs = vec![tab(5, 1, 0)];
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 1);
    assert_eq!(frame.slivers.len(), 1);
    assert_eq!(frame.slivers[0].x, SLIVER_LEFT);
    assert_eq!(frame.slivers[0].width, SLIVER_WIDTH);
    assert_eq!(frame.slivers[0].tab, 5);
    assert_eq!(frame.count_text.as_str(), "1");
    assert_eq!(
        frame.label.as_str(),
        "Show the group dock, 1 tab in 1 group"
    );
}

#[test]
fn no_tabs() {
    for (groups, tabs) in [
        (Vec::new(), Vec::new()),
        (vec![group(1), group(2)], Vec::new()),
        (Vec::new(), vec![tab(1, 1, 0)]),
    ] {
        let frame = capsule_frame(&groups, &tabs);
        assert!(frame.slivers.is_empty());
        assert_eq!(frame.tab_count, 0);
        assert_eq!(frame.group_count, 0);
        assert_eq!(frame.count_text.as_str(), "0");
        assert_eq!(
            frame.label.as_str(),
            "Show the group dock, 0 tabs in 0 groups"
        );
    }
}

#[test]
fn the_label_is_singular_for_one() {
    assert_eq!(
        capsule_label(22, 6),
        "Show the group dock, 22 tabs in 6 groups"
    );
    assert_eq!(capsule_label(1, 1), "Show the group dock, 1 tab in 1 group");
    assert_eq!(
        capsule_label(0, 0),
        "Show the group dock, 0 tabs in 0 groups"
    );
    assert_eq!(
        capsule_label(2, 1),
        "Show the group dock, 2 tabs in 1 group"
    );
    assert_eq!(
        capsule_label(1, 2),
        "Show the group dock, 1 tab in 2 groups"
    );
}

#[test]
fn the_widest_capsule_at_rest_is_23_slivers() {
    let (groups, tabs) = generated(3, 23, false);
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 3);
    assert_eq!(frame.slivers.len(), 23);
    assert!(frame.slivers.iter().all(|s| s.width == SLIVER_WIDTH));
    let (groups, tabs) = generated(3, 24, false);
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 3);
    assert_eq!(frame.slivers.len(), 24, "one per tab while 2 px fit");
    assert!(frame.slivers.iter().all(|s| s.width <= SLIVER_WIDTH));
}

#[test]
fn slivers_shrink_without_a_gap_when_the_area_is_full() {
    let (groups, tabs) = generated(5, 60, false);
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 5);
    assert_eq!(frame.slivers.len(), 60);
    let width = (SLIVER_RIGHT - SLIVER_LEFT) / 60.0;
    for pair in frame.slivers.windows(2) {
        assert!((pair[0].width - width).abs() < EPS);
        assert!(
            (pair[1].x - (pair[0].x + pair[0].width)).abs() < EPS,
            "no gap"
        );
    }
    assert!(width >= MIN_SLIVER_WIDTH);
}

#[test]
fn sixty_tabs() {
    let (groups, tabs) = generated(6, 60, false);
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 6);
    assert_eq!(frame.slivers.len(), 60);
    assert_eq!(frame.count_text.as_str(), "60");
}

#[test]
fn two_hundred_tabs_share_slivers_inside_one_group() {
    let (groups, tabs) = generated(6, 200, false);
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 6);
    assert_eq!(frame.tab_count, 200);
    assert_eq!(frame.count_text.as_str(), "200");
    assert_eq!(
        frame.label.as_str(),
        "Show the group dock, 200 tabs in 6 groups"
    );
    assert!(frame.slivers.len() < 200);
    assert_eq!(frame.slivers.len(), 81);
    assert_each_sliver_stands_for_one_group(&frame, &groups, &tabs);
}

#[test]
fn sixty_four_groups_of_three_tabs() {
    let (groups, tabs) = generated(64, 192, false);
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 64);
    assert_eq!(frame.tab_count, 192);
    assert_eq!(frame.count_text.as_str(), "192");
    assert!(frame.slivers.len() >= 64);
    assert_each_sliver_stands_for_one_group(&frame, &groups, &tabs);
}

#[test]
fn sixty_four_groups_of_one_tab_each_get_a_sliver() {
    let (groups, tabs) = generated(64, 64, false);
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 64);
    assert_eq!(frame.slivers.len(), 64);
}

#[test]
fn a_crowded_capsule_keeps_a_sliver_for_every_group_and_the_big_groups_get_more() {
    // one group of 150 tabs next to five single tabs
    let mut groups = vec![group(1)];
    let mut tabs: Vec<TabRow> = (0..150).map(|i| tab(100 + i, 1, i)).collect();
    for g in 2..=6 {
        groups.push(group(g));
        tabs.push(tab(1000 + g, g, 0));
    }
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 6);
    assert_eq!(frame.slivers.len(), 81);
    for g in 2..=6 {
        assert!(frame.slivers.iter().any(|s| s.tab == 1000 + g), "group {g}");
    }
    assert_each_sliver_stands_for_one_group(&frame, &groups, &tabs);
}

#[test]
fn a_shared_sliver_is_the_first_tab_it_stands_for_and_is_active_when_any_is() {
    let groups = vec![group(1)];
    let mut tabs: Vec<TabRow> = (0..120).map(|i| tab(i + 1, 1, i)).collect();
    tabs[100].active = true;
    let frame = capsule_frame(&groups, &tabs);
    assert_sound(&frame, &tabs, 1);
    assert_eq!(frame.slivers.len(), 81);
    assert_eq!(frame.slivers[0].tab, 1);
    let ids: Vec<i32> = frame.slivers.iter().map(|s| s.tab).collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "in tab order");
    assert_eq!(frame.slivers.iter().filter(|s| s.active).count(), 1);
    let active = frame.slivers.iter().find(|s| s.active);
    let active = active.map(|s| s.tab).unwrap_or_default();
    let next = ids.iter().copied().find(|&id| id > active);
    // the active tab (id 101) is inside the run the active sliver starts
    assert!(active <= 101 && next.is_none_or(|n| n > 101));
}

#[test]
fn more_groups_than_the_area_holds_still_get_one_sliver_each() {
    let (groups, tabs) = generated(100, 100, false);
    let frame = capsule_frame(&groups, &tabs);
    assert_eq!(frame.slivers.len(), 100);
    assert_eq!(frame.group_count, 100);
    let mut previous_right = SLIVER_LEFT;
    for sliver in &frame.slivers {
        assert!(sliver.x >= previous_right - EPS);
        assert!(sliver.width > 0.0);
        assert!(sliver.x + sliver.width <= SLIVER_RIGHT + EPS);
        previous_right = sliver.x + sliver.width;
    }
}

#[test]
fn the_frame_is_a_pure_function_of_the_rows() {
    let (groups, tabs) = generated(9, 77, false);
    let a = capsule_frame(&groups, &tabs);
    let b = capsule_frame(&groups, &tabs);
    assert_eq!(a.slivers.len(), b.slivers.len());
    for (x, y) in a.slivers.iter().zip(&b.slivers) {
        assert_eq!((x.tab, x.x, x.width), (y.tab, y.x, y.width));
    }
}

#[test]
fn every_size_up_to_300_tabs_is_sound() {
    for n in 0..=300 {
        for g in [1, 2, 6, 20, 64] {
            let (groups, tabs) = generated(g, n, false);
            let frame = capsule_frame(&groups, &tabs);
            assert_sound(&frame, &tabs, group_count(&tabs));
            assert_each_sliver_stands_for_one_group(&frame, &groups, &tabs);
        }
    }
}

/// Each sliver's tab belongs to a group, slivers of one group are consecutive, and the groups come
/// in the order of `groups`.
fn assert_each_sliver_stands_for_one_group(
    frame: &CapsuleFrame,
    groups: &[GroupRow],
    tabs: &[TabRow],
) {
    let mut last_group_position = 0;
    let mut seen = Vec::new();
    for sliver in &frame.slivers {
        let row = tabs.iter().find(|t| t.id == sliver.tab);
        let Some(row) = row else {
            panic!("sliver of an unknown tab {}", sliver.tab);
        };
        let position = groups.iter().position(|g| g.id == row.group);
        let Some(position) = position else {
            panic!("tab {} is in no group", row.id);
        };
        assert!(position >= last_group_position, "group order");
        if position != last_group_position || seen.is_empty() {
            seen.push(position);
            last_group_position = position;
        }
    }
    let mut once = seen.clone();
    once.dedup();
    assert_eq!(seen, once, "a group's slivers are consecutive");
    assert_eq!(seen.len(), frame.group_count, "a sliver for every group");
}
