// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Tab groups on the strip (groups-core lane, item `groups-strip`).
//!
//! **SKELETON.** Reading accessors that are one line are done; everything else says
//! `todo!("groups-strip")` until that item fills it in, together with the group-aware changes to
//! `insert_tab`, `move_tab`, `move_tab_by`, `set_tab_pinned`, `adopt_tab` and `forget_closed_tab`
//! in `controller.rs`. Signatures are the lane's frozen contract (`lane-groups-core-brief.md` §5 and
//! §6: the strip invariants I1-I8 and rule R1). Status flags and keys are in `group_status.rs`.

use std::time::SystemTime;

use crate::{ClosedGroup, GroupColour, GroupError, GroupId, RestoredGroup, TabGroup, TabId};

use super::BrowserController;

impl BrowserController {
    /// The window's groups, in strip order.
    pub fn groups(&self) -> &[TabGroup] {
        self.groups.as_slice()
    }

    pub fn group(&self, id: GroupId) -> Option<&TabGroup> {
        self.groups.get(id)
    }

    /// The group the tab is in; `None` for a loose or pinned tab, or an unknown tab.
    pub fn group_of(&self, tab: TabId) -> Option<GroupId> {
        self.tabs
            .iter()
            .find(|candidate| candidate.id() == tab)?
            .group()
    }

    /// The group's tabs in strip order (they are contiguous); empty for an unknown group.
    pub fn group_tabs(&self, id: GroupId) -> Vec<TabId> {
        self.tabs
            .iter()
            .filter(|tab| tab.group() == Some(id))
            .map(crate::Tab::id)
            .collect()
    }

    /// The unpinned tabs that are in no group, in strip order.
    pub fn loose_tabs(&self) -> Vec<TabId> {
        self.tabs
            .iter()
            .filter(|tab| !tab.is_pinned() && tab.group().is_none())
            .map(crate::Tab::id)
            .collect()
    }

    /// The id the next group will get; the session file saves it.
    pub fn next_group_id(&self) -> u64 {
        self.groups.next_id()
    }

    /// Describes the first broken strip invariant (I1-I8 in the brief), or `None`. For tests and
    /// debugging; no operation of the controller calls it.
    pub fn group_invariant_violation(&self) -> Option<String> {
        todo!("groups-strip")
    }

    /// Makes a group of `tabs` (de-duplicated, in strip order), at the place of the first one.
    pub fn create_group(
        &mut self,
        tabs: &[TabId],
        name: Option<&str>,
        colour: Option<GroupColour>,
        now: SystemTime,
    ) -> Result<GroupId, GroupError> {
        let _ = (tabs, name, colour, now);
        todo!("groups-strip")
    }

    pub fn rename_group(&mut self, group: GroupId, name: &str) -> Result<(), GroupError> {
        let _ = (group, name);
        todo!("groups-strip")
    }

    pub fn recolour_group(
        &mut self,
        group: GroupId,
        colour: GroupColour,
    ) -> Result<(), GroupError> {
        let _ = (group, colour);
        todo!("groups-strip")
    }

    /// With `Some(group)`, `index` is the tab's final place inside the group; with `None` it is its
    /// final place in the strip (after the pinned tabs, snapped out of any group's run).
    pub fn move_tab_to_group(
        &mut self,
        tab: TabId,
        group: Option<GroupId>,
        index: usize,
    ) -> Result<(), GroupError> {
        let _ = (tab, group, index);
        todo!("groups-strip")
    }

    /// Takes the tab out of its group and puts it just after the group's run.
    pub fn ungroup_tab(&mut self, tab: TabId) -> Result<(), GroupError> {
        let _ = tab;
        todo!("groups-strip")
    }

    /// Moves the group's run so the group is at `index` in `groups()`. `Ok(false)`: no change.
    pub fn reorder_group(&mut self, group: GroupId, index: usize) -> Result<bool, GroupError> {
        let _ = (group, index);
        todo!("groups-strip")
    }

    /// Closes all the group's tabs; see the brief's D11 for which tab is shown afterwards.
    pub fn close_group(&mut self, group: GroupId) -> Result<ClosedGroup, GroupError> {
        let _ = group;
        todo!("groups-strip")
    }

    /// Makes a group of `tabs` from a saved session. Pinned and unknown tabs are skipped;
    /// `NoTabs` if none is left.
    pub fn restore_group(
        &mut self,
        spec: RestoredGroup,
        tabs: &[TabId],
        now: SystemTime,
    ) -> Result<GroupId, GroupError> {
        let _ = (spec, tabs, now);
        todo!("groups-strip")
    }
}
