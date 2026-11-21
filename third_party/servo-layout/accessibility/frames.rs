/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: a11y-1 item 5 (`actions-frames`), EE7 (iframes) owns this file. It is Mercury's own: upstream
// Servo 0.8 builds no node for the document inside an `<iframe>`, so a page's accessibility tree
// stops at the frame. See `.superpowers/sdd/lane-a11y-1-brief.md`, item 5, decision D11.

//! The accessibility tree of an `<iframe>`.
//!
//! Every document has its own tree (layout builds one per pipeline, with the `TreeId` that is a
//! pure function of the `PipelineId`: `TreeId::from(pipeline_id)`). The iframe's own node
//! **is** the graft (decision D11): AccessKit lets a node that carries a `tree_id` keep its other
//! properties (role `Iframe`, the name from `title`, bounds) and takes the root of the grafted
//! tree as its only child, so the node needs no synthetic child and no new node id.
//!
//! The document inside the frame sends its updates on its own, straight to the embedder (they do
//! not pass through the parent), and only after the constellation has activated its accessibility
//! (`servo-constellation/mercury/a11y.rs`). Which tree an update belongs to, and in which order the
//! graft and the first update reach AccessKit, is the embedder's business: its queue holds a tree
//! until some update has grafted it.

use accesskit::TreeId;
use layout_api::{LayoutElementType, LayoutNode, LayoutNodeType};
use script::layout_dom::ServoLayoutNode;

/// The tree id of the document inside an `<iframe>` node, so that the node can graft it; `None`
/// for every other node, and for an iframe whose nested document is not known yet (no pipeline,
/// or it is still being created).
///
/// Servo has no `<frame>` element that nests a document (`<frameset>` is an ordinary container),
/// so the `<iframe>` is the only case.
pub(super) fn child_tree_id(dom_node: &ServoLayoutNode<'_>) -> Option<TreeId> {
    // `iframe_pipeline_id()` is only defined for an iframe element; check the type first so that
    // it is never asked about another node.
    if dom_node.type_id() != Some(LayoutNodeType::Element(LayoutElementType::HTMLIFrameElement)) {
        return None;
    }
    dom_node.iframe_pipeline_id().map(TreeId::from)
}
