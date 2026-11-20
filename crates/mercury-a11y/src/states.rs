// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! States, properties and values (EE4).
//!
//! Owner: item 3 (`core-states`).

use a11y_dom::Rendering;

use crate::host::HostState;
use crate::semantics::NodeSemantics;

/// Set the states, properties, value and `hidden` of `semantics`.
pub fn apply<'n, D>(_doc: &'n D, _node: D::N<'n>, _semantics: &mut NodeSemantics)
where
    D: Rendering + HostState,
{
}
