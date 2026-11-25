/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: the Web Locks thread, started by itself and talked to the way the script thread talks
//! to it: requests with a callback, the answers on that callback. The scenarios of exclusion,
//! sharing, `ifAvailable` and `steal` follow the registry tests of LunarWerxs `pr/web-locks`
//! (commit 23583d753, `components/script/dom/weblocks/registry.rs`, L0garithmic, MPL-2.0), written
//! again for the storage thread of servo/servo pull request #47981 (xbwwj, Xubai Wang). The
//! rules themselves are tested without a thread in the unit tests of `weblocks/mod.rs`.
//!
//! What this file adds: a client whose global scope is closed (a tab that was closed, a worker
//! that ended) loses its locks, by the message it sends and by the answer it gives to a probe.
//!
//! What it cannot run: a client in another process that dies. In one process the callback of a
//! client is a closure, and sending to it cannot fail; the unit tests make a client unreachable,
//! and the end to end case needs multi-process mode in a real window (an open item of the lane).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use profile::mem as profile_mem;
use servo_base::generic_channel::{GenericCallback, GenericSender, SendError};
use servo_default_resources as _;
use servo_url::{ImmutableOrigin, ServoUrl};
use storage::WebLocksThreadFactory;
use storage_traits::weblocks::{
    LockEventMsg, LockId, LockManagerSnapshotMsg, LockModeMsg, LockMsg, LockRequest, LockRequestId,
    WebLocksThreadMsg,
};

/// The interval of the probes: short, so that the tests that wait for one are quick.
const PROBE_INTERVAL: Duration = Duration::from_millis(50);
/// How long a test waits for something that must happen.
const LONG: Duration = Duration::from_secs(10);
/// How long a test waits to see that something does not happen.
const SHORT: Duration = Duration::from_millis(300);

fn start() -> GenericSender<WebLocksThreadMsg> {
    let mem_profiler_chan = profile_mem::Profiler::create();
    <GenericSender<WebLocksThreadMsg> as WebLocksThreadFactory>::with_probe_interval(
        mem_profiler_chan,
        "web-locks-test".to_owned(),
        PROBE_INTERVAL,
    )
}

fn origin(url: &str) -> ImmutableOrigin {
    ServoUrl::parse(url).unwrap().origin()
}

/// A window or a worker, as the storage thread sees it: an id, and a callback whose events the
/// test reads.
struct Client {
    id: String,
    storage: GenericSender<WebLocksThreadMsg>,
    origin: ImmutableOrigin,
    events: Receiver<LockEventMsg>,
    callback: GenericCallback<LockEventMsg>,
    /// What the callback of a real client reads from its task source: the global is closed.
    closed: Arc<AtomicBool>,
}

impl Client {
    fn new(storage: &GenericSender<WebLocksThreadMsg>, id: &str, origin: &ImmutableOrigin) -> Self {
        let (tx, events) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let callback = {
            let closed = closed.clone();
            let storage = storage.clone();
            let id = id.to_owned();
            GenericCallback::new(move |event: Result<LockEventMsg, SendError>| {
                let event = event.expect("a readable event");
                // The same decision as the callback of `LockManager::request_lock`: a closed
                // global is reported gone, whatever the event, and a probe of a live one is
                // ignored.
                if closed.load(Ordering::SeqCst) {
                    let _ = storage.send(WebLocksThreadMsg::ClientGone(id.clone()));
                } else if !matches!(event, LockEventMsg::Probe) {
                    let _ = tx.send(event);
                }
            })
            .unwrap()
        };
        Client {
            id: id.to_owned(),
            storage: storage.clone(),
            origin: origin.clone(),
            events,
            callback,
            closed,
        }
    }

    fn ask(&self, name: &str, mode: LockModeMsg, if_available: bool, steal: bool) -> LockRequestId {
        let id = LockRequestId::next();
        self.storage
            .send(WebLocksThreadMsg::Request(
                LockRequest {
                    id,
                    client_id: self.id.clone(),
                    name: name.to_owned(),
                    mode,
                    callback: self.callback.clone(),
                    if_available,
                    steal,
                },
                self.origin.clone(),
            ))
            .unwrap();
        id
    }

    fn request(&self, name: &str) -> LockRequestId {
        self.ask(name, LockModeMsg::Exclusive, false, false)
    }

    fn request_shared(&self, name: &str) -> LockRequestId {
        self.ask(name, LockModeMsg::Shared, false, false)
    }

    fn request_if_available(&self, name: &str) -> LockRequestId {
        self.ask(name, LockModeMsg::Exclusive, true, false)
    }

    fn steal(&self, name: &str) -> LockRequestId {
        self.ask(name, LockModeMsg::Exclusive, false, true)
    }

    fn release(&self, lock: &LockMsg) {
        self.storage
            .send(WebLocksThreadMsg::Release(
                lock.id,
                lock.name.clone(),
                self.origin.clone(),
            ))
            .unwrap();
    }

    fn abort(&self, name: &str, request: LockRequestId) {
        self.storage
            .send(WebLocksThreadMsg::Abort(
                request,
                self.origin.clone(),
                name.to_owned(),
            ))
            .unwrap();
    }

    /// The next thing the storage thread told this client.
    fn next(&self) -> LockEventMsg {
        self.events
            .recv_timeout(LONG)
            .expect("the storage thread should have said something")
    }

    fn expect_granted(&self, name: &str) -> LockMsg {
        match self.next() {
            LockEventMsg::Granted(Some(lock)) => {
                assert_eq!(lock.name, name);
                lock
            },
            other => panic!("expected a grant of {name:?}, got {other:?}"),
        }
    }

    fn expect_unavailable(&self) {
        match self.next() {
            LockEventMsg::Granted(None) => {},
            other => panic!("expected null for ifAvailable, got {other:?}"),
        }
    }

    fn expect_stolen(&self) {
        match self.next() {
            LockEventMsg::Stolen => {},
            other => panic!("expected the lock to be stolen, got {other:?}"),
        }
    }

    fn expect_silence(&self) {
        match self.events.recv_timeout(SHORT) {
            Err(RecvTimeoutError::Timeout) => {},
            other => panic!("expected nothing, got {other:?}"),
        }
    }

    /// The tab was closed, or the worker ended.
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn goes_away(&self) {
        self.storage
            .send(WebLocksThreadMsg::ClientGone(self.id.clone()))
            .unwrap();
    }
}

fn snapshot(
    storage: &GenericSender<WebLocksThreadMsg>,
    origin: &ImmutableOrigin,
) -> LockManagerSnapshotMsg {
    let (callback, receiver) = GenericCallback::new_blocking().unwrap();
    storage
        .send(WebLocksThreadMsg::Query(callback, origin.clone()))
        .unwrap();
    receiver.recv().expect("a snapshot")
}

fn held_by(snapshot: &LockManagerSnapshotMsg) -> Vec<(String, String)> {
    let mut held: Vec<_> = snapshot
        .held
        .iter()
        .map(|info| (info.name.clone(), info.client_id.clone()))
        .collect();
    held.sort();
    held
}

#[test]
fn an_exclusive_lock_excludes_and_is_handed_over() {
    let storage = start();
    let site = origin("https://example.com");
    let (a, b) = (
        Client::new(&storage, "a", &site),
        Client::new(&storage, "b", &site),
    );

    a.request("x");
    let lock_a = a.expect_granted("x");

    b.request("x");
    b.expect_silence();

    a.release(&lock_a);
    b.expect_granted("x");
}

#[test]
fn shared_locks_share_and_wait_for_an_exclusive_one() {
    let storage = start();
    let site = origin("https://example.com");
    let (a, b, c) = (
        Client::new(&storage, "a", &site),
        Client::new(&storage, "b", &site),
        Client::new(&storage, "c", &site),
    );

    a.request_shared("x");
    b.request_shared("x");
    let lock_a = a.expect_granted("x");
    let lock_b = b.expect_granted("x");

    c.request("x");
    c.expect_silence();

    a.release(&lock_a);
    c.expect_silence();
    b.release(&lock_b);
    c.expect_granted("x");
}

#[test]
fn if_available_answers_null_when_the_lock_is_taken_and_does_not_queue() {
    let storage = start();
    let site = origin("https://example.com");
    let (a, b) = (
        Client::new(&storage, "a", &site),
        Client::new(&storage, "b", &site),
    );

    a.request("x");
    let lock_a = a.expect_granted("x");

    b.request_if_available("x");
    b.expect_unavailable();

    a.release(&lock_a);
    b.expect_silence();
}

#[test]
fn if_available_takes_a_free_lock() {
    let storage = start();
    let site = origin("https://example.com");
    let a = Client::new(&storage, "a", &site);
    a.request_if_available("x");
    a.expect_granted("x");
}

#[test]
fn steal_takes_the_lock_and_tells_the_holder() {
    let storage = start();
    let site = origin("https://example.com");
    let (victim, waiter, thief) = (
        Client::new(&storage, "victim", &site),
        Client::new(&storage, "waiter", &site),
        Client::new(&storage, "thief", &site),
    );

    victim.request("x");
    let stolen_lock = victim.expect_granted("x");
    waiter.request("x");
    waiter.expect_silence();

    thief.steal("x");
    victim.expect_stolen();
    let lock_thief = thief.expect_granted("x");
    waiter.expect_silence();

    // The victim's callback ends later, and releases what it no longer has.
    victim.release(&stolen_lock);
    waiter.expect_silence();

    thief.release(&lock_thief);
    waiter.expect_granted("x");
}

#[test]
fn an_aborted_request_is_never_granted() {
    let storage = start();
    let site = origin("https://example.com");
    let (a, b, c) = (
        Client::new(&storage, "a", &site),
        Client::new(&storage, "b", &site),
        Client::new(&storage, "c", &site),
    );

    a.request("x");
    let lock_a = a.expect_granted("x");
    let request_b = b.request("x");
    c.request("x");

    b.abort("x", request_b);
    a.release(&lock_a);

    c.expect_granted("x");
    b.expect_silence();
}

#[test]
fn names_and_origins_are_independent() {
    let storage = start();
    let (site, other_site) = (origin("https://example.com"), origin("https://example.org"));
    let (a, b, c) = (
        Client::new(&storage, "a", &site),
        Client::new(&storage, "b", &site),
        Client::new(&storage, "c", &other_site),
    );

    a.request("x");
    a.expect_granted("x");
    b.request("y");
    b.expect_granted("y");
    c.request("x");
    c.expect_granted("x");
}

#[test]
fn query_lists_what_is_held_and_what_is_pending() {
    let storage = start();
    let site = origin("https://example.com");
    let (a, b) = (
        Client::new(&storage, "a", &site),
        Client::new(&storage, "b", &site),
    );

    a.request("x");
    a.expect_granted("x");
    b.request_shared("x");
    b.expect_silence();

    let snapshot = snapshot(&storage, &site);
    assert_eq!(held_by(&snapshot), vec![("x".to_owned(), "a".to_owned())]);
    assert_eq!(snapshot.pending.len(), 1);
    assert_eq!(snapshot.pending[0].client_id, "b");
    assert_eq!(snapshot.pending[0].mode, LockModeMsg::Shared);

    assert!(
        self::snapshot(&storage, &origin("https://example.org"))
            .held
            .is_empty(),
        "another origin sees nothing"
    );
}

#[test]
fn releasing_a_lock_that_was_never_held_changes_nothing() {
    let storage = start();
    let site = origin("https://example.com");
    let (a, b) = (
        Client::new(&storage, "a", &site),
        Client::new(&storage, "b", &site),
    );
    a.request("x");
    a.expect_granted("x");
    b.request("x");

    a.release(&LockMsg {
        id: LockId::next(),
        name: "x".to_owned(),
        mode: LockModeMsg::Exclusive,
    });
    b.expect_silence();
}

#[test]
fn a_client_that_goes_away_loses_its_locks_and_its_requests() {
    // The window was torn down and its manager said so (`WebLocksThreadMsg::ClientGone`).
    let storage = start();
    let site = origin("https://example.com");
    let (gone, waiter) = (
        Client::new(&storage, "gone", &site),
        Client::new(&storage, "waiter", &site),
    );

    gone.request("x");
    gone.expect_granted("x");
    gone.request("y");
    gone.expect_granted("y");
    waiter.request("x");
    waiter.expect_silence();
    gone.request("z");
    gone.expect_granted("z");
    waiter.request("z");
    waiter.expect_silence();
    // And a request of the client that is still queued behind the waiter.
    gone.request("x");

    gone.goes_away();

    waiter.expect_granted("x");
    waiter.expect_granted("z");
    let snapshot = snapshot(&storage, &site);
    assert_eq!(
        held_by(&snapshot),
        vec![
            ("x".to_owned(), "waiter".to_owned()),
            ("z".to_owned(), "waiter".to_owned())
        ]
    );
    assert!(snapshot.pending.is_empty(), "nothing of `gone` is queued");
}

#[test]
fn a_client_whose_global_is_closed_loses_its_lock_when_someone_waits() {
    // The window was discarded or the worker closed, and nobody told the storage thread: the
    // callback of the client finds its task source cancelled when the thread probes it.
    let storage = start();
    let site = origin("https://example.com");
    let (closed, waiter) = (
        Client::new(&storage, "closed", &site),
        Client::new(&storage, "waiter", &site),
    );

    closed.request("x");
    closed.expect_granted("x");
    closed.close();

    waiter.request("x");
    // Within a few probe intervals, not never.
    waiter.expect_granted("x");
    assert_eq!(
        held_by(&snapshot(&storage, &site)),
        vec![("x".to_owned(), "waiter".to_owned())]
    );
}

#[test]
fn a_request_of_a_closed_global_is_dropped_before_it_is_granted() {
    let storage = start();
    let site = origin("https://example.com");
    let (holder, closed, next) = (
        Client::new(&storage, "holder", &site),
        Client::new(&storage, "closed", &site),
        Client::new(&storage, "next", &site),
    );

    holder.request("x");
    let lock_holder = holder.expect_granted("x");
    closed.request("x");
    next.request("x");
    closed.close();

    // The probes find `closed` while it waits, so `next` is not stuck behind a grant that
    // nobody will use, whether the holder releases first or not.
    holder.release(&lock_holder);
    next.expect_granted("x");
}

#[test]
fn the_probes_stop_when_nobody_waits() {
    // A held lock with nobody waiting is left alone: closing its global changes nothing until a
    // request comes. (There is no timer to observe: this checks that the thread still answers.)
    let storage = start();
    let site = origin("https://example.com");
    let a = Client::new(&storage, "a", &site);
    a.request("x");
    a.expect_granted("x");
    a.close();
    std::thread::sleep(SHORT);
    assert_eq!(
        held_by(&snapshot(&storage, &site)),
        vec![("x".to_owned(), "a".to_owned())]
    );
}
