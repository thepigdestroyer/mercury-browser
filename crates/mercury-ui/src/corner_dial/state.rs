// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Small pure helpers of the dial: the corner names and the session override, the mapping to the Slint
//! enum, the accessible label and the keyboard stepping. All of it is pure and tested without a
//! window (`tests/corner_dial_state.rs`).

use mercury_settings::DialCorner;
use slint::SharedString;

use crate::DialCornerKind;

/// The serde name of `corner` ("top_left", "top_right", "bottom_left", "bottom_right"): what the
/// session file stores as the window's `dial_corner`. A test pins it against `serde_json`.
pub fn corner_name(corner: DialCorner) -> &'static str {
    match corner {
        DialCorner::TopLeft => "top_left",
        DialCorner::TopRight => "top_right",
        DialCorner::BottomLeft => "bottom_left",
        DialCorner::BottomRight => "bottom_right",
    }
}

/// The inverse of [`corner_name`]; `None` for any other text (the match is exact, so
/// `"TOP_LEFT"` is not a name).
pub fn corner_from_name(name: &str) -> Option<DialCorner> {
    match name {
        "top_left" => Some(DialCorner::TopLeft),
        "top_right" => Some(DialCorner::TopRight),
        "bottom_left" => Some(DialCorner::BottomLeft),
        "bottom_right" => Some(DialCorner::BottomRight),
        _ => None,
    }
}

/// The window's own corner (the session's `dial_corner`) if it names one, else the setting.
pub fn effective_corner(session_override: Option<&str>, setting: DialCorner) -> DialCorner {
    session_override
        .and_then(corner_from_name)
        .unwrap_or(setting)
}

/// The Slint enum of `corner` (an exhaustive match).
pub fn corner_to_slint(corner: DialCorner) -> DialCornerKind {
    match corner {
        DialCorner::TopLeft => DialCornerKind::TopLeft,
        DialCorner::TopRight => DialCornerKind::TopRight,
        DialCorner::BottomLeft => DialCornerKind::BottomLeft,
        DialCorner::BottomRight => DialCornerKind::BottomRight,
    }
}

/// The settings enum of `kind` (an exhaustive match).
pub fn corner_from_slint(kind: DialCornerKind) -> DialCorner {
    match kind {
        DialCornerKind::TopLeft => DialCorner::TopLeft,
        DialCornerKind::TopRight => DialCorner::TopRight,
        DialCornerKind::BottomLeft => DialCorner::BottomLeft,
        DialCornerKind::BottomRight => DialCorner::BottomRight,
    }
}

/// "Tab dial", then ", <group>" (a missing or blank group is left out), then, only while `loading`,
/// ", page loading <p> percent" with `p` = `progress` * 100 rounded and clamped to 0..=100, or
/// ", page loading" when the progress is unknown (negative or NaN).
pub fn dial_label(active_group: Option<&str>, loading: bool, progress: f32) -> SharedString {
    let mut label = String::from("Tab dial");
    if let Some(group) = active_group.map(str::trim).filter(|g| !g.is_empty()) {
        label.push_str(", ");
        label.push_str(group);
    }
    if loading {
        label.push_str(", page loading");
        if !progress.is_nan() && progress >= 0.0 {
            let percent = (progress * 100.0).round().clamp(0.0, 100.0) as i32;
            label.push_str(&format!(" {percent} percent"));
        }
    }
    SharedString::from(label)
}

/// The tab id after (`step` > 0) or before (`step` < 0) `focused` in ring order, wrapping; `-1` when
/// there are no tabs. A `focused` id that is not in the list steps onto the first (forward) or the
/// last (backward) id; with duplicates the first match counts. Only the sign of `step` matters, and
/// `step` 0 keeps `focused` if it is in the list, else gives `-1`.
pub fn step_focus(tab_ids_in_ring_order: &[i32], focused: i32, step: i32) -> i32 {
    let ids = tab_ids_in_ring_order;
    let Some(last) = ids.len().checked_sub(1) else {
        return -1;
    };
    let at = ids.iter().position(|&id| id == focused);
    match (step.signum(), at) {
        (0, Some(_)) => focused,
        (0, None) => -1,
        (1, Some(i)) => ids[if i == last { 0 } else { i + 1 }],
        (1, None) => ids[0],
        (_, Some(i)) => ids[if i == 0 { last } else { i - 1 }],
        (_, None) => ids[last],
    }
}
