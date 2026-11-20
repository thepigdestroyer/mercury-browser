// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The `FrameFeed` side of the backend (lane med-1, item 3), as `mercury-mse-gst` uses it: when the wake
//! closure is called, what a pull returns after a seek (the `Config` of the group of pictures it starts
//! at, a discontinuity), `Ended` in the same batch as the last frames, a seek to data that has not arrived.

mod model_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use mercury_mse::api::{FeedItem, FeedState, MediaSourceBackend, TrackKind};
use mercury_mse_testkit::ChunkPlan;
use model_support::*;

/// Counts the calls of the wake closure.
fn counting_wake(rig: &Rig) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&count);
    rig.backend.frame_feed().set_wake(Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
    }));
    count
}

/// Waits until the counter is above `before`.
#[track_caller]
fn woken_after(count: &AtomicUsize, before: usize, what: &str) -> usize {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let now = count.load(Ordering::SeqCst);
        if now > before {
            return now;
        }
        assert!(Instant::now() < deadline, "no wake after {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn the_wake_closure_is_called_for_everything_the_element_reads_again() {
    let s = load("h264-low");
    let rig = Rig::new();
    let wakes = counting_wake(&rig);
    let id = rig.add(&s.mime);
    assert_eq!(wakes.load(Ordering::SeqCst), 0, "nothing yet");
    // An init segment: tracks() and all_initialized() change.
    let feed = rig.backend.frame_feed();
    assert!(feed.tracks().is_empty() && !feed.all_initialized());
    rig.append_ok(id, &s.init.bytes);
    let n = woken_after(&wakes, 0, "the init segment");
    assert!(feed.all_initialized() && feed.tracks().len() == 1);
    // New frames.
    rig.append_ok(id, &s.media[0].bytes);
    let n = woken_after(&wakes, n, "new frames");
    // A duration change (the element re-reads the duration only when woken).
    rig.backend.set_duration(Some(30.0)).unwrap();
    let n = woken_after(&wakes, n, "a duration change");
    assert_eq!(feed.duration(), Some(30 * SEC));
    // A configuration change inside the buffer.
    let high = load("h264-high");
    rig.append_ok(id, &high.init.bytes);
    let n = woken_after(&wakes, n, "a new configuration");
    assert_eq!(feed.tracks()[0].width, 640);
    // The end of the stream.
    rig.backend.end_of_stream(None).unwrap();
    woken_after(&wakes, n, "the end of the stream");
}

#[test]
fn after_a_seek_the_next_pull_starts_with_the_config_and_a_discontinuity() {
    // low rendition for segments 0 and 1, high for 2 and 3, in one source buffer.
    let (low, high) = (load("h264-low"), load("h264-high"));
    let rig = Rig::new();
    let id = rig.add(&low.mime);
    rig.append_ok(id, &low.init.bytes);
    rig.append_ok(id, &low.media[0].bytes);
    rig.append_ok(id, &low.media[1].bytes);
    rig.append_ok(id, &high.init.bytes);
    rig.append_ok(id, &high.media[2].bytes);
    rig.append_ok(id, &high.media[3].bytes);
    let feed = rig.backend.frame_feed();
    let all = drain(&*feed, TrackKind::Video);
    assert_eq!(all.frames.len(), 200);
    assert_eq!(
        all.configs.len(),
        2,
        "the rendition switch is one Config item"
    );
    assert_eq!(all.configs[0].width, 320);
    assert_eq!(all.configs[1].width, 640);

    for (seek_s, want_width, want_first_s) in [
        (5.0, 640, 4.0),
        (1.0, 320, 0.0),
        (5.0, 640, 4.0),
        (6.0, 640, 6.0),
    ] {
        feed.seek((seek_s * 1e9) as i64);
        let batch = feed.pull(TrackKind::Video, 8);
        let FeedItem::Config(config) = &batch.items[0] else {
            panic!(
                "seek({seek_s}): the first item is {:?}, not a Config",
                batch.items[0]
            );
        };
        assert_eq!(config.width, want_width, "seek({seek_s})");
        let FeedItem::Frame { frame, discont } = &batch.items[1] else {
            panic!("seek({seek_s}): the second item is not a frame");
        };
        assert!(
            frame.keyframe && *discont,
            "seek({seek_s}): a keyframe with a discontinuity"
        );
        assert!(
            (frame.pts as f64 / 1e9 - want_first_s).abs() < 0.001,
            "seek({seek_s}) starts at {} s, not {want_first_s}",
            frame.pts as f64 / 1e9
        );
        // The frames that follow are in order and not discontinuities.
        for item in &batch.items[2..] {
            let FeedItem::Frame { discont, .. } = item else {
                panic!("another Config")
            };
            assert!(!discont);
        }
    }
}

#[test]
fn the_last_batch_carries_ended() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    let feed = rig.backend.frame_feed();
    // Not ended: the data runs out, the feed waits.
    let b = feed.pull(TrackKind::Video, 1000);
    assert_eq!(b.state, FeedState::WaitingForData);
    assert_eq!(
        b.items
            .iter()
            .filter(|i| matches!(i, FeedItem::Frame { .. }))
            .count(),
        200
    );
    rig.backend.end_of_stream(None).unwrap();
    // After the end the next pull has nothing and says so.
    let b = feed.pull(TrackKind::Video, 1000);
    assert!(b.items.is_empty());
    assert_eq!(b.state, FeedState::Ended);

    // From the start, in batches of 64: the batch that holds the last frame says Ended.
    feed.seek(0);
    let mut frames = 0;
    let mut states = Vec::new();
    loop {
        let b = feed.pull(TrackKind::Video, 64);
        frames += b
            .items
            .iter()
            .filter(|i| matches!(i, FeedItem::Frame { .. }))
            .count();
        states.push(b.state);
        if b.state != FeedState::More {
            break;
        }
        assert!(states.len() < 10);
    }
    assert_eq!(
        frames, 200,
        "the batch that says Ended has the last frames: {states:?}"
    );
    assert_eq!(*states.last().unwrap(), FeedState::Ended);
    assert!(
        states[..states.len() - 1]
            .iter()
            .all(|s| *s == FeedState::More)
    );
}

#[test]
fn a_seek_to_data_that_has_not_arrived_waits_and_resumes_on_the_wake() {
    let s = load("h264-low");
    let rig = Rig::new();
    let wakes = counting_wake(&rig);
    let id = rig.add(&s.mime);
    rig.append_ok(id, &s.init.bytes);
    rig.append_ok(id, &s.media[0].bytes);
    rig.append_ok(id, &s.media[1].bytes);
    let feed = rig.backend.frame_feed();
    feed.seek(6_000_000_000);
    let b = feed.pull(TrackKind::Video, 16);
    assert!(b.items.is_empty());
    assert_eq!(b.state, FeedState::WaitingForData);
    let before = wakes.load(Ordering::SeqCst);
    // The segment that covers 6 s arrives (4 s of data are missing before it).
    rig.append_ok(id, &s.media[3].bytes);
    woken_after(&wakes, before, "the segment the seek was waiting for");
    let b = feed.pull(TrackKind::Video, 16);
    let FeedItem::Config(_) = &b.items[0] else {
        panic!("a Config first")
    };
    let FeedItem::Frame { frame, discont } = &b.items[1] else {
        panic!("a frame")
    };
    assert!((frame.pts as f64 / 1e9 - 6.0).abs() < 0.001 && frame.keyframe && *discont);
}

#[test]
fn a_seek_to_or_past_the_end_of_an_ended_stream_ends_the_feed() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    rig.backend.end_of_stream(None).unwrap();
    let feed = rig.backend.frame_feed();
    for pos in [8_000_000_000i64, 9_000_000_000, 100_000_000_000] {
        feed.seek(pos);
        let b = feed.pull(TrackKind::Video, 16);
        assert!(b.items.is_empty(), "seek({pos}): nothing to serve");
        assert_eq!(b.state, FeedState::Ended, "seek({pos})");
    }
    // A seek into the stream serves it again.
    feed.seek(2_000_000_000);
    let b = feed.pull(TrackKind::Video, 1000);
    assert_eq!(b.state, FeedState::Ended);
    assert_eq!(
        b.items
            .iter()
            .filter(|i| matches!(i, FeedItem::Frame { .. }))
            .count(),
        150
    );
}

#[test]
fn a_kind_without_a_track_is_waiting_until_the_end_and_ended_after_it() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    let feed = rig.backend.frame_feed();
    let b = feed.pull(TrackKind::Audio, 16);
    assert!(b.items.is_empty());
    assert_eq!(b.state, FeedState::WaitingForData);
    rig.backend.end_of_stream(None).unwrap();
    assert_eq!(feed.pull(TrackKind::Audio, 16).state, FeedState::Ended);
    assert_eq!(feed.pull(TrackKind::Text, 16).state, FeedState::Ended);
}

#[test]
fn the_feed_follows_the_buffers_and_the_backend_is_a_feed_too() {
    use mercury_mse::api::FrameFeed;
    let (v, a) = (load("h264-low"), load("aac"));
    let rig = Rig::new();
    let (vid, aid) = (rig.add(&v.mime), rig.add(&a.mime));
    let feed = rig.backend.frame_feed();
    assert!(!feed.all_initialized());
    rig.append_ok(vid, &v.init.bytes);
    assert!(
        !feed.all_initialized(),
        "the audio buffer still lacks its init segment"
    );
    rig.append_ok(aid, &a.init.bytes);
    assert!(feed.all_initialized());
    assert_eq!(feed.tracks().len(), 2);
    // `Arc<MseBackend>` is a `FrameFeed` itself over the same state.
    let as_feed: Arc<dyn FrameFeed> = rig.backend.clone();
    assert_eq!(as_feed.tracks().len(), 2);
    rig.append_ok(vid, &v.media[0].bytes);
    assert_eq!(as_feed.pull(TrackKind::Video, 1000).items.len(), 51);
    // Removing a buffer removes its tracks from the feed.
    rig.backend.remove_source_buffer(aid).unwrap();
    assert_eq!(feed.tracks().len(), 1);
    assert_eq!(feed.tracks()[0].kind, TrackKind::Video);
}
