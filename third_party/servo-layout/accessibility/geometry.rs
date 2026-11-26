/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: A11Y-2 node bounds. The engine side of `mercury_a11y::geometry`. Stubs until A11Y-2 task 14.

use script::layout_dom::ServoLayoutNode;

use super::accessibility_node::AccessibilityNode;

// stub
pub(super) fn bounds_for(_node: &mut AccessibilityNode, _dom_node: &ServoLayoutNode<'_>) {}
