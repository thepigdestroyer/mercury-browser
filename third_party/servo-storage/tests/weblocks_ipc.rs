// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: tests of the Web Locks thread against a real IPC channel (net-1 review, finding 4). A new Mercury file;
// nothing in it is copied from Servo or another project.

//! The Web Locks thread with a **real** `GenericCallback`: an `ipc_channel` pair, not a closure.
//!
//! `tests/weblocks.rs` runs in one process, where a client's callback is a closure and sending to it cannot fail,
//! and the unit tests in `weblocks/mod.rs` use a fake client that the test makes unreachable. Neither shows that a
//! callback whose receiving end is gone (a content process that died) makes `GenericCallback::send` fail, which is
//! how the storage thread learns that a holder is dead. This binary asks for the IPC variant
//! (`opts.force_ipc`, which needs the `multiprocess` feature of `servo-base`: a dev-dependency of this crate), so
//! it is a test target of its own: the choice is made once per process, before the first channel is made.
//!
//! The "process" that dies is simulated by dropping the `GenericReceiver` that the client's callback sends to. The
//! end-to-end case, a real content process killed while it holds a lock, still needs `--multiprocess` in a window.

use std::sync::Once;
use std::time::{Duration, Instant};

use profile::mem as profile_mem;
use servo_base::generic_channel::{GenericCallback, GenericReceiver, GenericSender};
use servo_config::opts::{Opts, initialize_options};
use servo_default_resources as _;
use servo_url::{ImmutableOrigin, ServoUrl};
use storage::WebLocksThreadFactory;
use storage_traits::weblocks::{
    LockEventMsg, LockManagerSnapshotMsg, LockModeMsg, LockRequest, LockRequestId,
    WebLocksThreadMsg,
};

const PROBE_INTERVAL: Duration = Duration::from_millis(50);
const LONG: Duration = Duration::from_secs(10);
const SHORT: Duration = Duration::from_millis(300);

fn use_ipc() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        initialize_options(Opts {
            force_ipc: true,
            ..Opts::default()
        });
    });
}

fn start() -> GenericSender<WebLocksThreadMsg> {
    use_ipc();
    <GenericSender<WebLocksThreadMsg> as WebLocksThreadFactory>::with_probe_interval(
        profile_mem::Profiler::create(),
        "web-locks-ipc-test".to_owned(),
        PROBE_INTERVAL,
    )
}

fn origin() -> ImmutableOrigin {
    ServoUrl::parse("https://example.test/").unwrap().origin()
}

/// A client whose callback is an IPC channel: the storage thread sends to `callback`, the test reads `events`.
struct IpcClient {
    id: String,
    callback: GenericCallback<LockEventMsg>,
    events: GenericReceiver<LockEventMsg>,
}

impl IpcClient {
    fn new(id: &str) -> IpcClient {
        let (callback, events) =
            GenericCallback::<LockEventMsg>::new_blocking().expect("an IPC callback");
        IpcClient {
            id: id.to_owned(),
            callback,
            events,
        }
    }

    fn request(&self, storage: &GenericSender<WebLocksThreadMsg>, name: &str) {
        storage
            .send(WebLocksThreadMsg::Request(
                LockRequest {
                    id: LockRequestId::next(),
                    client_id: self.id.clone(),
                    name: name.to_owned(),
                    mode: LockModeMsg::Exclusive,
                    callback: self.callback.clone(),
                    if_available: false,
                    steal: false,
                },
                origin(),
            ))
            .unwrap();
    }

    fn expect_granted(&self, name: &str) {
        // A waiting client is probed every interval: the probes come before the grant.
        match self.next_other_than_probe(LONG) {
            Some(LockEventMsg::Granted(Some(lock))) => assert_eq!(lock.name, name),
            Some(other) => panic!("expected a grant of {name:?}, got {other:?}"),
            None => panic!("expected a grant of {name:?}, got nothing in time"),
        }
    }

    /// Reads what the storage thread sent, skipping probes, until `timeout` passes.
    fn next_other_than_probe(&self, timeout: Duration) -> Option<LockEventMsg> {
        let deadline = Instant::now() + timeout;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.events.try_recv_timeout(left) {
                Ok(LockEventMsg::Probe) => continue,
                Ok(event) => return Some(event),
                Err(_) => return None,
            }
        }
        None
    }
}

fn query(storage: &GenericSender<WebLocksThreadMsg>) -> LockManagerSnapshotMsg {
    let (callback, receiver) =
        GenericCallback::<LockManagerSnapshotMsg>::new_blocking().expect("an IPC callback");
    storage
        .send(WebLocksThreadMsg::Query(callback, origin()))
        .unwrap();
    match receiver.try_recv_timeout(LONG) {
        Ok(snapshot) => snapshot,
        Err(_) => panic!("the storage thread answers a query"),
    }
}

/// A holder whose process dies (its receiver is dropped) loses its lock, and the request that waited for it is granted.
#[test]
fn a_lock_is_released_when_the_receiving_end_of_its_holders_callback_is_dropped() {
    let storage = start();
    let holder = IpcClient::new("holder");
    let waiter = IpcClient::new("waiter");

    holder.request(&storage, "x");
    holder.expect_granted("x");
    waiter.request(&storage, "x");
    // While the holder lives, the waiter waits (probes of a live holder are answered by nobody and change nothing).
    assert!(
        waiter.next_other_than_probe(SHORT).is_none(),
        "the lock is still held"
    );

    // The content process dies: nobody reads the holder's channel any more.
    let IpcClient {
        callback, events, ..
    } = holder;
    drop(events);

    // The storage thread's next probe (every 50 ms while somebody waits) cannot send: the holder is dead.
    waiter.expect_granted("x");
    drop(callback);
}

/// `query()` does not list the lock of a holder that died, although nobody waits for it and no periodic probe runs.
#[test]
fn query_does_not_list_the_lock_of_a_dead_holder() {
    let storage = start();
    let holder = IpcClient::new("holder");
    let alive = IpcClient::new("alive");
    holder.request(&storage, "x");
    holder.expect_granted("x");
    alive.request(&storage, "y");
    alive.expect_granted("y");

    let snapshot = query(&storage);
    let mut held: Vec<_> = snapshot.held.iter().map(|lock| lock.name.clone()).collect();
    held.sort();
    assert_eq!(held, ["x", "y"], "both holders are alive");

    let IpcClient { events, .. } = holder;
    drop(events);

    // The first query after the death finds it out by probing; allow the channel a moment to report it.
    let deadline = Instant::now() + LONG;
    let held = loop {
        let held: Vec<String> = query(&storage)
            .held
            .into_iter()
            .map(|lock| lock.name)
            .collect();
        if held == ["y"] || Instant::now() > deadline {
            break held;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(held, ["y"], "the dead holder's lock is gone, the live one stays");
}
