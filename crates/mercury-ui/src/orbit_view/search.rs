// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Orbit's search (SPEC §4.6, brief D6): which tabs match a query and which one ⏎ zooms to.
//!
//! The rule is the one a find field has everywhere: case does not matter (Unicode lower-casing, so
//! "CAFÉ" finds "café"), every whitespace-separated word of the query must occur somewhere in the
//! tab's title or host, and an empty query matches everything. **Diacritics are left alone**:
//! "cafe" does not find "Café" (no accent folding; documented, and the hub ring's own predicate may
//! differ, see the brief's merge note).

use super::OrbitInput;

/// A query split into lower-case words, made once for many tabs.
struct Words(Vec<String>);

impl Words {
    fn new(query: &str) -> Self {
        Self(query.split_whitespace().map(str::to_lowercase).collect())
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn accepts(&self, title: &str, host: &str) -> bool {
        if self.0.is_empty() {
            return true;
        }
        let title = title.to_lowercase();
        let host = host.to_lowercase();
        self.0
            .iter()
            .all(|word| title.contains(word.as_str()) || host.contains(word.as_str()))
    }
}

/// Whether a tab with this title and host matches `query`: case-insensitive, every
/// whitespace-separated word must occur in the title or the host; an empty query matches all.
pub fn matches(query: &str, title: &str, host: &str) -> bool {
    Words::new(query).accepts(title, host)
}

/// The `TabRow.id` of the first matching tab in group order, then strip order; `None` for no match
/// or an empty (or blank) query.
pub fn top_result(query: &str, input: &OrbitInput) -> Option<i32> {
    let words = Words::new(query);
    if words.is_empty() {
        return None;
    }
    input.groups.iter().find_map(|group| {
        input
            .tabs_of(group.id)
            .into_iter()
            .find(|tab| words.accepts(&tab.title, &tab.host))
            .map(|tab| tab.id)
    })
}
