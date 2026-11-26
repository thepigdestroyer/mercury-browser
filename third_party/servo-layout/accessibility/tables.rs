/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: A11Y-2 tables and lists. The engine side of `mercury_a11y::tables` and `lists`. Stubs until
//! A11Y-2 task 13.

use super::accessibility_tree::{AccessibilityTree, AccessibilityUpdate};

// stub
pub(super) fn finish(_tree: &mut AccessibilityTree, _update: &mut AccessibilityUpdate<'_>) {}
