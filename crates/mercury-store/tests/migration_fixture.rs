// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
mod common;

use common::{NOW, TempDir};
use mercury_store::{BookmarkRoot, SiteSettingChange, SiteSettingRow, Store, StoreError};

const PROFILE_V1: &str = include_str!("fixtures/profile-v1.sql");
const SCHEMA_SOURCE: &str = include_str!("../src/schema.rs");

fn v1_file(dir: &TempDir) {
    let conn = rusqlite::Connection::open(dir.db()).unwrap();
    conn.execute_batch(PROFILE_V1).unwrap();
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn table_exists(conn: &rusqlite::Connection, name: &str) -> bool {
    count(
        conn,
        &format!("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = '{name}'"),
    ) == 1
}

#[test]
fn the_fixture_holds_migration_1_verbatim() {
    let begin = "-- migration 1 begins\n";
    let end = "\n\n-- migration 1 ends\n";
    let start = PROFILE_V1.find(begin).expect("begin marker") + begin.len();
    let stop = PROFILE_V1.find(end).expect("end marker");
    let ddl = &PROFILE_V1[start..stop];
    assert!(ddl.starts_with("CREATE TABLE places ("));
    assert!(
        SCHEMA_SOURCE.contains(ddl),
        "tests/fixtures/profile-v1.sql no longer matches migration 1 in src/schema.rs"
    );
}

#[test]
fn a_v1_profile_file_is_migrated_in_place_and_keeps_its_data() {
    let dir = TempDir::new("migrate-v1");
    v1_file(&dir);
    {
        let conn = rusqlite::Connection::open(dir.db()).unwrap();
        let version: u32 = count(&conn, "PRAGMA user_version") as u32;
        assert_eq!(version, 1, "the fixture is a version 1 file");
        assert!(!table_exists(&conn, "site_settings"));
    }

    let mut store = Store::open(&dir.db()).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    assert_eq!(store.history(None, None, 10).unwrap().len(), 3);
    let toolbar = store.bookmark_root(BookmarkRoot::Toolbar).unwrap();
    let menu = store.bookmark_root(BookmarkRoot::Other).unwrap();
    assert_eq!(store.bookmark_children(toolbar).unwrap().len(), 1);
    assert_eq!(store.bookmark_children(menu).unwrap().len(), 1);
    assert_eq!(store.downloads(10).unwrap().len(), 1);
    assert!(store.site_settings_snapshot().unwrap().rows.is_empty());

    store
        .apply_site_setting_changes(&[SiteSettingChange::Put(SiteSettingRow {
            pattern: "https://example.com".into(),
            axis: "camera".into(),
            qualifier: String::new(),
            value: "allow".into(),
            set_at: NOW,
            expires_at: None,
            last_used_at: None,
            revoke_exempt: false,
        })])
        .unwrap();
    assert_eq!(store.site_settings_snapshot().unwrap().rows.len(), 1);
    drop(store);

    let conn = rusqlite::Connection::open(dir.db()).unwrap();
    for table in ["site_settings", "protocol_handlers", "site_settings_meta"] {
        assert!(table_exists(&conn, table), "missing table {table}");
    }
    assert_eq!(count(&conn, "PRAGMA user_version"), 2);
    assert_eq!(count(&conn, "SELECT count(*) FROM places"), 3);
    assert_eq!(count(&conn, "SELECT count(*) FROM visits"), 3);
    assert_eq!(count(&conn, "SELECT count(*) FROM downloads"), 1);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM bookmarks WHERE kind = 1"),
        2
    );
}

#[test]
fn a_v3_file_is_refused_and_left_unmodified() {
    let dir = TempDir::new("migrate-v3");
    {
        let conn = rusqlite::Connection::open(dir.db()).unwrap();
        conn.execute_batch("CREATE TABLE future (x INTEGER); PRAGMA user_version = 3;")
            .unwrap();
    }
    let before = std::fs::read(dir.db()).unwrap();

    match Store::open(&dir.db()) {
        Err(StoreError::UnsupportedSchemaVersion { found, supported }) => {
            assert_eq!((found, supported), (3, 2));
        }
        Err(other) => panic!("unexpected error {other}"),
        Ok(_) => panic!("a newer schema must be refused"),
    }
    assert_eq!(std::fs::read(dir.db()).unwrap(), before);
    assert!(!dir.path().join("places.sqlite-wal").exists());
}

#[test]
fn opening_twice_does_not_rerun_migration_2() {
    let dir = TempDir::new("migrate-twice");
    v1_file(&dir);
    {
        let mut store = Store::open(&dir.db()).unwrap();
        store
            .apply_site_setting_changes(&[SiteSettingChange::SetMeta {
                key: "k".into(),
                value: "v".into(),
            }])
            .unwrap();
    }
    // A second run of migration 2 would fail on "table already exists" or lose the row.
    let store = Store::open(&dir.db()).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    assert_eq!(
        store.site_settings_snapshot().unwrap().meta,
        [("k".to_string(), "v".to_string())]
    );
}
