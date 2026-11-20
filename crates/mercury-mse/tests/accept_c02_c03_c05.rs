// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C02, C03 and C05 of the GstMSE spike, ported to the Mercury backend at core level (lane med-1, item
//! 3): audio and video in two source buffers (fragmented MP4 and WebM) and muxed in one (fragmented
//! MP4), appended the way a DASH player does, then played from the feed. What the spike checked on the
//! decoded pipeline (200 frames, 375 AAC or 400 Opus frames, `buffered` [0, 8 s], duration 8 s) is
//! checked here on the frames the feed serves.

mod model_support;

use mercury_mse::api::{
    CodecId, ElementReadyState, FeedState, MediaSourceBackend, MediaSourceReadyState, MseEvent,
    TrackKind,
};
use mercury_mse_testkit::ChunkPlan;
use model_support::*;

/// What a played-through stream must give.
struct Expect {
    kind: TrackKind,
    frames: usize,
    /// The end of the last frame, seconds.
    end_s: f64,
}

fn check_track(rig: &Rig, expect: &Expect) {
    let feed = rig.backend.frame_feed();
    let d = drain(&*feed, expect.kind);
    let label = expect.kind.name();
    assert_eq!(d.frames.len(), expect.frames, "{label}: frames served");
    assert_in_order(label, &d.frames);
    assert_eq!(d.frames[0].pts, 0, "{label}: first frame at 0");
    let last = d.frames.last().unwrap();
    assert!(
        ((last.pts + last.duration) as f64 / 1e9 - expect.end_s).abs() < 0.002,
        "{label}: the last frame ends at {}, expected {}",
        (last.pts + last.duration) as f64 / 1e9,
        expect.end_s
    );
    assert!(
        d.frames
            .iter()
            .all(|f| f.kind == expect.kind && !f.data.is_empty()),
        "{label}: kinds and data"
    );
    // The feed starts with a discontinuity and goes on without one.
    assert!(d.discont[0], "{label}: the first frame is a discontinuity");
    assert!(
        d.discont[1..].iter().all(|x| !x),
        "{label}: no discontinuity after the first frame"
    );
    assert_eq!(
        d.configs.len(),
        1,
        "{label}: one Config item for one configuration"
    );
    assert_eq!(d.configs[0].kind, expect.kind);
    // The stream has ended and was served to the end.
    assert_eq!(
        d.state,
        FeedState::Ended,
        "{label}: the feed ends after the last frame"
    );
}

/// Both frames of the first and second GOP of video are keyframes where the fixtures say.
fn check_video_keyframes(rig: &Rig) {
    let feed = rig.backend.frame_feed();
    feed.seek(0);
    let d = drain(&*feed, TrackKind::Video);
    let keys: Vec<f64> = d
        .frames
        .iter()
        .filter(|f| f.keyframe)
        .map(|f| f.pts as f64 / 1e9)
        .collect();
    assert_eq!(keys.len(), 4, "a keyframe every 2 s: {keys:?}");
    for (i, k) in keys.iter().enumerate() {
        assert!((k - 2.0 * i as f64).abs() < 0.001, "keyframe {i} at {k}");
    }
}

fn first_init_before_done(rig: &Rig, id: u32) {
    // The InitSegment event of a buffer comes before the AppendDone of the same append, and both only
    // once for a stream that has one init segment.
    let events = rig.rec.events();
    let init_at = events
        .iter()
        .position(|e| matches!(e, MseEvent::InitSegment { id: i, .. } if *i == id))
        .expect("an InitSegment event");
    let done_at = events
        .iter()
        .position(|e| matches!(e, MseEvent::AppendDone { id: i } if *i == id))
        .expect("an AppendDone event");
    assert!(
        init_at < done_at,
        "InitSegment before the first AppendDone of buffer {id}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, MseEvent::InitSegment { id: i, .. } if *i == id))
            .count(),
        1
    );
}

fn split_av(video: &str, audio: &str, audio_frames: usize) {
    let (v, a) = (load(video), load(audio));
    let rig = Rig::new();
    let (vid, aid) = (rig.add(&v.mime), rig.add(&a.mime));

    // Init segments first (the way dash.js does), then the media segments alternately.
    rig.append_ok(vid, &v.init.bytes);
    rig.append_ok(aid, &a.init.bytes);
    assert!(rig.backend.frame_feed().all_initialized());
    for i in 0..v.media.len().max(a.media.len()) {
        if let Some(s) = v.media.get(i) {
            rig.append_ok(vid, &s.bytes);
        }
        if let Some(s) = a.media.get(i) {
            rig.append_ok(aid, &s.bytes);
        }
    }

    assert_ranges(
        "video buffered",
        &rig.backend.buffered(vid),
        &[(0.0, 8.0)],
        2.0,
    );
    assert_ranges(
        "audio buffered",
        &rig.backend.buffered(aid),
        &[(0.0, 8.0)],
        2.0,
    );
    assert_ranges(
        "the element's buffered is the intersection",
        &rig.backend.element_buffered(),
        &[(0.0, 8.0)],
        2.0,
    );
    let tracks = rig.backend.frame_feed().tracks();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].kind, TrackKind::Video);
    assert_eq!(tracks[1].kind, TrackKind::Audio);

    rig.backend.end_of_stream(None).unwrap();
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Ended);
    let duration = rig
        .backend
        .duration()
        .expect("a duration after the end of the stream");
    assert!((duration - 8.0).abs() < 0.03, "duration {duration}");
    assert_eq!(
        rig.backend.element_ready_state(0.0),
        ElementReadyState::HaveEnoughData
    );

    check_track(
        &rig,
        &Expect {
            kind: TrackKind::Video,
            frames: 200,
            end_s: 8.0,
        },
    );
    check_track(
        &rig,
        &Expect {
            kind: TrackKind::Audio,
            frames: audio_frames,
            end_s: 8.0,
        },
    );
    first_init_before_done(&rig, vid);
    first_init_before_done(&rig, aid);
}

#[test]
fn c02_split_av_fmp4_h264_aac() {
    split_av("h264-low", "aac", 375);
}

#[test]
fn c05_split_av_webm_vp9_opus() {
    split_av("vp9-low", "opus", 400);
}

#[test]
fn c03_muxed_av_in_one_source_buffer_fmp4() {
    let m = load("h264-aac");
    let rig = Rig::new();
    let id = rig.add(&m.mime);
    rig.append_stream(id, &m, &ChunkPlan::Whole);
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 8.0)], 2.0);
    assert_ranges(
        "element buffered",
        &rig.backend.element_buffered(),
        &[(0.0, 8.0)],
        2.0,
    );
    rig.backend.end_of_stream(None).unwrap();
    let duration = rig.backend.duration().unwrap();
    assert!((duration - 8.0).abs() < 0.03, "duration {duration}");
    check_track(
        &rig,
        &Expect {
            kind: TrackKind::Video,
            frames: 200,
            end_s: 8.0,
        },
    );
    check_track(
        &rig,
        &Expect {
            kind: TrackKind::Audio,
            frames: 375,
            end_s: 8.0,
        },
    );
    first_init_before_done(&rig, id);
    // Both tracks come from the one buffer, and the init event listed both.
    let init = rig
        .rec
        .events()
        .into_iter()
        .find_map(|e| match e {
            MseEvent::InitSegment { init, .. } => Some(init),
            _ => None,
        })
        .unwrap();
    assert!(init.first);
    let kinds: Vec<TrackKind> = init.tracks.iter().map(|t| t.kind).collect();
    assert!(kinds.contains(&TrackKind::Video) && kinds.contains(&TrackKind::Audio));
}

#[test]
fn c02_keyframes_survive_into_the_feed() {
    let v = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&v.mime);
    rig.append_stream(id, &v, &ChunkPlan::Whole);
    rig.backend.end_of_stream(None).unwrap();
    check_video_keyframes(&rig);
}

#[test]
fn c02_the_feed_hands_out_each_buffers_own_track_configuration() {
    let (v, a) = (load("h264-low"), load("aac"));
    let rig = Rig::new();
    let (vid, aid) = (rig.add(&v.mime), rig.add(&a.mime));
    rig.append_stream(vid, &v, &ChunkPlan::Whole);
    rig.append_stream(aid, &a, &ChunkPlan::Whole);
    let tracks = rig.backend.frame_feed().tracks();
    let video = tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap();
    let audio = tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap();
    assert_eq!(video.codec.id(), CodecId::H264);
    assert_eq!((video.width, video.height), (320, 180));
    assert_eq!(audio.codec.id(), CodecId::Aac);
    assert_eq!((audio.sample_rate, audio.channels), (48_000, 1));
}

#[test]
fn c02_two_video_buffers_serve_the_first_one() {
    // The feed serves the first source buffer that has a track of the kind (decision of item 3).
    let (low, high) = (load("h264-low"), load("h264-high"));
    let rig = Rig::new();
    let (a, b) = (rig.add(&low.mime), rig.add(&high.mime));
    rig.append_stream(a, &low, &ChunkPlan::Whole);
    rig.append_stream(b, &high, &ChunkPlan::Whole);
    let feed = rig.backend.frame_feed();
    let tracks = feed.tracks();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].width, 320);
    // Both buffers hold data, and the element's buffered is their intersection.
    assert_ranges("a", &rig.backend.buffered(a), &[(0.0, 8.0)], 2.0);
    assert_ranges("b", &rig.backend.buffered(b), &[(0.0, 8.0)], 2.0);
    // When the first buffer goes, the second one takes over at the next pull.
    rig.backend.remove_source_buffer(a).unwrap();
    let tracks = feed.tracks();
    assert_eq!(tracks[0].width, 640);
    let d = drain(&*feed, TrackKind::Video);
    assert_eq!(d.frames.len(), 200);
}
