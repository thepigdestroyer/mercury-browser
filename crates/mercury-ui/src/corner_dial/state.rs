// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Small pure helpers of the dial: the corner names and the session override, the mapping to the Slint
//! enum, the accessible label and the keyboard stepping. STUB in the scaffold commit: item 2 `state`
//! of lane corner-dial writes the real bodies. The signatures are the pinned contract.

use mercury_settings::DialCorner;
use slint::SharedString;

use crate::DialCornerKind;

/// The serde name of `corner` ("top_left", "top_right", "bottom_left", "bottom_right"): what the
/// session file stores as the window's `dial_corner`.
pub fn corner_name(_corner: DialCorner) -> &'static str {
    ""
}

/// The inverse of [`corner_name`]; `None` for any other text.
pub fn corner_from_name(_name: &str) -> Option<DialCorner> {
    None
}

/// The window's own corner (the session's `dial_corner`) if it names one, else the setting.
pub fn effective_corner(_session_override: Option<&str>, setting: DialCorner) -> DialCorner {
    setting
}

/// The Slint enum of `corner` (an exhaustive match).
pub fn corner_to_slint(_corner: DialCorner) -> DialCornerKind {
    DialCornerKind::BottomRight
}

/// The settings enum of `kind` (an exhaustive match).
pub fn corner_from_slint(_kind: DialCornerKind) -> DialCorner {
    DialCorner::BottomRight
}

/// "Tab dial", then ", <group>", then ", page loading <p> percent" (", page loading" when the
/// progress is unknown, negative).
pub fn dial_label(_active_group: Option<&str>, _loading: bool, _progress: f32) -> SharedString {
    SharedString::new()
}

/// The tab id after (`step` > 0) or before (`step` < 0) `focused` in ring order, wrapping; `-1` when
/// there are no tabs.
pub fn step_focus(_tab_ids_in_ring_order: &[i32], _focused: i32, _step: i32) -> i32 {
    -1
}
