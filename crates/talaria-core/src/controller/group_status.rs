// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Status flags, activity and keys of tab groups (groups-core lane, item `groups-status`).
//!
//! What the interface shows for a tab ([`TabStatus`]) and for a group ([`GroupSummary`]) is
//! worked out here, in one place:
//!
//! - `loading`, `audible`, `muted`, `pinned` and `capture` are read from the tab when asked;
//! - `unread` and `crashed` are the two flags the tab stores, because they depend on what happened
//!   while nobody looked. `watch_event` and `settle_event` bracket `BrowserState::apply` in
//!   `handle_event`: the first notes what a background tab shows now, the second compares and
//!   sets the flags (brief D6);
//! - `activate` is the one way a tab becomes the active tab. It clears `unread`, records when the
//!   tab was shown (for [`group_tabs_by_recency`]) and notes which groups were left and entered,
//!   for [`stamp_group_activity`].
//!
//! The strip operations are in `group_ops.rs`; the model of names, colours and keys is
//! `groups.rs`. Nothing here reads the clock: the caller hands the time in.
//!
//! [`group_tabs_by_recency`]: BrowserController::group_tabs_by_recency
//! [`stamp_group_activity`]: BrowserController::stamp_group_activity

use std::cmp::Reverse;
use std::time::SystemTime;

use crate::{
    BrowserEvent, GroupError, GroupId, GroupKey, GroupSummary, LoadState, Tab, TabAudio, TabId,
    TabStatus,
};

use super::BrowserController;

/// What an event does to a tab's crash mark.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum CrashChange {
    #[default]
    Unchanged,
    /// The page's process crashed.
    Crashed,
    /// A new document is loading.
    Recovered,
}

/// What `handle_event` noted about a tab before it applied an event, to compare afterwards.
#[derive(Debug, Default)]
pub(super) struct EventWatch {
    /// The tab exists and is not the active tab: only such a tab becomes unread.
    background: bool,
    /// The event says a load finished, and the tab's load state was something else until now.
    finished_loading: bool,
    /// The event is a new icon (or the loss of the icon) that is not the one the tab shows now.
    icon_changed: bool,
    /// The title the tab showed before a `PageTitleChanged`; `None` for every other event.
    title_before: Option<String>,
    crash: CrashChange,
}

impl BrowserController {
    /// The one place a tab becomes the active tab: every `select_*`, the replacement after a
    /// close, a foreground tab being opened. Clears the tab's `unread`, records when it was shown
    /// and notes the group that was left (the previously active tab's) and the one entered, so
    /// that the next [`stamp_group_activity`](Self::stamp_group_activity) knows both were active.
    pub(super) fn activate(&mut self, tab_id: TabId) {
        // None when the previously active tab was just closed: its group cannot be told any more.
        let leaving = self.active_tab().and_then(Tab::group);
        self.active_tab_id = tab_id;
        self.activation_clock += 1;
        let shown = self.activation_clock;
        let entering = self.tab_mut(tab_id).and_then(|tab| {
            tab.set_last_shown(shown);
            tab.set_unread(false);
            tab.group()
        });
        for group in [leaving, entering].into_iter().flatten() {
            // A group that was just deleted with its last tab is not noted.
            if self.groups.get(group).is_some() {
                self.groups.touch(group);
            }
        }
    }

    /// Called by `handle_event` before the event is applied to the tab.
    ///
    /// Takes only what is cheap and cannot be had afterwards: whether a load finishes, whether
    /// the icon differs (compared here because applying the event replaces the icon, and icons
    /// are large, so they are never copied) and the title before a title event.
    pub(super) fn watch_event(&self, tab_id: TabId, event: &BrowserEvent) -> EventWatch {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id() == tab_id) else {
            return EventWatch::default();
        };
        let state = tab.state();
        let mut watch = EventWatch {
            // The active tab is what the user is looking at: nothing in it is news.
            background: tab_id != self.active_tab_id,
            ..EventWatch::default()
        };
        match event {
            BrowserEvent::LoadStateChanged { state: load } => match load {
                LoadState::Loaded => {
                    watch.finished_loading = *state.load_state() != LoadState::Loaded;
                }
                LoadState::Loading => watch.crash = CrashChange::Recovered,
                LoadState::Idle | LoadState::Failed { .. } => {}
            },
            BrowserEvent::PageLoadStarted => watch.crash = CrashChange::Recovered,
            BrowserEvent::PageCrashed => watch.crash = CrashChange::Crashed,
            BrowserEvent::PageIconChanged { icon } => {
                watch.icon_changed = icon.as_ref() != state.page_icon();
            }
            // Only this event brings a title. A new address resets the title to the address's
            // host without one: that alone is not news (the page may still fail to load).
            BrowserEvent::PageTitleChanged { .. } if watch.background => {
                watch.title_before = Some(state.page_title().to_owned());
            }
            _ => {}
        }
        watch
    }

    /// Called by `handle_event` after the event was applied: sets or clears `unread` and
    /// `crashed`.
    pub(super) fn settle_event(&mut self, tab_id: TabId, watch: EventWatch) {
        let Some(tab) = self.tab_mut(tab_id) else {
            return;
        };
        match watch.crash {
            CrashChange::Crashed => tab.set_crashed(true),
            CrashChange::Recovered => tab.set_crashed(false),
            CrashChange::Unchanged => {}
        }
        if !watch.background {
            return;
        }
        // The title as the tab shows it now: a blank one has become a placeholder.
        let title_changed = watch
            .title_before
            .is_some_and(|before| before != tab.state().page_title());
        if watch.finished_loading || watch.icon_changed || title_changed {
            tab.set_unread(true);
        }
    }

    /// What the interface shows for a tab; `None` if there is no such tab.
    pub fn tab_status(&self, tab: TabId) -> Option<TabStatus> {
        self.tabs
            .iter()
            .find(|candidate| candidate.id() == tab)
            .map(status_of)
    }

    /// The flags of a group's tabs together; `None` if there is no such group.
    pub fn group_summary(&self, group: GroupId) -> Option<GroupSummary> {
        self.groups.get(group)?;
        Some(GroupSummary::from_statuses(
            self.tabs
                .iter()
                .filter(|tab| tab.group() == Some(group))
                .map(status_of),
        ))
    }

    /// The group's tabs, most recently shown first (ties in strip order). Empty for an unknown
    /// group.
    pub fn group_tabs_by_recency(&self, group: GroupId) -> Vec<TabId> {
        let mut tabs = self
            .tabs
            .iter()
            .filter(|tab| tab.group() == Some(group))
            .map(|tab| (tab.last_shown(), tab.id()))
            .collect::<Vec<_>>();
        // Stable: tabs shown at the same count (never shown: 0) stay in strip order.
        tabs.sort_by_key(|(last_shown, _)| Reverse(*last_shown));
        tabs.into_iter().map(|(_, tab)| tab).collect()
    }

    /// Shows the group's most recently shown tab. False for an unknown group.
    pub fn select_group(&mut self, group: GroupId) -> bool {
        match self.group_tabs_by_recency(group).first() {
            Some(&tab) => {
                self.activate(tab);
                true
            }
            None => false,
        }
    }

    /// Gives the groups that were active since the last call, and the active tab's group, `now`
    /// as their `last_active`. The caller (the desktop's sync tick) supplies the time.
    pub fn stamp_group_activity(&mut self, now: SystemTime) {
        let active = self.active_tab().and_then(Tab::group);
        self.groups.stamp_activity(now, active);
    }

    /// `Err(KeyTaken)` if another group has `key`.
    pub fn set_group_key(
        &mut self,
        group: GroupId,
        key: Option<GroupKey>,
    ) -> Result<(), GroupError> {
        self.groups.set_key(group, key)
    }

    /// The settings window's "Swap": the group that had `key` gets `group`'s old key (or none).
    pub fn swap_group_key(&mut self, group: GroupId, key: GroupKey) -> Result<(), GroupError> {
        self.groups.swap_key(group, key)
    }

    /// The letters new groups take, in order; empty turns automatic keys off.
    pub fn set_group_key_sequence(&mut self, keys: Vec<GroupKey>) {
        self.groups.set_key_sequence(keys);
    }
}

/// What the interface shows for one tab.
fn status_of(tab: &Tab) -> TabStatus {
    let audio = tab.state().audio_indicator();
    TabStatus {
        // A restored tab has no page until it is first shown, so nothing is loading in it.
        loading: *tab.state().load_state() == LoadState::Loading && !tab.is_load_deferred(),
        audible: audio == TabAudio::Playing,
        muted: audio == TabAudio::Muted,
        unread: tab.is_unread(),
        pinned: tab.is_pinned(),
        crashed: tab.has_crashed(),
        capture: tab.page_requests().capture_access(),
    }
}
