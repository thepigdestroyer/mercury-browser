// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T04 of the GstMSE spike at core level (lane med-1, item 3): create, append and drop in every order,
//! with an append in flight, 200 iterations: no crash, and the threads and file descriptors of the
//! process are back to where they started (plus or minus two).
//!
//! This is the only test of its binary: other tests would start and stop threads of their own.

mod model_support;

use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use mercury_mse::MseBackend;
use mercury_mse::api::{
    AllDecoders, FeedState, Limits, MediaSourceBackend, MemoryBudget, MseEvent, SourceBufferId,
    TrackKind,
};
use mercury_mse_testkit::{SegmentedStream, tile};
use model_support::*;

struct Material {
    low: SegmentedStream,
    big: Vec<u8>,
}

/// Every backend of this test charges one budget: when all of them are gone it must read zero, which
/// is a deterministic leak detector for the frames' memory (the process RSS is too noisy to find a leak
/// of a few hundred kilobytes per lifecycle).
fn budget() -> &'static Arc<MemoryBudget> {
    static BUDGET: OnceLock<Arc<MemoryBudget>> = OnceLock::new();
    BUDGET.get_or_init(|| MemoryBudget::new(usize::MAX / 2))
}

fn new_backend(rec: &Arc<Recorder>) -> Arc<MseBackend> {
    let backend = Arc::new(MseBackend::with_budget(
        rec.sink(),
        Arc::new(AllDecoders),
        Limits::default(),
        Arc::clone(budget()),
    ));
    backend.open();
    backend
}

fn with_buffer(m: &Material, rec: &Arc<Recorder>) -> (Arc<MseBackend>, SourceBufferId) {
    let backend = new_backend(rec);
    let id = backend.add_source_buffer(&m.low.mime).unwrap();
    backend.append(id, m.low.init.bytes.clone()).unwrap();
    (backend, id)
}

fn wait_not_updating(backend: &MseBackend, id: SourceBufferId) {
    let deadline = Instant::now() + WAIT;
    while backend.updating(id) {
        assert!(Instant::now() < deadline, "still updating");
        thread::sleep(Duration::from_micros(200));
    }
}

/// One lifecycle, chosen by `variant`.
fn lifecycle(variant: usize, m: &Material) {
    let rec = Recorder::new();
    match variant {
        // dropped before anything started: no worker thread was ever made
        0 => drop(new_backend(&rec)),
        1 => {
            let backend = new_backend(&rec);
            backend.add_source_buffer(&m.low.mime).unwrap();
            drop(backend);
        }
        // dropped with an append in flight
        2 => {
            let (backend, id) = with_buffer(m, &rec);
            wait_not_updating(&backend, id);
            backend.append(id, m.big.clone()).unwrap();
            drop(backend);
        }
        // appended, waited for, dropped
        3 => {
            let (backend, id) = with_buffer(m, &rec);
            wait_not_updating(&backend, id);
            backend.append(id, m.low.media[0].bytes.clone()).unwrap();
            wait_not_updating(&backend, id);
            drop(backend);
        }
        // aborted, then dropped
        4 => {
            let (backend, id) = with_buffer(m, &rec);
            wait_not_updating(&backend, id);
            backend.append(id, m.big.clone()).unwrap();
            backend.abort(id).unwrap();
            drop(backend);
        }
        // closed with an append in flight, then dropped
        5 => {
            let (backend, id) = with_buffer(m, &rec);
            wait_not_updating(&backend, id);
            backend.append(id, m.big.clone()).unwrap();
            backend.close();
            drop(backend);
        }
        // a feed handle outlives the backend and still answers
        6 => {
            let (backend, id) = with_buffer(m, &rec);
            wait_not_updating(&backend, id);
            backend.append(id, m.low.media[0].bytes.clone()).unwrap();
            let feed = backend.frame_feed();
            feed.set_wake(Arc::new(|| {}));
            drop(backend);
            let batch = feed.pull(TrackKind::Video, 8);
            assert_ne!(batch.state, FeedState::More);
            assert!(feed.tracks().is_empty());
        }
        // dropped on another thread
        7 => {
            let (backend, id) = with_buffer(m, &rec);
            wait_not_updating(&backend, id);
            backend.append(id, m.big.clone()).unwrap();
            thread::spawn(move || drop(backend)).join().unwrap();
        }
        // events pending behind a slow closure at the time of the drop
        8 => {
            rec.add_hook(|_| thread::sleep(Duration::from_millis(3)));
            let (backend, id) = with_buffer(m, &rec);
            let _ = backend.set_duration(Some(5.0));
            let _ = backend.append(id, m.low.media[0].bytes.clone());
            drop(backend);
        }
        // two backends dropped in the reverse order
        _ => {
            let (a, ida) = with_buffer(m, &rec);
            let (b, idb) = with_buffer(m, &rec);
            let _ = b.append(idb, m.big.clone());
            let _ = a.remove(ida, 0.0, 1.0);
            drop(b);
            drop(a);
        }
    }
}

#[test]
fn t04_create_append_drop_in_every_order_200_iterations() {
    let low = mercury_mse_testkit::load("h264-low").unwrap();
    let tiled = tile(&low, 25).unwrap();
    let mut big = Vec::new();
    for s in &tiled.media {
        big.extend_from_slice(&s.bytes);
    }
    let m = Material { low, big };
    // Warm-up: the first iterations may start lazily created process-wide things.
    for i in 0..20 {
        lifecycle(i % 10, &m);
    }
    thread::sleep(Duration::from_millis(100));
    let threads_before = thread_count();
    let fds_before = fd_count();
    for i in 0..200 {
        lifecycle(i % 10, &m);
    }
    // The workers that ended on their own (a drop on the worker thread) need a moment to be reaped.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let threads_after = thread_count();
        let fds_after = fd_count();
        let threads_ok = match (threads_before, threads_after) {
            (Some(b), Some(a)) => a <= b + 2,
            _ => true,
        };
        let fds_ok = match (fds_before, fds_after) {
            (Some(b), Some(a)) => a <= b + 2,
            _ => true,
        };
        if threads_ok && fds_ok {
            eprintln!(
                "t04: threads {threads_before:?} -> {threads_after:?}, descriptors {fds_before:?} -> {fds_after:?}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "after 200 lifecycles: threads {threads_before:?} -> {threads_after:?}, descriptors {fds_before:?} -> {fds_after:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
    // Everything the frames held went back to the budget (the workers are gone, so nothing is in
    // flight): a leak of even one frame per lifecycle would show here.
    let deadline = Instant::now() + Duration::from_secs(3);
    while budget().used() != 0 {
        assert!(
            Instant::now() < deadline,
            "after 220 lifecycles the budget still holds {} bytes",
            budget().used()
        );
        thread::sleep(Duration::from_millis(20));
    }
    let _ = MseEvent::BufferedChanged { id: 0 };
}
