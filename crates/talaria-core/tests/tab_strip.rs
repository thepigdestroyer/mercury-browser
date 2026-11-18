// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Pinned tabs, reordering, "close other tabs", the Cmd+W rule for pinned tabs and tab search.

use talaria_core::{
    BrowserController, BrowserEvent, PinnedCloseOutcome, TabActivation, TabId, tab_matches_query,
};

/// A controller with `count` tabs (the first one included), the first selected.
fn controller_with_tabs(count: usize) -> (BrowserController, Vec<TabId>) {
    let mut controller = BrowserController::default();
    let mut ids = vec![controller.active_tab_id()];
    for _ in 1..count {
        ids.push(
            controller
                .open_tab_with(None, TabActivation::Background)
                .expect("a blank tab opens"),
        );
    }
    (controller, ids)
}

fn order(controller: &BrowserController) -> Vec<TabId> {
    controller.tabs().map(|tab| tab.id()).collect()
}

fn pinned(controller: &BrowserController) -> Vec<TabId> {
    controller
        .tabs()
        .filter(|tab| tab.is_pinned())
        .map(|tab| tab.id())
        .collect()
}

/// Pinned tabs always come first in the strip.
fn assert_pinned_first(controller: &BrowserController) {
    let flags = controller
        .tabs()
        .map(|tab| tab.is_pinned())
        .collect::<Vec<_>>();
    let count = controller.pinned_tab_count();
    assert!(flags[..count].iter().all(|pinned| *pinned), "{flags:?}");
    assert!(flags[count..].iter().all(|pinned| !*pinned), "{flags:?}");
}

#[test]
fn tabs_start_unpinned() {
    let (controller, _) = controller_with_tabs(3);

    assert_eq!(controller.pinned_tab_count(), 0);
    assert!(controller.tabs().all(|tab| !tab.is_pinned()));
}

#[test]
fn pinning_moves_the_tab_to_the_end_of_the_pinned_group() {
    let (mut controller, ids) = controller_with_tabs(4);

    assert!(controller.set_tab_pinned(ids[2], true));
    assert_eq!(order(&controller), [ids[2], ids[0], ids[1], ids[3]]);
    assert!(controller.set_tab_pinned(ids[3], true));
    assert_eq!(order(&controller), [ids[2], ids[3], ids[0], ids[1]]);
    assert_eq!(pinned(&controller), [ids[2], ids[3]]);
    assert_pinned_first(&controller);
}

#[test]
fn unpinning_moves_the_tab_to_the_start_of_the_unpinned_tabs() {
    let (mut controller, ids) = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));
    assert!(controller.set_tab_pinned(ids[2], true));

    assert!(controller.set_tab_pinned(ids[0], false));

    assert_eq!(order(&controller), [ids[1], ids[2], ids[0], ids[3]]);
    assert_eq!(controller.pinned_tab_count(), 2);
    assert_pinned_first(&controller);
}

#[test]
fn pinning_a_pinned_tab_or_an_unknown_tab_changes_nothing() {
    let (mut controller, ids) = controller_with_tabs(2);
    assert!(controller.set_tab_pinned(ids[1], true));

    assert!(!controller.set_tab_pinned(ids[1], true));
    assert!(!controller.set_tab_pinned(ids[0], false));
    assert!(!controller.set_tab_pinned(TabId::new(999), true));
    assert_eq!(order(&controller), [ids[1], ids[0]]);
}

#[test]
fn pinning_keeps_the_selection() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.select_tab(ids[2]));

    assert!(controller.set_tab_pinned(ids[2], true));

    assert_eq!(controller.active_tab_id(), ids[2]);
}

#[test]
fn moving_a_tab_puts_it_at_the_given_index() {
    let (mut controller, ids) = controller_with_tabs(5);

    // Right: the tab ends up at index 3, after the tab that was there.
    assert!(controller.move_tab(ids[1], 3, false));
    assert_eq!(order(&controller), [ids[0], ids[2], ids[3], ids[1], ids[4]]);
    // Left.
    assert!(controller.move_tab(ids[4], 0, false));
    assert_eq!(order(&controller), [ids[4], ids[0], ids[2], ids[3], ids[1]]);
    // To the end, with an index past it.
    assert!(controller.move_tab(ids[4], 99, false));
    assert_eq!(order(&controller), [ids[0], ids[2], ids[3], ids[1], ids[4]]);
}

#[test]
fn moving_a_tab_to_where_it_is_changes_nothing() {
    let (mut controller, ids) = controller_with_tabs(3);

    assert!(!controller.move_tab(ids[1], 1, false));
    assert!(!controller.move_tab(TabId::new(999), 0, false));
    assert_eq!(order(&controller), ids);
}

#[test]
fn moving_a_tab_keeps_the_selection() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.select_tab(ids[1]));

    assert!(controller.move_tab(ids[0], 2, false));

    assert_eq!(controller.active_tab_id(), ids[1]);
}

#[test]
fn an_unpinned_tab_cannot_be_moved_among_the_pinned_ones() {
    let (mut controller, ids) = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));

    // Index 0 as an unpinned tab: the first place after the pinned tabs.
    assert!(controller.move_tab(ids[3], 0, false));

    assert_eq!(order(&controller), [ids[0], ids[1], ids[3], ids[2]]);
    assert_eq!(pinned(&controller), [ids[0], ids[1]]);
}

#[test]
fn a_tab_dragged_into_the_pinned_group_is_pinned_there() {
    let (mut controller, ids) = controller_with_tabs(5);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));

    assert!(controller.move_tab(ids[3], 1, true));

    assert_eq!(order(&controller), [ids[0], ids[3], ids[1], ids[2], ids[4]]);
    assert_eq!(pinned(&controller), [ids[0], ids[3], ids[1]]);
    assert_pinned_first(&controller);
}

#[test]
fn a_tab_dragged_past_the_pinned_group_as_pinned_joins_its_end() {
    let (mut controller, ids) = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[0], true));

    assert!(controller.move_tab(ids[3], 3, true));

    assert_eq!(order(&controller), [ids[0], ids[3], ids[1], ids[2]]);
    assert_eq!(pinned(&controller), [ids[0], ids[3]]);
}

#[test]
fn a_pinned_tab_dragged_out_of_the_group_is_unpinned_there() {
    let (mut controller, ids) = controller_with_tabs(5);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));

    assert!(controller.move_tab(ids[0], 3, false));

    assert_eq!(order(&controller), [ids[1], ids[2], ids[3], ids[0], ids[4]]);
    assert_eq!(pinned(&controller), [ids[1]]);
    assert_pinned_first(&controller);
}

#[test]
fn pinned_tabs_reorder_among_themselves() {
    let (mut controller, ids) = controller_with_tabs(4);
    for id in &ids[..3] {
        assert!(controller.set_tab_pinned(*id, true));
    }

    assert!(controller.move_tab(ids[0], 2, true));
    assert_eq!(order(&controller), [ids[1], ids[2], ids[0], ids[3]]);
    // An index among the unpinned tabs, as pinned: the end of the pinned group.
    assert!(controller.move_tab(ids[1], 3, true));
    assert_eq!(order(&controller), [ids[2], ids[0], ids[1], ids[3]]);
    assert_eq!(controller.pinned_tab_count(), 3);
}

#[test]
fn the_keyboard_moves_a_tab_one_place_within_its_group() {
    let (mut controller, ids) = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[0], true));

    assert!(controller.move_tab_by(ids[2], -1));
    assert_eq!(order(&controller), [ids[0], ids[2], ids[1], ids[3]]);
    // Not into the pinned group, and not past either end.
    assert!(!controller.move_tab_by(ids[2], -1));
    assert!(!controller.move_tab_by(ids[0], -1));
    assert!(!controller.move_tab_by(ids[0], 1));
    assert!(controller.move_tab_by(ids[1], 1));
    assert!(!controller.move_tab_by(ids[1], 1));
    assert_eq!(order(&controller), [ids[0], ids[2], ids[3], ids[1]]);
}

#[test]
fn closing_other_tabs_keeps_the_pinned_tabs_and_the_chosen_one() {
    let (mut controller, ids) = controller_with_tabs(5);
    assert!(controller.set_tab_pinned(ids[1], true));

    assert_eq!(controller.close_other_tabs(ids[3]), 3);

    assert_eq!(order(&controller), [ids[1], ids[3]]);
    assert_eq!(controller.active_tab_id(), ids[3]);
}

#[test]
fn closing_other_tabs_from_a_pinned_tab_keeps_every_pinned_tab() {
    let (mut controller, ids) = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[2], true));

    assert_eq!(controller.close_other_tabs(ids[2]), 2);

    assert_eq!(order(&controller), [ids[0], ids[2]]);
    assert_eq!(controller.active_tab_id(), ids[2]);
}

#[test]
fn closing_other_tabs_of_an_unknown_tab_closes_nothing() {
    let (mut controller, ids) = controller_with_tabs(3);

    assert_eq!(controller.close_other_tabs(TabId::new(999)), 0);
    assert_eq!(order(&controller), ids);
}

#[test]
fn closing_other_tabs_drops_their_queued_commands() {
    let (mut controller, ids) = controller_with_tabs(2);
    for id in &ids {
        assert!(controller.handle_event(*id, BrowserEvent::AddressChanged { url: String::new() }));
    }
    assert!(controller.queue_command_for(ids[1], talaria_core::BrowserCommand::Reload));

    assert_eq!(controller.close_other_tabs(ids[0]), 1);

    assert!(!controller.has_pending_commands());
}

#[test]
fn the_close_shortcut_on_a_pinned_tab_selects_the_first_unpinned_tab() {
    let (mut controller, ids) = controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));
    assert!(controller.select_tab(ids[0]));

    assert_eq!(
        controller.step_off_pinned_tab(ids[0]),
        PinnedCloseOutcome::SelectedUnpinned
    );

    assert_eq!(controller.active_tab_id(), ids[2]);
    assert_eq!(order(&controller), ids);
}

#[test]
fn the_close_shortcut_on_a_pinned_tab_opens_a_tab_when_every_tab_is_pinned() {
    let (mut controller, ids) = controller_with_tabs(2);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));

    assert_eq!(
        controller.step_off_pinned_tab(ids[1]),
        PinnedCloseOutcome::OpenedTab
    );

    assert_eq!(controller.tabs().len(), 3);
    let active = controller.active_tab_id();
    assert!(!ids.contains(&active));
    assert_eq!(order(&controller)[2], active);
    assert_eq!(controller.pinned_tab_count(), 2);
}

#[test]
fn the_close_shortcut_on_an_unpinned_tab_is_left_to_closing() {
    let (mut controller, ids) = controller_with_tabs(2);

    assert_eq!(
        controller.step_off_pinned_tab(ids[1]),
        PinnedCloseOutcome::NotPinned
    );
    assert_eq!(
        controller.step_off_pinned_tab(TabId::new(999)),
        PinnedCloseOutcome::NotPinned
    );
    assert_eq!(order(&controller), ids);
}

#[test]
fn a_tab_a_pinned_page_opens_goes_after_the_pinned_tabs() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));

    let opened = controller
        .adopt_tab(ids[0], TabActivation::Background)
        .expect("the opener exists");

    assert_eq!(order(&controller), [ids[0], ids[1], opened, ids[2]]);
    assert_pinned_first(&controller);
}

#[test]
fn a_tab_an_unpinned_page_opens_still_goes_next_to_it() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));

    let opened = controller
        .adopt_tab(ids[1], TabActivation::Background)
        .expect("the opener exists");

    assert_eq!(order(&controller), [ids[0], ids[1], opened, ids[2]]);
}

#[test]
fn a_query_matches_words_in_the_title_or_the_address_in_any_case() {
    let url = "https://www.example.org/news/today";
    assert!(tab_matches_query("Rust Weekly", url, "rust"));
    assert!(tab_matches_query("Rust Weekly", url, "WEEKLY rust"));
    assert!(tab_matches_query("Rust Weekly", url, "example news"));
    assert!(tab_matches_query("Rust Weekly", url, "weekly example"));
    assert!(!tab_matches_query("Rust Weekly", url, "rust python"));
    assert!(tab_matches_query("Café Ünïcode", "", "café ÜNÏ"));
}

#[test]
fn an_empty_query_matches_every_tab() {
    assert!(tab_matches_query("Anything", "https://a.test/", ""));
    assert!(tab_matches_query("", "", "   "));
}

#[test]
fn matching_tabs_are_listed_in_strip_order() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();
    assert!(controller.handle_event(
        first,
        BrowserEvent::AddressChanged {
            url: "https://docs.rs/slint".into()
        }
    ));
    let second = controller
        .open_tab_with(None, TabActivation::Background)
        .expect("a blank tab opens");
    assert!(controller.handle_event(
        second,
        BrowserEvent::AddressChanged {
            url: "https://servo.org/".into()
        }
    ));
    assert!(controller.handle_event(
        second,
        BrowserEvent::PageTitleChanged {
            title: Some("Servo, the embeddable engine".into())
        }
    ));
    let third = controller
        .open_tab_with(None, TabActivation::Background)
        .expect("a blank tab opens");
    assert!(controller.set_tab_pinned(third, true));

    assert_eq!(controller.matching_tabs("servo"), [second]);
    assert_eq!(controller.matching_tabs("docs.rs"), [first]);
    assert_eq!(controller.matching_tabs(""), [third, first, second]);
    assert!(controller.matching_tabs("nothing here").is_empty());
}

// Restored, reopened and duplicated tabs (session lane) keep pinned tabs first.

fn restored(url: &str, pinned: bool) -> talaria_core::RestoredTab {
    talaria_core::RestoredTab {
        url: Some(url.to_owned()),
        title: url.to_owned(),
        zoom_percent: 100,
        pinned,
    }
}

#[test]
fn restored_pinned_tabs_open_pinned_in_strip_order() {
    let mut controller = BrowserController::default();
    let blank = controller.active_tab_id();
    // A session is put back at the end of the strip, in order.
    let a = controller
        .open_restored_tab(1, &restored("https://a.test/", true))
        .expect("the address resolves");
    let b = controller
        .open_restored_tab(2, &restored("https://b.test/", false))
        .expect("the address resolves");
    let c = controller
        .open_restored_tab(3, &restored("https://c.test/", true))
        .expect("the address resolves");

    assert_eq!(order(&controller), [a, c, blank, b]);
    assert_eq!(pinned(&controller), [a, c]);
    assert_pinned_first(&controller);
}

#[test]
fn a_reopened_pinned_tab_comes_back_pinned() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));

    // It was at index 1 of the pinned group when it closed; the strip moved on since.
    let reopened = controller
        .open_restored_tab(5, &restored("https://pinned.test/", true))
        .expect("the address resolves");

    assert_eq!(order(&controller), [ids[0], reopened, ids[1], ids[2]]);
    assert_pinned_first(&controller);
}

#[test]
fn a_reopened_unpinned_tab_never_lands_among_the_pinned_tabs() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));

    // It closed from index 0, before those tabs were pinned.
    let reopened = controller
        .open_restored_tab(0, &restored("https://closed.test/", false))
        .expect("the address resolves");

    assert_eq!(order(&controller), [ids[0], ids[1], reopened, ids[2]]);
    assert_eq!(controller.pinned_tab_count(), 2);
    assert_pinned_first(&controller);
}

#[test]
fn a_duplicated_pinned_tab_goes_right_after_it_pinned() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));

    // Duplicate Tab opens the copy at the original's index + 1.
    let copy = controller
        .open_restored_tab(1, &restored("https://first.test/", true))
        .expect("the address resolves");

    assert_eq!(order(&controller), [ids[0], copy, ids[1], ids[2]]);
    assert_eq!(pinned(&controller), [ids[0], copy, ids[1]]);
}

#[test]
fn new_blank_tabs_go_after_the_pinned_tabs_even_when_all_are_pinned() {
    let (mut controller, ids) = controller_with_tabs(2);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));

    let opened = controller.open_tab();

    assert_eq!(order(&controller), [ids[0], ids[1], opened]);
    assert_pinned_first(&controller);
}

#[test]
fn closing_other_tabs_when_there_is_none_changes_no_selection() {
    let (mut controller, ids) = controller_with_tabs(2);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.select_tab(ids[1]));

    // Only the pinned tab is "other": nothing closes, and the selection stays.
    assert_eq!(controller.close_other_tabs(ids[0]), 0);

    assert_eq!(controller.active_tab_id(), ids[1]);
}
