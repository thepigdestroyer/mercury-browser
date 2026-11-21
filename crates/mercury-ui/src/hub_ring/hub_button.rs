// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The hub button's donut (SPEC §4.3): one arc per group, in the group's colour, sized by its
//! tab count, in the button's own 30 px box; and its accessible label. Pure.
//!
//! STUB in the scaffold commit (signatures are the contract; item 2 `nav-beads` writes the bodies).

use crate::{DonutArc, GroupRow};

/// The button's edge in px.
pub const BUTTON_SIZE: f32 = 30.0;

/// The arcs of the donut: one group = a full ring, none = one grey ring in `fallback`.
pub fn donut(_groups: &[GroupRow], _fallback: slint::Color) -> Vec<DonutArc> {
    Vec::new()
}

/// "Tab groups, 6 groups, 22 tabs" ("1 group", "1 tab"; no groups: "Tab groups, no tabs").
pub fn label(_groups: &[GroupRow]) -> String {
    String::new()
}
