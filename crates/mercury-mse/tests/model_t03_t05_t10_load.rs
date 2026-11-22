// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T03, T05 and T10 of the GstMSE spike at core level (lane med-1, item 3): concurrent callers (four
//! threads on four source buffers, two on one, a thread polling every millisecond), throughput (the
//! latency of one large append, a thousand small ones), and sixteen backends at once.

mod model_support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use mercury_mse::api::{
    AppendErrorKind, MediaSourceBackend, MseError, MseEvent, TimeRange, TrackKind,
};
use mercury_mse_testkit::{ChunkPlan, SegmentedStream, tile};
use model_support::*;

fn is_normalized(ranges: &[TimeRange]) -> bool {
    ranges.iter().all(|r| r.start < r.end) && ranges.windows(2).all(|w| w[0].end < w[1].start)
}

// ---- T03 ----

#[test]
fn t03_four_threads_on_four_source_buffers() {
    let streams: Vec<SegmentedStream> = ["h264-low", "aac", "vp9-low", "opus"]
        .iter()
        .map(|n| load(n))
        .collect();
    let rig = Arc::new(Rig::new());
    let ids: Vec<u32> = streams.iter().map(|s| rig.add(&s.mime)).collect();
    let barrier = Arc::new(Barrier::new(streams.len()));
    let handles: Vec<_> = streams
        .into_iter()
        .zip(ids.iter().copied())
        .map(|(stream, id)| {
            let rig = Arc::clone(&rig);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                rig.append_stream(id, &stream, &ChunkPlan::Fixed(4096));
            })
        })
        .collect();
    for h in handles {
        h.join().expect("an appending thread panicked");
    }
    for id in &ids {
        assert_ranges(
            &format!("buffer {id}"),
            &rig.backend.buffered(*id),
            &[(0.0, 8.0)],
            2.0,
        );
    }
    rig.backend.end_of_stream(None).unwrap();
    // The feed serves the first video and first audio buffer; both complete.
    let feed = rig.backend.frame_feed();
    assert_eq!(drain(&*feed, TrackKind::Video).frames.len(), 200);
    assert_eq!(drain(&*feed, TrackKind::Audio).frames.len(), 375);
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendError { .. })),
        0
    );
}

#[test]
fn t03_two_threads_on_one_source_buffer_the_second_gets_invalid_state() {
    let s = load("h264-low");
    let rig = Arc::new(Rig::new());
    let id = rig.add(&s.mime);
    rig.append_ok(id, &s.init.bytes);
    let refusals = Arc::new(AtomicUsize::new(0));
    let accepted = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|t| {
            let rig = Arc::clone(&rig);
            let barrier = Arc::clone(&barrier);
            let (refusals, accepted) = (Arc::clone(&refusals), Arc::clone(&accepted));
            let seg = s.media.iter().map(|m| m.bytes.clone()).collect::<Vec<_>>();
            thread::spawn(move || {
                barrier.wait();
                for round in 0..200 {
                    let bytes = seg[(round + t) % seg.len()].clone();
                    let before = rig.rec.completions(id);
                    match rig.backend.append(id, bytes) {
                        Ok(_) => {
                            accepted.fetch_add(1, Ordering::SeqCst);
                            // Wait for the append that this thread started (the other thread's appends
                            // interleave in the count, so wait until updating is false).
                            let deadline = Instant::now() + WAIT;
                            while rig.backend.updating(id) {
                                assert!(Instant::now() < deadline, "stuck updating");
                                thread::yield_now();
                            }
                            let _ = before;
                        }
                        Err(MseError::InvalidState(_)) => {
                            refusals.fetch_add(1, Ordering::SeqCst);
                            thread::yield_now();
                        }
                        Err(other) => panic!("a refusal that is not InvalidState: {other:?}"),
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    rig.wait_idle(&[id]);
    eprintln!(
        "t03: {} appends accepted, {} refused with InvalidState",
        accepted.load(Ordering::SeqCst),
        refusals.load(Ordering::SeqCst)
    );
    // One completion event per accepted append, and no error.
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendDone { .. })),
        accepted.load(Ordering::SeqCst) + 1,
        "the init append and every accepted append"
    );
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendError { .. })),
        0
    );
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 8.0)], 2.0);
}

#[test]
fn t03_a_thread_polling_every_millisecond_sees_consistent_answers() {
    let (v, a) = (load("h264-aac"), load("opus"));
    let rig = Arc::new(Rig::new());
    let vid = rig.add(&v.mime);
    let aid = rig.add(&a.mime);
    let stop = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let poller = {
        let rig = Arc::clone(&rig);
        let (stop, polls) = (Arc::clone(&stop), Arc::clone(&polls));
        thread::spawn(move || {
            let mut last_done = 0;
            while !stop.load(Ordering::SeqCst) {
                for id in [vid, aid] {
                    let ranges = rig.backend.buffered(id);
                    assert!(is_normalized(&ranges), "buffered of {id}: {ranges:?}");
                    let _ = rig.backend.updating(id);
                }
                let el = rig.backend.element_buffered();
                assert!(is_normalized(&el), "element buffered: {el:?}");
                let _ = rig.backend.seekable();
                let _ = rig.backend.element_ready_state(1.0);
                // The completed appends only ever grow.
                let done = rig.rec.count(|e| matches!(e, MseEvent::AppendDone { .. }));
                assert!(done >= last_done);
                last_done = done;
                polls.fetch_add(1, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(1));
            }
        })
    };
    let feed = rig.backend.frame_feed();
    let consumer = {
        let feed = Arc::clone(&feed);
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                for kind in [TrackKind::Video, TrackKind::Audio] {
                    let _ = feed.pull(kind, 16);
                }
                thread::sleep(Duration::from_millis(1));
            }
        })
    };
    for _round in 0..4 {
        rig.append_stream(vid, &v, &ChunkPlan::Fixed(8192));
        rig.append_stream(aid, &a, &ChunkPlan::Fixed(8192));
    }
    // remove during playback, from the main thread
    rig.remove_ok(vid, 0.0, 2.0);
    rig.remove_ok(aid, 0.0, 2.0);
    let deadline = Instant::now() + WAIT;
    while polls.load(Ordering::SeqCst) < 20 {
        assert!(Instant::now() < deadline, "the poller never ran");
        thread::sleep(Duration::from_millis(2));
    }
    stop.store(true, Ordering::SeqCst);
    poller.join().unwrap();
    consumer.join().unwrap();
    assert!(polls.load(Ordering::SeqCst) > 5, "the poller ran");
}

// ---- T05 ----

#[test]
fn t05_append_done_within_100_ms_of_a_one_megabyte_append_and_the_data_is_buffered() {
    let low = load("h264-low");
    let big = tile(&low, 18).unwrap();
    let mut data = Vec::new();
    for seg in &big.media {
        data.extend_from_slice(&seg.bytes);
    }
    assert!(data.len() >= 1_000_000);
    let rig = Rig::new();
    let id = rig.add(&low.mime);
    rig.append_ok(id, &big.init.bytes);
    // At the moment AppendDone is delivered the data is in buffered and updating is false.
    let seen = Arc::new(std::sync::Mutex::new(None));
    {
        let backend = Arc::clone(&rig.backend);
        let seen = Arc::clone(&seen);
        rig.rec.add_hook(move |event| {
            if matches!(event, MseEvent::AppendDone { .. }) {
                *seen.lock().unwrap() = Some((backend.updating(id), backend.buffered(id)));
            }
        });
    }
    let mut latencies = Vec::new();
    for _ in 0..5 {
        let t0 = Instant::now();
        rig.append_ok(id, &data);
        latencies.push(t0.elapsed());
    }
    latencies.sort();
    eprintln!(
        "t05: 1 MB append to AppendDone: median {:?}, max {:?}",
        latencies[2],
        latencies.last().unwrap()
    );
    assert!(
        latencies[2] < Duration::from_millis(100),
        "median {:?}",
        latencies[2]
    );
    let (updating, ranges) = seen.lock().unwrap().clone().expect("a hook run");
    assert!(!updating, "updating is false when AppendDone arrives");
    assert_ranges("buffered at AppendDone", &ranges, &[(0.0, 144.0)], 2.0);
}

#[test]
fn t05_a_thousand_small_appends() {
    let s = load("opus");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    let mut all = s.init.bytes.clone();
    for m in &s.media {
        all.extend_from_slice(&m.bytes);
    }
    let piece = all.len() / 1000 + 1;
    let t0 = Instant::now();
    let mut count = 0;
    for chunk in all.chunks(piece) {
        rig.append_ok(id, chunk);
        count += 1;
    }
    let elapsed = t0.elapsed();
    assert!(count >= 900, "{count} appends");
    eprintln!(
        "t05: {count} small appends in {elapsed:?} ({:?} each)",
        elapsed / count as u32
    );
    assert!(elapsed < Duration::from_secs(10));
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 8.0)], 2.0);
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendDone { .. })),
        count
    );
    assert_eq!(
        rig.rec.count(|e| matches!(
            e,
            MseEvent::AppendError {
                kind: AppendErrorKind::Decode,
                ..
            }
        )),
        0
    );
}

// ---- T10 ----

#[test]
fn t10_sixteen_backends_at_once_do_not_cross_talk() {
    let low = load("h264-low");
    let aac = load("aac");
    let n = 16;
    let barrier = Arc::new(Barrier::new(n));
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let (low, aac) = (low.clone(), aac.clone());
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let rig = Rig::new();
                let (vid, aid) = (rig.add(&low.mime), rig.add(&aac.mime));
                // Each backend plays its own copy of the media at its own offset.
                let offset = i as f64;
                rig.backend.set_timestamp_offset(vid, offset).unwrap();
                rig.backend.set_timestamp_offset(aid, offset).unwrap();
                barrier.wait();
                rig.append_stream(vid, &low, &ChunkPlan::Fixed(10_000 + i * 500));
                rig.append_stream(aid, &aac, &ChunkPlan::Fixed(10_000 + i * 500));
                rig.backend.end_of_stream(None).unwrap();
                (i, rig)
            })
        })
        .collect();
    let rigs: Vec<(usize, Rig)> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let mut worker_names = std::collections::HashSet::new();
    for (i, rig) in &rigs {
        let offset = *i as f64;
        let ids: Vec<u32> = vec![0, 1];
        for id in ids {
            assert_ranges(
                &format!("backend {i} buffer {id}"),
                &rig.backend.buffered(id),
                &[(offset, offset + 8.0)],
                2.0,
            );
        }
        let d = rig.backend.duration().unwrap();
        assert!(
            (d - (offset + 8.0)).abs() < 0.05,
            "backend {i} duration {d}"
        );
        let feed = rig.backend.frame_feed();
        // The stream starts at its offset: the player seeks there first (a position far before the
        // first buffered range waits for data that will not come).
        feed.seek((offset * 1e9) as i64);
        let v = drain(&*feed, TrackKind::Video);
        assert_eq!(v.frames.len(), 200, "backend {i}");
        assert_eq!(
            v.frames[0].pts,
            (offset * 1e9) as i64,
            "backend {i} first frame"
        );
        let a = drain(&*feed, TrackKind::Audio);
        assert_eq!(a.frames.len(), 375, "backend {i}");
        // Only this backend's own ids appear in its events, and each on its own worker thread.
        let events = rig.rec.snapshot();
        for r in &events {
            if let MseEvent::AppendDone { id, .. } | MseEvent::InitSegment { id, .. } = r.event {
                assert!(id <= 1, "backend {i} saw an event for buffer {id}");
            }
        }
        assert_eq!(
            rig.rec.count(|e| matches!(e, MseEvent::AppendDone { .. })),
            rig_appends(&low, &aac, 10_000 + i * 500),
            "backend {i}: one AppendDone per append, none from the others"
        );
        worker_names.insert(events[0].thread_name.clone());
    }
    assert_eq!(
        worker_names.len(),
        n,
        "sixteen backends, sixteen workers: {worker_names:?}"
    );
}

fn rig_appends(low: &SegmentedStream, aac: &SegmentedStream, chunk: usize) -> usize {
    use mercury_mse_testkit::chunks;
    let plan = ChunkPlan::Fixed(chunk);
    let count = |s: &SegmentedStream| {
        chunks(&s.init.bytes, &plan).len()
            + s.media
                .iter()
                .map(|m| chunks(&m.bytes, &plan).len())
                .sum::<usize>()
    };
    count(low) + count(aac)
}
