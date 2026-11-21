# Patched servo-storage-traits 0.8.0

This directory vendors `servo-storage-traits` 0.8.0 under its own licence (`MPL-2.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`e74add8ac3dcd4bd29c91f6f06e4aaeb9d047e903d74d3324d4dbfe8d3447640`), built from https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/shared/storage`,
plus Mercury's changes in `third_party/patches/servo-storage-traits.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff servo-storage-traits` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Changes

Changes made by lane net-1 (Web Locks, ED4).

### The Web Locks thread's messages — `lib.rs`, new `weblocks.rs`

- **What:** `weblocks.rs` (new, MPL-2.0 like the rest; it carries upstream code, see below) defines `WebLocksThreadMsg`, `LockRequest`,
  `LockEventMsg`, `LockMode` and the snapshot types. `lib.rs` has `pub mod weblocks;`, a fifth `StorageThreads` field
  `web_locks_thread` (and constructor argument) and `impl GenericSend<WebLocksThreadMsg> for StorageThreads` (0.8.0 lines about
  10, 14, 27, 37 and 150).
- **Why:** `navigator.locks` (WhatsApp Web hangs at "Loading your chats" without it, servo/servo#47893). The lock table has
  to live outside the content process so that locks span tabs and processes.
- **Source:** servo/servo#47981 "storages: implement Web Locks API" (draft, xbwwj, MPL-2.0), branch head `a900121b`, the
  storage-thread design; the file says so at its top.
- **Tested by:** `servo-storage/tests/weblocks.rs` and the unit tests in `servo-storage/weblocks/mod.rs` (`make test-servo-storage`).
- **Drop when:** the Servo bump that contains PR #47981 (or its successor).

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/servo-storage-traits.patch` and this directory
once Mercury no longer needs to change the crate.
