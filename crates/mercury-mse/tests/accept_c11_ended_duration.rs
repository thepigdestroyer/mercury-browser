// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C11 of the GstMSE spike at core level (lane med-1, item 3): the ready state of the MediaSource
//! (closed, open, ended, open again), `endOfStream` with and without an error, an append after the end,
//! and the duration rules (NaN before any initialization segment, the spec's `InvalidStateError` below
//! the highest presentation timestamp, larger and live durations, the rounding to the highest end time).

mod model_support;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use mercury_mse::api::{
    ElementReadyState, EosError, FeedState, MediaSourceBackend, MediaSourceReadyState, MseError,
    MseEvent, TrackKind,
};
use mercury_mse_testkit::ChunkPlan;
use model_support::*;

/// The sequence of MediaSource-level events (the lifecycle the DOM fires events for).
fn lifecycle(rig: &Rig) -> Vec<String> {
    rig.rec
        .events()
        .into_iter()
        .filter_map(|e| match e {
            MseEvent::SourceEnded { error } => Some(format!("ended:{}", eos_name(error))),
            MseEvent::DurationChanged(d) => Some(format!("duration:{d:?}")),
            _ => None,
        })
        .collect()
}

#[test]
fn c11_ready_state_closed_open_ended_open_closed() {
    let s = load("h264-low");
    let rig = Rig::closed(Default::default());
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Closed);
    // A closed MediaSource takes no source buffer.
    assert!(matches!(
        rig.backend.add_source_buffer(&s.mime),
        Err(MseError::InvalidState(_))
    ));
    rig.backend.open();
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Open);
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &ChunkPlan::Whole);

    rig.backend.end_of_stream(None).unwrap();
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Ended);
    assert!(rig.rec.wait_for(WAIT, |log| {
        log.iter()
            .any(|r| matches!(r.event, MseEvent::SourceEnded { error: None }))
    }));
    // Ended: end_of_stream again, set_duration and add_source_buffer are invalid.
    assert!(matches!(
        rig.backend.end_of_stream(None),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.set_duration(Some(9.0)),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.add_source_buffer(&s.mime),
        Err(MseError::InvalidState(_))
    ));

    // An append in the ended state reopens the MediaSource and completes.
    let tile = mercury_mse_testkit::tile(&s, 2).unwrap();
    rig.append_ok(id, &tile.media[4].bytes);
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Open);
    rig.backend.end_of_stream(None).unwrap();
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Ended);
    // The appended segment lies at 8 s to 10 s: the duration grew with it and the stream plays to 10 s.
    let d = rig.backend.duration().unwrap();
    assert!((d - 10.0).abs() < 0.001, "duration {d}");

    // Detaching closes it and drops every source buffer.
    rig.backend.close();
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Closed);
    assert_eq!(rig.backend.source_buffer_count(), 0);
    assert_eq!(rig.backend.duration(), None);
    assert!(rig.backend.buffered(id).is_empty());
    assert!(matches!(
        rig.backend.append(id, vec![1]),
        Err(MseError::InvalidState(_))
    ));
}

#[test]
fn c11_every_transition_into_ended_is_one_source_ended_event() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    rig.backend.end_of_stream(None).unwrap();
    rig.append_ok(id, &s.media[3].bytes); // reopens
    rig.backend.end_of_stream(None).unwrap();
    assert!(rig.rec.wait_for(WAIT, |log| {
        log.iter()
            .filter(|r| matches!(r.event, MseEvent::SourceEnded { .. }))
            .count()
            == 2
    }));
    // Nothing else arrives for the ended state.
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::SourceEnded { .. })),
        2
    );
}

#[test]
fn c11_end_of_stream_with_network_and_decode_errors() {
    let s = load("h264-low");
    for (error, label) in [(EosError::Network, "network"), (EosError::Decode, "decode")] {
        let rig = Rig::new();
        let id = rig.add(&s.mime);
        rig.append_ok(id, &s.init.bytes);
        rig.append_ok(id, &s.media[0].bytes);
        rig.backend.end_of_stream(Some(error)).unwrap();
        assert_eq!(
            rig.backend.ready_state(),
            MediaSourceReadyState::Ended,
            "{label}"
        );
        assert!(
            rig.rec.wait_for(WAIT, |log| log
                .iter()
                .any(|r| r.event == MseEvent::SourceEnded { error: Some(error) })),
            "{label}: SourceEnded"
        );
        // With an error the duration is left alone (no highest-end rounding).
        assert!(
            !lifecycle(&rig)
                .iter()
                .any(|e| e.starts_with("duration:") && e.contains("2.0")),
            "{label}: {:?}",
            lifecycle(&rig)
        );
        // The media element has an error now: appends are invalid until the next attach.
        assert!(matches!(
            rig.backend.append(id, s.media[1].bytes.clone()),
            Err(MseError::InvalidState(_))
        ));
        rig.backend.close();
        rig.backend.open();
        let id = rig.add(&s.mime);
        rig.append_stream(id, &s, &ChunkPlan::Whole);
        assert_ranges(
            "after the new attach",
            &rig.backend.buffered(id),
            &[(0.0, 8.0)],
            2.0,
        );
    }
}

#[test]
fn c11_append_after_ended_plays_to_the_end() {
    // Segments 0-1, end of stream, then segments 2-3 in the ended state, then the end again: all 200
    // frames are served.
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_ok(id, &s.init.bytes);
    rig.append_ok(id, &s.media[0].bytes);
    rig.append_ok(id, &s.media[1].bytes);
    rig.backend.end_of_stream(None).unwrap();
    let feed = rig.backend.frame_feed();
    let first = drain(&*feed, TrackKind::Video);
    assert_eq!(first.frames.len(), 100);
    assert_eq!(first.state, FeedState::Ended);
    rig.append_ok(id, &s.media[2].bytes);
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Open);
    // The feed is no longer at an end, and the new frames follow without a restart.
    let more = drain(&*feed, TrackKind::Video);
    assert_eq!(more.frames.len(), 50);
    assert_eq!(more.state, FeedState::WaitingForData);
    rig.append_ok(id, &s.media[3].bytes);
    rig.backend.end_of_stream(None).unwrap();
    let rest = drain(&*feed, TrackKind::Video);
    assert_eq!(rest.frames.len(), 50);
    assert_eq!(rest.state, FeedState::Ended);
    assert!(rest.discont.iter().all(|d| !d));
}

#[test]
fn c11_duration_is_nan_before_the_first_initialization_segment() {
    let s = load("h264-low");
    let rig = Rig::new();
    assert_eq!(
        rig.backend.duration(),
        None,
        "a new MediaSource has no duration"
    );
    let id = rig.add(&s.mime);
    assert_eq!(
        rig.backend.duration(),
        None,
        "nor does it get one from addSourceBuffer"
    );
    assert_eq!(rig.backend.frame_feed().duration(), None);
    rig.append_ok(id, &s.init.bytes);
    // After the init segment: what the segment declares, or +Infinity when it declares nothing.
    let d = rig
        .backend
        .duration()
        .expect("a duration after the init segment");
    // What the init segment declares (8 s), or +Infinity when it declares none.
    assert!(d == f64::INFINITY || (d - 8.0).abs() < 0.03, "duration {d}");
    eprintln!("c11: the duration after the h264-low init segment is {d}");
    let events = lifecycle(&rig);
    assert_eq!(
        events.len(),
        1,
        "one DurationChanged for the init segment: {events:?}"
    );
    assert_eq!(events[0], format!("duration:{:?}", Some(d)));
    // The init segment event comes after the duration change (the element's durationchange precedes
    // loadedmetadata).
    let all = rig.rec.events();
    let dur_at = all
        .iter()
        .position(|e| matches!(e, MseEvent::DurationChanged(_)))
        .unwrap();
    let init_at = all
        .iter()
        .position(|e| matches!(e, MseEvent::InitSegment { .. }))
        .unwrap();
    assert!(dur_at < init_at);
}

#[test]
fn c11_duration_is_8_seconds_after_the_end_of_the_stream() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    rig.backend.end_of_stream(None).unwrap();
    let d = rig.backend.duration().unwrap();
    assert!((d - 8.0).abs() < 0.001, "duration {d}");
    assert!(rig.rec.wait_for(WAIT, |log| {
        log.iter()
            .any(|r| matches!(r.event, MseEvent::SourceEnded { .. }))
    }),);
    // The feed hands the duration to the pipeline as nanoseconds.
    let ns = rig.backend.frame_feed().duration().unwrap();
    assert!((ns - 8_000_000_000).abs() < 1_000_000);
}

#[test]
fn c11_set_duration_rules() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);

    // Before any data: larger, smaller and +Infinity all work; NaN and negative are TypeErrors.
    rig.backend.set_duration(Some(10.0)).unwrap();
    assert_eq!(rig.backend.duration(), Some(10.0));
    rig.backend.set_duration(Some(10.0)).unwrap(); // the same value: no event
    rig.backend.set_duration(Some(f64::INFINITY)).unwrap();
    assert_eq!(rig.backend.duration(), Some(f64::INFINITY));
    assert!(matches!(
        rig.backend.set_duration(None),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.set_duration(Some(f64::NAN)),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.set_duration(Some(-1.0)),
        Err(MseError::Type(_))
    ));
    assert_eq!(rig.backend.duration(), Some(f64::INFINITY));

    rig.append_stream(id, &s, &ChunkPlan::Whole);
    // Below the highest presentation timestamp: InvalidState, and nothing changed.
    assert!(matches!(
        rig.backend.set_duration(Some(5.0)),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.set_duration(Some(7.0)),
        Err(MseError::InvalidState(_))
    ));
    assert_eq!(rig.backend.duration(), Some(f64::INFINITY));
    // Between the last frame's timestamp (7.96 s) and its end (8.0 s): rounded up to the end.
    rig.backend.set_duration(Some(7.99)).unwrap();
    assert_eq!(rig.backend.duration(), Some(8.0));
    // After removing the excess, a smaller duration is accepted.
    rig.remove_ok(id, 5.0, f64::INFINITY);
    let end = rig.buffered_secs(id)[0].1;
    assert!((4.9..=5.001).contains(&end), "buffered end {end}");
    rig.backend.set_duration(Some(5.0)).unwrap();
    assert_eq!(rig.backend.duration(), Some(5.0));
    // Larger, and then live.
    rig.backend.set_duration(Some(20.0)).unwrap();
    assert_eq!(rig.backend.duration(), Some(20.0));
    rig.backend.set_duration(Some(f64::INFINITY)).unwrap();

    // One DurationChanged per real change, none for the same value or a refusal.
    let durations: Vec<Option<f64>> = rig
        .rec
        .events()
        .iter()
        .filter_map(|e| match e {
            MseEvent::DurationChanged(d) => Some(*d),
            _ => None,
        })
        .collect();
    assert!(rig.rec.wait_for(WAIT, |log| {
        log.iter()
            .filter(|r| matches!(r.event, MseEvent::DurationChanged(_)))
            .count()
            >= 6
    }));
    let durations2: Vec<Option<f64>> = rig
        .rec
        .events()
        .iter()
        .filter_map(|e| match e {
            MseEvent::DurationChanged(d) => Some(*d),
            _ => None,
        })
        .collect();
    assert!(durations2.len() >= durations.len());
    for pair in durations2.windows(2) {
        assert_ne!(
            pair[0], pair[1],
            "two identical DurationChanged in a row: {durations2:?}"
        );
    }
    assert_eq!(
        durations2,
        vec![
            Some(10.0),
            Some(f64::INFINITY),
            Some(8.0),
            Some(5.0),
            Some(20.0),
            Some(f64::INFINITY)
        ]
    );
}

#[test]
fn c11_the_duration_grows_with_the_media_when_the_init_segment_gave_a_finite_one() {
    // A source whose duration was set to 3 s and then gets 8 s of media: the media extends it.
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.backend.set_duration(Some(3.0)).unwrap();
    rig.append_ok(id, &s.init.bytes);
    assert_eq!(
        rig.backend.duration(),
        Some(3.0),
        "an init segment does not override a duration"
    );
    for seg in &s.media {
        rig.append_ok(id, &seg.bytes);
    }
    let d = rig.backend.duration().unwrap();
    assert!((d - 8.0).abs() < 0.001, "duration {d}");
}

#[test]
fn c11_set_duration_remove_and_end_of_stream_while_updating_are_invalid_state() {
    // Park the worker inside the InitSegment closure, so that the append is updating while the other
    // calls are made (calling abort() there would wait for the parked closure: not done here).
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    let (parked_tx, parked_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let go_rx = Arc::new(Mutex::new(go_rx));
    rig.rec.add_hook(move |e| {
        if matches!(e, MseEvent::InitSegment { .. }) {
            parked_tx.send(()).ok();
            go_rx.lock().unwrap().recv().ok();
        }
    });
    rig.backend.append(id, s.init.bytes.clone()).unwrap();
    parked_rx.recv_timeout(WAIT).expect("parked");
    assert!(rig.backend.updating(id));
    assert!(matches!(
        rig.backend.set_duration(Some(9.0)),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.end_of_stream(None),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.remove(id, 0.0, 1.0),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.append(id, s.media[0].bytes.clone()),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.set_timestamp_offset(id, 1.0),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.set_append_window_end(id, 5.0),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend
            .set_mode(id, mercury_mse::api::AppendMode::Sequence),
        Err(MseError::InvalidState(_))
    ));
    go_tx.send(()).unwrap();
    rig.wait_idle(&[id]);
    rig.backend.end_of_stream(None).unwrap();
}

#[test]
fn c11_seekable_and_live_seekable_range() {
    let s = load("h264-low");
    let rig = Rig::new();
    assert!(rig.backend.seekable().is_empty(), "NaN duration: empty");
    let id = rig.add(&s.mime);
    rig.append_ok(id, &s.init.bytes);
    rig.append_ok(id, &s.media[0].bytes);
    // The init segment gave +Infinity or a finite duration; make both cases explicit.
    rig.backend.set_duration(Some(8.0)).unwrap();
    assert_ranges(
        "finite duration",
        &rig.backend.seekable(),
        &[(0.0, 8.0)],
        0.5,
    );
    rig.backend.set_duration(Some(f64::INFINITY)).unwrap();
    // +Infinity without a live range: 0 to the end of what is buffered.
    assert_ranges(
        "infinite, no live range",
        &rig.backend.seekable(),
        &[(0.0, 2.0)],
        2.0,
    );
    // With a live range: the union of it and what is buffered.
    rig.backend.set_live_seekable_range(1.0, 30.0).unwrap();
    assert_ranges(
        "infinite, live range",
        &rig.backend.seekable(),
        &[(0.0, 30.0)],
        1.0,
    );
    rig.backend.set_live_seekable_range(5.0, 30.0).unwrap();
    assert_ranges(
        "live range after the data",
        &rig.backend.seekable(),
        &[(0.0, 30.0)],
        1.0,
    );
    assert!(matches!(
        rig.backend.set_live_seekable_range(-1.0, 3.0),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.set_live_seekable_range(4.0, 3.0),
        Err(MseError::Type(_))
    ));
    rig.backend.clear_live_seekable_range().unwrap();
    assert_ranges("cleared", &rig.backend.seekable(), &[(0.0, 2.0)], 2.0);
    rig.backend.end_of_stream(None).unwrap();
    assert!(matches!(
        rig.backend.set_live_seekable_range(0.0, 3.0),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.clear_live_seekable_range(),
        Err(MseError::InvalidState(_))
    ));
}

#[test]
fn c11_element_ready_state_follows_the_buffered_ranges() {
    let (v, a) = (load("h264-low"), load("aac"));
    let rig = Rig::new();
    assert_eq!(
        rig.backend.element_ready_state(0.0),
        ElementReadyState::HaveNothing
    );
    let (vid, aid) = (rig.add(&v.mime), rig.add(&a.mime));
    assert_eq!(
        rig.backend.element_ready_state(0.0),
        ElementReadyState::HaveNothing
    );
    rig.append_ok(vid, &v.init.bytes);
    // One buffer still lacks its init segment: still nothing.
    assert_eq!(
        rig.backend.element_ready_state(0.0),
        ElementReadyState::HaveNothing
    );
    rig.append_ok(aid, &a.init.bytes);
    assert_eq!(
        rig.backend.element_ready_state(0.0),
        ElementReadyState::HaveMetadata
    );
    // Data in one buffer only: the intersection is empty.
    rig.append_ok(vid, &v.media[0].bytes);
    assert_eq!(
        rig.backend.element_ready_state(0.0),
        ElementReadyState::HaveMetadata
    );
    rig.append_ok(aid, &a.media[0].bytes);
    // 2 s ahead: future data (less than 3 s).
    assert_eq!(
        rig.backend.element_ready_state(0.0),
        ElementReadyState::HaveFutureData
    );
    // Within the last 100 ms of the range: current data only.
    assert_eq!(
        rig.backend.element_ready_state(1.95),
        ElementReadyState::HaveCurrentData
    );
    // Outside every range: metadata.
    assert_eq!(
        rig.backend.element_ready_state(3.0),
        ElementReadyState::HaveMetadata
    );
    // 4 s of data: enough at the start.
    rig.append_ok(vid, &v.media[1].bytes);
    rig.append_ok(aid, &a.media[1].bytes);
    assert_eq!(
        rig.backend.element_ready_state(0.0),
        ElementReadyState::HaveEnoughData
    );
    assert_eq!(
        rig.backend.element_ready_state(2.0),
        ElementReadyState::HaveFutureData
    );
    // The end of the stream: enough wherever the position is inside the data, up to its very end.
    rig.append_ok(vid, &v.media[2].bytes);
    rig.append_ok(aid, &a.media[2].bytes);
    rig.append_ok(vid, &v.media[3].bytes);
    rig.append_ok(aid, &a.media[3].bytes);
    rig.backend.end_of_stream(None).unwrap();
    assert_eq!(
        rig.backend.element_ready_state(7.99),
        ElementReadyState::HaveEnoughData
    );
    assert_eq!(
        rig.backend.element_ready_state(8.0),
        ElementReadyState::HaveEnoughData
    );
    assert_eq!(
        rig.backend.element_ready_state(9.0),
        ElementReadyState::HaveMetadata
    );
    // Removing the media at the position stalls it: metadata again.
    rig.backend.seeking(4.0);
    rig.remove_ok(vid, 3.0, 5.0);
    assert_eq!(
        rig.backend.element_ready_state(4.0),
        ElementReadyState::HaveMetadata
    );
}

#[test]
fn c11_the_seeking_algorithm_reopens_an_ended_stream() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    assert!(!rig.backend.seeking(3.0), "open: nothing to reopen");
    rig.backend.end_of_stream(None).unwrap();
    assert!(
        rig.backend.seeking(1.0),
        "ended: reopened, the DOM fires sourceopen"
    );
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Open);
    assert!(!rig.backend.seeking(2.0));
}
