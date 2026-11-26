// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
mod common;

use common::{DAY_MS, Fixture, NOW, TempDir};
use mercury_store::{ClearRange, RecentSearch, Store, VisitTransition};

fn terms(store: &Store, prefix: &str) -> Vec<String> {
    store
        .recent_searches(prefix, 20)
        .unwrap()
        .into_iter()
        .map(|search| search.term)
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn a_search_is_recorded_with_its_engine_and_time() {
    let mut f = Fixture::new("searches-record");

    assert!(
        f.store
            .record_search("Rust Book", Some("duckduckgo"), NOW)
            .unwrap()
    );

    assert_eq!(
        f.store.recent_searches("", 10).unwrap(),
        [RecentSearch {
            term: "Rust Book".into(),
            engine_id: Some("duckduckgo".into()),
            count: 1,
            last_at_ms: NOW,
        }]
    );
}

#[test]
fn a_repeated_search_is_one_row_with_the_latest_spelling_and_engine() {
    let mut f = Fixture::new("searches-repeat");
    f.store
        .record_search("Rust Book", Some("duckduckgo"), NOW)
        .unwrap();

    assert!(
        f.store
            .record_search("  rust   book ", Some("ecosia"), NOW + 1)
            .unwrap()
    );

    assert_eq!(
        f.store.recent_searches("", 10).unwrap(),
        [RecentSearch {
            term: "rust book".into(),
            engine_id: Some("ecosia".into()),
            count: 2,
            last_at_ms: NOW + 1,
        }]
    );
}

#[test]
fn searches_are_listed_newest_first() {
    let mut f = Fixture::new("searches-order");
    f.store.record_search("alpha", None, NOW).unwrap();
    f.store.record_search("beta", None, NOW + 1).unwrap();
    f.store.record_search("gamma", None, NOW + 2).unwrap();
    assert_eq!(terms(&f.store, ""), ["gamma", "beta", "alpha"]);

    f.store.record_search("alpha", None, NOW + 3).unwrap();
    assert_eq!(terms(&f.store, ""), ["alpha", "gamma", "beta"]);
}

#[test]
fn at_most_twenty_searches_are_listed() {
    let mut f = Fixture::new("searches-limit");
    for n in 0..30i64 {
        f.store
            .record_search(&format!("term {n:02}"), None, NOW + n)
            .unwrap();
    }

    assert_eq!(f.store.recent_searches("", 100).unwrap().len(), 20);
    assert_eq!(f.store.recent_searches("", 5).unwrap().len(), 5);
}

#[test]
fn a_prefix_matches_the_start_of_a_term_ignoring_case() {
    let mut f = Fixture::new("searches-prefix");
    f.store.record_search("Rust Book", None, NOW).unwrap();
    f.store.record_search("ruby", None, NOW + 1).unwrap();
    f.store.record_search("Lisp", None, NOW + 2).unwrap();

    assert_eq!(terms(&f.store, "ru"), ["ruby", "Rust Book"]);
    assert_eq!(terms(&f.store, "RU"), ["ruby", "Rust Book"]);
    assert!(terms(&f.store, "bo").is_empty());
}

#[test]
fn prefixes_are_text_and_not_patterns() {
    let mut f = Fixture::new("searches-wildcards");
    f.store.record_search("Rust Book", None, NOW).unwrap();
    f.store.record_search("ruby", None, NOW + 1).unwrap();

    for prefix in ["%", "_", "r%", "r_"] {
        assert!(terms(&f.store, prefix).is_empty(), "{prefix:?}");
    }
}

#[test]
fn empty_long_and_control_character_terms_are_refused() {
    let mut f = Fixture::new("searches-refused");
    let too_long = "a".repeat(257);
    for term in ["", "   ", too_long.as_str(), "a\u{0}b"] {
        assert!(!f.store.record_search(term, None, NOW).unwrap(), "{term:?}");
    }
    assert_eq!(f.count("SELECT count(*) FROM searches"), 0);

    assert!(f.store.record_search(&"a".repeat(256), None, NOW).unwrap());
    assert_eq!(terms(&f.store, "")[0].len(), 256);
}

#[test]
fn only_the_newest_two_hundred_searches_are_kept() {
    let mut f = Fixture::new("searches-cap");
    for n in 0..205i64 {
        f.store
            .record_search(&format!("term {n:03}"), None, NOW + n)
            .unwrap();
    }

    assert_eq!(f.count("SELECT count(*) FROM searches"), 200);
    assert_eq!(
        f.count(
            "SELECT count(*) FROM searches WHERE term_key IN
             ('term 000', 'term 001', 'term 002', 'term 003', 'term 004')"
        ),
        0
    );
    assert_eq!(
        f.count("SELECT count(*) FROM searches WHERE term_key = 'term 005'"),
        1
    );
}

#[test]
fn a_search_is_deleted_by_its_term_ignoring_case() {
    let mut f = Fixture::new("searches-delete");
    f.store.record_search("Rust Book", None, NOW).unwrap();

    assert!(f.store.delete_recent_search("RUST book").unwrap());
    assert!(!f.store.delete_recent_search("RUST book").unwrap());
    assert!(terms(&f.store, "").is_empty());
}

#[test]
fn clearing_everything_removes_every_search() {
    let mut f = Fixture::new("searches-clear-all");
    f.store.record_search("one", None, NOW).unwrap();
    f.store.record_search("two", None, NOW + 1).unwrap();
    f.store.record_search("three", None, NOW + 2).unwrap();

    assert_eq!(f.store.clear_recent_searches(ClearRange::All).unwrap(), 3);
    assert!(terms(&f.store, "").is_empty());
}

#[test]
fn clearing_a_range_keeps_older_searches() {
    let mut f = Fixture::new("searches-clear-since");
    f.store.record_search("old", None, NOW - DAY_MS).unwrap();
    f.store.record_search("new", None, NOW).unwrap();

    assert_eq!(
        f.store
            .clear_recent_searches(ClearRange::Since { from_ms: NOW })
            .unwrap(),
        1
    );
    assert_eq!(terms(&f.store, ""), ["old"]);
}

#[test]
fn clearing_history_clears_the_searches_in_the_same_range() {
    let mut f = Fixture::new("searches-clear-history");
    f.store
        .record_search("old", None, NOW - 2 * DAY_MS)
        .unwrap();
    f.store.record_search("new", None, NOW).unwrap();
    f.store
        .record_visit(
            "https://example.com/old",
            None,
            VisitTransition::Link,
            NOW - 2 * DAY_MS,
        )
        .unwrap();
    f.store
        .record_visit("https://example.com/new", None, VisitTransition::Link, NOW)
        .unwrap();

    f.store
        .clear_history(
            ClearRange::Since {
                from_ms: NOW - DAY_MS,
            },
            NOW,
        )
        .unwrap();
    assert_eq!(terms(&f.store, ""), ["old"]);
    assert_eq!(f.store.history(None, None, 10).unwrap().len(), 1);

    f.store.clear_history(ClearRange::All, NOW).unwrap();
    assert!(terms(&f.store, "").is_empty());
}

#[test]
fn searches_survive_reopening_the_database() {
    let dir = TempDir::new("searches-reopen");
    {
        let mut store = Store::open(&dir.db()).unwrap();
        store
            .record_search("Rust Book", Some("duckduckgo"), NOW)
            .unwrap();
    }

    let store = Store::open(&dir.db()).unwrap();
    assert_eq!(
        store.recent_searches("ru", 10).unwrap(),
        [RecentSearch {
            term: "Rust Book".into(),
            engine_id: Some("duckduckgo".into()),
            count: 1,
            last_at_ms: NOW,
        }]
    );
}

#[test]
fn a_deleted_search_leaves_no_trace_in_the_file_after_a_purge() {
    let mut f = Fixture::new("searches-purge");
    for n in 0..150i64 {
        f.store
            .record_search(&format!("Plutonium Secret Query {n}"), None, NOW + n)
            .unwrap();
    }
    for n in 0..150 {
        assert!(
            f.store
                .delete_recent_search(&format!("plutonium secret query {n}"))
                .unwrap()
        );
    }
    f.store.purge_deleted().unwrap();

    let bytes = std::fs::read(f.dir.db()).unwrap();
    for needle in [&b"Plutonium"[..], b"plutonium", b"Secret Query"] {
        assert!(
            !contains(&bytes, needle),
            "{:?} is still in the file",
            String::from_utf8_lossy(needle)
        );
    }
}
