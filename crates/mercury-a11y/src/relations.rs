// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Relations between nodes (`aria-controls`, `aria-describedby`, `aria-owns`, ...).
//!
//! Owner: item 3 (`core-states`).

use a11y_dom::Rendering;
use accname::IdIndex;

use crate::host::HostState;
use crate::semantics::NodeSemantics;

/// Fill `semantics.relations`.
pub fn apply<'n, D>(
    _doc: &'n D,
    _node: D::N<'n>,
    _ids: &IdIndex<'n, D::N<'n>>,
    _semantics: &mut NodeSemantics,
) where
    D: Rendering + HostState,
{
}
