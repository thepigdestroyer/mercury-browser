// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Operation serials of lane med-1b (decision D3), against a real `MseBackend` (real worker thread,
//! real parser): the completion events carry the `OpId` that `append` and `remove` returned, a stale
//! event (one already queued when `abort()` ran) is told apart from the event of the next append, and
//! `SourceBufferOps` (the operation state machine the DOM's `SourceBuffer` uses), driven by random
//! operations against the real backend, never lets a stale event through, loses an accepted operation or
//! lets `abort()` through during a removal, whichever of the worker and the page's task is first.
//!
//! Replay one random sequence with `MSE_SERIAL_SEED=n`.

mod model_support;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use mercury_mse::api::{
    AllDecoders, EventSink, Limits, MediaSourceBackend, MediaSourceReadyState, MseEvent, OpId,
    SourceBufferId,
};
use mercury_mse::source_buffer_ops::{SbEvent, SourceBufferOps};
use mercury_mse::{MseBackend, OpTracker};
use model_support::{Rng, WAIT, load};

fn completion(event: &MseEvent) -> Option<(SourceBufferId, OpId)> {
    match event {
        MseEvent::AppendDone { id, op }
        | MseEvent::AppendError { id, op, .. }
        | MseEvent::RemoveDone { id, op } => Some((*id, *op)),
        _ => None,
    }
}

/// The script task queue: events wait here until the "DOM" (the test thread) processes them.
#[derive(Default)]
struct TaskQueue {
    events: Mutex<VecDeque<MseEvent>>,
    cv: Condvar,
    /// How many completion events the backend emitted per operation, counted when they are emitted.
    emitted: Mutex<HashMap<OpId, usize>>,
}

impl TaskQueue {
    fn sink(self: &Arc<Self>) -> EventSink {
        let queue = Arc::clone(self);
        Arc::new(move |event| {
            if let Some((_, op)) = completion(&event) {
                *queue.emitted.lock().unwrap().entry(op).or_default() += 1;
            }
            queue.events.lock().unwrap().push_back(event);
            queue.cv.notify_all();
        })
    }

    fn pop(&self) -> Option<MseEvent> {
        self.events.lock().unwrap().pop_front()
    }

    /// Waits until a completion event of `op` is queued (not processed), or the timeout.
    fn wait_for_completion_of(&self, op: OpId, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut events = self.events.lock().unwrap();
        loop {
            if events
                .iter()
                .any(|e| completion(e).is_some_and(|(_, o)| o == op))
            {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            events = self.cv.wait_timeout(events, deadline - now).unwrap().0;
        }
    }

    fn wait_for_any(&self, timeout: Duration) -> bool {
        let events = self.events.lock().unwrap();
        if !events.is_empty() {
            return true;
        }
        !self.cv.wait_timeout(events, timeout).unwrap().1.timed_out()
    }
}

fn backend(queue: &Arc<TaskQueue>, limits: Limits) -> Arc<MseBackend> {
    let backend = Arc::new(MseBackend::new(queue.sink(), Arc::new(AllDecoders), limits));
    backend.open();
    backend
}

// ----------------------------------------------------------------------------------------------
// The stale event, deterministically

#[test]
fn serial_abort_after_the_event_was_posted_and_a_new_append_in_the_same_task() {
    let s = load("aac");
    let queue = Arc::new(TaskQueue::default());
    let backend = backend(&queue, Limits::default());
    let id = backend.add_source_buffer(&s.mime).unwrap();
    let mut tracker = OpTracker::new();

    // The init segment, processed normally.
    let init = backend.append(id, s.init.bytes.clone()).unwrap();
    tracker.begin(init);
    assert!(queue.wait_for_completion_of(init, WAIT));
    process_until(&queue, &mut tracker, init);
    assert_eq!(tracker.pending(), None);

    // The first media segment: its AppendDone is posted into the queue, and the page has not run it.
    let first = backend.append(id, s.media[0].bytes.clone()).unwrap();
    tracker.begin(first);
    assert!(
        queue.wait_for_completion_of(first, WAIT),
        "the AppendDone was not posted"
    );

    // The page's task: abort() and, in the same task, a new appendBuffer().
    backend.abort(id).unwrap();
    tracker.retire();
    let second = backend.append(id, s.media[1].bytes.clone()).unwrap();
    tracker.begin(second);
    assert!(second > first, "an OpId is never reused or reordered");

    // Now the queued tasks run. The stale event is for the retired operation.
    let stale = queue
        .pop()
        .expect("the stale AppendDone is first in the queue");
    assert_eq!(completion(&stale), Some((id, first)), "{stale:?}");
    assert!(
        !tracker.finish(first),
        "the stale AppendDone must not end the new append"
    );
    assert_eq!(
        tracker.pending(),
        Some(second),
        "the new append is still pending"
    );
    // The new append's own event is accepted, exactly once.
    assert!(queue.wait_for_completion_of(second, WAIT));
    let mut accepted = 0;
    while let Some(event) = queue.pop() {
        if let Some((_, op)) = completion(&event) {
            assert_eq!(op, second, "{event:?}");
            if tracker.finish(op) {
                accepted += 1;
            }
        }
    }
    assert_eq!(accepted, 1);
    assert!(!backend.updating(id));
    assert!(
        !backend.buffered(id).is_empty(),
        "the second segment's data is buffered"
    );
}

/// Processes queued events through `tracker` until one for `op` has been seen.
fn process_until(queue: &Arc<TaskQueue>, tracker: &mut OpTracker, op: OpId) {
    while let Some(event) = queue.pop() {
        if let Some((_, o)) = completion(&event) {
            let accepted = tracker.finish(o);
            if o == op {
                assert!(accepted, "{event:?}");
                return;
            }
        }
    }
    panic!("no completion event for {op}");
}

#[test]
fn serial_operation_ids_increase_across_appends_removes_and_buffers() {
    let s = load("aac");
    let queue = Arc::new(TaskQueue::default());
    let backend = backend(&queue, Limits::default());
    let a = backend.add_source_buffer(&s.mime).unwrap();
    let b = backend.add_source_buffer(&s.mime).unwrap();
    let mut last: Option<OpId> = None;
    let mut note = |op: OpId| {
        if let Some(prev) = last {
            assert!(op > prev, "{op} after {prev}");
        }
        last = Some(op);
    };
    for id in [a, b] {
        let op = backend.append(id, s.init.bytes.clone()).unwrap();
        note(op);
        assert!(queue.wait_for_completion_of(op, WAIT));
        let op = backend.append(id, s.media[0].bytes.clone()).unwrap();
        note(op);
        assert!(queue.wait_for_completion_of(op, WAIT));
        let op = backend.remove(id, 0.0, 0.5).unwrap();
        note(op);
        assert!(queue.wait_for_completion_of(op, WAIT));
    }
    // A refused call does not use an id: the next accepted operation is still larger than the last.
    assert!(backend.remove(a, 5.0, 1.0).is_err());
    let op = backend.append(a, Vec::new()).unwrap();
    note(op);
}

#[test]
fn serial_every_completion_event_carries_the_id_of_its_own_operation() {
    let s = load("aac");
    let queue = Arc::new(TaskQueue::default());
    let backend = backend(&queue, Limits::default());
    let id = backend.add_source_buffer(&s.mime).unwrap();
    let mut returned = Vec::new();
    returned.push(backend.append(id, s.init.bytes.clone()).unwrap());
    assert!(queue.wait_for_completion_of(returned[0], WAIT));
    for seg in s.media.iter().take(3) {
        let op = backend.append(id, seg.bytes.clone()).unwrap();
        returned.push(op);
        assert!(queue.wait_for_completion_of(op, WAIT));
    }
    let op = backend.remove(id, 0.0, 1.0).unwrap();
    returned.push(op);
    assert!(queue.wait_for_completion_of(op, WAIT));
    // A failing append: its AppendError carries its id as well.
    let bad = backend.append(id, vec![0xAB; 64]).unwrap();
    returned.push(bad);
    assert!(queue.wait_for_completion_of(bad, WAIT));

    let mut seen = Vec::new();
    while let Some(event) = queue.pop() {
        if let Some((event_id, op)) = completion(&event) {
            assert_eq!(event_id, id);
            seen.push(op);
        }
    }
    assert_eq!(seen, returned, "one completion per operation, in order");
}

#[test]
fn serial_an_aborted_append_that_never_finished_emits_nothing_for_its_id() {
    // The backend's own second line of defence (the generation) still holds: with the worker held up by
    // a big append, an abort() right after append() leaves no event for that operation.
    let s = load("h264-low");
    let big = mercury_mse_testkit::tile(&s, 30).unwrap();
    let queue = Arc::new(TaskQueue::default());
    let backend = backend(&queue, Limits::default());
    let id = backend.add_source_buffer(&s.mime).unwrap();
    let init = backend.append(id, big.init.bytes.clone()).unwrap();
    assert!(queue.wait_for_completion_of(init, WAIT));
    let mut aborted = Vec::new();
    for seg in big.media.iter().take(10) {
        let op = backend.append(id, seg.bytes.clone()).unwrap();
        backend.abort(id).unwrap();
        aborted.push(op);
    }
    // A probe operation behind them: when it completes, every earlier job has run.
    let probe = backend.append(id, Vec::new()).unwrap();
    assert!(queue.wait_for_completion_of(probe, WAIT));
    let mut seen_aborted = 0;
    while let Some(event) = queue.pop() {
        if let Some((_, op)) = completion(&event)
            && aborted.contains(&op)
        {
            seen_aborted += 1;
        }
    }
    // An event may have been posted before its abort() (that is the stale event the tracker exists
    // for), but never more than one per operation.
    let emitted = queue.emitted.lock().unwrap();
    for op in &aborted {
        assert!(emitted.get(op).copied().unwrap_or(0) <= 1, "{op}");
    }
    assert!(seen_aborted <= aborted.len());
}

// ----------------------------------------------------------------------------------------------
// The random walk: the DOM's SourceBuffer is `SourceBufferOps`, the backend is the real one
//
// The walk plays the DOM the way the shipped code does (`abort()`: `check_abort`, the backend's
// `abort`, then `abort`; `removeSourceBuffer()`: the backend, then `abort_pending` and `detach`) with
// every operation of the page, including `abort()` right after `remove()` (finding 1 of the review of
// round 2a: the old walk skipped it, and the DOM answered it from the worker's busy flag).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Append,
    Remove,
}

struct Buf {
    id: SourceBufferId,
    ops: SourceBufferOps,
    /// The operation the walk began last and that was not completed or retired since.
    latest: Option<(OpId, Kind)>,
    next_piece: usize,
}

#[derive(Default, Debug)]
struct Stats {
    accepted: usize,
    stale: usize,
    unknown_buffer: usize,
    appends: usize,
    removes: usize,
    /// `abort()` calls that retired a pending append.
    aborted_appends: usize,
    /// `abort()` calls refused because a removal was pending, and how many of those found the worker
    /// already finished (its `RemoveDone` queued, not yet handled).
    refused_aborts: usize,
    refused_worker_done: usize,
}

struct Dom {
    seed: u64,
    step: usize,
    backend: Arc<MseBackend>,
    queue: Arc<TaskQueue>,
    bufs: Vec<Buf>,
    pieces: Vec<Vec<u8>>,
    mime: String,
    issued: HashSet<OpId>,
    accepted: HashSet<OpId>,
    last_op: Option<OpId>,
    stats: Stats,
}

impl Dom {
    fn new(seed: u64, mime: &str, pieces: Vec<Vec<u8>>) -> Dom {
        let queue = Arc::new(TaskQueue::default());
        Dom {
            seed,
            step: 0,
            backend: backend(&queue, Limits::default()),
            queue,
            bufs: Vec::new(),
            pieces,
            mime: mime.to_string(),
            issued: HashSet::new(),
            accepted: HashSet::new(),
            last_op: None,
            stats: Stats::default(),
        }
    }

    fn ctx(&self) -> String {
        format!(
            "seed {} step {} (replay with MSE_SERIAL_SEED={})",
            self.seed, self.step, self.seed
        )
    }

    fn began(&mut self, buf: usize, op: OpId, kind: Kind) {
        if let Some(last) = self.last_op {
            assert!(op > last, "{}: {op} is not after {last}", self.ctx());
        }
        self.last_op = Some(op);
        self.issued.insert(op);
        let event = match kind {
            Kind::Append => self.bufs[buf].ops.begin_append(op),
            Kind::Remove => self.bufs[buf].ops.begin_remove(op),
        };
        assert_eq!(event, SbEvent::UpdateStart, "{}", self.ctx());
        self.bufs[buf].latest = Some((op, kind));
    }

    fn append(&mut self, rng: &mut Rng) {
        let Some(i) = self.pick_idle(rng) else { return };
        let data = match rng.below(100) {
            0..=4 => Vec::new(),
            5..=7 => vec![0xAB; 16],
            _ => {
                let n = self.bufs[i].next_piece;
                self.bufs[i].next_piece = (n + 1) % self.pieces.len();
                self.pieces[n].clone()
            }
        };
        let id = self.bufs[i].id;
        if let Ok(op) = self.backend.append(id, data) {
            self.stats.appends += 1;
            self.began(i, op, Kind::Append);
            if rng.chance(40) {
                // Let the worker finish and post the event before the page runs its next task.
                self.queue
                    .wait_for_completion_of(op, Duration::from_millis(5));
            }
        }
    }

    fn remove(&mut self, rng: &mut Rng) {
        let Some(i) = self.pick_idle(rng) else { return };
        let id = self.bufs[i].id;
        if let Ok(op) = self.backend.remove(id, 0.0, 1.0) {
            self.stats.removes += 1;
            self.began(i, op, Kind::Remove);
            if rng.chance(40) {
                self.queue
                    .wait_for_completion_of(op, Duration::from_millis(5));
            }
        }
    }

    /// The page's `abort()`, as the DOM runs it.
    fn abort(&mut self, rng: &mut Rng) {
        if self.bufs.is_empty() {
            return;
        }
        let i = rng.below(self.bufs.len());
        let id = self.bufs[i].id;
        let was = self.bufs[i].latest;
        let ctx = self.ctx();
        // Step 2: an append error ends the stream ("decode"), and abort() then throws before anything.
        if self.backend.ready_state() != MediaSourceReadyState::Open {
            return;
        }
        match self.bufs[i].ops.check_abort() {
            Err(refused) => {
                // The range removal algorithm is running: whether or not the worker finished it.
                assert!(
                    matches!(was, Some((_, Kind::Remove))),
                    "{ctx}: abort() refused ({refused}) with {was:?} pending"
                );
                assert!(self.bufs[i].ops.updating(), "{ctx}");
                assert_eq!(self.bufs[i].ops.abort(), Err(refused), "{ctx}");
                assert!(
                    self.bufs[i].ops.updating(),
                    "{ctx}: a refused abort() changed updating"
                );
                assert_eq!(self.bufs[i].ops.pending(), was.map(|w| w.0), "{ctx}");
                self.stats.refused_aborts += 1;
                if !self.backend.updating(id) {
                    self.stats.refused_worker_done += 1;
                }
            }
            Ok(()) => {
                assert!(
                    !matches!(was, Some((_, Kind::Remove))),
                    "{ctx}: abort() allowed during a removal"
                );
                if let Err(error) = self.backend.abort(id) {
                    // The worker's append error ended the stream between the check and the call: the
                    // page's abort() throws on step 2 and nothing else happens.
                    assert_ne!(
                        self.backend.ready_state(),
                        MediaSourceReadyState::Open,
                        "{ctx}: the backend refused an abort() the DOM allowed: {error}"
                    );
                    return;
                }
                let events = self.bufs[i].ops.abort().expect("abort() was checked");
                let want: &[SbEvent] = if was.is_some() {
                    self.stats.aborted_appends += 1;
                    &[SbEvent::Abort, SbEvent::UpdateEnd]
                } else {
                    &[]
                };
                assert_eq!(events, want, "{ctx}");
                self.bufs[i].latest = None;
            }
        }
    }

    fn remove_buffer(&mut self, rng: &mut Rng) {
        if self.bufs.is_empty() {
            return;
        }
        let i = rng.below(self.bufs.len());
        if self.backend.remove_source_buffer(self.bufs[i].id).is_ok() {
            let ctx = self.ctx();
            let was = self.bufs[i].latest;
            let events = self.bufs[i].ops.abort_pending();
            self.bufs[i].ops.detach();
            let want: &[SbEvent] = if was.is_some() {
                &[SbEvent::Abort, SbEvent::UpdateEnd]
            } else {
                &[]
            };
            assert_eq!(
                events, want,
                "{ctx}: removeSourceBuffer with {was:?} pending"
            );
            assert!(!self.bufs[i].ops.updating(), "{ctx}");
            self.bufs.remove(i);
        }
    }

    fn close_and_reopen(&mut self) {
        self.backend.close();
        for buf in &mut self.bufs {
            buf.ops.detach();
            assert!(!buf.ops.updating());
        }
        self.bufs.clear();
        self.backend.open();
    }

    fn add_buffer(&mut self) {
        if self.bufs.len() < 2
            && let Ok(id) = self.backend.add_source_buffer(&self.mime)
        {
            self.bufs.push(Buf {
                id,
                ops: SourceBufferOps::new(),
                latest: None,
                next_piece: 0,
            });
        }
    }

    fn pick_idle(&self, rng: &mut Rng) -> Option<usize> {
        let idle: Vec<usize> = (0..self.bufs.len())
            .filter(|i| !self.bufs[*i].ops.updating())
            .collect();
        (!idle.is_empty()).then(|| idle[rng.below(idle.len())])
    }

    /// `updating` is exactly "an operation is pending", and the walk's own record agrees.
    fn check_invariants(&self) {
        let ctx = self.ctx();
        for buf in &self.bufs {
            assert_eq!(buf.ops.updating(), buf.ops.pending().is_some(), "{ctx}");
            assert_eq!(buf.ops.pending(), buf.latest.map(|l| l.0), "{ctx}");
            assert_eq!(
                buf.ops.pending_is_remove(),
                matches!(buf.latest, Some((_, Kind::Remove))),
                "{ctx}"
            );
        }
    }

    /// The page's task for one queued event.
    fn handle(&mut self, event: MseEvent) {
        let Some((id, op)) = completion(&event) else {
            return;
        };
        let ctx = self.ctx();
        assert!(
            self.issued.contains(&op),
            "{ctx}: {event:?} for an operation never begun"
        );
        let Some(i) = self.bufs.iter().position(|b| b.id == id) else {
            self.stats.unknown_buffer += 1;
            return;
        };
        let latest = self.bufs[i].latest;
        let (events, kind, want) = match event {
            MseEvent::AppendError { .. } => (
                self.bufs[i].ops.failed(op),
                Kind::Append,
                [SbEvent::Error, SbEvent::UpdateEnd],
            ),
            MseEvent::RemoveDone { .. } => (
                self.bufs[i].ops.finished(op),
                Kind::Remove,
                [SbEvent::Update, SbEvent::UpdateEnd],
            ),
            _ => (
                self.bufs[i].ops.finished(op),
                Kind::Append,
                [SbEvent::Update, SbEvent::UpdateEnd],
            ),
        };
        if events.is_empty() {
            assert_ne!(
                latest.map(|l| l.0),
                Some(op),
                "{ctx}: the event for the pending operation was refused: {event:?}"
            );
            self.stats.stale += 1;
        } else {
            // An accepted completion is the latest accepted operation, once, of the right sort.
            assert_eq!(events, want, "{ctx}: {event:?}");
            assert_eq!(
                latest,
                Some((op, kind)),
                "{ctx}: accepted {event:?}, latest is {latest:?}"
            );
            assert!(self.accepted.insert(op), "{ctx}: {op} accepted twice");
            self.bufs[i].latest = None;
            assert!(
                !self.bufs[i].ops.updating(),
                "{ctx}: updating is still true after the accepted {event:?}"
            );
            assert!(
                !self.backend.updating(id),
                "{ctx}: the backend is still updating after the accepted {event:?}"
            );
            self.stats.accepted += 1;
        }
    }

    fn process(&mut self, count: usize) {
        for _ in 0..count {
            match self.queue.pop() {
                Some(event) => self.handle(event),
                None => return,
            }
        }
    }

    /// Runs the queue until no buffer is waiting for a completion: every accepted operation that was not
    /// retired must complete.
    fn settle(&mut self) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.process(usize::MAX);
            if self.bufs.iter().all(|b| !b.ops.updating()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{}: an operation never completed: {:?}",
                self.ctx(),
                self.bufs
                    .iter()
                    .map(|b| (b.id, b.latest))
                    .collect::<Vec<_>>()
            );
            self.queue.wait_for_any(Duration::from_millis(20));
        }
    }

    fn finish(mut self) -> Stats {
        self.settle();
        // After the dust: at most one completion event per operation, ever.
        let ctx = self.ctx();
        let emitted = self.queue.emitted.lock().unwrap();
        for (op, n) in emitted.iter() {
            assert!(*n <= 1, "{ctx}: {n} completion events for {op}");
            assert!(
                self.issued.contains(op),
                "{ctx}: an event for {op}, never begun"
            );
        }
        drop(emitted);
        self.stats
    }
}

fn seeds() -> Vec<u64> {
    match std::env::var("MSE_SERIAL_SEED") {
        Ok(v) => vec![v.parse().expect("MSE_SERIAL_SEED is a number")],
        Err(_) => (1..=1000).collect(),
    }
}

fn pieces(name: &str) -> (String, Vec<Vec<u8>>) {
    let s = load(name);
    let mut pieces = vec![s.init.bytes.clone()];
    pieces.extend(s.media.iter().map(|m| m.bytes.clone()));
    (s.mime.clone(), pieces)
}

#[test]
fn serial_random_operations_never_let_a_stale_event_through() {
    let (mime, pieces) = pieces("aac");
    let mut total = Stats::default();
    for seed in seeds() {
        let mut dom = Dom::new(seed, &mime, pieces.clone());
        let mut rng = Rng::new(seed);
        dom.add_buffer();
        for step in 0..100 {
            dom.step = step;
            match rng.below(100) {
                0..=34 => dom.append(&mut rng),
                35..=46 => dom.remove(&mut rng),
                47..=58 => dom.abort(&mut rng),
                59..=62 => dom.remove_buffer(&mut rng),
                63..=65 => dom.close_and_reopen(),
                66..=72 => dom.add_buffer(),
                73..=76 => {
                    let _ = dom.backend.set_duration(Some(rng.below(12) as f64));
                }
                _ => {
                    let n = 1 + rng.below(3);
                    dom.process(n);
                }
            }
            dom.check_invariants();
        }
        let stats = dom.finish();
        total.accepted += stats.accepted;
        total.stale += stats.stale;
        total.unknown_buffer += stats.unknown_buffer;
        total.appends += stats.appends;
        total.removes += stats.removes;
        total.aborted_appends += stats.aborted_appends;
        total.refused_aborts += stats.refused_aborts;
        total.refused_worker_done += stats.refused_worker_done;
    }
    eprintln!("serial random walk: {total:?}");
    if std::env::var("MSE_SERIAL_SEED").is_err() {
        // The walk must really exercise both sides: events that complete an operation, and events that
        // were already queued when the page retired it.
        assert!(total.accepted > 1_000, "{total:?}");
        assert!(
            total.stale > 100,
            "too few stale events were exercised: {total:?}"
        );
        assert!(total.removes > 1_000, "{total:?}");
        assert!(total.aborted_appends > 100, "{total:?}");
        // abort() right after remove(), with the worker done and with the RemoveDone still unhandled.
        assert!(
            total.refused_aborts > 100,
            "too few abort()s during a removal were exercised: {total:?}"
        );
        assert!(
            total.refused_worker_done > 20,
            "too few of them found the worker already finished: {total:?}"
        );
    }
}

// ----------------------------------------------------------------------------------------------
// T02 with the new return types

#[test]
fn serial_chained_appends_from_inside_the_closure_complete_in_order() {
    // The page starts the next append from the `updateend` of the last one: here, from inside the event
    // closure itself, on the worker thread. The ids it gets back are the ids the next events carry.
    let s = load("aac");
    let n = s.media.len().min(4);
    let log: Arc<Mutex<(Vec<OpId>, Vec<OpId>)>> = Arc::default(); // (returned, seen)
    let done = Arc::new(AtomicUsize::new(0));
    let backend_cell: Arc<Mutex<Option<Weak<MseBackend>>>> = Arc::default();
    let segments: Arc<Vec<Vec<u8>>> = Arc::new(s.media.iter().map(|m| m.bytes.clone()).collect());
    let sink: EventSink = {
        let log = Arc::clone(&log);
        let done = Arc::clone(&done);
        let cell = Arc::clone(&backend_cell);
        let segments = Arc::clone(&segments);
        Arc::new(move |event| {
            let Some((id, op)) = completion(&event) else {
                return;
            };
            log.lock().unwrap().1.push(op);
            let k = done.fetch_add(1, Ordering::SeqCst);
            if k < n
                && let Some(backend) = cell.lock().unwrap().as_ref().and_then(Weak::upgrade)
            {
                let next = backend
                    .append(id, segments[k].clone())
                    .expect("chained append");
                log.lock().unwrap().0.push(next);
            }
        })
    };
    let backend = Arc::new(MseBackend::new(
        sink,
        Arc::new(AllDecoders),
        Limits::default(),
    ));
    *backend_cell.lock().unwrap() = Some(Arc::downgrade(&backend));
    backend.open();
    let id = backend.add_source_buffer(&s.mime).unwrap();
    let first = backend.append(id, s.init.bytes.clone()).unwrap();
    log.lock().unwrap().0.push(first);
    let deadline = Instant::now() + WAIT;
    while done.load(Ordering::SeqCst) < n + 1 {
        assert!(
            Instant::now() < deadline,
            "the chain stopped at {}",
            done.load(Ordering::SeqCst)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(30));
    let (mut returned, seen) = log.lock().unwrap().clone();
    // The first id is logged by this thread after `append` returned, so it may land after a chained one.
    returned.sort();
    assert_eq!(
        returned.len(),
        n + 1,
        "returned {returned:?}, seen {seen:?}"
    );
    assert_eq!(
        seen, returned,
        "each event carries the id its append returned, in order"
    );
}

#[test]
fn serial_abort_and_append_from_inside_a_completion_closure() {
    // The page's `updateend` handler calls abort() and appendBuffer(): the old operation is retired by
    // the abort and never completes again; the new one completes once with its own id.
    let s = load("aac");
    let queue = Arc::new(TaskQueue::default());
    let calls = Arc::new(Mutex::new(Vec::<(OpId, OpId)>::new())); // (event's op, new op)
    let cell: Arc<Mutex<Option<Weak<MseBackend>>>> = Arc::default();
    let seg = s.media[0].bytes.clone();
    let sink: EventSink = {
        let inner = queue.sink();
        let calls = Arc::clone(&calls);
        let cell = Arc::clone(&cell);
        Arc::new(move |event| {
            let hook = match completion(&event) {
                Some((id, op))
                    if calls.lock().unwrap().is_empty()
                        && matches!(event, MseEvent::AppendDone { .. }) =>
                {
                    Some((id, op))
                }
                _ => None,
            };
            inner(event);
            if let Some((id, op)) = hook
                && let Some(backend) = cell.lock().unwrap().as_ref().and_then(Weak::upgrade)
            {
                backend.abort(id).expect("abort from inside the closure");
                let new = backend.append(id, seg.clone()).expect("append after abort");
                calls.lock().unwrap().push((op, new));
            }
        })
    };
    let backend = Arc::new(MseBackend::new(
        sink,
        Arc::new(AllDecoders),
        Limits::default(),
    ));
    *cell.lock().unwrap() = Some(Arc::downgrade(&backend));
    backend.open();
    let id = backend.add_source_buffer(&s.mime).unwrap();
    let init = backend.append(id, s.init.bytes.clone()).unwrap();
    assert!(queue.wait_for_completion_of(init, WAIT));
    let (event_op, new) = loop {
        if let Some(pair) = calls.lock().unwrap().first().copied() {
            break pair;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(event_op, init);
    assert!(new > init);
    assert!(queue.wait_for_completion_of(new, WAIT));
    // Nothing else completes for either id after that.
    std::thread::sleep(Duration::from_millis(30));
    let emitted = queue.emitted.lock().unwrap();
    assert_eq!(emitted.get(&init), Some(&1));
    assert_eq!(emitted.get(&new), Some(&1));
}
