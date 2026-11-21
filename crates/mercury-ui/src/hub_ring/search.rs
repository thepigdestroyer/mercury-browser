// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The ring's search filter (brief D8). Pure: no window, no Slint models.
//!
//! The query is trimmed and split on whitespace; every term must match (AND) as a
//! case-insensitive, diacritic-folded substring of the tab's title, host or group name. An empty
//! query is inactive. Rank: all terms at the start of a title word, then a title substring, then
//! the host, then the group name; ties keep ring order. The centre lists the top two results and a
//! "Search the web for ..." row.
//!
//! STUB in the scaffold commit (types and signatures are the contract; item 1 `search` writes the
//! bodies).

use crate::TabRow;

/// What the filter knows about one tab (built by the controller from the rows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchDoc {
    pub tab: i32,
    pub title: String,
    pub host: String,
    pub group_id: i32,
    pub group_name: String,
    /// The group's key letter ("S"), empty for none (loose).
    pub key_label: String,
    /// 1-based position among the shown tabs of its group (0 for a pinned tab).
    pub index_in_group: usize,
    pub pinned: bool,
}

/// A parsed query: folded terms.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub terms: Vec<String>,
    /// The trimmed text as typed (for "Search the web for ...").
    pub text: String,
}

impl Query {
    /// Trims, splits on whitespace and folds (see [`fold`]).
    pub fn parse(text: &str) -> Self {
        Self {
            terms: Vec::new(),
            text: text.trim().to_owned(),
        }
    }

    /// At least one term: the ring fades non-matches.
    pub fn is_active(&self) -> bool {
        !self.terms.is_empty()
    }
}

/// How well a tab matches; lower is better (`0` = every term starts a title word).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score(pub u8);

/// One result row of the centre.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRowData {
    pub tab: i32,
    pub title: String,
    /// "Lisbon trip · ⌥S 3 · ⏎ opens"; a loose tab "Loose · 2 · ⏎ opens"; a pinned tab "Pinned · ⏎ opens".
    pub path: String,
}

/// The answer to one query over every tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    /// The query has at least one term.
    pub active: bool,
    /// Every matching tab id, best first (ties in ring order).
    pub matches: Vec<i32>,
    /// The top two, for the centre.
    pub rows: Vec<SearchRowData>,
    /// How many matches are not in `rows`.
    pub more: usize,
}

/// The score of `doc` for `query`, or `None` when a term does not match. An inactive query matches
/// everything with `Score(0)`.
pub fn matches(_doc: &SearchDoc, _query: &Query) -> Option<Score> {
    None
}

/// Runs `query` over `docs` (given in ring order, pinned tabs last).
pub fn run(_docs: &[SearchDoc], _query: &str) -> SearchResult {
    SearchResult::default()
}

/// Sets `matches_query` of every row: all true for an inactive result, else true for the ids in
/// `result.matches` only.
pub fn set_matches(_tabs: &mut [TabRow], _result: &SearchResult) {}

/// The text of the last row of the centre: `Search the web for “tram”`.
pub fn web_row_text(query: &str) -> String {
    format!("Search the web for “{}”", query.trim())
}

/// Lower-cases and folds the Latin-1 and Latin-Extended-A letters to their base letters (`é` to `e`,
/// `ß` is kept); other scripts are only lower-cased.
pub fn fold(text: &str) -> String {
    text.to_lowercase()
}

/// Moves the selected row of the centre's list (`rows` = results + the web row, so 1 to 3): up or
/// down, wrapping.
pub fn step_row(selected: usize, rows: usize, up: bool) -> usize {
    let _ = (selected, rows, up);
    0
}
