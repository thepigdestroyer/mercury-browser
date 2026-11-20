// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C02 (P0): split audio and video in fragmented MP4, the way DASH delivers them. Two source buffers
//! (`h264-low`, `aac`); init, then the four media segments of each, appended whole; play to the end;
//! `end_of_stream`. The checks are the list in `play::judge` (tracks exposed, frames, audio, events,
//! `buffered`, duration, ready state, colours). GstMSE 1.28.9 deadlocks for this layout almost every time:
//! `play::split_av` tries it three times and then plays each half alone as a control.
//! MED-1 reuses this scenario as its first regression test (see `tests.rs`): `run_with_attempts` is the
//! entry point for a test that wants one attempt.

use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;

use crate::play;
use crate::support::{Res, Scn, fixture, run_scenario};

pub const TITLE: &str = "split A/V in fMP4 (h264-low + aac) appended whole, played to the end";

/// Attempts at the split layout (each deadlock costs about 20 s: 3 s to notice, 15 s of teardown).
const ATTEMPTS: usize = 3;

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_with_attempts(ctx, ATTEMPTS.max(ctx.iterations))
}

/// C02 with `attempts` tries at the split layout (then the two single-source-buffer controls).
pub fn run_with_attempts(ctx: &Ctx, attempts: usize) -> ScenarioResult {
    run_scenario(ctx, "C02", TITLE, true, |ctx, scn| body(ctx, scn, attempts))
}

fn body(ctx: &Ctx, scn: &mut Scn, attempts: usize) -> Res<()> {
    let video = fixture("h264-low")?;
    let audio = fixture("aac")?;
    play::split_av(ctx, scn, "c02", &video, &audio, attempts)
}
