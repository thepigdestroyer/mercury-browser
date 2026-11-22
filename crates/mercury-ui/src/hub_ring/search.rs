// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The ring's search filter (brief D8). Pure: no window, no Slint models.
//!
//! The query is trimmed and split on whitespace; every term must match (AND) as a
//! case-insensitive, diacritic-folded substring of the tab's title, host or group name. An empty
//! query is inactive. Rank: all terms at the start of a title word, then a title substring, then
//! the host, then the group name; ties keep ring order. The centre lists the top two results and a
//! "Search the web for ..." row.

use std::collections::HashSet;

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
            terms: text
                .split_whitespace()
                .map(fold)
                .filter(|term| !term.is_empty())
                .collect(),
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
///
/// Each term gets the best level it reaches (0: starts a title word, 1: inside the title, 2: in the
/// host, 3: in the group name); the tab's score is the worst level of its terms.
pub fn matches(doc: &SearchDoc, query: &Query) -> Option<Score> {
    if !query.is_active() {
        return Some(Score(0));
    }
    let folded = FoldedDoc::new(doc);
    let mut worst = 0;
    for term in &query.terms {
        worst = worst.max(folded.level(term)?);
    }
    Some(Score(worst))
}

/// A document's searchable fields, folded once per query.
struct FoldedDoc {
    title: String,
    host: String,
    group: String,
}

impl FoldedDoc {
    fn new(doc: &SearchDoc) -> Self {
        Self {
            title: fold(&doc.title),
            host: fold(&doc.host),
            group: fold(&doc.group_name),
        }
    }

    /// The best level at which `term` (already folded, not empty) matches, or `None`.
    fn level(&self, term: &str) -> Option<u8> {
        if starts_a_word(&self.title, term) {
            Some(0)
        } else if self.title.contains(term) {
            Some(1)
        } else if self.host.contains(term) {
            Some(2)
        } else if self.group.contains(term) {
            Some(3)
        } else {
            None
        }
    }
}

/// `term` occurs in `text` at the start of the text or right after a character that is not a
/// letter or a digit.
fn starts_a_word(text: &str, term: &str) -> bool {
    text.match_indices(term).any(|(start, _)| {
        text[..start]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric())
    })
}

/// "Lisbon trip · ⌥S 3 · ⏎ opens"; without a key "Loose · 2 · ⏎ opens"; pinned "Pinned · ⏎ opens".
fn path_text(doc: &SearchDoc) -> String {
    if doc.pinned {
        "Pinned · ⏎ opens".to_owned()
    } else if doc.key_label.is_empty() {
        format!("{} · {} · ⏎ opens", doc.group_name, doc.index_in_group)
    } else {
        format!(
            "{} · ⌥{} {} · ⏎ opens",
            doc.group_name, doc.key_label, doc.index_in_group
        )
    }
}

/// Runs `query` over `docs` (given in ring order, pinned tabs last).
///
/// An inactive query matches every tab in the order of `docs` and has no result rows (the centre
/// lists results only while searching).
pub fn run(docs: &[SearchDoc], query: &str) -> SearchResult {
    let query = Query::parse(query);
    if !query.is_active() {
        return SearchResult {
            active: false,
            matches: docs.iter().map(|doc| doc.tab).collect(),
            rows: Vec::new(),
            more: 0,
        };
    }
    let mut hits: Vec<(Score, &SearchDoc)> = docs
        .iter()
        .filter_map(|doc| matches(doc, &query).map(|score| (score, doc)))
        .collect();
    // A stable sort: equal scores keep the order of `docs`.
    hits.sort_by_key(|(score, _)| *score);
    let rows = hits
        .iter()
        .take(2)
        .map(|(_, doc)| SearchRowData {
            tab: doc.tab,
            title: doc.title.clone(),
            path: path_text(doc),
        })
        .collect();
    SearchResult {
        active: true,
        rows,
        more: hits.len().saturating_sub(2),
        matches: hits.iter().map(|(_, doc)| doc.tab).collect(),
    }
}

/// Sets `matches_query` of every row: all true for an inactive result, else true for the ids in
/// `result.matches` only.
pub fn set_matches(tabs: &mut [TabRow], result: &SearchResult) {
    if !result.active {
        for tab in tabs {
            tab.matches_query = true;
        }
        return;
    }
    let lit: HashSet<i32> = result.matches.iter().copied().collect();
    for tab in tabs {
        tab.matches_query = lit.contains(&tab.id);
    }
}

/// The text of the last row of the centre: `Search the web for “tram”`.
pub fn web_row_text(query: &str) -> String {
    format!("Search the web for “{}”", query.trim())
}

/// Lower-cases and folds the Latin-1 and Latin-Extended-A letters to their base letters (`é` to `e`,
/// `ß` is kept); other scripts are only lower-cased.
///
/// Combining marks (U+0300 to U+036F) are dropped, so a decomposed `é` folds like the composed one.
/// A few letters fold to two (`æ` to `ae`, `œ` to `oe`, `þ` to `th`, `ĳ` to `ij`).
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        fold_char(c, &mut out);
    }
    out
}

/// Pushes the folded form of the already lower-cased `c`.
fn fold_char(c: char, out: &mut String) {
    let base: &str = match c {
        '\u{300}'..='\u{36f}' => "",
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ð' | 'ď' | 'đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĳ' => "ij",
        'ĵ' => "j",
        'ķ' | 'ĸ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' | 'ŋ' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ſ' => "s",
        'þ' => "th",
        'ţ' | 'ť' | 'ŧ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        other => {
            out.push(other);
            return;
        }
    };
    out.push_str(base);
}

/// Moves the selected row of the centre's list (`rows` = results + the web row, so 1 to 3): up or
/// down, wrapping. A selection past the end counts as the last row; no rows gives 0.
pub fn step_row(selected: usize, rows: usize, up: bool) -> usize {
    if rows == 0 {
        return 0;
    }
    let selected = selected.min(rows - 1);
    if up {
        (selected + rows - 1) % rows
    } else {
        (selected + 1) % rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_latin_1_letter_folds_to_ascii_except_sharp_s() {
        for code in 0xC0..=0xFF_u32 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            if c == '×' || c == '÷' || c == 'ß' {
                continue;
            }
            let folded = fold(&c.to_string());
            assert!(
                !folded.is_empty() && folded.is_ascii(),
                "{c} U+{code:04X} folded to {folded:?}"
            );
        }
        assert_eq!(fold("ß"), "ß");
        assert_eq!(fold("×÷"), "×÷");
    }

    #[test]
    fn every_latin_extended_a_letter_folds_to_ascii() {
        for code in 0x100..=0x17F_u32 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            let folded = fold(&c.to_string());
            assert!(
                !folded.is_empty() && folded.is_ascii(),
                "{c} U+{code:04X} folded to {folded:?}"
            );
        }
    }

    #[test]
    fn fold_is_idempotent() {
        for text in [
            "Ünïcödé Straße Łódź İstanbul",
            "plain ascii 28",
            "日本語 Αθήνα",
        ] {
            assert_eq!(fold(&fold(text)), fold(text), "{text}");
        }
    }

    #[test]
    fn a_word_starts_at_the_front_and_after_a_non_alphanumeric() {
        assert!(starts_a_word("tram 28", "tram"));
        assert!(starts_a_word("the tram", "tram"));
        assert!(starts_a_word("a-tram", "tram"));
        assert!(
            starts_a_word("xtram tram", "tram"),
            "a later occurrence counts"
        );
        assert!(!starts_a_word("xtram", "tram"));
        assert!(!starts_a_word("tram", "ram"));
        assert!(!starts_a_word("", "tram"));
    }

    #[test]
    fn the_path_of_a_pinned_tab_ignores_its_group() {
        let doc = SearchDoc {
            tab: 1,
            title: "Mail".into(),
            host: "mail.example.org".into(),
            group_id: -2,
            group_name: "Loose".into(),
            key_label: String::new(),
            index_in_group: 0,
            pinned: true,
        };
        assert_eq!(path_text(&doc), "Pinned · ⏎ opens");
    }
}
