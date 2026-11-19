// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Pinned tabs, reordering, "close other tabs", the Cmd+W rule for pinned tabs and tab search.

use std::time::Duration;

use talaria_core::{
    BrowserCommand, BrowserController, BrowserEvent, CloseTabOutcome, ExternalLinkRequest,
    LoadState, MediaPlaybackState, MediaProbe, PageRequestId, PinnedCloseOutcome, TabActivation,
    TabAudio, TabCommand, TabId, VisitUpdate, tab_matches_query,
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
    assert!(controller.select_tab(ids[0]));

    // The only other tab is pinned: nothing closes, and the selection stays.
    assert_eq!(controller.close_other_tabs(ids[1]), 0);

    assert_eq!(controller.active_tab_id(), ids[0]);
}

// Audio mute (media-session lane) next to pinned tabs, sessions, reopened and duplicated tabs.
//
// Mute lives in each tab's state and the commands queued for it are addressed by tab id, so
// none of the moves below may change which tab is muted. Mute itself is never saved: a
// restored, reopened or duplicated tab opens unmuted.

/// Like `controller_with_tabs`, with every tab past its first address, so the commands queued
/// for it can be dispatched (a tab that is not engine-ready holds them back).
fn ready_controller_with_tabs(count: usize) -> (BrowserController, Vec<TabId>) {
    let (mut controller, ids) = controller_with_tabs(count);
    for id in &ids {
        assert!(controller.handle_event(*id, BrowserEvent::AddressChanged { url: String::new() }));
    }
    (controller, ids)
}

fn drain(controller: &mut BrowserController) -> Vec<TabCommand> {
    std::iter::from_fn(|| controller.next_command()).collect()
}

fn mute_command(tab_id: TabId) -> TabCommand {
    TabCommand::new(tab_id, BrowserCommand::SetAudioMuted { muted: true })
}

/// The page reports media; `audible` elements of it make sound.
fn probe_event(audible: u32) -> BrowserEvent {
    BrowserEvent::MediaProbed {
        probe: MediaProbe {
            playing: 1,
            audible,
            unseen: false,
            fullscreen_video: false,
        },
    }
}

fn address_event(url: &str) -> BrowserEvent {
    BrowserEvent::AddressChanged {
        url: url.to_owned(),
    }
}

/// The `SetAudioMuted` commands in the queue, dispatchable or not. The queue has no accessor,
/// and a command for a tab that is gone can never be dispatched, so `next_command` cannot show
/// whether it was dropped; the controller's `Debug` output lists the whole queue.
fn queued_mutes(controller: &BrowserController) -> usize {
    format!("{controller:?}").matches("SetAudioMuted").count()
}

/// Exactly the tabs in `muted` are muted, and show it; every other tab is silent.
fn assert_muted_exactly(controller: &BrowserController, muted: &[TabId]) {
    for tab in controller.tabs() {
        let is_muted = muted.contains(&tab.id());
        assert_eq!(tab.state().is_audio_muted(), is_muted, "{:?}", tab.id());
        let expected = if is_muted {
            TabAudio::Muted
        } else {
            TabAudio::Silent
        };
        assert_eq!(tab.state().audio_indicator(), expected, "{:?}", tab.id());
    }
}

/// Selects a restored tab and lets its page load and report audible media, as the engine would.
fn load_restored_tab(controller: &mut BrowserController, tab_id: TabId, url: &str) {
    assert!(controller.select_tab(tab_id));
    assert_eq!(controller.take_initial_url(tab_id).as_deref(), Some(url));
    assert!(controller.handle_event(tab_id, address_event(url)));
    assert!(controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: LoadState::Loaded
        }
    ));
    assert!(controller.handle_event(tab_id, probe_event(1)));
}

#[test]
fn a_muted_tab_stays_muted_when_it_is_pinned_and_unpinned() {
    let (mut controller, ids) = ready_controller_with_tabs(4);
    assert!(controller.toggle_audio_muted(ids[2]));
    assert_eq!(drain(&mut controller), [mute_command(ids[2])]);
    assert!(controller.select_tab(ids[1]));

    assert!(controller.set_tab_pinned(ids[2], true));
    assert_eq!(order(&controller), [ids[2], ids[0], ids[1], ids[3]]);
    assert_eq!(pinned(&controller), [ids[2]]);
    assert_muted_exactly(&controller, &[ids[2]]);
    assert_eq!(controller.active_tab_id(), ids[1]);

    assert!(controller.set_tab_pinned(ids[2], false));
    assert_eq!(order(&controller), [ids[2], ids[0], ids[1], ids[3]]);
    assert!(pinned(&controller).is_empty());
    assert_muted_exactly(&controller, &[ids[2]]);
    assert_eq!(controller.active_tab_id(), ids[1]);

    // With the muted tab itself selected, pinning keeps it selected.
    assert!(controller.select_tab(ids[2]));
    assert!(controller.set_tab_pinned(ids[2], true));
    assert_eq!(controller.active_tab_id(), ids[2]);
    assert_eq!(pinned(&controller), [ids[2]]);
    assert_muted_exactly(&controller, &[ids[2]]);
    // The page is still muted, so nothing is sent to it.
    assert!(drain(&mut controller).is_empty());
}

#[test]
fn a_muted_tab_stays_muted_when_dragged_into_and_out_of_the_pinned_group() {
    let (mut controller, ids) = ready_controller_with_tabs(5);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.toggle_audio_muted(ids[3]));
    assert!(controller.select_tab(ids[1]));
    assert_eq!(drain(&mut controller), [mute_command(ids[3])]);

    // Dragged in: pinned, at the index it was dropped on.
    assert!(controller.move_tab(ids[3], 1, true));
    assert_eq!(order(&controller), [ids[0], ids[3], ids[1], ids[2], ids[4]]);
    assert_eq!(pinned(&controller), [ids[0], ids[3]]);
    assert_muted_exactly(&controller, &[ids[3]]);
    assert_pinned_first(&controller);

    // Reordered among the pinned tabs.
    assert!(controller.move_tab(ids[3], 0, true));
    assert_eq!(order(&controller), [ids[3], ids[0], ids[1], ids[2], ids[4]]);
    assert_muted_exactly(&controller, &[ids[3]]);

    // Dragged out again, past the unpinned tabs.
    assert!(controller.move_tab(ids[3], 4, false));
    assert_eq!(order(&controller), [ids[0], ids[1], ids[2], ids[4], ids[3]]);
    assert_eq!(pinned(&controller), [ids[0]]);
    assert_muted_exactly(&controller, &[ids[3]]);
    assert_pinned_first(&controller);

    assert_eq!(controller.active_tab_id(), ids[1]);
    assert!(drain(&mut controller).is_empty());
}

#[test]
fn a_muted_tab_stays_muted_when_the_keyboard_moves_it() {
    let (mut controller, ids) = ready_controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));
    assert!(controller.toggle_audio_muted(ids[1]));
    assert!(controller.toggle_audio_muted(ids[3]));
    assert!(controller.select_tab(ids[2]));
    assert_eq!(drain(&mut controller).len(), 2);

    // A muted pinned tab moves within the pinned group, a muted unpinned one within its own.
    assert!(controller.move_tab_by(ids[1], -1));
    assert_eq!(order(&controller), [ids[1], ids[0], ids[2], ids[3]]);
    assert!(controller.move_tab_by(ids[3], -1));
    assert_eq!(order(&controller), [ids[1], ids[0], ids[3], ids[2]]);
    assert_muted_exactly(&controller, &[ids[1], ids[3]]);

    // At the edge of a group nothing moves, and nothing about the tab changes.
    assert!(!controller.move_tab_by(ids[1], -1));
    assert!(!controller.move_tab_by(ids[3], -1));
    assert!(controller.move_tab_by(ids[3], 1));
    assert!(!controller.move_tab_by(ids[3], 1));
    assert_eq!(order(&controller), [ids[1], ids[0], ids[2], ids[3]]);
    assert_eq!(pinned(&controller), [ids[1], ids[0]]);
    assert_muted_exactly(&controller, &[ids[1], ids[3]]);
    assert_pinned_first(&controller);

    assert_eq!(controller.active_tab_id(), ids[2]);
    assert!(drain(&mut controller).is_empty());
}

#[test]
fn a_muted_tab_stays_muted_as_the_kept_tab_of_close_other_tabs() {
    let (mut controller, ids) = ready_controller_with_tabs(5);
    assert!(controller.set_tab_pinned(ids[1], true));
    assert!(controller.toggle_audio_muted(ids[3]));
    assert!(controller.select_tab(ids[0]));
    drain(&mut controller);

    assert_eq!(controller.close_other_tabs(ids[3]), 3);

    assert_eq!(order(&controller), [ids[1], ids[3]]);
    assert_eq!(pinned(&controller), [ids[1]]);
    // The kept tab is selected, as for any tab, and is still the muted one.
    assert_eq!(controller.active_tab_id(), ids[3]);
    assert_muted_exactly(&controller, &[ids[3]]);
}

#[test]
fn a_muted_pinned_tab_stays_muted_whether_it_is_kept_or_other_tabs_close_around_it() {
    let (mut controller, ids) = ready_controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[2], true));
    assert!(controller.toggle_audio_muted(ids[0]));
    assert!(controller.select_tab(ids[1]));
    drain(&mut controller);

    // Kept by being pinned: only the unpinned tab that is not chosen closes.
    assert_eq!(controller.close_other_tabs(ids[3]), 1);
    assert_eq!(order(&controller), [ids[0], ids[2], ids[3]]);
    assert_eq!(controller.active_tab_id(), ids[3]);
    assert_muted_exactly(&controller, &[ids[0]]);

    // Kept by being the chosen tab.
    assert_eq!(controller.close_other_tabs(ids[0]), 1);
    assert_eq!(order(&controller), [ids[0], ids[2]]);
    assert_eq!(controller.active_tab_id(), ids[0]);
    assert_muted_exactly(&controller, &[ids[0]]);
    assert_pinned_first(&controller);
}

#[test]
fn the_audio_indicator_travels_with_its_tab_when_tabs_move() {
    let (mut controller, ids) = ready_controller_with_tabs(4);
    // ids[1] plays and ids[2] plays but is muted.
    for id in [ids[1], ids[2]] {
        assert!(controller.handle_event(
            id,
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Playing
            }
        ));
    }
    assert!(controller.toggle_audio_muted(ids[2]));
    drain(&mut controller);

    assert!(controller.set_tab_pinned(ids[2], true));
    assert!(controller.move_tab(ids[1], 0, true));

    let indicators = controller
        .tabs()
        .map(|tab| (tab.id(), tab.state().audio_indicator()))
        .collect::<Vec<_>>();
    assert_eq!(
        indicators,
        [
            (ids[1], TabAudio::Playing),
            (ids[2], TabAudio::Muted),
            (ids[0], TabAudio::Silent),
            (ids[3], TabAudio::Silent),
        ]
    );

    // Un-muting a pinned tab shows its playing media again.
    assert!(controller.toggle_audio_muted(ids[2]));
    assert_eq!(
        controller
            .tabs()
            .find(|tab| tab.id() == ids[2])
            .expect("the tab exists")
            .state()
            .audio_indicator(),
        TabAudio::Playing
    );
}

#[test]
fn toggling_mute_of_an_unknown_or_closed_tab_returns_false_and_queues_nothing() {
    let (mut controller, ids) = ready_controller_with_tabs(3);
    assert_eq!(controller.close_tab(ids[2]), CloseTabOutcome::Closed);
    // The way the queue is counted does see a mute that is queued.
    assert!(controller.toggle_audio_muted(ids[0]));
    assert_eq!(queued_mutes(&controller), 1);

    assert!(!controller.toggle_audio_muted(TabId::new(999)));
    assert!(!controller.toggle_audio_muted(ids[2]));

    assert_eq!(queued_mutes(&controller), 1);
    assert_eq!(drain(&mut controller), [mute_command(ids[0])]);
    assert_eq!(order(&controller), [ids[0], ids[1]]);
    assert_muted_exactly(&controller, &[ids[0]]);
}

#[test]
fn closing_other_tabs_drops_the_queued_mute_of_the_tabs_it_closes() {
    let (mut controller, ids) = ready_controller_with_tabs(5);
    assert!(controller.set_tab_pinned(ids[1], true));
    // Queued and not yet dispatched, in this order: a tab that closes, the kept tab, a pinned
    // tab (which stays) and a second tab that closes.
    for id in [ids[3], ids[2], ids[1], ids[4]] {
        assert!(controller.toggle_audio_muted(id));
    }
    assert_eq!(queued_mutes(&controller), 4);

    assert_eq!(controller.close_other_tabs(ids[2]), 3);

    assert_eq!(order(&controller), [ids[1], ids[2]]);
    assert_eq!(queued_mutes(&controller), 2);
    assert_eq!(
        drain(&mut controller),
        [mute_command(ids[2]), mute_command(ids[1])]
    );
    assert_muted_exactly(&controller, &[ids[1], ids[2]]);
}

#[test]
fn a_queued_mute_follows_its_tab_when_the_tab_moves() {
    let (mut controller, ids) = ready_controller_with_tabs(4);
    // Queued while the tab is third in the strip, dispatched after it has been moved around.
    assert!(controller.toggle_audio_muted(ids[2]));

    assert!(controller.set_tab_pinned(ids[2], true));
    assert!(controller.set_tab_pinned(ids[3], true));
    assert!(controller.move_tab_by(ids[2], 1));
    assert!(controller.set_tab_pinned(ids[2], false));
    assert_eq!(order(&controller), [ids[3], ids[2], ids[0], ids[1]]);

    // Index 2 is another tab now; the command is still for the muted one.
    assert_eq!(drain(&mut controller), [mute_command(ids[2])]);
    assert_muted_exactly(&controller, &[ids[2]]);
}

#[test]
fn a_reopened_pinned_tab_opens_unmuted_in_the_pinned_group() {
    let (mut controller, ids) = ready_controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.toggle_audio_muted(ids[0]));
    drain(&mut controller);
    // The tab's menu closes a pinned tab; it comes back from the closed-tab list.
    assert_eq!(controller.close_tab(ids[0]), CloseTabOutcome::Closed);

    let reopened = controller
        .open_restored_tab(0, &restored("https://pinned.test/", true))
        .expect("the address resolves");

    assert_ne!(reopened, ids[0]);
    assert_eq!(order(&controller), [reopened, ids[1], ids[2]]);
    assert_eq!(pinned(&controller), [reopened]);
    assert_muted_exactly(&controller, &[]);
    assert_eq!(queued_mutes(&controller), 0);

    // Selected and loaded, it is not muted either, whatever the page reports.
    load_restored_tab(&mut controller, reopened, "https://pinned.test/");
    assert!(drain(&mut controller).is_empty());
    assert_muted_exactly(&controller, &[]);
}

#[test]
fn restored_tabs_open_unmuted_beside_a_muted_tab_and_stay_unmuted_once_loaded() {
    let (mut controller, ids) = ready_controller_with_tabs(2);
    assert!(controller.toggle_audio_muted(ids[0]));
    drain(&mut controller);

    // A session is put back at the end of the strip, in order.
    let first = controller
        .open_restored_tab(1, &restored("https://a.test/", true))
        .expect("the address resolves");
    let second = controller
        .open_restored_tab(2, &restored("https://b.test/", false))
        .expect("the address resolves");

    assert_eq!(order(&controller), [first, ids[0], second, ids[1]]);
    assert_pinned_first(&controller);
    assert_muted_exactly(&controller, &[ids[0]]);
    assert_eq!(queued_mutes(&controller), 0);

    load_restored_tab(&mut controller, first, "https://a.test/");
    load_restored_tab(&mut controller, second, "https://b.test/");
    assert!(drain(&mut controller).is_empty());
    assert_eq!(queued_mutes(&controller), 0);
    assert_muted_exactly(&controller, &[ids[0]]);
}

#[test]
fn a_duplicate_of_a_muted_tab_opens_unmuted_and_the_original_stays_muted() {
    let (mut controller, ids) = ready_controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.toggle_audio_muted(ids[0]));
    assert!(controller.toggle_audio_muted(ids[2]));
    drain(&mut controller);
    assert_eq!(order(&controller), ids);

    // Duplicate Tab opens the copy at the original's index + 1, pinned like the original.
    let pinned_copy = controller
        .open_restored_tab(1, &restored("https://first.test/", true))
        .expect("the address resolves");
    let copy = controller
        .open_restored_tab(4, &restored("https://third.test/", false))
        .expect("the address resolves");

    assert_eq!(
        order(&controller),
        [ids[0], pinned_copy, ids[1], ids[2], copy]
    );
    assert_eq!(pinned(&controller), [ids[0], pinned_copy]);
    assert_muted_exactly(&controller, &[ids[0], ids[2]]);
    assert_eq!(queued_mutes(&controller), 0);

    load_restored_tab(&mut controller, pinned_copy, "https://first.test/");
    load_restored_tab(&mut controller, copy, "https://third.test/");
    assert!(drain(&mut controller).is_empty());
    assert_muted_exactly(&controller, &[ids[0], ids[2]]);
}

#[test]
fn reopened_tabs_still_land_in_their_own_group_among_muted_tabs() {
    let (mut controller, ids) = ready_controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));
    assert!(controller.toggle_audio_muted(ids[0]));
    assert!(controller.toggle_audio_muted(ids[2]));
    drain(&mut controller);

    // Closed from index 0, before those tabs were pinned: it goes after the pinned group.
    let unpinned = controller
        .open_restored_tab(0, &restored("https://unpinned.test/", false))
        .expect("the address resolves");
    assert_eq!(
        order(&controller),
        [ids[0], ids[1], unpinned, ids[2], ids[3]]
    );
    // Pinned, with an index far past the group: the end of the pinned group.
    let at_the_end = controller
        .open_restored_tab(99, &restored("https://end.test/", true))
        .expect("the address resolves");
    assert_eq!(
        order(&controller),
        [ids[0], ids[1], at_the_end, unpinned, ids[2], ids[3]]
    );
    // Pinned, at the front.
    let at_the_front = controller
        .open_restored_tab(0, &restored("https://front.test/", true))
        .expect("the address resolves");
    assert_eq!(
        order(&controller),
        [
            at_the_front,
            ids[0],
            ids[1],
            at_the_end,
            unpinned,
            ids[2],
            ids[3]
        ]
    );

    assert_eq!(controller.pinned_tab_count(), 4);
    assert_pinned_first(&controller);
    assert_muted_exactly(&controller, &[ids[0], ids[2]]);
    assert_eq!(queued_mutes(&controller), 0);
}

#[test]
fn tabs_opened_with_an_address_go_after_the_pinned_tabs_even_when_all_are_pinned() {
    // The way links from other apps and downloads open tabs.
    let (mut controller, ids) = ready_controller_with_tabs(2);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));

    let foreground = controller
        .open_tab_with(Some("https://one.test/"), TabActivation::Foreground)
        .expect("the address resolves");
    let background = controller
        .open_tab_with(Some("https://two.test/"), TabActivation::Background)
        .expect("the address resolves");

    assert_eq!(order(&controller), [ids[0], ids[1], foreground, background]);
    assert_eq!(controller.active_tab_id(), foreground);
    assert_pinned_first(&controller);
}

#[test]
fn a_tab_a_muted_pinned_page_opens_is_unmuted_and_goes_after_the_pinned_tabs() {
    let (mut controller, ids) = ready_controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.set_tab_pinned(ids[1], true));
    assert!(controller.toggle_audio_muted(ids[0]));
    drain(&mut controller);

    let opened = controller
        .adopt_tab(ids[0], TabActivation::Background)
        .expect("the opener exists");

    assert_eq!(order(&controller), [ids[0], ids[1], opened, ids[2]]);
    assert_eq!(pinned(&controller), [ids[0], ids[1]]);
    assert_pinned_first(&controller);
    assert_muted_exactly(&controller, &[ids[0]]);
    assert_eq!(controller.active_tab_id(), ids[0]);
    assert_eq!(queued_mutes(&controller), 0);

    // The new tab's own events do not mute it; the opener's still re-apply the opener's mute.
    assert!(controller.handle_event(opened, address_event("https://popup.test/")));
    assert!(controller.handle_event(opened, probe_event(1)));
    assert!(drain(&mut controller).is_empty());
    assert!(controller.handle_event(ids[0], probe_event(1)));
    assert_eq!(drain(&mut controller), [mute_command(ids[0])]);
    assert_muted_exactly(&controller, &[ids[0]]);
}

#[test]
fn a_foreground_tab_a_muted_pinned_page_opens_is_selected_and_unmuted() {
    let (mut controller, ids) = ready_controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.toggle_audio_muted(ids[0]));
    drain(&mut controller);

    let opened = controller
        .adopt_tab(ids[0], TabActivation::Foreground)
        .expect("the opener exists");

    assert_eq!(order(&controller), [ids[0], opened, ids[1], ids[2]]);
    assert_eq!(controller.active_tab_id(), opened);
    assert_pinned_first(&controller);
    assert_muted_exactly(&controller, &[ids[0]]);
    assert_eq!(queued_mutes(&controller), 0);
}

/// Four ready tabs; the third is muted, then pinned and moved to the front, so that no tab is
/// where the muted one was when it was muted.
fn muted_pinned_tab_that_moved() -> (BrowserController, Vec<TabId>) {
    let (mut controller, ids) = ready_controller_with_tabs(4);
    assert!(controller.toggle_audio_muted(ids[2]));
    assert_eq!(drain(&mut controller), [mute_command(ids[2])]);
    assert!(controller.set_tab_pinned(ids[1], true));
    assert!(controller.set_tab_pinned(ids[2], true));
    assert!(controller.move_tab(ids[2], 0, true));
    assert_eq!(order(&controller), [ids[2], ids[1], ids[0], ids[3]]);
    (controller, ids)
}

#[test]
fn a_muted_pinned_tab_that_moved_is_muted_again_for_its_own_id_when_media_is_audible() {
    let (mut controller, ids) = muted_pinned_tab_that_moved();

    // The page un-muted its media: one mute, for this tab, though it is no longer where it was.
    assert!(controller.handle_event(ids[2], probe_event(1)));
    assert_eq!(drain(&mut controller), [mute_command(ids[2])]);

    // Silenced media queues nothing, and neither do the other tabs, among them ids[0], which
    // now sits at the index the muted tab had when it was muted.
    assert!(controller.handle_event(ids[2], probe_event(0)));
    for id in [ids[0], ids[1], ids[3]] {
        assert!(controller.handle_event(id, probe_event(1)));
    }
    assert!(drain(&mut controller).is_empty());
    assert_muted_exactly(&controller, &[ids[2]]);
    assert_eq!(pinned(&controller), [ids[2], ids[1]]);
}

#[test]
fn a_muted_pinned_tab_that_moved_is_muted_again_for_its_own_id_on_a_new_address() {
    let (mut controller, ids) = muted_pinned_tab_that_moved();

    assert!(controller.handle_event(ids[2], address_event("https://next.test/")));
    assert_eq!(drain(&mut controller), [mute_command(ids[2])]);

    for id in [ids[0], ids[1], ids[3]] {
        assert!(controller.handle_event(id, address_event("https://other.test/")));
    }
    assert!(drain(&mut controller).is_empty());

    // Unpinned and moved again, it is still the one tab that is muted again.
    assert!(controller.set_tab_pinned(ids[2], false));
    assert!(controller.handle_event(ids[2], address_event("https://after-unpin.test/")));
    assert_eq!(drain(&mut controller), [mute_command(ids[2])]);
    assert_muted_exactly(&controller, &[ids[2]]);
    assert_pinned_first(&controller);
}

// Closing other tabs leaves behind what closing each of them would: the merged lanes keep
// per-tab state in the controller (history titles, requests to open links in other apps).

fn external_link(url: &str) -> BrowserEvent {
    BrowserEvent::ExternalLinkRequested(ExternalLinkRequest {
        id: PageRequestId::new(7),
        url: url.to_owned(),
        page_url: None,
        received_at: Duration::from_secs(2),
    })
}

fn commit_page(
    controller: &mut BrowserController,
    tab_id: TabId,
    url: &str,
    history_length: usize,
    history_index: usize,
) {
    assert!(controller.handle_event(tab_id, address_event(url)));
    assert!(controller.handle_event(
        tab_id,
        BrowserEvent::PageCommitted {
            url: url.to_owned(),
            history_length,
            history_index,
        }
    ));
}

/// Two ready tabs; the second one's page replaced its own history entry, so the new visit
/// carries the old page's title provisionally, to be written when the tab closes (the title
/// is placed in an earlier pump than the replacing commit, as in `visit_recording`). Returns
/// the controller with the history updates so far taken.
fn controller_with_a_provisional_title() -> (BrowserController, Vec<TabId>) {
    let (mut controller, ids) = ready_controller_with_tabs(2);
    let tab_id = ids[1];
    commit_page(&mut controller, tab_id, "https://app.test/", 2, 1);
    assert!(controller.handle_event(
        tab_id,
        BrowserEvent::PageTitleAt {
            url: "https://app.test/".to_owned(),
            title: Some("App".to_owned())
        }
    ));
    assert_eq!(
        controller.take_visit_updates().len(),
        2,
        "the visit and its title"
    );
    commit_page(&mut controller, tab_id, "https://app.test/?q=1", 2, 1);
    assert_eq!(
        controller.take_visit_updates().len(),
        2,
        "the old visit goes, the new one comes"
    );
    (controller, ids)
}

fn provisional_title_update() -> VisitUpdate {
    VisitUpdate::Title {
        url: "https://app.test/?q=1".to_owned(),
        title: "App".to_owned(),
    }
}

#[test]
fn closing_a_tab_writes_the_title_its_visit_carried() {
    // The reference for the next test: what `close_tab` does with a tab's history state.
    let (mut controller, ids) = controller_with_a_provisional_title();

    assert_eq!(controller.close_tab(ids[1]), CloseTabOutcome::Closed);

    assert_eq!(
        controller.take_visit_updates(),
        [provisional_title_update()]
    );
}

#[test]
fn closing_other_tabs_writes_the_title_the_visits_of_the_closed_tabs_carried() {
    let (mut controller, ids) = controller_with_a_provisional_title();

    assert_eq!(controller.close_other_tabs(ids[0]), 1);

    assert_eq!(
        controller.take_visit_updates(),
        [provisional_title_update()]
    );
}

#[test]
fn closing_other_tabs_drops_the_external_link_requests_of_the_tabs_it_closes() {
    // As for `close_tab`: a request from a tab that is gone is never shown.
    let (mut controller, ids) = ready_controller_with_tabs(4);
    assert!(controller.set_tab_pinned(ids[1], true));
    for (n, id) in ids.iter().enumerate() {
        controller.handle_event(*id, external_link(&format!("tel:{n}")));
    }

    assert_eq!(controller.close_other_tabs(ids[0]), 2);

    let requests = controller
        .take_external_link_requests()
        .into_iter()
        .map(|(tab_id, request)| (tab_id, request.url))
        .collect::<Vec<_>>();
    assert_eq!(
        requests,
        [(ids[0], "tel:0".to_owned()), (ids[1], "tel:1".to_owned())]
    );
}
