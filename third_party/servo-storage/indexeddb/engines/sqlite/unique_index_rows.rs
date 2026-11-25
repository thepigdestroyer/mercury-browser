// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: the rows of unique indexes (net-1 review, finding 3). A new Mercury file; nothing in it is copied from
// Servo or another project. The hooks are in `../sqlite.rs`; `servo-storage/MERCURY.md` lists them.

//! The rows of the unique indexes of an object store.
//!
//! Servo 0.8's SQLite backend creates the tables `index_data` and `unique_index_data` but writes neither: it
//! stores values as opaque structured-clone blobs and cannot evaluate a key path. Index queries are therefore
//! answered by the script, which derives the records of an index from the whole object store (see
//! `servo-script/dom/indexeddb/idbindex.rs`). A unique index cannot be enforced that way, because a put has to
//! be refused when it is executed, in order with the other requests of its transaction.
//!
//! So the script sends the keys of the record in each unique index of the store with the put
//! ([`UniqueIndexKeys`]), and this module keeps them in `unique_index_data`, whose primary key `(index_id,
//! value)` makes a second record with the same key in the same index a constraint violation. The rows are
//! replaced when a record is put again and removed when a record, a store or an index goes. Non-unique
//! indexes have no rows: nothing needs them.
//!
//! What this does **not** do: a unique index created on a store that already has records does not see them
//! (the script does not backfill the rows, so a duplicate of an old record is not caught), and neither does a
//! record written before this module existed. A violation is reported as the SQLite error text
//! `ConstraintViolation`, which the script turns into a `ConstraintError`.

use rusqlite::{Connection, Error, OptionalExtension, params};
use storage_traits::indexeddb::{IndexedDBKeyType, PutItemResult, UniqueIndexKeys};

use super::{SqliteEngine, encoding, object_store_model};

/// A lookup by record: deleting the rows of one record must not scan the table (a bulk load of N records
/// would be O(N^2)). `CREATE INDEX IF NOT EXISTS` is cheap once it exists and adds the index to databases
/// that were created before it.
const LOOKUP_BY_RECORD: &str = "CREATE INDEX IF NOT EXISTS unique_index_data_by_record \
     ON unique_index_data (object_store_id, object_data_key)";

/// Whether the store has a unique index: only then are there rows to keep, and only then is a delete or a
/// clear worth the extra work.
pub(super) fn store_has_unique_index(connection: &Connection, store_id: i32) -> Result<bool, Error> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_store_index WHERE object_store_id = ? AND unique_index = 1)",
        params![store_id],
        |row| row.get(0),
    )
}

/// `SqliteEngine::put_item` inside one savepoint, so that a put that a unique index refuses leaves nothing
/// behind: not the record, not a row, not the key generator's number. Without keys to keep there is nothing to
/// refuse, and the put is exactly Servo's.
pub(super) fn put_item_atomically(
    connection: &Connection,
    store: object_store_model::Model,
    key: IndexedDBKeyType,
    value: Vec<u8>,
    should_overwrite: bool,
    key_generator_current_number: Option<i64>,
    unique_index_keys: &[UniqueIndexKeys],
) -> Result<PutItemResult, Error> {
    if unique_index_keys.is_empty() {
        return SqliteEngine::put_item(
            connection,
            store,
            key,
            value,
            should_overwrite,
            key_generator_current_number,
            unique_index_keys,
        );
    }
    connection.execute_batch("SAVEPOINT mercury_put")?;
    let result = SqliteEngine::put_item(
        connection,
        store,
        key,
        value,
        should_overwrite,
        key_generator_current_number,
        unique_index_keys,
    );
    if result.is_ok() {
        connection.execute_batch("RELEASE mercury_put")?;
    } else {
        // Undo, then release the savepoint itself; the put's own error is what the caller reports.
        let _ = connection.execute_batch("ROLLBACK TO mercury_put; RELEASE mercury_put");
    }
    result
}

/// Replaces the rows of the record `serialized_key` of store `store_id` with the keys it has now. On a
/// constraint violation the error is returned and the caller rolls the put back (see `put_item`).
pub(super) fn replace_rows(
    connection: &Connection,
    store_id: i32,
    serialized_key: &[u8],
    unique_index_keys: &[UniqueIndexKeys],
) -> Result<(), Error> {
    if unique_index_keys.is_empty() {
        // A store with no unique index (or an old caller) has no rows to keep or to replace.
        return Ok(());
    }
    connection.execute(LOOKUP_BY_RECORD, [])?;
    connection.execute(
        "DELETE FROM unique_index_data WHERE object_store_id = ? AND object_data_key = ?",
        params![store_id, serialized_key],
    )?;
    for entry in unique_index_keys {
        let index_id: Option<i64> = connection
            .query_row(
                "SELECT id FROM object_store_index \
                 WHERE object_store_id = ? AND name = ? AND unique_index = 1",
                params![store_id, entry.index_name],
                |row| row.get(0),
            )
            .optional()?;
        // An index that is gone has nothing to check.
        let Some(index_id) = index_id else {
            continue;
        };
        let mut done: Vec<&IndexedDBKeyType> = Vec::new();
        for key in &entry.keys {
            // The members of a multiEntry key are distinct, but a caller that repeats one must not
            // collide with itself.
            if done.contains(&key) {
                continue;
            }
            done.push(key);
            connection.execute(
                "INSERT INTO unique_index_data (index_id, value, object_store_id, object_data_key) \
                 VALUES (?, ?, ?, ?)",
                params![index_id, encoding::serialize(key), store_id, serialized_key],
            )?;
        }
    }
    Ok(())
}

/// Removes the rows of the records `serialized_keys` of store `store_id` (records that are deleted).
pub(super) fn remove_rows_of_records(
    connection: &Connection,
    store_id: i32,
    serialized_keys: &[Vec<u8>],
) -> Result<(), Error> {
    if serialized_keys.is_empty() {
        return Ok(());
    }
    connection.execute(LOOKUP_BY_RECORD, [])?;
    let mut statement = connection.prepare(
        "DELETE FROM unique_index_data WHERE object_store_id = ? AND object_data_key = ?",
    )?;
    for key in serialized_keys {
        statement.execute(params![store_id, key])?;
    }
    Ok(())
}

/// Removes every row of store `store_id` (`clear`).
pub(super) fn remove_rows_of_store(connection: &Connection, store_id: i32) -> Result<(), Error> {
    connection.execute(
        "DELETE FROM unique_index_data WHERE object_store_id = ?",
        params![store_id],
    )?;
    Ok(())
}

/// Removes the rows of the index `index_name` of store `store_id` (the index is deleted).
pub(super) fn remove_rows_of_index(
    connection: &Connection,
    store_id: i32,
    index_name: &str,
) -> Result<(), Error> {
    connection.execute(
        "DELETE FROM unique_index_data WHERE index_id IN \
         (SELECT id FROM object_store_index WHERE object_store_id = ? AND name = ?)",
        params![store_id, index_name],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use storage_traits::indexeddb::{IndexedDBKeyRange, KeyPath};

    use super::super::create;
    use super::*;

    fn number(value: f64) -> IndexedDBKeyType {
        IndexedDBKeyType::Number(value)
    }

    fn text(value: &str) -> IndexedDBKeyType {
        IndexedDBKeyType::String(value.to_owned())
    }

    fn keys(index: &str, keys: Vec<IndexedDBKeyType>) -> Vec<UniqueIndexKeys> {
        vec![UniqueIndexKeys {
            index_name: index.to_owned(),
            keys,
        }]
    }

    /// An in-memory database with one store, `people`, and a unique index `by_email` (and a plain one,
    /// `by_name`), made with the engine's own `create_tables`, `create_store` and `create_index`.
    fn database() -> (Connection, object_store_model::Model) {
        let connection = Connection::open_in_memory().unwrap();
        create::create_tables(&connection).unwrap();
        SqliteEngine::create_store(&connection, "people", None, false).unwrap();
        SqliteEngine::create_index(
            &connection,
            "people",
            "by_email".to_owned(),
            KeyPath::String("email".to_owned()),
            true,
            false,
        )
        .unwrap();
        SqliteEngine::create_index(
            &connection,
            "people",
            "by_name".to_owned(),
            KeyPath::String("name".to_owned()),
            false,
            false,
        )
        .unwrap();
        let store = SqliteEngine::object_store_by_name(&connection, "people").unwrap();
        (connection, store)
    }

    fn put(
        connection: &Connection,
        store: &object_store_model::Model,
        key: f64,
        value: u8,
        unique_index_keys: &[UniqueIndexKeys],
    ) -> Result<PutItemResult, Error> {
        put_item_atomically(
            connection,
            store.clone(),
            number(key),
            vec![value],
            true,
            None,
            unique_index_keys,
        )
    }

    fn records(connection: &Connection, store: &object_store_model::Model) -> Vec<Vec<u8>> {
        SqliteEngine::get_all_items(
            connection,
            store.clone(),
            IndexedDBKeyRange::default(),
            None,
        )
        .unwrap()
    }

    fn rows(connection: &Connection) -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM unique_index_data", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn is_constraint_violation(error: &Error) -> bool {
        // The text that the script maps to a ConstraintError is the Debug text of the SQLite error.
        format!("{error:?}").contains("ConstraintViolation")
    }

    #[test]
    fn a_second_record_with_the_same_unique_key_is_a_constraint_violation_and_changes_nothing() {
        let (connection, store) = database();
        put(&connection, &store, 1.0, 10, &keys("by_email", vec![text("a@x")])).unwrap();

        let error = put(&connection, &store, 2.0, 20, &keys("by_email", vec![text("a@x")]))
            .expect_err("the key is taken");
        assert!(is_constraint_violation(&error), "{error:?}");

        // The refused put left no record behind (the object_data write was rolled back with it).
        assert_eq!(records(&connection, &store), vec![vec![10]]);
        assert_eq!(rows(&connection), 1);
    }

    #[test]
    fn putting_a_record_again_with_its_own_key_is_not_a_violation_and_replaces_its_rows() {
        let (connection, store) = database();
        put(&connection, &store, 1.0, 10, &keys("by_email", vec![text("a@x")])).unwrap();
        // The same record, the same key: an update.
        put(&connection, &store, 1.0, 11, &keys("by_email", vec![text("a@x")])).unwrap();
        assert_eq!(records(&connection, &store), vec![vec![11]]);
        assert_eq!(rows(&connection), 1);

        // The record changes its key: the old key is free for another record.
        put(&connection, &store, 1.0, 12, &keys("by_email", vec![text("b@x")])).unwrap();
        put(&connection, &store, 2.0, 20, &keys("by_email", vec![text("a@x")])).unwrap();
        assert_eq!(rows(&connection), 2);
    }

    #[test]
    fn a_record_with_no_key_in_the_index_is_not_in_it() {
        let (connection, store) = database();
        // Two records without a key for the unique index do not collide with each other.
        put(&connection, &store, 1.0, 10, &keys("by_email", vec![])).unwrap();
        put(&connection, &store, 2.0, 20, &keys("by_email", vec![])).unwrap();
        assert_eq!(rows(&connection), 0);
        assert_eq!(records(&connection, &store).len(), 2);
    }

    #[test]
    fn multientry_members_are_unique_across_records_and_a_repeated_member_does_not_collide_with_itself() {
        let (connection, store) = database();
        put(
            &connection,
            &store,
            1.0,
            10,
            &keys("by_email", vec![text("red"), text("green"), text("red")]),
        )
        .unwrap();
        assert_eq!(rows(&connection), 2);

        // Another record that shares one member is refused as a whole.
        let error = put(
            &connection,
            &store,
            2.0,
            20,
            &keys("by_email", vec![text("blue"), text("green")]),
        )
        .expect_err("`green` is taken");
        assert!(is_constraint_violation(&error), "{error:?}");
        assert_eq!(rows(&connection), 2, "not even `blue` was kept");
        assert_eq!(records(&connection, &store), vec![vec![10]]);

        // A record with new members is fine.
        put(
            &connection,
            &store,
            3.0,
            30,
            &keys("by_email", vec![text("blue"), text("yellow")]),
        )
        .unwrap();
        assert_eq!(rows(&connection), 4);
    }

    #[test]
    fn the_unique_key_is_free_again_when_its_record_is_deleted_or_the_store_is_cleared() {
        let (connection, store) = database();
        put(&connection, &store, 1.0, 10, &keys("by_email", vec![text("a@x")])).unwrap();
        put(&connection, &store, 2.0, 20, &keys("by_email", vec![text("b@x")])).unwrap();

        SqliteEngine::delete_item(
            &connection,
            store.clone(),
            IndexedDBKeyRange::only(number(1.0)),
        )
        .unwrap();
        assert_eq!(rows(&connection), 1);
        put(&connection, &store, 3.0, 30, &keys("by_email", vec![text("a@x")])).unwrap();

        SqliteEngine::clear(&connection, store.clone()).unwrap();
        assert_eq!(rows(&connection), 0);
        put(&connection, &store, 4.0, 40, &keys("by_email", vec![text("a@x")])).unwrap();
        put(&connection, &store, 5.0, 50, &keys("by_email", vec![text("b@x")])).unwrap();
    }

    #[test]
    fn a_range_delete_removes_the_rows_of_every_record_in_the_range_and_only_those() {
        let (connection, store) = database();
        for n in 1..=5 {
            put(
                &connection,
                &store,
                f64::from(n),
                n as u8,
                &keys("by_email", vec![text(&format!("k{n}"))]),
            )
            .unwrap();
        }
        SqliteEngine::delete_item(
            &connection,
            store.clone(),
            IndexedDBKeyRange::new(Some(number(2.0)), Some(number(4.0)), false, false),
        )
        .unwrap();
        assert_eq!(rows(&connection), 2);
        // k2..k4 are free, k1 and k5 are not.
        put(&connection, &store, 6.0, 60, &keys("by_email", vec![text("k3")])).unwrap();
        assert!(
            put(&connection, &store, 7.0, 70, &keys("by_email", vec![text("k5")])).is_err()
        );
    }

    #[test]
    fn deleting_the_index_drops_its_rows_and_a_put_for_a_deleted_index_checks_nothing() {
        let (connection, store) = database();
        put(&connection, &store, 1.0, 10, &keys("by_email", vec![text("a@x")])).unwrap();
        SqliteEngine::delete_index(&connection, "people", "by_email".to_owned()).unwrap();
        assert_eq!(rows(&connection), 0);
        // A put that still names the index (a request already in flight) is not an error and writes no row.
        put(&connection, &store, 2.0, 20, &keys("by_email", vec![text("a@x")])).unwrap();
        put(&connection, &store, 3.0, 30, &keys("by_email", vec![text("a@x")])).unwrap();
        assert_eq!(rows(&connection), 0);
    }

    #[test]
    fn a_non_unique_index_is_never_enforced_even_if_named() {
        let (connection, store) = database();
        put(&connection, &store, 1.0, 10, &keys("by_name", vec![text("Ann")])).unwrap();
        put(&connection, &store, 2.0, 20, &keys("by_name", vec![text("Ann")])).unwrap();
        assert_eq!(rows(&connection), 0);
    }

    #[test]
    fn a_put_with_no_unique_keys_touches_no_rows() {
        let (connection, store) = database();
        put(&connection, &store, 1.0, 10, &keys("by_email", vec![text("a@x")])).unwrap();
        // A store with no unique index sends nothing: the rows of other records are not touched.
        put(&connection, &store, 2.0, 20, &[]).unwrap();
        assert_eq!(rows(&connection), 1);
    }
}
