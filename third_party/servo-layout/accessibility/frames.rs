/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: a11y-1 item 5 (`actions-frames`) owns this file. The lead's stub only keeps the pinned
// interface in place so that item 4's call compiles; item 5 replaces the body (EE7, iframes). See
// `.superpowers/sdd/lane-a11y-1-brief.md`.

use accesskit::TreeId;
use script::layout_dom::ServoLayoutNode;

/// The tree id of the document inside an `<iframe>` or `<frame>` node, so that the node can graft
/// it; `None` for every other node, or when the nested document is not known yet.
#[allow(dead_code)]
pub(super) fn child_tree_id(_dom_node: &ServoLayoutNode<'_>) -> Option<TreeId> {
    None
}
