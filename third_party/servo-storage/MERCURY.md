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
- **Tested by:** 24 unit tests in `weblocks/mod.rs` and 13 thread tests in `tests/weblocks.rs`; `tests/indexeddb_multientry.rs`
  (new, PolyForm) pins the facts about the `index_data` and `unique_index_data` tables that the IndexedDB multiEntry work
  relies on (it includes `indexeddb/engines/sqlite/create.rs` by `#[path]`). Run with `make test-servo-storage` (the crate has
  dev-dependencies, so cargo runs it on its own manifest, with `servo-storage-traits` patched in).
- **Drop when:** the Servo bump that contains PR #47981.

`Cargo.lock` of this crate is changed the same way (`servo-storage-traits` as a path package; listed as `unmarked` in `third_party/vendor.toml`), so that `make test-servo-storage` runs against the vendored traits crate.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-storage.patch` and this directory
once Mercury no longer needs to change the crate.
