// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Drag and drop in Orbit (SPEC §4.6, brief D11): the payload that `DragArea` carries to `DropArea`,
//! the one resolver that decides what a drop means (whatever started the drag), the slot to final
//! place conversion and the group operations the glue runs on the controller.
//!
//! STUB in the scaffold commit: the types and signatures are final, the bodies answer `NoChange`
//! and `Err`; item 3 `drag` of lane orbit-view writes the real ones. The types are the contract of
//! the machine (item 1) and the glue (item 5).

use std::rc::Rc;

use slint::DataTransfer;

use super::OrbitInput;

/// What is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A tab (a preview, or a segment of a ring): the `TabRow.id`.
    Tab(i32),
    /// A planet (reorder): the `GroupRow.id`.
    Group(i32),
}

/// What the pointer is over while dragging, or where it was released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// A planet, a chip or the level-2 centre ring: the `GroupRow.id`. `slot` is the insertion slot
    /// in the group when the pointer is on its ring (what `RingBands` reports), `None` = at the end
    /// (a halo or a chip).
    Group { group: i32, slot: Option<usize> },
    /// The dashed "New group" ring.
    NewGroup,
    /// Empty stage (also "New group" for a tab: the popover opens).
    Empty,
}

/// What a drop means. Never an error: a bad drop is `Reject`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// A valid drop that changes nothing (the tab's own slot, a group onto itself).
    NoChange,
    /// Not allowed: a pinned source, an unknown id, a stale payload, a group onto "New group".
    Reject,
    /// Move the tab: `group` is `None` for the loose planet; `index` is the final place in the group.
    MoveTab {
        tab: i32,
        group: Option<i32>,
        index: usize,
    },
    /// Make a new group from the tab (the glue opens the name popover first).
    NewGroup { tab: i32 },
    /// Move the group to `index` in the group order.
    ReorderGroup { group: i32, index: usize },
}

/// What the glue asks `talaria-core`'s controller to do (one `match` in `orbit.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupOp {
    /// `move_tab_to_group(tab, Some(group), index)`.
    MoveTab { tab: i32, group: i32, index: usize },
    /// `move_tab_to_group(tab, None, index)` or `ungroup_tab`: the tab becomes loose.
    Ungroup { tab: i32 },
    /// `create_group(&[tab], name, colour)`; `colour` is a preset name.
    CreateGroup {
        tabs: Vec<i32>,
        name: Option<String>,
        colour: Option<String>,
    },
    /// `reorder_group(group, index)`.
    ReorderGroup { group: i32, index: usize },
}

impl Decision {
    /// The controller operation for this decision, if it has one (`NewGroup` has none until the
    /// popover is confirmed: the machine then emits `Effect::Apply(GroupOp::CreateGroup ..)`).
    /// STUB: always `None`.
    pub fn op(&self) -> Option<GroupOp> {
        None
    }
}

/// What a `DragArea` carries: the in-app payload (the authority) with the session of the Orbit that
/// made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrbitPayload {
    /// The nonce of the Orbit that was open when the drag started.
    pub session: u64,
    pub source: Source,
}

/// Why a `DataTransfer` is not an Orbit payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadError {
    /// No in-app payload at all (empty, plain text, an image, another app's data).
    NotOrbit,
    /// A payload from another Orbit (an older open, another window).
    StaleSession,
    /// An id that is not an id (negative other than the loose planet).
    BadId,
}

impl OrbitPayload {
    /// The `DataTransfer` for a `DragArea`: user data is the authority; the plain text is the tab's
    /// title or the group's name, never a URL. STUB: user data only.
    pub fn to_data_transfer(&self, _text: &str) -> DataTransfer {
        let mut data = DataTransfer::default();
        data.set_user_data(Rc::new(*self));
        data
    }

    /// Reads a payload back, accepting only this `session`. STUB: always `NotOrbit`.
    pub fn from_data_transfer(
        _data: &DataTransfer,
        _session: u64,
    ) -> Result<OrbitPayload, PayloadError> {
        Err(PayloadError::NotOrbit)
    }
}

/// What dropping `source` on `target` means for these rows. STUB: `NoChange`.
pub fn resolve(_source: Source, _target: DropTarget, _input: &OrbitInput) -> Decision {
    Decision::NoChange
}

/// The final place in the group for a tab dropped at insertion `slot` (`ring-bands` reports slots
/// before removal): when the tab is already in that group and the slot is after its own place the
/// final place is `slot - 1`. STUB: `slot`.
pub fn slot_to_final_index(
    _current_group: Option<i32>,
    _current_index: usize,
    _target_group: i32,
    slot: usize,
) -> usize {
    slot
}

/// Whether a `DropArea` should accept this payload on this target (what `can-drop` answers; the
/// Slint side maps `true` to `DragAction.move`). STUB: `false`.
pub fn can_accept(_payload: &OrbitPayload, _target: DropTarget, _input: &OrbitInput) -> bool {
    false
}
