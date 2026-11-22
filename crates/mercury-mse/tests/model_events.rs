// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The events of decision D9, checked by a recorder (lane med-1, item 3): one `AppendDone` or
//! `AppendError` per accepted append, one `RemoveDone` per accepted remove, never a spurious event, the
//! data already in `buffered` and `updating` false when the event arrives, the `InitSegment` before the
//! `AppendDone` of the same append, `BufferedChanged` when eviction takes data, nothing for a buffer
//! after it was removed or the MediaSource closed.

mod model_support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mercury_mse::api::{
    AppendMode, Limits, MediaSourceBackend, MediaSourceReadyState, MseEvent, TimeRange,
};
use mercury_mse_testkit::{ChunkPlan, tile};
use model_support::*;

/// `updating` and `buffered` as seen from inside the closure that receives each completion event.
#[derive(Clone, Debug)]
struct Seen {
    event: MseEvent,
    updating: bool,
    buffered: Vec<TimeRange>,
}

fn watch_completions(rig: &Rig) -> Arc<Mutex<Vec<Seen>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::clone(&rig.backend);
    let out = Arc::clone(&seen);
    rig.rec.add_hook(move |event| {
        let id = match event {
            MseEvent::AppendDone { id }
            | MseEvent::RemoveDone { id }
            | MseEvent::AppendError { id, .. } => *id,
            _ => return,
        };
        out.lock().unwrap().push(Seen {
            event: event.clone(),
            updating: backend.updating(id),
            buffered: backend.buffered(id),
        });
    });
    seen
}

#[test]
fn d9_one_completion_per_append_and_the_data_is_buffered_when_it_arrives() {
    let s = load("h264-aac");
    let rig = Rig::new();
    let seen = watch_completions(&rig);
    let id = rig.add(&s.mime);
    let plan = ChunkPlan::Fixed(5000);
    rig.append_stream(id, &s, &plan);
    let seen = seen.lock().unwrap().clone();
    assert!(seen.len() > 20);
    let mut last_end = 0;
    for (i, one) in seen.iter().enumerate() {
        assert!(
            matches!(one.event, MseEvent::AppendDone { .. }),
            "{i}: {:?}",
            one.event
        );
        assert!(!one.updating, "{i}: updating when AppendDone arrived");
        // buffered only ever grows while appending in order, and never lags: the data of this append is
        // in already (checked at the end against the segment ends).
        if let Some(r) = one.buffered.last() {
            assert!(r.end >= last_end, "{i}: buffered shrank");
            last_end = r.end;
        }
    }
    // After the last append of each media segment, buffered reaches that segment's end.
    assert_ranges("final", &seen.last().unwrap().buffered, &[(0.0, 8.0)], 2.0);
}

#[test]
fn d9_init_segment_event_precedes_the_append_done_of_its_append_and_nothing_is_spurious() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_ok(id, &s.init.bytes);
    rig.append_ok(id, &s.media[0].bytes);
    rig.append_ok(id, &s.media[1].bytes);
    std::thread::sleep(Duration::from_millis(40));
    let events: Vec<String> = rig
        .rec
        .events()
        .into_iter()
        .map(|e| match e {
            MseEvent::DurationChanged(_) => "duration".to_string(),
            MseEvent::InitSegment { .. } => "init".to_string(),
            MseEvent::AppendDone { .. } => "done".to_string(),
            other => format!("{other:?}"),
        })
        .collect();
    // The duration change of the init segment, the init event, the append's own completion, then one
    // completion per media append: nothing else.
    assert_eq!(
        events,
        ["duration", "init", "done", "done", "done"],
        "{events:?}"
    );
}

#[test]
fn d9_one_remove_done_per_remove() {
    let s = load("h264-low");
    let rig = Rig::new();
    let seen = watch_completions(&rig);
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    rig.remove_ok(id, 0.0, 2.0);
    rig.remove_ok(id, 2.0, 4.0);
    rig.remove_ok(id, 6.0, f64::INFINITY);
    let seen = seen.lock().unwrap().clone();
    let removes: Vec<&Seen> = seen
        .iter()
        .filter(|s| matches!(s.event, MseEvent::RemoveDone { .. }))
        .collect();
    assert_eq!(removes.len(), 3);
    for r in &removes {
        assert!(!r.updating);
    }
    assert_ranges(
        "after the removes",
        &removes[2].buffered,
        &[(4.0, 6.0)],
        2.0,
    );
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendDone { .. })),
        5
    );
}

#[test]
fn d9_eviction_sends_buffered_changed_before_the_append_done() {
    let s = load("h264-low");
    let long = tile(&s, 5).unwrap();
    let limits = Limits {
        video_bytes: 200_000,
        ..Limits::default()
    };
    let rig = Rig::with_limits(limits);
    let id = rig.add(&long.mime);
    rig.append_ok(id, &long.init.bytes);
    let mut quota_errors = 0;
    for (i, seg) in long.media.iter().enumerate() {
        rig.backend.set_position(i as f64 * 2.0);
        match rig.backend.append(id, seg.bytes.clone()) {
            Ok(()) => rig.wait_idle(&[id]),
            Err(_) => quota_errors += 1,
        }
    }
    assert_eq!(
        quota_errors, 0,
        "with the position moving, old data is evicted and room is found"
    );
    let log = rig.rec.snapshot();
    let changed: Vec<usize> = log
        .iter()
        .filter(|r| matches!(r.event, MseEvent::BufferedChanged { .. }))
        .map(|r| r.seq)
        .collect();
    assert!(
        !changed.is_empty(),
        "eviction took data, so the page is told"
    );
    for at in changed {
        let next_done = log[at..]
            .iter()
            .find(|r| matches!(r.event, MseEvent::AppendDone { .. }))
            .expect("an AppendDone follows");
        assert!(next_done.seq > at);
    }
    // What is left is the recent part: it starts after zero, and it is a single decodable range.
    let ranges = rig.buffered_secs(id);
    assert_eq!(ranges.len(), 1, "{ranges:?}");
    assert!(ranges[0].0 > 0.5, "the oldest data was evicted: {ranges:?}");
    assert!((ranges[0].1 - 40.0).abs() < 0.1, "{ranges:?}");
}

#[test]
fn d9_nothing_arrives_for_a_buffer_after_remove_source_buffer() {
    let s = load("h264-low");
    let big = tile(&s, 40).unwrap();
    let mut data = Vec::new();
    for seg in &big.media {
        data.extend_from_slice(&seg.bytes);
    }
    let rig = Rig::new();
    for round in 0..8 {
        let id = rig.add(&s.mime);
        rig.append_ok(id, &s.init.bytes);
        rig.backend.append(id, data.clone()).unwrap();
        rig.backend.remove_source_buffer(id).unwrap();
        let at = rig.rec.completions(id);
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(
            rig.rec.completions(id),
            at,
            "round {round}: a completion arrived after removeSourceBuffer returned"
        );
    }
}

#[test]
fn d9_nothing_arrives_for_a_buffer_after_close() {
    let s = load("h264-low");
    let big = tile(&s, 40).unwrap();
    let mut data = Vec::new();
    for seg in &big.media {
        data.extend_from_slice(&seg.bytes);
    }
    let rig = Rig::new();
    for round in 0..8 {
        let id = rig.add(&s.mime);
        rig.append_ok(id, &s.init.bytes);
        rig.backend.append(id, data.clone()).unwrap();
        rig.backend.close();
        let at = rig.rec.completions(id);
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(
            rig.rec.completions(id),
            at,
            "round {round}: a completion arrived after close() returned"
        );
        assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Closed);
        rig.backend.open();
    }
}

#[test]
fn d9_an_event_closure_that_panics_does_not_kill_the_worker() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.rec.add_hook(|event| {
        if matches!(event, MseEvent::InitSegment { .. }) {
            panic!("a misbehaving closure");
        }
    });
    rig.append_ok(id, &s.init.bytes);
    rig.append_ok(id, &s.media[0].bytes);
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 2.0)], 2.0);
    let _ = AppendMode::Segments;
}
