// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T01 and T02 of the GstMSE spike at core level (lane med-1, item 3): which thread events arrive on,
//! how fast `append` returns, and whether every method of the backend may be called from inside every
//! event closure without a deadlock or a panic (R17 section 11.2 rules 2 and 3).

mod model_support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use mercury_mse::MseBackend;
use mercury_mse::api::{
    AppendMode, EosError, FrameFeed, Limits, MediaSourceBackend, MseEvent, SourceBufferId,
    TrackKind,
};
use mercury_mse_testkit::tile;
use model_support::*;

// ---- T01 ----

#[test]
fn t01_events_arrive_on_the_worker_thread_never_on_the_callers() {
    let s = load("h264-aac");
    let rig = Rig::new();
    let me = thread::current().id();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &mercury_mse_testkit::ChunkPlan::Whole);
    // Events that synchronous calls cause are posted by the worker too.
    rig.backend.set_duration(Some(20.0)).unwrap();
    rig.backend.end_of_stream(None).unwrap();
    assert!(rig.rec.wait_for(WAIT, |log| {
        log.iter()
            .any(|r| matches!(r.event, MseEvent::SourceEnded { .. }))
    }));
    let log = rig.rec.snapshot();
    assert!(log.len() >= 8, "{} events", log.len());
    let first = log[0].thread;
    for r in &log {
        assert_ne!(r.thread, me, "{:?} arrived on the caller's thread", r.event);
        assert!(
            r.thread_name.starts_with("mse-"),
            "{:?} arrived on {:?}",
            r.event,
            r.thread_name
        );
        assert_eq!(r.thread, first, "one worker thread per MediaSource");
    }
    // Two backends have two workers with different names.
    let other = Rig::new();
    let oid = other.add(&s.mime);
    other.append_ok(oid, &s.init.bytes);
    let other_name = other.rec.snapshot()[0].thread_name.clone();
    assert_ne!(other_name, log[0].thread_name);
}

#[test]
fn t01_append_of_one_megabyte_returns_in_under_5_ms() {
    let low = load("h264-low");
    let big = tile(&low, 18).unwrap();
    let mut data = Vec::new();
    for seg in &big.media {
        data.extend_from_slice(&seg.bytes);
    }
    assert!(data.len() >= 1_000_000, "{} bytes", data.len());
    let rig = Rig::new();
    let id = rig.add(&low.mime);
    rig.append_ok(id, &big.init.bytes);
    let mut times = Vec::new();
    let mut dones = Vec::new();
    for _ in 0..15 {
        let copy = data.clone();
        let before = rig.rec.completions(id);
        let t0 = Instant::now();
        rig.backend.append(id, copy).unwrap();
        times.push(t0.elapsed());
        assert!(rig.backend.updating(id) || rig.rec.completions(id) > before);
        let outcome = rig.rec.wait_completion(id, before, WAIT).expect("done");
        dones.push(t0.elapsed());
        assert_eq!(outcome, Outcome::Done);
    }
    times.sort();
    dones.sort();
    let median = times[times.len() / 2];
    eprintln!(
        "t01: append(1 MB) returns in median {median:?} (max {:?}); AppendDone after median {:?} (max {:?})",
        times.last().unwrap(),
        dones[dones.len() / 2],
        dones.last().unwrap()
    );
    assert!(
        median < Duration::from_millis(5),
        "median append latency {median:?}"
    );
}

// ---- T02 ----

/// Calls every method of the backend with arguments that are mostly valid, ignoring the results. The
/// destructive ones (`close`, `remove_source_buffer`) are included when `destructive` is set.
fn call_everything(backend: &Arc<MseBackend>, id: SourceBufferId, destructive: bool) {
    let b: &dyn MediaSourceBackend = &**backend;
    let mime = "video/mp4; codecs=\"avc1.42C00C\"";
    b.open();
    let _ = b.ready_state();
    let new = b.add_source_buffer(mime);
    let _ = b.source_buffer_count();
    let _ = b.duration();
    let _ = b.set_duration(Some(30.0));
    let _ = b.set_duration(Some(f64::INFINITY));
    let _ = b.set_live_seekable_range(0.0, 5.0);
    let _ = b.clear_live_seekable_range();
    for target in [Some(id), new.as_ref().ok().copied()].into_iter().flatten() {
        let _ = b.append(target, Vec::new());
        let _ = b.abort(target);
        let _ = b.remove(target, 0.0, 1.0);
        let _ = b.change_type(target, mime);
        let _ = b.updating(target);
        let _ = b.buffered(target);
        let _ = b.mode(target);
        let _ = b.set_mode(target, AppendMode::Segments);
        let _ = b.timestamp_offset(target);
        let _ = b.set_timestamp_offset(target, 0.0);
        let _ = b.append_window_start(target);
        let _ = b.append_window_end(target);
        let _ = b.set_append_window_start(target, 0.0);
        let _ = b.set_append_window_end(target, f64::INFINITY);
    }
    let _ = b.element_buffered();
    let _ = b.seekable();
    let _ = b.element_ready_state(1.0);
    b.set_position(1.0);
    let _ = b.seeking(1.5);
    let feed = b.frame_feed();
    let _ = feed.tracks();
    let _ = feed.all_initialized();
    let _ = FrameFeed::duration(&**backend);
    feed.seek(1_000_000_000);
    for kind in TrackKind::ALL {
        let _ = feed.pull(kind, 8);
    }
    feed.set_wake(Arc::new(|| {}));
    let _ = b.end_of_stream(Some(EosError::Network));
    if destructive {
        if let Ok(extra) = new {
            let _ = b.remove_source_buffer(extra);
        }
        b.close();
        b.open();
    }
}

/// The scenarios, each producing one kind of event.
#[derive(Clone, Copy, Debug)]
enum Kind {
    InitSegment,
    AppendDone,
    AppendError,
    RemoveDone,
    DurationChanged,
    SourceEnded,
    BufferedChanged,
}

fn is_kind(event: &MseEvent, kind: Kind) -> bool {
    matches!(
        (event, kind),
        (MseEvent::InitSegment { .. }, Kind::InitSegment)
            | (MseEvent::AppendDone { .. }, Kind::AppendDone)
            | (MseEvent::AppendError { .. }, Kind::AppendError)
            | (MseEvent::RemoveDone { .. }, Kind::RemoveDone)
            | (MseEvent::DurationChanged(_), Kind::DurationChanged)
            | (MseEvent::SourceEnded { .. }, Kind::SourceEnded)
            | (MseEvent::BufferedChanged { .. }, Kind::BufferedChanged)
    )
}

/// Runs the scenario that produces `kind` with a hook that calls everything from inside every event of
/// that kind (and a budget, so that the appends the hook makes do not feed back for ever).
fn scenario(kind: Kind, destructive: bool) {
    let s = load("h264-low");
    let limits = if matches!(kind, Kind::BufferedChanged) {
        Limits {
            video_bytes: 100_000,
            ..Limits::default()
        }
    } else {
        Limits::default()
    };
    let rig = Rig::with_limits(limits);
    let id = rig.add(&s.mime);
    let calls = Arc::new(AtomicUsize::new(0));
    let weak: Weak<MseBackend> = Arc::downgrade(&rig.backend);
    {
        let calls = Arc::clone(&calls);
        rig.rec.add_hook(move |event| {
            if is_kind(event, kind)
                && calls.fetch_add(1, Ordering::SeqCst) < 12
                && let Some(backend) = weak.upgrade()
            {
                call_everything(&backend, id, destructive);
            }
        });
    }
    // In the destructive variant a closure may close the MediaSource or remove the buffer, which cancels
    // the append the scenario is waiting for: then the scenario does not wait for completions.
    let append = |bytes: &[u8]| {
        if destructive {
            let _ = rig.backend.append(id, bytes.to_vec());
            thread::sleep(Duration::from_millis(15));
        } else {
            let _ = rig.append(id, bytes);
        }
    };
    match kind {
        Kind::InitSegment | Kind::AppendDone | Kind::DurationChanged => {
            append(&s.init.bytes);
            for seg in &s.media {
                append(&seg.bytes);
            }
        }
        Kind::AppendError => {
            append(&[0xAB; 64]);
        }
        Kind::RemoveDone => {
            append(&s.init.bytes);
            append(&s.media[0].bytes);
            let _ = rig.backend.set_duration(Some(8.0));
            let before = rig.rec.completions(id);
            if rig.backend.remove(id, 0.0, 1.0).is_ok() {
                if destructive {
                    thread::sleep(Duration::from_millis(30));
                } else {
                    let _ = rig.rec.wait_completion(id, before, WAIT);
                }
            }
        }
        Kind::SourceEnded => {
            append(&s.init.bytes);
            append(&s.media[0].bytes);
            let _ = rig.backend.end_of_stream(None);
        }
        Kind::BufferedChanged => {
            let long = tile(&s, 5).unwrap();
            append(&long.init.bytes);
            for (i, seg) in long.media.iter().enumerate() {
                rig.backend.set_position((i as f64) * 2.0);
                append(&seg.bytes);
            }
        }
    }
    // Let everything the hook caused settle: the worker is idle when a probe job comes back.
    let _ = rig.backend.set_duration(Some(1234.5));
    thread::sleep(Duration::from_millis(30));
    assert!(
        rig.rec.count(|e| is_kind(e, kind)) >= 1,
        "the scenario for {kind:?} produced no such event"
    );
}

#[test]
fn t02_every_method_from_inside_every_event_closure() {
    for kind in [
        Kind::InitSegment,
        Kind::AppendDone,
        Kind::AppendError,
        Kind::RemoveDone,
        Kind::DurationChanged,
        Kind::SourceEnded,
        Kind::BufferedChanged,
    ] {
        watchdog(
            &format!("t02-{kind:?}"),
            Duration::from_secs(2),
            move || scenario(kind, false),
        );
    }
}

#[test]
fn t02_close_and_remove_source_buffer_from_inside_every_event_closure() {
    for kind in [
        Kind::InitSegment,
        Kind::AppendDone,
        Kind::AppendError,
        Kind::RemoveDone,
        Kind::DurationChanged,
        Kind::SourceEnded,
    ] {
        watchdog(
            &format!("t02d-{kind:?}"),
            Duration::from_secs(2),
            move || scenario(kind, true),
        );
    }
}

#[test]
fn t02_the_last_reference_dropped_inside_an_event_closure() {
    // The closure owns the only strong reference and drops it on the worker thread: the drop must not
    // try to join the thread it is running on, and must not panic.
    let s = load("h264-low");
    let slot: Arc<Mutex<Option<Arc<MseBackend>>>> = Arc::new(Mutex::new(None));
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let rec = Recorder::new();
    let backend = Arc::new(MseBackend::new(
        rec.sink(),
        Arc::new(mercury_mse::api::AllDecoders),
        Limits::default(),
    ));
    backend.open();
    let id = backend.add_source_buffer(&s.mime).unwrap();
    {
        let slot = Arc::clone(&slot);
        let done_tx = done_tx.clone();
        rec.add_hook(move |event| {
            if matches!(event, MseEvent::AppendDone { .. }) {
                let taken = slot.lock().unwrap().take();
                if let Some(backend) = taken {
                    drop(backend);
                    let _ = done_tx.send(());
                }
            }
        });
    }
    *slot.lock().unwrap() = Some(Arc::clone(&backend));
    // Give up the test's own reference after the append is queued: the slot's one is then the last.
    backend.append(id, s.init.bytes.clone()).unwrap();
    drop(backend);
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the closure dropped the backend and returned");
    // The worker thread ends by itself.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if rec.snapshot().last().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn t02_a_slow_event_closure_does_not_stall_the_callers() {
    // Events are posted from the worker: a closure that takes a long time delays later events but never
    // the calls of the script thread.
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    let gate = Arc::new(Mutex::new(()));
    let held = gate.lock().unwrap();
    {
        let gate = Arc::clone(&gate);
        rig.rec.add_hook(move |event| {
            if matches!(event, MseEvent::InitSegment { .. }) {
                drop(gate.lock().unwrap());
            }
        });
    }
    rig.backend.append(id, s.init.bytes.clone()).unwrap();
    // The worker is now (or soon) stuck in the closure; the script thread's calls all return at once.
    let t0 = Instant::now();
    for _ in 0..1000 {
        let _ = rig.backend.buffered(id);
        let _ = rig.backend.updating(id);
        let _ = rig.backend.element_buffered();
        let _ = rig.backend.seekable();
        rig.backend.set_position(1.0);
    }
    let _ = rig.backend.set_duration(Some(3.0));
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "{:?}",
        t0.elapsed()
    );
    drop(held);
    rig.wait_idle(&[id]);
}
