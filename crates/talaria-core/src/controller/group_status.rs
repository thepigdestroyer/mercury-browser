// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Status flags, activity and keys of tab groups (groups-core lane, item `groups-status`).
//!
//! **SKELETON.** `activate`, `watch_event` and `settle_event` are the hooks `controller.rs`
//! already calls (so the existing code paths run the lane's code from the first commit); they do
//! the minimum today. The public methods say `todo!("groups-status")` until that item fills them
//! in. Signatures are the lane's frozen contract (`lane-groups-core-brief.md` §5); the strip
//! operations are in `group_ops.rs`.

use std::time::SystemTime;

use crate::{BrowserEvent, GroupError, GroupId, GroupKey, GroupSummary, TabId, TabStatus};

use super::BrowserController;

/// What `handle_event` noted about a tab before it applied an event, to compare afterwards.
#[derive(Debug, Default)]
pub(super) struct EventWatch {}

impl BrowserController {
    /// The one place a tab becomes the active tab: every `select_*`, the replacement after a
    /// close, a foreground tab being opened. Clears the tab's `unread` and records when it was
    /// shown.
    pub(super) fn activate(&mut self, tab_id: TabId) {
        self.active_tab_id = tab_id;
        self.activation_clock += 1;
        let shown = self.activation_clock;
        if let Some(tab) = self.tab_mut(tab_id) {
            tab.set_last_shown(shown);
            tab.set_unread(false);
        }
    }

    /// Called by `handle_event` before the event is applied to the tab.
    pub(super) fn watch_event(&self, tab_id: TabId, event: &BrowserEvent) -> EventWatch {
        let _ = (tab_id, event);
        EventWatch::default()
    }

    /// Called by `handle_event` after the event was applied: sets or clears `unread` and
    /// `crashed`.
    pub(super) fn settle_event(&mut self, tab_id: TabId, watch: EventWatch) {
        let _ = (tab_id, watch);
    }

    /// What the interface shows for a tab; `None` if there is no such tab.
    pub fn tab_status(&self, tab: TabId) -> Option<TabStatus> {
        let _ = tab;
        todo!("groups-status")
    }

    /// The flags of a group's tabs together; `None` if there is no such group.
    pub fn group_summary(&self, group: GroupId) -> Option<GroupSummary> {
        let _ = group;
        todo!("groups-status")
    }

    /// The group's tabs, most recently shown first (ties in strip order). Empty for an unknown
    /// group.
    pub fn group_tabs_by_recency(&self, group: GroupId) -> Vec<TabId> {
        let _ = group;
        todo!("groups-status")
    }

    /// Shows the group's most recently shown tab. False for an unknown group.
    pub fn select_group(&mut self, group: GroupId) -> bool {
        let _ = group;
        todo!("groups-status")
    }

    /// Gives the groups that were active since the last call, and the active tab's group, `now`
    /// as their `last_active`. The caller (the desktop's sync tick) supplies the time.
    pub fn stamp_group_activity(&mut self, now: SystemTime) {
        let _ = now;
        todo!("groups-status")
    }

    /// `Err(KeyTaken)` if another group has `key`.
    pub fn set_group_key(
        &mut self,
        group: GroupId,
        key: Option<GroupKey>,
    ) -> Result<(), GroupError> {
        let _ = (group, key);
        todo!("groups-status")
    }

    /// The settings window's "Swap": the group that had `key` gets `group`'s old key (or none).
    pub fn swap_group_key(&mut self, group: GroupId, key: GroupKey) -> Result<(), GroupError> {
        let _ = (group, key);
        todo!("groups-status")
    }

    /// The letters new groups take, in order; empty turns automatic keys off.
    pub fn set_group_key_sequence(&mut self, keys: Vec<GroupKey>) {
        let _ = keys;
        todo!("groups-status")
    }
}
