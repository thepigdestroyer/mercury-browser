// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! D7: each new codec plays through the real backend and `MseSrc` to the end of the stream with software
//! decoders: `isTypeSupported` says yes, every frame comes out (video) or the whole duration does
//! (audio), and a seek in real time restarts at the target. The fixtures are item 3's (`hevc-low`,
//! `ac3`, `eac3`, `flac`, `flac-webm`, `mp3`, `vorbis`); the parsers are `mercury-mse`'s, so these tests
//! cover the whole path from appended bytes to decoded samples.
//!
//! The decoders are the ones Mercury's plugin list gives (libav for HEVC, AC-3, E-AC-3, FLAC and MP3,
//! `vorbisdec` for Vorbis): see `tests/common/env.rs`.

mod common;

use std::time::Duration;

use common::real::{RealSource, load};
use common::rig::{Options, Rig, Shape, watchdog};
use common::scenario::seek_and_measure;
use common::sinks::Colour;
use mercury_mse::{FeedItem, FeedState, TrackKind, is_type_supported};
use mercury_mse_gst::GstTypeSupport;
use mercury_mse_testkit::ChunkPlan;

const LIMIT: Duration = Duration::from_secs(40);

/// `true` when one of `names` is in the registry (a skip, or a failure with the bundled GStreamer, when
/// none is).
fn have_any(names: &[&str]) -> bool {
    common::env::init();
    if names
        .iter()
        .any(|name| gstreamer::ElementFactory::find(name).is_some())
    {
        return true;
    }
    common::env::skip_or_fail(names[0]);
    false
}

/// An open MediaSource with `fixture` appended whole and ended.
fn source_of(fixture: &str) -> RealSource {
    let source = RealSource::new();
    let stream = load(fixture);
    let id = source.add(&stream.mime);
    source.append_stream(id, &stream, &ChunkPlan::Whole);
    source.end_of_stream();
    source
}

/// What the parser and the track buffer hold for `fixture`: the number of frames and their total
/// duration in seconds, read by draining the feed.
fn stored(fixture: &str, kind: TrackKind) -> (usize, f64) {
    let source = source_of(fixture);
    let feed = source.feed();
    let (mut frames, mut nanos) = (0usize, 0i64);
    loop {
        let batch = feed.pull(kind, 64);
        for item in batch.items {
            if let FeedItem::Frame { frame, .. } = item {
                frames += 1;
                nanos += frame.duration;
            }
        }
        if batch.state != FeedState::More {
            assert_eq!(batch.state, FeedState::Ended, "{fixture} did not drain");
            break;
        }
    }
    (frames, nanos as f64 / 1e9)
}

fn options(kind: TrackKind, sync: bool) -> Options {
    Options {
        shape: Shape::PlayBin,
        sync,
        video: kind == TrackKind::Video,
        audio: kind == TrackKind::Audio,
        colour: kind == TrackKind::Video,
    }
}

/// Plays `fixture` to the end as fast as the decoders go.
fn plays_to_the_end(fixture: &'static str, kind: TrackKind) {
    let stream = load(fixture);
    assert!(
        is_type_supported(&stream.mime, &GstTypeSupport),
        "isTypeSupported({:?}) is false",
        stream.mime
    );
    let (expected_frames, expected_seconds) = stored(fixture, kind);
    assert!(expected_frames > 0, "{fixture}: no frames were stored");
    let source = source_of(fixture);
    let rig = Rig::new(source.feed(), options(kind, false));
    rig.play();
    rig.wait_eos(LIMIT);
    match kind {
        TrackKind::Video => {
            let video = rig.video_stats();
            let video = video.lock().unwrap();
            eprintln!(
                "{fixture}: {} frames out of {expected_frames}, caps {:?}",
                video.count(),
                video.caps
            );
            assert_eq!(video.count(), expected_frames, "{fixture} frames");
            let colours = video.colours_since(0);
            assert_eq!(colours.first().map(|c| c.1), Some(Colour::Red));
            assert_eq!(colours.last().map(|c| c.1), Some(Colour::Blue));
        }
        _ => {
            let audio = rig.audio_stats();
            let audio = audio.lock().unwrap();
            eprintln!(
                "{fixture}: {} buffers, {:.3} s out of {expected_seconds:.3} s, caps {:?}",
                audio.count(),
                audio.total_seconds(),
                audio.caps
            );
            assert!(audio.count() > 0, "{fixture}: no audio came out");
            // Decoders drop a little at the start (priming) and the stored durations round: 0.15 s.
            assert!(
                (audio.total_seconds() - expected_seconds).abs() < 0.15,
                "{fixture}: {:.3} s out, {expected_seconds:.3} s in",
                audio.total_seconds()
            );
            assert!(audio.total_seconds() > 7.5, "{fixture}: too short");
        }
    }
}

/// Plays in real time, seeks to 5 s inside the buffer and checks what restarts there.
fn seeks(fixture: &'static str, kind: TrackKind) {
    let source = source_of(fixture);
    let rig = Rig::new(source.feed(), options(kind, true));
    rig.play();
    rig.wait_until(LIMIT, "the first second", |rig| match kind {
        TrackKind::Video => rig.video_stats().lock().unwrap().count() >= 25,
        _ => rig.audio_stats().lock().unwrap().total_seconds() >= 0.5,
    });
    let target = 5_000_000_000u64;
    let outcome = seek_and_measure(&rig, target, true, Duration::from_secs(10));
    eprintln!("{fixture} seek to 5 s: {outcome:?}");
    assert_eq!(outcome.async_done, 1, "{outcome:?}");
    match kind {
        TrackKind::Video => {
            assert_eq!(
                outcome.first_video_colour,
                Some(Colour::Blue),
                "{outcome:?}"
            );
            assert!(
                outcome.first_video_pts.unwrap() + 100_000_000 >= target,
                "{outcome:?}"
            );
        }
        _ => {
            let pts = outcome.first_audio_pts.unwrap();
            // Audio frames are 20 to 100 ms long; the first buffer after an accurate seek starts at or
            // just before the target.
            assert!(
                pts + 250_000_000 >= target && pts < target + 250_000_000,
                "{outcome:?}"
            );
        }
    }
    rig.wait_eos(LIMIT);
}

macro_rules! codec_tests {
    ($name:ident, $fixture:expr, $kind:expr, [$($element:expr),+ $(,)?]) => {
        mod $name {
            use super::*;

            #[test]
            fn plays_to_the_end() {
                if !have_any(&[$($element),+]) {
                    return;
                }
                require!("playbin3", "appsrc");
                watchdog(concat!(stringify!($name), "-play"), 90, || {
                    super::plays_to_the_end($fixture, $kind)
                });
            }

            #[test]
            fn seeks_inside_the_buffer() {
                if !have_any(&[$($element),+]) {
                    return;
                }
                require!("playbin3", "appsrc");
                watchdog(concat!(stringify!($name), "-seek"), 90, || {
                    super::seeks($fixture, $kind)
                });
            }
        }
    };
}

codec_tests!(hevc, "hevc-low", TrackKind::Video, ["avdec_h265"]);
codec_tests!(ac3, "ac3", TrackKind::Audio, ["avdec_ac3"]);
codec_tests!(eac3, "eac3", TrackKind::Audio, ["avdec_eac3"]);
codec_tests!(
    flac_in_mp4,
    "flac",
    TrackKind::Audio,
    ["avdec_flac", "flacdec"]
);
codec_tests!(
    flac_in_webm,
    "flac-webm",
    TrackKind::Audio,
    ["avdec_flac", "flacdec"]
);
codec_tests!(
    mp3,
    "mp3",
    TrackKind::Audio,
    ["avdec_mp3float", "avdec_mp3", "mpg123audiodec"]
);
codec_tests!(vorbis, "vorbis", TrackKind::Audio, ["vorbisdec"]);
