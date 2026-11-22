// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Place words and zone abbreviations as IANA zone ids (stub: the time-zone task fills it).

/// The IANA id and the display name for a place word (`nyc`, `new york`, `est`), if known.
pub(crate) fn lookup(_word: &str) -> Option<(String, String)> {
    None
}
