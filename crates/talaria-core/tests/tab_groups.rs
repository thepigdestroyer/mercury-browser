// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Tab groups on the strip: creating, moving, reordering, closing and restoring groups, and how
//! the older strip operations (`insert`, `move_tab`, `move_tab_by`, pinning, `adopt_tab`,
//! `open_restored_tab`, closing) behave around group runs.
//!
//! A group is a contiguous run of tabs in the strip (brief D2). Every test reads the strip through
//! [`layout`], which writes `3@1` for tab 3 in group 1, `^2` for a pinned tab and plain `4` for a
//! loose one, and ends by checking the strip invariants (I1-I8) with `group_invariant_violation`.

use std::time::{Duration, SystemTime};

use talaria_core::{
    BrowserCommand, BrowserController, BrowserEvent, CloseTabOutcome, GroupColour, GroupError,
    GroupId, GroupKey, GroupPreset, MAX_GROUPS, RestoredGroup, RestoredTab, Rgb, TabActivation,
    TabId,
};

// ----- helpers -------------------------------------------------------------------------------

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)
}

fn t(number: u64) -> TabId {
    TabId::new(number)
}

fn g(number: u64) -> GroupId {
    GroupId::new(number)
}

fn key(letter: char) -> GroupKey {
    GroupKey::from_letter(letter).expect("a letter")
}

/// A controller with `count` tabs numbered 1..=count (the first one selected).
fn controller_with_tabs(count: u64) -> BrowserController {
    let mut controller = BrowserController::default();
    for number in 2..=count {
        let id = controller
            .open_tab_with(None, TabActivation::Background)
            .expect("a blank tab opens");
        assert_eq!(id, t(number));
    }
    assert_eq!(controller.active_tab_id(), t(1));
    controller
}

/// The strip as text: `^1` pinned, `2@1` in group 1, `3` loose.
fn layout(controller: &BrowserController) -> String {
    controller
        .tabs()
        .map(|tab| {
            let mut text = String::new();
            if tab.is_pinned() {
                text.push('^');
            }
            text.push_str(&tab.id().value().to_string());
            if let Some(group) = tab.group() {
                text.push('@');
                text.push_str(&group.value().to_string());
            }
            text
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn group_ids(controller: &BrowserController) -> Vec<u64> {
    controller
        .groups()
        .iter()
        .map(|group| group.id.value())
        .collect()
}

/// Fails with the description of the first broken strip invariant.
fn check(controller: &BrowserController) {
    if let Some(violation) = controller.group_invariant_violation() {
        panic!("{violation} (strip: {})", layout(controller));
    }
}

/// Makes a group of the numbered tabs with default name, colour and key.
fn make(controller: &mut BrowserController, tabs: &[u64]) -> GroupId {
    let tabs = tabs.iter().copied().map(t).collect::<Vec<_>>();
    let group = controller
        .create_group(&tabs, None, None, now())
        .expect("the group is made");
    check(controller);
    group
}

fn ids(numbers: &[u64]) -> Vec<TabId> {
    numbers.iter().copied().map(t).collect()
}

fn restored_tab(pinned: bool) -> RestoredTab {
    RestoredTab {
        url: None,
        title: "Restored".to_owned(),
        zoom_percent: 100,
        pinned,
    }
}

/// What a refused operation must leave alone.
#[derive(Debug, PartialEq)]
struct Snapshot {
    layout: String,
    groups: Vec<talaria_core::TabGroup>,
    active: TabId,
    next_group_id: u64,
}

fn snapshot(controller: &BrowserController) -> Snapshot {
    Snapshot {
        layout: layout(controller),
        groups: controller.groups().to_vec(),
        active: controller.active_tab_id(),
        next_group_id: controller.next_group_id(),
    }
}

// ----- create_group --------------------------------------------------------------------------

#[test]
fn a_new_group_gathers_its_tabs_where_the_first_of_them_was() {
    let mut controller = controller_with_tabs(6);

    let group = controller
        .create_group(&[t(5), t(2), t(4)], None, None, now())
        .expect("the group is made");

    // Members are taken in strip order, whatever order they were named in.
    assert_eq!(layout(&controller), "1 2@1 4@1 5@1 3 6");
    assert_eq!(controller.group_tabs(group), ids(&[2, 4, 5]));
    assert_eq!(controller.group_of(t(4)), Some(group));
    assert_eq!(controller.group_of(t(3)), None);
    assert_eq!(controller.loose_tabs(), ids(&[1, 3, 6]));
    check(&controller);
}

#[test]
fn a_new_group_takes_the_default_name_colour_and_key() {
    let mut controller = controller_with_tabs(4);

    let first = make(&mut controller, &[1]);
    let second = make(&mut controller, &[2]);

    let first = controller.group(first).expect("the group exists");
    assert_eq!(first.name, "Group 1");
    assert_eq!(first.colour, GroupColour::Preset(GroupPreset::Blue));
    assert_eq!(first.key, Some(key('A')));
    assert_eq!(first.created, now());
    let second = controller.group(second).expect("the group exists");
    assert_eq!(second.name, "Group 2");
    assert_eq!(second.colour, GroupColour::Preset(GroupPreset::Amber));
    assert_eq!(second.key, Some(key('S')));
    assert_eq!(group_ids(&controller), [1, 2]);
}

#[test]
fn a_given_name_and_colour_are_used() {
    let mut controller = controller_with_tabs(3);
    let custom = GroupColour::Custom(Rgb {
        red: 0x2f,
        green: 0x6f,
        blue: 0xd6,
    });

    let group = controller
        .create_group(&[t(2), t(3)], Some("  Reading list "), Some(custom), now())
        .expect("the group is made");

    let group = controller.group(group).expect("the group exists");
    assert_eq!(group.name, "Reading list");
    assert_eq!(group.colour, custom);
}

#[test]
fn creating_a_group_never_changes_the_selection() {
    let mut controller = controller_with_tabs(4);
    assert!(controller.select_tab(t(3)));

    make(&mut controller, &[1, 2, 3]);
    assert_eq!(controller.active_tab_id(), t(3));

    make(&mut controller, &[3, 4]);
    assert_eq!(controller.active_tab_id(), t(3));
}

#[test]
fn a_group_with_a_pinned_tab_is_refused_and_nothing_changes() {
    let mut controller = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(t(2), true));
    let before = snapshot(&controller);

    assert_eq!(
        controller.create_group(&[t(1), t(2), t(3)], None, None, now()),
        Err(GroupError::TabPinned)
    );

    assert_eq!(snapshot(&controller), before);
    assert!(controller.groups().is_empty());
    check(&controller);
}

#[test]
fn unknown_tabs_and_empty_lists_are_refused_in_a_fixed_order() {
    let mut controller = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(t(1), true));
    let before = snapshot(&controller);

    assert_eq!(
        controller.create_group(&[t(2), t(99)], None, None, now()),
        Err(GroupError::TabNotFound)
    );
    // Unknown comes before pinned.
    assert_eq!(
        controller.create_group(&[t(1), t(99)], None, None, now()),
        Err(GroupError::TabNotFound)
    );
    // Pinned comes before an empty result: there is no "no usable tabs" here, only the empty list.
    assert_eq!(
        controller.create_group(&[t(1)], None, None, now()),
        Err(GroupError::TabPinned)
    );
    assert_eq!(
        controller.create_group(&[], None, None, now()),
        Err(GroupError::NoTabs)
    );

    assert_eq!(snapshot(&controller), before);
}

#[test]
fn a_tab_named_twice_is_in_the_group_once() {
    let mut controller = controller_with_tabs(4);

    let group = controller
        .create_group(&[t(3), t(3), t(2), t(3)], None, None, now())
        .expect("the group is made");

    assert_eq!(controller.group_tabs(group), ids(&[2, 3]));
    assert_eq!(layout(&controller), "1 2@1 3@1 4");
    check(&controller);
}

#[test]
fn a_tab_taken_from_the_middle_of_a_group_does_not_split_it() {
    let mut controller = controller_with_tabs(6);
    let first = make(&mut controller, &[2, 3, 4]);

    let second = make(&mut controller, &[3]);

    // The new run starts where tab 3 was, which is inside the first run, so it goes after it (R1).
    assert_eq!(layout(&controller), "1 2@1 4@1 3@2 5 6");
    assert_eq!(group_ids(&controller), [first.value(), second.value()]);
}

#[test]
fn a_tab_taken_from_a_groups_edge_leaves_the_run_whole_on_either_side() {
    // First tab of the group: the new group goes before the old one, and the book follows.
    let mut controller = controller_with_tabs(6);
    let old = make(&mut controller, &[2, 3, 4]);
    let new = make(&mut controller, &[2]);
    assert_eq!(layout(&controller), "1 2@2 3@1 4@1 5 6");
    assert_eq!(group_ids(&controller), [new.value(), old.value()]);

    // Last tab of the group: the new group goes right after it.
    let mut controller = controller_with_tabs(6);
    let old = make(&mut controller, &[2, 3, 4]);
    let new = make(&mut controller, &[4]);
    assert_eq!(layout(&controller), "1 2@1 3@1 4@2 5 6");
    assert_eq!(group_ids(&controller), [old.value(), new.value()]);
}

#[test]
fn members_of_two_groups_gather_in_the_place_of_the_first_member() {
    let mut controller = controller_with_tabs(7);
    make(&mut controller, &[2, 3]);
    make(&mut controller, &[5, 6]);

    let new = make(&mut controller, &[6, 3, 7]);

    // 3 left group 1 (2 stays) and 6 left group 2 (5 stays); the run starts at 3's old place,
    // which is the end edge of group 1's run, so it is not snapped.
    assert_eq!(layout(&controller), "1 2@1 3@3 6@3 7@3 4 5@2");
    assert_eq!(group_ids(&controller), [1, new.value(), 2]);
}

#[test]
fn a_group_emptied_by_creating_another_is_deleted_and_its_key_freed() {
    let mut controller = controller_with_tabs(6);
    let old = make(&mut controller, &[2, 3]);
    assert_eq!(
        controller.group(old).and_then(|group| group.key),
        Some(key('A'))
    );
    let other = make(&mut controller, &[5]);
    assert_eq!(
        controller.group(other).and_then(|group| group.key),
        Some(key('S'))
    );

    let new = make(&mut controller, &[3, 2]);

    assert!(controller.group(old).is_none());
    assert_eq!(group_ids(&controller), [new.value(), other.value()]);
    assert_eq!(layout(&controller), "1 2@3 3@3 4 5@2 6");
    // The freed key and name are free for the new group; the id is not reused.
    let new = controller.group(new).expect("the group exists");
    assert_eq!(new.key, Some(key('A')));
    assert_eq!(new.name, "Group 1");
    assert_eq!(new.id, g(3));
}

#[test]
fn the_limit_of_sixty_four_groups_counts_the_groups_a_call_empties() {
    let mut controller = controller_with_tabs(70);
    for number in 1..=MAX_GROUPS as u64 {
        make(&mut controller, &[number]);
    }
    assert_eq!(controller.groups().len(), MAX_GROUPS);
    let before = snapshot(&controller);

    // A 65th group from loose tabs is refused, and nothing changes.
    assert_eq!(
        controller.create_group(&[t(65)], None, None, now()),
        Err(GroupError::TooManyGroups)
    );
    assert_eq!(
        controller.create_group(&[t(66), t(67)], None, None, now()),
        Err(GroupError::TooManyGroups)
    );
    assert_eq!(snapshot(&controller), before);

    // Tab 1 is alone in its group, so the call empties it: 64 - 1 + 1 = 64.
    let new = make(&mut controller, &[1]);
    assert_eq!(controller.groups().len(), MAX_GROUPS);
    assert_eq!(new, g(65));
    // Two groups emptied, one made.
    let newer = make(&mut controller, &[2, 3]);
    assert_eq!(controller.groups().len(), MAX_GROUPS - 1);
    assert!(controller.group(newer).is_some());
}

#[test]
fn the_limit_is_kept_when_the_call_empties_no_group() {
    let mut controller = controller_with_tabs(70);
    let wide = make(&mut controller, &[1, 2]);
    for number in 3..=(MAX_GROUPS as u64 + 1) {
        make(&mut controller, &[number]);
    }
    assert_eq!(controller.groups().len(), MAX_GROUPS);
    let before = snapshot(&controller);

    // Tab 1 leaves `wide`, but tab 2 stays: no group is emptied, so this would be the 65th.
    assert_eq!(
        controller.create_group(&[t(1)], None, None, now()),
        Err(GroupError::TooManyGroups)
    );
    assert_eq!(snapshot(&controller), before);

    // Both its tabs: the group is emptied, so the count stays at 64.
    make(&mut controller, &[1, 2]);
    assert!(controller.group(wide).is_none());
    assert_eq!(controller.groups().len(), MAX_GROUPS);
}

// ----- move_tab_to_group ---------------------------------------------------------------------

/// Six tabs; tabs 2, 3 and 4 are group 1.
fn six_tabs_with_group() -> (BrowserController, GroupId) {
    let mut controller = controller_with_tabs(6);
    let group = make(&mut controller, &[2, 3, 4]);
    assert_eq!(layout(&controller), "1 2@1 3@1 4@1 5 6");
    (controller, group)
}

#[test]
fn a_tab_moves_into_a_group_at_the_given_place_in_it() {
    let (mut controller, group) = six_tabs_with_group();
    assert_eq!(controller.move_tab_to_group(t(6), Some(group), 1), Ok(()));
    assert_eq!(layout(&controller), "1 2@1 6@1 3@1 4@1 5");
    check(&controller);

    let (mut controller, group) = six_tabs_with_group();
    assert_eq!(controller.move_tab_to_group(t(6), Some(group), 0), Ok(()));
    assert_eq!(layout(&controller), "1 6@1 2@1 3@1 4@1 5");

    // An index past the end means the end of the group.
    let (mut controller, group) = six_tabs_with_group();
    assert_eq!(controller.move_tab_to_group(t(1), Some(group), 99), Ok(()));
    assert_eq!(layout(&controller), "2@1 3@1 4@1 1@1 5 6");
    check(&controller);
}

#[test]
fn a_tab_moves_within_its_group_to_a_final_place_in_it() {
    let (mut controller, group) = six_tabs_with_group();
    assert_eq!(controller.move_tab_to_group(t(2), Some(group), 2), Ok(()));
    assert_eq!(layout(&controller), "1 3@1 4@1 2@1 5 6");

    assert_eq!(controller.move_tab_to_group(t(4), Some(group), 0), Ok(()));
    assert_eq!(layout(&controller), "1 4@1 3@1 2@1 5 6");

    // Already there: nothing changes.
    assert_eq!(controller.move_tab_to_group(t(4), Some(group), 0), Ok(()));
    assert_eq!(layout(&controller), "1 4@1 3@1 2@1 5 6");
    check(&controller);
}

#[test]
fn moving_a_groups_only_tab_to_its_own_group_changes_nothing() {
    let mut controller = controller_with_tabs(4);
    let group = make(&mut controller, &[2]);
    let before = snapshot(&controller);

    assert_eq!(controller.move_tab_to_group(t(2), Some(group), 0), Ok(()));
    assert_eq!(controller.move_tab_to_group(t(2), Some(group), 9), Ok(()));

    assert_eq!(snapshot(&controller), before);
}

#[test]
fn a_tab_moves_out_of_a_group_to_a_strip_place_and_is_snapped_out_of_runs() {
    // Outside every run: the index is the final strip place.
    let (mut controller, _) = six_tabs_with_group();
    assert_eq!(controller.move_tab_to_group(t(6), None, 0), Ok(()));
    assert_eq!(layout(&controller), "6 1 2@1 3@1 4@1 5");

    // Out of the group and onto its edge: stays at the edge.
    let (mut controller, _) = six_tabs_with_group();
    assert_eq!(controller.move_tab_to_group(t(3), None, 1), Ok(()));
    assert_eq!(layout(&controller), "1 3 2@1 4@1 5 6");

    // An index inside the run it left or another run goes to the end of that run (R1).
    let (mut controller, _) = six_tabs_with_group();
    assert_eq!(controller.move_tab_to_group(t(3), None, 2), Ok(()));
    assert_eq!(layout(&controller), "1 2@1 4@1 3 5 6");
    let (mut controller, _) = six_tabs_with_group();
    assert_eq!(controller.move_tab_to_group(t(6), None, 2), Ok(()));
    assert_eq!(layout(&controller), "1 2@1 3@1 4@1 6 5");
    check(&controller);
}

#[test]
fn a_loose_tab_moved_to_none_is_placed_after_the_pinned_tabs() {
    let mut controller = controller_with_tabs(5);
    assert!(controller.set_tab_pinned(t(5), true));
    make(&mut controller, &[2, 3]);
    assert_eq!(layout(&controller), "^5 1 2@1 3@1 4");

    assert_eq!(controller.move_tab_to_group(t(4), None, 0), Ok(()));

    assert_eq!(layout(&controller), "^5 4 1 2@1 3@1");
    check(&controller);
}

#[test]
fn a_tab_moved_to_another_group_empties_and_deletes_its_old_one() {
    let mut controller = controller_with_tabs(6);
    let old = make(&mut controller, &[2]);
    let new = make(&mut controller, &[3, 4]);
    assert_eq!(layout(&controller), "1 2@1 3@2 4@2 5 6");

    assert_eq!(controller.move_tab_to_group(t(2), Some(new), 0), Ok(()));

    assert_eq!(layout(&controller), "1 2@2 3@2 4@2 5 6");
    assert!(controller.group(old).is_none());
    assert_eq!(group_ids(&controller), [new.value()]);
    // Its key is free: the next group gets it.
    let next = make(&mut controller, &[5]);
    assert_eq!(
        controller.group(next).and_then(|group| group.key),
        Some(key('A'))
    );
}

#[test]
fn a_tab_moves_between_groups_without_splitting_either() {
    let mut controller = controller_with_tabs(7);
    let first = make(&mut controller, &[1, 2, 3]);
    let second = make(&mut controller, &[5, 6]);
    assert_eq!(layout(&controller), "1@1 2@1 3@1 4 5@2 6@2 7");

    assert_eq!(controller.move_tab_to_group(t(2), Some(second), 1), Ok(()));

    assert_eq!(layout(&controller), "1@1 3@1 4 5@2 2@2 6@2 7");
    assert_eq!(group_ids(&controller), [first.value(), second.value()]);
    check(&controller);
}

#[test]
fn a_pinned_tab_cannot_be_moved_into_or_out_of_a_group_run() {
    let (mut controller, group) = six_tabs_with_group();
    assert!(controller.set_tab_pinned(t(6), true));
    let before = snapshot(&controller);

    assert_eq!(
        controller.move_tab_to_group(t(6), Some(group), 0),
        Err(GroupError::TabPinned)
    );
    assert_eq!(
        controller.move_tab_to_group(t(6), None, 3),
        Err(GroupError::TabPinned)
    );

    assert_eq!(snapshot(&controller), before);
}

#[test]
fn moving_an_unknown_tab_or_into_an_unknown_group_is_refused() {
    let (mut controller, group) = six_tabs_with_group();
    let before = snapshot(&controller);

    assert_eq!(
        controller.move_tab_to_group(t(99), Some(group), 0),
        Err(GroupError::TabNotFound)
    );
    assert_eq!(
        controller.move_tab_to_group(t(99), None, 0),
        Err(GroupError::TabNotFound)
    );
    assert_eq!(
        controller.move_tab_to_group(t(5), Some(g(42)), 0),
        Err(GroupError::NotFound)
    );

    assert_eq!(snapshot(&controller), before);
}

#[test]
fn moving_tabs_between_groups_never_changes_the_selection() {
    let (mut controller, group) = six_tabs_with_group();
    assert!(controller.select_tab(t(3)));

    controller
        .move_tab_to_group(t(3), None, 5)
        .expect("the tab moves");
    controller
        .move_tab_to_group(t(6), Some(group), 0)
        .expect("the tab moves");

    assert_eq!(controller.active_tab_id(), t(3));
}

// ----- ungroup_tab ---------------------------------------------------------------------------

#[test]
fn an_ungrouped_tab_lands_directly_after_its_former_groups_run() {
    for (tab, expected) in [
        (2, "1 3@1 4@1 2 5 6"),
        (3, "1 2@1 4@1 3 5 6"),
        (4, "1 2@1 3@1 4 5 6"),
    ] {
        let (mut controller, group) = six_tabs_with_group();
        assert_eq!(controller.ungroup_tab(t(tab)), Ok(()));
        assert_eq!(layout(&controller), expected, "ungrouping tab {tab}");
        assert_eq!(controller.group_of(t(tab)), None);
        assert!(controller.group(group).is_some());
        check(&controller);
    }
}

#[test]
fn ungrouping_the_only_tab_deletes_the_group_and_frees_its_key() {
    let mut controller = controller_with_tabs(4);
    let group = make(&mut controller, &[2]);

    assert_eq!(controller.ungroup_tab(t(2)), Ok(()));

    assert_eq!(layout(&controller), "1 2 3 4");
    assert!(controller.group(group).is_none());
    let next = make(&mut controller, &[3]);
    assert_eq!(
        controller.group(next).and_then(|group| group.key),
        Some(key('A'))
    );
}

#[test]
fn ungrouping_a_loose_or_pinned_tab_is_a_no_op_and_an_unknown_tab_is_refused() {
    let mut controller = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(t(4), true));
    make(&mut controller, &[2, 3]);
    let before = snapshot(&controller);

    assert_eq!(controller.ungroup_tab(t(1)), Ok(()));
    assert_eq!(controller.ungroup_tab(t(4)), Ok(()));
    assert_eq!(snapshot(&controller), before);
    assert_eq!(controller.ungroup_tab(t(99)), Err(GroupError::TabNotFound));
    assert_eq!(snapshot(&controller), before);
}

#[test]
fn ungrouping_never_changes_the_selection() {
    let (mut controller, _) = six_tabs_with_group();
    assert!(controller.select_tab(t(3)));

    assert_eq!(controller.ungroup_tab(t(3)), Ok(()));

    assert_eq!(controller.active_tab_id(), t(3));
}

// ----- reorder_group -------------------------------------------------------------------------

/// Seven tabs; groups 1 = {2, 3}, 2 = {4, 5}, 3 = {6}; tabs 1 and 7 loose.
fn three_groups() -> BrowserController {
    let mut controller = controller_with_tabs(7);
    make(&mut controller, &[2, 3]);
    make(&mut controller, &[4, 5]);
    make(&mut controller, &[6]);
    assert_eq!(layout(&controller), "1 2@1 3@1 4@2 5@2 6@3 7");
    assert_eq!(group_ids(&controller), [1, 2, 3]);
    controller
}

#[test]
fn a_group_moves_earlier_to_just_before_the_group_it_displaces() {
    let mut controller = three_groups();

    assert_eq!(controller.reorder_group(g(3), 0), Ok(true));

    assert_eq!(layout(&controller), "1 6@3 2@1 3@1 4@2 5@2 7");
    assert_eq!(group_ids(&controller), [3, 1, 2]);
    check(&controller);
}

#[test]
fn a_group_moves_later_to_just_after_the_group_it_displaces() {
    let mut controller = three_groups();
    assert_eq!(controller.reorder_group(g(1), 2), Ok(true));
    assert_eq!(layout(&controller), "1 4@2 5@2 6@3 2@1 3@1 7");
    assert_eq!(group_ids(&controller), [2, 3, 1]);

    let mut controller = three_groups();
    assert_eq!(controller.reorder_group(g(1), 1), Ok(true));
    assert_eq!(layout(&controller), "1 4@2 5@2 2@1 3@1 6@3 7");
    assert_eq!(group_ids(&controller), [2, 1, 3]);
    check(&controller);
}

#[test]
fn reordering_to_the_same_place_reports_no_change() {
    let mut controller = three_groups();
    let before = snapshot(&controller);

    assert_eq!(controller.reorder_group(g(2), 1), Ok(false));
    assert_eq!(controller.reorder_group(g(3), 2), Ok(false));
    // An index past the end means the last place.
    assert_eq!(controller.reorder_group(g(3), 99), Ok(false));

    assert_eq!(snapshot(&controller), before);
}

#[test]
fn a_reorder_index_past_the_end_means_the_last_place() {
    let mut controller = three_groups();

    assert_eq!(controller.reorder_group(g(1), 99), Ok(true));

    assert_eq!(group_ids(&controller), [2, 3, 1]);
    assert_eq!(layout(&controller), "1 4@2 5@2 6@3 2@1 3@1 7");
}

#[test]
fn reordering_leaves_the_loose_tabs_between_runs_where_they_are() {
    let mut controller = controller_with_tabs(8);
    make(&mut controller, &[2, 3]);
    make(&mut controller, &[5, 6]);
    assert_eq!(layout(&controller), "1 2@1 3@1 4 5@2 6@2 7 8");

    assert_eq!(controller.reorder_group(g(2), 0), Ok(true));
    assert_eq!(layout(&controller), "1 5@2 6@2 2@1 3@1 4 7 8");
    // Back to the second place: the run goes just after group 1's, and the loose tab 4, which
    // was between the runs, is now after both (the loose tabs keep their order among themselves).
    assert_eq!(controller.reorder_group(g(2), 1), Ok(true));
    assert_eq!(layout(&controller), "1 2@1 3@1 5@2 6@2 4 7 8");
    check(&controller);
}

#[test]
fn reordering_leaves_pinned_tabs_in_front_and_the_selection_alone() {
    let mut controller = controller_with_tabs(6);
    assert!(controller.set_tab_pinned(t(6), true));
    make(&mut controller, &[2, 3]);
    make(&mut controller, &[4]);
    assert!(controller.select_tab(t(3)));

    assert_eq!(controller.reorder_group(g(2), 0), Ok(true));

    assert_eq!(layout(&controller), "^6 1 4@2 2@1 3@1 5");
    assert_eq!(controller.active_tab_id(), t(3));
    check(&controller);
}

#[test]
fn reordering_an_unknown_group_is_refused() {
    let mut controller = three_groups();
    let before = snapshot(&controller);

    assert_eq!(controller.reorder_group(g(9), 0), Err(GroupError::NotFound));

    assert_eq!(snapshot(&controller), before);
}

// ----- rename and recolour -------------------------------------------------------------------

#[test]
fn a_group_is_renamed_and_recoloured_without_touching_the_strip() {
    let (mut controller, group) = six_tabs_with_group();
    let before = layout(&controller);

    assert_eq!(controller.rename_group(group, "  Research "), Ok(()));
    assert_eq!(
        controller.recolour_group(group, GroupColour::Preset(GroupPreset::Teal)),
        Ok(())
    );

    let group = controller.group(group).expect("the group exists");
    assert_eq!(group.name, "Research");
    assert_eq!(group.colour, GroupColour::Preset(GroupPreset::Teal));
    assert_eq!(layout(&controller), before);
}

#[test]
fn renaming_to_nothing_or_an_unknown_group_is_refused() {
    let (mut controller, group) = six_tabs_with_group();
    let before = snapshot(&controller);

    assert_eq!(
        controller.rename_group(group, " \t\u{7} "),
        Err(GroupError::EmptyName)
    );
    assert_eq!(
        controller.rename_group(g(9), "x"),
        Err(GroupError::NotFound)
    );
    assert_eq!(
        controller.recolour_group(g(9), GroupColour::Preset(GroupPreset::Slate)),
        Err(GroupError::NotFound)
    );

    assert_eq!(snapshot(&controller), before);
}

// ----- closing: the last tab of a group, close_other_tabs, close_group -----------------------

#[test]
fn closing_a_groups_last_tab_deletes_the_group_and_the_next_group_takes_its_key() {
    let mut controller = controller_with_tabs(5);
    let first = make(&mut controller, &[2]);
    assert_eq!(
        controller.group(first).and_then(|group| group.key),
        Some(key('A'))
    );

    assert_eq!(controller.close_tab(t(2)), CloseTabOutcome::Closed);

    assert!(controller.groups().is_empty());
    assert!(controller.group(first).is_none());
    check(&controller);
    let second = make(&mut controller, &[3]);
    assert_eq!(second, g(2), "ids are not reused");
    assert_eq!(
        controller.group(second).and_then(|group| group.key),
        Some(key('A'))
    );
}

#[test]
fn closing_a_tab_in_the_middle_of_a_group_keeps_the_run_and_the_group() {
    let (mut controller, group) = six_tabs_with_group();

    assert_eq!(controller.close_tab(t(3)), CloseTabOutcome::Closed);

    assert_eq!(layout(&controller), "1 2@1 4@1 5 6");
    assert_eq!(controller.group_tabs(group), ids(&[2, 4]));
    check(&controller);
}

#[test]
fn closing_other_tabs_deletes_the_groups_it_empties_and_frees_their_keys() {
    let mut controller = controller_with_tabs(6);
    let first = make(&mut controller, &[2, 3]);
    let second = make(&mut controller, &[4]);
    assert!(controller.set_tab_pinned(t(6), true));

    // Keep a member of the second group: the first group goes, the second keeps its one tab.
    assert_eq!(controller.close_other_tabs(t(4)), 4);
    assert_eq!(layout(&controller), "^6 4@2");
    assert!(controller.group(first).is_none());
    assert!(controller.group(second).is_some());
    assert_eq!(controller.active_tab_id(), t(4));
    check(&controller);
    // The first group's key (A) is free again; the second still holds S.
    let opened = controller.open_tab();
    let third = make(&mut controller, &[opened.value()]);
    assert_eq!(
        controller.group(third).and_then(|group| group.key),
        Some(key('A'))
    );
    assert_eq!(
        controller.group(second).and_then(|group| group.key),
        Some(key('S'))
    );
}

#[test]
fn closing_other_tabs_keeps_the_group_of_the_tab_it_keeps() {
    let mut controller = controller_with_tabs(6);
    let first = make(&mut controller, &[2, 3]);
    let second = make(&mut controller, &[4]);

    assert_eq!(controller.close_other_tabs(t(2)), 5);

    assert_eq!(layout(&controller), "2@1");
    assert!(controller.group(first).is_some());
    assert!(controller.group(second).is_none());
    assert_eq!(controller.active_tab_id(), t(2));
    check(&controller);
    // Group 2's key (S) is free; group 1 still holds A.
    let opened = controller.open_tab();
    let next = make(&mut controller, &[opened.value()]);
    assert_eq!(
        controller.group(next).and_then(|group| group.key),
        Some(key('S'))
    );
}

#[test]
fn closing_a_group_closes_its_tabs_and_frees_its_key() {
    let mut controller = controller_with_tabs(6);
    let group = make(&mut controller, &[3, 4]);
    let before = controller.group(group).cloned().expect("the group exists");
    assert!(controller.select_tab(t(1)));

    let closed = controller.close_group(group).expect("the group closes");

    assert_eq!(closed.group, before);
    assert_eq!(closed.tabs, ids(&[3, 4]));
    assert_eq!(closed.opened_blank, None);
    assert_eq!(layout(&controller), "1 2 5 6");
    assert!(controller.groups().is_empty());
    // The selection was elsewhere, so it did not move.
    assert_eq!(controller.active_tab_id(), t(1));
    check(&controller);
    let next = make(&mut controller, &[5]);
    assert_eq!(
        controller.group(next).and_then(|group| group.key),
        Some(key('A'))
    );
}

#[test]
fn closing_the_active_tabs_group_selects_the_tab_before_the_run() {
    for active in [3, 4] {
        let mut controller = controller_with_tabs(6);
        let group = make(&mut controller, &[3, 4]);
        assert!(controller.select_tab(t(active)));

        let closed = controller.close_group(group).expect("the group closes");

        assert_eq!(controller.active_tab_id(), t(2), "active was {active}");
        assert_eq!(closed.opened_blank, None);
        assert_eq!(layout(&controller), "1 2 5 6");
        check(&controller);
    }
}

#[test]
fn closing_the_active_tabs_group_at_the_start_selects_the_tab_after_the_run() {
    let mut controller = controller_with_tabs(5);
    let group = make(&mut controller, &[1, 2]);
    assert!(controller.select_tab(t(2)));

    controller.close_group(group).expect("the group closes");

    assert_eq!(controller.active_tab_id(), t(3));
    assert_eq!(layout(&controller), "3 4 5");
    check(&controller);
}

#[test]
fn closing_a_group_that_holds_every_tab_opens_a_blank_tab_and_selects_it() {
    let mut controller = controller_with_tabs(3);
    let group = make(&mut controller, &[1, 2, 3]);
    assert!(controller.select_tab(t(2)));

    let closed = controller.close_group(group).expect("the group closes");

    let blank = closed.opened_blank.expect("a blank tab was opened");
    assert_eq!(closed.tabs, ids(&[1, 2, 3]));
    assert_eq!(blank, t(4));
    assert_eq!(controller.active_tab_id(), blank);
    assert_eq!(layout(&controller), "4");
    assert!(controller.groups().is_empty());
    check(&controller);
}

#[test]
fn closing_a_group_leaves_pinned_tabs_alone_and_may_select_one() {
    let mut controller = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(t(1), true));
    let group = make(&mut controller, &[2, 3]);
    assert!(controller.select_tab(t(2)));
    assert_eq!(layout(&controller), "^1 2@1 3@1 4");

    let closed = controller.close_group(group).expect("the group closes");

    assert_eq!(layout(&controller), "^1 4");
    assert_eq!(controller.pinned_tab_count(), 1);
    // The tab just before the run is the pinned one.
    assert_eq!(controller.active_tab_id(), t(1));
    assert_eq!(closed.opened_blank, None);
    check(&controller);
}

#[test]
fn closing_a_group_of_every_unpinned_tab_selects_the_pinned_tab_not_a_blank_one() {
    let mut controller = controller_with_tabs(3);
    assert!(controller.set_tab_pinned(t(1), true));
    let group = make(&mut controller, &[2, 3]);
    assert!(controller.select_tab(t(3)));

    let closed = controller.close_group(group).expect("the group closes");

    assert_eq!(closed.opened_blank, None);
    assert_eq!(controller.active_tab_id(), t(1));
    assert_eq!(layout(&controller), "^1");
}

#[test]
fn closing_a_group_drops_the_commands_queued_for_its_tabs() {
    let mut controller = controller_with_tabs(3);
    let group = make(&mut controller, &[2, 3]);
    for id in [t(1), t(2), t(3)] {
        assert!(controller.handle_event(id, BrowserEvent::AddressChanged { url: String::new() }));
    }
    assert!(controller.queue_command_for(t(3), BrowserCommand::Reload));

    controller.close_group(group).expect("the group closes");

    assert!(!controller.has_pending_commands());
}

#[test]
fn closing_an_unknown_group_is_refused() {
    let (mut controller, _) = six_tabs_with_group();
    let before = snapshot(&controller);

    assert_eq!(controller.close_group(g(9)), Err(GroupError::NotFound));

    assert_eq!(snapshot(&controller), before);
}

// ----- pinning, move_tab, move_tab_by --------------------------------------------------------

#[test]
fn pinning_a_grouped_tab_takes_it_out_of_the_group() {
    let (mut controller, group) = six_tabs_with_group();

    assert!(controller.set_tab_pinned(t(3), true));

    assert_eq!(layout(&controller), "^3 1 2@1 4@1 5 6");
    assert_eq!(controller.group_of(t(3)), None);
    assert_eq!(controller.group_tabs(group), ids(&[2, 4]));
    check(&controller);
}

#[test]
fn pinning_a_groups_only_tab_deletes_the_group() {
    let mut controller = controller_with_tabs(4);
    let group = make(&mut controller, &[2]);

    assert!(controller.set_tab_pinned(t(2), true));

    assert_eq!(layout(&controller), "^2 1 3 4");
    assert!(controller.group(group).is_none());
    check(&controller);
}

#[test]
fn unpinning_a_tab_puts_it_before_the_first_group_not_inside_one() {
    let mut controller = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(t(1), true));
    make(&mut controller, &[2, 3]);
    assert_eq!(layout(&controller), "^1 2@1 3@1 4");

    assert!(controller.set_tab_pinned(t(1), false));

    assert_eq!(layout(&controller), "1 2@1 3@1 4");
    assert_eq!(controller.group_of(t(1)), None);
    check(&controller);
}

#[test]
fn dragging_a_grouped_tab_into_the_pinned_area_pins_it_and_leaves_the_group() {
    let (mut controller, _) = six_tabs_with_group();

    assert!(controller.move_tab(t(3), 0, true));

    assert_eq!(layout(&controller), "^3 1 2@1 4@1 5 6");
    check(&controller);
}

#[test]
fn an_unpinned_move_of_a_grouped_tab_stays_inside_its_run() {
    // Left of the run, right of the run, and inside it.
    let (mut controller, _) = six_tabs_with_group();
    assert!(controller.move_tab(t(3), 0, false));
    assert_eq!(layout(&controller), "1 3@1 2@1 4@1 5 6");

    let (mut controller, _) = six_tabs_with_group();
    assert!(controller.move_tab(t(3), 99, false));
    assert_eq!(layout(&controller), "1 2@1 4@1 3@1 5 6");

    let (mut controller, _) = six_tabs_with_group();
    assert!(controller.move_tab(t(2), 3, false));
    assert_eq!(layout(&controller), "1 3@1 4@1 2@1 5 6");

    // Where it already is: no change.
    let (mut controller, _) = six_tabs_with_group();
    assert!(!controller.move_tab(t(4), 5, false));
    assert!(!controller.move_tab(t(2), 0, false));
    assert_eq!(layout(&controller), "1 2@1 3@1 4@1 5 6");
    check(&controller);
}

#[test]
fn a_groups_only_tab_stays_put_whatever_index_it_is_moved_to() {
    let mut controller = controller_with_tabs(4);
    make(&mut controller, &[2]);

    assert!(!controller.move_tab(t(2), 0, false));
    assert!(!controller.move_tab(t(2), 99, false));

    assert_eq!(layout(&controller), "1 2@1 3 4");
}

#[test]
fn a_loose_tab_dropped_inside_a_run_goes_to_the_end_of_it_but_not_at_an_edge() {
    // Inside the run: after it.
    let mut controller = controller_with_tabs(5);
    make(&mut controller, &[2, 3, 4]);
    assert!(controller.move_tab(t(1), 2, false));
    assert_eq!(layout(&controller), "2@1 3@1 4@1 1 5");
    check(&controller);

    // At the run's start edge: untouched.
    let mut controller = controller_with_tabs(5);
    make(&mut controller, &[2, 3, 4]);
    assert!(controller.move_tab(t(5), 1, false));
    assert_eq!(layout(&controller), "1 5 2@1 3@1 4@1");

    // At the run's end edge: untouched.
    let mut controller = controller_with_tabs(5);
    make(&mut controller, &[2, 3, 4]);
    assert!(controller.move_tab(t(1), 3, false));
    assert_eq!(layout(&controller), "2@1 3@1 4@1 1 5");

    // A snap that lands the tab where it was is no move.
    let mut controller = controller_with_tabs(5);
    make(&mut controller, &[2, 3, 4]);
    assert!(!controller.move_tab(t(5), 3, false));
    assert_eq!(layout(&controller), "1 2@1 3@1 4@1 5");
    check(&controller);
}

#[test]
fn a_loose_tab_is_never_moved_in_front_of_the_pinned_tabs() {
    let mut controller = controller_with_tabs(5);
    assert!(controller.set_tab_pinned(t(5), true));
    make(&mut controller, &[2, 3]);

    assert!(controller.move_tab(t(4), 0, false));

    assert_eq!(layout(&controller), "^5 4 1 2@1 3@1");
    check(&controller);
}

#[test]
fn the_keyboard_moves_a_grouped_tab_within_its_run_only() {
    let (mut controller, _) = six_tabs_with_group();

    assert!(controller.move_tab_by(t(3), -1));
    assert_eq!(layout(&controller), "1 3@1 2@1 4@1 5 6");
    // The edges of the run stop it, though other tabs lie beyond.
    assert!(!controller.move_tab_by(t(3), -1));
    assert!(controller.move_tab_by(t(4), -1));
    assert_eq!(layout(&controller), "1 3@1 4@1 2@1 5 6");
    assert!(!controller.move_tab_by(t(2), 1));
    assert!(controller.move_tab_by(t(3), 1));
    assert_eq!(layout(&controller), "1 4@1 3@1 2@1 5 6");
    check(&controller);
}

#[test]
fn the_keyboard_steps_a_loose_tab_over_a_whole_run_in_one_move() {
    let mut controller = controller_with_tabs(5);
    make(&mut controller, &[2, 3, 4]);

    // Right over the run, then back left over it.
    assert!(controller.move_tab_by(t(1), 1));
    assert_eq!(layout(&controller), "2@1 3@1 4@1 1 5");
    assert!(controller.move_tab_by(t(1), -1));
    assert_eq!(layout(&controller), "1 2@1 3@1 4@1 5");
    // A tab on the other side steps left over it to just before it, never into it.
    assert!(controller.move_tab_by(t(5), -1));
    assert_eq!(layout(&controller), "1 5 2@1 3@1 4@1");
    check(&controller);
}

#[test]
fn the_keyboard_steps_over_one_run_at_a_time_and_stops_at_the_edges() {
    let mut controller = controller_with_tabs(4);
    make(&mut controller, &[2]);
    make(&mut controller, &[3]);
    assert_eq!(layout(&controller), "1 2@1 3@2 4");

    assert!(controller.move_tab_by(t(1), 1));
    assert_eq!(layout(&controller), "2@1 1 3@2 4");
    assert!(controller.move_tab_by(t(1), 1));
    assert_eq!(layout(&controller), "2@1 3@2 1 4");
    assert!(!controller.move_tab_by(t(4), 1));
    assert!(!controller.move_tab_by(t(2), -1));
    assert!(
        !controller.move_tab_by(t(2), 1),
        "a group's only tab stays in its run"
    );
    check(&controller);
}

#[test]
fn the_keyboard_never_moves_a_loose_tab_in_front_of_the_pinned_tabs() {
    let mut controller = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(t(1), true));
    make(&mut controller, &[3, 4]);
    assert_eq!(layout(&controller), "^1 2 3@1 4@1");

    assert!(!controller.move_tab_by(t(2), -1));
    assert!(controller.move_tab_by(t(2), 1));
    assert_eq!(layout(&controller), "^1 3@1 4@1 2");
    check(&controller);
}

// ----- new tabs: adopt_tab, open_tab, open_restored_tab --------------------------------------

#[test]
fn a_tab_a_grouped_page_opens_joins_the_group_right_after_its_opener() {
    let (mut controller, group) = six_tabs_with_group();

    let opened = controller
        .adopt_tab(t(3), TabActivation::Background)
        .expect("the opener exists");

    assert_eq!(opened, t(7));
    assert_eq!(layout(&controller), "1 2@1 3@1 7@1 4@1 5 6");
    assert_eq!(controller.group_of(opened), Some(group));
    // The last tab of the run: the new one is the new end of the run.
    let last = controller
        .adopt_tab(t(4), TabActivation::Background)
        .expect("the opener exists");
    assert_eq!(layout(&controller), "1 2@1 3@1 7@1 4@1 8@1 5 6");
    assert_eq!(controller.group_of(last), Some(group));
    check(&controller);
}

#[test]
fn a_foreground_tab_a_page_opens_in_a_group_is_selected_and_in_the_group() {
    let (mut controller, group) = six_tabs_with_group();

    let opened = controller
        .adopt_tab(t(2), TabActivation::Foreground)
        .expect("the opener exists");

    assert_eq!(controller.active_tab_id(), opened);
    assert_eq!(controller.group_of(opened), Some(group));
    assert_eq!(layout(&controller), "1 2@1 7@1 3@1 4@1 5 6");
    check(&controller);
}

#[test]
fn a_tab_a_loose_page_opens_is_loose_even_next_to_a_group() {
    let (mut controller, _) = six_tabs_with_group();

    // Tab 1 is loose and its right neighbour starts a run.
    let before_run = controller
        .adopt_tab(t(1), TabActivation::Background)
        .expect("the opener exists");
    assert_eq!(layout(&controller), "1 7 2@1 3@1 4@1 5 6");
    assert_eq!(controller.group_of(before_run), None);

    // Tab 5 is loose and its left neighbour ends a run.
    let after_run = controller
        .adopt_tab(t(5), TabActivation::Background)
        .expect("the opener exists");
    assert_eq!(layout(&controller), "1 7 2@1 3@1 4@1 5 8 6");
    assert_eq!(controller.group_of(after_run), None);
    check(&controller);
}

#[test]
fn a_tab_a_pinned_page_opens_is_loose_and_goes_after_the_pinned_tabs() {
    let mut controller = controller_with_tabs(5);
    assert!(controller.set_tab_pinned(t(1), true));
    make(&mut controller, &[2, 3]);
    assert_eq!(layout(&controller), "^1 2@1 3@1 4 5");

    let opened = controller
        .adopt_tab(t(1), TabActivation::Background)
        .expect("the opener exists");

    // Index 1 is the start of the run, an edge: the tab is loose and in front of the group.
    assert_eq!(layout(&controller), "^1 6 2@1 3@1 4 5");
    assert_eq!(controller.group_of(opened), None);
    check(&controller);
}

#[test]
fn adopting_for_an_unknown_opener_changes_nothing() {
    let (mut controller, _) = six_tabs_with_group();
    let before = snapshot(&controller);

    assert_eq!(controller.adopt_tab(t(99), TabActivation::Foreground), None);

    assert_eq!(snapshot(&controller), before);
}

#[test]
fn a_new_tab_is_loose_at_the_end_of_the_strip_even_after_a_group() {
    let mut controller = controller_with_tabs(3);
    make(&mut controller, &[2, 3]);

    let opened = controller.open_tab();
    assert_eq!(layout(&controller), "1 2@1 3@1 4");
    assert_eq!(controller.group_of(opened), None);
    assert_eq!(controller.active_tab_id(), opened);

    let background = controller
        .open_tab_with(None, TabActivation::Background)
        .expect("a blank tab opens");
    assert_eq!(layout(&controller), "1 2@1 3@1 4 5");
    assert_eq!(controller.group_of(background), None);
    check(&controller);
}

#[test]
fn a_restored_tab_inside_a_run_snaps_to_the_end_of_it() {
    // Every interior index of a run of three tabs at 1..=3 gives the same result.
    for index in [2, 3] {
        let (mut controller, group) = six_tabs_with_group();
        let opened = controller
            .open_restored_tab(index, &restored_tab(false))
            .expect("the tab opens");
        assert_eq!(layout(&controller), "1 2@1 3@1 4@1 7 5 6", "index {index}");
        assert_eq!(controller.group_of(opened), None);
        assert_eq!(controller.group_tabs(group).len(), 3);
        check(&controller);
    }
}

#[test]
fn a_restored_tab_at_a_runs_edge_stays_there() {
    let (mut controller, _) = six_tabs_with_group();
    controller
        .open_restored_tab(1, &restored_tab(false))
        .expect("the tab opens");
    assert_eq!(layout(&controller), "1 7 2@1 3@1 4@1 5 6");

    let (mut controller, _) = six_tabs_with_group();
    controller
        .open_restored_tab(4, &restored_tab(false))
        .expect("the tab opens");
    assert_eq!(layout(&controller), "1 2@1 3@1 4@1 7 5 6");

    let (mut controller, _) = six_tabs_with_group();
    controller
        .open_restored_tab(99, &restored_tab(false))
        .expect("the tab opens");
    assert_eq!(layout(&controller), "1 2@1 3@1 4@1 5 6 7");
    check(&controller);
}

#[test]
fn a_restored_pinned_tab_goes_among_the_pinned_tabs_and_never_into_a_group() {
    let (mut controller, _) = six_tabs_with_group();

    let opened = controller
        .open_restored_tab(3, &restored_tab(true))
        .expect("the tab opens");

    assert_eq!(layout(&controller), "^7 1 2@1 3@1 4@1 5 6");
    assert_eq!(controller.group_of(opened), None);
    check(&controller);
}

// ----- restore_group -------------------------------------------------------------------------

#[test]
fn a_restored_group_keeps_its_id_name_colour_and_key() {
    let mut controller = controller_with_tabs(5);
    let spec = RestoredGroup {
        id_hint: Some(7),
        name: Some("Work".to_owned()),
        colour: Some(GroupColour::Preset(GroupPreset::Green)),
        key: Some(key('Q')),
    };

    let group = controller
        .restore_group(spec, &[t(4), t(2)], now())
        .expect("the group is made");

    assert_eq!(group, g(7));
    assert_eq!(layout(&controller), "1 2@7 4@7 3 5");
    let restored = controller.group(group).expect("the group exists");
    assert_eq!(restored.name, "Work");
    assert_eq!(restored.colour, GroupColour::Preset(GroupPreset::Green));
    assert_eq!(restored.key, Some(key('Q')));
    assert_eq!(restored.created, now());
    // New ids continue after the highest one seen.
    assert_eq!(controller.next_group_id(), 8);
    assert_eq!(make(&mut controller, &[5]), g(8));
    check(&controller);
}

#[test]
fn the_saved_id_counter_can_be_raised_but_never_lowered() {
    let mut controller = controller_with_tabs(3);
    assert_eq!(controller.next_group_id(), 1);

    // The session restore passes the counter it saved last time.
    controller.raise_next_group_id(40);
    assert_eq!(controller.next_group_id(), 40);
    assert_eq!(make(&mut controller, &[1]), g(40));

    controller.raise_next_group_id(5);
    assert_eq!(
        controller.next_group_id(),
        41,
        "a lower floor changes nothing"
    );
    assert_eq!(make(&mut controller, &[2]), g(41));

    // Slint's `int` is the limit.
    controller.raise_next_group_id(u64::MAX);
    assert_eq!(controller.next_group_id(), i32::MAX as u64);
    check(&controller);
}

#[test]
fn a_restored_groups_key_is_dropped_when_another_group_holds_it() {
    let mut controller = controller_with_tabs(5);
    let holder = make(&mut controller, &[1]);
    assert_eq!(
        controller.group(holder).and_then(|group| group.key),
        Some(key('A'))
    );
    let spec = RestoredGroup {
        key: Some(key('A')),
        ..RestoredGroup::default()
    };

    let group = controller
        .restore_group(spec, &[t(2), t(3)], now())
        .expect("the group is made");

    assert_eq!(controller.group(group).and_then(|group| group.key), None);
    assert_eq!(
        controller.group(holder).and_then(|group| group.key),
        Some(key('A'))
    );
    check(&controller);
}

#[test]
fn a_restored_group_without_a_name_takes_the_default_one() {
    let mut controller = controller_with_tabs(3);

    let group = controller
        .restore_group(RestoredGroup::default(), &[t(2)], now())
        .expect("the group is made");

    assert_eq!(
        controller.group(group).map(|group| group.name.as_str()),
        Some("Group 1")
    );
}

#[test]
fn restoring_a_group_skips_pinned_and_unknown_tabs() {
    let mut controller = controller_with_tabs(5);
    assert!(controller.set_tab_pinned(t(1), true));

    let group = controller
        .restore_group(
            RestoredGroup::default(),
            &[t(1), t(99), t(3), t(3), t(4)],
            now(),
        )
        .expect("the group is made");

    assert_eq!(controller.group_tabs(group), ids(&[3, 4]));
    assert_eq!(layout(&controller), "^1 2 3@1 4@1 5");
    check(&controller);
}

#[test]
fn restoring_a_group_with_no_usable_tab_is_refused() {
    let mut controller = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(t(1), true));
    let before = snapshot(&controller);

    assert_eq!(
        controller.restore_group(RestoredGroup::default(), &[t(1), t(99)], now()),
        Err(GroupError::NoTabs)
    );
    assert_eq!(
        controller.restore_group(RestoredGroup::default(), &[], now()),
        Err(GroupError::NoTabs)
    );

    assert_eq!(snapshot(&controller), before);
}

#[test]
fn restoring_a_group_takes_tabs_out_of_their_old_groups() {
    let mut controller = controller_with_tabs(6);
    let old = make(&mut controller, &[2, 3]);

    let new = controller
        .restore_group(RestoredGroup::default(), &[t(3), t(4)], now())
        .expect("the group is made");

    assert_eq!(layout(&controller), "1 2@1 3@2 4@2 5 6");
    assert_eq!(group_ids(&controller), [old.value(), new.value()]);
    check(&controller);
}

#[test]
fn restoring_a_group_past_the_limit_is_refused() {
    let mut controller = controller_with_tabs(70);
    for number in 1..=MAX_GROUPS as u64 {
        make(&mut controller, &[number]);
    }
    let before = snapshot(&controller);

    assert_eq!(
        controller.restore_group(RestoredGroup::default(), &[t(65)], now()),
        Err(GroupError::TooManyGroups)
    );

    assert_eq!(snapshot(&controller), before);
}

// ----- keys through the controller -----------------------------------------------------------

#[test]
fn new_groups_take_the_default_key_sequence_in_creation_order() {
    let mut controller = controller_with_tabs(11);

    for number in 1..=10 {
        make(&mut controller, &[number]);
    }

    let keys = controller
        .groups()
        .iter()
        .map(|group| group.key.map(GroupKey::letter))
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        [
            Some('A'),
            Some('S'),
            Some('D'),
            Some('F'),
            Some('G'),
            Some('H'),
            Some('J'),
            Some('K'),
            Some('L'),
            None
        ]
    );
}

#[test]
fn a_custom_key_sequence_applies_to_groups_made_after_it() {
    let mut controller = controller_with_tabs(6);
    let first = make(&mut controller, &[1]);

    controller.set_group_key_sequence(GroupKey::parse_sequence("QWE"));
    let second = make(&mut controller, &[2]);
    let third = make(&mut controller, &[3]);
    let fourth = make(&mut controller, &[4]);
    let fifth = make(&mut controller, &[5]);

    let key_of =
        |controller: &BrowserController, group| controller.group(group).and_then(|group| group.key);
    assert_eq!(key_of(&controller, first), Some(key('A')));
    assert_eq!(key_of(&controller, second), Some(key('Q')));
    assert_eq!(key_of(&controller, third), Some(key('W')));
    assert_eq!(key_of(&controller, fourth), Some(key('E')));
    assert_eq!(key_of(&controller, fifth), None);
}

#[test]
fn an_empty_key_sequence_gives_new_groups_no_key() {
    let mut controller = controller_with_tabs(3);
    controller.set_group_key_sequence(Vec::new());

    let group = make(&mut controller, &[2]);

    assert_eq!(controller.group(group).and_then(|group| group.key), None);
}

#[test]
fn a_key_is_held_by_one_group_and_taking_it_names_the_holder() {
    let mut controller = controller_with_tabs(4);
    let first = make(&mut controller, &[1]);
    let second = make(&mut controller, &[2]);
    assert_eq!(
        controller.group(first).and_then(|group| group.key),
        Some(key('A'))
    );
    assert_eq!(
        controller.group(second).and_then(|group| group.key),
        Some(key('S'))
    );

    assert_eq!(
        controller.set_group_key(second, Some(key('A'))),
        Err(GroupError::KeyTaken { holder: first })
    );
    assert_eq!(
        controller.group(second).and_then(|group| group.key),
        Some(key('S'))
    );
    // Keeping its own key is fine, and so is a free one or none at all.
    assert_eq!(controller.set_group_key(second, Some(key('S'))), Ok(()));
    assert_eq!(controller.set_group_key(second, Some(key('Z'))), Ok(()));
    assert_eq!(controller.set_group_key(second, None), Ok(()));
    assert_eq!(controller.group(second).and_then(|group| group.key), None);
    assert_eq!(
        controller.set_group_key(g(99), Some(key('B'))),
        Err(GroupError::NotFound)
    );
    check(&controller);
}

#[test]
fn swapping_a_key_gives_the_holder_the_old_one() {
    let mut controller = controller_with_tabs(4);
    let first = make(&mut controller, &[1]);
    let second = make(&mut controller, &[2]);

    assert_eq!(controller.swap_group_key(second, key('A')), Ok(()));

    assert_eq!(
        controller.group(second).and_then(|group| group.key),
        Some(key('A'))
    );
    assert_eq!(
        controller.group(first).and_then(|group| group.key),
        Some(key('S'))
    );

    // A group with no key swapping for a held one leaves the holder with none.
    assert_eq!(controller.set_group_key(second, None), Ok(()));
    assert_eq!(controller.swap_group_key(second, key('S')), Ok(()));
    assert_eq!(
        controller.group(second).and_then(|group| group.key),
        Some(key('S'))
    );
    assert_eq!(controller.group(first).and_then(|group| group.key), None);
    check(&controller);
}

#[test]
fn a_deleted_groups_key_goes_to_the_next_group_made() {
    let mut controller = controller_with_tabs(5);
    let first = make(&mut controller, &[2]);
    let second = make(&mut controller, &[3]);
    assert_eq!(controller.close_tab(t(2)), CloseTabOutcome::Closed);
    assert!(controller.group(first).is_none());

    let third = make(&mut controller, &[4]);

    assert_eq!(
        controller.group(second).and_then(|group| group.key),
        Some(key('S'))
    );
    assert_eq!(
        controller.group(third).and_then(|group| group.key),
        Some(key('A'))
    );
}

// ----- the random sequence -------------------------------------------------------------------

/// A fixed-seed xorshift generator: the test needs no dependency and is the same every run.
struct Rng(u64);

impl Rng {
    fn step(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.step() % bound as u64) as usize
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

/// The operations the sequence runs and how likely each one is (out of 100). Groups are made and
/// grown more often than they are closed, so runs get long enough to have interiors.
const OPERATIONS: [(&str, usize); 13] = [
    ("create_group", 15),
    ("move_tab_to_group", 20),
    ("ungroup_tab", 5),
    ("reorder_group", 8),
    ("close_tab", 5),
    ("close_group", 3),
    ("set_tab_pinned", 5),
    ("move_tab", 10),
    ("move_tab_by", 10),
    ("adopt_tab", 8),
    ("open_restored_tab", 8),
    ("close_other_tabs", 2),
    ("open_tab", 1),
];

fn choose_operation(rng: &mut Rng) -> usize {
    let mut roll = rng.below(100);
    for (op, (_, weight)) in OPERATIONS.iter().enumerate() {
        if roll < *weight {
            return op;
        }
        roll -= weight;
    }
    unreachable!("the weights add up to 100")
}

/// A tab of the strip, or now and then one that does not exist.
fn pick_tab(rng: &mut Rng, controller: &BrowserController) -> TabId {
    if rng.chance(16) {
        return t(9_999);
    }
    let all = controller.tabs().map(|tab| tab.id()).collect::<Vec<_>>();
    all[rng.below(all.len())]
}

/// A group of the window, or now and then one that does not exist.
fn pick_group(rng: &mut Rng, controller: &BrowserController) -> GroupId {
    if controller.groups().is_empty() || rng.chance(16) {
        return g(999_999);
    }
    controller.groups()[rng.below(controller.groups().len())].id
}

/// Tabs to make a group of: a stretch of neighbours in the strip (which is how people select
/// tabs) or a few picked at random, possibly with an unknown one.
fn pick_members(rng: &mut Rng, controller: &BrowserController) -> Vec<TabId> {
    if rng.chance(2) {
        let all = controller.tabs().map(|tab| tab.id()).collect::<Vec<_>>();
        let start = rng.below(all.len());
        let end = (start + 1 + rng.below(4)).min(all.len());
        return all[start..end].to_vec();
    }
    let count = 1 + rng.below(4);
    (0..count).map(|_| pick_tab(rng, controller)).collect()
}

/// Runs operation `op`; returns whether it did something and whether it may change the selection.
fn apply(rng: &mut Rng, controller: &mut BrowserController, op: usize) -> (bool, bool) {
    match op {
        0 => {
            let tabs = pick_members(rng, controller);
            (
                controller.create_group(&tabs, None, None, now()).is_ok(),
                false,
            )
        }
        1 => {
            let tab = pick_tab(rng, controller);
            let group = if rng.chance(5) {
                None
            } else {
                Some(pick_group(rng, controller))
            };
            let index = rng.below(9);
            (
                controller.move_tab_to_group(tab, group, index).is_ok(),
                false,
            )
        }
        2 => {
            let tab = pick_tab(rng, controller);
            (controller.ungroup_tab(tab).is_ok(), false)
        }
        3 => {
            let group = pick_group(rng, controller);
            let index = rng.below(6);
            (controller.reorder_group(group, index) == Ok(true), false)
        }
        4 => {
            let tab = pick_tab(rng, controller);
            (controller.close_tab(tab) == CloseTabOutcome::Closed, true)
        }
        5 => {
            let group = pick_group(rng, controller);
            (controller.close_group(group).is_ok(), true)
        }
        6 => {
            let tab = pick_tab(rng, controller);
            // Pinning is rarer than unpinning, so most tabs stay in the unpinned part.
            let pinned = rng.chance(3);
            (controller.set_tab_pinned(tab, pinned), false)
        }
        7 => {
            let tab = pick_tab(rng, controller);
            let index = rng.below(20);
            let pinned = rng.chance(6);
            (controller.move_tab(tab, index, pinned), false)
        }
        8 => {
            let tab = pick_tab(rng, controller);
            let delta = match rng.below(6) {
                0 => -2,
                1 => 2,
                2 | 3 => -1,
                _ => 1,
            };
            (controller.move_tab_by(tab, delta), false)
        }
        9 => {
            let opener = pick_tab(rng, controller);
            let foreground = rng.chance(3);
            let activation = if foreground {
                TabActivation::Foreground
            } else {
                TabActivation::Background
            };
            (
                controller.adopt_tab(opener, activation).is_some(),
                foreground,
            )
        }
        10 => {
            let index = rng.below(controller.tabs().len() + 3);
            let restored = restored_tab(rng.chance(6));
            (
                controller.open_restored_tab(index, &restored).is_ok(),
                false,
            )
        }
        11 => {
            let keep = pick_tab(rng, controller);
            (controller.close_other_tabs(keep) > 0, true)
        }
        _ => {
            controller.open_tab();
            (true, true)
        }
    }
}

#[test]
fn a_thousand_random_operations_keep_every_strip_invariant() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut controller = controller_with_tabs(12);
    let mut done = [0_usize; OPERATIONS.len()];
    let mut most_groups = 0;
    let mut longest_run = 0;

    for step in 0..1_000 {
        let tab_count = controller.tabs().len();
        // Keep the strip between 8 and 20 tabs so there is always something to group and close.
        let op = if tab_count < 8 {
            [9, 10, 12][rng.below(3)]
        } else if tab_count > 20 {
            4
        } else {
            choose_operation(&mut rng)
        };
        let before = snapshot(&controller);

        let (did, may_change_selection) = apply(&mut rng, &mut controller, op);

        let name = OPERATIONS[op].0;
        if let Some(violation) = controller.group_invariant_violation() {
            panic!(
                "step {step}, {name}: {violation}\nbefore: {}\nafter:  {}",
                before.layout,
                layout(&controller)
            );
        }
        if !did {
            // A refused operation leaves everything as it was.
            assert_eq!(
                snapshot(&controller),
                before,
                "step {step}: {name} did nothing but changed something"
            );
        } else {
            done[op] += 1;
            if !may_change_selection {
                assert_eq!(
                    controller.active_tab_id(),
                    before.active,
                    "step {step}: {name} changed the selection"
                );
            }
        }
        most_groups = most_groups.max(controller.groups().len());
        for group in controller.groups() {
            longest_run = longest_run.max(controller.group_tabs(group.id).len());
        }
    }

    // The sequence must have exercised what it is there to test.
    for ((name, _), count) in OPERATIONS.iter().zip(done) {
        assert!(count >= 5, "{name} succeeded only {count} times: {done:?}");
    }
    assert!(most_groups >= 4, "only {most_groups} groups at most");
    assert!(
        longest_run >= 5,
        "no group ever had more than {longest_run} tabs"
    );
}
