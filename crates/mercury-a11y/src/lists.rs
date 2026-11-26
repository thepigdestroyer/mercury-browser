// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! List item positions for AccessKit: which item of how many an `<li>` is.

use a11y_dom::{Node, Rendering};

use crate::host::HostState;

/// The 1-based position of a list item and the size of its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListItemInfo {
    pub position: usize,
    pub size: usize,
}

/// The position of an `li` among the `li` children of its list, and the number of them. A `value`
/// or `reversed` attribute changes the marker, not the position.
pub fn list_item_info<'n, D: Rendering + HostState>(
    _doc: &'n D,
    item: D::N<'n>,
) -> Option<ListItemInfo> {
    if !item.is_element("li") {
        return None;
    }
    let list = item.parent()?;
    let items: Vec<D::N<'n>> = list.children().filter(|c| c.is_element("li")).collect();
    let position = items.iter().position(|c| *c == item)? + 1;
    Some(ListItemInfo {
        position,
        size: items.len(),
    })
}
