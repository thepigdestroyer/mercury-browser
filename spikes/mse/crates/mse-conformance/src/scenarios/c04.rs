// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C04 (P1): arbitrary chunking. C02's play-through (muxed, one source buffer: see C02) with the bytes cut the way a network cuts them: 1 KiB pieces,
//! 64 KiB pieces, and the mid-cut plan (13 bytes = inside a box header, 4096 = inside a sample, the rest,
//! a 1-byte tail; the init segment cut 7 bytes in). Every plan gets a fresh rig and is judged by C02's checks;
//! and each must give the same frame count and `buffered` as the whole-segment baseline of this same run.

use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;

use crate::play::{self, PlayResult, PlaySpec};
use crate::probes::keep_going;
use crate::support::{
    Chunking, Res, Scn, compare_ranges, dur_ms, fast_config, fixture, ms_of, run_scenario,
};

pub const TITLE: &str = "arbitrary chunking: whole, 1 KiB, 64 KiB, mid-box/mid-sample cuts";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C04", TITLE, true, body)
}

/// The same frame count and the same `buffered` as the whole-segment baseline.
fn compare_to_baseline(scn: &mut Scn, label: &str, result: &PlayResult, base: &PlayResult) {
    scn.check(
        &format!("{label}.same-frames-as-whole"),
        result.video.count == base.video.count,
        format!(
            "{} video frames with this plan, {} with whole segments",
            result.video.count, base.video.count
        ),
    );
    let mut deviations = Vec::new();
    for ((id, ranges), (_, base_ranges)) in result.buffered.iter().zip(&base.buffered) {
        let expected: Vec<(i64, i64)> = base_ranges
            .iter()
            .map(|r| (ms_of(r.start), ms_of(r.end)))
            .collect();
        deviations.extend(compare_ranges(
            &format!("buffered of source buffer {id}"),
            ranges,
            &expected,
            2,
        ));
    }
    scn.check(
        &format!("{label}.same-buffered-as-whole"),
        deviations.is_empty(),
        deviations.join("; "),
    );
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    // One source buffer with audio and video: the split layout deadlocks GstMSE 1.28.9 (see C02), and chunking
    // is about the byte stream, not about how many source buffers there are.
    let muxed = fixture("h264-aac")?;
    let plans = [
        Chunking::Whole,
        Chunking::Fixed(1024),
        Chunking::Fixed(65_536),
        Chunking::MidCuts,
    ];
    let mut baseline: Option<PlayResult> = None;
    for plan in plans {
        let label = format!("plan.{}", plan.name());
        let spec = PlaySpec {
            label: label.clone(),
            streams: vec![(&muxed, plan)],
            config: fast_config(ctx),
            expect_video: true,
            expect_audio: true,
        };
        let outcome = play::full_play(scn, &spec);
        let Some(result) = keep_going(scn, &label, outcome)? else {
            continue;
        };
        play::judge(scn, &spec, &result)?;
        scn.note(format!(
            "{label}: {} appends, {} ms to append everything, {} ms from play() to the end",
            result.appends,
            dur_ms(result.append_time),
            dur_ms(result.play_time)
        ));
        match (&baseline, plan) {
            (Some(base), _) => compare_to_baseline(scn, &label, &result, base),
            (None, Chunking::Whole) => baseline = Some(result),
            (None, _) => {
                scn.note(format!(
                    "{label}: there is no whole-segment baseline to compare with (it failed)"
                ));
            }
        }
    }
    Ok(())
}
