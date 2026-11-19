// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Tab groups on the strip (groups-core lane, item `groups-strip`).
//!
//! A group is a contiguous run of tabs (brief D2): the strip stays one `Vec<Tab>`, pinned tabs
//! first (never in a group), then loose tabs and group runs in any mix. The order of the groups
//! in the [`GroupBook`] is the order of their runs in the strip, so there is one source of truth;
//! `sync_group_order` restores it after anything that could change it.
//!
//! This file holds the group operations (`create_group`, `move_tab_to_group`, `ungroup_tab`,
//! `reorder_group`, `close_group`, `restore_group`, `rename_group`, `recolour_group`), the
//! invariant check (`group_invariant_violation`, brief §6 I1-I8) and the small helpers the
//! older strip functions in `controller.rs` call from their group branches:
//!
//! - `group_run` and `run_around`: where a group's tabs are;
//! - `snap_out_of_run`: rule R1 (a loose tab never lands strictly inside a run);
//! - `place_among_groups`: where `insert_tab` puts a new tab;
//! - `past_adjacent_run`: how `move_tab_by` steps a loose tab over a whole run;
//! - `settle_groups`: deletes the groups a removal left empty (their keys are free again) and puts
//!   the book back in strip order.
//!
//! Every operation is atomic: it checks everything it can refuse before it changes anything, and
//! `create_group` / `restore_group`, whose book changes can still fail, restore a saved copy of
//! the book on failure. Selection never changes here except in `close_group` (D11), which
//! goes through `activate` like every other path (`group_status.rs`).

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use crate::{
    ClosedGroup, GroupBook, GroupColour, GroupError, GroupId, MAX_GROUPS, RestoredGroup, Tab,
    TabActivation, TabGroup, TabId,
};

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

    /// Makes sure no group created from now on gets an id below `floor` (clamped to the largest
    /// id Slint can carry). The session restore calls this with the saved counter, so an id that
    /// a closed tab remembers from an earlier run never names a different group.
    pub fn raise_next_group_id(&mut self, floor: u64) {
        self.groups.raise_next_id(floor);
    }

    /// Describes the first broken strip invariant (I1-I8 in the brief), or `None`. For tests and
    /// debugging; no operation of the controller calls it.
    pub fn group_invariant_violation(&self) -> Option<String> {
        // I1: every tab's group is in the book.
        for tab in &self.tabs {
            if let Some(group) = tab.group()
                && self.groups.get(group).is_none()
            {
                return Some(format!(
                    "I1: tab {} is in group {}, which is not in the book",
                    tab.id().value(),
                    group.value()
                ));
            }
        }
        // I2: every group has a tab.
        for group in self.groups.as_slice() {
            if !self.tabs.iter().any(|tab| tab.group() == Some(group.id)) {
                return Some(format!("I2: group {} has no tab", group.id.value()));
            }
        }
        // I3: the tabs of a group are contiguous.
        for group in self.groups.as_slice() {
            let places = self
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, tab)| tab.group() == Some(group.id))
                .map(|(place, _)| place)
                .collect::<Vec<_>>();
            if let (Some(first), Some(last)) = (places.first(), places.last())
                && last - first + 1 != places.len()
            {
                return Some(format!(
                    "I3: group {} is not contiguous: its tabs are at strip places {places:?}",
                    group.id.value()
                ));
            }
        }
        // I4: pinned tabs have no group and come first.
        let leading_pinned = self.tabs.iter().take_while(|tab| tab.is_pinned()).count();
        for (place, tab) in self.tabs.iter().enumerate() {
            if tab.is_pinned() && place >= leading_pinned {
                return Some(format!(
                    "I4: pinned tab {} follows an unpinned tab",
                    tab.id().value()
                ));
            }
            if tab.is_pinned()
                && let Some(group) = tab.group()
            {
                return Some(format!(
                    "I4: pinned tab {} is in group {}",
                    tab.id().value(),
                    group.value()
                ));
            }
        }
        // I5: ids and keys are unique.
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        for group in self.groups.as_slice() {
            if !ids.insert(group.id) {
                return Some(format!("I5: group id {} is used twice", group.id.value()));
            }
            if let Some(key) = group.key
                && !keys.insert(key)
            {
                return Some(format!(
                    "I5: key {} is held by two groups (one is group {})",
                    key.letter(),
                    group.id.value()
                ));
            }
        }
        // I6: the limit.
        if self.groups.len() > MAX_GROUPS {
            return Some(format!(
                "I6: {} groups, at most {MAX_GROUPS} are allowed",
                self.groups.len()
            ));
        }
        // I7: the book is in strip order.
        let strip_order = self.strip_group_order();
        let book_order = self
            .groups
            .as_slice()
            .iter()
            .map(|group| group.id)
            .collect::<Vec<_>>();
        if strip_order != book_order {
            return Some(format!(
                "I7: the book lists the groups as {:?} but the strip has them as {:?}",
                book_order.iter().map(|id| id.value()).collect::<Vec<_>>(),
                strip_order.iter().map(|id| id.value()).collect::<Vec<_>>()
            ));
        }
        // I8: tab ids are unique, and the active tab is one of them.
        let mut tab_ids = HashSet::new();
        for tab in &self.tabs {
            if !tab_ids.insert(tab.id()) {
                return Some(format!("I8: tab id {} is used twice", tab.id().value()));
            }
        }
        if !tab_ids.contains(&self.active_tab_id) {
            return Some(format!(
                "I8: the active tab {} is not in the strip",
                self.active_tab_id.value()
            ));
        }
        None
    }

    /// Makes a group of `tabs` (de-duplicated, in strip order), at the place of the first one.
    ///
    /// `Err(TabNotFound)` for an unknown tab, then `Err(TabPinned)`, then `Err(NoTabs)` for an
    /// empty list; `Err(TooManyGroups)` only if the window would end with more than 64 groups
    /// (groups this call empties do not count, D12). The members leave their old groups; a group
    /// left empty is deleted and its key is free for the new one. The selection does not change.
    pub fn create_group(
        &mut self,
        tabs: &[TabId],
        name: Option<&str>,
        colour: Option<GroupColour>,
        now: SystemTime,
    ) -> Result<GroupId, GroupError> {
        self.gather_into_new_group(tabs, false, |book| book.create(name, colour, now))
    }

    pub fn rename_group(&mut self, group: GroupId, name: &str) -> Result<(), GroupError> {
        self.groups.rename(group, name)
    }

    pub fn recolour_group(
        &mut self,
        group: GroupId,
        colour: GroupColour,
    ) -> Result<(), GroupError> {
        self.groups.recolour(group, colour)
    }

    /// With `Some(group)`, `index` is the tab's final place inside the group; with `None` it is its
    /// final place in the strip (after the pinned tabs, snapped out of any group's run).
    ///
    /// `Err(TabNotFound)`, then `Err(TabPinned)`, then `Err(NotFound)` for an unknown group. A
    /// group left empty is deleted. The selection does not change.
    pub fn move_tab_to_group(
        &mut self,
        tab: TabId,
        group: Option<GroupId>,
        index: usize,
    ) -> Result<(), GroupError> {
        let from = self
            .tabs
            .iter()
            .position(|candidate| candidate.id() == tab)
            .ok_or(GroupError::TabNotFound)?;
        if self.tabs[from].is_pinned() {
            return Err(GroupError::TabPinned);
        }
        if let Some(target) = group
            && self.groups.get(target).is_none()
        {
            return Err(GroupError::NotFound);
        }

        let mut moving = self.tabs.remove(from);
        let at = match group {
            // The run without this tab (it may have no other tab: then the tab stays).
            Some(target) => match self.group_run(target) {
                Some((start, end)) => start + index.min(end + 1 - start),
                None => from,
            },
            None => {
                let pinned_count = self.pinned_tab_count();
                self.snap_out_of_run(index.clamp(pinned_count, self.tabs.len()))
            }
        };
        moving.set_group(group);
        self.tabs.insert(at, moving);
        self.settle_groups();
        Ok(())
    }

    /// Takes the tab out of its group and puts it just after the group's run. A loose tab is left
    /// as it is; an unknown tab is `Err(TabNotFound)`.
    pub fn ungroup_tab(&mut self, tab: TabId) -> Result<(), GroupError> {
        let from = self
            .tabs
            .iter()
            .position(|candidate| candidate.id() == tab)
            .ok_or(GroupError::TabNotFound)?;
        let Some(old) = self.tabs[from].group() else {
            return Ok(());
        };
        // The run's last place, which is where the tab ends up once it has left the run.
        let end = self.group_run(old).map_or(from, |(_, end)| end);
        let mut moving = self.tabs.remove(from);
        moving.set_group(None);
        self.tabs.insert(end, moving);
        self.settle_groups();
        Ok(())
    }

    /// Moves the group's run so the group is at `index` in `groups()`. `Ok(false)`: no change.
    ///
    /// Moving earlier puts the run directly before the run of the group now at `index`; moving
    /// later, directly after it. Loose tabs between the runs keep their order.
    pub fn reorder_group(&mut self, group: GroupId, index: usize) -> Result<bool, GroupError> {
        let order = self.strip_group_order();
        let position = order
            .iter()
            .position(|candidate| *candidate == group)
            .ok_or(GroupError::NotFound)?;
        let target = index.min(order.len() - 1);
        if target == position {
            return Ok(false);
        }
        let anchor = order[target];
        let (start, end) = self
            .group_run(group)
            .expect("a group in the strip order has a run");
        let block = self.tabs.drain(start..=end).collect::<Vec<_>>();
        let (anchor_start, anchor_end) = self
            .group_run(anchor)
            .expect("a group in the strip order has a run");
        let at = if target < position {
            anchor_start
        } else {
            anchor_end + 1
        };
        self.tabs.splice(at..at, block);
        self.sync_group_order();
        Ok(true)
    }

    /// Closes all the group's tabs; see the brief's D11 for which tab is shown afterwards: if the
    /// active tab is among them, the tab just before the run, else the one just after it, else
    /// (the group held every tab) a new blank tab, opened and selected first and returned as
    /// `opened_blank`. Otherwise the selection stays. The group is deleted and its key freed.
    pub fn close_group(&mut self, group: GroupId) -> Result<ClosedGroup, GroupError> {
        let closed = self
            .groups
            .get(group)
            .cloned()
            .ok_or(GroupError::NotFound)?;
        let tabs = self.group_tabs(group);
        let Some((start, end)) = self.group_run(group) else {
            // A group with no tab cannot exist (I2); drop it if it does.
            self.groups.remove(group);
            return Ok(ClosedGroup {
                group: closed,
                tabs,
                opened_blank: None,
            });
        };

        let mut opened_blank = None;
        let mut replacement = None;
        if tabs.contains(&self.active_tab_id) {
            if tabs.len() == self.tabs.len() {
                opened_blank = Some(self.open_blank_tab(TabActivation::Foreground));
            } else if start > 0 {
                replacement = Some(self.tabs[start - 1].id());
            } else {
                replacement = Some(self.tabs[end + 1].id());
            }
        }

        self.tabs.retain(|tab| tab.group() != Some(group));
        for tab in &tabs {
            self.forget_closed_tab(*tab);
        }
        self.groups.remove(group);
        if let Some(replacement) = replacement {
            self.activate(replacement);
        }
        Ok(ClosedGroup {
            group: closed,
            tabs,
            opened_blank,
        })
    }

    /// Makes a group of `tabs` from a saved session. Pinned and unknown tabs are skipped;
    /// `NoTabs` if none is left. Otherwise like [`create_group`](Self::create_group), with the
    /// book's `restore` rules for the id hint and the key.
    pub fn restore_group(
        &mut self,
        spec: RestoredGroup,
        tabs: &[TabId],
        now: SystemTime,
    ) -> Result<GroupId, GroupError> {
        self.gather_into_new_group(tabs, true, |book| book.restore(spec, now))
    }
}

// The strip-side helpers. `pub(super)` ones are called from the group branches in `controller.rs`.
impl BrowserController {
    /// The first and last strip place of the group's tabs; `None` when it has no tab.
    pub(super) fn group_run(&self, group: GroupId) -> Option<(usize, usize)> {
        let start = self
            .tabs
            .iter()
            .position(|tab| tab.group() == Some(group))?;
        let end = self
            .tabs
            .iter()
            .rposition(|tab| tab.group() == Some(group))?;
        Some((start, end))
    }

    /// The run of the group the tab at `place` is in; `None` for a loose or pinned tab.
    pub(super) fn run_around(&self, place: usize) -> Option<(usize, usize)> {
        self.group_run(self.tabs.get(place)?.group()?)
    }

    /// Rule R1: a loose tab inserted at strip `index` (the place it will have, in a strip that
    /// does not hold it yet) that falls strictly inside a run, with the tab before and the tab
    /// after in the same group, goes to the end of that run instead. A place at a run's edge is
    /// kept.
    pub(super) fn snap_out_of_run(&self, index: usize) -> usize {
        if self.groups.is_empty() {
            return index;
        }
        let before = index.checked_sub(1).and_then(|place| self.tabs.get(place));
        let (Some(before), Some(after)) = (before, self.tabs.get(index)) else {
            return index;
        };
        let Some(group) = before.group().filter(|group| after.group() == Some(*group)) else {
            return index;
        };
        let mut end = index;
        while self
            .tabs
            .get(end)
            .is_some_and(|tab| tab.group() == Some(group))
        {
            end += 1;
        }
        end
    }

    /// Where `insert_tab` puts a new unpinned `tab` that was asked for at strip `index`: inside
    /// its own group's run if it has a group (the index clamped into the run, or just after it),
    /// else snapped out of any run (R1).
    pub(super) fn place_among_groups(&self, index: usize, tab: &mut Tab) -> usize {
        match tab.group() {
            Some(group) => match self.group_run(group) {
                Some((start, end)) => index.clamp(start, end + 1),
                None => {
                    // Its group has no tab here (it cannot happen through this API): keep I1.
                    tab.set_group(None);
                    self.snap_out_of_run(index)
                }
            },
            None => self.snap_out_of_run(index),
        }
    }

    /// For `move_tab_by`: a loose tab at `from` that would step to `to`, which is in a run, steps
    /// over the whole run instead: to the run's last place going right, its first going left.
    pub(super) fn past_adjacent_run(&self, from: usize, to: usize) -> usize {
        if self.groups.is_empty() || self.tabs[from].group().is_some() {
            return to;
        }
        match self.run_around(to) {
            Some((_, end)) if to > from => end,
            Some((start, _)) => start,
            None => to,
        }
    }

    /// The groups in the order of their first tab in the strip.
    fn strip_group_order(&self) -> Vec<GroupId> {
        let mut order = Vec::new();
        for group in self.tabs.iter().filter_map(Tab::group) {
            if !order.contains(&group) {
                order.push(group);
            }
        }
        order
    }

    /// Deletes every group that has no tab (its key is free again).
    pub(super) fn drop_empty_groups(&mut self) {
        if self.groups.is_empty() {
            return;
        }
        let present = self
            .tabs
            .iter()
            .filter_map(Tab::group)
            .collect::<HashSet<_>>();
        let empty = self
            .groups
            .as_slice()
            .iter()
            .map(|group| group.id)
            .filter(|id| !present.contains(id))
            .collect::<Vec<_>>();
        for id in empty {
            self.groups.remove(id);
        }
    }

    /// Puts the book in the order of the runs in the strip.
    pub(super) fn sync_group_order(&mut self) {
        if self.groups.len() < 2 {
            return;
        }
        let order = self.strip_group_order();
        let in_order = self
            .groups
            .as_slice()
            .iter()
            .map(|group| group.id)
            .eq(order.iter().copied());
        if !in_order {
            self.groups.reorder_to_match(&order);
        }
    }

    /// What every way a tab leaves the strip or its group does at the end: deletes the groups left
    /// empty and keeps the book in strip order.
    pub(super) fn settle_groups(&mut self) {
        self.drop_empty_groups();
        self.sync_group_order();
    }

    /// The shared body of `create_group` and `restore_group`. `lenient` skips pinned and unknown
    /// tabs instead of refusing them. `make` adds the group to the book.
    fn gather_into_new_group(
        &mut self,
        wanted: &[TabId],
        lenient: bool,
        make: impl FnOnce(&mut GroupBook) -> Result<GroupId, GroupError>,
    ) -> Result<GroupId, GroupError> {
        let wanted = wanted.iter().copied().collect::<HashSet<_>>();
        if !lenient {
            let known = self.tabs.iter().map(Tab::id).collect::<HashSet<_>>();
            if wanted.iter().any(|tab| !known.contains(tab)) {
                return Err(GroupError::TabNotFound);
            }
            if self
                .tabs
                .iter()
                .any(|tab| tab.is_pinned() && wanted.contains(&tab.id()))
            {
                return Err(GroupError::TabPinned);
            }
        }
        // The members' strip places, ascending (so in strip order).
        let members = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| !tab.is_pinned() && wanted.contains(&tab.id()))
            .map(|(place, _)| place)
            .collect::<Vec<_>>();
        let Some(first) = members.first().copied() else {
            return Err(GroupError::NoTabs);
        };

        // The groups this call empties: all their tabs are members.
        let mut counts: HashMap<GroupId, (usize, usize)> = HashMap::new();
        for (place, tab) in self.tabs.iter().enumerate() {
            if let Some(group) = tab.group() {
                let (all, taken) = counts.entry(group).or_default();
                *all += 1;
                if members.binary_search(&place).is_ok() {
                    *taken += 1;
                }
            }
        }
        let mut emptied = counts
            .into_iter()
            .filter(|(_, (all, taken))| all == taken)
            .map(|(group, _)| group)
            .collect::<Vec<_>>();
        emptied.sort();

        // The book changes first, and is put back if it refuses: the strip is not touched until
        // nothing can fail any more.
        let saved = self.groups.clone();
        for group in emptied {
            self.groups.remove(group);
        }
        let id = match make(&mut self.groups) {
            Ok(id) => id,
            Err(error) => {
                self.groups = saved;
                return Err(error);
            }
        };

        let mut taken = Vec::with_capacity(members.len());
        for place in members.iter().rev() {
            taken.push(self.tabs.remove(*place));
        }
        taken.reverse();
        for tab in &mut taken {
            tab.set_group(Some(id));
        }
        // Every tab before `first` is not a member, so `first` is the place in the strip without
        // the members; R1 keeps the run from splitting another one.
        let at = self.snap_out_of_run(first);
        self.tabs.splice(at..at, taken);
        self.sync_group_order();
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    //! The invariant check is the net under every operation, so it is tested on strips that
    //! really are broken (the public API cannot build one).

    use super::*;

    fn controller_with_tabs(count: u64) -> BrowserController {
        let mut controller = BrowserController::default();
        for _ in 1..count {
            controller
                .open_tab_with(None, TabActivation::Background)
                .expect("a blank tab opens");
        }
        controller
    }

    fn tab_ids(controller: &BrowserController) -> Vec<TabId> {
        controller.tabs.iter().map(Tab::id).collect()
    }

    fn group_at(controller: &mut BrowserController, places: &[usize]) -> GroupId {
        let ids = tab_ids(controller);
        let tabs = places.iter().map(|place| ids[*place]).collect::<Vec<_>>();
        controller
            .create_group(&tabs, None, None, SystemTime::UNIX_EPOCH)
            .expect("the group is made")
    }

    #[test]
    fn a_sound_strip_has_no_violation() {
        let mut controller = controller_with_tabs(5);
        group_at(&mut controller, &[1, 2]);
        group_at(&mut controller, &[4]);
        assert_eq!(controller.group_invariant_violation(), None);
    }

    #[test]
    fn a_tab_in_a_group_the_book_does_not_have_breaks_i1() {
        let mut controller = controller_with_tabs(3);
        controller.tabs[1].set_group(Some(GroupId::new(40)));
        let violation = controller.group_invariant_violation().expect("broken");
        assert!(violation.starts_with("I1"), "{violation}");
    }

    #[test]
    fn a_group_with_no_tab_breaks_i2() {
        let mut controller = controller_with_tabs(3);
        let group = group_at(&mut controller, &[1]);
        controller.tabs[1].set_group(None);
        let violation = controller.group_invariant_violation().expect("broken");
        assert!(violation.starts_with("I2"), "{violation}");
        assert!(
            violation.contains(&group.value().to_string()),
            "{violation}"
        );
    }

    #[test]
    fn a_group_split_in_two_runs_breaks_i3() {
        let mut controller = controller_with_tabs(4);
        let group = group_at(&mut controller, &[1, 2]);
        controller.tabs[2].set_group(None);
        controller.tabs[3].set_group(Some(group));
        let violation = controller.group_invariant_violation().expect("broken");
        assert!(violation.starts_with("I3"), "{violation}");
    }

    #[test]
    fn a_grouped_pinned_tab_breaks_i4() {
        let mut controller = controller_with_tabs(3);
        let group = group_at(&mut controller, &[0, 1]);
        controller.tabs[0].set_pinned(true);
        controller.tabs[0].set_group(Some(group));
        let violation = controller.group_invariant_violation().expect("broken");
        assert!(violation.starts_with("I4"), "{violation}");
    }

    #[test]
    fn a_pinned_tab_after_an_unpinned_one_breaks_i4() {
        let mut controller = controller_with_tabs(3);
        controller.tabs[2].set_pinned(true);
        let violation = controller.group_invariant_violation().expect("broken");
        assert!(violation.starts_with("I4"), "{violation}");
    }

    #[test]
    fn a_book_out_of_strip_order_breaks_i7() {
        let mut controller = controller_with_tabs(4);
        let first = group_at(&mut controller, &[0]);
        let second = group_at(&mut controller, &[2]);
        controller.groups.reorder_to_match(&[second, first]);
        let violation = controller.group_invariant_violation().expect("broken");
        assert!(violation.starts_with("I7"), "{violation}");
    }

    #[test]
    fn an_active_tab_that_is_not_in_the_strip_breaks_i8() {
        let mut controller = controller_with_tabs(2);
        controller.active_tab_id = TabId::new(99);
        let violation = controller.group_invariant_violation().expect("broken");
        assert!(violation.starts_with("I8"), "{violation}");
    }

    #[test]
    fn a_repeated_tab_id_breaks_i8() {
        let mut controller = controller_with_tabs(2);
        let duplicate = Tab::new(controller.tabs[0].id(), crate::BrowserState::default());
        controller.tabs.push(duplicate);
        let violation = controller.group_invariant_violation().expect("broken");
        assert!(violation.starts_with("I8"), "{violation}");
    }

    #[test]
    fn the_run_helpers_find_runs_and_snap_out_of_them() {
        let mut controller = controller_with_tabs(6);
        let group = group_at(&mut controller, &[1, 2, 3]);
        assert_eq!(controller.group_run(group), Some((1, 3)));
        assert_eq!(controller.run_around(2), Some((1, 3)));
        assert_eq!(controller.run_around(0), None);
        assert_eq!(controller.run_around(9), None);
        // Strictly inside: 2 and 3. At the edges (0, 1, 4) and beyond: untouched.
        let snapped = (0..=7)
            .map(|index| controller.snap_out_of_run(index))
            .collect::<Vec<_>>();
        assert_eq!(snapped, [0, 1, 4, 4, 4, 5, 6, 7]);
    }
}
