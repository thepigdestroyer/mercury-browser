/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: carried from servo/servo pull request #47981 "storages: implement Web Locks API"
// (draft, by xbwwj, Xubai Wang), branch `weblocks` of xbwwj/servo, head commit a900121b
// (`components/shared/storage/weblocks.rs`), MPL-2.0. Where this file differs from the pull
// request, the line carries a `Mercury:` comment: `LockRequest` has one callback instead of two
// and `LockEventMsg` says what the storage thread tells a client; `ClientGone` releases what a
// client holds when its window, worker or process goes away.

use std::sync::atomic::{AtomicUsize, Ordering};

use malloc_size_of_derive::MallocSizeOf;
use profile_traits::mem::ReportsChan;
use serde::{Deserialize, Serialize};
use servo_base::generic_channel::GenericCallback;
use servo_url::ImmutableOrigin;
use uuid::Uuid;

static LOCK_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, MallocSizeOf, PartialEq, Serialize)]
pub struct LockId(usize);

impl LockId {
    pub fn next() -> Self {
        Self(LOCK_ID.fetch_add(1, Ordering::Relaxed))
    }
}

// TODO: once servo implements client id, this can be changed to usize
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, MallocSizeOf, PartialEq, Serialize)]
pub struct LockRequestId(Uuid);

impl LockRequestId {
    pub fn next() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum WebLocksThreadMsg {
    Request(LockRequest, ImmutableOrigin),
    Query(GenericCallback<LockManagerSnapshotMsg>, ImmutableOrigin),
    /// A held lock is released: its id, its name and its origin.
    Release(LockId, String, ImmutableOrigin),
    Abort(LockRequestId, ImmutableOrigin, String),
    /// Mercury: the client with this id (a window or a worker, see `LockRequest::client_id`) is
    /// gone. Every lock it holds is released and every request it has queued is dropped. Sent by
    /// the client itself when it is torn down, and by its callback when it finds the global scope
    /// closed (`LockEventMsg::Probe`). The storage thread also acts on it by itself when it can
    /// no longer reach a client's callback (its process died).
    ClientGone(String),
    CollectMemoryReport(ReportsChan),
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct LockManagerSnapshotMsg {
    pub held: Vec<LockInfoMsg>,
    pub pending: Vec<LockInfoMsg>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LockInfoMsg {
    pub name: String,
    pub mode: LockModeMsg,
    pub client_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, MallocSizeOf, PartialEq, Serialize)]
pub enum LockModeMsg {
    Shared,
    Exclusive,
}

// Mercury: `Clone` added (a `LockEventMsg` is cloned in tests).
#[derive(Clone, Debug, Deserialize, MallocSizeOf, Serialize)]
pub struct LockMsg {
    pub id: LockId,
    pub name: String,
    pub mode: LockModeMsg,
}

/// Mercury: what the storage thread tells the client of a lock request, over `LockRequest::callback`.
///
/// The pull request has two callbacks (`held_callback: GenericCallback<Option<LockMsg>>` and
/// `released_callback: GenericCallback<bool>`) and a stolen lock sends `false` to the second.
/// One callback carries all of it here, so that a client that is gone can be found by one probe.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum LockEventMsg {
    /// The request was granted (`Some`), or it asked for `ifAvailable` and the lock is not
    /// available (`None`): the client runs its callback with the lock, or with null.
    Granted(Option<LockMsg>),
    /// The lock this request holds was stolen by a `steal` request: reject the promise of the
    /// request with an `AbortError`.
    Stolen,
    /// Is this client still alive? Sent on a timer while another request waits behind something
    /// this client holds or has queued. A client whose global scope is closed answers with
    /// `WebLocksThreadMsg::ClientGone`; any other client ignores it. A client that cannot be
    /// reached at all (its process died) is found by the failed send.
    Probe,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LockRequest {
    pub id: LockRequestId,
    /// Names the window or worker that asked, in `query()` results and in
    /// `WebLocksThreadMsg::ClientGone`. Unique per global scope.
    pub client_id: String,
    pub name: String,
    pub mode: LockModeMsg,
    /// Mercury: replaces the pull request's `held_callback` and `released_callback`.
    pub callback: GenericCallback<LockEventMsg>,
    pub if_available: bool,
    pub steal: bool,
}
