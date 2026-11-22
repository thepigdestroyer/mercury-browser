// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! One test per codec on the default decoder route (decision D12): Mercury's plugin list as shipped,
//! `libgstapplemedia` (VideoToolbox) loaded, no rank pinned, so `decodebin3` picks `vtdec_hw` where it can.
//! Each prints the decoders it used (number T07). A VideoToolbox session can in principle hang in an
//! unkillable state while the screen is locked (R17 section 2), so every play runs under the watchdog of
//! `rig.rs`; on this machine none did (decision D15, the measurements are in the notes of item 2). The
//! record-only `t07_av1_route_report` is `#[ignore]`d: run it with
//! `crates/mercury-mse-gst/test.sh test --release -p mercury-mse-gst --test default_route -- --ignored --nocapture`.

mod common;

use std::time::Duration;

use common::real::{RealSource, load};
use common::rig::{Options, Rig, Shape, watchdog};
use mercury_mse_testkit::ChunkPlan;

fn play_default_route(fixtures: &'static [&'static str], label: &'static str) {
    common::env::use_default_route();
    common::env::init();
    watchdog(label, 120, move || {
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
                shape: Shape::PlayBin,
                sync: false,
                colour: false,
                ..Options::default()
            },
        );
        rig.play();
        rig.wait_eos(Duration::from_secs(60));
        let decoders: Vec<String> = rig
            .dump_tree()
            .lines()
            .filter_map(|line| {
                line.split_once('[')?
                    .1
                    .split_once(']')
                    .map(|(f, _)| f.to_owned())
            })
            .filter(|factory| {
                factory.contains("dec")
                    && !factory.contains("decodebin")
                    && !factory.contains("uridecode")
            })
            .collect();
        let frames = rig.video_stats().lock().unwrap().count();
        eprintln!("T07 default route {label}: {frames} frames through {decoders:?}");
        assert!((198..=202).contains(&frames), "{frames} frames");
    });
}

/// What happened when `fixtures` were played on the current ranks: `EOS, N frames through [decoders]` or
/// the first line of the failure (an error message, a hang). Never panics: this is for the report.
fn route_outcome(fixtures: &'static [&'static str], label: &'static str) -> String {
    let attempt = std::panic::catch_unwind(|| {
        watchdog(label, 90, move || {
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
                    shape: Shape::PlayBin,
                    sync: false,
                    colour: false,
                    ..Options::default()
                },
            );
            rig.play();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rig.wait_eos(Duration::from_secs(60))
            }));
            let tree = rig.dump_tree();
            let decoders: Vec<&str> = tree
                .lines()
                .filter_map(|line| line.split_once('[')?.1.split_once(']').map(|(f, _)| f))
                .filter(|factory| {
                    factory.contains("dec")
                        && !factory.contains("decodebin")
                        && !factory.contains("uridecode")
                })
                .collect();
            let frames = rig.video_stats().lock().unwrap().count();
            let errors = rig.bus_log.lock().unwrap().errors.clone();
            match outcome {
                Ok(()) => format!("EOS, {frames} frames through {decoders:?}"),
                Err(_) => format!(
                    "no EOS, {frames} frames, decoders {decoders:?}, errors {:?}",
                    errors.first()
                ),
            }
        })
    });
    attempt.unwrap_or_else(|panic| {
        let text = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default();
        format!("failed: {}", text.lines().next().unwrap_or_default())
    })
}

/// D8: the numbers R17 9.1 puts in its AV1 table, for this machine, printed and never asserted: what the
/// registry says about AV1 decoders (VideoToolbox's `vtdec_hw` included, loaded because this is the
/// default-route binary), whether Mercury offers AV1 here (the D8 policy), what plays AV1 with no rank
/// changed, and what plays it with `dav1ddec` ranked first. Meant for the arm64 CI job and for the
/// owner on an M-series Mac: `... --test default_route -- --ignored --nocapture t07_av1_route_report`.
#[test]
#[ignore = "T07/D8: record-only report of the AV1 routes (VideoToolbox, default, dav1d); prints, never fails; run with --ignored --nocapture"]
fn t07_av1_route_report() {
    use gstreamer as gst;
    use gstreamer::prelude::*;
    use mercury_mse::{CodecId, TrackKind, TypeSupport};

    common::env::use_default_route();
    common::env::init();
    let caps = mercury_mse_gst::decoder_caps(CodecId::Av1).expect("AV1 decoder caps");
    let mut factories: Vec<String> =
        gst::ElementFactory::factories_with_type(gst::ElementFactoryType::DECODER, gst::Rank::NONE)
            .iter()
            .filter(|factory| factory.can_sink_any_caps(&caps))
            .map(|factory| format!("{} (rank {:?})", factory.name(), factory.rank()))
            .collect();
    factories.sort();
    eprintln!(
        "T07 AV1 report on {}/{}:",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    eprintln!("T07   AV1 decoders in the registry: {factories:?}");
    eprintln!(
        "T07   VideoToolbox decoder for AV1 (vtdec*): {}",
        factories.iter().any(|name| name.starts_with("vtdec"))
    );
    eprintln!(
        "T07   Mercury offers AV1 here (GstTypeSupport, D8 policy, MERCURY_MSE_AV1): {}",
        mercury_mse_gst::GstTypeSupport.has_decoder(TrackKind::Video, CodecId::Av1)
    );
    eprintln!(
        "T07   default route (ranks as shipped): {}",
        route_outcome(&["av1-low-av1c"], "av1-default")
    );
    if let Some(factory) = gst::ElementFactory::find("dav1ddec") {
        factory.set_rank(gst::Rank::PRIMARY + 100);
        eprintln!(
            "T07   dav1ddec ranked first: {}",
            route_outcome(&["av1-low-av1c"], "av1-dav1d")
        );
    } else {
        eprintln!("T07   dav1ddec ranked first: dav1ddec is not in the registry");
    }
}

// D15: these three ran with `--ignored` three times in a row on the Intel build machine, screen locked,
// no window and no network, a second each: `vtdec_hw` for H.264 and VP9, `dav1ddec` for AV1 (this Mac's
// VideoToolbox has no AV1), 200 frames each. So they are normal tests, except on Apple Silicon, where
// VideoToolbox's AV1 and VP9 behaviour is not measured (R17 9.1): there they stay ignored and the arm64
// job of `mse.yml` runs them record-only with `--ignored`. The default-route mode must be chosen before
// anything initialises GStreamer, hence the first call in each test.

#[test]
#[cfg_attr(
    all(target_arch = "aarch64", target_os = "macos"),
    ignore = "T07/D15: VideoToolbox on Apple Silicon is not measured; the arm64 job runs it record-only"
)]
fn t07_default_route_h264_and_aac() {
    common::env::use_default_route();
    require!("avdec_h264", "avdec_aac");
    play_default_route(&["h264-low", "aac"], "default-h264-aac");
}

#[test]
#[cfg_attr(
    all(target_arch = "aarch64", target_os = "macos"),
    ignore = "T07/D15: VideoToolbox on Apple Silicon is not measured; the arm64 job runs it record-only"
)]
fn t07_default_route_vp9_and_opus() {
    common::env::use_default_route();
    require!("vp9dec", "opusdec");
    play_default_route(&["vp9-low", "opus"], "default-vp9-opus");
}

#[test]
#[cfg_attr(
    all(target_arch = "aarch64", target_os = "macos"),
    ignore = "T07/D8/D15: AV1 through VideoToolbox fails on the hosted arm64 VM (R17 9.1, -12906); see t07_av1_route_report"
)]
fn t07_default_route_av1() {
    common::env::use_default_route();
    require!("dav1ddec");
    play_default_route(&["av1-low-av1c"], "default-av1");
}
