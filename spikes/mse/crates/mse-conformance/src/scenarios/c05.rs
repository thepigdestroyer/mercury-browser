// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C05 (P1): split audio and video in WebM (`vp9-low` + `opus`), with C02's checks. WebM is YouTube's
//! main path; it exercises `matroskademux` and the VP9 and Opus decoders instead of `qtdemux`. Same layout
//! problem as C02 (two source buffers): see `play::split_av`.

use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;

use crate::play;
use crate::support::{Res, Scn, fixture, run_scenario};

pub const TITLE: &str = "split A/V in WebM (vp9-low + opus) appended whole, played to the end";

const ATTEMPTS: usize = 2;

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C05", TITLE, true, body)
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let video = fixture("vp9-low")?;
    let audio = fixture("opus")?;
    play::split_av(
        ctx,
        scn,
        "c05",
        &video,
        &audio,
        ATTEMPTS.max(ctx.iterations),
    )
}
