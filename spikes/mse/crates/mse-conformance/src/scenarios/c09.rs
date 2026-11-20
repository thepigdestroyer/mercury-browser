// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C09 (P1): `changeType` mid-stream, what YouTube does when it changes codec. One video source buffer is
//! created for one codec and switched with `change_type` to the other (MP4/H.264 <-> WebM/VP9), each switch
//! followed by the new type's init segment and its media segments; video only (one source buffer: two deadlock GstMSE 1.28.9, see C02).
//!
//! * `h264-to-vp9`: H.264 segments 0-1, then VP9 segments 2-3.
//! * `vp9-h264-alternating`: VP9, H.264, VP9, H.264 (three `change_type` calls, "and back").
//! * `h264-low-to-high-mime`: H.264 `avc1.42C00C` to `avc1.42C01E` inside MP4 (does `change_type` take any other
//!   type string, or only the same one?).
//!
//! Pass when playback continues through the switch: 200 +- 2 frames, right colours, no error event, the end of
//! the stream arrives, `buffered` is [0, 8 s]. When GstMSE refuses `change_type`, the error class and message
//! are recorded as a finding: that is what decides how the DOM must handle `changeType`.

use std::time::Duration;

use mse_rig::backend::EosError;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_segments::{ChunkPlan, SegmentedStream};

use crate::probes::keep_going;
use crate::scenarios::c08::judge_switch;
use crate::support::check_buffered;
use crate::support::{
    LOOSE_TOL_MS, NOMINAL_MS, Res, Scn, append_inits, append_segment, diagnostics, fast_config,
    fixture, new_rig, one_feed, run_scenario, shutdown, wait_eos,
};

pub const TITLE: &str = "changeType mid-stream: H.264 to VP9, and alternating back";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C09", TITLE, true, body)
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let h264 = fixture("h264-low")?;
    let h264_high = fixture("h264-high")?;
    let vp9 = fixture("vp9-low")?;
    let outcome = change_run(ctx, scn, "h264-to-vp9", &[&h264, &h264, &vp9, &vp9]);
    keep_going(scn, "h264-to-vp9", outcome)?;
    let outcome = change_run(
        ctx,
        scn,
        "vp9-h264-alternating",
        &[&vp9, &h264, &vp9, &h264],
    );
    keep_going(scn, "vp9-h264-alternating", outcome)?;
    // Same container, other codec string (profile/level): does `change_type` accept any change of the type string?
    let outcome = change_run(
        ctx,
        scn,
        "h264-low-to-high-mime",
        &[&h264, &h264, &h264_high, &h264_high],
    );
    keep_going(scn, "h264-low-to-high-mime", outcome)?;
    Ok(())
}

/// Plays `segments[i]`'s segment i for i in 0..4 through one video source buffer, calling `change_type` and
/// appending the new init segment whenever the type changes.
fn change_run(ctx: &Ctx, scn: &mut Scn, label: &str, segments: &[&SegmentedStream; 4]) -> Res<()> {
    let rig = new_rig(scn, fast_config(ctx))?;
    let first = segments[0];
    let [video] = one_feed(&rig, first)?;
    append_inits(&rig, &[video])?;
    let mut current = first.mime.clone();
    let mut change_calls = 0;
    for (index, stream) in segments.iter().enumerate() {
        if stream.mime != current {
            change_calls += 1;
            if let Err(error) = rig.backend.change_type(video.id, &stream.mime) {
                scn.metric(&format!("{label}.change_type_error"), format!("{error:?}"));
                scn.finding(format!(
                    "{label}: change_type({:?}) before segment {index} was refused: {error:?}; the DOM cannot rely on changeType (it would have to recreate the source buffer or throw NotSupportedError)",
                    stream.mime
                ));
                shutdown(scn, &format!("c09-{label}"), rig);
                return Ok(());
            }
            append_segment(&rig, video.id, &stream.init, &ChunkPlan::Whole)?;
            current = stream.mime.clone();
        }
        append_segment(&rig, video.id, &stream.media[index], &ChunkPlan::Whole)?;
    }
    scn.metric(&format!("{label}.change_type_calls"), change_calls);
    check_buffered(
        scn,
        &rig,
        &format!("{label}.buffered"),
        video.id,
        &[(0, NOMINAL_MS)],
        LOOSE_TOL_MS,
    )?;
    rig.backend.end_of_stream(EosError::None)?;
    rig.play()?;
    let eos = wait_eos(&rig, Duration::from_secs(20));
    scn.check(
        &format!("{label}.eos"),
        eos.reached,
        format!(
            "the stream did not end after the codec switches: {}",
            diagnostics(&rig)
        ),
    );
    judge_switch(scn, label, &rig, &[video.id], None);
    shutdown(scn, &format!("c09-{label}"), rig);
    Ok(())
}
