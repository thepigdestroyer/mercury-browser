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
    Move { tab: i32, group: i32, index: usize },
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
    ActivateDigit { group: i32, n: u8 },
}

/// The result of one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavOutcome {
    pub state: NavState,
    pub intent: Option<Intent>,
    /// The key was used (the popover's `FocusScope` accepts it).
    pub handled: bool,
}

/// One key applied to `state` over `order`.
pub fn step(_order: &FocusOrder, state: &NavState, _key: NavKey) -> NavOutcome {
    NavOutcome {
        state: *state,
        intent: None,
        handled: false,
    }
}
