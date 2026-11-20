// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C06 (P0): seek inside the buffered range. Everything is appended (the stream is not ended, as for a
//! player that is still fetching); play 1 s; seek to 5.0 s (blue half), back to 0.5 s (red half), then to
//! 7.9 s (inside the last GOP). Real time (`sync=true`), so the latency to the first frame after each seek is
//! a latency a user would see.
//!
//! Per seek: the first frame of the new position reaches the sink within 500 ms; the picture is the right
//! colour; `PlayEvent::SeekDone` arrives (PlayBin shape); playback does not stall afterwards (at least 15
//! frames in the 1.2 s that follow); audio and video resume together (their last timestamps within 300 ms of
//! each other); the position is where the seek put it. Flush counts at the sinks are recorded. At the end
//! `end_of_stream` must end the stream (no stall left over from the seeks).

use std::time::Duration;

use mse_rig::backend::EosError;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::Shape;

use crate::probes::{SeekObs, seek_and_observe};
use crate::support::{
    Res, Scn, append_inits, append_media, diagnostics, fixture, fmt_opt, judge_colours, ms_of,
    new_rig, nominal_colour, one_feed, run_scenario, shutdown, t_ms, timed_config, wait_eos,
};

pub const TITLE: &str = "seek inside the buffered range: 5.0 s, back to 0.5 s, 7.9 s";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C06", TITLE, true, body)
}

struct SeekSpec {
    name: &'static str,
    target_ms: u64,
    /// Playback time after the first frame, in ms.
    play_ms: u64,
    /// Whether enough media is left after the target to check for steady playback (>= 15 frames).
    steady: bool,
}

const SEEKS: [SeekSpec; 3] = [
    SeekSpec {
        name: "to-blue-5.0s",
        target_ms: 5_000,
        play_ms: 1_200,
        steady: true,
    },
    SeekSpec {
        name: "back-to-red-0.5s",
        target_ms: 500,
        play_ms: 1_200,
        steady: true,
    },
    SeekSpec {
        name: "to-last-gop-7.9s",
        target_ms: 7_900,
        play_ms: 600,
        steady: false,
    },
];

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    // One source buffer holding audio and video (two source buffers deadlock GstMSE 1.28.9: see C02).
    let muxed = fixture("h264-aac")?;
    let rig = new_rig(scn, timed_config(ctx))?;
    let feeds = one_feed(&rig, &muxed)?;
    append_inits(&rig, &feeds)?;
    append_media(&rig, &feeds, 0..4)?;
    rig.play()?;
    let playing = rig.wait_until(Duration::from_secs(8), |r| {
        r.position().is_some_and(|p| p >= t_ms(1_000))
    });
    if !scn.check(
        "plays-before-seek",
        playing,
        format!(
            "the position did not reach 1.0 s within 8 s of play() with all data buffered; {}",
            diagnostics(&rig)
        ),
    ) {
        shutdown(scn, "c06", rig);
        return Ok(());
    }

    for spec in &SEEKS {
        let target = t_ms(spec.target_ms);
        let obs = seek_and_observe(
            &rig,
            target,
            Duration::from_secs(2),
            Duration::from_millis(2_500),
            Duration::from_secs(3),
            Duration::from_millis(spec.play_ms),
        )?;
        judge_seek(ctx, scn, spec, &obs);
    }

    rig.backend.end_of_stream(EosError::None)?;
    let eos = wait_eos(&rig, Duration::from_secs(6));
    scn.check(
        "eos-after-seeks",
        eos.reached,
        format!(
            "the stream did not end within 6 s of end_of_stream after the seeks; {}",
            diagnostics(&rig)
        ),
    );
    shutdown(scn, "c06", rig);
    Ok(())
}

fn judge_seek(ctx: &Ctx, scn: &mut Scn, spec: &SeekSpec, obs: &SeekObs) {
    let name = spec.name;
    scn.metric(&format!("seek.{name}.first_frame_ms"), obs.first_frame_ms)
        .metric(&format!("seek.{name}.video_frames"), obs.video.count)
        .metric(&format!("seek.{name}.audio_buffers"), obs.audio.count)
        .metric(&format!("seek.{name}.video_flushes"), obs.video.flushes)
        .metric(&format!("seek.{name}.audio_flushes"), obs.audio.flushes)
        .metric(&format!("seek.{name}.seek_dones"), obs.new_seek_dones)
        .metric(
            &format!("seek.{name}.video_first_pts_ms"),
            obs.video.first_pts.map(ms_of),
        )
        .metric(
            &format!("seek.{name}.audio_first_pts_ms"),
            obs.audio.first_pts.map(ms_of),
        )
        .metric(&format!("seek.{name}.position_ms"), obs.position.map(ms_of));

    scn.check(
        &format!("seek.{name}.first-frame"),
        obs.first_frame_ms.is_some_and(|ms| ms <= 500),
        format!(
            "first frame at the new position: {} (limit 500 ms, waited up to 3 s); frames since the seek: {}, last video pts {}",
            crate::support::fmt_ms(obs.first_frame_ms),
            obs.video.count,
            fmt_opt(obs.video.last_pts)
        ),
    );
    if obs.first_frame_ms.is_none() {
        return;
    }

    let fresh = obs.fresh_colour();
    let verdict = judge_colours(&fresh, nominal_colour, t_ms(40));
    scn.check(
        &format!("seek.{name}.colour"),
        !fresh.is_empty() && verdict.ok(),
        if fresh.is_empty() {
            "no colour sample was recorded for the frames after the seek".to_owned()
        } else {
            format!("{}: {}", verdict.summary(), verdict.wrong.join("; "))
        },
    );

    if ctx.shape == Shape::PlayBin {
        scn.check(
            &format!("seek.{name}.seek-done"),
            obs.new_seek_dones >= 1,
            "no PlayEvent::SeekDone followed the seek".to_owned(),
        );
    } else {
        scn.note_once("Bare shape: there is no GstPlay, so SeekDone is not checked");
    }

    let position_ok = obs.position.is_some_and(|p| {
        ms_of(p) + 100 >= i64::try_from(spec.target_ms).unwrap_or(0)
            && ms_of(p) <= i64::try_from(spec.target_ms + spec.play_ms + 1_500).unwrap_or(i64::MAX)
    });
    scn.check(
        &format!("seek.{name}.position"),
        position_ok,
        format!(
            "position {} after seeking to {} ms and playing {} ms",
            fmt_opt(obs.position),
            spec.target_ms,
            spec.play_ms
        ),
    );

    if spec.steady {
        scn.check(
            &format!("seek.{name}.no-stall"),
            obs.video.count >= 15,
            format!(
                "only {} video frames in the {} ms after the first frame (a steady 25 fps gives about 30)",
                obs.video.count, spec.play_ms
            ),
        );
        let together = match (obs.audio.last_pts, obs.video.last_pts) {
            (Some(audio), Some(video)) => {
                let gap = (ms_of(audio) - ms_of(video)).abs();
                (
                    gap <= 300,
                    format!(
                        "last audio pts {} and last video pts {} differ by {gap} ms (limit 300 ms)",
                        fmt_opt(Some(audio)),
                        fmt_opt(Some(video))
                    ),
                )
            }
            _ => (
                false,
                format!(
                    "audio did not resume with the video: {} audio buffers, last audio pts {}",
                    obs.audio.count,
                    fmt_opt(obs.audio.last_pts)
                ),
            ),
        };
        scn.check(&format!("seek.{name}.av-together"), together.0, together.1);
    } else {
        scn.check(
            &format!("seek.{name}.some-frames"),
            obs.video.count >= 1,
            "no frame after seeking into the last GOP".to_owned(),
        );
    }
}
