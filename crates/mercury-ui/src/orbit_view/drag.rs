// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Drag and drop in Orbit (SPEC §4.6, brief D11): the payload that `DragArea` carries to `DropArea`,
//! the one resolver that decides what a drop means (whatever started the drag), the slot to final
//! place conversion and the group operations the glue runs on the controller.
//!
//! The types are the contract of the machine (item 1) and the glue (item 5). Everything here is
//! pure: no window, no clock.
//!
//! - The payload is `slint::DataTransfer` user data ([`OrbitPayload`]); only user data that downcasts
//!   to it, from this session, with sane ids, is accepted. Text is never read back: another app can
//!   forge text, it cannot forge an `Rc<OrbitPayload>`.
//! - The plain text of a drag is the tab's title or the group's name, never a URL.
//! - [`resolve`] is the one place that decides what a drop means; [`can_accept`] is "would the drop
//!   change something".

use std::rc::Rc;

use slint::DataTransfer;

use super::{LOOSE_GROUP_ID, OrbitInput, Target};

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
    /// Move the tab: `group` is `None` for the loose planet; `index` is the final place in the group
    /// (for the loose planet: among the pseudo planet's tabs, informational).
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

/// What a `DragArea` of this kind carries: `"tab"` (a preview) or `"group"` (a planet).
pub fn source_of(kind: &str, id: i32) -> Option<Source> {
    match kind {
        "tab" => Some(Source::Tab(id)),
        "group" => Some(Source::Group(id)),
        _ => None,
    }
}

/// Where a `DropArea` of this kind is: `"group"`, `"new-group"` or `"empty"`; `slot` is -1 for
/// "at the end".
pub fn drop_target_of(kind: &str, id: i32, slot: i32) -> Option<DropTarget> {
    match kind {
        "group" => Some(DropTarget::Group {
            group: id,
            slot: usize::try_from(slot).ok(),
        }),
        "new-group" => Some(DropTarget::NewGroup),
        "empty" => Some(DropTarget::Empty),
        _ => None,
    }
}

/// The [`Target`] of a Slint id where a negative id means "none" (the backdrop).
pub fn target_from_id(id: i32, make: fn(i32) -> Target) -> Target {
    if id < 0 { Target::Backdrop } else { make(id) }
}

/// The target the view shows as lit during a drag of `drag_source` in `session`: only one the drop
/// would change something on (`None` for no target, or one that [`can_accept`] refuses).
pub fn lit_target(
    drag_source: Source,
    session: u64,
    target: Option<DropTarget>,
    input: &OrbitInput,
) -> Option<DropTarget> {
    let target = target?;
    let payload = OrbitPayload {
        session,
        source: drag_source,
    };
    can_accept(&payload, target, input).then_some(target)
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
    /// `NoChange` and `Reject` have none. A move to the loose planet (`group: None`) is `Ungroup`:
    /// the controller's strip index is not known to Orbit, so `index` is dropped there.
    pub fn op(&self) -> Option<GroupOp> {
        match *self {
            Decision::MoveTab {
                tab,
                group: Some(group),
                index,
            } => Some(GroupOp::MoveTab { tab, group, index }),
            Decision::MoveTab {
                tab, group: None, ..
            } => Some(GroupOp::Ungroup { tab }),
            Decision::ReorderGroup { group, index } => Some(GroupOp::ReorderGroup { group, index }),
            Decision::NoChange | Decision::Reject | Decision::NewGroup { .. } => None,
        }
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
    /// title or the group's name (see [`drag_text`]) so that a drop elsewhere reads something. A text
    /// that looks like a URL is left out: Orbit never puts an address on the pasteboard.
    pub fn to_data_transfer(&self, text: &str) -> DataTransfer {
        let mut data = DataTransfer::default();
        data.set_user_data(Rc::new(*self));
        if !looks_like_url(text) {
            data.set_plain_text(text.into());
        }
        data
    }

    /// Reads a payload back, accepting only this `session`. Only user data that is an
    /// [`OrbitPayload`] counts; the plain text is never parsed.
    pub fn from_data_transfer(
        data: &DataTransfer,
        session: u64,
    ) -> Result<OrbitPayload, PayloadError> {
        let user = data.user_data().ok_or(PayloadError::NotOrbit)?;
        let payload = *user
            .downcast::<OrbitPayload>()
            .map_err(|_| PayloadError::NotOrbit)?;
        if payload.session != session {
            return Err(PayloadError::StaleSession);
        }
        let sane = match payload.source {
            Source::Tab(id) => id >= 0,
            Source::Group(id) => id >= 0 || id == LOOSE_GROUP_ID,
        };
        if sane {
            Ok(payload)
        } else {
            Err(PayloadError::BadId)
        }
    }
}

/// Whether a text is an address and not words: no whitespace and a scheme separator, a `www.`
/// start, one of the schemes without `//`, or a scheme-less address (see [`scheme_less_address`]).
fn looks_like_url(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    lower.contains("://")
        || lower.starts_with("www.")
        || ["mailto:", "data:", "javascript:", "about:", "blob:"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
        || scheme_less_address(&lower)
}

/// An address without its scheme, as a page with no title is often shown: a host with a dot-label
/// or a port, then `/`, `?` or `#` ("example.com/a?b=1", "localhost:8080/x"). A bare host
/// ("example.com") and words with a slash ("TCP/IP") are not.
fn scheme_less_address(text: &str) -> bool {
    let Some(end) = text.find(['/', '?', '#']) else {
        return false;
    };
    let authority = &text[..end];
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host, true)
        }
        _ => (authority, false),
    };
    let host_chars = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_'));
    let dotted = host.contains('.') && host.split('.').all(|label| !label.is_empty());
    host_chars && (dotted || port)
}

/// Whether a text is a bare host name (with an optional port): the one kind of address that is
/// fine as a drag text.
fn is_bare_host(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty()
        || text.contains(char::is_whitespace)
        || text.contains(['/', '?', '#', '@', '\\'])
    {
        return false;
    }
    match text.rsplit_once(':') {
        None => true,
        Some((host, port)) => {
            !host.is_empty() && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
        }
    }
}

/// The plain text a drag carries: the tab's title or the group's name; for a tab whose title is an
/// address (a page without a title) its host, tested as a bare host (so a `www.` host is fine);
/// empty for an unknown id or a host that is itself an address. Never a URL.
pub fn drag_text(source: Source, input: &OrbitInput) -> String {
    match source {
        Source::Tab(id) => {
            let Some(tab) = input.tab(id) else {
                return String::new();
            };
            if !looks_like_url(&tab.title) {
                return tab.title.to_string();
            }
            if is_bare_host(&tab.host) {
                tab.host.trim().to_string()
            } else {
                String::new()
            }
        }
        Source::Group(id) => input
            .group(id)
            .map(|group| group.name.to_string())
            .unwrap_or_default(),
    }
}

/// What dropping `source` on `target` means for these rows.
///
/// - A tab onto a group: [`Decision::MoveTab`] with the final place (see [`slot_to_final_index`]);
///   a slot past the end is the end; `slot: None` is the end of another group, and nothing for the
///   tab's own group (to reorder inside a group, drop on its ring). The loose planet gives
///   `group: None` (its `index` is the place among the pseudo planet's tabs; see [`Decision::op`]);
///   a tab that is already loose changes nothing there.
/// - A tab onto "New group" or empty space: [`Decision::NewGroup`].
/// - A group onto another group: [`Decision::ReorderGroup`] with the target's place in the order
///   (the slot is ignored); onto itself or onto empty space: `NoChange`; onto "New group" or from or
///   onto the loose planet (always last, not reorderable): `Reject`.
/// - A pinned tab, an unknown tab or group: `Reject`.
pub fn resolve(source: Source, target: DropTarget, input: &OrbitInput) -> Decision {
    match source {
        Source::Tab(tab) => resolve_tab(tab, target, input),
        Source::Group(group) => resolve_group(group, target, input),
    }
}

fn resolve_tab(tab: i32, target: DropTarget, input: &OrbitInput) -> Decision {
    let Some(row) = input.tab(tab) else {
        return Decision::Reject;
    };
    if input.is_pinned(tab) {
        return Decision::Reject;
    }
    let (group, slot) = match target {
        DropTarget::NewGroup | DropTarget::Empty => return Decision::NewGroup { tab },
        DropTarget::Group { group, slot } => (group, slot),
    };
    if input.group(group).is_none() {
        return Decision::Reject;
    }
    let tabs = input.tabs_of(group);
    let len = tabs.len();
    let own = row.group == group;
    if group == LOOSE_GROUP_ID {
        if own {
            return Decision::NoChange;
        }
        return Decision::MoveTab {
            tab,
            group: None,
            index: slot.map_or(len, |slot| slot.min(len)),
        };
    }
    if !own {
        return Decision::MoveTab {
            tab,
            group: Some(group),
            index: slot.map_or(len, |slot| slot.min(len)),
        };
    }
    let (Some(at), Some(slot)) = (tabs.iter().position(|t| t.id == tab), slot) else {
        return Decision::NoChange;
    };
    let index = slot_to_final_index(Some(group), at, group, slot.min(len));
    if index == at {
        Decision::NoChange
    } else {
        Decision::MoveTab {
            tab,
            group: Some(group),
            index,
        }
    }
}

fn resolve_group(group: i32, target: DropTarget, input: &OrbitInput) -> Decision {
    if group == LOOSE_GROUP_ID || input.group(group).is_none() {
        return Decision::Reject;
    }
    match target {
        DropTarget::NewGroup => Decision::Reject,
        DropTarget::Empty => Decision::NoChange,
        DropTarget::Group { group: to, .. } => {
            if to == LOOSE_GROUP_ID {
                return Decision::Reject;
            }
            let Some(index) = input.groups.iter().position(|g| g.id == to) else {
                return Decision::Reject;
            };
            if to == group {
                Decision::NoChange
            } else {
                Decision::ReorderGroup { group, index }
            }
        }
    }
}

/// The final place in the group for a tab dropped at insertion `slot` (`ring-bands` reports slots
/// before removal): when the tab is already in that group and the slot is after its own place the
/// final place is `slot - 1`; its own slot and the one after it leave it where it is; an earlier
/// slot is the place. For another group the slot is the place.
pub fn slot_to_final_index(
    current_group: Option<i32>,
    current_index: usize,
    target_group: i32,
    slot: usize,
) -> usize {
    if current_group != Some(target_group) {
        return slot;
    }
    if slot > current_index + 1 {
        slot - 1
    } else if slot < current_index {
        slot
    } else {
        current_index
    }
}

/// Whether a `DropArea` should accept this payload on this target (what `can-drop` answers; the
/// Slint side maps `true` to `DragAction.move` and `false` to none): exactly when [`resolve`] would
/// change something.
pub fn can_accept(payload: &OrbitPayload, target: DropTarget, input: &OrbitInput) -> bool {
    matches!(
        resolve(payload.source, target, input),
        Decision::MoveTab { .. } | Decision::NewGroup { .. } | Decision::ReorderGroup { .. }
    )
}
