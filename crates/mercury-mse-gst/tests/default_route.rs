// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! One test per codec on the default decoder route (decision D12): Mercury's plugin list as shipped,
//! `libgstapplemedia` (VideoToolbox) loaded, no rank pinned, so `decodebin3` picks `vtdec_hw` where it can.
//! `#[ignore]`d: on this machine the screen is locked and anything that may open a VideoToolbox, Metal or
//! GPU session can hang in an unkillable state (R17 section 2; the lane brief, section 2). The owner runs
//! them with `crates/mercury-mse-gst/test.sh test --release -p mercury-mse-gst --test default_route -- --ignored`
//! once the screen is free; each prints the decoders it used. Number: T07, recorded in the lane report.

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

#[test]
#[ignore = "T07/D12: loads VideoToolbox and does not pin software decoders; the screen is locked on the build machine"]
fn t07_default_route_h264_and_aac() {
    play_default_route(&["h264-low", "aac"], "default-h264-aac");
}

#[test]
#[ignore = "T07/D12: loads VideoToolbox and does not pin software decoders; the screen is locked on the build machine"]
fn t07_default_route_vp9_and_opus() {
    play_default_route(&["vp9-low", "opus"], "default-vp9-opus");
}

#[test]
#[ignore = "T07/D12: loads VideoToolbox and does not pin software decoders; the screen is locked on the build machine"]
fn t07_default_route_av1() {
    play_default_route(&["av1-low-av1c"], "default-av1");
}
