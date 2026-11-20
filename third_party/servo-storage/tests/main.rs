/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

mod cache_storage;
mod client_storage;
// Mercury: the multiEntry rows of the IndexedDB item (tests/indexeddb_multientry.rs, item 4).
mod indexeddb_multientry;
mod storage_thread;
// Mercury: Web Locks (tests/weblocks.rs).
mod weblocks;
mod webstorage;
