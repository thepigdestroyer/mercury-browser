// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! D18: the Opus pre-skip. The `OpusHead` of the track says how many samples at 48 kHz the decoder
//! produces before the audio starts (312 for libopus at 48 kHz); a player has to drop them, or the audio
//! starts 6.5 ms late against the video and the stream is that much longer than the packets say.
//!
//! The acceptance is a sample count, not a listening test: what comes out of the decoder is the packets'
//! samples less the pre-skip, and the first buffer is stamped 0. `opusdec` does not read the pre-skip from
//! the caps (it parses the `OpusHead` only to log it); it clips by a `GstAudioClippingMeta` on the
//! buffer, which is what the source element puts on the first buffer after the decoder starts.

mod common;

use std::time::Duration;

use common::frames;
use common::real::{RealSource, load};
use common::rig::{Options, Rig, Shape, watchdog};
use mercury_mse::{CodecConfig, FrameFeed, TrackKind};
use mercury_mse_testkit::ChunkPlan;

/// The pre-skip of the track the real parser read from the fixture's `OpusHead`.
fn pre_skip_of(feed: &dyn FrameFeed) -> u32 {
    let tracks = feed.tracks();
    let audio = tracks
        .iter()
        .find(|track| track.kind == TrackKind::Audio)
        .expect("an audio track");
    let CodecConfig::Opus { head } = &audio.codec else {
        panic!("not an Opus track: {:?}", audio.codec);
    };
    u32::from(u16::from_le_bytes([head[10], head[11]]))
}

fn decode_opus(shape: Shape) -> (u32, usize, f64, Option<u64>) {
    let source = RealSource::new();
    let stream = load("opus");
    let id = source.add(&stream.mime);
    source.append_stream(id, &stream, &ChunkPlan::Whole);
    source.end_of_stream();
    let pre_skip = pre_skip_of(&*source.feed());
    let rig = Rig::new(
        source.feed(),
        Options {
            shape,
            video: false,
            sync: false,
            colour: false,
            ..Options::default()
        },
    );
    rig.play();
    rig.wait_eos(Duration::from_secs(30));
    let audio = rig.audio_stats();
    let audio = audio.lock().unwrap();
    let first_pts = audio.arrivals.first().and_then(|a| a.pts);
    let samples = (audio.total_seconds() * 48_000.0).round() as usize;
    eprintln!(
        "Opus {shape:?}: pre-skip {pre_skip}, {} buffers, {samples} samples, first pts {first_pts:?}, caps {:?}",
        audio.count(),
        audio.caps
    );
    (pre_skip, samples, audio.total_seconds(), first_pts)
}

fn packet_samples() -> usize {
    let track = frames::track("opus", TrackKind::Audio);
    // The demuxer's durations are not the packets' (it shortens one packet by the codec delay), so the
    // count comes from each packet's TOC byte: one 20 ms frame (frame-count code 0, 960 samples).
    const SAMPLES: [usize; 32] = [
        480, 960, 1920, 2880, 480, 960, 1920, 2880, 480, 960, 1920, 2880, 480, 960, 480, 960, 120,
        240, 480, 960, 120, 240, 480, 960, 120, 240, 480, 960, 120, 240, 480, 960,
    ];
    track
        .frames
        .iter()
        .map(|frame| {
            let toc = frame.data[0];
            assert_eq!(toc & 3, 0, "one frame per packet in the fixture");
            SAMPLES[usize::from(toc >> 3)]
        })
        .sum()
}

#[test]
fn the_decoded_opus_stream_is_the_packets_less_the_pre_skip() {
    require!("opusdec", "playbin3", "appsrc");
    watchdog("opus-preskip", 60, || {
        let (pre_skip, samples, _, first_pts) = decode_opus(Shape::PlayBin);
        let packets = packet_samples();
        assert!(pre_skip > 0, "the fixture has a pre-skip to apply");
        // The brief's bound, then the exact count: 960 samples are 20 ms and the pre-skip is only 312,
        // so the bound alone could not tell a stream that kept it from one that dropped it.
        assert!(
            samples.abs_diff(8 * 48_000 - pre_skip as usize) <= 960,
            "{samples} samples"
        );
        assert_eq!(samples, packets - pre_skip as usize, "samples out");
        assert_eq!(first_pts, Some(0), "the first buffer is stamped 0");
    });
}

#[test]
fn the_bare_shape_applies_the_pre_skip_too() {
    require!("opusdec", "decodebin3", "appsrc");
    watchdog("opus-preskip-bare", 60, || {
        let (pre_skip, samples, _, first_pts) = decode_opus(Shape::Bare);
        assert_eq!(samples, packet_samples() - pre_skip as usize, "samples out");
        assert_eq!(first_pts, Some(0));
    });
}
