// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C03 (P1): muxed audio and video in one source buffer (`h264-aac`), with C02's checks.

use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;

use crate::play::{self, PlaySpec};
use crate::support::{Chunking, Res, Scn, fast_config, fixture, run_scenario};

pub const TITLE: &str = "muxed A/V in one source buffer (h264-aac), played to the end";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C03", TITLE, true, body)
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let muxed = fixture("h264-aac")?;
    let spec = PlaySpec {
        label: "c03".to_owned(),
        streams: vec![(&muxed, Chunking::Whole)],
        config: fast_config(ctx),
        expect_video: true,
        expect_audio: true,
    };
    let result = play::full_play(scn, &spec)?;
    play::judge(scn, &spec, &result)
}
