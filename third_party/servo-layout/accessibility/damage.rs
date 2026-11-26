/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: A11Y-2 update gate. The engine side of `mercury_a11y::gate`. Stubs until A11Y-2 task 8 wires it in.

use super::accessibility_tree::{AccessibilityTree, AccessibilityUpdate};

// stub
pub(super) fn gate(_tree: &mut AccessibilityTree, _update: &mut AccessibilityUpdate<'_>) {}
