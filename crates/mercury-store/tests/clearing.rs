// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Deleting history so that it does not linger: whole sites, and the purge that leaves no copy
//! of a deleted row in free pages, the full-text index or the write-ahead log.

mod common;

use common::{DAY_MS, Fixture, NOW, TempDir};
use mercury_store::{BookmarkRoot, ClearRange, Store, StoreError, VisitTransition};

fn visit(store: &mut Store, url: &str, title: &str, at: i64) {
    store
        .record_visit(url, Some(title), VisitTransition::Link, at)
        .unwrap();
}

fn urls(store: &Store) -> Vec<String> {
    let mut urls = store
        .history(None, None, 100)
        .unwrap()
        .into_iter()
        .map(|entry| entry.url)
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    urls
}

#[test]
fn deleting_a_site_removes_every_page_of_that_host_and_only_those() {
    let mut f = Fixture::new("delete-site");
    for url in [
        "https://example.com/",
        "http://www.example.com/a?b",
        "https://example.com:8443/c",
        "https://example.com#top",
        "https://user@example.com/d",
        "https://example.community/",
        "https://mail.example.com/",
        "https://other.test/example.com/",
    ] {
        visit(&mut f.store, url, "T", NOW);
    }

    let removed = f.store.delete_site("Example.COM", NOW).unwrap();

    assert_eq!(removed, 5);
    assert_eq!(
        urls(&f.store),
        vec![
            "https://example.community/",
            "https://mail.example.com/",
            "https://other.test/example.com/",
        ]
    );
    assert_eq!(f.count("SELECT count(*) FROM places"), 3);
}

#[test]
fn deleting_a_site_keeps_bookmarked_pages_without_their_visits() {
    let mut f = Fixture::new("delete-site-bookmark");
    visit(&mut f.store, "https://example.com/kept", "Kept", NOW);
    let toolbar = f.store.bookmark_root(BookmarkRoot::Toolbar).unwrap();
    f.store
        .add_bookmark(
            toolbar,
            None,
            "Mark",
            "https://example.com/kept",
            "g-1",
            NOW,
        )
        .unwrap();

    assert_eq!(f.store.delete_site("example.com", NOW).unwrap(), 1);

    let place = f.place("https://example.com/kept").unwrap();
    assert_eq!((place.visit_count, place.title), (0, None));
    assert!(urls(&f.store).is_empty());
}

#[test]
fn a_site_must_be_named() {
    let mut f = Fixture::new("delete-site-empty");
    assert!(matches!(
        f.store.delete_site(" ", NOW),
        Err(StoreError::InvalidArgument(_))
    ));
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn a_cleared_and_purged_history_leaves_no_trace_in_the_files() {
    let dir = TempDir::new("purge");
    let mut store = Store::open(&dir.db()).unwrap();
    // Enough rows to span several pages and full-text segments.
    for n in 0..400 {
        visit(
            &mut store,
            &format!("https://secretsite{n}.example/private"),
            &format!("Zanzibarqux diary {n}"),
            NOW - DAY_MS,
        );
    }
    visit(&mut store, "https://kept.example/", "Kept page", NOW);

    store
        .clear_history(
            ClearRange::Since {
                from_ms: NOW - 2 * DAY_MS,
            },
            NOW,
        )
        .unwrap();
    store.purge_deleted().unwrap();

    let mut bytes = std::fs::read(dir.db()).unwrap();
    if let Ok(wal) = std::fs::read(dir.db().with_extension("sqlite-wal")) {
        bytes.extend(wal);
    }
    for needle in [&b"secretsite"[..], b"Zanzibarqux", b"zanzibarqux"] {
        assert!(
            !contains(&bytes, needle),
            "{:?} is still in the files",
            String::from_utf8_lossy(needle)
        );
    }
    assert!(urls(&store).is_empty());
}

#[test]
fn clearing_everything_leaves_bookmarks_as_if_never_visited() {
    let mut f = Fixture::new("clear-all-bookmarks");
    let toolbar = f.store.bookmark_root(BookmarkRoot::Toolbar).unwrap();
    visit(
        &mut f.store,
        "https://visited.example/",
        "Visited",
        NOW - DAY_MS,
    );
    f.store
        .add_bookmark(
            toolbar,
            None,
            "Mark",
            "https://visited.example/",
            "g-1",
            NOW,
        )
        .unwrap();
    f.store
        .add_bookmark(toolbar, None, "Never", "https://never.example/", "g-2", NOW)
        .unwrap();
    visit(&mut f.store, "https://gone.example/", "Gone", NOW);
    let never_before = f.place("https://never.example/").unwrap();

    assert_eq!(f.store.clear_history(ClearRange::All, NOW).unwrap(), 2);

    assert_eq!(f.place("https://gone.example/"), None);
    assert_eq!(f.place("https://never.example/").unwrap(), never_before);
    let visited = f.place("https://visited.example/").unwrap();
    assert_eq!(
        (
            visited.title,
            visited.visit_count,
            visited.typed_count,
            visited.last_visit_at
        ),
        (None, 0, 0, None)
    );
    // Scored as an unvisited bookmark, like the one never visited.
    assert_eq!(visited.frecency, never_before.frecency);
    assert_eq!(f.count("SELECT count(*) FROM visits"), 0);
    assert!(f.store.history(Some("gone"), None, 10).unwrap().is_empty());
    assert_eq!(
        f.store
            .suggest("visited", 5, NOW)
            .unwrap()
            .into_iter()
            .map(|s| s.title)
            .collect::<Vec<_>>(),
        vec![Some("Mark".to_owned())]
    );
}
