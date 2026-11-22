// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The shrunk dock's contents: one sliver per tab fitted into the capsule, the count and the
//! accessible label (SPEC §4.4, brief D14 and D15). Pure. The public signatures and the constants
//! are the pinned contract (`DockGeometry` in `dock-models.slint` has the same numbers).

use std::collections::HashMap;

use slint::SharedString;

use crate::{DockSliver, GroupRow, TabRow};

/// Left edge of the sliver area in the capsule.
pub const SLIVER_LEFT: f32 = 14.0;
/// Right edge of the sliver area.
pub const SLIVER_RIGHT: f32 = 176.0;
/// Left edge of the count text.
pub const COUNT_X: f32 = 182.0;
/// Width of the count text box.
pub const COUNT_WIDTH: f32 = 26.0;
/// Left edge of the chevron.
pub const CHEVRON_X: f32 = 212.0;
/// A sliver's width at rest.
pub const SLIVER_WIDTH: f32 = 6.0;
/// The gap between two slivers at rest.
pub const SLIVER_GAP: f32 = 1.0;
/// The narrowest a sliver gets (D14).
pub const MIN_SLIVER_WIDTH: f32 = 2.0;

/// What `DockCapsule` is given.
#[derive(Clone, Debug, Default)]
pub struct CapsuleFrame {
    /// Slivers in group order and tab order; `x` and `width` in the capsule.
    pub slivers: Vec<DockSliver>,
    /// The true number of tabs ("22").
    pub tab_count: usize,
    /// The number of groups.
    pub group_count: usize,
    /// The text beside the slivers: the tab count.
    pub count_text: SharedString,
    /// "Show the group dock, 22 tabs in 6 groups".
    pub label: SharedString,
}

/// Fits the slivers of every group's tabs (in the order given) into the capsule.
///
/// The slivers run in the order of `groups`, and inside a group by `index_in_group` (stable); a tab
/// whose `group` is in no row of `groups` is ignored. At rest a sliver is [`SLIVER_WIDTH`] wide with
/// [`SLIVER_GAP`] between. When that does not fit, the gap goes and the width shrinks to
/// [`MIN_SLIVER_WIDTH`]; beyond that one sliver stands for several tabs of one group (its `tab` is
/// the first of them, it is `active` when any of them is, and it has the shade of the first), with
/// at least one sliver per group. Only with more groups than the area holds at the minimum width do
/// the slivers get narrower than [`MIN_SLIVER_WIDTH`]. They never overlap and never leave
/// [`SLIVER_LEFT`]..[`SLIVER_RIGHT`].
pub fn capsule_frame(groups: &[GroupRow], tabs: &[TabRow]) -> CapsuleFrame {
    let runs = group_runs(groups, tabs);
    let tab_count: usize = runs.iter().map(Vec::len).sum();
    let group_count = runs.len();
    let counts: Vec<usize> = runs.iter().map(Vec::len).collect();
    let total_slivers = sliver_count(&counts);
    let shares = apportion(&counts, total_slivers);

    let area = SLIVER_RIGHT - SLIVER_LEFT;
    let (pitch, width) = sliver_pitch(total_slivers, area);
    let mut slivers = Vec::with_capacity(total_slivers);
    for (run, share) in runs.iter().zip(&shares) {
        for part in 0..*share {
            let from = part * run.len() / share;
            let to = (part + 1) * run.len() / share;
            let members = &run[from..to];
            let first = members[0];
            let x = SLIVER_LEFT + pitch * slivers.len() as f32;
            slivers.push(DockSliver {
                tab: first.id,
                shade: first.shade,
                active: members.iter().any(|tab| tab.active),
                x,
                width: width.min((SLIVER_RIGHT - x).max(0.0)),
            });
        }
    }

    CapsuleFrame {
        slivers,
        tab_count,
        group_count,
        count_text: SharedString::from(tab_count.to_string()),
        label: SharedString::from(capsule_label(tab_count, group_count)),
    }
}

/// "Show the group dock, 22 tabs in 6 groups" ("1 tab in 1 group" in the singular).
pub fn capsule_label(tabs: usize, groups: usize) -> String {
    let tab_noun = if tabs == 1 { "tab" } else { "tabs" };
    let group_noun = if groups == 1 { "group" } else { "groups" };
    format!("Show the group dock, {tabs} {tab_noun} in {groups} {group_noun}")
}

/// The tabs of every group that has one, in the order of `groups`, each run in tab order.
fn group_runs<'a>(groups: &[GroupRow], tabs: &'a [TabRow]) -> Vec<Vec<&'a TabRow>> {
    let mut position: HashMap<i32, usize> = HashMap::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        position.entry(group.id).or_insert(index);
    }
    let mut buckets: Vec<Vec<&TabRow>> = vec![Vec::new(); groups.len()];
    for tab in tabs {
        if let Some(&index) = position.get(&tab.group) {
            buckets[index].push(tab);
        }
    }
    buckets.retain(|bucket| !bucket.is_empty());
    for bucket in &mut buckets {
        bucket.sort_by_key(|tab| tab.index_in_group);
    }
    buckets
}

/// How many slivers to draw for groups holding `counts` tabs: one per tab while they are at least
/// [`MIN_SLIVER_WIDTH`] wide, never fewer than one per group.
fn sliver_count(counts: &[usize]) -> usize {
    let tabs: usize = counts.iter().sum();
    let widest = ((SLIVER_RIGHT - SLIVER_LEFT) / MIN_SLIVER_WIDTH).floor() as usize;
    tabs.min(widest).max(counts.len())
}

/// The distance between the left edges of two slivers and the sliver width for `count` slivers.
fn sliver_pitch(count: usize, area: f32) -> (f32, f32) {
    if count == 0 {
        return (SLIVER_WIDTH + SLIVER_GAP, SLIVER_WIDTH);
    }
    let rest = SLIVER_WIDTH + SLIVER_GAP;
    if count as f32 * rest - SLIVER_GAP <= area {
        return (rest, SLIVER_WIDTH);
    }
    let width = (area / count as f32).min(SLIVER_WIDTH);
    (width, width)
}

/// Splits `total` slivers over the groups: one each, the rest in proportion to how many more tabs
/// a group has than one (largest remainder), so a group never gets more slivers than tabs.
fn apportion(counts: &[usize], total: usize) -> Vec<usize> {
    let spare_tabs: usize = counts.iter().map(|count| count - 1).sum();
    let spare_slivers = total.saturating_sub(counts.len()).min(spare_tabs);
    let mut shares: Vec<usize> = Vec::with_capacity(counts.len());
    let mut remainders: Vec<(usize, usize)> = Vec::with_capacity(counts.len());
    for (index, count) in counts.iter().enumerate() {
        let weight = (count - 1) * spare_slivers;
        let whole = weight.checked_div(spare_tabs).unwrap_or(0);
        let rest = weight.checked_rem(spare_tabs).unwrap_or(0);
        shares.push(1 + whole);
        remainders.push((rest, index));
    }
    let given: usize = shares.iter().sum();
    let mut left = (counts.len() + spare_slivers).saturating_sub(given);
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (rest, index) in remainders {
        if left == 0 || rest == 0 {
            break;
        }
        shares[index] += 1;
        left -= 1;
    }
    shares
}
