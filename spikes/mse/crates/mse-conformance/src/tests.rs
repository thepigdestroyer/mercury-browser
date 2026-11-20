// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! End-to-end tests: scenarios C02 (split A/V) and C03 (muxed) run for real against the bundled GStreamer, in
//! both pipeline shapes, so that MED-1 can reuse them as regression tests of the `MediaSource` backend
//! (`c02::run_with_attempts` and `c03::run` take a `Ctx` and need nothing else).
//!
//! What they assert today is what GstMSE 1.28.9 does (see `.superpowers/research/R17-mse-spike.md`): split A/V
//! (two source buffers) deadlocks, so the default C02 test checks that the scenario reaches a verdict without a
//! harness error and that each half played alone is fine; the strict test (`c02_split_av_passes`) is
//! `#[ignore]`d until a backend workaround exists. A muxed stream (one source buffer) plays all 200 frames.
//!
//! They need the unpacked framework (`make gstreamer-macos`) and run through
//! `spikes/mse/run.sh test -p mse-conformance`, which sets the environment (`-- --ignored` for the strict
//! one). GStreamer is initialised once per process; the tests take a lock so that only one rig exists at a
//! time. The private registry and scratch directory are under `$TMPDIR/opencode/lanes/sp-mse/` (removed with
//! the rest of that folder when the lane ends).

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use mse_rig::env::{self, GstInfo, InitOptions, PluginMode};
use mse_rig::harness::Ctx;
use mse_rig::report::{ScenarioResult, Status};
use mse_rig::rig::Shape;

use crate::scenarios::{c02, c03};

static GST: OnceLock<GstInfo> = OnceLock::new();
static ONE_RIG_AT_A_TIME: Mutex<()> = Mutex::new(());

fn scratch_root() -> PathBuf {
    std::env::temp_dir()
        .join("opencode")
        .join("lanes")
        .join("sp-mse")
        .join(format!("conformance-test-{}", std::process::id()))
}

fn ctx(shape: Shape) -> (MutexGuard<'static, ()>, Ctx) {
    let guard = ONE_RIG_AT_A_TIME
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let root = scratch_root();
    let info = GST
        .get_or_init(|| {
            env::init(&InitOptions {
                prefix: None,
                plugin_mode: PluginMode::SystemPath,
                scratch_dir: root.join("gst"),
                glib_main_loop: false,
            })
            .expect(
                "GStreamer set-up failed: run through spikes/mse/run.sh, after `make gstreamer-macos`",
            )
        })
        .clone();
    let scratch = root.join("scenario");
    std::fs::create_dir_all(&scratch).expect("the scratch directory can be created");
    let ctx = Ctx {
        shape,
        plugin_mode: PluginMode::SystemPath,
        glib_main_loop: false,
        scratch,
        iterations: 1,
        seed: 1,
        info,
    };
    (guard, ctx)
}

fn describe(result: &ScenarioResult) -> String {
    format!(
        "{} {:?}: {}\nmetrics: {:#?}",
        result.id,
        result.status,
        result.notes.join("\n"),
        result.metrics
    )
}

fn check(result: &ScenarioResult, name: &str) -> Option<bool> {
    result
        .metrics
        .get(&format!("check.{name}"))
        .and_then(serde_json::Value::as_bool)
}

/// C02 reaches a verdict (never a harness `Error`) and the two controls, video alone and audio alone, play.
fn assert_c02_verdict_and_controls(shape: Shape) {
    let (_guard, ctx) = ctx(shape);
    let result = c02::run_with_attempts(&ctx, 1);
    assert!(
        matches!(result.status, Status::Pass | Status::Fail),
        "{}",
        describe(&result)
    );
    for control in ["c02.control-video-only", "c02.control-audio-only"] {
        assert_eq!(
            check(&result, &format!("{control}.tracks-exposed")),
            Some(true),
            "{control}: {}",
            describe(&result)
        );
        assert_eq!(
            check(&result, &format!("{control}.eos")),
            Some(true),
            "{control}: {}",
            describe(&result)
        );
    }
    assert_eq!(
        check(&result, "c02.control-video-only.video-frames"),
        Some(true),
        "{}",
        describe(&result)
    );
}

#[test]
fn c02_reaches_a_verdict_and_each_half_plays_alone_in_the_playbin_shape() {
    assert_c02_verdict_and_controls(Shape::PlayBin);
}

#[test]
fn c02_reaches_a_verdict_and_each_half_plays_alone_in_the_bare_shape() {
    assert_c02_verdict_and_controls(Shape::Bare);
}

/// The strict C02 for MED-1: two source buffers play to the end. Fails with GstMSE 1.28.9 (it deadlocks).
#[test]
#[ignore = "GstMSE 1.28.9 deadlocks with two source buffers; MED-1 enables this once its backend has a workaround"]
fn c02_split_av_passes() {
    let (_guard, ctx) = ctx(Shape::PlayBin);
    let result = c02::run_with_attempts(&ctx, 1);
    assert_eq!(result.status, Status::Pass, "{}", describe(&result));
}

/// One source buffer holding audio and video: the stream plays through and a clean run ends with a clean verdict
/// on the checks that GstMSE gets right (frames, order, no errors); the checks it gets wrong are listed here so
/// that a regression test notices when one of them starts to pass.
#[test]
fn c03_muxed_plays_all_frames_in_the_playbin_shape() {
    let (_guard, ctx) = ctx(Shape::PlayBin);
    let result = c03::run(&ctx);
    assert!(
        matches!(result.status, Status::Pass | Status::Fail),
        "{}",
        describe(&result)
    );
    for name in [
        "c03.tracks-exposed",
        "c03.eos",
        "c03.video-frames",
        "c03.audio-span",
        "c03.no-errors",
    ] {
        assert_eq!(
            check(&result, name),
            Some(true),
            "{name}: {}",
            describe(&result)
        );
    }
}
