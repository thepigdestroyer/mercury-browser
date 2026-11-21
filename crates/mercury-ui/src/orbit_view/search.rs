// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Orbit's search (SPEC §4.6, brief D6): which tabs match a query and which one ⏎ zooms to.
//!
//! STUB in the scaffold commit; item 1 `machine` of lane orbit-view writes the real bodies.

use super::OrbitInput;

/// Whether a tab with this title and host matches `query`: case-insensitive, every
/// whitespace-separated word must occur in the title or the host; an empty query matches all.
/// STUB: `true`.
pub fn matches(_query: &str, _title: &str, _host: &str) -> bool {
    true
}

/// The `TabRow.id` of the first matching tab in group order, then strip order; `None` for no match
/// or an empty query. STUB: `None`.
pub fn top_result(_query: &str, _input: &OrbitInput) -> Option<i32> {
    None
}
