// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! List positions for AccessKit: which item of how many an `<li>` is.

use a11y_dom::Rendering;

use crate::host::HostState;

/// The 1-based position of a list item and the size of its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListItemInfo {
    pub position: usize,
    pub size: usize,
}

// stub
pub fn list_item_info<'n, D: Rendering + HostState>(
    _doc: &'n D,
    _item: D::N<'n>,
) -> Option<ListItemInfo> {
    None
}
