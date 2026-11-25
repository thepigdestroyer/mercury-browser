// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C13 of the GstMSE spike at core level (lane med-1, item 3): abort and errors. Every bad input ends in
//! an `AppendError` with `updating == false` (the append error algorithm: the MediaSource ends with a
//! decode error), or in an `InvalidState` refusal; never in a hang, never in a panic; and the backend
//! stays usable (queries answer, `close()` and `open()` attach it again to a good stream).

mod model_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mercury_mse::api::{
    AppendErrorKind, AppendMode, EosError, MediaSourceBackend, MediaSourceReadyState, MseError,
    MseEvent, SourceBufferId, TrackKind,
};
use mercury_mse_testkit::{ChunkPlan, SegmentedStream, tile};
use model_support::*;

/// What a bad append must leave behind, and that the backend works again after a new attach.
fn assert_defined_error(rig: &Rig, id: SourceBufferId, outcome: &Outcome, what: &str) {
    match outcome {
        Outcome::Error(kind) => assert!(
            matches!(kind, AppendErrorKind::Decode | AppendErrorKind::Unsupported),
            "{what}: error kind {kind:?}"
        ),
        other => panic!("{what}: expected an AppendError, got {other:?}"),
    }
    assert!(
        !rig.backend.updating(id),
        "{what}: updating after the error"
    );
    assert_eq!(
        rig.backend.ready_state(),
        MediaSourceReadyState::Ended,
        "{what}: the append error algorithm ends the stream"
    );
    assert!(
        rig.rec.wait_for(WAIT, |log| log.iter().any(|r| r.event
            == MseEvent::SourceEnded {
                error: Some(EosError::Decode)
            })),
        "{what}: SourceEnded with the decode error"
    );
    // Queries still answer, the element has an error so appends are refused, nothing hangs.
    let _ = rig.backend.buffered(id);
    let _ = rig.backend.element_buffered();
    let _ = rig.backend.seekable();
    assert!(matches!(
        rig.backend.append(id, vec![0; 16]),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.end_of_stream(None),
        Err(MseError::InvalidState(_))
    ));
    // A new attach starts clean.
    rig.backend.close();
    rig.backend.open();
    let good = load("h264-low");
    let fresh = rig.add(&good.mime);
    rig.append_stream(fresh, &good, &ChunkPlan::Whole);
    assert_ranges(
        &format!("{what}: a new attach plays"),
        &rig.backend.buffered(fresh),
        &[(0.0, 8.0)],
        2.0,
    );
    let d = drain(&*rig.backend.frame_feed(), TrackKind::Video);
    assert_eq!(
        d.frames.len(),
        200,
        "{what}: the new attach serves its frames"
    );
}

#[test]
fn c13_garbage_as_the_first_append() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    let noise = Rng::new(0x00C1_3C13).bytes(4096);
    let outcome = rig.append(id, &noise);
    assert_defined_error(&rig, id, &outcome, "garbage");
}

#[test]
fn c13_a_megabyte_of_garbage_does_not_hang() {
    let rig = Rig::new();
    let good = load("vp9-low");
    let id = rig.add(&good.mime);
    let noise = Rng::new(7).bytes(1 << 20);
    let started = std::time::Instant::now();
    let outcome = rig.append(id, &noise);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_defined_error(&rig, id, &outcome, "1 MB of garbage");
}

#[test]
fn c13_a_media_segment_before_any_init_segment() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    let outcome = rig.append(id, &good.media[0].bytes);
    assert_defined_error(&rig, id, &outcome, "media before init");
}

#[test]
fn c13_webm_into_an_mp4_buffer_and_back() {
    let (mp4, webm) = (load("h264-low"), load("vp9-low"));
    let rig = Rig::new();
    let id = rig.add(&mp4.mime);
    let outcome = rig.append(id, &webm.init.bytes);
    assert_defined_error(&rig, id, &outcome, "webm into mp4");

    let rig = Rig::new();
    let id = rig.add(&webm.mime);
    let outcome = rig.append(id, &mp4.init.bytes);
    assert_defined_error(&rig, id, &outcome, "mp4 into webm");
}

#[test]
fn c13_an_init_segment_with_the_wrong_kind_of_track() {
    // A video init segment in a buffer that was declared audio, and the other way round.
    let (video, audio) = (load("h264-low"), load("aac"));
    let rig = Rig::new();
    let id = rig.add(&audio.mime);
    let outcome = rig.append(id, &video.init.bytes);
    assert_defined_error(&rig, id, &outcome, "video into an audio buffer");

    let rig = Rig::new();
    let id = rig.add(&video.mime);
    let outcome = rig.append(id, &audio.init.bytes);
    assert_defined_error(&rig, id, &outcome, "audio into a video buffer");

    // A muxed init segment in a video-only buffer.
    let muxed = load("h264-aac");
    let rig = Rig::new();
    let id = rig.add(&video.mime);
    let outcome = rig.append(id, &muxed.init.bytes);
    assert_defined_error(&rig, id, &outcome, "muxed into a video buffer");
}

#[test]
fn c13_garbage_after_good_data_keeps_what_was_buffered() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    rig.append_ok(id, &good.init.bytes);
    rig.append_ok(id, &good.media[0].bytes);
    let outcome = rig.append(id, &Rng::new(3).bytes(2048));
    match outcome {
        Outcome::Error(_) | Outcome::Done => {}
        other => panic!("{other:?}"),
    }
    // Whatever the parser made of the noise, what was buffered before stays.
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 2.0)], 2.0);
}

#[test]
fn c13_a_truncated_segment_completed_later_works() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    rig.append_ok(id, &good.init.bytes);
    let segment = &good.media[0].bytes;
    let cut = segment.len() / 2 + 3; // inside a sample, not on a box edge
    rig.append_ok(id, &segment[..cut]);
    rig.append_ok(id, &segment[cut..]);
    for s in &good.media[1..] {
        rig.append_ok(id, &s.bytes);
    }
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 8.0)], 2.0);
    rig.backend.end_of_stream(None).unwrap();
    let d = drain(&*rig.backend.frame_feed(), TrackKind::Video);
    assert_eq!(d.frames.len(), 200);
}

#[test]
fn c13_half_a_segment_then_abort_then_a_clean_reappend() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    rig.append_ok(id, &good.init.bytes);
    let segment = &good.media[0].bytes;
    rig.append_ok(id, &segment[..segment.len() / 2 + 3]);
    // The partial segment is dropped by abort(); a whole segment appended next is parsed from its start.
    rig.backend.abort(id).unwrap();
    rig.append_ok(id, segment);
    for s in &good.media[1..] {
        rig.append_ok(id, &s.bytes);
    }
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 8.0)], 2.0);
    rig.backend.end_of_stream(None).unwrap();
    let d = drain(&*rig.backend.frame_feed(), TrackKind::Video);
    assert_eq!(
        d.frames.len(),
        200,
        "the aborted partial segment left no duplicate frames"
    );
}

#[test]
fn c13_the_same_init_segment_twice() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    rig.append_ok(id, &good.init.bytes);
    rig.append_ok(id, &good.init.bytes);
    for s in &good.media {
        rig.append_ok(id, &s.bytes);
    }
    rig.append_ok(id, &good.init.bytes); // a switch to the same rendition, in the middle
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 8.0)], 2.0);
    rig.backend.end_of_stream(None).unwrap();
    let d = drain(&*rig.backend.frame_feed(), TrackKind::Video);
    assert_eq!(d.frames.len(), 200);
    assert_eq!(d.configs.len(), 1, "the same configuration is not a switch");
    // The second and third init segment events are not "first".
    let firsts: Vec<bool> = rig
        .rec
        .events()
        .into_iter()
        .filter_map(|e| match e {
            MseEvent::InitSegment { init, .. } => Some(init.first),
            _ => None,
        })
        .collect();
    assert_eq!(firsts, vec![true, false, false]);
}

#[test]
fn c13_append_to_a_removed_buffer_is_invalid_state() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    rig.append_ok(id, &good.init.bytes);
    rig.backend.remove_source_buffer(id).unwrap();
    assert!(matches!(
        rig.backend.append(id, vec![1]),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.remove(id, 0.0, 1.0),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.abort(id),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.set_mode(id, AppendMode::Sequence),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.set_timestamp_offset(id, 1.0),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.set_append_window_start(id, 1.0),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.set_append_window_end(id, 9.0),
        Err(MseError::InvalidState(_))
    ));
    assert!(rig.backend.buffered(id).is_empty());
    assert!(!rig.backend.updating(id));
    // Removing it again, or removing a buffer that never existed: not found.
    assert!(matches!(
        rig.backend.remove_source_buffer(id),
        Err(MseError::NotFound(_))
    ));
    assert!(matches!(
        rig.backend.remove_source_buffer(77),
        Err(MseError::NotFound(_))
    ));
    assert!(matches!(
        rig.backend.append(77, vec![1]),
        Err(MseError::NotFound(_))
    ));
    assert_eq!(rig.backend.source_buffer_count(), 0);
    // The attributes still read.
    assert_eq!(rig.backend.mode(id).unwrap(), AppendMode::Segments);
    // Ids are not reused.
    let again = rig.add(&good.mime);
    assert_ne!(again, id);
}

#[test]
fn c13_append_while_updating_is_invalid_state() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    let (parked_tx, parked_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let go_rx = Arc::new(Mutex::new(go_rx));
    rig.rec.add_hook(move |e| {
        if matches!(e, MseEvent::InitSegment { .. }) {
            parked_tx.send(()).ok();
            go_rx.lock().unwrap().recv().ok();
        }
    });
    rig.backend.append(id, good.init.bytes.clone()).unwrap();
    parked_rx
        .recv_timeout(WAIT)
        .expect("parked in the InitSegment closure");
    assert!(rig.backend.updating(id));
    let second = rig.backend.append(id, good.media[0].bytes.clone());
    assert!(
        matches!(second, Err(MseError::InvalidState(_))),
        "{second:?}"
    );
    go_tx.send(()).unwrap();
    rig.wait_idle(&[id]);
    // The refused append left no trace: exactly one AppendDone so far, and the buffer works.
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendDone { .. })),
        1
    );
    rig.append_ok(id, &good.media[0].bytes);
}

#[test]
fn c13_abort_works_when_nothing_is_updating_and_resets_the_window() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    rig.backend.set_append_window_start(id, 1.0).unwrap();
    rig.backend.set_append_window_end(id, 5.0).unwrap();
    rig.backend.abort(id).unwrap();
    assert_eq!(rig.backend.append_window_start(id).unwrap(), 0.0);
    assert_eq!(rig.backend.append_window_end(id).unwrap(), f64::INFINITY);
    rig.backend.end_of_stream(None).unwrap();
    // In the ended state abort() is invalid (the MediaSource is not open).
    assert!(matches!(
        rig.backend.abort(id),
        Err(MseError::InvalidState(_))
    ));
}

#[test]
fn c13_abort_during_a_large_append_leaves_no_event_and_a_usable_buffer() {
    let good = load("h264-low");
    let big: SegmentedStream = tile(&good, 40).unwrap();
    let mut data = Vec::new();
    for s in &big.media {
        data.extend_from_slice(&s.bytes);
    }
    assert!(data.len() > 1_500_000, "{} bytes", data.len());
    let rig = Rig::new();
    let id = rig.add(&good.mime);
    rig.append_ok(id, &good.init.bytes);

    let mut aborted_in_flight = 0;
    for round in 0..12 {
        let before = rig.rec.completions(id);
        rig.backend.append(id, data.clone()).unwrap();
        rig.backend.abort(id).unwrap();
        assert!(
            !rig.backend.updating(id),
            "round {round}: updating after abort"
        );
        let at_abort = rig.rec.completions(id);
        if at_abort == before {
            aborted_in_flight += 1;
        }
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            rig.rec.completions(id),
            at_abort,
            "round {round}: a completion event arrived after abort() returned"
        );
        assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Open);
        // The buffer accepts data again, and the next append completes normally.
        rig.append_ok(id, &good.media[round % 4].bytes);
    }
    eprintln!("c13 abort-large: {aborted_in_flight} of 12 aborts hit an append in flight");
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendError { .. })),
        0,
        "an abort is not an error"
    );
}

#[test]
fn c13_remove_cannot_be_aborted_and_checks_its_arguments() {
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    // Duration is NaN before the first init segment.
    assert!(matches!(
        rig.backend.remove(id, 0.0, 1.0),
        Err(MseError::Type(_))
    ));
    rig.append_ok(id, &good.init.bytes);
    rig.backend.set_duration(Some(8.0)).unwrap();
    assert!(matches!(
        rig.backend.remove(id, -1.0, 1.0),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.remove(id, 9.0, 10.0),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.remove(id, 2.0, 2.0),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.remove(id, 3.0, 2.0),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.remove(id, f64::NAN, 2.0),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.remove(id, 0.0, f64::NAN),
        Err(MseError::Type(_))
    ));
    rig.append_ok(id, &good.media[0].bytes);
    rig.remove_ok(id, 0.0, f64::INFINITY);
    assert!(rig.backend.buffered(id).is_empty());

    // abort() while a removal runs: park the worker in the `RemoveDone`-less phase is not possible (the
    // removal is one step), so this checks the other direction: remove in the ended state reopens.
    rig.append_ok(id, &good.media[0].bytes);
    rig.backend.end_of_stream(None).unwrap();
    rig.remove_ok(id, 0.0, 1.0);
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Open);
}

#[test]
fn c13_the_buffer_full_flag_and_quota_exceeded() {
    use mercury_mse::api::Limits;
    let good = load("h264-low");
    // A quota too small for the stream: the second segment finds the buffer full (nothing is old
    // enough to evict with the position at 0) and appendBuffer throws QuotaExceededError.
    let limits = Limits {
        video_bytes: 40_000,
        audio_bytes: 40_000,
        text_bytes: 1_000,
        ..Limits::default()
    };
    let rig = Rig::with_limits(limits);
    let id = rig.add(&good.mime);
    rig.append_ok(id, &good.init.bytes);
    let mut refused = None;
    for (i, s) in good.media.iter().enumerate() {
        match rig.backend.append(id, s.bytes.clone()) {
            Ok(()) => {
                rig.wait_idle(&[id]);
            }
            Err(e) => {
                refused = Some((i, e));
                break;
            }
        }
    }
    let (_, error) = refused.expect("a 40 kB quota cannot hold 8 s of this stream");
    assert!(matches!(error, MseError::QuotaExceeded(_)), "{error:?}");
    assert!(
        !rig.backend.updating(id),
        "a refused append is not updating"
    );
    // The page frees room with remove() and tries again.
    rig.backend.set_position(0.0);
    rig.remove_ok(id, 0.0, 2.0);
    let again = rig.backend.append(id, good.media[1].bytes.clone());
    assert!(
        again.is_ok() || matches!(again, Err(MseError::QuotaExceeded(_))),
        "{again:?}"
    );
}

#[test]
fn c13_every_error_is_reported_once() {
    // An error never produces a completion event of its own for a later append, and the flag stays
    // until the next attach.
    let stop = Arc::new(AtomicBool::new(false));
    let rig = Rig::new();
    let good = load("h264-low");
    let id = rig.add(&good.mime);
    assert_eq!(
        rig.append(id, &[0u8; 64]),
        Outcome::Error(AppendErrorKind::Decode)
    );
    for _ in 0..5 {
        assert!(matches!(
            rig.backend.append(id, good.init.bytes.clone()),
            Err(MseError::InvalidState(_))
        ));
    }
    stop.store(true, Ordering::SeqCst);
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendError { .. })),
        1
    );
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::SourceEnded { .. })),
        1
    );
}
