// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Which engines may be asked for suggestions (owner decision O-23).

/// The engines remote suggestions may be fetched from: the privacy-friendly ones. Google, Bing and
/// Brave never (owner decision O-23, 18 Nov 2026).
pub const REMOTE_SUGGEST_ENGINE_IDS: [&str; 4] = ["duckduckgo", "ecosia", "startpage", "wikipedia"];

/// Whether remote suggestions may be fetched from the engine with this id.
pub fn remote_suggestions_allowed(engine_id: &str) -> bool {
    REMOTE_SUGGEST_ENGINE_IDS.contains(&engine_id)
}
