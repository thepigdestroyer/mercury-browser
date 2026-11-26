# Patched servo-storage 0.8.0

This directory vendors `servo-storage` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`346be02eac3b3be12d17a806b56076f2cb706adbe2aa1b3331964a77ec3a5856`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/storage`,
plus Mercury's changes in `third_party/patches/servo-storage.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-storage` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane net-1 (Web Locks, ED4; the IndexedDB schema facts, ED3).

### The Web Locks thread — `lib.rs`, `storage_thread.rs`, new `weblocks/mod.rs`

- **What:** `weblocks/mod.rs` (new, MPL-2.0, carries upstream code) is the lock manager and its thread: exclusive and
  shared locks per origin and name, a queue, `ifAvailable`, `steal`, abort, `query()`, and release of everything a client
  holds when it goes away. `lib.rs` has `mod weblocks;` and `pub use weblocks::WebLocksThreadFactory;` (public so the
  integration test can start the thread). `storage_thread.rs` starts the thread beside the others and passes its sender as
  the fifth argument of `StorageThreads::new` (0.8.0 lines about 12, 14, 34, 38 and 44).
- **How a held lock is released when its window, worker or process goes away:** (1) the `LockManager` in the content
  process sends `ClientGone` when it is finalized; (2) while a request waits, the thread probes every second and a client
  whose global is closed answers `ClientGone`; (3) a failed `GenericCallback::send` (the receiving process died) counts as
  "client gone" at any moment. No edit of `servo-constellation`.
- **Why:** `navigator.locks`, servo/servo#47893.
- **Source:** servo/servo#47981 (xbwwj, draft, MPL-2.0, head `a900121b`) for the storage-thread design; LunarWerxs
  `pr/web-locks` commit `23583d753` (MPL-2.0, written with an AI assistant) for the idea that a lock lives as long as the
  promise of its callback and for the scenarios of the tests, rewritten here. The draft did not work as written (a lock was
  never released, `ifAvailable` was inverted, `steal` told nobody, the argument checks never all ran); the differences are
  listed in the lane's notes and the module comment.
- **`query()` probes first:** `LockManager::snapshot_lock_state` probes every client that holds a lock or has a request queued
  and drops the ones that cannot be reached, so a lock whose holder died is not listed (the periodic probe runs only while
  somebody waits, which a `query()` alone does not) (net-1 review, finding 4).
- **Tested by:** unit tests in `weblocks/mod.rs` (with a fake client that becomes unreachable, among them
  `the_snapshot_does_not_list_the_locks_of_a_client_that_cannot_be_reached`) and 13 thread tests in `tests/weblocks.rs`; and
  `tests/weblocks_ipc.rs`, which runs the thread with a **real** `GenericCallback` (an `ipc_channel` pair, `opts.force_ipc`, a
  test target of its own with `servo-base`'s `multiprocess` feature as a dev-dependency): the receiving end of a holder's callback
  is dropped and the lock is released (the waiter is granted), and `query()` stops listing the dead holder's lock. A real content
  process killed while it holds a lock (`--multiprocess`, in a window) is still an open item, and `dom_web_locks_enabled` stays off.
  Run with `make test-servo-storage` (the crate has dev-dependencies, so cargo runs it on its own manifest, with
  `servo-storage-traits` patched in).

### Unique indexes (ED3) — `indexeddb/engines/sqlite.rs`, new `indexeddb/engines/sqlite/unique_index_rows.rs`

- **What:** `put_item` takes the keys of the record in each unique index of its store (`UniqueIndexKeys`, from `servo-script`) and
  `unique_index_rows.rs` keeps them in `unique_index_data`, whose primary key `(index_id, value)` makes a second record with the
  same key in the same index a SQLite constraint violation, which the script reports as a `ConstraintError`. A put runs in one
  savepoint, so a refused put leaves nothing behind (not the record, not a row, not the key generator's number); a put again of
  a record replaces its rows; `delete_item`, `clear` and `delete_index` remove them (a lookup by record is created on first use so that a
  bulk load is not quadratic); a multiEntry index has one row per member. 0.8.0 lines about 237 (`put_item`), 280 and 294
  (`delete_item`, `clear`), 432 (`delete_index`), 551 (the `PutItem` arm).
- **Limits:** a unique index created on a store that already has records does not see them (nothing backfills the rows), and neither
  do records written before this change, so a duplicate of an old record is not caught. Non-unique indexes have no rows; their
  queries are answered by the script (`servo-script/MERCURY.md`, change 15).
- **Why:** Dexie-style `&email` schemas silently stored duplicates (net-1 review, finding 3).
- **Tested by:** the unit tests of `unique_index_rows.rs` (a duplicate is refused and changes nothing, an update replaces its rows,
  multiEntry members are unique across records, delete, range delete, clear and delete-index free the keys), which put records through the
  engine's own `put_item`, `delete_item`, `clear` and `delete_index` on an in-memory database made with the engine's own
  `create_tables`. `tests/indexeddb_schema_facts.rs` (formerly `indexeddb_multientry.rs`) pins facts about SQLite and Servo's schema
  by hand-written SQL, calls no Mercury code and is **not** evidence for ED3.
- **Drop when:** Servo maintains real index rows (servo#47893's gaps).
- **Drop when:** the Servo bump that contains PR #47981.

`Cargo.lock` of this crate is changed the same way (`servo-storage-traits` as a path package; listed as `unmarked` in `third_party/vendor.toml`), so that `make test-servo-storage` runs against the vendored traits crate.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-storage.patch` and this directory
once Mercury no longer needs to change the crate.
