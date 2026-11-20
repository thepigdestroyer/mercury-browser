// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The actions a node advertises to assistive technology.
//!
//! Owner: item 3 (`core-states`). Item 5's `accessibility_actions.rs` (script side) implements
//! exactly [`SUPPORTED_ACTIONS`].

use a11y_dom::Rendering;
use accesskit::Action;

use crate::host::HostState;
use crate::semantics::NodeSemantics;

/// Every action the engine can perform. A test pins this list.
pub const SUPPORTED_ACTIONS: &[Action] = &[];

/// Add the actions `node` supports to `semantics.node`.
pub fn apply<'n, D>(_doc: &'n D, _node: D::N<'n>, _semantics: &mut NodeSemantics)
where
    D: Rendering + HostState,
{
}
