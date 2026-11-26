/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: A11Y-2 EE9. A translation of Firefox's `HTMLTableAccessible::IsProbablyLayoutTable`
//! (`accessible/html/HTMLTableAccessible.cpp`, MPL-2.0): decides whether a `<table>` is used for
//! layout rather than data.

use a11y_dom::Rendering;

use crate::host::HostState;

// stub
pub fn is_probably_layout_table<'n, D: Rendering + HostState>(
    _doc: &'n D,
    _table: D::N<'n>,
) -> bool {
    false
}
