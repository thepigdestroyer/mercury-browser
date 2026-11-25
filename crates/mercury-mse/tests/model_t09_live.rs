// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T09 of the GstMSE spike at core level (lane med-1, item 3): a live stream. The duration is
//! +Infinity, the live seekable range slides, old data is removed behind the window, the source buffer
//! is in `Sequence` mode so that segments follow each other whatever their timestamps say, and the
//! stream can end.

mod model_support;

use mercury_mse::api::{AppendMode, MediaSourceBackend, MseError, MseEvent, TrackKind};
use model_support::*;

#[test]
fn t09_live_stream_with_a_sliding_window_in_sequence_mode() {
    let s = load("h264-low");
    let rig = Rig::new();
    assert_eq!(rig.backend.duration(), None, "NaN before anything is known");
    rig.backend.set_duration(Some(f64::INFINITY)).unwrap();
    let id = rig.add(&s.mime);
    rig.backend.set_mode(id, AppendMode::Sequence).unwrap();
    assert_eq!(rig.backend.mode(id).unwrap(), AppendMode::Sequence);
    rig.append_ok(id, &s.init.bytes);
    assert_eq!(
        rig.backend.duration(),
        Some(f64::INFINITY),
        "an init segment does not change a duration that is not NaN"
    );

    let mut removed_to = 0.0f64;
    let mut widest = 0.0f64;
    for k in 0..30usize {
        // Segments of the fixture repeat in turn: in sequence mode each is placed right after the
        // previous one, so the timeline is continuous.
        rig.append_ok(id, &s.media[k % 4].bytes);
        let end = 2.0 * (k as f64 + 1.0);
        let ranges = rig.buffered_secs(id);
        assert_eq!(ranges.len(), 1, "after segment {k}: {ranges:?}");
        assert!(
            (ranges[0].1 - end).abs() < 0.002,
            "after segment {k}: {ranges:?}"
        );
        // The live window: the last 20 s are seekable; everything older than 30 s is removed.
        rig.backend
            .set_live_seekable_range((end - 20.0).max(0.0), end)
            .unwrap();
        if end > 30.0 {
            let cut = end - 30.0;
            rig.remove_ok(id, removed_to, cut);
            removed_to = cut;
        }
        let ranges = rig.buffered_secs(id);
        widest = widest.max(ranges[0].1 - ranges[0].0);
        assert!(
            ranges[0].0 >= removed_to - 2.1,
            "start {} after removing to {removed_to}",
            ranges[0].0
        );
        // seekable: the union of the live range and what is buffered, +Infinity duration.
        let seekable = secs(&rig.backend.seekable());
        assert_eq!(seekable.len(), 1);
        let want_start = ((end - 20.0).max(0.0)).min(ranges[0].0);
        assert!(
            (seekable[0].0 - want_start).abs() < 0.01,
            "seekable {seekable:?} buffered {ranges:?}"
        );
        assert!((seekable[0].1 - end).abs() < 0.01);
    }
    assert!(widest <= 32.1, "the window never held more than {widest} s");
    assert_eq!(rig.backend.duration(), Some(f64::INFINITY));
    // One DurationChanged only (the explicit set_duration), however long the stream.
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::DurationChanged(_))),
        1
    );

    // The feed serves the window and nothing before it.
    let feed = rig.backend.frame_feed();
    feed.seek(40_000_000_000);
    let d = drain(&*feed, TrackKind::Video);
    assert!(d.frames[0].keyframe);
    assert!(
        d.frames[0].pts <= 40_000_000_000 && d.frames[0].pts >= 38_000_000_000,
        "{}",
        d.frames[0].pts
    );
    assert_in_order("live", &d.frames);

    // The live stream ends: the duration becomes the end of the data.
    rig.backend.end_of_stream(None).unwrap();
    let dur = rig.backend.duration().unwrap();
    assert!((dur - 60.0).abs() < 0.01, "duration {dur}");
    let seekable = secs(&rig.backend.seekable());
    assert_eq!(seekable.len(), 1);
    assert!(
        (seekable[0].0).abs() < 0.001 && (seekable[0].1 - 60.0).abs() < 0.01,
        "{seekable:?}"
    );
}

#[test]
fn t09_live_duration_stays_infinite_and_set_duration_obeys_the_rules() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_ok(id, &s.init.bytes);
    rig.backend.set_duration(Some(f64::INFINITY)).unwrap();
    rig.backend.set_mode(id, AppendMode::Sequence).unwrap();
    for segment in s.media.iter().take(4) {
        rig.append_ok(id, &segment.bytes);
    }
    assert_eq!(rig.backend.duration(), Some(f64::INFINITY));
    // A finite duration below the data is refused, above it is accepted, +Infinity again is accepted.
    assert!(matches!(
        rig.backend.set_duration(Some(3.0)),
        Err(MseError::InvalidState(_))
    ));
    rig.backend.set_duration(Some(100.0)).unwrap();
    rig.backend.set_duration(Some(f64::INFINITY)).unwrap();
    // The live range needs 0 <= start <= end.
    assert!(matches!(
        rig.backend.set_live_seekable_range(5.0, 1.0),
        Err(MseError::Type(_))
    ));
    rig.backend.set_live_seekable_range(5.0, 5.0).unwrap();
    rig.backend.clear_live_seekable_range().unwrap();
}

#[test]
fn t09_old_data_is_removed_and_the_feed_does_not_serve_it() {
    let s = load("h264-low");
    let rig = Rig::new();
    rig.backend.set_duration(Some(f64::INFINITY)).unwrap();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &mercury_mse_testkit::ChunkPlan::Whole);
    rig.remove_ok(id, 0.0, 4.0);
    let start = rig.buffered_secs(id)[0].0;
    assert!(
        (start - 4.0).abs() < 0.001,
        "buffered now starts at {start}"
    );
    let feed = rig.backend.frame_feed();
    feed.seek(0);
    let d = drain(&*feed, TrackKind::Video);
    assert!(
        d.frames.is_empty() && d.state == mercury_mse::api::FeedState::WaitingForData,
        "a seek to removed data waits for it: {} frames, {:?}",
        d.frames.len(),
        d.state
    );
    feed.seek(5_000_000_000);
    let d = drain(&*feed, TrackKind::Video);
    assert!(
        (d.frames[0].pts as f64 / 1e9 - 4.0).abs() < 0.001,
        "starts at the keyframe at 4 s"
    );
    assert_eq!(d.frames.len(), 100);
}
