// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Late tracks, the model half of decision D17 (lane med-1b, item 1 `core`): a source buffer added, or
//! whose initialization segment arrives, after the element announced its streams. The feed must list the
//! track in `tracks()` and wake the consumer for every change, `all_initialized()` keeps its meaning, a
//! quality switch is not a new track, and the late track is served from the current position, not from
//! the start of the stream.

mod model_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use mercury_mse::api::{FeedState, MediaSourceBackend, TrackKind};
use model_support::*;

fn counting_wake(rig: &Rig) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&count);
    rig.backend.frame_feed().set_wake(Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
    }));
    count
}

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
fn late_a_second_buffer_added_and_initialized_later_appears_in_tracks_and_wakes() {
    let (v, a) = (load("h264-low"), load("aac"));
    let rig = Rig::new();
    let wakes = counting_wake(&rig);
    let feed = rig.backend.frame_feed();

    // The video starts alone: the element announces one stream and plays.
    let vid = rig.add(&v.mime);
    rig.append_stream(vid, &v, &mercury_mse_testkit::ChunkPlan::Whole);
    let n = woken_after(&wakes, 0, "the video's init segment");
    assert!(feed.all_initialized());
    assert_eq!(feed.tracks().len(), 1);
    assert_eq!(feed.tracks()[0].kind, TrackKind::Video);
    assert!(!drain(feed.as_ref(), TrackKind::Video).frames.is_empty());

    // 1 s later the page adds an audio buffer: nothing in tracks() yet, but the feed is no longer
    // fully initialized (the meaning of all_initialized() is "every buffer that exists has its init").
    rig.backend.set_position(1.0);
    let aid = rig.add(&a.mime);
    assert!(!feed.all_initialized());
    assert_eq!(feed.tracks().len(), 1, "no init segment yet, no track");

    // Its init segment arrives: the feed lists both, wakes, and is fully initialized again.
    rig.append_ok(aid, &a.init.bytes);
    woken_after(&wakes, n, "the late init segment");
    assert!(feed.all_initialized());
    let tracks = feed.tracks();
    assert_eq!(tracks.len(), 2, "{tracks:?}");
    assert_eq!(tracks[0].kind, TrackKind::Video);
    assert_eq!(tracks[1].kind, TrackKind::Audio);
}

#[test]
fn late_a_buffer_whose_init_arrives_late_keeps_all_initialized_false_until_then() {
    let (v, a) = (load("h264-low"), load("aac"));
    let rig = Rig::new();
    let wakes = counting_wake(&rig);
    let feed = rig.backend.frame_feed();
    let (vid, aid) = (rig.add(&v.mime), rig.add(&a.mime));

    rig.append_ok(vid, &v.init.bytes);
    let n = woken_after(&wakes, 0, "the video's init segment");
    assert!(!feed.all_initialized(), "the audio buffer has no init yet");
    assert_eq!(feed.tracks().len(), 1);

    // Media appended to the video meanwhile is served; the audio kind just waits.
    rig.append_ok(vid, &v.media[0].bytes);
    assert!(!drain(feed.as_ref(), TrackKind::Video).frames.is_empty());
    assert_eq!(
        feed.pull(TrackKind::Audio, 8).state,
        FeedState::WaitingForData
    );

    rig.append_ok(aid, &a.init.bytes);
    woken_after(&wakes, n, "the audio init segment");
    assert!(feed.all_initialized());
    assert_eq!(feed.tracks().len(), 2);
}

#[test]
fn late_every_change_of_tracks_wakes_the_feed() {
    let (v, a) = (load("h264-low"), load("aac"));
    let rig = Rig::new();
    let wakes = counting_wake(&rig);
    let feed = rig.backend.frame_feed();
    let (vid, aid) = (rig.add(&v.mime), rig.add(&a.mime));

    rig.append_ok(vid, &v.init.bytes);
    let n = woken_after(&wakes, 0, "the video's init segment");
    rig.append_ok(aid, &a.init.bytes);
    let n = woken_after(&wakes, n, "the audio's init segment");
    assert_eq!(feed.tracks().len(), 2);

    // A track that disappears is a change too: the consumer must be able to see it.
    rig.backend.remove_source_buffer(aid).unwrap();
    woken_after(&wakes, n, "removing the audio buffer");
    assert_eq!(feed.tracks().len(), 1);
    assert_eq!(feed.tracks()[0].kind, TrackKind::Video);
}

#[test]
fn late_a_quality_switch_is_not_a_new_track() {
    let (low, high) = (load("h264-low"), load("h264-high"));
    let rig = Rig::new();
    let wakes = counting_wake(&rig);
    let feed = rig.backend.frame_feed();
    let id = rig.add(&low.mime);
    rig.append_ok(id, &low.init.bytes);
    let n = woken_after(&wakes, 0, "the init segment");
    let before = feed.tracks();
    assert_eq!(before.len(), 1);
    let low_size = (before[0].width, before[0].height);

    // The same buffer gets the init segment of another rendition: same kind, replaced configuration.
    rig.append_ok(id, &high.init.bytes);
    woken_after(&wakes, n, "the new configuration");
    let after = feed.tracks();
    assert_eq!(after.len(), 1, "a quality switch adds no track: {after:?}");
    assert_eq!(after[0].kind, TrackKind::Video);
    assert_eq!(after[0].track_id, before[0].track_id);
    assert_ne!(
        (after[0].width, after[0].height),
        low_size,
        "the configuration is the new one"
    );
    assert!(feed.all_initialized());
}

#[test]
fn late_the_late_track_is_served_from_the_current_position() {
    let (v, a) = (load("h264-low"), load("aac"));
    let rig = Rig::new();
    let feed = rig.backend.frame_feed();
    let vid = rig.add(&v.mime);
    rig.append_stream(vid, &v, &mercury_mse_testkit::ChunkPlan::Whole);

    // The pipeline runs: the video cursor exists and has delivered the first frames.
    let first = feed.pull(TrackKind::Video, 10);
    assert!(!first.items.is_empty());

    // Playback is at 3 s when the audio buffer shows up with the whole stream.
    rig.backend.set_position(3.0);
    let aid = rig.add(&a.mime);
    rig.append_stream(aid, &a, &mercury_mse_testkit::ChunkPlan::Whole);
    let audio = drain(feed.as_ref(), TrackKind::Audio);
    assert!(!audio.frames.is_empty(), "the late track serves frames");
    let first_pts = audio.frames[0].pts as f64 / 1e9;
    assert!(
        (2.9..=3.001).contains(&first_pts),
        "the late audio starts at {first_pts} s, playback is at 3 s"
    );
    assert!(
        audio.discont[0],
        "the first frame of a new track is a discontinuity"
    );
    assert_eq!(
        audio.configs.len(),
        1,
        "and it comes with its configuration"
    );
    assert_in_order("late audio", &audio.frames);
}

#[test]
fn late_a_track_that_exists_from_the_start_is_served_from_the_start() {
    // The rule above must not move a normal start: the element is at 0 when the pipeline pulls its two
    // kinds one after the other, so both begin where the feed was pointed.
    let s = load("h264-aac");
    let rig = Rig::new();
    let feed = rig.backend.frame_feed();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &mercury_mse_testkit::ChunkPlan::Whole);
    rig.backend.set_position(0.0);
    let video = drain(feed.as_ref(), TrackKind::Video);
    let audio = drain(feed.as_ref(), TrackKind::Audio);
    assert!(
        video.frames[0].pts as f64 / 1e9 < 0.1,
        "video {:?}",
        video.frames[0].pts
    );
    assert!(
        audio.frames[0].pts as f64 / 1e9 < 0.1,
        "audio {:?}",
        audio.frames[0].pts
    );
}

#[test]
fn late_after_a_seek_both_kinds_start_at_the_target() {
    // A seek restarts every cursor; the kind pulled second is not "late": it starts at the target too.
    let (v, a) = (load("h264-low"), load("aac"));
    let rig = Rig::new();
    let feed = rig.backend.frame_feed();
    let vid = rig.add(&v.mime);
    let aid = rig.add(&a.mime);
    rig.append_stream(vid, &v, &mercury_mse_testkit::ChunkPlan::Whole);
    rig.append_stream(aid, &a, &mercury_mse_testkit::ChunkPlan::Whole);
    rig.backend.set_position(6.0);
    rig.backend.seeking(2.0);
    feed.seek(2_000_000_000);
    let video = drain(feed.as_ref(), TrackKind::Video);
    let audio = drain(feed.as_ref(), TrackKind::Audio);
    let v0 = video.frames[0].pts as f64 / 1e9;
    let a0 = audio.frames[0].pts as f64 / 1e9;
    assert!(
        v0 <= 2.0,
        "video starts at the random access point at or before 2 s: {v0}"
    );
    assert!((1.9..=2.001).contains(&a0), "audio starts at {a0}");
}
