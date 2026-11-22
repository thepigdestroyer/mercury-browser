// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `abort()` during a removal, with a real `MseBackend` (real worker thread, real parser) and the DOM's
//! half played by `SourceBufferOps` (med-1b review fix, finding 1). The MSE `abort()` algorithm, step 3:
//! "If the range removal algorithm is running, then throw an InvalidStateError". The removal runs from
//! `remove()` until the `RemoveDone` is handled by the page's task, so `abort()` must throw whether the
//! backend's worker is still busy or has already finished and queued the event. Before the fix the DOM
//! asked the backend's busy flag, which the worker clears *before* it posts `RemoveDone`: the answer
//! depended on a thread race.
//!
//! The same shape is exercised by the random walk of `serial_backend.rs`.

mod model_support;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use mercury_mse::MseBackend;
use mercury_mse::api::{
    AllDecoders, EventSink, Limits, MediaSourceBackend, MseEvent, OpId, SourceBufferId,
};
use mercury_mse::source_buffer_ops::{SbEvent, SourceBufferOps};
use model_support::{WAIT, load};

fn completion(event: &MseEvent) -> Option<(SourceBufferId, OpId)> {
    match event {
        MseEvent::AppendDone { id, op }
        | MseEvent::AppendError { id, op, .. }
        | MseEvent::RemoveDone { id, op } => Some((*id, *op)),
        _ => None,
    }
}

/// The script task queue plus a gate that can hold the worker thread inside an event delivery.
#[derive(Default)]
struct Rig {
    events: Mutex<VecDeque<MseEvent>>,
    cv: Condvar,
    /// When set, the next `AppendDone` blocks the worker thread until `release` is called.
    arm: AtomicBool,
    gate: Mutex<Gate>,
    gate_cv: Condvar,
}

#[derive(Default)]
struct Gate {
    holding: bool,
    released: bool,
}

impl Rig {
    fn sink(self: &Arc<Self>) -> EventSink {
        let rig = Arc::clone(self);
        Arc::new(move |event| {
            let hold = matches!(event, MseEvent::AppendDone { .. })
                && rig.arm.swap(false, Ordering::SeqCst);
            if hold {
                let mut gate = rig.gate.lock().unwrap();
                gate.holding = true;
                rig.gate_cv.notify_all();
                while !gate.released {
                    gate = rig.gate_cv.wait(gate).unwrap();
                }
                gate.holding = false;
                gate.released = false;
            }
            rig.events.lock().unwrap().push_back(event);
            rig.cv.notify_all();
        })
    }

    /// The worker is (about to be) held inside the next `AppendDone`.
    fn hold_the_worker_at_the_next_append_done(&self) {
        self.arm.store(true, Ordering::SeqCst);
    }

    fn wait_until_the_worker_is_held(&self) {
        let deadline = Instant::now() + WAIT;
        let mut gate = self.gate.lock().unwrap();
        while !gate.holding {
            let now = Instant::now();
            assert!(now < deadline, "the worker never reached the gate");
            gate = self.gate_cv.wait_timeout(gate, deadline - now).unwrap().0;
        }
    }

    fn release_the_worker(&self) {
        let mut gate = self.gate.lock().unwrap();
        gate.released = true;
        self.gate_cv.notify_all();
    }

    fn pop(&self) -> Option<MseEvent> {
        self.events.lock().unwrap().pop_front()
    }

    /// Waits until a completion event of `op` is queued (not consumed).
    fn wait_for_completion_of(&self, op: OpId) {
        let deadline = Instant::now() + WAIT;
        let mut events = self.events.lock().unwrap();
        loop {
            if events
                .iter()
                .any(|e| completion(e).is_some_and(|(_, o)| o == op))
            {
                return;
            }
            let now = Instant::now();
            assert!(now < deadline, "no completion event for {op}");
            events = self.cv.wait_timeout(events, deadline - now).unwrap().0;
        }
    }

    /// The page's task for the queued completion events: returns what `ops` says for each, in order.
    fn deliver(
        &self,
        ops: &mut SourceBufferOps,
        only: SourceBufferId,
    ) -> Vec<(OpId, Vec<SbEvent>)> {
        let mut out = Vec::new();
        while let Some(event) = self.pop() {
            let Some((id, op)) = completion(&event) else {
                continue;
            };
            if id != only {
                continue;
            }
            let events = match event {
                MseEvent::AppendError { .. } => ops.failed(op),
                _ => ops.finished(op),
            };
            out.push((op, events));
        }
        out
    }
}

/// Lets the held worker go when the test ends, however it ends.
struct ReleaseOnDrop(Arc<Rig>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release_the_worker();
    }
}

fn backend(rig: &Arc<Rig>) -> Arc<MseBackend> {
    let backend = Arc::new(MseBackend::new(
        rig.sink(),
        Arc::new(AllDecoders),
        Limits::default(),
    ));
    backend.open();
    backend
}

/// A buffer with its init segment and first media segment appended and the events delivered.
fn loaded(rig: &Arc<Rig>, backend: &MseBackend, name: &str) -> (SourceBufferId, SourceBufferOps) {
    let s = load(name);
    let id = backend.add_source_buffer(&s.mime).unwrap();
    let mut ops = SourceBufferOps::new();
    for bytes in [&s.init.bytes, &s.media[0].bytes] {
        let op = backend.append(id, bytes.clone()).unwrap();
        let _ = ops.begin_append(op);
        rig.wait_for_completion_of(op);
        let done = rig.deliver(&mut ops, id);
        assert_eq!(done.last().map(|(o, _)| *o), Some(op));
    }
    assert!(!ops.updating());
    assert!(!backend.buffered(id).is_empty());
    (id, ops)
}

#[test]
fn ops_abort_is_refused_after_the_worker_finished_the_removal_before_the_page_saw_it() {
    let rig = Arc::new(Rig::default());
    let backend = backend(&rig);
    let (id, mut ops) = loaded(&rig, &backend, "aac");

    let op = backend.remove(id, 0.0, f64::INFINITY).unwrap();
    assert_eq!(ops.begin_remove(op), SbEvent::UpdateStart);
    // The worker finished and posted RemoveDone; the page's task has not run it yet.
    rig.wait_for_completion_of(op);
    assert!(
        !backend.updating(id),
        "the worker is done: the backend's own busy flag is clear (the case the old check missed)"
    );
    assert!(
        backend.buffered(id).is_empty(),
        "the removal itself took effect"
    );

    assert!(
        ops.abort().is_err(),
        "abort() must throw while the removal's RemoveDone is unhandled"
    );
    assert!(
        ops.updating(),
        "a refused abort() changes nothing: updating is still true"
    );

    // The page's task now handles RemoveDone: one update and one updateend, nothing was retired.
    assert_eq!(
        rig.deliver(&mut ops, id),
        vec![(op, vec![SbEvent::Update, SbEvent::UpdateEnd])]
    );
    assert!(!ops.updating());
    assert_eq!(ops.abort(), Ok(Vec::new()));
}

#[test]
fn ops_abort_is_refused_while_the_worker_has_not_started_the_removal() {
    let rig = Arc::new(Rig::default());
    let backend = backend(&rig);
    let (a, _a_ops) = loaded(&rig, &backend, "aac");
    let (b, mut b_ops) = loaded(&rig, &backend, "aac");
    let s = load("aac");
    // A failed assertion below must not leave the worker held: dropping the backend joins it.
    let _release_on_unwind = ReleaseOnDrop(Arc::clone(&rig));

    // Hold the one worker thread of the media source inside an event delivery of buffer `a`'s append,
    // so a removal on `b` is accepted but not run.
    rig.hold_the_worker_at_the_next_append_done();
    let held = backend.append(a, s.media[1].bytes.clone()).unwrap();
    rig.wait_until_the_worker_is_held();

    let op = backend.remove(b, 0.0, f64::INFINITY).unwrap();
    assert_eq!(b_ops.begin_remove(op), SbEvent::UpdateStart);
    assert!(
        backend.updating(b),
        "the worker has not run the removal: the backend still counts it as updating"
    );
    assert!(b_ops.abort().is_err());
    assert!(b_ops.updating());
    // The backend itself refuses in this state too (its own second line of defence).
    assert!(backend.abort(b).is_err());

    rig.release_the_worker();
    rig.wait_for_completion_of(op);
    rig.wait_for_completion_of(held);
    // Now the worker has finished: still refused, still pending.
    assert!(b_ops.abort().is_err());
    assert!(b_ops.updating());

    assert_eq!(
        rig.deliver(&mut b_ops, b),
        vec![(op, vec![SbEvent::Update, SbEvent::UpdateEnd])]
    );
    assert!(!b_ops.updating());
}

#[test]
fn ops_remove_then_immediate_abort_is_always_refused_whoever_wins_the_race() {
    let rig = Arc::new(Rig::default());
    let backend = backend(&rig);
    let (id, mut ops) = loaded(&rig, &backend, "aac");
    let s = load("aac");

    let mut finished_before_abort = 0;
    for round in 0..300 {
        // Fill something to remove so the worker has real work (alternating with an empty range).
        if round % 2 == 0 {
            let op = backend
                .append(id, s.media[round / 2 % s.media.len()].bytes.clone())
                .unwrap();
            let _ = ops.begin_append(op);
            rig.wait_for_completion_of(op);
            rig.deliver(&mut ops, id);
        }
        let op = backend.remove(id, 0.0, f64::INFINITY).unwrap();
        let _ = ops.begin_remove(op);
        let worker_done = !backend.updating(id);
        finished_before_abort += usize::from(worker_done);

        assert!(
            ops.abort().is_err(),
            "round {round}: abort() during a removal succeeded"
        );
        assert!(ops.updating(), "round {round}");

        rig.wait_for_completion_of(op);
        assert!(
            ops.abort().is_err(),
            "round {round}: refused after the worker finished"
        );
        assert_eq!(
            rig.deliver(&mut ops, id),
            vec![(op, vec![SbEvent::Update, SbEvent::UpdateEnd])],
            "round {round}"
        );
        assert!(!ops.updating());
    }
    eprintln!(
        "remove then abort: the worker was already done in {finished_before_abort}/300 rounds"
    );
}

#[test]
fn ops_abort_after_an_append_whose_done_is_queued_retires_it() {
    // The other half of the same rule: an append is abortable, and its queued AppendDone is stale.
    let rig = Arc::new(Rig::default());
    let backend = backend(&rig);
    let (id, mut ops) = loaded(&rig, &backend, "aac");
    let s = load("aac");

    let first = backend.append(id, s.media[1].bytes.clone()).unwrap();
    let _ = ops.begin_append(first);
    rig.wait_for_completion_of(first);

    // The page's task: abort(), then appendBuffer() again.
    assert!(ops.check_abort().is_ok());
    backend.abort(id).unwrap();
    assert_eq!(ops.abort(), Ok(vec![SbEvent::Abort, SbEvent::UpdateEnd]));
    let second = backend.append(id, s.media[2].bytes.clone()).unwrap();
    let _ = ops.begin_append(second);

    // The stale event first, then the new one: only the new one ends the new append.
    let stale = rig
        .pop()
        .expect("the stale AppendDone is first in the queue");
    assert_eq!(completion(&stale), Some((id, first)));
    assert!(ops.finished(first).is_empty());
    assert!(ops.updating());
    rig.wait_for_completion_of(second);
    assert_eq!(
        rig.deliver(&mut ops, id),
        vec![(second, vec![SbEvent::Update, SbEvent::UpdateEnd])]
    );
}
