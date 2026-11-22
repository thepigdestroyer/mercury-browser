// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The ring's key navigation as a pure state machine (SPEC §5 "Ring popover" row, §5.1, brief D3,
//! D5, D10): which tab or bead has the focus and what a key asks the host to do. No window; the
//! controller maps Slint's `KeyEvent` to a [`NavKey`] and an [`Intent`] to the host.
//!
//! Order of the focus: the tab segments in ring order, then the pinned beads left to right, then
//! round to the first segment (the beads sit between the last and the first segment on the
//! circle). ← and → walk it with wrapping; ↑ and ↓ go to the first tab of the previous or next
//! group (wrapping); ⌥← and ⌥→ move the focused tab one place within its group; ⌥↑ and ⌥↓ move it to
//! the end of the previous or next group; ⌘⌫ closes it; ⏎ opens it; M mutes it.
//!
//! STUB in the scaffold commit (types and signatures are the contract; item 2 `nav-beads` writes
//! the bodies).

/// Where the focus is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    None,
    /// A segment (`TabRow.id`).
    Tab(i32),
    /// A pinned bead (`TabRow.id`).
    Bead(i32),
}

/// One segment in ring order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabSlot {
    pub id: i32,
    /// `GroupRow.id`.
    pub group: i32,
}

/// One group in ring order with the key that selects it while ⌥ is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupSlot {
    pub id: i32,
    /// Upper-case letter, `None` for a group without a key (loose).
    pub key: Option<char>,
}

/// The ring's focus order, built from the shown tabs and the pinned beads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusOrder {
    /// Segments in ring order (groups in order, tabs in index order).
    pub tabs: Vec<TabSlot>,
    pub groups: Vec<GroupSlot>,
    /// Pinned tab ids, left to right.
    pub beads: Vec<i32>,
}

impl FocusOrder {
    /// `tabs` and `groups` as `RingBandsData` has them (`data.tabs`, `data.groups`) and `beads` the
    /// pinned tab ids in their order. `GroupRow.key_label` gives the key.
    pub fn new(tabs: &[(i32, i32)], groups: &[(i32, Option<char>)], beads: &[i32]) -> Self {
        Self {
            tabs: tabs
                .iter()
                .map(|&(id, group)| TabSlot { id, group })
                .collect(),
            groups: groups
                .iter()
                .map(|&(id, key)| GroupSlot { id, key })
                .collect(),
            beads: beads.to_vec(),
        }
    }
}

/// A key the ring understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavKey {
    Left,
    Right,
    Up,
    Down,
    Enter,
    /// ⌘⌫.
    CloseTab,
    /// ⌥←, ⌥→.
    MoveLeft,
    MoveRight,
    /// ⌥↑, ⌥↓.
    MoveToPrevGroup,
    MoveToNextGroup,
    /// M.
    Mute,
    /// A printable character (no ⌘ or ⌃) that no other rule took: it goes to the query.
    Type(char),
    Escape,
    /// ⌥ went down / up.
    OptionDown,
    OptionUp,
    /// The `hold_ms` timer of ⌥ ran out.
    HoldElapsed,
    /// A letter pressed while ⌥ is down: selects the group with that key.
    Letter(char),
    /// A digit 1 to 9 pressed while a group is selected.
    Digit(u8),
}

/// What the machine remembers between keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavState {
    pub cursor: Cursor,
    /// ⌥ is down (armed or held).
    pub option_down: bool,
    /// The key look is on (`HoldElapsed`, or a group letter while ⌥ is down).
    pub held: bool,
    /// The group chosen by its letter.
    pub held_group: Option<i32>,
    /// The search field has text: Esc clears it first.
    pub query_active: bool,
}

/// What a key asks the host or the controller to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Show the tab and close the popover.
    Activate(i32),
    /// The same for a bead.
    ActivateBead(i32),
    Close(i32),
    Mute(i32),
    /// Move `tab` to `group` at `index` (index in that group's shown tabs).
    Move {
        tab: i32,
        group: i32,
        index: usize,
    },
    /// Append the character to the query and focus the field.
    TypeChar(char),
    ClearQuery,
    ClosePopover,
    /// Start the `hold_ms` timer.
    BeginHold,
    /// ⌥ went up before the timer: stop it.
    CancelHold,
    /// Release of ⌥ with a group chosen: show that group's last active tab and close.
    CommitGroup(i32),
    /// Digit `n` (1 to 9) with a group chosen: show its n-th tab and close.
    ActivateDigit {
        group: i32,
        n: u8,
    },
}

/// The result of one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavOutcome {
    pub state: NavState,
    pub intent: Option<Intent>,
    /// The key was used (the popover's `FocusScope` accepts it).
    pub handled: bool,
}

/// The focus sequence of ← and →: every tab in ring order, then every bead left to right.
fn sequence(order: &FocusOrder) -> Vec<Cursor> {
    order
        .tabs
        .iter()
        .map(|slot| Cursor::Tab(slot.id))
        .chain(order.beads.iter().map(|&id| Cursor::Bead(id)))
        .collect()
}

/// The cursor one step along [`sequence`], wrapping. From no cursor (or a cursor that is not in
/// the order any more) → goes to the first tab, ← to the last tab (a bead when there is no tab).
fn walk(order: &FocusOrder, cursor: Cursor, forward: bool) -> Cursor {
    let seq = sequence(order);
    let Some(last) = seq.len().checked_sub(1) else {
        return Cursor::None;
    };
    match seq.iter().position(|&c| c == cursor) {
        Some(i) if forward => seq[if i == last { 0 } else { i + 1 }],
        Some(i) => seq[if i == 0 { last } else { i - 1 }],
        None if forward => seq[0],
        None => match order.tabs.last() {
            Some(slot) => Cursor::Tab(slot.id),
            None => seq[last],
        },
    }
}

/// The group ids in ring order. With `with_tabs_only` the groups that have no tab are left out.
/// A group that only the tabs name (not `order.groups`) comes after the listed ones.
fn group_ids(order: &FocusOrder, with_tabs_only: bool) -> Vec<i32> {
    let mut ids: Vec<i32> = Vec::new();
    if with_tabs_only {
        for slot in &order.tabs {
            if !ids.contains(&slot.group) {
                ids.push(slot.group);
            }
        }
        return ids;
    }
    ids.extend(order.groups.iter().map(|group| group.id));
    for slot in &order.tabs {
        if !ids.contains(&slot.group) {
            ids.push(slot.group);
        }
    }
    ids
}

fn tab_count(order: &FocusOrder, group: i32) -> usize {
    order.tabs.iter().filter(|slot| slot.group == group).count()
}

fn first_tab_of(order: &FocusOrder, group: i32) -> Option<i32> {
    order
        .tabs
        .iter()
        .find(|slot| slot.group == group)
        .map(|slot| slot.id)
}

/// ↑ and ↓: the first tab of the previous or next group, wrapping. From a bead or no cursor, ↓
/// enters the first group and ↑ the last.
fn jump_group(order: &FocusOrder, cursor: Cursor, forward: bool) -> Cursor {
    let groups = group_ids(order, true);
    let Some(last) = groups.len().checked_sub(1) else {
        return cursor;
    };
    let current = match cursor {
        Cursor::Tab(id) => order
            .tabs
            .iter()
            .find(|slot| slot.id == id)
            .and_then(|slot| groups.iter().position(|&g| g == slot.group)),
        _ => None,
    };
    let target = match (current, forward) {
        (Some(i), true) => groups[if i == last { 0 } else { i + 1 }],
        (Some(i), false) => groups[if i == 0 { last } else { i - 1 }],
        (None, true) => groups[0],
        (None, false) => groups[last],
    };
    first_tab_of(order, target).map_or(cursor, Cursor::Tab)
}

/// Where the cursor goes when the focused tab or bead is closed (the host removes it later).
fn cursor_after_close(order: &FocusOrder, cursor: Cursor) -> Cursor {
    match cursor {
        Cursor::None => Cursor::None,
        Cursor::Tab(id) => {
            let Some(i) = order.tabs.iter().position(|slot| slot.id == id) else {
                return Cursor::None;
            };
            match order.tabs.get(i + 1) {
                Some(next) => Cursor::Tab(next.id),
                None if i > 0 => Cursor::Tab(order.tabs[i - 1].id),
                None => Cursor::None,
            }
        }
        Cursor::Bead(id) => {
            let Some(i) = order.beads.iter().position(|&bead| bead == id) else {
                return Cursor::None;
            };
            match order.beads.get(i + 1) {
                Some(&next) => Cursor::Bead(next),
                None if i > 0 => Cursor::Bead(order.beads[i - 1]),
                None => order
                    .tabs
                    .last()
                    .map_or(Cursor::None, |slot| Cursor::Tab(slot.id)),
            }
        }
    }
}

/// ⌥← and ⌥→: the tab's group and its 0-based place in it, and the group's tab count.
fn place_in_group(order: &FocusOrder, tab: i32) -> Option<(i32, usize, usize)> {
    let group = order.tabs.iter().find(|slot| slot.id == tab)?.group;
    let place = order
        .tabs
        .iter()
        .filter(|slot| slot.group == group)
        .position(|slot| slot.id == tab)?;
    Some((group, place, tab_count(order, group)))
}

fn move_within_group(order: &FocusOrder, cursor: Cursor, right: bool) -> Option<Intent> {
    let Cursor::Tab(tab) = cursor else {
        return None;
    };
    let (group, place, count) = place_in_group(order, tab)?;
    // The index is the insertion slot with the tab still in place: one place left is slot
    // `place - 1`, one place right is slot `place + 2`.
    let index = if right {
        (place + 1 < count).then_some(place + 2)?
    } else {
        place.checked_sub(1)?
    };
    Some(Intent::Move { tab, group, index })
}

/// ⌥↑ and ⌥↓: to the end slot of the previous or next group, no wrapping.
fn move_to_group(order: &FocusOrder, cursor: Cursor, next: bool) -> Option<Intent> {
    let Cursor::Tab(tab) = cursor else {
        return None;
    };
    let (group, _, _) = place_in_group(order, tab)?;
    let groups = group_ids(order, false);
    let at = groups.iter().position(|&g| g == group)?;
    let target = if next {
        *groups.get(at + 1)?
    } else {
        *groups.get(at.checked_sub(1)?)?
    };
    Some(Intent::Move {
        tab,
        group: target,
        index: tab_count(order, target),
    })
}

/// One key applied to `state` over `order`.
///
/// `outcome.state` is the state after the key as far as the machine can tell, so a caller can
/// simply keep it: the cursor (also the one after ⌘⌫, before the host has removed the tab),
/// `option_down`, `held`, `held_group`, and `query_active` (set by [`NavKey::Type`], cleared by
/// the first Esc). The controller may overwrite any field when it knows better.
///
/// `handled` is true for every key the ring owns, even when it does nothing (for example ⌥← at the
/// first place of a group), so that the key does not leak to the page. While ⌥ is down, the plain
/// keys (arrows, ⏎, ⌘⌫, M, typing) are not handled; the ⌥ combinations, Esc, the hold timer, group
/// letters and digits are.
pub fn step(order: &FocusOrder, state: &NavState, key: NavKey) -> NavOutcome {
    let mut next = *state;
    let mut intent = None;
    let handled;
    match key {
        NavKey::Left
        | NavKey::Right
        | NavKey::Up
        | NavKey::Down
        | NavKey::Enter
        | NavKey::CloseTab
        | NavKey::Mute
        | NavKey::Type(_)
            if state.option_down =>
        {
            handled = false;
        }
        NavKey::Left | NavKey::Right => {
            next.cursor = walk(order, state.cursor, key == NavKey::Right);
            handled = true;
        }
        NavKey::Up | NavKey::Down => {
            next.cursor = jump_group(order, state.cursor, key == NavKey::Down);
            handled = true;
        }
        NavKey::Enter => {
            intent = match state.cursor {
                Cursor::Tab(id) => Some(Intent::Activate(id)),
                Cursor::Bead(id) => Some(Intent::ActivateBead(id)),
                Cursor::None => None,
            };
            handled = true;
        }
        NavKey::CloseTab => {
            intent = match state.cursor {
                Cursor::Tab(id) | Cursor::Bead(id) => Some(Intent::Close(id)),
                Cursor::None => None,
            };
            if intent.is_some() {
                next.cursor = cursor_after_close(order, state.cursor);
            }
            handled = true;
        }
        NavKey::MoveLeft | NavKey::MoveRight => {
            intent = move_within_group(order, state.cursor, key == NavKey::MoveRight);
            handled = true;
        }
        NavKey::MoveToPrevGroup | NavKey::MoveToNextGroup => {
            intent = move_to_group(order, state.cursor, key == NavKey::MoveToNextGroup);
            handled = true;
        }
        NavKey::Mute => {
            intent = match state.cursor {
                Cursor::Tab(id) | Cursor::Bead(id) => Some(Intent::Mute(id)),
                Cursor::None => None,
            };
            handled = true;
        }
        NavKey::Type(c) => {
            intent = Some(Intent::TypeChar(c));
            next.query_active = true;
            handled = true;
        }
        NavKey::Escape => {
            if state.query_active {
                intent = Some(Intent::ClearQuery);
                next.query_active = false;
            } else {
                intent = Some(Intent::ClosePopover);
            }
            handled = true;
        }
        NavKey::OptionDown => {
            if !state.option_down {
                next.option_down = true;
                intent = Some(Intent::BeginHold);
            }
            handled = true;
        }
        NavKey::OptionUp => {
            handled = state.option_down;
            if state.option_down {
                intent = match (state.held, state.held_group) {
                    (true, Some(group)) => Some(Intent::CommitGroup(group)),
                    _ => Some(Intent::CancelHold),
                };
                next.option_down = false;
                next.held = false;
                next.held_group = None;
            }
        }
        NavKey::HoldElapsed => {
            handled = state.option_down;
            if state.option_down {
                next.held = true;
            }
        }
        NavKey::Letter(c) => {
            handled = state.option_down;
            if state.option_down {
                let wanted = c.to_uppercase().next();
                let group = order
                    .groups
                    .iter()
                    .find(|group| group.key.is_some() && group.key == wanted);
                if let Some(group) = group {
                    next.held = true;
                    next.held_group = Some(group.id);
                }
            }
        }
        NavKey::Digit(n) => match state.held_group {
            Some(group) if (1..=9).contains(&n) => {
                if tab_count(order, group) >= usize::from(n) {
                    intent = Some(Intent::ActivateDigit { group, n });
                }
                handled = true;
            }
            _ => handled = false,
        },
    }
    NavOutcome {
        state: next,
        intent,
        handled,
    }
}
