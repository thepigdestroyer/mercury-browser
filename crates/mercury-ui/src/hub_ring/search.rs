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
//! # What [`fold`] folds
//!
//! Text is lower-cased, then (a hand-written table, no dependency):
//!
//! - **Latin**: Latin-1 and Latin Extended-A letters (`é` to `e`, `ł` to `l`, `ø` to `o`), the
//!   Latin Extended-B letters of Romanian and pinyin (`ș ț ǎ ǐ ǒ ǔ ǖ ǘ ǚ ǜ ǹ`) and of Vietnamese
//!   (`ơ ư`), and all of Latin Extended Additional U+1E00 to U+1EFF (Vietnamese `ệ ạ ố`, `ḥ ṣ`).
//!   `æ œ þ ĳ` fold to two letters (`ae oe th ij`) and `ß`/`ẞ` to `ss`, in the query and in the
//!   haystack alike, so `strasse` and `straße` find each other.
//! - **Greek**: the tonos and dialytika vowels (`ά έ ή ί ό ύ ώ ΐ ΰ ϊ ϋ`) fold to the plain vowel and the
//!   final sigma `ς` to `σ`, so `ΟΔΟΣ`, `οδος` and `οδοσ` are one word.
//! - **Combining marks** U+0300 to U+036F are dropped, so a decomposed `é` folds like `é`
//!   (and Turkish `İ`, which lower-cases to `i` plus U+0307, becomes `i`).
//!
//! **Not folded**: every other script matches exactly (Cyrillic `й` is not `и`; no
//! transliteration), polytonic Greek (U+1F00 to U+1FFF) is only lower-cased, and CJK is exact
//! (a decomposed dakuten U+3099 or U+309A is not composed, `ガ` is not `カ`).

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

/// Lower-cases and folds accented Latin and Greek letters to their base letters (`é` to `e`, `ß` to
/// `ss`, `ά` to `α`, `ς` to `σ`; the module doc lists the coverage); other scripts are only
/// lower-cased. Folding twice gives the same text as folding once.
///
/// Combining marks (U+0300 to U+036F) are dropped, so a decomposed `é` folds like the composed one.
/// A few letters fold to two (`æ` to `ae`, `œ` to `oe`, `þ` to `th`, `ĳ` to `ij`, `ß` to `ss`).
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
        'ß' => "ss",
        // Latin Extended-B: pinyin, Romanian, Vietnamese horn letters. Uppercase forms are
        // lower-cased before they get here, so only the lowercase letters are listed.
        'ǎ' => "a",
        'ǐ' => "i",
        'ǒ' | 'ơ' => "o",
        '\u{1d3}'..='\u{1dc}' | 'ư' => "u", // ǔ ǖ ǘ ǚ ǜ (ǔ is U+01D4)
        'ǹ' => "n",
        'ș' => "s",
        'ț' => "t",
        // Latin Extended Additional. Every uppercase letter in the block has a lowercase partner,
        // so each range stands for the lowercase letters in it.
        '\u{1e00}'..='\u{1e01}' | '\u{1e9a}' | '\u{1ea0}'..='\u{1eb7}' => "a",
        '\u{1e02}'..='\u{1e07}' => "b",
        '\u{1e08}'..='\u{1e09}' => "c",
        '\u{1e0a}'..='\u{1e13}' => "d",
        '\u{1e14}'..='\u{1e1d}' | '\u{1eb8}'..='\u{1ec7}' => "e",
        '\u{1e1e}'..='\u{1e1f}' => "f",
        '\u{1e20}'..='\u{1e21}' => "g",
        '\u{1e22}'..='\u{1e2b}' | '\u{1e96}' => "h",
        '\u{1e2c}'..='\u{1e2f}' | '\u{1ec8}'..='\u{1ecb}' => "i",
        '\u{1e30}'..='\u{1e35}' => "k",
        '\u{1e36}'..='\u{1e3d}' | '\u{1efa}'..='\u{1efb}' => "l",
        '\u{1e3e}'..='\u{1e43}' => "m",
        '\u{1e44}'..='\u{1e4b}' => "n",
        '\u{1e4c}'..='\u{1e53}' | '\u{1ecc}'..='\u{1ee3}' => "o",
        '\u{1e54}'..='\u{1e57}' => "p",
        '\u{1e58}'..='\u{1e5f}' => "r",
        '\u{1e60}'..='\u{1e69}' | '\u{1e9b}'..='\u{1e9d}' => "s",
        '\u{1e6a}'..='\u{1e71}' | '\u{1e97}' => "t",
        '\u{1e72}'..='\u{1e7b}' | '\u{1ee4}'..='\u{1ef1}' => "u",
        '\u{1e7c}'..='\u{1e7f}' | '\u{1efc}'..='\u{1efd}' => "v",
        '\u{1e80}'..='\u{1e89}' | '\u{1e98}' => "w",
        '\u{1e8a}'..='\u{1e8d}' => "x",
        '\u{1e8e}'..='\u{1e8f}'
        | '\u{1e99}'
        | '\u{1ef2}'..='\u{1ef9}'
        | '\u{1efe}'..='\u{1eff}' => "y",
        '\u{1e90}'..='\u{1e95}' => "z",
        // Greek: tonos and dialytika vowels to the plain vowel, final sigma to sigma.
        'ά' => "α",
        'έ' => "ε",
        'ή' => "η",
        'ί' | 'ΐ' | 'ϊ' => "ι",
        'ό' => "ο",
        'ύ' | 'ΰ' | 'ϋ' => "υ",
        'ώ' => "ω",
        'ς' => "σ",
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
    fn every_latin_1_letter_folds_to_ascii() {
        for code in 0xC0..=0xFF_u32 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            if c == '×' || c == '÷' {
                continue;
            }
            let folded = fold(&c.to_string());
            assert!(
                !folded.is_empty() && folded.is_ascii(),
                "{c} U+{code:04X} folded to {folded:?}"
            );
        }
        assert_eq!(fold("ß"), "ss");
        assert_eq!(fold("ẞ"), "ss");
        assert_eq!(fold("×÷"), "×÷");
    }

    #[test]
    fn every_latin_extended_additional_letter_folds_to_ascii() {
        // U+1E9F (Greek delta with a stroke) is the one non-Latin letter of the block.
        for code in (0x1E00..=0x1EFF_u32).filter(|&code| code != 0x1E9F) {
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
    fn the_extended_b_letters_of_romanian_pinyin_and_vietnamese_fold_to_ascii() {
        for (text, base) in [
            ("ș Ș", "s s"),
            ("ț Ț", "t t"),
            ("ǎ Ǎ", "a a"),
            ("ǐ Ǐ", "i i"),
            ("ǒ Ǒ", "o o"),
            ("ǔ Ǔ ǖ Ǖ ǘ Ǘ ǚ Ǚ ǜ Ǜ", "u u u u u u u u u u"),
            ("ǹ Ǹ", "n n"),
            ("ơ Ơ", "o o"),
            ("ư Ư", "u u"),
            ("đ Đ ø Ø ł Ł", "d d o o l l"),
        ] {
            assert_eq!(fold(text), base, "{text}");
        }
    }

    #[test]
    fn every_greek_letter_folds_to_a_plain_lowercase_letter() {
        for code in 0x386..=0x3CE_u32 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            if !c.is_alphabetic() {
                continue;
            }
            let folded: Vec<char> = fold(&c.to_string()).chars().collect();
            assert!(
                matches!(folded.as_slice(), [b] if ('α'..='ω').contains(b) && *b != 'ς'),
                "{c} U+{code:04X} folded to {folded:?}"
            );
        }
        assert_eq!(fold("ΟΔΟΣ"), "οδοσ");
        assert_eq!(fold("οδος"), "οδοσ");
        assert_eq!(fold("ΆΈΉΊΌΎΏ ΐΰ"), "αεηιουω ιυ");
    }

    #[test]
    fn folding_every_char_of_the_tables_is_idempotent() {
        for code in 0..=0x2000_u32 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            let once = fold(&c.to_string());
            assert_eq!(fold(&once), once, "{c} U+{code:04X}");
        }
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
            "日本語 Αθήνα ΟΔΟΣ Việt Nam Nǐ hǎo România ẞ",
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
