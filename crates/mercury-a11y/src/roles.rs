// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Roles: the ARIA role list and the HTML-AAM element roles as AccessKit roles.
//!
//! Owner: item 2 (`core-names`). The skeleton only maps what `accname::role` answers to
//! `GenericContainer`, so that the crate compiles and the pipeline runs.

use a11y_dom::Rendering;
use accesskit::Role;

use crate::host::HostState;

/// The AccessKit role of an element or text node.
pub fn role<'n, D>(_doc: &'n D, node: D::N<'n>) -> Role
where
    D: Rendering + HostState,
{
    use a11y_dom::{Node, NodeKind};
    match node.kind() {
        NodeKind::Text => Role::TextRun,
        NodeKind::Element => Role::GenericContainer,
    }
}
