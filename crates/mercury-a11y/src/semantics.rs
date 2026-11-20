// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The result of computing a node's accessibility semantics, and the one function that computes it.
//!
//! The layout crate copies [`NodeSemantics::node`] onto its own accessibility node (keeping the
//! children, bounds, transform and graft it owns) and turns [`Relations`] into AccessKit node ids.

use a11y_dom::{NodeId, Rendering};
use accesskit::Node;
use accname::IdIndex;

use crate::host::HostState;
use crate::{actions, names, relations, roles, states};

/// The DOM nodes a node relates to, as the DOM's own ids. Empty when the attribute is absent or
/// names nothing in the document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Relations {
    pub controls: Vec<NodeId>,
    pub described_by: Vec<NodeId>,
    pub details: Vec<NodeId>,
    pub error_message: Vec<NodeId>,
    pub flow_to: Vec<NodeId>,
    pub labelled_by: Vec<NodeId>,
    pub owns: Vec<NodeId>,
    pub active_descendant: Option<NodeId>,
}

/// Everything the engine needs to know about one DOM node.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeSemantics {
    /// Role, name (`label`), description, states, value, ranges, flags and advertised actions;
    /// no children, bounds, transform, tree id or relation lists.
    pub node: Node,
    pub relations: Relations,
    /// `aria-hidden`, `inert`, the `hidden` attribute, `visibility: hidden`: the node and its
    /// subtree are not exposed. (`display: none` is layout's own hide.)
    pub hidden: bool,
}

/// Compute the semantics of `node`. The order is fixed: role, name and description, states, relations,
/// advertised actions; each step may read what the earlier ones set.
pub fn compute<'n, D>(doc: &'n D, node: D::N<'n>, ids: &IdIndex<'n, D::N<'n>>) -> NodeSemantics
where
    D: Rendering + HostState,
{
    let role = roles::role(doc, node);
    let mut semantics = NodeSemantics {
        node: Node::new(role),
        relations: Relations::default(),
        hidden: false,
    };
    names::apply(doc, node, ids, &mut semantics);
    states::apply(doc, node, &mut semantics);
    relations::apply(doc, node, ids, &mut semantics);
    actions::apply(doc, node, &mut semantics);
    semantics
}
