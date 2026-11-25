/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The Web Locks thread: one lock manager per origin, shared by every window and worker of that
//! origin in every content process.
//!
//! Mercury: carried from servo/servo pull request #47981 "storages: implement Web Locks API"
//! (draft, by xbwwj, Xubai Wang), branch `weblocks` of xbwwj/servo, head commit a900121b
//! (`components/storage/weblocks/mod.rs`), MPL-2.0. The queue, the held set and the algorithm
//! names come from there. What differs is listed here, and each place carries a `Mercury:`
//! comment.
//!
//! - The pull request's `request_lock` tested `ifAvailable` backwards (it answered "not
//!   available" when the lock was grantable, and never when it was not) and computed "is first"
//!   from a request that was not yet in the queue, so every request waited. Both follow the
//!   specification here.
//! - A stolen lock tells its client (`LockEventMsg::Stolen`), so that the client can reject the
//!   promise of the `request()` call with an `AbortError`; the pull request sent `false` to a
//!   callback nobody read.
//! - Nothing in the pull request released a lock when its client went away. Here a client that
//!   cannot be reached (its process died, which shows as a failed send), a client that says it is
//!   gone (`WebLocksThreadMsg::ClientGone`) and a client that answers a `Probe` that its global
//!   scope is closed all lose what they hold and what they queued. The thread probes on a timer,
//!   and only while some request is waiting: that is the only time a lock held by a client that is
//!   gone matters.
//! - The lock state is a type of its own, generic over the client, so that these rules are tested
//!   without a thread or a channel (the tests at the end of this file and `tests/weblocks.rs`).
//! - The thread ends when its channel closes instead of panicking.
//!
//! Locks are keyed by origin. The storage partition key arrives with EC8.

use std::collections::VecDeque;
use std::thread;
use std::time::{Duration, Instant};

use log::warn;
use profile_traits::mem::{ProcessReports, ProfilerChan as MemProfilerChan, Report};
use rustc_hash::FxHashMap;
use servo_base::generic_channel::{
    self, GenericCallback, GenericReceiver, GenericSender, ReceiveError, TryReceiveError,
};
use servo_url::ImmutableOrigin;
use storage_traits::weblocks::{
    LockEventMsg, LockId, LockInfoMsg, LockManagerSnapshotMsg, LockModeMsg, LockMsg, LockRequest,
    LockRequestId, WebLocksThreadMsg,
};

/// How often the thread asks the clients it is waiting behind whether they are alive, while a
/// request is waiting.
const PROBE_INTERVAL: Duration = Duration::from_secs(1);

pub trait WebLocksThreadFactory {
    fn new(mem_profiler_chan: MemProfilerChan, reporter_name: String) -> Self;

    /// Mercury: like `new`, with the interval of the liveness probes. The tests use a short one.
    fn with_probe_interval(
        mem_profiler_chan: MemProfilerChan,
        reporter_name: String,
        probe_interval: Duration,
    ) -> Self;
}

impl WebLocksThreadFactory for GenericSender<WebLocksThreadMsg> {
    fn new(mem_profiler_chan: MemProfilerChan, reporter_name: String) -> Self {
        Self::with_probe_interval(mem_profiler_chan, reporter_name, PROBE_INTERVAL)
    }

    fn with_probe_interval(
        mem_profiler_chan: MemProfilerChan,
        reporter_name: String,
        probe_interval: Duration,
    ) -> Self {
        let (chan, port) = generic_channel::channel().unwrap();
        let chan2 = chan.clone();
        thread::Builder::new()
            .name("WebLocksManager".to_owned())
            .spawn(move || {
                mem_profiler_chan.run_with_memory_reporting(
                    || WebLocksManager::new(port, probe_interval).start(),
                    reporter_name,
                    chan2,
                    WebLocksThreadMsg::CollectMemoryReport,
                );
            })
            .expect("thread spawning failed");
        chan
    }
}

struct WebLocksManager {
    port: GenericReceiver<WebLocksThreadMsg>,
    /// Per-origin managers,
    managers: FxHashMap<ImmutableOrigin, LockManager<GenericCallback<LockEventMsg>>>,
    probe_interval: Duration,
}

impl WebLocksManager {
    fn new(port: GenericReceiver<WebLocksThreadMsg>, probe_interval: Duration) -> Self {
        Self {
            port,
            managers: Default::default(),
            probe_interval,
        }
    }

    fn start(&mut self) {
        let mut last_probe = Instant::now();
        loop {
            // Mercury: while a request waits, wake up when the next probe is due. Otherwise there
            // is nothing to do until a message arrives.
            let had_waiters = self.has_waiters();
            let message = if had_waiters {
                let wait = self.probe_interval.saturating_sub(last_probe.elapsed());
                match self.port.try_recv_timeout(wait) {
                    Ok(message) => Some(message),
                    Err(TryReceiveError::Empty) => None,
                    Err(TryReceiveError::ReceiveError(ReceiveError::DeserializationFailed(
                        error,
                    ))) => {
                        warn!("A Web Locks message could not be read: {error}");
                        None
                    },
                    Err(TryReceiveError::ReceiveError(_)) => return,
                }
            } else {
                match self.port.recv() {
                    Ok(message) => Some(message),
                    Err(ReceiveError::DeserializationFailed(error)) => {
                        warn!("A Web Locks message could not be read: {error}");
                        None
                    },
                    Err(_) => return,
                }
            };
            if !had_waiters {
                // The first waiter is probed one interval after it queued, not at once.
                last_probe = Instant::now();
            }

            if let Some(message) = message {
                self.handle_message(message);
            }

            if !self.has_waiters() {
                last_probe = Instant::now();
            } else if last_probe.elapsed() >= self.probe_interval {
                self.probe_clients();
                last_probe = Instant::now();
            }
            // Mercury: an origin with nothing held and nothing queued is forgotten.
            self.managers.retain(|_, manager| !manager.is_idle());
        }
    }

    fn handle_message(&mut self, message: WebLocksThreadMsg) {
        match message {
            WebLocksThreadMsg::Request(request, origin) => {
                self.obtain_lock_manager(origin)
                    .request_lock(NewLockRequest::from(request));
            },
            WebLocksThreadMsg::Query(sender, origin) => {
                let snapshot = self.obtain_lock_manager(origin).snapshot_lock_state();
                _ = sender.send(snapshot);
            },
            WebLocksThreadMsg::CollectMemoryReport(sender) => {
                let reports = self.collect_memory_reports();
                sender.send(ProcessReports::new(reports));
            },
            WebLocksThreadMsg::Release(lock_id, name, origin) => {
                self.release_lock(origin, name, lock_id);
            },
            WebLocksThreadMsg::Abort(request_id, origin, name) => {
                self.abort_request(origin, name, request_id);
            },
            WebLocksThreadMsg::ClientGone(client_id) => {
                // Mercury: a client has one id in every origin it ever asked in.
                for manager in self.managers.values_mut() {
                    manager.release_client(&client_id);
                }
            },
        }
    }

    fn has_waiters(&self) -> bool {
        self.managers.values().any(LockManager::has_waiters)
    }

    /// Mercury: ask every client that holds or waits for a contended lock whether it is alive.
    fn probe_clients(&mut self) {
        for manager in self.managers.values_mut() {
            manager.probe();
        }
    }

    /// <https://www.w3.org/TR/web-locks/#obtain-a-lock-manager>
    fn obtain_lock_manager(
        &mut self,
        origin: ImmutableOrigin,
    ) -> &mut LockManager<GenericCallback<LockEventMsg>> {
        // TODO: servo does not follow storage standard
        self.managers.entry(origin).or_default()
    }

    fn collect_memory_reports(&self) -> Vec<Report> {
        // TODO: actual report
        vec![]
    }

    /// <https://www.w3.org/TR/web-locks/#algorithm-release-lock>
    fn release_lock(&mut self, origin: ImmutableOrigin, name: String, lock_id: LockId) {
        // Step 1. skip assert
        // Step 2-5. Let
        let manager = self.obtain_lock_manager(origin);
        manager.release_lock(&name, lock_id);
    }

    /// <https://www.w3.org/TR/web-locks/#algorithm-abort-request>
    fn abort_request(&mut self, origin: ImmutableOrigin, name: String, request_id: LockRequestId) {
        // Step 1. skip assert
        // Step 2-5. Let
        let manager = self.obtain_lock_manager(origin);
        manager.abort_request(&name, request_id);
    }
}

/// Mercury: something the lock manager can tell about its requests: a window or a worker.
pub(crate) trait LockClient {
    /// Delivers `event`. Returns false when the client cannot be reached any more.
    fn notify(&self, event: LockEventMsg) -> bool;
}

impl LockClient for GenericCallback<LockEventMsg> {
    fn notify(&self, event: LockEventMsg) -> bool {
        // A failed send means the receiving end is gone. In multi-process mode that is how the
        // storage thread learns that a content process died: its callback's channel is closed.
        match self.send(event) {
            Ok(()) => true,
            Err(error) => {
                warn!("A Web Locks client cannot be reached ({error}); its locks are released");
                false
            },
        }
    }
}

/// A request for a lock, as the lock manager takes it: `LockRequest` with the callback made
/// generic.
pub(crate) struct NewLockRequest<C> {
    pub(crate) id: LockRequestId,
    pub(crate) client_id: String,
    pub(crate) name: String,
    pub(crate) mode: LockModeMsg,
    pub(crate) client: C,
    pub(crate) if_available: bool,
    pub(crate) steal: bool,
}

impl From<LockRequest> for NewLockRequest<GenericCallback<LockEventMsg>> {
    fn from(request: LockRequest) -> Self {
        Self {
            id: request.id,
            client_id: request.client_id,
            name: request.name,
            mode: request.mode,
            client: request.callback,
            if_available: request.if_available,
            steal: request.steal,
        }
    }
}

struct QueuedRequest<C> {
    id: LockRequestId,
    client_id: String,
    name: String,
    mode: LockModeMsg,
    client: C,
}

struct HeldLock<C> {
    name: String,
    mode: LockModeMsg,
    client_id: String,
    client: C,
}

/// Pages and workers sharing a storage bucket opened in the same user agent
/// share a lock manager.
/// <https://www.w3.org/TR/web-locks/#lock-manager>
pub(crate) struct LockManager<C> {
    queue_map: FxHashMap<String, VecDeque<QueuedRequest<C>>>,
    held: FxHashMap<LockId, HeldLock<C>>,
    /// Mercury: clients found unreachable while an operation ran. Their locks and requests are
    /// dropped before the operation returns.
    dead: Vec<String>,
}

impl<C> Default for LockManager<C> {
    fn default() -> Self {
        Self {
            queue_map: Default::default(),
            held: Default::default(),
            dead: Default::default(),
        }
    }
}

impl<C: LockClient> LockManager<C> {
    /// Mercury: nothing is held and nothing is queued.
    pub(crate) fn is_idle(&self) -> bool {
        self.held.is_empty() && !self.has_waiters()
    }

    /// Mercury: some request is waiting for a lock.
    pub(crate) fn has_waiters(&self) -> bool {
        self.queue_map.values().any(|queue| !queue.is_empty())
    }

    /// <https://www.w3.org/TR/web-locks/#get-the-lock-request-queue>
    fn lock_request_queue(&mut self, name: &str) -> &mut VecDeque<QueuedRequest<C>> {
        // Step 1.
        // Step 2.
        self.queue_map.entry(name.to_owned()).or_default()
    }

    /// Mercury: would a request for `name` in `mode` have to wait, if it were queued now?
    fn would_wait(&self, name: &str, mode: LockModeMsg) -> bool {
        let is_first = self.queue_map.get(name).is_none_or(VecDeque::is_empty);
        !self.is_grantable(name, mode, is_first)
    }

    /// <https://www.w3.org/TR/web-locks/#grantable>
    fn is_grantable(&self, name: &str, mode: LockModeMsg, is_first: bool) -> bool {
        // Step 1 to 6 skip
        // Step 7.
        if !is_first {
            return false;
        }
        match mode {
            // Step 8.
            LockModeMsg::Exclusive => !self.held.values().any(|lock| lock.name == name),
            // Step 9.
            LockModeMsg::Shared => !self
                .held
                .values()
                .any(|lock| lock.mode == LockModeMsg::Exclusive && lock.name == name),
        }
    }

    /// Mercury: tells a client something and notes that it is gone when it cannot be reached.
    fn tell(&mut self, client: &C, client_id: &str, event: LockEventMsg) -> bool {
        let reached = client.notify(event);
        if !reached {
            self.dead.push(client_id.to_owned());
        }
        reached
    }

    /// <https://www.w3.org/TR/web-locks/#algorithm-request-lock>
    pub(crate) fn request_lock(&mut self, request: NewLockRequest<C>) {
        // Step 3.1 to 3.3. skip to avoid &mut
        let NewLockRequest {
            id,
            client_id,
            name,
            mode,
            client,
            if_available,
            steal,
        } = request;
        let request = QueuedRequest {
            id,
            client_id,
            name: name.clone(),
            mode,
            client,
        };

        // Step 3.4. If steal is true,
        if steal {
            // Step 3.4.1. Every held lock with this name is released, and the request that
            // holds it is rejected with an AbortError (the client does that on `Stolen`).
            let stolen: Vec<LockId> = self
                .held
                .iter()
                .filter(|(_, lock)| lock.name == name)
                .map(|(lock_id, _)| *lock_id)
                .collect();
            for lock_id in stolen {
                if let Some(lock) = self.held.remove(&lock_id) {
                    self.tell(&lock.client, &lock.client_id, LockEventMsg::Stolen);
                }
            }
            // Step 3.4.2. Prepend request in queue.
            self.lock_request_queue(&name).push_front(request);
        }
        // Step 3.5. Otherwise
        else {
            // Mercury: a client that is gone and holds what this request needs must not block it.
            if self.would_wait(&name, mode) {
                self.probe_name(&name);
                self.purge_dead();
            }

            // Step 3.5.1. if ifAvailable and not grantable, the client runs its callback with
            // null, and the request is not queued.
            // Mercury: the pull request answered null when the request *was* grantable.
            if if_available && self.would_wait(&name, mode) {
                self.tell(
                    &request.client,
                    &request.client_id,
                    LockEventMsg::Granted(None),
                );
                self.purge_dead();
                return;
            }
            // Step 3.5.2. Enqueue request in queue.
            self.lock_request_queue(&name).push_back(request);
        }

        // Step 3.6. Process the lock request queue queue.
        self.process_lock_request_queue(&name);
        self.purge_dead();
    }

    /// <https://www.w3.org/TR/web-locks/#process-the-lock-request-queue>
    fn process_lock_request_queue(&mut self, name: &str) {
        // Step 1. skip assert

        // Step 2. For each request of queue:
        while let Some(head) = self.queue_map.get(name).and_then(|queue| queue.front()) {
            // Step 2.1. If request is not grantable, return.
            if !self.is_grantable(name, head.mode, true) {
                break;
            }
            // Step 2.2. Remove request from queue.
            let Some(request) = self.queue_map.get_mut(name).and_then(VecDeque::pop_front) else {
                break;
            };

            // Step 2.3 to 2.8. skip
            // Step 12. Let lock be a new lock with ...
            let lock_id = LockId::next();
            let msg = LockMsg {
                id: lock_id,
                name: request.name.clone(),
                mode: request.mode,
            };

            // Step 14. Enqueue the following steps on callback’s relevant settings
            // object’s responsible event loop.
            // The inner steps continue on that thread.
            // Mercury: a client that cannot be reached does not get the lock (Step 13 below is
            // not run), and the next request is looked at.
            if self.tell(
                &request.client,
                &request.client_id,
                LockEventMsg::Granted(Some(msg)),
            ) {
                // Step 13. Append lock to manager’s held lock set.
                self.held.insert(
                    lock_id,
                    HeldLock {
                        name: request.name,
                        mode: request.mode,
                        client_id: request.client_id,
                        client: request.client,
                    },
                );
            }
        }

        // Mercury: an empty queue is not kept.
        if self.queue_map.get(name).is_some_and(VecDeque::is_empty) {
            self.queue_map.remove(name);
        }
    }

    /// <https://www.w3.org/TR/web-locks/#algorithm-release-lock>
    pub(crate) fn release_lock(&mut self, name: &str, lock_id: LockId) {
        // Step 6. Remove lock from the manager’s held lock set.
        // Mercury: a lock that is not held any more (it was stolen, or its client was dropped)
        // changes nothing.
        let Some(lock) = self.held.remove(&lock_id) else {
            return;
        };
        if lock.name != name {
            warn!(
                "A Web Locks release named {name:?} for a lock named {:?}",
                lock.name
            );
        }

        // Step 7. Process the lock request queue queue.
        self.process_lock_request_queue(&lock.name);
        self.purge_dead();
    }

    /// <https://www.w3.org/TR/web-locks/#algorithm-abort-request>
    pub(crate) fn abort_request(&mut self, name: &str, request_id: LockRequestId) {
        // Step 6. Remove request from queue.
        let queue = self.lock_request_queue(name);
        if let Some(pos) = queue.iter().position(|request| request.id == request_id) {
            queue.remove(pos);
        }

        // Step 7. Process the lock request queue queue.
        self.process_lock_request_queue(name);
        self.purge_dead();
    }

    /// Mercury: the client with this id is gone. Whatever it holds is released and whatever it
    /// queued is dropped, then the requests that were waiting behind it are looked at.
    pub(crate) fn release_client(&mut self, client_id: &str) {
        self.forget_client(client_id);
        self.purge_dead();
    }

    fn forget_client(&mut self, client_id: &str) {
        let mut names: Vec<String> = Vec::new();
        self.held.retain(|_, lock| {
            let keep = lock.client_id != client_id;
            if !keep {
                names.push(lock.name.clone());
            }
            keep
        });
        for (name, queue) in self.queue_map.iter_mut() {
            let before = queue.len();
            queue.retain(|request| request.client_id != client_id);
            if queue.len() != before {
                names.push(name.clone());
            }
        }
        names.sort();
        names.dedup();
        for name in names {
            self.process_lock_request_queue(&name);
        }
    }

    /// Mercury: drops everything that belongs to a client found unreachable. Dropping can grant a
    /// lock to another client that is unreachable too, so this goes on until none is left.
    fn purge_dead(&mut self) {
        while let Some(client_id) = self.dead.pop() {
            self.forget_client(&client_id);
        }
    }

    /// Mercury: probes every client that holds or waits for the lock `name`.
    fn probe_name(&mut self, name: &str) {
        for lock in self.held.values().filter(|lock| lock.name == name) {
            if !lock.client.notify(LockEventMsg::Probe) {
                self.dead.push(lock.client_id.clone());
            }
        }
        if let Some(queue) = self.queue_map.get(name) {
            for request in queue {
                if !request.client.notify(LockEventMsg::Probe) {
                    self.dead.push(request.client_id.clone());
                }
            }
        }
    }

    /// Mercury: probes the clients of every lock that has requests waiting for it.
    pub(crate) fn probe(&mut self) {
        let contended: Vec<String> = self
            .queue_map
            .iter()
            .filter(|(_, queue)| !queue.is_empty())
            .map(|(name, _)| name.clone())
            .collect();
        for name in contended {
            self.probe_name(&name);
        }
        self.purge_dead();
    }

    /// Mercury: probes every client that holds a lock or has a request queued, whatever the lock, and drops the
    /// ones that cannot be reached (net-1 review, finding 4). `query()` calls it, so a lock whose holder died is
    /// not listed: the periodic probe runs only while somebody waits, which a `query()` alone does not.
    pub(crate) fn probe_all(&mut self) {
        for lock in self.held.values() {
            if !lock.client.notify(LockEventMsg::Probe) {
                self.dead.push(lock.client_id.clone());
            }
        }
        for queue in self.queue_map.values() {
            for request in queue {
                if !request.client.notify(LockEventMsg::Probe) {
                    self.dead.push(request.client_id.clone());
                }
            }
        }
        self.purge_dead();
    }

    /// <https://www.w3.org/TR/web-locks/#snapshot-the-lock-state>
    pub(crate) fn snapshot_lock_state(&mut self) -> LockManagerSnapshotMsg {
        // Mercury: a holder that died is not listed (see `probe_all`).
        self.probe_all();

        // Step 1. skip assert
        // Step 2. Let pending be a new list.
        let mut pending = vec![];
        // Step 3. For each queue of lock request queue map:
        for queue in self.queue_map.values() {
            // Step 3.1. For each request of queue:
            for request in queue.iter() {
                // Step 3.1.1. Append ...
                pending.push(LockInfoMsg {
                    name: request.name.clone(),
                    mode: request.mode,
                    client_id: request.client_id.clone(),
                });
            }
        }

        // Step 4. Let held be a new list.
        let mut held = vec![];
        // Step 5. For each lock of manager’s held lock set:
        for lock in self.held.values() {
            // Step 5.1. Append ...
            held.push(LockInfoMsg {
                name: lock.name.clone(),
                mode: lock.mode,
                client_id: lock.client_id.clone(),
            });
        }

        // Step 6. Resolve promise with held and pending.
        LockManagerSnapshotMsg { held, pending }
    }
}

#[cfg(test)]
mod tests {
    //! The rules of the lock manager, with no thread and no channel. The scenarios of
    //! exclusion, sharing, `ifAvailable` and `steal` follow the registry tests of LunarWerxs
    //! `pr/web-locks` (commit 23583d753, `components/script/dom/weblocks/registry.rs`,
    //! L0garithmic, MPL-2.0), written again for this lock manager.

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// A client the test can look at and make unreachable.
    #[derive(Clone)]
    struct Fake {
        events: Rc<RefCell<Vec<LockEventMsg>>>,
        reachable: Rc<Cell<bool>>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake {
                events: Default::default(),
                reachable: Rc::new(Cell::new(true)),
            }
        }

        /// The locks this client was granted, in order.
        fn granted(&self) -> Vec<LockMsg> {
            self.events
                .borrow()
                .iter()
                .filter_map(|event| match event {
                    LockEventMsg::Granted(Some(lock)) => Some(lock.clone()),
                    _ => None,
                })
                .collect()
        }

        fn was_told_unavailable(&self) -> bool {
            self.events
                .borrow()
                .iter()
                .any(|event| matches!(event, LockEventMsg::Granted(None)))
        }

        fn was_stolen_from(&self) -> bool {
            self.events
                .borrow()
                .iter()
                .any(|event| matches!(event, LockEventMsg::Stolen))
        }

        fn probes(&self) -> usize {
            self.events
                .borrow()
                .iter()
                .filter(|event| matches!(event, LockEventMsg::Probe))
                .count()
        }

        fn disconnect(&self) {
            self.reachable.set(false);
        }
    }

    impl LockClient for Fake {
        fn notify(&self, event: LockEventMsg) -> bool {
            if !self.reachable.get() {
                return false;
            }
            self.events.borrow_mut().push(event);
            true
        }
    }

    fn request(
        client: &Fake,
        client_id: &str,
        name: &str,
        mode: LockModeMsg,
    ) -> (NewLockRequest<Fake>, LockRequestId) {
        let id = LockRequestId::next();
        let request = NewLockRequest {
            id,
            client_id: client_id.to_owned(),
            name: name.to_owned(),
            mode,
            client: client.clone(),
            if_available: false,
            steal: false,
        };
        (request, id)
    }

    fn exclusive(client: &Fake, client_id: &str, name: &str) -> NewLockRequest<Fake> {
        request(client, client_id, name, LockModeMsg::Exclusive).0
    }

    fn shared(client: &Fake, client_id: &str, name: &str) -> NewLockRequest<Fake> {
        request(client, client_id, name, LockModeMsg::Shared).0
    }

    fn held_names(manager: &mut LockManager<Fake>) -> Vec<(String, String)> {
        let mut held: Vec<_> = manager
            .snapshot_lock_state()
            .held
            .into_iter()
            .map(|info| (info.name, info.client_id))
            .collect();
        held.sort();
        held
    }

    #[test]
    fn exclusive_excludes_then_hands_over() {
        let mut manager = LockManager::default();
        let (a, b) = (Fake::new(), Fake::new());

        manager.request_lock(exclusive(&a, "a", "x"));
        assert_eq!(a.granted().len(), 1);

        manager.request_lock(exclusive(&b, "b", "x"));
        assert!(b.granted().is_empty(), "b waits while a holds the lock");
        assert_eq!(manager.snapshot_lock_state().pending.len(), 1);

        manager.release_lock("x", a.granted()[0].id);
        assert_eq!(b.granted().len(), 1, "b is granted when a releases");
        assert_eq!(held_names(&mut manager), vec![("x".into(), "b".into())]);

        manager.release_lock("x", b.granted()[0].id);
        assert!(manager.is_idle());
    }

    #[test]
    fn locks_with_other_names_do_not_interfere() {
        let mut manager = LockManager::default();
        let (a, b) = (Fake::new(), Fake::new());
        manager.request_lock(exclusive(&a, "a", "x"));
        manager.request_lock(exclusive(&b, "b", "y"));
        assert_eq!(a.granted().len(), 1);
        assert_eq!(b.granted().len(), 1);
    }

    #[test]
    fn shared_locks_share_and_exclude_an_exclusive_one() {
        let mut manager = LockManager::default();
        let (a, b, c) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(shared(&a, "a", "x"));
        manager.request_lock(shared(&b, "b", "x"));
        assert_eq!(a.granted().len(), 1);
        assert_eq!(b.granted().len(), 1, "two shared locks are held together");

        manager.request_lock(exclusive(&c, "c", "x"));
        assert!(c.granted().is_empty());

        manager.release_lock("x", a.granted()[0].id);
        assert!(c.granted().is_empty(), "b still holds it");
        manager.release_lock("x", b.granted()[0].id);
        assert_eq!(c.granted().len(), 1);
    }

    #[test]
    fn a_shared_request_does_not_pass_a_waiting_exclusive_one() {
        let mut manager = LockManager::default();
        let (a, b, c) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(shared(&a, "a", "x"));
        manager.request_lock(exclusive(&b, "b", "x"));
        manager.request_lock(shared(&c, "c", "x"));
        assert!(
            c.granted().is_empty(),
            "the queue is first come, first served"
        );

        manager.release_lock("x", a.granted()[0].id);
        assert_eq!(b.granted().len(), 1);
        assert!(c.granted().is_empty());
        manager.release_lock("x", b.granted()[0].id);
        assert_eq!(c.granted().len(), 1);
    }

    #[test]
    fn shared_requests_at_the_head_are_granted_together() {
        let mut manager = LockManager::default();
        let (a, b, c) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(exclusive(&a, "a", "x"));
        manager.request_lock(shared(&b, "b", "x"));
        manager.request_lock(shared(&c, "c", "x"));
        manager.release_lock("x", a.granted()[0].id);
        assert_eq!(b.granted().len(), 1);
        assert_eq!(c.granted().len(), 1);
    }

    #[test]
    fn if_available_runs_with_null_when_the_lock_is_taken_and_is_not_queued() {
        let mut manager = LockManager::default();
        let (a, b) = (Fake::new(), Fake::new());
        manager.request_lock(exclusive(&a, "a", "x"));

        let mut ask = exclusive(&b, "b", "x");
        ask.if_available = true;
        manager.request_lock(ask);
        assert!(b.was_told_unavailable());
        assert!(b.granted().is_empty());
        assert!(
            manager.snapshot_lock_state().pending.is_empty(),
            "not queued"
        );

        manager.release_lock("x", a.granted()[0].id);
        assert!(b.granted().is_empty(), "nothing is handed to b later");
    }

    #[test]
    fn if_available_gets_the_lock_when_it_is_free() {
        // The pull request answered null exactly here.
        let mut manager = LockManager::default();
        let a = Fake::new();
        let mut ask = exclusive(&a, "a", "x");
        ask.if_available = true;
        manager.request_lock(ask);
        assert_eq!(a.granted().len(), 1);
        assert!(!a.was_told_unavailable());
    }

    #[test]
    fn if_available_does_not_pass_a_queued_request() {
        let mut manager = LockManager::default();
        let (a, b, c) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(shared(&a, "a", "x"));
        manager.request_lock(exclusive(&b, "b", "x"));
        let mut ask = shared(&c, "c", "x");
        ask.if_available = true;
        manager.request_lock(ask);
        assert!(c.was_told_unavailable(), "b is queued ahead of c");
    }

    #[test]
    fn steal_takes_the_lock_from_the_holder_and_goes_first() {
        let mut manager = LockManager::default();
        let (victim, waiter, thief) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(exclusive(&victim, "victim", "x"));
        manager.request_lock(exclusive(&waiter, "waiter", "x"));

        let mut steal = exclusive(&thief, "thief", "x");
        steal.steal = true;
        manager.request_lock(steal);

        assert!(victim.was_stolen_from());
        assert_eq!(thief.granted().len(), 1, "the thief holds the lock at once");
        assert!(waiter.granted().is_empty(), "and the waiter is behind it");
        assert_eq!(held_names(&mut manager), vec![("x".into(), "thief".into())]);

        // The victim's callback finishes later and releases a lock it no longer has.
        manager.release_lock("x", victim.granted()[0].id);
        assert_eq!(held_names(&mut manager), vec![("x".into(), "thief".into())]);
        assert!(
            waiter.granted().is_empty(),
            "the late release changed nothing"
        );

        manager.release_lock("x", thief.granted()[0].id);
        assert_eq!(waiter.granted().len(), 1);
    }

    #[test]
    fn steal_takes_every_shared_holder() {
        let mut manager = LockManager::default();
        let (a, b, thief) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(shared(&a, "a", "x"));
        manager.request_lock(shared(&b, "b", "x"));
        let mut steal = exclusive(&thief, "thief", "x");
        steal.steal = true;
        manager.request_lock(steal);
        assert!(a.was_stolen_from());
        assert!(b.was_stolen_from());
        assert_eq!(held_names(&mut manager), vec![("x".into(), "thief".into())]);
    }

    #[test]
    fn steal_on_a_free_lock_just_takes_it() {
        let mut manager = LockManager::default();
        let thief = Fake::new();
        let mut steal = exclusive(&thief, "thief", "x");
        steal.steal = true;
        manager.request_lock(steal);
        assert_eq!(thief.granted().len(), 1);
    }

    #[test]
    fn abort_removes_a_queued_request_and_unblocks_the_next() {
        let mut manager = LockManager::default();
        let (a, b, c) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(exclusive(&a, "a", "x"));
        let (ask_b, b_id) = request(&b, "b", "x", LockModeMsg::Exclusive);
        manager.request_lock(ask_b);
        manager.request_lock(exclusive(&c, "c", "x"));

        manager.abort_request("x", b_id);
        assert_eq!(manager.snapshot_lock_state().pending.len(), 1);

        manager.release_lock("x", a.granted()[0].id);
        assert!(
            b.granted().is_empty(),
            "an aborted request is never granted"
        );
        assert_eq!(c.granted().len(), 1);
    }

    #[test]
    fn aborting_the_head_of_a_shared_queue_lets_the_shared_requests_behind_it_in() {
        let mut manager = LockManager::default();
        let (a, b, c) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(shared(&a, "a", "x"));
        let (ask_b, b_id) = request(&b, "b", "x", LockModeMsg::Exclusive);
        manager.request_lock(ask_b);
        manager.request_lock(shared(&c, "c", "x"));
        manager.abort_request("x", b_id);
        assert_eq!(c.granted().len(), 1, "c only waited for b");
    }

    #[test]
    fn aborting_a_request_that_is_not_queued_changes_nothing() {
        let mut manager = LockManager::default();
        let a = Fake::new();
        let (ask, id) = request(&a, "a", "x", LockModeMsg::Exclusive);
        manager.request_lock(ask);
        manager.abort_request("x", id);
        assert_eq!(held_names(&mut manager), vec![("x".into(), "a".into())]);
    }

    #[test]
    fn releasing_a_lock_that_is_not_held_changes_nothing() {
        let mut manager = LockManager::default();
        let (a, b) = (Fake::new(), Fake::new());
        manager.request_lock(exclusive(&a, "a", "x"));
        manager.request_lock(exclusive(&b, "b", "x"));
        manager.release_lock("x", LockId::next());
        assert_eq!(held_names(&mut manager), vec![("x".into(), "a".into())]);
        assert!(b.granted().is_empty());
    }

    #[test]
    fn a_client_that_goes_away_loses_what_it_holds_and_what_it_queued() {
        // The pipeline or the worker that asked is gone: its tab was closed.
        let mut manager = LockManager::default();
        let (gone, waiter) = (Fake::new(), Fake::new());
        manager.request_lock(exclusive(&gone, "gone", "x"));
        manager.request_lock(exclusive(&gone, "gone", "y"));
        manager.request_lock(exclusive(&waiter, "waiter", "x"));
        manager.request_lock(exclusive(&gone, "gone", "z"));
        manager.request_lock(exclusive(&waiter, "waiter", "z"));
        // gone also queues behind waiter on x.
        manager.request_lock(exclusive(&gone, "gone", "x"));

        manager.release_client("gone");

        assert_eq!(
            held_names(&mut manager),
            vec![("x".into(), "waiter".into()), ("z".into(), "waiter".into())],
            "the waiter got both locks the other client held"
        );
        assert!(manager.snapshot_lock_state().pending.is_empty());
    }

    #[test]
    fn a_client_that_is_gone_does_not_disturb_other_clients() {
        let mut manager = LockManager::default();
        let (a, b) = (Fake::new(), Fake::new());
        manager.request_lock(exclusive(&a, "a", "x"));
        manager.release_client("nobody");
        manager.release_client("b");
        assert_eq!(held_names(&mut manager), vec![("x".into(), "a".into())]);
        assert!(b.events.borrow().is_empty());
    }

    #[test]
    fn a_lock_is_not_given_to_a_client_that_cannot_be_reached() {
        // A queued request of a content process that died: the storage thread finds out when it
        // sends the grant, and the next request gets the lock.
        let mut manager = LockManager::default();
        let (holder, dead, next) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(exclusive(&holder, "holder", "x"));
        manager.request_lock(exclusive(&dead, "dead", "x"));
        manager.request_lock(exclusive(&next, "next", "x"));

        dead.disconnect();
        manager.release_lock("x", holder.granted()[0].id);

        assert!(dead.granted().is_empty());
        assert_eq!(held_names(&mut manager), vec![("x".into(), "next".into())]);
    }

    #[test]
    fn everything_a_client_that_cannot_be_reached_has_is_dropped_with_it() {
        let mut manager = LockManager::default();
        let (dead, waiter) = (Fake::new(), Fake::new());
        manager.request_lock(exclusive(&dead, "dead", "x"));
        manager.request_lock(exclusive(&dead, "dead", "y"));
        manager.request_lock(exclusive(&waiter, "waiter", "y"));
        manager.request_lock(exclusive(&waiter, "waiter", "x"));

        dead.disconnect();
        // The next probe round finds it: the waiters of x and y are behind its locks.
        manager.probe();

        assert_eq!(
            held_names(&mut manager),
            vec![("x".into(), "waiter".into()), ("y".into(), "waiter".into())]
        );
    }

    #[test]
    fn a_probe_reaches_the_holders_and_the_waiters_of_a_contended_lock_only() {
        let mut manager = LockManager::default();
        let (holder, waiter, bystander) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(exclusive(&holder, "holder", "x"));
        manager.request_lock(exclusive(&waiter, "waiter", "x"));
        manager.request_lock(exclusive(&bystander, "bystander", "y"));

        // The waiter's own request probed the holder once, when it had to wait.
        let before = holder.probes();
        manager.probe();

        assert_eq!(holder.probes(), before + 1);
        assert_eq!(waiter.probes(), 1);
        assert_eq!(bystander.probes(), 0, "nobody waits for y");
    }

    #[test]
    fn a_holder_that_cannot_be_reached_does_not_block_a_new_request() {
        // No timer needed: the request itself probes the holder it has to wait for.
        let mut manager = LockManager::default();
        let (dead, next) = (Fake::new(), Fake::new());
        manager.request_lock(exclusive(&dead, "dead", "x"));
        dead.disconnect();

        manager.request_lock(exclusive(&next, "next", "x"));

        assert_eq!(next.granted().len(), 1);
        assert_eq!(held_names(&mut manager), vec![("x".into(), "next".into())]);
    }

    #[test]
    fn an_uncontended_request_does_not_probe_anyone() {
        let mut manager = LockManager::default();
        let (a, b) = (Fake::new(), Fake::new());
        manager.request_lock(shared(&a, "a", "x"));
        manager.request_lock(shared(&b, "b", "x"));
        assert_eq!(a.probes(), 0);
    }

    #[test]
    fn the_snapshot_lists_what_is_held_and_what_is_pending() {
        let mut manager = LockManager::default();
        let (a, b) = (Fake::new(), Fake::new());
        manager.request_lock(exclusive(&a, "a", "x"));
        manager.request_lock(shared(&b, "b", "x"));
        let snapshot = manager.snapshot_lock_state();
        assert_eq!(snapshot.held.len(), 1);
        assert_eq!(snapshot.held[0].name, "x");
        assert_eq!(snapshot.held[0].mode, LockModeMsg::Exclusive);
        assert_eq!(snapshot.held[0].client_id, "a");
        assert_eq!(snapshot.pending.len(), 1);
        assert_eq!(snapshot.pending[0].mode, LockModeMsg::Shared);
        assert_eq!(snapshot.pending[0].client_id, "b");
    }

    #[test]
    fn the_snapshot_does_not_list_the_locks_of_a_client_that_cannot_be_reached() {
        let mut manager = LockManager::default();
        let (dead, alive, waiter) = (Fake::new(), Fake::new(), Fake::new());
        manager.request_lock(exclusive(&dead, "dead", "x"));
        manager.request_lock(exclusive(&alive, "alive", "y"));
        // `waiter` queues for `x`, behind the holder that is about to die.
        manager.request_lock(exclusive(&waiter, "waiter", "x"));
        dead.disconnect();

        // No periodic probe has run (nobody waited for a second): the query itself finds out.
        let snapshot = manager.snapshot_lock_state();
        let held: Vec<(String, String)> = snapshot
            .held
            .iter()
            .map(|lock| (lock.name.clone(), lock.client_id.clone()))
            .collect();
        // `dead` lost `x`, which went to the waiter; `alive` still has `y`.
        assert!(held.contains(&("x".to_owned(), "waiter".to_owned())), "{held:?}");
        assert!(held.contains(&("y".to_owned(), "alive".to_owned())), "{held:?}");
        assert!(!held.iter().any(|(_, client)| client == "dead"), "{held:?}");
        assert!(snapshot.pending.is_empty());
    }

    #[test]
    fn a_manager_with_nothing_in_it_is_idle() {
        let mut manager = LockManager::default();
        assert!(manager.is_idle());
        let a = Fake::new();
        manager.request_lock(exclusive(&a, "a", "x"));
        assert!(!manager.is_idle());
        assert!(!manager.has_waiters(), "a held lock is not a waiter");
        manager.release_client("a");
        assert!(manager.is_idle());
    }
}
