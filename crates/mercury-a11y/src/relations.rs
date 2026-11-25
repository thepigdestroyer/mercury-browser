// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Relations between nodes (`aria-controls`, `aria-describedby`, `aria-owns`, ...).
//!
//! Owner: item 3 (`core-states`). [`apply`] fills [`crate::Relations`] with the DOM's own
//! [`a11y_dom::NodeId`]s; the layout crate maps them to AccessKit ids (a node that is not in the tree
//! yet is retried on the next update).
//!
//! # Rules
//!
//! * An IDREF list is resolved through the document's [`IdIndex`], **in the tree scope of the
//!   referencing node** (an id inside a shadow root is not visible from outside). An id that resolves
//!   to nothing is dropped; so is a duplicate, and so is the node itself (`aria-owns`, `aria-controls`
//!   and the rest say nothing useful about the node itself). Each list holds at most
//!   [`MAX_RELATED`] nodes: a page that lists ten thousand ids does not get a ten-thousand-entry
//!   relation.
//! * `labelled_by`: `aria-labelledby` when it resolves to something, else the **native labels** of a
//!   labelable element (`label[for]` in document order, then the wrapping `label` when the control is
//!   its first labelable descendant and the label has no `for`), the `legend` of a `fieldset`, the
//!   `caption` of a `table` and the `figcaption` of a `figure`. (The name itself is `names`'.)
//! * `error_message`: the first id of `aria-errormessage`, **only on a node that is invalid**
//!   (`aria-errormessage` applies when `aria-invalid` is true; `states` runs first).
//! * `active_descendant`: the first id of `aria-activedescendant`. The target is not required to be
//!   a descendant: combobox patterns point into a popup the element only `aria-controls`.
//! * `owns`: a node cannot own itself, one of its **ancestors** (a cycle) or a node that an **earlier**
//!   element in document order already owns: the first owner wins, as the specification says for an
//!   element claimed twice. The scan for earlier owners walks the document up to the node, once per
//!   node that has `aria-owns`.

use std::collections::HashSet;

use a11y_dom::{Node, NodeId, NodeKind, Rendering, ancestors, descendants, elements};
use accname::IdIndex;

use crate::host::HostState;
use crate::semantics::NodeSemantics;

/// The most nodes one relation lists.
pub const MAX_RELATED: usize = 256;

/// Fill `semantics.relations`.
pub fn apply<'n, D>(
    doc: &'n D,
    node: D::N<'n>,
    ids: &IdIndex<'n, D::N<'n>>,
    semantics: &mut NodeSemantics,
) where
    D: Rendering + HostState,
{
    if node.kind() != NodeKind::Element {
        return;
    }
    let invalid = semantics.node.invalid().is_some();
    let relations = &mut semantics.relations;
    relations.controls = id_list(node, ids, "aria-controls");
    relations.described_by = id_list(node, ids, "aria-describedby");
    relations.details = id_list(node, ids, "aria-details");
    relations.flow_to = id_list(node, ids, "aria-flowto");
    relations.labelled_by = labelled_by(node, ids);
    relations.owns = owns(doc, node, ids);
    relations.active_descendant = first_id(node, ids, "aria-activedescendant");
    if invalid {
        relations.error_message = first_id(node, ids, "aria-errormessage")
            .into_iter()
            .collect();
    }
}

/// The nodes an IDREF list names, in the order written, without the node itself and duplicates.
fn resolve_list<'n, N: Node<'n>>(node: N, ids: &IdIndex<'n, N>, attribute: &str) -> Vec<N> {
    let mut found: Vec<N> = Vec::new();
    let Some(value) = node.attr(attribute) else {
        return found;
    };
    for token in value.split_ascii_whitespace() {
        if found.len() >= MAX_RELATED {
            break;
        }
        if let Some(target) = ids.get(node, token)
            && target != node
            && !found.contains(&target)
        {
            found.push(target);
        }
    }
    found
}

fn id_list<'n, N: Node<'n>>(node: N, ids: &IdIndex<'n, N>, attribute: &str) -> Vec<NodeId> {
    resolve_list(node, ids, attribute)
        .into_iter()
        .map(Node::id)
        .collect()
}

/// The node the first id of a single-IDREF attribute names.
fn first_id<'n, N: Node<'n>>(node: N, ids: &IdIndex<'n, N>, attribute: &str) -> Option<NodeId> {
    let token = node.attr(attribute)?.split_ascii_whitespace().next()?;
    let target = ids.get(node, token)?;
    (target != node).then(|| target.id())
}

// ---------------------------------------------------------------------------------------------
// labelled by
// ---------------------------------------------------------------------------------------------

/// Labelable elements (HTML): the ones a `label` can label.
fn is_labelable<'n, N: Node<'n>>(node: N) -> bool {
    match node.local_name() {
        "button" | "meter" | "output" | "progress" | "select" | "textarea" => true,
        "input" => !node
            .attr("type")
            .is_some_and(|ty| ty.trim().eq_ignore_ascii_case("hidden")),
        _ => false,
    }
}

fn labelled_by<'n, N: Node<'n>>(node: N, ids: &IdIndex<'n, N>) -> Vec<NodeId> {
    let explicit = id_list(node, ids, "aria-labelledby");
    if !explicit.is_empty() {
        return explicit;
    }
    let mut labels: Vec<N> = Vec::new();
    if is_labelable(node) {
        labels.extend(ids.labels_for(node).iter().copied());
        if let Some(wrapper) = ancestors(node).find(|a| a.is_element("label"))
            && !wrapper.has_attr("for")
            && first_labelable_descendant(wrapper) == Some(node)
            && !labels.contains(&wrapper)
        {
            labels.push(wrapper);
        }
    } else {
        let caption = match node.local_name() {
            "fieldset" => Some("legend"),
            "table" => Some("caption"),
            "figure" => Some("figcaption"),
            _ => None,
        };
        if let Some(caption) = caption
            && let Some(child) = node.children().find(|c| c.is_element(caption))
        {
            labels.push(child);
        }
    }
    labels.truncate(MAX_RELATED);
    labels.into_iter().map(Node::id).collect()
}

/// The control a `label` without `for` labels: its first labelable descendant.
fn first_labelable_descendant<'n, N: Node<'n>>(label: N) -> Option<N> {
    descendants(label).find(|d| d.kind() == NodeKind::Element && is_labelable(*d))
}

// ---------------------------------------------------------------------------------------------
// owns
// ---------------------------------------------------------------------------------------------

fn owns<'n, D>(doc: &'n D, node: D::N<'n>, ids: &IdIndex<'n, D::N<'n>>) -> Vec<NodeId>
where
    D: Rendering + HostState,
{
    if !node.has_attr("aria-owns") {
        return Vec::new();
    }
    let mut targets = resolve_list(node, ids, "aria-owns");
    // A node cannot own one of its own ancestors: that is a cycle.
    targets.retain(|target| !ancestors(node).any(|ancestor| ancestor == *target));
    if targets.is_empty() {
        return Vec::new();
    }
    // The first owner in document order keeps a node that two elements claim.
    let mut claimed: HashSet<NodeId> = HashSet::new();
    for earlier in elements(doc).take_while(|element| *element != node) {
        if earlier.has_attr("aria-owns") {
            claimed.extend(id_list(earlier, ids, "aria-owns"));
        }
    }
    targets
        .into_iter()
        .filter(|target| !claimed.contains(&target.id()))
        .map(Node::id)
        .collect()
}
