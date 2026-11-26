/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: A11Y-2 text runs. The engine side of `mercury_a11y::text`. Stubs until A11Y-2 task 10.

use script::layout_dom::ServoLayoutNode;

use super::accessibility_node::AccessibilityNode;
use super::accessibility_tree::{AccessibilityTree, AccessibilityUpdate};

// stub
pub(super) fn update_children(
    _node: &mut AccessibilityNode,
    _dom_node: &ServoLayoutNode<'_>,
    _tree: &mut AccessibilityTree,
    _update: &mut AccessibilityUpdate<'_>,
) {
}

// stub
pub(super) fn text_value(_dom_node: &ServoLayoutNode<'_>) -> Option<String> {
    None
}
