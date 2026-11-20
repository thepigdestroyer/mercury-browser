// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Accessible name and description (accname 1.2) on top of the `accname` crate.
//!
//! Owner: item 2 (`core-names`).

use a11y_dom::Rendering;
use accname::IdIndex;

use crate::host::HostState;
use crate::semantics::NodeSemantics;

/// Set the name (`label`) and the description of `semantics.node`.
pub fn apply<'n, D>(
    _doc: &'n D,
    _node: D::N<'n>,
    _ids: &IdIndex<'n, D::N<'n>>,
    _semantics: &mut NodeSemantics,
) where
    D: Rendering + HostState,
{
}
