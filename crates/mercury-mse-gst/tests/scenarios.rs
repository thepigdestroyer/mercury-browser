// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The playback half of the GstMSE spike's scenarios C02 to C15 (brief section 6, item 4), against the
//! real `MseBackend` (worker thread, parsers, processor, track buffers) behind the `FrameFeed`, through
//! `MseSrc`, with software decoders and `appsink`s. Fixture content: 8 s, 25 fps, red for 4 s then blue.
//!
//! The scenarios run in both pipeline shapes where it matters: `bare` is `MseSrc ! decodebin3 ! appsinks`,
//! `playbin` is `playbin3` with `source-setup` calling `attach`, the way the browser builds its player.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::real::{RealSource, load};
use common::rig::{Options, Rig, Shape, watchdog};
use common::scenario::{max_pts_gap, seek_and_measure};
use common::sinks::{Colour, classify};
use mercury_mse::{MediaSourceBackend, SourceBufferId};
use mercury_mse_testkit::{ChunkPlan, SegmentedStream};

const LIMIT: Duration = Duration::from_secs(30);
const SEC: u64 = 1_000_000_000;

fn options(shape: Shape, sync: bool) -> Options {
    Options {
        shape,
        sync,
        ..Options::default()
    }
}

/// What a played-through stream gave at the sinks.
struct Played {
    video_frames: usize,
    audio_seconds: f64,
    first_colour: Option<Colour>,
    last_colour: Option<Colour>,
    video_caps: usize,
    max_gap: u64,
    gaps: Vec<(usize, u64, u64)>,
}

fn summary(rig: &Rig) -> Played {
    let video = rig.video.as_ref().map(|s| s.stats.lock().unwrap());
    let audio = rig.audio.as_ref().map(|s| s.stats.lock().unwrap());
    let colours = video
        .as_ref()
        .map(|v| v.colours_since(0))
        .unwrap_or_default();
    Played {
        video_frames: video.as_ref().map_or(0, |v| v.count()),
        audio_seconds: audio.as_ref().map_or(0.0, |a| a.total_seconds()),
        first_colour: colours.first().map(|c| c.1),
        last_colour: colours.last().map(|c| c.1),
        video_caps: video.as_ref().map_or(0, |v| v.caps.len()),
        max_gap: video.as_ref().map_or(0, |v| max_pts_gap(v, 0)),
        gaps: video
            .as_ref()
            .map_or_else(Vec::new, |v| common::scenario::gaps_over(v, 41_000_000)),
    }
}

/// Appends `fixtures` (one source buffer each) while the pipeline already plays, ends the stream and waits
/// for the end of the pipeline. The way a page feeds a player that was attached first.
fn play_fixtures(shape: Shape, fixtures: &[&str], plan: &ChunkPlan) -> Played {
    let source = RealSource::new();
    let rig = Rig::new(source.feed(), options(shape, false));
    rig.play();
    let streams: Vec<(SourceBufferId, SegmentedStream)> = fixtures
        .iter()
        .map(|name| {
            let stream = load(name);
            (source.add(&stream.mime), stream)
        })
        .collect();
    // Init segments first (both buffers), then the media segments of the buffers in turn.
    for (id, stream) in &streams {
        source.append_plan(*id, &stream.init.bytes, plan);
    }
    let segments = streams.iter().map(|(_, s)| s.media.len()).max().unwrap();
    for index in 0..segments {
        for (id, stream) in &streams {
            if let Some(segment) = stream.media.get(index) {
                source.append_plan(*id, &segment.bytes, plan);
            }
        }
    }
    source.end_of_stream();
    rig.wait_eos(LIMIT);
    summary(&rig)
}

fn check_full(label: &str, played: &Played, audio_seconds: (f64, f64)) {
    assert!(
        (198..=202).contains(&played.video_frames),
        "{label}: {} video frames",
        played.video_frames
    );
    assert!(
        played.audio_seconds >= audio_seconds.0 && played.audio_seconds <= audio_seconds.1,
        "{label}: {:.3} s of audio",
        played.audio_seconds
    );
    assert_eq!(played.first_colour, Some(Colour::Red), "{label}");
    assert_eq!(played.last_colour, Some(Colour::Blue), "{label}");
    assert!(
        played.max_gap <= 41_000_000,
        "{label}: video gap {} ns: {:?}",
        played.max_gap,
        played.gaps
    );
}

// ---- C02, C03, C05: split and muxed audio and video, fragmented MP4 and WebM ----

macro_rules! both_shapes {
    ($name:ident, $require:expr, $fixtures:expr, $audio:expr) => {
        mod $name {
            use super::*;

            #[test]
            fn bare() {
                require!("decodebin3", "videoconvert", "appsrc");
                for element in $require {
                    require!(element);
                }
                watchdog(concat!(stringify!($name), "-bare"), 60, || {
                    let played = play_fixtures(Shape::Bare, $fixtures, &ChunkPlan::Whole);
                    check_full(concat!(stringify!($name), " bare"), &played, $audio);
                });
            }

            #[test]
            fn playbin() {
                require!("playbin3", "videoconvert", "appsrc");
                for element in $require {
                    require!(element);
                }
                watchdog(concat!(stringify!($name), "-playbin"), 60, || {
                    let played = play_fixtures(Shape::PlayBin, $fixtures, &ChunkPlan::Whole);
                    check_full(concat!(stringify!($name), " playbin"), &played, $audio);
                });
            }
        }
    };
}

both_shapes!(
    c02_split_fmp4_h264_aac,
    ["avdec_h264", "avdec_aac"],
    &["h264-low", "aac"],
    (7.95, 8.05)
);
both_shapes!(
    c03_muxed_fmp4_h264_aac,
    ["avdec_h264", "avdec_aac"],
    &["h264-aac"],
    (7.95, 8.05)
);
both_shapes!(
    c05_split_webm_vp9_opus,
    ["vp9dec", "opusdec"],
    &["vp9-low", "opus"],
    (7.9, 8.05)
);
both_shapes!(
    c05_split_webm_av1_opus,
    ["dav1ddec", "opusdec"],
    &["av1-low-av1c", "opus"],
    (7.9, 8.05)
);
both_shapes!(
    c02_split_fmp4_h264_high_aac,
    ["avdec_h264", "avdec_aac"],
    &["h264-high", "aac"],
    (7.95, 8.05)
);
both_shapes!(
    c02_h264_with_b_frames_aac,
    ["avdec_h264", "avdec_aac"],
    &["h264-bframes", "aac"],
    (7.95, 8.05)
);

// ---- C04: the same playback under every chunk plan ----

#[test]
fn c04_playback_is_identical_under_every_chunk_plan() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c04", 240, || {
        let mut results = Vec::new();
        for plan in [
            ChunkPlan::Whole,
            ChunkPlan::Fixed(65_536),
            ChunkPlan::Fixed(1024),
            ChunkPlan::Sizes(vec![13, 4096, 1]),
        ] {
            let played = play_fixtures(Shape::PlayBin, &["h264-low", "aac"], &plan);
            eprintln!(
                "C04 {plan:?}: {} frames, {:.3} s audio",
                played.video_frames, played.audio_seconds
            );
            check_full(&format!("{plan:?}"), &played, (7.95, 8.05));
            results.push((
                played.video_frames,
                (played.audio_seconds * 1000.0).round() as i64,
            ));
        }
        assert!(results.windows(2).all(|w| w[0] == w[1]), "{results:?}");
    });
}

// ---- C06: seeks inside the buffer ----

fn seeks_in_buffer(shape: Shape) {
    let source = RealSource::new();
    let stream = load("h264-aac");
    let id = source.add(&stream.mime);
    source.append_stream(id, &stream, &ChunkPlan::Whole);
    source.end_of_stream();
    let rig = Rig::new(source.feed(), options(shape, true));
    rig.play();
    rig.wait_until(LIMIT, "the first second of video", |rig| {
        rig.video_stats().lock().unwrap().count() >= 25
    });
    for (target, colour) in [
        (5 * SEC, Colour::Blue),
        (SEC / 2, Colour::Red),
        (7_900_000_000, Colour::Blue),
    ] {
        let outcome = seek_and_measure(&rig, target, true, Duration::from_secs(10));
        eprintln!(
            "C06 {shape:?} seek to {} ms: {outcome:?}",
            target / 1_000_000
        );
        assert_eq!(outcome.first_video_colour, Some(colour), "{outcome:?}");
        assert!(
            outcome.first_video_pts.unwrap() + 100_000_000 >= target,
            "{outcome:?}"
        );
        assert!(
            outcome.first_video.unwrap() < Duration::from_millis(500),
            "{outcome:?}"
        );
        assert_eq!(outcome.async_done, 1, "one seek-done per seek: {outcome:?}");
        let (video, audio) = (outcome.first_video.unwrap(), outcome.first_audio.unwrap());
        let apart = video.abs_diff(audio);
        assert!(
            apart < Duration::from_millis(300),
            "audio and video resume apart: {outcome:?}"
        );
    }
}

#[test]
fn c06_seek_in_buffer_bare() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "decodebin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c06-bare", 90, || seeks_in_buffer(Shape::Bare));
}

#[test]
fn c06_seek_in_buffer_playbin() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c06-playbin", 90, || seeks_in_buffer(Shape::PlayBin));
}

// ---- C07: seeks to data that is not there yet ----

/// A source with `h264-low` and `aac`, both with only their init segments and the first `segments` media
/// segments appended.
fn two_buffers(segments: usize) -> (RealSource, [(SourceBufferId, SegmentedStream); 2]) {
    let source = RealSource::new();
    let video = load("h264-low");
    let audio = load("aac");
    let ids = [source.add(&video.mime), source.add(&audio.mime)];
    for (id, stream) in ids.iter().zip([&video, &audio]) {
        source.append_init(*id, stream);
        for index in 0..segments {
            source.append_segment(*id, stream, index);
        }
    }
    let [v, a] = ids;
    (source, [(v, video), (a, audio)])
}

#[test]
fn c07_seek_to_an_unbuffered_range_holds_then_resumes() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c07", 90, || {
        // Data for 0 to 4 s only; the stream is not ended.
        let (source, buffers) = two_buffers(2);
        let rig = Rig::new(source.feed(), options(Shape::PlayBin, true));
        rig.play();
        rig.wait_until(LIMIT, "frames", |rig| {
            rig.video_stats().lock().unwrap().count() >= 10
        });
        let flushes = rig.video_stats().lock().unwrap().flushes;
        assert!(rig.seek(5 * SEC, true));
        std::thread::sleep(Duration::from_millis(1000));
        rig.pump_bus();
        {
            let stats = rig.video_stats();
            let stats = stats.lock().unwrap();
            let mark = stats.flush_marks.get(flushes as usize).copied();
            assert!(
                mark.is_some_and(|m| m == stats.arrivals.len()),
                "no frame may follow a seek to unbuffered data (flush at {mark:?}, {} frames)",
                stats.arrivals.len()
            );
        }
        // The pipeline waits in its seek (an asynchronous state change): it has no position to give, or the
        // target itself. It does not end and it does not fail.
        let position = rig.position().map(|p| p.nseconds());
        eprintln!("C07 position while waiting: {position:?}");
        assert!(
            position.is_none_or(|p| p.abs_diff(5 * SEC) < SEC / 5),
            "the position is held at the seek target: {position:?}"
        );
        rig.pump_bus();
        assert_eq!(rig.bus_log.lock().unwrap().eos, 0, "the stream did not end");
        // The data arrives: segment 2 of both (4 s to 6 s).
        let appended = {
            let mut latest = None;
            for (id, stream) in &buffers {
                latest = Some(source.append_segment(*id, stream, 2));
            }
            latest.unwrap()
        };
        let started = appended.saturating_duration_since(rig.t0);
        rig.wait_until(
            Duration::from_secs(10),
            "the first frame after the append",
            |rig| {
                let stats = rig.video_stats();
                let stats = stats.lock().unwrap();
                stats
                    .flush_marks
                    .get(flushes as usize)
                    .is_some_and(|&m| stats.arrivals.len() > m)
            },
        );
        let stats = rig.video_stats();
        let stats = stats.lock().unwrap();
        let first = &stats.arrivals[stats.flush_marks[flushes as usize]];
        let latency = first.at.saturating_sub(started);
        eprintln!(
            "C07 resume {latency:?} after the append, first pts {:?}",
            first.pts
        );
        assert!(
            latency < Duration::from_millis(1000),
            "resume took {latency:?}"
        );
        assert_eq!(first.rgb.map(classify), Some(Colour::Blue));
        assert!(first.pts.unwrap() + 100_000_000 >= 5 * SEC);
    });
}

#[test]
fn c07_seek_with_only_an_init_segment_then_append() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c07-init", 90, || {
        let (source, buffers) = two_buffers(0);
        let rig = Rig::new(source.feed(), options(Shape::PlayBin, true));
        rig.play();
        std::thread::sleep(Duration::from_millis(300));
        // With nothing to decode, `playbin3` has no stream to seek in and refuses the seek (GstPlay, which
        // the player uses, ignores seeks until it knows the media is seekable). Recorded for the player
        // hook; the page seeks again when the data arrives.
        let early = rig.seek(2 * SEC, true);
        eprintln!("C07 init-only: a seek before the first buffer is accepted: {early}");
        // The DOM seeks (currentTime = 2): the backend is told, so that its feed serves from there.
        source.backend.seeking(2.0);
        source.backend.set_position(2.0);
        // `seeking` only moves the model's position; the feed's cursor starts at 0 until a seek reaches it,
        // and `playbin3` refuses seeks while nothing has prerolled. So the hook moves the cursor itself
        // (`FrameFeed::seek` is callable from any thread) until the pipeline can take the seek.
        mercury_mse::FrameFeed::seek(&*source.feed(), (2 * SEC) as i64);
        for (id, stream) in &buffers {
            source.append_segment(*id, stream, 1);
        }
        // The segment at 2 s to 4 s is in: the pipeline prerolls on its first frame (which `playbin3` shows
        // for the segment that starts at 0, so the sink would wait for 2 s of running time), and the seek
        // the player repeats is then accepted.
        let flushes = rig.video_stats().lock().unwrap().flushes;
        rig.wait_until(
            Duration::from_secs(10),
            "the pipeline to accept the seek",
            |rig| rig.seek(2 * SEC, true),
        );
        let accepted = rig.elapsed();
        rig.wait_until(
            Duration::from_secs(10),
            "the first frame after the seek",
            |rig| {
                let stats = rig.video_stats();
                let stats = stats.lock().unwrap();
                stats
                    .flush_marks
                    .get(flushes as usize)
                    .is_some_and(|&mark| stats.arrivals.len() > mark)
            },
        );
        let stats = rig.video_stats();
        let stats = stats.lock().unwrap();
        let first = &stats.arrivals[stats.flush_marks[flushes as usize]];
        let latency = first.at.saturating_sub(accepted);
        eprintln!(
            "C07 init-only: first frame after the repeated seek {latency:?}, pts {:?}",
            first.pts
        );
        assert!(
            latency < Duration::from_millis(1000),
            "resume took {latency:?}"
        );
        assert_eq!(first.rgb.map(classify), Some(Colour::Red));
    });
}

#[test]
fn c07_seek_to_or_past_the_duration_ends_the_stream() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c07-end", 90, || {
        let source = RealSource::new();
        let stream = load("h264-aac");
        let id = source.add(&stream.mime);
        source.append_stream(id, &stream, &ChunkPlan::Whole);
        source.end_of_stream();
        let rig = Rig::new(source.feed(), options(Shape::PlayBin, true));
        rig.play();
        rig.wait_until(LIMIT, "frames", |rig| {
            rig.video_stats().lock().unwrap().count() >= 10
        });
        for target in [8 * SEC, 9 * SEC] {
            rig.pump_bus();
            let before = rig.bus_log.lock().unwrap().eos;
            let frames = rig.video_stats().lock().unwrap().count();
            assert!(rig.seek(target, true));
            rig.wait_until(Duration::from_secs(5), "the end of the stream", |rig| {
                rig.bus_log.lock().unwrap().eos > before
            });
            let after = rig.video_stats().lock().unwrap().count();
            eprintln!(
                "C07 seek to {} s: EOS with {} new frames",
                target / SEC,
                after - frames
            );
            assert!(after - frames <= 1, "a seek past the data shows no frames");
            // Back to the beginning for the next case.
            assert!(rig.seek(0, true));
            rig.wait_until(
                Duration::from_secs(10),
                "a frame after the seek back",
                |rig| rig.video_stats().lock().unwrap().count() > after,
            );
        }
    });
}

// ---- C08: quality switches inside one source buffer ----

/// Appends init `a`, its segments `first`, then init `b` and its segments `second`, ... : each pair is
/// `(stream, segment indexes)`.
fn append_switches(
    source: &RealSource,
    id: SourceBufferId,
    parts: &[(&SegmentedStream, &[usize])],
) {
    for (stream, indexes) in parts {
        source.append_init(id, stream);
        for &index in *indexes {
            source.append_segment(id, stream, index);
        }
    }
}

fn plays_switches(
    shape: Shape,
    parts: &[(&SegmentedStream, &[usize])],
    switches: usize,
    label: &str,
) {
    let source = RealSource::new();
    let audio = load("aac");
    let audio_id = source.add(&audio.mime);
    source.append_stream(audio_id, &audio, &ChunkPlan::Whole);
    let video_id = source.add(&parts[0].0.mime);
    append_switches(&source, video_id, parts);
    source.end_of_stream();
    let rig = Rig::new(source.feed(), options(shape, false));
    rig.play();
    rig.wait_eos(LIMIT);
    let played = summary(&rig);
    let video = rig.video_stats();
    let caps = video.lock().unwrap().caps.clone();
    eprintln!(
        "C08 {label} {shape:?}: {} frames, caps {caps:?}",
        played.video_frames
    );
    assert!(
        (198..=202).contains(&played.video_frames),
        "{label}: {} frames",
        played.video_frames
    );
    assert_eq!(
        played.video_caps,
        switches + 1,
        "{label}: caps change once per switch: {caps:?}"
    );
    assert!(
        played.max_gap <= 41_000_000,
        "{label}: gap {} ns",
        played.max_gap
    );
}

#[test]
fn c08_quality_switch_up_and_alternate_and_overwrite() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c08", 180, || {
        let low = load("h264-low");
        let high = load("h264-high");
        // up: low for the first two segments, high for the last two
        plays_switches(
            Shape::PlayBin,
            &[(&low, &[0, 1]), (&high, &[2, 3])],
            1,
            "up",
        );
        // alternate: three switches
        plays_switches(
            Shape::PlayBin,
            &[(&low, &[0]), (&high, &[1]), (&low, &[2]), (&high, &[3])],
            3,
            "alternate",
        );
        // overwrite: all of low is in, then high replaces the last half before playback starts
        plays_switches(
            Shape::PlayBin,
            &[(&low, &[0, 1, 2, 3]), (&high, &[2, 3])],
            1,
            "overwrite",
        );
    });
}

#[test]
fn c08_bare_quality_switch_alternate() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "decodebin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c08-bare", 90, || {
        let low = load("h264-low");
        let high = load("h264-high");
        plays_switches(
            Shape::Bare,
            &[(&low, &[0]), (&high, &[1]), (&low, &[2]), (&high, &[3])],
            3,
            "alternate",
        );
    });
}

// ---- C11: the end of the stream ----

#[test]
fn c11_eos_after_the_last_frame_and_a_replay_after_ended() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c11", 90, || {
        let source = RealSource::new();
        let stream = load("h264-aac");
        let id = source.add(&stream.mime);
        source.append_stream(id, &stream, &ChunkPlan::Whole);
        source.end_of_stream();
        let rig = Rig::new(source.feed(), options(Shape::PlayBin, false));
        rig.play();
        rig.wait_eos(LIMIT);
        {
            let stats = rig.video_stats();
            let stats = stats.lock().unwrap();
            assert!(
                (198..=202).contains(&stats.count()),
                "{} frames before EOS",
                stats.count()
            );
            assert!(
                stats.last_pts().unwrap() > 7_900_000_000,
                "the last frame is the last one"
            );
        }
        let first_run = rig.video_stats().lock().unwrap().count();
        // The element is at its end: the MediaSource is ended. Seeking back reopens it (the DOM calls
        // `seeking`), and the data that is there plays again to a second end.
        assert!(
            source.backend.seeking(0.0),
            "an ended source is open again after a seek"
        );
        rig.pump_bus();
        let eos_before = rig.bus_log.lock().unwrap().eos;
        assert!(rig.seek(0, true));
        // The stream was reopened by the seek; the page appends nothing more and ends it again.
        source.end_of_stream();
        rig.wait_until(LIMIT, "the second end of the stream", |rig| {
            rig.bus_log.lock().unwrap().eos > eos_before
        });
        let second = rig.video_stats().lock().unwrap().count() - first_run;
        eprintln!("C11 replay: {second} frames");
        assert!(
            (198..=202).contains(&second),
            "the replay shows {second} frames"
        );
    });
}

// ---- C15: a dash.js-shaped loop on a tiled 60 s stream ----

#[test]
fn c15_dash_loop_on_a_tiled_stream_does_not_stall() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c15", 300, || {
        let copies = 8; // 64 s
        let video = mercury_mse_testkit::tile(&load("h264-low"), copies).expect("tile video");
        let audio = mercury_mse_testkit::tile(&load("aac"), copies).expect("tile audio");
        let source = Arc::new(RealSource::new());
        let video_id = source.add(&video.mime);
        let audio_id = source.add(&audio.mime);
        source.append_init(video_id, &video);
        source.append_init(audio_id, &audio);
        let rig = Arc::new(Rig::new(source.feed(), options(Shape::PlayBin, false)));
        rig.play();
        let segments = video.media.len().max(audio.media.len());
        let appended = {
            let (source, rig) = (Arc::clone(&source), Arc::clone(&rig));
            std::thread::spawn(move || {
                for index in 0..segments {
                    for (id, stream) in [(video_id, &video), (audio_id, &audio)] {
                        if let Some(segment) = stream.media.get(index) {
                            source.append(id, &segment.bytes);
                        }
                    }
                    // Like dash.js: drop what is more than 4 s behind the position.
                    if let Some(position) = rig.position() {
                        let behind = position.nseconds() as f64 / 1e9 - 4.0;
                        if behind > 1.0 {
                            for id in [video_id, audio_id] {
                                let _ = source.backend.remove(id, 0.0, behind);
                            }
                        }
                    }
                }
                source.end_of_stream();
            })
        };
        rig.wait_eos(Duration::from_secs(120));
        appended.join().unwrap();
        let stats = rig.video_stats();
        let stats = stats.lock().unwrap();
        let mut worst = Duration::ZERO;
        for pair in stats.arrivals.windows(2) {
            worst = worst.max(pair[1].at.saturating_sub(pair[0].at));
        }
        eprintln!(
            "C15: {} frames, longest gap between frames {worst:?}",
            stats.count()
        );
        assert!(stats.count() >= 1590, "{} frames of 1600", stats.count());
        assert!(worst < Duration::from_millis(200), "a stall of {worst:?}");
    });
}

/// What the real feed gives for a B-frame stream: the decode and presentation times of the first frames
/// (GStreamer's `DTS` is unsigned, so a negative decode time has to be dealt with by the element).
#[test]
fn c02_b_frame_timestamps_of_the_feed() {
    use mercury_mse::{FeedItem, TrackKind};
    common::env::init();
    let source = RealSource::new();
    let stream = load("h264-bframes");
    let id = source.add(&stream.mime);
    source.append_stream(id, &stream, &ChunkPlan::Whole);
    source.end_of_stream();
    let feed = source.feed();
    let mut frames = Vec::new();
    loop {
        let batch = feed.pull(TrackKind::Video, 64);
        for item in batch.items {
            if let FeedItem::Frame { frame, .. } = item {
                frames.push((frame.pts, frame.dts, frame.keyframe));
            }
        }
        if batch.state != mercury_mse::FeedState::More {
            break;
        }
    }
    eprintln!(
        "B-frames from the feed: {} frames; first ten (pts, dts, key): {:?}",
        frames.len(),
        &frames[..10.min(frames.len())]
    );
    let negative = frames.iter().filter(|f| f.1 < 0).count();
    eprintln!(
        "B-frames from the feed: {negative} frames have a negative dts, min dts {:?}",
        frames.iter().map(|f| f.1).min()
    );
    assert_eq!(frames.len(), 200);
}

/// Seeks across a rendition switch: the feed starts again at a random access point of the other
/// rendition, and the caps have to follow (the element does not know which configuration the pipeline
/// holds after a flush, the feed says it with a `Config` item).
#[test]
fn c08_seeks_across_a_rendition_switch_follow_the_caps() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("c08-seeks", 90, || {
        let source = RealSource::new();
        let low = load("h264-low");
        let high = load("h264-high");
        let audio = load("aac");
        let audio_id = source.add(&audio.mime);
        source.append_stream(audio_id, &audio, &ChunkPlan::Whole);
        let video_id = source.add(&low.mime);
        append_switches(&source, video_id, &[(&low, &[0, 1]), (&high, &[2, 3])]);
        source.end_of_stream();
        let rig = Rig::new(source.feed(), options(Shape::PlayBin, true));
        rig.play();
        rig.wait_until(LIMIT, "frames", |rig| {
            rig.video_stats().lock().unwrap().count() >= 25
        });
        let width_after_flush = |rig: &Rig, flush: usize| -> Option<String> {
            let stats = rig.video_stats();
            let stats = stats.lock().unwrap();
            let mark = *stats.flush_marks.get(flush)?;
            let arrival = stats.arrivals.get(mark)?;
            Some(stats.caps[arrival.caps].clone())
        };
        for (flush, (target, width)) in [(6 * SEC, 640), (SEC, 320), (5 * SEC, 640), (2 * SEC, 320)]
            .into_iter()
            .enumerate()
        {
            let outcome = seek_and_measure(&rig, target, true, Duration::from_secs(10));
            let caps = width_after_flush(&rig, flush).expect("a frame after the flush");
            eprintln!(
                "C08 seek to {} s: {:?} {caps}",
                target / SEC,
                outcome.first_video_colour
            );
            assert!(
                caps.contains(&format!("width=(int){width}")),
                "after a seek to {} s the frames are {width} wide: {caps}",
                target / SEC
            );
            assert_eq!(
                outcome.first_video_colour,
                Some(if target >= 4 * SEC {
                    Colour::Blue
                } else {
                    Colour::Red
                })
            );
        }
    });
}
