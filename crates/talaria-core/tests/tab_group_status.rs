// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the interface shows for a tab and for a group of tabs (groups-core lane, item
//! `groups-status`): the loading, sound, unread, crashed, pinned and capture flags, the recency of
//! a group's tabs, `select_group`, the "last active" stamps and the group keys.
//!
//! Tests that make groups also need the group model and the strip operations of the same lane.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use talaria_core::{
    BrowserController, BrowserEvent, CloseTabOutcome, GroupError, GroupId, GroupKey, GroupSummary,
    LoadState, MediaPlaybackState, MediaProbe, PageIcon, PinnedCloseOutcome, RestoredTab,
    TabActivation, TabId, TabStatus, placeholder_title,
};

// --- helpers -------------------------------------------------------------------------------

/// A controller with `count` tabs (the first one included), the first shown, the others opened
/// behind it. They are in the strip in this order.
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

fn loading() -> BrowserEvent {
    BrowserEvent::LoadStateChanged {
        state: LoadState::Loading,
    }
}

fn loaded() -> BrowserEvent {
    BrowserEvent::LoadStateChanged {
        state: LoadState::Loaded,
    }
}

fn failed() -> BrowserEvent {
    BrowserEvent::LoadStateChanged {
        state: LoadState::Failed {
            message: "the connection was refused".to_owned(),
        },
    }
}

fn title(text: &str) -> BrowserEvent {
    BrowserEvent::PageTitleChanged {
        title: Some(text.to_owned()),
    }
}

fn address(url: &str) -> BrowserEvent {
    BrowserEvent::AddressChanged {
        url: url.to_owned(),
    }
}

/// A 1 by 1 icon; two shades are two different icons.
fn icon(shade: u8) -> BrowserEvent {
    BrowserEvent::PageIconChanged {
        icon: Some(PageIcon::from_rgba(1, 1, vec![shade, 0, 0, 255]).expect("a 1 by 1 icon")),
    }
}

fn no_icon() -> BrowserEvent {
    BrowserEvent::PageIconChanged { icon: None }
}

fn playing() -> BrowserEvent {
    BrowserEvent::MediaPlaybackChanged {
        state: MediaPlaybackState::Playing,
    }
}

fn paused() -> BrowserEvent {
    BrowserEvent::MediaPlaybackChanged {
        state: MediaPlaybackState::Paused,
    }
}

fn send(controller: &mut BrowserController, tab: TabId, event: BrowserEvent) {
    assert!(
        controller.handle_event(tab, event),
        "the tab {tab:?} exists"
    );
}

fn status(controller: &BrowserController, tab: TabId) -> TabStatus {
    controller.tab_status(tab).expect("the tab exists")
}

fn unread(controller: &BrowserController, tab: TabId) -> bool {
    let from_status = status(controller, tab).unread;
    let from_tab = controller
        .tabs()
        .find(|candidate| candidate.id() == tab)
        .expect("the tab exists")
        .is_unread();
    assert_eq!(from_status, from_tab, "the status and the tab agree");
    from_status
}

/// A background tab's page finishes loading, which marks it unread.
fn make_unread(controller: &mut BrowserController, tab: TabId) {
    assert_ne!(controller.active_tab_id(), tab, "only a background tab");
    send(controller, tab, loaded());
    assert!(unread(controller, tab), "{tab:?} is unread");
}

/// The user looks at `tab` and goes back to `back`: whatever was unread in `tab` is read.
fn glance(controller: &mut BrowserController, tab: TabId, back: TabId) {
    assert!(controller.select_tab(tab));
    assert!(controller.select_tab(back));
}

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn group_of_tabs(controller: &mut BrowserController, tabs: &[TabId], now: u64) -> GroupId {
    controller
        .create_group(tabs, None, None, at(now))
        .expect("the group is made")
}

fn key(letter: char) -> GroupKey {
    GroupKey::from_letter(letter).expect("a letter")
}

fn key_of(controller: &BrowserController, group: GroupId) -> Option<GroupKey> {
    controller.group(group).expect("the group exists").key
}

fn last_active(controller: &BrowserController, group: GroupId) -> SystemTime {
    controller
        .group(group)
        .expect("the group exists")
        .last_active
}

fn assert_whole(controller: &BrowserController) {
    assert_eq!(controller.group_invariant_violation(), None);
}

fn restored(url: &str) -> RestoredTab {
    RestoredTab {
        url: Some(url.to_owned()),
        title: url.to_owned(),
        zoom_percent: 100,
        pinned: false,
    }
}

// --- a tab that is not known ---------------------------------------------------------------

#[test]
fn an_unknown_or_closed_tab_has_no_status() {
    let (mut controller, ids) = controller_with_tabs(2);
    assert_eq!(controller.tab_status(TabId::new(999)), None);

    assert_eq!(controller.close_tab(ids[1]), CloseTabOutcome::Closed);
    assert_eq!(controller.tab_status(ids[1]), None);
    assert!(controller.tab_status(ids[0]).is_some());
}

#[test]
fn an_event_for_an_unknown_tab_changes_nothing() {
    let (mut controller, ids) = controller_with_tabs(2);

    assert!(!controller.handle_event(TabId::new(999), loaded()));
    assert!(!controller.handle_event(TabId::new(999), BrowserEvent::PageCrashed));

    for tab in ids {
        assert_eq!(status(&controller, tab), TabStatus::default());
    }
}

#[test]
fn a_new_tab_has_no_flags() {
    let (controller, ids) = controller_with_tabs(2);

    for tab in ids {
        assert_eq!(status(&controller, tab), TabStatus::default());
    }
}

// --- loading -------------------------------------------------------------------------------

#[test]
fn a_tab_is_loading_exactly_while_its_load_state_is_loading() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];
    assert!(!status(&controller, tab).loading, "idle");

    send(&mut controller, tab, loading());
    assert!(status(&controller, tab).loading, "loading");
    assert!(!status(&controller, ids[0]).loading, "only that tab");

    send(&mut controller, tab, loaded());
    assert!(!status(&controller, tab).loading, "loaded");

    send(&mut controller, tab, loading());
    assert!(status(&controller, tab).loading, "loading again");
    send(&mut controller, tab, failed());
    assert!(!status(&controller, tab).loading, "failed");
}

#[test]
fn the_active_tab_loads_like_any_other() {
    let (mut controller, ids) = controller_with_tabs(2);

    send(&mut controller, ids[0], loading());
    assert!(status(&controller, ids[0]).loading);
    send(&mut controller, ids[0], loaded());
    assert!(!status(&controller, ids[0]).loading);
}

#[test]
fn a_tab_opened_with_an_address_is_loading_from_the_start() {
    let mut controller = BrowserController::default();
    let tab = controller
        .open_tab_with(Some("https://a.test/"), TabActivation::Background)
        .expect("the address resolves");

    assert!(status(&controller, tab).loading);
    assert!(!unread(&controller, tab), "starting to load is not news");
}

#[test]
fn a_restored_tab_whose_page_is_not_created_yet_is_not_loading() {
    let mut controller = BrowserController::default();
    let tab = controller
        .open_restored_tab(1, &restored("https://a.test/"))
        .expect("the address resolves");
    assert!(controller.is_restored_without_page(tab));
    assert!(!status(&controller, tab).loading, "nothing started yet");

    // Even if a load state were reported for it, the page does not exist, so nothing loads.
    send(&mut controller, tab, loading());
    assert!(!status(&controller, tab).loading);

    // Once it is shown and the engine takes its address, it loads like any other tab.
    assert!(controller.select_tab(tab));
    assert_eq!(
        controller.take_initial_url(tab).as_deref(),
        Some("https://a.test/")
    );
    assert!(!controller.is_restored_without_page(tab));
    assert!(status(&controller, tab).loading);
}

// --- unread: what sets it ------------------------------------------------------------------

#[test]
fn a_background_tab_that_finishes_loading_is_unread() {
    let (mut controller, ids) = controller_with_tabs(3);
    let (shown, tab, other) = (ids[0], ids[1], ids[2]);

    send(&mut controller, tab, loading());
    assert!(!unread(&controller, tab), "starting to load is not news");

    send(&mut controller, tab, loaded());
    assert!(unread(&controller, tab));
    assert!(!unread(&controller, shown));
    assert!(!unread(&controller, other));
}

#[test]
fn a_load_that_finishes_from_any_other_state_is_news() {
    let (mut controller, ids) = controller_with_tabs(2);
    let (shown, tab) = (ids[0], ids[1]);

    // Idle to loaded.
    send(&mut controller, tab, loaded());
    assert!(unread(&controller, tab));
    glance(&mut controller, tab, shown);

    // Loaded to loaded again: nothing happened.
    send(&mut controller, tab, loaded());
    assert!(!unread(&controller, tab));

    // Loading to loaded.
    send(&mut controller, tab, loading());
    send(&mut controller, tab, loaded());
    assert!(unread(&controller, tab));
    glance(&mut controller, tab, shown);

    // Failed to loaded: the page came back.
    send(&mut controller, tab, failed());
    assert!(!unread(&controller, tab), "a failure is not news");
    send(&mut controller, tab, loaded());
    assert!(unread(&controller, tab));
}

#[test]
fn a_failed_load_does_not_make_a_tab_unread() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    send(&mut controller, tab, loading());
    send(&mut controller, tab, failed());
    assert!(!unread(&controller, tab));

    // A tab that fails straight from idle neither.
    let (mut controller, ids) = controller_with_tabs(2);
    send(&mut controller, ids[1], failed());
    assert!(!unread(&controller, ids[1]));
}

#[test]
fn the_active_tab_never_becomes_unread() {
    let (mut controller, ids) = controller_with_tabs(2);
    let shown = ids[0];

    send(&mut controller, shown, loading());
    send(&mut controller, shown, loaded());
    send(&mut controller, shown, title("A new title"));
    send(&mut controller, shown, icon(1));
    send(&mut controller, shown, icon(2));
    send(&mut controller, shown, BrowserEvent::PageCrashed);
    assert!(!unread(&controller, shown));
}

#[test]
fn a_different_title_makes_a_background_tab_unread() {
    let (mut controller, ids) = controller_with_tabs(2);
    let (shown, tab) = (ids[0], ids[1]);

    send(&mut controller, tab, title("News"));
    assert!(unread(&controller, tab));
    glance(&mut controller, tab, shown);

    send(&mut controller, tab, title("More news"));
    assert!(unread(&controller, tab));
}

#[test]
fn the_same_title_again_does_not_make_a_tab_unread() {
    let (mut controller, ids) = controller_with_tabs(2);
    let (shown, tab) = (ids[0], ids[1]);

    send(&mut controller, tab, title("News"));
    assert!(unread(&controller, tab));
    glance(&mut controller, tab, shown);

    send(&mut controller, tab, title("News"));
    assert!(!unread(&controller, tab));
}

#[test]
fn a_blank_title_counts_as_the_placeholder_it_becomes() {
    let (mut controller, ids) = controller_with_tabs(2);
    let (shown, tab) = (ids[0], ids[1]);
    assert_eq!(
        controller
            .tabs()
            .find(|candidate| candidate.id() == tab)
            .expect("the tab exists")
            .state()
            .page_title(),
        placeholder_title("")
    );

    // A tab that has no title yet and is told it has none: nothing changed.
    send(
        &mut controller,
        tab,
        BrowserEvent::PageTitleChanged { title: None },
    );
    send(&mut controller, tab, title("   "));
    assert!(!unread(&controller, tab));

    // A real title, seen; then the page drops it and falls back to the placeholder: a change.
    send(&mut controller, tab, title("News"));
    assert!(unread(&controller, tab));
    glance(&mut controller, tab, shown);
    send(
        &mut controller,
        tab,
        BrowserEvent::PageTitleChanged { title: None },
    );
    assert!(unread(&controller, tab));
}

#[test]
fn a_different_icon_makes_a_background_tab_unread() {
    let (mut controller, ids) = controller_with_tabs(2);
    let (shown, tab) = (ids[0], ids[1]);

    // The first icon of a tab that had none.
    send(&mut controller, tab, icon(1));
    assert!(unread(&controller, tab));
    glance(&mut controller, tab, shown);

    // The same icon again: not news.
    send(&mut controller, tab, icon(1));
    assert!(!unread(&controller, tab));

    // Another icon.
    send(&mut controller, tab, icon(2));
    assert!(unread(&controller, tab));
    glance(&mut controller, tab, shown);

    // No icon where there was one.
    send(&mut controller, tab, no_icon());
    assert!(unread(&controller, tab));
    glance(&mut controller, tab, shown);

    // No icon where there was none.
    send(&mut controller, tab, no_icon());
    assert!(!unread(&controller, tab));
}

#[test]
fn a_new_address_alone_is_not_news() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    // A navigation starting resets the title to the new address's host without a title event;
    // it is only news once the page reports a title or finishes loading.
    send(&mut controller, tab, address("https://a.test/"));
    send(&mut controller, tab, BrowserEvent::PageLoadStarted);
    send(&mut controller, tab, loading());
    send(&mut controller, tab, address("https://b.test/"));
    assert!(!unread(&controller, tab));
    send(&mut controller, tab, failed());
    assert!(!unread(&controller, tab));
}

#[test]
fn events_that_are_not_news_leave_a_background_tab_read() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    send(&mut controller, tab, address(""));
    send(&mut controller, tab, loading());
    send(&mut controller, tab, BrowserEvent::PageLoadStarted);
    send(
        &mut controller,
        tab,
        BrowserEvent::HistoryChanged {
            can_go_back: true,
            can_go_forward: false,
        },
    );
    send(
        &mut controller,
        tab,
        BrowserEvent::StatusTextChanged {
            text: Some("https://a.test/".to_owned()),
        },
    );
    send(&mut controller, tab, playing());
    send(&mut controller, tab, paused());
    send(&mut controller, tab, BrowserEvent::PageCrashed);
    assert!(!unread(&controller, tab));
}

#[test]
fn unread_stays_until_the_tab_is_shown() {
    let (mut controller, ids) = controller_with_tabs(3);
    let (shown, tab, other) = (ids[0], ids[1], ids[2]);
    make_unread(&mut controller, tab);

    // Other tabs being shown, and more events, change nothing.
    assert!(controller.select_tab(other));
    assert!(controller.select_tab(shown));
    send(&mut controller, tab, BrowserEvent::PageLoadStarted);
    send(&mut controller, tab, playing());
    assert!(unread(&controller, tab));

    assert!(controller.select_tab(tab));
    assert!(!unread(&controller, tab));
}

// --- unread: every way a tab becomes the active tab clears it -----------------------------

#[test]
fn selecting_a_tab_reads_it() {
    let (mut controller, ids) = controller_with_tabs(3);
    make_unread(&mut controller, ids[1]);

    assert!(controller.select_tab(ids[1]));

    assert_eq!(controller.active_tab_id(), ids[1]);
    assert!(!unread(&controller, ids[1]));
}

#[test]
fn selecting_a_tab_by_index_reads_it() {
    let (mut controller, ids) = controller_with_tabs(3);
    make_unread(&mut controller, ids[2]);

    assert!(controller.select_tab_at(2));

    assert_eq!(controller.active_tab_id(), ids[2]);
    assert!(!unread(&controller, ids[2]));
}

#[test]
fn selecting_the_next_tab_reads_it() {
    let (mut controller, ids) = controller_with_tabs(3);
    make_unread(&mut controller, ids[1]);

    controller.select_next_tab(false);

    assert_eq!(controller.active_tab_id(), ids[1]);
    assert!(!unread(&controller, ids[1]));
}

#[test]
fn selecting_the_previous_tab_reads_it() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.select_tab(ids[2]));
    make_unread(&mut controller, ids[1]);

    controller.select_next_tab(true);

    assert_eq!(controller.active_tab_id(), ids[1]);
    assert!(!unread(&controller, ids[1]));
}

#[test]
fn selecting_the_last_tab_reads_it() {
    let (mut controller, ids) = controller_with_tabs(3);
    make_unread(&mut controller, ids[2]);

    controller.select_last_tab();

    assert_eq!(controller.active_tab_id(), ids[2]);
    assert!(!unread(&controller, ids[2]));
}

#[test]
fn closing_the_active_tab_reads_the_tab_that_replaces_it() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.select_tab(ids[2]));
    make_unread(&mut controller, ids[1]);
    make_unread(&mut controller, ids[0]);

    assert_eq!(controller.close_tab(ids[2]), CloseTabOutcome::Closed);

    // The tab before the closed one is shown; the one that was not chosen stays unread.
    assert_eq!(controller.active_tab_id(), ids[1]);
    assert!(!unread(&controller, ids[1]));
    assert!(unread(&controller, ids[0]));
}

#[test]
fn closing_other_tabs_reads_the_tab_that_is_kept() {
    let (mut controller, ids) = controller_with_tabs(3);
    make_unread(&mut controller, ids[1]);

    assert_eq!(controller.close_other_tabs(ids[1]), 2);

    assert_eq!(controller.active_tab_id(), ids[1]);
    assert!(!unread(&controller, ids[1]));
}

#[test]
fn stepping_off_a_pinned_tab_reads_the_unpinned_tab_it_selects() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[0], true));
    make_unread(&mut controller, ids[1]);
    make_unread(&mut controller, ids[2]);

    assert_eq!(
        controller.step_off_pinned_tab(ids[0]),
        PinnedCloseOutcome::SelectedUnpinned
    );

    assert_eq!(controller.active_tab_id(), ids[1]);
    assert!(!unread(&controller, ids[1]));
    assert!(unread(&controller, ids[2]));
}

#[test]
fn opening_a_tab_in_front_makes_the_old_tab_a_background_tab() {
    let (mut controller, ids) = controller_with_tabs(1);
    let shown = ids[0];

    let opened = controller.open_tab();

    assert_eq!(controller.active_tab_id(), opened);
    // The tab left behind now collects news; the new one, being shown, does not.
    send(&mut controller, shown, loaded());
    send(&mut controller, opened, loaded());
    assert!(unread(&controller, shown));
    assert!(!unread(&controller, opened));
}

#[test]
fn a_tab_a_page_opens_in_front_makes_the_old_tab_a_background_tab() {
    let (mut controller, ids) = controller_with_tabs(1);
    let shown = ids[0];

    let opened = controller
        .adopt_tab(shown, TabActivation::Foreground)
        .expect("the opener exists");

    assert_eq!(controller.active_tab_id(), opened);
    send(&mut controller, shown, title("The opener"));
    send(&mut controller, opened, title("The new tab"));
    assert!(unread(&controller, shown));
    assert!(!unread(&controller, opened));
}

#[test]
fn a_tab_a_page_opens_behind_collects_news_at_once() {
    let (mut controller, ids) = controller_with_tabs(1);
    let shown = ids[0];

    let opened = controller
        .adopt_tab(shown, TabActivation::Background)
        .expect("the opener exists");

    assert_eq!(controller.active_tab_id(), shown);
    send(&mut controller, opened, loaded());
    assert!(unread(&controller, opened));
}

// --- crashed -------------------------------------------------------------------------------

#[test]
fn a_crashed_page_is_marked_until_a_new_document_loads() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    // The engine reports the failed load first, then the crash.
    send(&mut controller, tab, failed());
    assert!(!status(&controller, tab).crashed, "a failure alone");
    send(&mut controller, tab, BrowserEvent::PageCrashed);
    assert!(status(&controller, tab).crashed);
    assert!(
        controller
            .tabs()
            .any(|candidate| candidate.id() == tab && candidate.has_crashed())
    );
    assert!(!status(&controller, ids[0]).crashed, "only that tab");

    send(&mut controller, tab, BrowserEvent::PageLoadStarted);
    assert!(!status(&controller, tab).crashed);
}

#[test]
fn a_load_starting_clears_the_crash_mark() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    send(&mut controller, tab, BrowserEvent::PageCrashed);
    assert!(status(&controller, tab).crashed);
    send(&mut controller, tab, loading());
    assert!(!status(&controller, tab).crashed);
    assert!(status(&controller, tab).loading);
}

#[test]
fn other_events_keep_the_crash_mark() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    send(&mut controller, tab, BrowserEvent::PageCrashed);
    send(&mut controller, tab, failed());
    send(&mut controller, tab, title("Aw, snap"));
    send(&mut controller, tab, icon(1));
    assert!(status(&controller, tab).crashed);
}

#[test]
fn the_active_tab_can_crash() {
    let (mut controller, ids) = controller_with_tabs(2);
    let shown = ids[0];

    send(&mut controller, shown, BrowserEvent::PageCrashed);

    assert!(status(&controller, shown).crashed);
    assert!(!unread(&controller, shown));
    // Selecting a tab does not clear its crash mark; only a new document does.
    assert!(controller.select_tab(ids[1]));
    assert!(controller.select_tab(shown));
    assert!(status(&controller, shown).crashed);
}

#[test]
fn navigating_a_crashed_tab_before_its_page_exists_clears_the_crash_mark() {
    let mut controller = BrowserController::default();
    let tab = controller
        .open_tab_with(Some("https://a.test/"), TabActivation::Foreground)
        .expect("the address resolves");
    send(&mut controller, tab, BrowserEvent::PageCrashed);
    assert!(status(&controller, tab).crashed);

    // The tab's web view is about to be created again, so the load start never reaches
    // `handle_event`; showing the load is what has to clear the mark.
    controller
        .queue_navigation("https://b.test/")
        .expect("the address resolves");

    assert!(!status(&controller, tab).crashed);
    assert!(status(&controller, tab).loading);
}

#[test]
fn a_crash_is_not_news() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    send(&mut controller, tab, BrowserEvent::PageCrashed);

    assert!(status(&controller, tab).crashed);
    assert!(!unread(&controller, tab));
}

// --- sound ---------------------------------------------------------------------------------

#[test]
fn a_tab_that_plays_media_is_audible() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    send(&mut controller, tab, playing());
    let flags = status(&controller, tab);
    assert!(flags.audible);
    assert!(!flags.muted);
    assert!(!status(&controller, ids[0]).audible);

    send(&mut controller, tab, paused());
    assert!(!status(&controller, tab).audible);
}

#[test]
fn media_that_a_probe_found_silent_is_not_audible() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    send(&mut controller, tab, playing());
    send(
        &mut controller,
        tab,
        BrowserEvent::MediaProbed {
            probe: MediaProbe {
                playing: 1,
                audible: 0,
                unseen: false,
                fullscreen_video: false,
            },
        },
    );
    assert!(!status(&controller, tab).audible);

    send(
        &mut controller,
        tab,
        BrowserEvent::MediaProbed {
            probe: MediaProbe {
                playing: 1,
                audible: 1,
                unseen: false,
                fullscreen_video: false,
            },
        },
    );
    assert!(status(&controller, tab).audible);
}

#[test]
fn a_muted_tab_shows_muted_whether_or_not_it_plays() {
    let (mut controller, ids) = controller_with_tabs(2);
    let tab = ids[1];

    assert!(controller.toggle_audio_muted(tab));
    let flags = status(&controller, tab);
    assert!(flags.muted, "muted while silent, so it can be unmuted");
    assert!(!flags.audible);

    send(&mut controller, tab, playing());
    let flags = status(&controller, tab);
    assert!(flags.muted);
    assert!(!flags.audible, "muted media makes no sound");

    assert!(controller.toggle_audio_muted(tab));
    let flags = status(&controller, tab);
    assert!(!flags.muted);
    assert!(flags.audible, "unmuted again, and still playing");
    assert!(!status(&controller, ids[0]).muted);
}

#[test]
fn muting_an_unknown_tab_leaves_every_status_alone() {
    let (mut controller, ids) = controller_with_tabs(2);

    assert!(!controller.toggle_audio_muted(TabId::new(999)));

    for tab in ids {
        assert!(!status(&controller, tab).muted);
    }
}

// --- pinned --------------------------------------------------------------------------------

#[test]
fn the_pinned_flag_follows_the_tab() {
    let (mut controller, ids) = controller_with_tabs(3);
    assert!(controller.set_tab_pinned(ids[2], true));

    assert!(status(&controller, ids[2]).pinned);
    assert!(!status(&controller, ids[0]).pinned);
    assert!(!status(&controller, ids[1]).pinned);

    assert!(controller.set_tab_pinned(ids[2], false));
    assert!(!status(&controller, ids[2]).pinned);
}

// --- camera and microphone -----------------------------------------------------------------

/// Camera and microphone are asked for only on macOS; elsewhere they are denied without a prompt.
#[cfg(target_os = "macos")]
mod capture {
    use super::*;
    use talaria_core::{
        CaptureAccess, PagePermission, PageRequest, PageRequestAnswer, PageRequestId,
        PageRequestKind, PageRequestReply,
    };

    const PAGE: &str = "https://meet.example/room";

    fn allow(controller: &mut BrowserController, tab: TabId, id: u64, permission: PagePermission) {
        send(controller, tab, address(PAGE));
        send(
            controller,
            tab,
            BrowserEvent::PageRequested(PageRequest {
                id: PageRequestId::new(id),
                url: PAGE.to_owned(),
                kind: PageRequestKind::Permission(permission),
                received_at: Duration::ZERO,
            }),
        );
        let pending = controller
            .pending_page_request(tab)
            .expect("the request is shown");
        assert!(controller.answer_page_request(
            tab,
            pending.id(),
            PageRequestReply::new(PageRequestAnswer::Accept),
            Duration::from_secs(1),
        ));
    }

    #[test]
    fn the_capture_flag_shows_what_the_page_may_use() {
        let (mut controller, ids) = controller_with_tabs(2);
        let tab = ids[1];
        assert_eq!(status(&controller, tab).capture, CaptureAccess::default());

        allow(&mut controller, tab, 1, PagePermission::Camera);
        assert_eq!(
            status(&controller, tab).capture,
            CaptureAccess {
                camera: true,
                microphone: false
            }
        );
        allow(&mut controller, tab, 2, PagePermission::Microphone);
        assert_eq!(
            status(&controller, tab).capture,
            CaptureAccess {
                camera: true,
                microphone: true
            }
        );
        assert_eq!(
            status(&controller, tab).capture,
            controller.capture_access(tab)
        );
        assert_eq!(
            status(&controller, ids[0]).capture,
            CaptureAccess::default()
        );
        assert!(!unread(&controller, tab), "a permission is not news");

        // A new document ends the access, and the flag with it.
        send(&mut controller, tab, BrowserEvent::PageLoadStarted);
        assert_eq!(status(&controller, tab).capture, CaptureAccess::default());
    }

    #[test]
    fn a_group_with_a_tab_that_captures_shows_it() {
        let (mut controller, ids) = controller_with_tabs(3);
        let group = group_of_tabs(&mut controller, &[ids[1], ids[2]], 1);
        assert!(
            !controller
                .group_summary(group)
                .expect("a group")
                .any_capture
        );

        allow(&mut controller, ids[2], 1, PagePermission::Microphone);

        assert!(
            controller
                .group_summary(group)
                .expect("a group")
                .any_capture
        );
        assert_whole(&controller);
    }
}

// --- the flags of a group ------------------------------------------------------------------

#[test]
fn a_group_summarises_the_flags_of_its_tabs_only() {
    let (mut controller, ids) = controller_with_tabs(5);
    let (shown, first, second, loose, pinned) = (ids[0], ids[1], ids[2], ids[3], ids[4]);
    let group = group_of_tabs(&mut controller, &[first, second], 1);
    assert!(controller.set_tab_pinned(pinned, true));
    assert_eq!(
        controller.group_summary(group),
        Some(GroupSummary {
            tabs: 2,
            ..GroupSummary::default()
        })
    );

    send(&mut controller, first, loading());
    assert_eq!(
        controller.group_summary(group),
        Some(GroupSummary {
            tabs: 2,
            loading_tabs: 1,
            any_loading: true,
            ..GroupSummary::default()
        })
    );

    send(&mut controller, second, loaded());
    send(&mut controller, second, playing());
    assert_eq!(
        controller.group_summary(group),
        Some(GroupSummary {
            tabs: 2,
            loading_tabs: 1,
            unread_tabs: 1,
            any_loading: true,
            any_audible: true,
            any_unread: true,
            any_crashed: false,
            any_capture: false,
        })
    );

    // Tabs outside the group, whatever they do, are not counted.
    send(&mut controller, loose, BrowserEvent::PageCrashed);
    send(&mut controller, loose, loaded());
    send(&mut controller, pinned, loading());
    send(&mut controller, shown, loading());
    let summary = controller.group_summary(group).expect("a group");
    assert!(!summary.any_crashed);
    assert_eq!(summary.tabs, 2);
    assert_eq!(summary.loading_tabs, 1);
    assert_eq!(summary.unread_tabs, 1);

    send(&mut controller, second, BrowserEvent::PageCrashed);
    assert!(
        controller
            .group_summary(group)
            .expect("a group")
            .any_crashed
    );

    // Looking at the unread tab reads it.
    assert!(controller.select_tab(second));
    let summary = controller.group_summary(group).expect("a group");
    assert_eq!(summary.unread_tabs, 0);
    assert!(!summary.any_unread);
    assert_whole(&controller);
}

#[test]
fn a_summary_counts_a_muted_tab_as_not_audible() {
    let (mut controller, ids) = controller_with_tabs(3);
    let group = group_of_tabs(&mut controller, &[ids[1], ids[2]], 1);
    send(&mut controller, ids[1], playing());
    send(&mut controller, ids[2], playing());
    assert!(
        controller
            .group_summary(group)
            .expect("a group")
            .any_audible
    );

    assert!(controller.toggle_audio_muted(ids[1]));
    assert!(
        controller
            .group_summary(group)
            .expect("a group")
            .any_audible
    );
    assert!(controller.toggle_audio_muted(ids[2]));
    assert!(
        !controller
            .group_summary(group)
            .expect("a group")
            .any_audible
    );
}

#[test]
fn an_unknown_group_has_no_summary() {
    let (mut controller, ids) = controller_with_tabs(2);
    let group = group_of_tabs(&mut controller, &[ids[1]], 1);

    assert_eq!(controller.group_summary(GroupId::new(999)), None);
    assert!(controller.group_summary(group).is_some());

    assert_eq!(controller.close_tab(ids[1]), CloseTabOutcome::Closed);
    assert_eq!(controller.group_summary(group), None, "the group is gone");
}

// --- recency and select_group --------------------------------------------------------------

/// `count` tabs, the first shown, with the tabs `members` (indexes) in one group.
fn world(count: usize, members: &[usize]) -> (BrowserController, Vec<TabId>, GroupId) {
    let (mut controller, ids) = controller_with_tabs(count);
    let tabs = members.iter().map(|index| ids[*index]).collect::<Vec<_>>();
    let group = group_of_tabs(&mut controller, &tabs, 1);
    (controller, ids, group)
}

#[test]
fn tabs_never_shown_are_in_strip_order() {
    let (controller, ids, group) = world(4, &[1, 2, 3]);

    assert_eq!(
        controller.group_tabs_by_recency(group),
        [ids[1], ids[2], ids[3]]
    );
}

#[test]
fn a_groups_tabs_are_listed_most_recently_shown_first() {
    let (mut controller, ids, group) = world(5, &[1, 2, 3]);
    let (shown, first, second, third) = (ids[0], ids[1], ids[2], ids[3]);

    assert!(controller.select_tab(third));
    assert!(controller.select_tab(shown));
    assert!(controller.select_tab(first));
    assert!(controller.select_tab(ids[4]));

    // `first` was shown last, then `third`; `second` was never shown.
    assert_eq!(
        controller.group_tabs_by_recency(group),
        [first, third, second]
    );
    assert_eq!(
        controller.group_tabs(group),
        [first, second, third],
        "the strip order is not touched"
    );

    assert!(controller.select_tab(second));
    assert_eq!(
        controller.group_tabs_by_recency(group),
        [second, first, third]
    );
    assert_whole(&controller);
}

#[test]
fn only_tabs_that_are_shown_count_as_shown() {
    let (mut controller, ids, group) = world(4, &[1, 2, 3]);

    // Events, moves and pins of background tabs do not make them recent.
    send(&mut controller, ids[3], loaded());
    send(&mut controller, ids[3], title("Busy"));
    assert!(controller.toggle_audio_muted(ids[2]));

    assert_eq!(
        controller.group_tabs_by_recency(group),
        [ids[1], ids[2], ids[3]]
    );
}

#[test]
fn every_way_of_showing_a_tab_makes_it_the_most_recent() {
    // Every tab is in the group, so every path ends in it.
    let all = [0, 1, 2, 3];

    let (mut controller, ids, group) = world(4, &all);
    assert!(controller.select_tab(ids[2]));
    assert_eq!(controller.group_tabs_by_recency(group)[0], ids[2]);

    let (mut controller, ids, group) = world(4, &all);
    assert!(controller.select_tab_at(1));
    assert_eq!(controller.group_tabs_by_recency(group)[0], ids[1]);

    let (mut controller, ids, group) = world(4, &all);
    controller.select_next_tab(false);
    assert_eq!(controller.group_tabs_by_recency(group)[0], ids[1]);

    let (mut controller, ids, group) = world(4, &all);
    controller.select_next_tab(true);
    assert_eq!(controller.group_tabs_by_recency(group)[0], ids[3]);

    let (mut controller, ids, group) = world(4, &all);
    controller.select_last_tab();
    assert_eq!(controller.group_tabs_by_recency(group)[0], ids[3]);

    // Closing the shown tab shows its neighbour.
    let (mut controller, ids, group) = world(4, &all);
    assert!(controller.select_tab(ids[2]));
    assert_eq!(controller.close_tab(ids[2]), CloseTabOutcome::Closed);
    assert_eq!(controller.group_tabs_by_recency(group)[0], ids[1]);

    // Closing the others shows the one that is kept.
    let (mut controller, ids, group) = world(4, &all);
    assert_eq!(controller.close_other_tabs(ids[2]), 3);
    assert_eq!(controller.group_tabs_by_recency(group), [ids[2]]);

    // A tab a page opens in front joins its opener's group and is shown.
    let (mut controller, ids, group) = world(4, &all);
    let opened = controller
        .adopt_tab(ids[1], TabActivation::Foreground)
        .expect("the opener exists");
    assert_eq!(controller.active_tab_id(), opened);
    assert_eq!(controller.group_of(opened), Some(group));
    assert_eq!(controller.group_tabs_by_recency(group)[0], opened);
    assert_whole(&controller);
}

#[test]
fn stepping_off_a_pinned_tab_makes_the_unpinned_tab_the_most_recent() {
    let (mut controller, ids, group) = world(4, &[1, 2, 3]);
    assert!(controller.set_tab_pinned(ids[0], true));
    assert!(controller.select_tab(ids[3]));
    assert!(controller.select_tab(ids[0]));

    assert_eq!(
        controller.step_off_pinned_tab(ids[0]),
        PinnedCloseOutcome::SelectedUnpinned
    );

    assert_eq!(controller.active_tab_id(), ids[1]);
    assert_eq!(controller.group_tabs_by_recency(group)[0], ids[1]);
}

#[test]
fn an_unknown_group_has_no_tabs_by_recency() {
    let (controller, _ids, _group) = world(3, &[1, 2]);

    assert!(
        controller
            .group_tabs_by_recency(GroupId::new(999))
            .is_empty()
    );
}

#[test]
fn selecting_a_group_shows_its_most_recently_shown_tab() {
    let (mut controller, ids, group) = world(5, &[1, 2, 3]);
    assert!(controller.select_tab(ids[3]));
    assert!(controller.select_tab(ids[2]));
    assert!(controller.select_tab(ids[4]));
    assert_eq!(controller.active_tab_id(), ids[4]);

    assert!(controller.select_group(group));

    assert_eq!(controller.active_tab_id(), ids[2]);
    assert_eq!(controller.group_tabs_by_recency(group)[0], ids[2]);
    assert_whole(&controller);
}

#[test]
fn selecting_a_group_that_was_never_shown_shows_its_first_tab() {
    let (mut controller, ids, group) = world(4, &[1, 2, 3]);

    assert!(controller.select_group(group));

    assert_eq!(controller.active_tab_id(), ids[1]);
}

#[test]
fn selecting_the_group_that_is_shown_keeps_its_tab() {
    let (mut controller, ids, group) = world(4, &[1, 2, 3]);
    assert!(controller.select_tab(ids[2]));
    assert!(controller.select_tab(ids[3]));

    assert!(controller.select_group(group));

    assert_eq!(controller.active_tab_id(), ids[3]);
}

#[test]
fn selecting_an_unknown_group_changes_nothing() {
    let (mut controller, ids, group) = world(4, &[1, 2, 3]);
    assert!(controller.select_tab(ids[2]));

    assert!(!controller.select_group(GroupId::new(999)));

    assert_eq!(controller.active_tab_id(), ids[2]);
    assert_eq!(controller.group_tabs_by_recency(group)[0], ids[2]);
}

#[test]
fn selecting_a_group_reads_the_tab_it_shows() {
    let (mut controller, ids, group) = world(4, &[1, 2, 3]);
    assert!(controller.select_tab(ids[2]));
    assert!(controller.select_tab(ids[1]));
    assert!(controller.select_tab(ids[0]));
    make_unread(&mut controller, ids[1]);
    make_unread(&mut controller, ids[3]);

    assert!(controller.select_group(group));

    // The tab shown last is the one that opens; the group's other unread tab stays unread.
    assert_eq!(controller.active_tab_id(), ids[1]);
    assert!(!unread(&controller, ids[1]));
    assert!(unread(&controller, ids[3]));
}

#[test]
fn selecting_a_group_works_for_a_restored_tab_that_has_no_page_yet() {
    let mut controller = BrowserController::default();
    let blank = controller.active_tab_id();
    let first = controller
        .open_restored_tab(1, &restored("https://a.test/"))
        .expect("the address resolves");
    let second = controller
        .open_restored_tab(2, &restored("https://b.test/"))
        .expect("the address resolves");
    let group = group_of_tabs(&mut controller, &[first, second], 1);
    assert!(controller.is_load_deferred(first));
    assert!(controller.is_load_deferred(second));

    assert!(controller.select_group(group));

    // The first tab is shown, so its page is created the way `select_tab` does it.
    assert_eq!(controller.active_tab_id(), first);
    assert!(!controller.is_load_deferred(first));
    assert!(controller.is_load_deferred(second));
    assert_eq!(
        controller.take_initial_url(first).as_deref(),
        Some("https://a.test/")
    );
    assert!(status(&controller, first).loading);
    assert!(!status(&controller, second).loading);
    assert_ne!(controller.active_tab_id(), blank);
    assert_whole(&controller);
}

// --- last active ---------------------------------------------------------------------------

#[test]
fn a_new_group_has_no_activity_until_it_is_stamped() {
    let (controller, _ids, group) = world(3, &[1, 2]);

    assert_eq!(controller.group(group).expect("a group").created, at(1));
    assert_eq!(last_active(&controller, group), at(1));
}

#[test]
fn the_group_of_the_shown_tab_is_stamped() {
    let (mut controller, ids, group) = world(3, &[0, 1]);
    let other = group_of_tabs(&mut controller, &[ids[2]], 1);

    controller.stamp_group_activity(at(10));

    assert_eq!(last_active(&controller, group), at(10));
    assert_eq!(last_active(&controller, other), at(1), "not shown");
    assert_eq!(
        controller.group(group).expect("a group").created,
        at(1),
        "the creation time is not touched"
    );
}

#[test]
fn nothing_is_stamped_while_a_loose_tab_is_shown_and_nothing_happened() {
    let (mut controller, _ids, group) = world(3, &[1, 2]);

    controller.stamp_group_activity(at(10));
    controller.stamp_group_activity(at(20));

    assert_eq!(last_active(&controller, group), at(1));
}

#[test]
fn a_group_that_was_shown_is_stamped_once_and_then_forgotten() {
    // [c, a, b]: the group is `a` and `b`, the loose tab `c` is shown.
    let (mut controller, ids, group) = world(3, &[1, 2]);
    assert!(controller.select_tab(ids[1]));
    assert!(controller.select_tab(ids[0]));

    controller.stamp_group_activity(at(10));
    assert_eq!(last_active(&controller, group), at(10));

    // Nothing happened since: the group stays as it was.
    controller.stamp_group_activity(at(20));
    assert_eq!(last_active(&controller, group), at(10));
}

#[test]
fn leaving_a_group_by_any_path_stamps_it() {
    type Path = fn(&mut BrowserController, &[TabId]);
    // Strip [a, b, c]: the group is `a` and `b`, with `a` shown; every path ends on the loose `c`
    // or on a tab opened behind the group.
    let paths: [(&str, Path); 6] = [
        ("select_tab", |controller, ids| {
            assert!(controller.select_tab(ids[2]));
        }),
        ("select_tab_at", |controller, _| {
            assert!(controller.select_tab_at(2));
        }),
        ("select_last_tab", |controller, _| {
            controller.select_last_tab()
        }),
        ("select_next_tab backwards", |controller, _| {
            controller.select_next_tab(true);
        }),
        ("open_tab", |controller, _| {
            controller.open_tab();
        }),
        ("adopt_tab in front", |controller, ids| {
            assert!(
                controller
                    .adopt_tab(ids[2], TabActivation::Foreground)
                    .is_some()
            );
        }),
    ];

    for (name, path) in paths {
        let (mut controller, ids, group) = world(3, &[0, 1]);
        assert_eq!(controller.group_of(controller.active_tab_id()), Some(group));

        path(&mut controller, &ids);

        assert_eq!(
            controller.group_of(controller.active_tab_id()),
            None,
            "{name}"
        );
        controller.stamp_group_activity(at(10));
        assert_eq!(last_active(&controller, group), at(10), "{name}");
        assert_whole(&controller);
    }
}

#[test]
fn a_group_visited_between_two_stamps_is_stamped_even_if_left_again() {
    // [a1, a2, b1, b2]: groups A and B; A is shown.
    let (mut controller, ids) = controller_with_tabs(4);
    let group_a = group_of_tabs(&mut controller, &[ids[0], ids[1]], 1);
    let group_b = group_of_tabs(&mut controller, &[ids[2], ids[3]], 1);
    controller.stamp_group_activity(at(10));
    assert_eq!(last_active(&controller, group_a), at(10));
    assert_eq!(last_active(&controller, group_b), at(1));

    // Into B and out again before the next stamp, ending in A.
    assert!(controller.select_tab(ids[2]));
    assert!(controller.select_tab(ids[1]));
    controller.stamp_group_activity(at(20));

    assert_eq!(last_active(&controller, group_a), at(20), "shown");
    assert_eq!(last_active(&controller, group_b), at(20), "visited");
}

#[test]
fn moving_between_groups_stamps_the_one_left_and_the_one_entered() {
    let (mut controller, ids) = controller_with_tabs(4);
    let group_a = group_of_tabs(&mut controller, &[ids[0], ids[1]], 1);
    let group_b = group_of_tabs(&mut controller, &[ids[2], ids[3]], 1);
    controller.stamp_group_activity(at(10));

    assert!(controller.select_group(group_b));
    controller.stamp_group_activity(at(20));
    assert_eq!(last_active(&controller, group_a), at(20), "left");
    assert_eq!(last_active(&controller, group_b), at(20), "entered");

    // Staying in B: A keeps its time, B moves on.
    assert!(controller.select_tab(ids[3]));
    controller.stamp_group_activity(at(30));
    assert_eq!(last_active(&controller, group_a), at(20));
    assert_eq!(last_active(&controller, group_b), at(30));
}

#[test]
fn a_group_entered_and_then_lost_by_closing_its_shown_tab_is_still_stamped() {
    // [c, a, b]: the group is `a` and `b`, the loose `c` is shown. Closing `a` shows `c`.
    let (mut controller, ids, group) = world(3, &[1, 2]);
    assert!(controller.select_tab(ids[1]));
    assert_eq!(controller.close_tab(ids[1]), CloseTabOutcome::Closed);
    assert_eq!(controller.active_tab_id(), ids[0]);

    controller.stamp_group_activity(at(10));

    assert_eq!(last_active(&controller, group), at(10));
    assert_whole(&controller);
}

// --- group keys ----------------------------------------------------------------------------

/// `count` tabs and one group of each tab but the first, in strip order.
fn keyed_world(count: usize) -> (BrowserController, Vec<GroupId>) {
    let (mut controller, ids) = controller_with_tabs(count);
    let groups = ids[1..]
        .iter()
        .map(|tab| group_of_tabs(&mut controller, &[*tab], 1))
        .collect();
    (controller, groups)
}

#[test]
fn new_groups_take_the_keys_a_s_d_in_creation_order() {
    let (controller, groups) = keyed_world(5);

    assert_eq!(key_of(&controller, groups[0]), Some(key('A')));
    assert_eq!(key_of(&controller, groups[1]), Some(key('S')));
    assert_eq!(key_of(&controller, groups[2]), Some(key('D')));
    assert_eq!(key_of(&controller, groups[3]), Some(key('F')));
    assert_whole(&controller);
}

#[test]
fn a_key_that_another_group_has_is_refused() {
    let (mut controller, groups) = keyed_world(4);

    assert_eq!(
        controller.set_group_key(groups[1], Some(key('A'))),
        Err(GroupError::KeyTaken { holder: groups[0] })
    );

    assert_eq!(key_of(&controller, groups[0]), Some(key('A')));
    assert_eq!(key_of(&controller, groups[1]), Some(key('S')), "unchanged");
}

#[test]
fn a_group_may_keep_set_change_and_drop_its_key() {
    let (mut controller, groups) = keyed_world(3);

    assert_eq!(controller.set_group_key(groups[0], Some(key('A'))), Ok(()));
    assert_eq!(controller.set_group_key(groups[0], Some(key('Q'))), Ok(()));
    assert_eq!(key_of(&controller, groups[0]), Some(key('Q')));

    // The old key is free for another group.
    assert_eq!(controller.set_group_key(groups[1], Some(key('A'))), Ok(()));
    assert_eq!(key_of(&controller, groups[1]), Some(key('A')));

    assert_eq!(controller.set_group_key(groups[1], None), Ok(()));
    assert_eq!(key_of(&controller, groups[1]), None);
    assert_eq!(controller.set_group_key(groups[0], Some(key('A'))), Ok(()));
}

#[test]
fn swapping_gives_the_holder_the_old_key() {
    let (mut controller, groups) = keyed_world(4);

    assert_eq!(controller.swap_group_key(groups[1], key('A')), Ok(()));

    assert_eq!(key_of(&controller, groups[1]), Some(key('A')));
    assert_eq!(key_of(&controller, groups[0]), Some(key('S')));
    assert_eq!(key_of(&controller, groups[2]), Some(key('D')), "untouched");
}

#[test]
fn swapping_with_a_group_that_has_no_key_leaves_the_holder_without_one() {
    let (mut controller, groups) = keyed_world(3);
    assert_eq!(controller.set_group_key(groups[1], None), Ok(()));

    assert_eq!(controller.swap_group_key(groups[1], key('A')), Ok(()));

    assert_eq!(key_of(&controller, groups[1]), Some(key('A')));
    assert_eq!(key_of(&controller, groups[0]), None);
}

#[test]
fn swapping_to_a_free_key_just_moves_it() {
    let (mut controller, groups) = keyed_world(3);

    assert_eq!(controller.swap_group_key(groups[0], key('Q')), Ok(()));

    assert_eq!(key_of(&controller, groups[0]), Some(key('Q')));
    assert_eq!(key_of(&controller, groups[1]), Some(key('S')));
    // `A` is nobody's now.
    assert_eq!(controller.set_group_key(groups[1], Some(key('A'))), Ok(()));
}

#[test]
fn keys_of_an_unknown_group_are_refused() {
    let (mut controller, _groups) = keyed_world(3);

    assert_eq!(
        controller.set_group_key(GroupId::new(999), Some(key('Q'))),
        Err(GroupError::NotFound)
    );
    assert_eq!(
        controller.swap_group_key(GroupId::new(999), key('Q')),
        Err(GroupError::NotFound)
    );
}

#[test]
fn a_custom_key_sequence_gives_new_groups_its_letters_in_order() {
    let (mut controller, ids) = controller_with_tabs(5);
    controller.set_group_key_sequence(vec![key('Q'), key('W'), key('E')]);

    let groups = ids[..4]
        .iter()
        .map(|tab| group_of_tabs(&mut controller, &[*tab], 1))
        .collect::<Vec<_>>();

    assert_eq!(key_of(&controller, groups[0]), Some(key('Q')));
    assert_eq!(key_of(&controller, groups[1]), Some(key('W')));
    assert_eq!(key_of(&controller, groups[2]), Some(key('E')));
    assert_eq!(
        key_of(&controller, groups[3]),
        None,
        "the sequence is used up"
    );
}

#[test]
fn a_new_sequence_does_not_take_keys_from_groups_that_have_them() {
    let (mut controller, groups) = keyed_world(3);
    assert_eq!(key_of(&controller, groups[0]), Some(key('A')));

    controller.set_group_key_sequence(vec![key('A'), key('Q')]);
    let ids = controller.loose_tabs();
    let added = group_of_tabs(&mut controller, &ids[..1], 2);

    assert_eq!(key_of(&controller, groups[0]), Some(key('A')), "kept");
    assert_eq!(key_of(&controller, groups[1]), Some(key('S')), "kept");
    assert_eq!(key_of(&controller, added), Some(key('Q')), "the first free");
}

#[test]
fn an_empty_key_sequence_turns_automatic_keys_off() {
    let (mut controller, ids) = controller_with_tabs(4);
    let first = group_of_tabs(&mut controller, &[ids[0]], 1);
    assert_eq!(key_of(&controller, first), Some(key('A')));

    controller.set_group_key_sequence(Vec::new());
    let second = group_of_tabs(&mut controller, &[ids[1]], 1);
    let third = group_of_tabs(&mut controller, &[ids[2]], 1);

    assert_eq!(key_of(&controller, first), Some(key('A')), "kept");
    assert_eq!(key_of(&controller, second), None);
    assert_eq!(key_of(&controller, third), None);
    // Keys can still be given by hand.
    assert_eq!(controller.set_group_key(second, Some(key('Z'))), Ok(()));
    assert_eq!(key_of(&controller, second), Some(key('Z')));
}

#[test]
fn automatic_keys_skip_the_ones_that_are_taken() {
    let (mut controller, ids) = controller_with_tabs(4);
    let first = group_of_tabs(&mut controller, &[ids[0]], 1);
    assert_eq!(controller.set_group_key(first, Some(key('S'))), Ok(()));

    let second = group_of_tabs(&mut controller, &[ids[1]], 1);
    let third = group_of_tabs(&mut controller, &[ids[2]], 1);

    assert_eq!(key_of(&controller, second), Some(key('A')));
    assert_eq!(key_of(&controller, third), Some(key('D')));
}
