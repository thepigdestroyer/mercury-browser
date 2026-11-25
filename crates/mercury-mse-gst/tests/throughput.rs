// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T05, T07 and T11: how fast the feed gets frames through a software decoder, which decoder
//! `decodebin3` picks by default, and A/V arrival offsets in real time. The numbers are recorded in the
//! output (and in the notes of the lane); the assertions only guard what must hold on any machine.

mod common;

use std::time::Duration;

use common::fakefeed::FakeFeed;
use common::real::{RealSource, load};
use common::rig::{Options, Rig, Shape, watchdog};
use gstreamer as gst;
use gstreamer::prelude::*;
use mercury_mse::TrackKind;
use mercury_mse_testkit::ChunkPlan;

const LIMIT: Duration = Duration::from_secs(60);

fn fps_of(rig: &Rig) -> (usize, f64) {
    let stats = rig.video_stats();
    let stats = stats.lock().unwrap();
    let first = stats.arrivals.first().map(|a| a.at).unwrap_or_default();
    let last = stats.arrivals.last().map(|a| a.at).unwrap_or_default();
    let seconds = last.saturating_sub(first).as_secs_f64().max(1e-6);
    (
        stats.count(),
        (stats.count().saturating_sub(1)) as f64 / seconds,
    )
}

/// T05: synthetic 1080p, about 6 Mbit/s, through the feed with `sync=false`.
fn throughput(codec: &str, decoder: &str, frames: u32) {
    let Some(track) = common::frames::synthetic_1080p(codec, frames) else {
        return;
    };
    if !common::env::have(&[decoder, "playbin3", "appsrc"]) {
        return;
    }
    let bytes: usize = track.frames.iter().map(|f| f.data.len()).sum();
    let feed = FakeFeed::new();
    feed.add_track(&track);
    feed.set_duration(Some(i64::from(frames) * 40_000_000));
    feed.set_initialized(true);
    feed.release_all();
    feed.end();
    let rig = Rig::new(
        feed,
        Options {
            audio: false,
            colour: false,
            sync: false,
            ..Options::default()
        },
    );
    rig.play();
    rig.wait_eos(Duration::from_secs(90));
    let (count, fps) = fps_of(&rig);
    eprintln!(
        "T05 {codec} 1080p: {count} of {} frames through {decoder}, {fps:.0} fps, {:.1} Mbit/s of stream",
        track.frames.len(),
        bytes as f64 * 8.0 / (f64::from(frames) * 0.04) / 1e6
    );
    assert_eq!(count, track.frames.len(), "every frame is decoded");
}

#[test]
fn t05_throughput_h264_1080p() {
    require!("openh264enc", "avdec_h264");
    watchdog("t05-h264", 240, || throughput("h264", "avdec_h264", 150));
}

#[test]
fn t05_throughput_vp9_1080p() {
    require!("vp9enc", "vp9dec");
    watchdog("t05-vp9", 360, || throughput("vp9", "vp9dec", 60));
}

/// T07: the decoder that serves each codec in this registry, and the other candidates by rank. In the
/// private test GStreamer there is no VideoToolbox (decision D12), so this records the software route;
/// the route of the shipped build (`vtdec_hw` for H.264 and VP9, `avdec_aac`, `dav1ddec`, `opusdec`) is
/// the spike's table, R17 section 8.2, and R16 P2 is unchanged.
#[test]
fn t07_default_decoder_per_codec() {
    require!("playbin3", "appsrc");
    watchdog("t07", 240, || {
        for (fixtures, codec, expect) in [
            (
                &["h264-low", "aac"][..],
                "H.264 + AAC",
                ["avdec_h264", "avdec_aac"],
            ),
            (
                &["vp9-low", "opus"][..],
                "VP9 + Opus",
                ["vp9dec", "opusdec"],
            ),
            (&["av1-low-av1c"][..], "AV1", ["dav1ddec", ""]),
        ] {
            if !common::env::have(
                &expect
                    .iter()
                    .filter(|e| !e.is_empty())
                    .copied()
                    .collect::<Vec<_>>(),
            ) {
                continue;
            }
            let source = RealSource::new();
            for name in fixtures {
                let stream = load(name);
                let id = source.add(&stream.mime);
                source.append_stream(id, &stream, &ChunkPlan::Whole);
            }
            source.end_of_stream();
            let rig = Rig::new(
                source.feed(),
                Options {
                    sync: false,
                    colour: false,
                    ..Options::default()
                },
            );
            rig.play();
            rig.wait_eos(LIMIT);
            // The decoders in the finished pipeline.
            let tree = rig.dump_tree();
            let decoders: Vec<String> = tree
                .lines()
                .filter_map(|line| {
                    let (_, rest) = line.split_once('[')?;
                    let (factory, _) = rest.split_once(']')?;
                    let element = gst::ElementFactory::find(factory)?;
                    element
                        .metadata("klass")
                        .is_some_and(|k| k.contains("Decoder"))
                        .then(|| factory.to_owned())
                })
                .collect();
            eprintln!("T07 {codec}: decoders in the pipeline {decoders:?}");
            for wanted in expect.iter().filter(|e| !e.is_empty()) {
                assert!(
                    decoders.iter().any(|d| d == wanted),
                    "{codec}: {wanted} not used: {decoders:?}"
                );
            }
        }
        // The candidates by rank, for the record.
        for (name, caps) in [
            (
                "H.264",
                "video/x-h264, stream-format=(string)avc, alignment=(string)au",
            ),
            ("VP9", "video/x-vp9"),
            (
                "AAC",
                "audio/mpeg, mpegversion=(int)4, stream-format=(string)raw",
            ),
            ("Opus", "audio/x-opus"),
            (
                "AV1",
                "video/x-av1, stream-format=(string)obu-stream, alignment=(string)tu",
            ),
        ] {
            let caps: gst::Caps = caps.parse().unwrap();
            let mut candidates: Vec<(i32, String)> = gst::ElementFactory::factories_with_type(
                gst::ElementFactoryType::DECODER,
                gst::Rank::MARGINAL,
            )
            .iter()
            .filter(|f| f.can_sink_any_caps(&caps))
            .map(|f| (i32::from(f.rank()), f.name().to_string()))
            .collect();
            candidates.sort_by(|a, b| b.cmp(a));
            eprintln!("T07 {name}: candidates by rank (software pinned first): {candidates:?}");
        }
    });
}

/// T11: real time, `h264-high` and `aac` in two source buffers. The video frame and the audio that cover
/// the same media time reach their sinks within 80 ms of each other.
#[test]
fn t11_clock_av_sync_in_real_time() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("t11", 120, || {
        let source = RealSource::new();
        for name in ["h264-high", "aac"] {
            let stream = load(name);
            let id = source.add(&stream.mime);
            source.append_stream(id, &stream, &ChunkPlan::Whole);
        }
        source.end_of_stream();
        let rig = Rig::new(
            source.feed(),
            Options {
                shape: Shape::PlayBin,
                sync: true,
                colour: false,
                ..Options::default()
            },
        );
        rig.play();
        rig.wait_eos(Duration::from_secs(30));
        let video = rig.video_stats();
        let audio = rig.audio_stats();
        let (video, audio) = (video.lock().unwrap(), audio.lock().unwrap());
        let mut worst = Duration::ZERO;
        let mut total = Duration::ZERO;
        let mut n = 0u32;
        for v in &video.arrivals {
            let Some(pts) = v.pts else { continue };
            let Some(a) = audio.arrivals.iter().find(|a| {
                a.pts
                    .is_some_and(|ap| ap <= pts && pts < ap + a.duration.unwrap_or(0))
            }) else {
                continue;
            };
            let offset = v.at.abs_diff(a.at);
            worst = worst.max(offset);
            total += offset;
            n += 1;
        }
        let dropped = rig.video.as_ref().and_then(|s| s.rendered_and_dropped());
        eprintln!(
            "T11 real time: {} video frames, {} audio buffers, A/V arrival offset max {worst:?}, mean {:?}, \
             sink (rendered, dropped) {dropped:?}, wall time {:?}",
            video.count(),
            audio.count(),
            total / n.max(1),
            rig.elapsed()
        );
        assert!(n > 100, "only {n} frames could be paired");
        assert!(worst < Duration::from_millis(80), "A/V offset {worst:?}");
        assert!((198..=202).contains(&video.count()));
    });
}

#[allow(dead_code)]
fn unused(_: TrackKind) {}
