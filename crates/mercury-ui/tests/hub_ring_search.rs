// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The hub ring's search filter and the centre disc's strings (lane hub-ring item 1; brief D8, D10,
//! D13), without a window. The documents come from the 22-tab mockup scene plus its three pinned
//! tabs; the strings are the ones the four `ring-*` mockups draw.

use std::time::{Duration, Instant};

use mercury_ui::CentreMode;
use mercury_ui::hub_ring::centre::{self, CentreFocus, CentreInput, HeldGroup};
use mercury_ui::hub_ring::fixtures::mockup_input;
use mercury_ui::hub_ring::search::{
    Query, Score, SearchDoc, SearchResult, fold, matches, run, set_matches, step_row, web_row_text,
};
use mercury_ui::hub_ring::{HubRingInput, TabMedia};
use mercury_ui::ring_bands::fixtures::generated;

// ---- helpers ---------------------------------------------------------------------------------

/// The filter's documents for the mockup scene, in ring order (pinned tabs last).
fn mockup_docs() -> Vec<SearchDoc> {
    docs_from(&mockup_input(false))
}

fn docs_from(input: &HubRingInput) -> Vec<SearchDoc> {
    input
        .tabs
        .iter()
        .map(|tab| {
            let group = input.groups.iter().find(|group| group.id == tab.group);
            SearchDoc {
                tab: tab.id,
                title: tab.title.to_string(),
                host: tab.host.to_string(),
                group_id: tab.group,
                group_name: group.map(|g| g.name.to_string()).unwrap_or_default(),
                key_label: group.map(|g| g.key_label.to_string()).unwrap_or_default(),
                index_in_group: if tab.pinned {
                    0
                } else {
                    tab.index_in_group as usize + 1
                },
                pinned: tab.pinned,
            }
        })
        .collect()
}

fn doc(tab: i32, title: &str, host: &str, group: &str) -> SearchDoc {
    SearchDoc {
        tab,
        title: title.to_owned(),
        host: host.to_owned(),
        group_id: 1,
        group_name: group.to_owned(),
        key_label: "Q".to_owned(),
        index_in_group: 1,
        pinned: false,
    }
}

fn id_of(docs: &[SearchDoc], title: &str) -> i32 {
    docs.iter()
        .find(|doc| doc.title == title)
        .unwrap_or_else(|| panic!("no tab titled {title}"))
        .tab
}

fn doc_of<'a>(docs: &'a [SearchDoc], title: &str) -> &'a SearchDoc {
    docs.iter().find(|doc| doc.title == title).expect("title")
}

fn titles(docs: &[SearchDoc], result: &SearchResult) -> Vec<String> {
    result
        .matches
        .iter()
        .map(|id| {
            docs.iter()
                .find(|doc| doc.tab == *id)
                .expect("a matched id is a document")
                .title
                .clone()
        })
        .collect()
}

fn score_of(doc: &SearchDoc, query: &str) -> Option<Score> {
    matches(doc, &Query::parse(query))
}

// ---- search: the query -----------------------------------------------------------------------

#[test]
fn parse_trims_splits_on_any_whitespace_and_folds() {
    let query = Query::parse("  Café \t TRAM\n28  ");
    assert_eq!(query.terms, ["cafe", "tram", "28"]);
    assert_eq!(query.text, "Café \t TRAM\n28");
    assert!(query.is_active());
}

#[test]
fn empty_and_blank_queries_are_inactive_and_match_every_tab() {
    let docs = mockup_docs();
    assert_eq!(docs.len(), 25);
    let everything: Vec<i32> = docs.iter().map(|doc| doc.tab).collect();
    for text in ["", " ", "  \t\n "] {
        assert!(!Query::parse(text).is_active(), "{text:?}");
        let result = run(&docs, text);
        assert!(!result.active, "{text:?}");
        assert_eq!(
            result.matches, everything,
            "{text:?} matches all in ring order"
        );
        assert_eq!(result.more, 0, "{text:?}");
        assert!(result.rows.is_empty(), "no result rows without a query");
        for doc in &docs {
            assert_eq!(
                score_of(doc, text),
                Some(Score(0)),
                "{text:?} {}",
                doc.title
            );
        }
    }
}

// ---- search: case and diacritics -------------------------------------------------------------

#[test]
fn case_does_not_matter_in_the_query_or_the_title() {
    let docs = mockup_docs();
    let essay = id_of(&docs, "Essay on maps");
    for text in ["essay", "ESSAY", "EsSaY on MAPS"] {
        let result = run(&docs, text);
        assert_eq!(result.matches, [essay], "{text}");
    }
    let shouting = [doc(1, "BIG NEWS", "news.example.org", "G")];
    assert_eq!(run(&shouting, "big news").matches, [1]);
}

#[test]
fn diacritics_are_folded_on_both_sides() {
    let docs = [
        doc(1, "Café", "a.example.org", "G"),
        doc(2, "école", "b.example.org", "G"),
        doc(3, "Cafe del Mar", "c.example.org", "G"),
        doc(4, "Ångström units", "d.example.org", "G"),
        doc(5, "Łódź guide", "e.example.org", "G"),
        doc(6, "Dvořák: New World", "f.example.org", "G"),
        doc(7, "Søren and Œuvre", "g.example.org", "G"),
    ];
    assert_eq!(run(&docs, "cafe").matches, [1, 3]);
    assert_eq!(run(&docs, "CAFÉ").matches, [1, 3]);
    assert_eq!(run(&docs, "ÉCOLE").matches, [2]);
    assert_eq!(run(&docs, "ecole").matches, [2]);
    assert_eq!(run(&docs, "angstrom").matches, [4]);
    assert_eq!(run(&docs, "lodz").matches, [5]);
    assert_eq!(run(&docs, "dvorak").matches, [6]);
    assert_eq!(run(&docs, "soren").matches, [7]);
    assert_eq!(run(&docs, "oeuvre").matches, [7]);
}

#[test]
fn a_decomposed_accent_matches_like_the_composed_letter() {
    let docs = [doc(1, "Cafe\u{301} Lisboa", "a.example.org", "G")];
    assert_eq!(run(&docs, "cafe").matches, [1]);
    assert_eq!(run(&docs, "café").matches, [1]);
}

#[test]
fn sharp_s_is_kept_not_folded_to_ss() {
    let docs = [
        doc(1, "Straße nach Wien", "a.example.org", "G"),
        doc(2, "Strasse nach Bern", "b.example.org", "G"),
    ];
    assert_eq!(run(&docs, "straße").matches, [1]);
    assert_eq!(
        run(&docs, "STRAẞE").matches,
        [1],
        "capital sharp s lower-cases to ß"
    );
    assert_eq!(run(&docs, "strasse").matches, [2]);
}

#[test]
fn non_latin_scripts_match_exactly() {
    let docs = [
        doc(1, "日本語のニュース", "a.example.jp", "G"),
        doc(2, "Αθήνα guide", "b.example.gr", "G"),
        doc(3, "Новости дня", "c.example.ru", "G"),
    ];
    assert_eq!(run(&docs, "ニュース").matches, [1]);
    assert_eq!(run(&docs, "日本").matches, [1]);
    assert_eq!(run(&docs, "ΑΘΉΝΑ").matches, [2], "case is folded");
    assert!(
        run(&docs, "αθηνα").matches.is_empty(),
        "the Greek accent is not folded"
    );
    assert_eq!(run(&docs, "НОВОСТИ").matches, [3]);
    assert!(
        run(&docs, "novosti").matches.is_empty(),
        "no transliteration"
    );
}

#[test]
fn fold_keeps_the_text_otherwise_and_lowercases() {
    assert_eq!(fold("Café ÉCOLE Straße"), "cafe ecole straße");
    assert_eq!(fold("Tram 28"), "tram 28");
    assert_eq!(fold("日本語"), "日本語");
    assert_eq!(fold(""), "");
}

// ---- search: terms and fields ----------------------------------------------------------------

#[test]
fn every_term_must_match_in_any_order_and_across_fields() {
    let docs = mockup_docs();
    let tram = id_of(&docs, "Tram 28 timetable");
    // a title word and the group name
    assert_eq!(run(&docs, "lisbon tram").matches, [tram]);
    assert_eq!(run(&docs, "tram lisbon").matches, [tram]);
    assert_eq!(run(&docs, "  TRAM   28 lisbon ").matches, [tram]);
    // a title word and the host
    let essay = id_of(&docs, "Essay on maps");
    assert_eq!(run(&docs, "maps essays").matches, [essay]);
    // one term that matches nowhere kills the match
    assert!(run(&docs, "tram stays").matches.is_empty());
    assert!(run(&docs, "tram zzzz").matches.is_empty());
}

#[test]
fn a_term_matches_inside_a_word() {
    let docs = mockup_docs();
    let result = run(&docs, "ring");
    // "The ring road", "Slint rendering" (host and title), "Pasteis de Belem"? no: only "ring" substrings.
    let found = titles(&docs, &result);
    assert!(found.contains(&"The ring road, a history".to_owned()));
    assert!(found.contains(&"Slint rendering".to_owned()));
}

#[test]
fn the_host_is_searched() {
    let docs = mockup_docs();
    let essay = doc_of(&docs, "Essay on maps");
    assert_eq!(essay.host, "essays.example.net");
    let result = run(&docs, "essays");
    assert_eq!(result.matches, [essay.tab]);
    assert_eq!(score_of(essay, "essays"), Some(Score(2)));
    assert_eq!(
        run(&docs, "example.net").matches.len(),
        6,
        "forum, essays, notes, stories, longform, recipes"
    );
}

#[test]
fn the_group_name_is_searched() {
    let docs = mockup_docs();
    let result = run(&docs, "shopping");
    assert_eq!(
        titles(&docs, &result),
        ["Running shoes", "Desk lamp", "Bookshelf"],
        "all three tabs of the group, in ring order"
    );
    for doc in docs.iter().filter(|doc| doc.group_name == "Shopping") {
        assert_eq!(score_of(doc, "shopping"), Some(Score(3)));
    }
}

#[test]
fn the_key_letter_and_the_path_are_not_searched() {
    let docs = mockup_docs();
    assert!(run(&docs, "opens").matches.is_empty());
    assert!(run(&docs, "⌥s").matches.is_empty());
    assert!(run(&docs, "·").matches.is_empty());
}

// ---- search: ranking -------------------------------------------------------------------------

#[test]
fn word_start_ranks_before_substring_before_host_before_group() {
    // ring order: group-name hit, host hit, substring hit, word-start hit
    let docs = [
        doc(10, "Zeta", "alpha.example.org", "Highway"),
        doc(11, "Zulu", "way.example.org", "Other"),
        doc(12, "Subway map", "other.example.org", "Other"),
        doc(13, "Way home", "other.example.org", "Other"),
    ];
    let result = run(&docs, "way");
    assert_eq!(result.matches, [13, 12, 11, 10]);
    assert_eq!(score_of(&docs[3], "way"), Some(Score(0)));
    assert_eq!(score_of(&docs[2], "way"), Some(Score(1)));
    assert_eq!(score_of(&docs[1], "way"), Some(Score(2)));
    assert_eq!(score_of(&docs[0], "way"), Some(Score(3)));
}

#[test]
fn a_word_start_follows_any_non_letter() {
    let docs = [
        doc(1, "Notes (tram) and more", "a.example.org", "G"),
        doc(2, "Metro-Tram 28", "b.example.org", "G"),
        doc(3, "Xtram", "c.example.org", "G"),
    ];
    assert_eq!(score_of(&docs[0], "tram"), Some(Score(0)));
    assert_eq!(score_of(&docs[1], "tram"), Some(Score(0)));
    assert_eq!(score_of(&docs[2], "tram"), Some(Score(1)));
    assert_eq!(run(&docs, "tram").matches, [1, 2, 3]);
}

#[test]
fn a_tab_scores_as_badly_as_its_worst_term() {
    let docs = mockup_docs();
    let tram = doc_of(&docs, "Tram 28 timetable");
    assert_eq!(score_of(tram, "tram"), Some(Score(0)));
    assert_eq!(score_of(tram, "tram 28"), Some(Score(0)));
    assert_eq!(
        score_of(tram, "tram imetab"),
        Some(Score(1)),
        "a substring term"
    );
    assert_eq!(
        score_of(tram, "tram transit"),
        Some(Score(2)),
        "a host term"
    );
    assert_eq!(
        score_of(tram, "tram lisbon"),
        Some(Score(3)),
        "a group term"
    );
    assert_eq!(score_of(tram, "tram nowhere"), None);
}

#[test]
fn ranking_on_the_mockup_scene() {
    let docs = mockup_docs();
    let result = run(&docs, "servo");
    // title word start: "Servo layout notes" (ring order first), "Interview: Servo";
    // host only: "WebRender overview" (servo.example.org), which comes first in ring order
    // but ranks last.
    assert_eq!(
        titles(&docs, &result),
        [
            "Servo layout notes",
            "Interview: Servo",
            "WebRender overview"
        ]
    );
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0].title, "Servo layout notes");
    assert_eq!(result.rows[1].title, "Interview: Servo");
    assert_eq!(result.more, 1);
}

#[test]
fn ties_keep_the_order_of_the_documents_not_the_ids() {
    let docs = [
        doc(50, "Big tram", "a.example.org", "G"),
        doc(7, "Little tram", "b.example.org", "G"),
        doc(30, "Tram stop", "c.example.org", "G"),
        doc(40, "Metro tram", "d.example.org", "G"),
    ];
    // "Tram stop" and "Little tram"/"Big tram"/"Metro tram" are all word starts: one score.
    assert_eq!(run(&docs, "tram").matches, [50, 7, 30, 40]);
    // The same ids in another order come out in that order.
    let reversed: Vec<SearchDoc> = docs.iter().rev().cloned().collect();
    assert_eq!(run(&reversed, "tram").matches, [40, 30, 7, 50]);
}

// ---- search: rows and `more` -----------------------------------------------------------------

#[test]
fn the_centre_gets_the_top_two_and_the_rest_is_counted() {
    let docs = mockup_docs();
    let result = run(&docs, "e");
    assert!(result.active);
    assert!(result.matches.len() > 4);
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0].tab, result.matches[0]);
    assert_eq!(result.rows[1].tab, result.matches[1]);
    assert_eq!(result.more, result.matches.len() - 2);
}

#[test]
fn few_matches_leave_no_more() {
    let docs = mockup_docs();
    let none = run(&docs, "zzzz");
    assert!(none.active);
    assert!(none.matches.is_empty() && none.rows.is_empty());
    assert_eq!(none.more, 0);

    let one = run(&docs, "tram");
    assert_eq!((one.matches.len(), one.rows.len(), one.more), (1, 1, 0));

    // "lamp" and "bookshelf": two tabs
    let docs2 = [
        doc(1, "Alpha site", "a.example.org", "G"),
        doc(2, "Alpha shop", "b.example.org", "G"),
        doc(3, "Beta", "c.example.org", "G"),
    ];
    let two = run(&docs2, "alpha");
    assert_eq!((two.matches.len(), two.rows.len(), two.more), (2, 2, 0));
    let three = run(&docs2, "a");
    assert_eq!(
        (three.matches.len(), three.rows.len(), three.more),
        (3, 2, 1)
    );
}

#[test]
fn a_title_with_the_query_twice_is_one_match() {
    let docs = [
        doc(1, "tram tram tram", "tram.example.org", "Tram group"),
        doc(2, "Other", "b.example.org", "G"),
    ];
    let result = run(&docs, "tram");
    assert_eq!(result.matches, [1]);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.more, 0);
    // the same term twice is not two tabs either
    assert_eq!(run(&docs, "tram tram").matches, [1]);
}

#[test]
fn rows_carry_the_title_and_the_path_text() {
    let docs = mockup_docs();
    let result = run(&docs, "tram");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].tab, id_of(&docs, "Tram 28 timetable"));
    assert_eq!(result.rows[0].title, "Tram 28 timetable");
    assert_eq!(result.rows[0].path, "Lisbon trip · ⌥S 3 · ⏎ opens");
}

#[test]
fn the_path_without_a_key_is_the_group_and_the_index() {
    let mut loose = doc(1, "Weather", "weather.example.org", "Loose");
    loose.key_label.clear();
    loose.index_in_group = 2;
    let result = run(&[loose], "weather");
    assert_eq!(result.rows[0].path, "Loose · 2 · ⏎ opens");
}

#[test]
fn a_pinned_tab_matches_and_is_named_pinned() {
    let docs = mockup_docs();
    let mail = doc_of(&docs, "Mail");
    assert!(mail.pinned);
    let result = run(&docs, "mail");
    assert_eq!(result.matches, [mail.tab]);
    assert_eq!(result.rows[0].title, "Mail");
    assert_eq!(result.rows[0].path, "Pinned · ⏎ opens");
    // by host as well
    assert_eq!(
        run(&docs, "calendar.example").matches,
        [doc_of(&docs, "Calendar").tab]
    );
    // a pinned tab is also found next to ring tabs, and ranks after an equal ring tab
    let both = run(&docs, "maps");
    let found = titles(&docs, &both);
    assert!(found.contains(&"Maps".to_owned()));
    assert!(found.contains(&"Essay on maps".to_owned()));
}

#[test]
fn no_match_in_a_query_of_only_punctuation() {
    let docs = mockup_docs();
    for text in ["?!", "«»", "\"\""] {
        let result = run(&docs, text);
        assert!(result.active, "{text}");
        assert!(result.matches.is_empty(), "{text}");
    }
}

// ---- search: speed ---------------------------------------------------------------------------

#[test]
fn a_long_query_over_five_hundred_tabs_is_fast() {
    let (groups, tabs) = generated(10, 500, false);
    let docs = docs_from(&HubRingInput {
        groups,
        tabs,
        ..HubRingInput::default()
    });
    assert_eq!(docs.len(), 500);
    let long_token = "a".repeat(200);
    let many_terms = "tab of group ".repeat(16)[..200].to_owned();
    assert_eq!(many_terms.chars().count(), 200);
    for (query, expect_some) in [(long_token.as_str(), false), (many_terms.trim(), true)] {
        // best of five: a loaded machine can stall one run
        let mut best = Duration::MAX;
        let mut result = SearchResult::default();
        for _ in 0..5 {
            let start = Instant::now();
            result = run(&docs, query);
            best = best.min(start.elapsed());
        }
        assert_eq!(!result.matches.is_empty(), expect_some, "{query}");
        assert!(best < Duration::from_millis(20), "{best:?} for {query:?}");
    }
}

// ---- search: marking the rows ----------------------------------------------------------------

#[test]
fn set_matches_marks_exactly_the_matching_rows() {
    let input = mockup_input(false);
    let docs = docs_from(&input);
    let mut tabs = input.tabs.clone();
    let tram = id_of(&docs, "Tram 28 timetable");

    set_matches(&mut tabs, &run(&docs, "tram"));
    for tab in &tabs {
        assert_eq!(tab.matches_query, tab.id == tram, "{}", tab.title);
    }

    // another query overwrites the earlier marks, pinned rows included
    set_matches(&mut tabs, &run(&docs, "mail"));
    let lit: Vec<&str> = tabs
        .iter()
        .filter(|tab| tab.matches_query)
        .map(|tab| tab.title.as_str())
        .collect();
    assert_eq!(lit, ["Mail"]);
    assert!(
        tabs.iter()
            .find(|tab| tab.title == "Mail")
            .expect("mail")
            .pinned
    );

    // no match: everything fades
    set_matches(&mut tabs, &run(&docs, "zzzz"));
    assert!(tabs.iter().all(|tab| !tab.matches_query));

    // an inactive query lights everything again
    set_matches(&mut tabs, &run(&docs, "  "));
    assert!(tabs.iter().all(|tab| tab.matches_query));
}

// ---- search: the web row and the selection ---------------------------------------------------

#[test]
fn the_web_row_quotes_the_trimmed_query() {
    assert_eq!(web_row_text("tram"), "Search the web for “tram”");
    assert_eq!(web_row_text("  tram 28 \t"), "Search the web for “tram 28”");
    assert_eq!(web_row_text("?!"), "Search the web for “?!”");
    assert_eq!(
        web_row_text("say \"hi\""),
        "Search the web for “say \"hi\"”"
    );
}

#[test]
fn step_row_wraps_through_the_rows() {
    // three rows: two results and the web row
    assert_eq!(step_row(0, 3, false), 1);
    assert_eq!(step_row(1, 3, false), 2);
    assert_eq!(step_row(2, 3, false), 0, "down wraps to the top");
    assert_eq!(step_row(2, 3, true), 1);
    assert_eq!(step_row(1, 3, true), 0);
    assert_eq!(step_row(0, 3, true), 2, "up wraps to the web row");
    // two rows
    assert_eq!(step_row(0, 2, false), 1);
    assert_eq!(step_row(1, 2, false), 0);
    assert_eq!(step_row(0, 2, true), 1);
    // the web row alone
    assert_eq!(step_row(0, 1, false), 0);
    assert_eq!(step_row(0, 1, true), 0);
    // a stale selection is brought back into range
    assert_eq!(step_row(5, 3, false), 0);
    assert!(step_row(5, 3, true) < 3);
    assert_eq!(step_row(0, 0, false), 0);
    assert_eq!(step_row(0, 0, true), 0);
}

// ---- centre ----------------------------------------------------------------------------------

fn focus_on(title: &str) -> CentreFocus {
    let docs = mockup_docs();
    let input = mockup_input(false);
    let doc = doc_of(&docs, title);
    CentreFocus {
        tab: doc.tab,
        title: doc.title.clone(),
        host: doc.host.clone(),
        group_name: doc.group_name.clone(),
        key_label: doc.key_label.clone(),
        index_in_group: doc.index_in_group,
        pinned: doc.pinned,
        media: input
            .media
            .iter()
            .copied()
            .find(|media| media.tab == doc.tab),
    }
}

fn rest_input(title: &str) -> CentreInput {
    CentreInput {
        total_tabs: 22,
        focus: Some(focus_on(title)),
        ..CentreInput::default()
    }
}

fn playing(position: Option<(u32, u32)>) -> TabMedia {
    TabMedia {
        tab: 115,
        playing: true,
        can_pause: true,
        position,
        ..TabMedia::default()
    }
}

fn audio_input(media: TabMedia) -> CentreInput {
    let mut input = rest_input("Interview: Servo");
    input.focus.as_mut().expect("focus").media = Some(media);
    input
}

#[test]
fn rest_shows_the_title_the_path_and_the_hint() {
    let view = centre::view(&rest_input("Essay on maps"));
    assert_eq!(view.mode, CentreMode::Rest);
    assert_eq!(view.title, "Essay on maps");
    assert_eq!(view.subtitle, "Reading · ⌥D 2 · essays.example.net");
    assert_eq!(view.field_placeholder, "Search 22 tabs");
    assert_eq!(view.footer, "Hold ⌥ to see group keys");
    assert!(view.held_key.is_empty() && view.held_name.is_empty() && view.held_help.is_empty());
    assert!(view.play_state.is_empty());
    assert!(!view.can_pause && !view.paused && !view.muted);
}

#[test]
fn the_subtitle_of_a_loose_and_of_a_pinned_tab() {
    let mut input = rest_input("Weather");
    let focus = input.focus.as_mut().expect("focus");
    focus.key_label.clear();
    focus.group_name = "Loose".to_owned();
    focus.index_in_group = 1;
    assert_eq!(centre::view(&input).subtitle, "Loose · weather.example.org");

    let pinned = centre::view(&rest_input("Mail"));
    assert_eq!(pinned.title, "Mail");
    assert_eq!(pinned.subtitle, "Pinned · mail.example.org");
}

#[test]
fn the_placeholder_counts_every_tab() {
    for (total, text) in [
        (22, "Search 22 tabs"),
        (25, "Search 25 tabs"),
        (2, "Search 2 tabs"),
        (1, "Search 1 tab"),
        (0, "Search tabs"),
    ] {
        let mut input = rest_input("Essay on maps");
        input.total_tabs = total;
        assert_eq!(centre::view(&input).field_placeholder, text, "{total}");
    }
}

#[test]
fn searching_has_its_own_footer() {
    let mut input = rest_input("Essay on maps");
    input.query_active = true;
    let view = centre::view(&input);
    assert_eq!(view.mode, CentreMode::Searching);
    assert_eq!(view.footer, "↑ ↓ choose · esc clears");
    assert_eq!(view.field_placeholder, "Search 22 tabs");

    input.more_matches = 3;
    assert_eq!(centre::view(&input).footer, "+3 more lit in the ring");
    input.more_matches = 1;
    assert_eq!(centre::view(&input).footer, "+1 more lit in the ring");

    // a query without any tab in focus is still the searching state
    input.focus = None;
    input.more_matches = 0;
    assert_eq!(centre::view(&input).mode, CentreMode::Searching);
}

#[test]
fn held_without_a_group_asks_for_a_group_key() {
    let mut input = rest_input("Essay on maps");
    input.held = true;
    let view = centre::view(&input);
    assert_eq!(view.mode, CentreMode::Held);
    assert_eq!(view.held_key, "⌥");
    assert_eq!(view.held_name, "");
    assert_eq!(view.held_help, "press a group key");
    assert_eq!(view.footer, "");
}

#[test]
fn held_with_a_group_names_it_and_the_digits() {
    let mut input = rest_input("Essay on maps");
    input.held = true;
    input.held_group = Some(HeldGroup {
        name: "Lisbon trip".to_owned(),
        key_label: "S".to_owned(),
        tab_count: 4,
    });
    let view = centre::view(&input);
    assert_eq!(view.mode, CentreMode::Held);
    assert_eq!(view.held_key, "⌥ S");
    assert_eq!(view.held_name, "Lisbon trip");
    assert_eq!(view.held_help, "press 1–4 for a tab");
    assert!(
        view.held_help.contains('\u{2013}'),
        "an en dash, not a hyphen"
    );
    assert_eq!(view.footer, "");

    for (count, help) in [
        (1, "press 1 for a tab"),
        (2, "press 1–2 for a tab"),
        (9, "press 1–9 for a tab"),
        (10, "press 1–9 for a tab"),
        (60, "press 1–9 for a tab"),
    ] {
        let group = input.held_group.as_mut().expect("group");
        group.tab_count = count;
        assert_eq!(centre::view(&input).held_help, help, "{count}");
    }
}

#[test]
fn held_wins_over_searching_and_over_audio() {
    let mut input = audio_input(playing(None));
    input.query_active = true;
    input.held = true;
    assert_eq!(centre::view(&input).mode, CentreMode::Held);
    input.held = false;
    assert_eq!(centre::view(&input).mode, CentreMode::Searching);
    input.query_active = false;
    assert_eq!(centre::view(&input).mode, CentreMode::Audio);
}

#[test]
fn audio_with_a_position_reads_like_the_mockup() {
    let view = centre::view(&audio_input(playing(Some((760, 2880)))));
    assert_eq!(view.mode, CentreMode::Audio);
    assert_eq!(view.title, "Interview: Servo");
    assert_eq!(view.subtitle, "Reading · ⌥D 6 · podcasts.example.org");
    assert_eq!(view.play_state, "Playing · 12:40 of 48:00");
    assert!(view.can_pause);
    assert!(!view.paused && !view.muted);
}

#[test]
fn the_mockup_fixtures_audio_tab_gives_the_same_state() {
    let input = rest_input("Interview: Servo");
    let media = input.focus.as_ref().expect("focus").media.expect("media");
    assert_eq!(media.position, Some((760, 2880)));
    let view = centre::view(&input);
    assert_eq!(view.mode, CentreMode::Audio);
    assert_eq!(view.play_state, "Playing · 12:40 of 48:00");
}

#[test]
fn audio_without_a_position_says_only_playing() {
    let view = centre::view(&audio_input(playing(None)));
    assert_eq!(view.mode, CentreMode::Audio);
    assert_eq!(view.play_state, "Playing");
}

#[test]
fn audio_times_over_an_hour_have_hours() {
    let view = centre::view(&audio_input(playing(Some((3725, 7322)))));
    assert_eq!(view.play_state, "Playing · 1:02:05 of 2:02:02");
    let view = centre::view(&audio_input(playing(Some((59, 3600)))));
    assert_eq!(view.play_state, "Playing · 0:59 of 1:00:00");
    let view = centre::view(&audio_input(playing(Some((0, 5)))));
    assert_eq!(view.play_state, "Playing · 0:00 of 0:05");
}

#[test]
fn audio_paused_and_without_a_pause_command() {
    let paused = TabMedia {
        playing: false,
        paused: true,
        ..playing(Some((760, 2880)))
    };
    let view = centre::view(&audio_input(paused));
    assert_eq!(view.mode, CentreMode::Audio);
    assert_eq!(view.play_state, "Paused");
    assert!(view.paused);
    assert!(view.can_pause);

    let no_pause = TabMedia {
        can_pause: false,
        ..playing(None)
    };
    let view = centre::view(&audio_input(no_pause));
    assert_eq!(view.play_state, "Playing");
    assert!(!view.can_pause);
}

#[test]
fn audio_muted_wins_over_playing() {
    let muted = TabMedia {
        muted: true,
        ..playing(Some((760, 2880)))
    };
    let view = centre::view(&audio_input(muted));
    assert_eq!(view.mode, CentreMode::Audio);
    assert_eq!(view.play_state, "Muted");
    assert!(view.muted);

    // a muted tab that is silent is still the audio state
    let silent = TabMedia {
        tab: 115,
        muted: true,
        ..TabMedia::default()
    };
    let view = centre::view(&audio_input(silent));
    assert_eq!(view.mode, CentreMode::Audio);
    assert_eq!(view.play_state, "Muted");
}

#[test]
fn a_media_entry_with_no_sound_state_is_the_rest_state() {
    let view = centre::view(&audio_input(TabMedia {
        tab: 115,
        can_pause: true,
        ..TabMedia::default()
    }));
    assert_eq!(view.mode, CentreMode::Rest);
    assert!(view.play_state.is_empty());
    assert!(!view.can_pause);
}

#[test]
fn collapsed_pinned_tabs_replace_the_hint() {
    let mut input = rest_input("Essay on maps");
    input.pinned_collapsed = 3;
    assert_eq!(centre::view(&input).footer, "3 pinned tabs");
    input.pinned_collapsed = 1;
    assert_eq!(centre::view(&input).footer, "1 pinned tab");
    input.pinned_collapsed = 0;
    assert_eq!(centre::view(&input).footer, "Hold ⌥ to see group keys");

    // while searching or holding ⌥ their own footers stay
    input.pinned_collapsed = 3;
    input.query_active = true;
    assert_eq!(centre::view(&input).footer, "↑ ↓ choose · esc clears");
    input.query_active = false;
    input.held = true;
    assert_eq!(centre::view(&input).footer, "");
}

#[test]
fn an_empty_window_has_a_neutral_centre() {
    let view = centre::view(&CentreInput::default());
    assert_eq!(view.mode, CentreMode::Rest);
    assert_eq!(view.title, "");
    assert_eq!(view.subtitle, "");
    assert_eq!(view.play_state, "");
    assert_eq!(view.field_placeholder, "Search tabs");
    assert_eq!(view.footer, "Hold ⌥ to see group keys");
    assert!(!view.can_pause && !view.paused && !view.muted);
}
