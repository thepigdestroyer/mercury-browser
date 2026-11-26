// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The protection tables: exceptions, the weekly tally and bounce records (R13 MH12, MH32).

mod common;

use common::{Fixture, NOW, TempDir};
use mercury_store::{
    BookmarkRoot, ClearRange, ProtectionBounceRow, ProtectionExceptionRow, ProtectionTallyRow,
    Store, VisitTransition,
};

fn exception(site: &str, level: Option<&str>, overrides: &str) -> ProtectionExceptionRow {
    ProtectionExceptionRow {
        site: site.to_owned(),
        shields_down: false,
        level: level.map(str::to_owned),
        overrides: overrides.to_owned(),
        updated_at: NOW,
    }
}

fn bounce(
    site: &str,
    last_activation_at: Option<i64>,
    bounced_at: Option<i64>,
) -> ProtectionBounceRow {
    ProtectionBounceRow {
        site: site.to_owned(),
        last_activation_at,
        bounced_at,
    }
}

fn tally(day: i64, category: &str, count: i64) -> ProtectionTallyRow {
    ProtectionTallyRow {
        day,
        category: category.to_owned(),
        count,
    }
}

fn sites(rows: &[ProtectionBounceRow]) -> Vec<&str> {
    rows.iter().map(|row| row.site.as_str()).collect()
}

#[test]
fn an_exception_round_trips_with_its_level_overrides_and_shields_down() {
    let mut store = Store::open_in_memory().unwrap();
    let row = ProtectionExceptionRow {
        shields_down: true,
        ..exception("news.example", Some("strict"), "ads=off,cookies=strict")
    };
    store.put_protection_exception(&row).unwrap();
    assert_eq!(store.protection_exceptions().unwrap(), [row]);
}

#[test]
fn an_exception_without_a_level_reads_back_as_none() {
    let mut store = Store::open_in_memory().unwrap();
    let row = exception("plain.example", None, "");
    store.put_protection_exception(&row).unwrap();
    assert_eq!(store.protection_exceptions().unwrap(), [row]);
}

#[test]
fn putting_an_exception_again_replaces_it() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .put_protection_exception(&exception("a.example", Some("strict"), "ads=off"))
        .unwrap();
    let replacement = ProtectionExceptionRow {
        shields_down: true,
        updated_at: NOW + 1,
        ..exception("a.example", None, "")
    };
    store.put_protection_exception(&replacement).unwrap();
    assert_eq!(store.protection_exceptions().unwrap(), [replacement]);
}

#[test]
fn removing_an_exception_says_whether_it_existed() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .put_protection_exception(&exception("a.example", None, ""))
        .unwrap();
    assert!(store.remove_protection_exception("a.example").unwrap());
    assert!(!store.remove_protection_exception("a.example").unwrap());
    assert!(store.protection_exceptions().unwrap().is_empty());
}

#[test]
fn exceptions_are_listed_ordered_by_site() {
    let mut store = Store::open_in_memory().unwrap();
    for site in ["c.example", "a.example", "b.example"] {
        store
            .put_protection_exception(&exception(site, None, ""))
            .unwrap();
    }
    let listed: Vec<String> = store
        .protection_exceptions()
        .unwrap()
        .into_iter()
        .map(|row| row.site)
        .collect();
    assert_eq!(listed, ["a.example", "b.example", "c.example"]);
}

#[test]
fn clearing_exceptions_returns_how_many_were_removed() {
    let mut store = Store::open_in_memory().unwrap();
    for site in ["a.example", "b.example"] {
        store
            .put_protection_exception(&exception(site, None, ""))
            .unwrap();
    }
    assert_eq!(store.clear_protection_exceptions().unwrap(), 2);
    assert!(store.protection_exceptions().unwrap().is_empty());
}

#[test]
fn the_tally_adds_across_calls_to_the_same_day_and_category() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .add_protection_tally(5, "ads_and_trackers", 3)
        .unwrap();
    store
        .add_protection_tally(5, "ads_and_trackers", 4)
        .unwrap();
    store.add_protection_tally(5, "cryptominers", 1).unwrap();
    let rows = store.protection_tally_since(0).unwrap();
    assert_eq!(
        rows,
        [tally(5, "ads_and_trackers", 7), tally(5, "cryptominers", 1),]
    );
}

#[test]
fn the_tally_saturates_at_the_largest_count() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .add_protection_tally(5, "ads_and_trackers", u64::MAX)
        .unwrap();
    store
        .add_protection_tally(5, "ads_and_trackers", u64::MAX)
        .unwrap();
    store
        .add_protection_tally(5, "ads_and_trackers", 1)
        .unwrap();
    assert_eq!(
        store.protection_tally_since(0).unwrap(),
        [tally(5, "ads_and_trackers", i64::MAX)]
    );
}

#[test]
fn the_tally_since_filters_by_day() {
    let mut store = Store::open_in_memory().unwrap();
    for day in [1, 2, 3] {
        store.add_protection_tally(day, "cryptominers", 1).unwrap();
    }
    let days: Vec<i64> = store
        .protection_tally_since(2)
        .unwrap()
        .into_iter()
        .map(|row| row.day)
        .collect();
    assert_eq!(days, [2, 3]);
}

#[test]
fn pruning_the_tally_deletes_only_older_days() {
    let mut store = Store::open_in_memory().unwrap();
    for day in [1, 2, 3] {
        store.add_protection_tally(day, "cryptominers", 1).unwrap();
    }
    assert_eq!(store.prune_protection_tally(2).unwrap(), 1);
    let days: Vec<i64> = store
        .protection_tally_since(0)
        .unwrap()
        .into_iter()
        .map(|row| row.day)
        .collect();
    assert_eq!(days, [2, 3]);
}

#[test]
fn clearing_the_tally_returns_how_many_rows_were_removed() {
    let mut store = Store::open_in_memory().unwrap();
    store.add_protection_tally(1, "cryptominers", 1).unwrap();
    store.add_protection_tally(2, "cryptominers", 1).unwrap();
    assert_eq!(store.clear_protection_tally().unwrap(), 2);
    assert!(store.protection_tally_since(0).unwrap().is_empty());
}

#[test]
fn replacing_bounce_records_swaps_the_whole_set() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .replace_protection_bounce(&[
            bounce("a.example", Some(1), None),
            bounce("b.example", None, Some(2)),
        ])
        .unwrap();
    store
        .replace_protection_bounce(&[bounce("c.example", Some(3), Some(4))])
        .unwrap();
    assert_eq!(
        store.protection_bounce_records().unwrap(),
        [bounce("c.example", Some(3), Some(4))]
    );
}

#[test]
fn a_failing_bounce_replacement_leaves_the_old_set() {
    let mut store = Store::open_in_memory().unwrap();
    let old = [bounce("a.example", Some(1), None)];
    store.replace_protection_bounce(&old).unwrap();
    let broken = [
        bounce("b.example", None, Some(2)),
        bounce("b.example", None, Some(3)),
    ];
    assert!(store.replace_protection_bounce(&broken).is_err());
    assert_eq!(store.protection_bounce_records().unwrap(), old);
}

#[test]
fn clearing_bounce_records_returns_how_many_were_removed() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .replace_protection_bounce(&[
            bounce("a.example", Some(1), None),
            bounce("b.example", None, Some(2)),
        ])
        .unwrap();
    assert_eq!(store.clear_protection_bounce().unwrap(), 2);
    assert!(store.protection_bounce_records().unwrap().is_empty());
}

#[test]
fn a_ranged_history_clear_deletes_bounce_rows_with_a_time_in_range() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .replace_protection_bounce(&[
            // Activation in range: deleted.
            bounce("x.example", Some(100), None),
            // Both times before the range: kept.
            bounce("y.example", Some(10), Some(20)),
            // One time in range: deleted.
            bounce("z.example", Some(10), Some(60)),
        ])
        .unwrap();
    store
        .clear_history(ClearRange::Since { from_ms: 50 }, NOW)
        .unwrap();
    assert_eq!(
        sites(&store.protection_bounce_records().unwrap()),
        ["y.example"]
    );
}

#[test]
fn a_full_history_clear_deletes_every_bounce_row() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .replace_protection_bounce(&[bounce("y.example", Some(10), Some(20))])
        .unwrap();
    store.clear_history(ClearRange::All, NOW).unwrap();
    assert!(store.protection_bounce_records().unwrap().is_empty());
}

#[test]
fn exceptions_and_the_tally_survive_a_history_clear() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .put_protection_exception(&exception("a.example", Some("strict"), ""))
        .unwrap();
    store.add_protection_tally(5, "cryptominers", 2).unwrap();
    store.clear_history(ClearRange::All, NOW).unwrap();
    store
        .clear_history(ClearRange::Since { from_ms: 0 }, NOW)
        .unwrap();
    assert_eq!(store.protection_exceptions().unwrap().len(), 1);
    assert_eq!(
        store.protection_tally_since(0).unwrap(),
        [tally(5, "cryptominers", 2)]
    );
}

#[test]
fn a_removed_exception_leaves_no_bytes_in_the_file() {
    let marker = "secret-exception-marker.example";
    let mut fixture = Fixture::new("secure-delete");
    fixture
        .store
        .put_protection_exception(&exception(marker, Some("strict"), "ads=off"))
        .unwrap();
    assert!(fixture.store.remove_protection_exception(marker).unwrap());
    fixture.store.purge_deleted().unwrap();
    let file = std::fs::read(fixture.dir.db()).unwrap();
    assert!(
        !file
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
    );
}

#[test]
fn a_version_1_file_migrates_to_2_and_keeps_its_data() {
    let dir = TempDir::new("migrate-v1");
    {
        let mut store = Store::open(&dir.db()).unwrap();
        store
            .record_visit(
                "https://example.com/",
                Some("Example"),
                VisitTransition::Link,
                NOW,
            )
            .unwrap();
        let toolbar = store.bookmark_root(BookmarkRoot::Toolbar).unwrap();
        store
            .add_bookmark(
                toolbar,
                None,
                "Example",
                "https://example.com/",
                "guid-1",
                NOW,
            )
            .unwrap();
    }
    // Turn the file into a version-1 one: the only difference from version 2 is the three
    // protection tables.
    {
        let conn = rusqlite::Connection::open(dir.db()).unwrap();
        conn.execute_batch(
            "DROP TABLE protection_bounce;
             DROP TABLE protection_tally;
             DROP TABLE protection_exceptions;
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }

    let store = Store::open(&dir.db()).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    assert_eq!(store.history(None, None, 10).unwrap().len(), 1);
    let toolbar = store.bookmark_root(BookmarkRoot::Toolbar).unwrap();
    assert_eq!(store.bookmark_children(toolbar).unwrap().len(), 1);
    assert!(store.protection_exceptions().unwrap().is_empty());
}
