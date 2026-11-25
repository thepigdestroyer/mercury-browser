/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

pub mod cache_storage;
pub mod client_storage;
mod indexeddb;
pub(crate) mod shared;
mod storage_thread;
// Mercury: Web Locks (servo/servo#47981, see weblocks/mod.rs).
mod weblocks;
mod webstorage;

pub use cache_storage::CacheStorageThreadFactory;
pub use client_storage::ClientStorageThreadFactory;
pub(crate) use indexeddb::IndexedDBThreadFactory;
pub use storage_thread::new_storage_threads;
// Mercury: public so that the tests can start the Web Locks thread by itself (tests/weblocks.rs).
pub use weblocks::WebLocksThreadFactory;
pub(crate) use webstorage::WebStorageThreadFactory;
