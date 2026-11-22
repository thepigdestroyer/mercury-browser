// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: facts about the index tables of the IndexedDB SQLite store. This is a new Mercury file; nothing in it
// is copied from Servo or another project. It includes Servo's own `create.rs` (MPL-2.0) as a module, unchanged,
// so that the facts are checked against the real schema.

//! **What these tests are, and are not.** They pin facts about SQLite and about Servo's schema (that a unique
//! index's primary key refuses a repeated member, that the Debug text of a constraint failure names
//! `ConstraintViolation`, ...), written as SQL by hand. They call no Mercury code and are **not** evidence that
//! IndexedDB's multiEntry or unique indexes work. That evidence is:
//!
//! - the unit tests of `indexeddb/engines/sqlite/unique_index_rows.rs`, which put records through the engine's
//!   own `put_item`, `delete_item`, `clear` and `delete_index` (unique keys, multiEntry members, rollback of a
//!   refused put);
//! - the unit tests of `key.rs` and `idbindex.rs` in `servo-script` (key extraction, multiEntry expansion,
//!   ordering).
//!
//! The facts, which the writer in `unique_index_rows.rs` relies on:
//!
//! - `index_data` holds several rows for one record (one per member of a multiEntry key), and several records
//!   may share a member;
//! - `unique_index_data` holds one row per member too, and refuses a member that another record already has;
//! - both tables are therefore usable for multiEntry as they are: no migration is needed;
//! - the Debug text of a SQLite constraint failure names `ConstraintViolation`, which
//!   `map_backend_error_to_request_error` in `servo-script` turns into a ConstraintError.

use rusqlite::ffi::ErrorCode;
use rusqlite::{Connection, Error, params};

#[path = "../indexeddb/engines/sqlite/create.rs"]
mod create;

/// An in-memory store with the schema of `create.rs`, foreign keys enforced (Servo leaves them off
/// by default; enforcing them here checks that the rows a writer would produce are consistent).
fn open() -> Connection {
    let connection = Connection::open_in_memory().expect("open an in-memory database");
    create::create_tables(&connection).expect("create the tables");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enforce foreign keys");
    connection
}

/// One object store (id 1) with three records, keys `k1` to `k3`, and two multiEntry indexes on it:
/// id 1 is not unique, id 2 is unique.
fn seed(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO object_store (id, name, auto_increment) VALUES (1, 'chats', 0)",
            [],
        )
        .expect("insert the store");
    for key in [b"k1", b"k2", b"k3"] {
        connection
            .execute(
                "INSERT INTO object_data (object_store_id, key, data) VALUES (1, ?, x'00')",
                params![key.as_slice()],
            )
            .expect("insert a record");
    }
    for (id, name, unique) in [(1, "tags", false), (2, "ids", true)] {
        connection
            .execute(
                "INSERT INTO object_store_index \
                 (id, object_store_id, name, key_path, unique_index, multi_entry_index) \
                 VALUES (?, 1, ?, x'00', ?, 1)",
                params![id, name, unique],
            )
            .expect("insert an index");
    }
}

fn add_row(
    connection: &Connection,
    index_id: i64,
    member: &[u8],
    record: &[u8],
) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT INTO index_data (index_id, value, object_data_key, object_store_id) VALUES (?, ?, ?, 1)",
        params![index_id, member, record],
    )
}

fn add_unique_row(
    connection: &Connection,
    index_id: i64,
    member: &[u8],
    record: &[u8],
) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT INTO unique_index_data (index_id, value, object_store_id, object_data_key) VALUES (?, ?, 1, ?)",
        params![index_id, member, record],
    )
}

fn count(connection: &Connection, sql: &str, params: impl rusqlite::Params) -> i64 {
    connection
        .query_row(sql, params, |row| row.get(0))
        .expect("count rows")
}

fn constraint_failure(result: rusqlite::Result<usize>) -> Error {
    match result {
        Err(error) => error,
        Ok(rows) => panic!("expected a constraint failure, {rows} row(s) were written"),
    }
}

#[test]
fn a_multientry_record_has_one_row_per_member_in_index_data() {
    let connection = open();
    seed(&connection);

    // Record k1 has the tags a, b and c.
    for member in [b"a", b"b", b"c"] {
        add_row(&connection, 1, member, b"k1").expect("one row per member");
    }

    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM index_data WHERE index_id = 1 AND object_data_key = ?",
            params![b"k1".as_slice()],
        ),
        3
    );
    // The members are found by value.
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM index_data WHERE index_id = 1 AND value = ?",
            params![b"b".as_slice()],
        ),
        1
    );
}

#[test]
fn several_records_may_share_a_member_of_a_non_unique_index() {
    let connection = open();
    seed(&connection);

    add_row(&connection, 1, b"b", b"k1").expect("k1 has b");
    add_row(&connection, 1, b"b", b"k2").expect("k2 has b as well");
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM index_data WHERE index_id = 1 AND value = ?",
            params![b"b".as_slice()],
        ),
        2
    );

    // A record contributes a member once: the primary key is (index, value, record).
    let error = constraint_failure(add_row(&connection, 1, b"b", b"k1"));
    assert!(matches!(
        error,
        Error::SqliteFailure(failure, _) if failure.code == ErrorCode::ConstraintViolation
    ));
}

#[test]
fn a_unique_multientry_record_has_one_row_per_member_and_members_are_not_shared() {
    let connection = open();
    seed(&connection);

    // Record k1 has the members x and y of the unique index: one row each is fine.
    add_unique_row(&connection, 2, b"x", b"k1").expect("x");
    add_unique_row(&connection, 2, b"y", b"k1").expect("y");

    // Record k2 may have z, but not y, which k1 already has.
    add_unique_row(&connection, 2, b"z", b"k2").expect("z");
    let error = constraint_failure(add_unique_row(&connection, 2, b"y", b"k2"));
    assert!(matches!(
        error,
        Error::SqliteFailure(failure, _) if failure.code == ErrorCode::ConstraintViolation
    ));

    // The same member in another unique index is another row: the primary key starts with the index.
    connection
        .execute(
            "INSERT INTO object_store_index \
             (id, object_store_id, name, key_path, unique_index, multi_entry_index) \
             VALUES (3, 1, 'other', x'00', 1, 1)",
            [],
        )
        .expect("insert a third index");
    add_unique_row(&connection, 3, b"y", b"k2").expect("y in another index");

    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM unique_index_data WHERE index_id = 2",
            [],
        ),
        3
    );
}

#[test]
fn an_update_replaces_the_rows_of_a_record() {
    let connection = open();
    seed(&connection);

    // k1 had the tags a and b ...
    add_row(&connection, 1, b"a", b"k1").expect("a");
    add_row(&connection, 1, b"b", b"k1").expect("b");
    add_row(&connection, 1, b"b", b"k2").expect("k2 keeps its row");

    // ... and is put again with the tags b and c: delete its rows, then insert the new ones.
    connection
        .execute(
            "DELETE FROM index_data WHERE index_id = 1 AND object_data_key = ?",
            params![b"k1".as_slice()],
        )
        .expect("delete the old rows");
    add_row(&connection, 1, b"b", b"k1").expect("b");
    add_row(&connection, 1, b"c", b"k1").expect("c");

    let members: Vec<String> = connection
        .prepare(
            "SELECT CAST(value AS TEXT) FROM index_data WHERE object_data_key = ? ORDER BY value",
        )
        .expect("prepare")
        .query_map(params![b"k1".as_slice()], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert_eq!(members, vec!["b", "c"]);
    // The other record is untouched.
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM index_data WHERE object_data_key = ?",
            params![b"k2".as_slice()],
        ),
        1
    );
}

#[test]
fn the_rows_of_a_record_are_deleted_before_the_record() {
    let connection = open();
    seed(&connection);
    add_row(&connection, 1, b"a", b"k1").expect("a");
    add_row(&connection, 1, b"b", b"k1").expect("b");
    add_unique_row(&connection, 2, b"x", b"k1").expect("x");

    // The foreign keys refuse to delete a record that rows still refer to.
    let delete_record = |connection: &Connection| {
        connection.execute(
            "DELETE FROM object_data WHERE object_store_id = 1 AND key = ?",
            params![b"k1".as_slice()],
        )
    };
    let error = constraint_failure(delete_record(&connection));
    assert!(matches!(
        error,
        Error::SqliteFailure(failure, _) if failure.code == ErrorCode::ConstraintViolation
    ));

    // The order `delete_store` uses: the index rows, then the records.
    connection
        .execute(
            "DELETE FROM index_data WHERE object_data_key = ?",
            params![b"k1".as_slice()],
        )
        .expect("delete the index rows");
    connection
        .execute(
            "DELETE FROM unique_index_data WHERE object_data_key = ?",
            params![b"k1".as_slice()],
        )
        .expect("delete the unique index rows");
    assert_eq!(delete_record(&connection).expect("delete the record"), 1);
}

#[test]
fn the_tables_need_no_migration_for_multientry() {
    let connection = open();

    // object_store_index already has the multiEntry flag ...
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('object_store_index')")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert!(columns.contains(&"multi_entry_index".to_owned()));
    assert!(columns.contains(&"unique_index".to_owned()));

    // ... and the primary key of index_data names the record, so a record can have several rows;
    // that of unique_index_data does not, so a member belongs to one record.
    let primary_key = |table: &str| -> Vec<String> {
        connection
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('{table}') WHERE pk > 0 ORDER BY pk"
            ))
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };
    assert_eq!(
        primary_key("index_data"),
        vec!["index_id", "value", "object_data_key"]
    );
    assert_eq!(primary_key("unique_index_data"), vec!["index_id", "value"]);
}

#[test]
fn a_sqlite_constraint_failure_names_the_constraint_class_that_the_error_mapping_looks_for() {
    // sqlite.rs reports a failure as `BackendError::DbErr(format!("{error:?}"))`, and
    // `map_backend_error_to_request_error` (servo-script, key.rs) looks for "ConstraintViolation".
    let connection = open();
    seed(&connection);
    add_unique_row(&connection, 2, b"x", b"k1").expect("x");
    let error = constraint_failure(add_unique_row(&connection, 2, b"x", b"k2"));
    assert!(
        format!("{error:?}").contains("ConstraintViolation"),
        "the Debug text of the error is {error:?}"
    );
}
