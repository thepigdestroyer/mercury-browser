// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C07 (P0): seek to an unbuffered range, then append there. Three sub-runs, each in a fresh rig, real time, on
//! the muxed fixture `h264-aac` (one source buffer holding audio and video: two source buffers deadlock GstMSE
//! 1.28.9, see C02; its segments start at 0, 1984, 3989 and 5995 ms, the video in them at 0, 2, 4 and 6 s):
//!
//! * `unbuffered`: segments 0-1 appended (0-4 s), play, seek to 5.5 s (nothing buffered there). The element
//!   must not run on (no frame from the old data, the position holds), `ElementReadyState` must fall below
//!   `HaveFutureData`; then the segment that contains the target is appended (what a player does) and
//!   playback must resume within 1 s of that append's `UpdateEnd`, with the right (blue) picture. Both
//!   ready-state transitions are recorded with their times.
//! * `init-only`: only the init segments are appended, then a seek to 2.0 s, then play, then the segments that
//!   contain 2.0 s (media segments 1 and 2). Playback must start at the target, red, within 1 s.
//! * `past-duration`: everything appended and ended; seek to 20 s (beyond the duration) and to exactly 8 s.
//!   The outcome must be defined (refusal, end of stream or error within 4 s, never a hang); seeking to the
//!   duration itself must end the stream.

use std::time::{Duration, Instant};

use mse_rig::backend::EosError;
use mse_rig::events::{ElementReadyState, MseEvent, PlayEvent};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::{Rig, Shape};

use crate::probes::{hold_and_watch, keep_going};
use crate::support::{
    Res, Scn, Watch, append_inits, append_media, diagnostics, fixture, fmt_opt, fmt_ready_states,
    judge_colours, ms_between, ms_of, new_rig, nominal_colour, one_feed, ready_state_events,
    reset_stats, run_scenario, shutdown, t_ms, timed_config, vstats,
};

pub const TITLE: &str =
    "seek to an unbuffered range then append there; seek before any media; seek past the duration";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C07", TITLE, true, body)
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let outcome = unbuffered_then_append(ctx, scn);
    keep_going(scn, "unbuffered", outcome)?;
    let outcome = seek_before_media(ctx, scn);
    keep_going(scn, "init-only", outcome)?;
    let outcome = seek_past_duration(ctx, scn);
    keep_going(scn, "past-duration", outcome)?;
    Ok(())
}

fn unbuffered_then_append(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let muxed = fixture("h264-aac")?;
    let rig = new_rig(scn, timed_config(ctx))?;
    let feeds = one_feed(&rig, &muxed)?;
    append_inits(&rig, &feeds)?;
    append_media(&rig, &feeds, 0..2)?;
    rig.play()?;
    let playing = rig.wait_until(Duration::from_secs(8), |r| {
        r.position().is_some_and(|p| p >= t_ms(800))
    });
    if !scn.check(
        "unbuffered.plays-before-seek",
        playing,
        format!("playback of 0-4 s did not start: {}", diagnostics(&rig)),
    ) {
        shutdown(scn, "c07-unbuffered", rig);
        return Ok(());
    }

    // Seek to 5.5 s, where nothing is buffered (0-4 s is).
    let mark = rig.log.snapshot().len();
    reset_stats(&rig);
    rig.seek(t_ms(5_500))?;
    let hold = hold_and_watch(&rig, Duration::from_millis(1_500), t_ms(3_500));
    let fall = ready_state_events(&rig.log.snapshot(), mark);
    let state_now = rig.element_ready_state();
    scn.metric(
        "unbuffered.ready_states_after_seek",
        fmt_ready_states(&fall),
    )
    .metric(
        "unbuffered.ready_state_after_hold",
        format!("{state_now:?}"),
    )
    .metric(
        "unbuffered.position_during_hold_ms",
        hold.max_position.map(ms_of),
    )
    .metric(
        "unbuffered.last_video_pts_during_hold_ms",
        hold.last_video_pts.map(ms_of),
    );
    scn.check(
        "unbuffered.does-not-run-on",
        !hold.frame_at_or_after && hold.max_position.is_none_or(|p| ms_of(p) <= 5_800),
        format!(
            "after seeking to the unbuffered 5.5 s the max position was {}, the last video frame {} (frames at or after 3.5 s must not arrive: only 0-4 s is buffered and the seek flushed it)",
            fmt_opt(hold.max_position),
            fmt_opt(hold.last_video_pts)
        ),
    );
    scn.check(
        "unbuffered.ready-state-falls",
        state_now < ElementReadyState::HaveFutureData,
        format!(
            "ElementReadyState is {state_now:?} 1.5 s after seeking into an unbuffered range (spec: HAVE_METADATA); transitions: {}",
            fmt_ready_states(&fall)
        ),
    );

    // Append the segment that contains the target, as a player does.
    let mark = rig.log.snapshot().len();
    let watch = Watch::start(&rig.video, Duration::from_secs(6), |s| {
        s.last_pts.is_some_and(|pts| ms_of(pts) >= 4_000)
    });
    append_media(&rig, &feeds, 2..3)?;
    let appended = Instant::now();
    let resumed = rig.wait_until(Duration::from_secs(3), |r| {
        r.video
            .lock()
            .map(|s| s.last_pts.is_some_and(|pts| ms_of(pts) >= 4_000))
            .unwrap_or(false)
    });
    let fired = watch.finish();
    let latency = fired.map(|at| ms_between(appended, at));
    append_media(&rig, &feeds, 3..4)?;
    rig.backend.end_of_stream(EosError::None)?;
    std::thread::sleep(Duration::from_millis(1_000));
    let rise = ready_state_events(&rig.log.snapshot(), mark);
    let after = vstats(&rig);
    scn.metric("unbuffered.resume_after_update_end_ms", latency)
        .metric(
            "unbuffered.ready_states_after_append",
            fmt_ready_states(&rise),
        )
        .metric(
            "unbuffered.first_video_pts_after_append_ms",
            after.first_pts.map(ms_of),
        );
    scn.check(
        "unbuffered.resumes-within-1s",
        resumed && latency.is_some_and(|ms| ms <= 1_000),
        format!(
            "after the append of the segment holding 5.5 s the first frame came {} after its UpdateEnd (limit 1000 ms, waited up to 3 s); {}",
            crate::support::fmt_ms(latency),
            diagnostics(&rig)
        ),
    );
    let fresh: Vec<_> = after
        .colour
        .iter()
        .copied()
        .filter(|sample| ms_of(sample.pts) >= 3_500)
        .collect();
    let verdict = judge_colours(&fresh, nominal_colour, t_ms(40));
    scn.check(
        "unbuffered.picture-is-blue",
        !fresh.is_empty() && verdict.ok(),
        if fresh.is_empty() {
            "no colour sample after the resume".to_owned()
        } else {
            format!("{}: {}", verdict.summary(), verdict.wrong.join("; "))
        },
    );
    scn.check(
        "unbuffered.position-advances",
        rig.position().is_some_and(|p| ms_of(p) >= 5_800),
        format!(
            "position {} one second after the append",
            fmt_opt(rig.position())
        ),
    );
    shutdown(scn, "c07-unbuffered", rig);
    Ok(())
}

fn seek_before_media(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let muxed = fixture("h264-aac")?;
    let rig = new_rig(scn, timed_config(ctx))?;
    let feeds = one_feed(&rig, &muxed)?;
    append_inits(&rig, &feeds)?;
    let seek = rig.seek(t_ms(2_000));
    scn.metric("init-only.seek_result", format!("{seek:?}"));
    let seek_refused = seek.is_err();
    if ctx.shape == Shape::Bare {
        if let Err(error) = &seek {
            scn.note(format!(
                "Bare shape: the pipeline has no pads yet, so a seek to 2.0 s after only the init segments is refused by the pipeline ({error}); GstPlay queues such a seek. The seek is repeated after the media is appended."
            ));
        }
    } else {
        scn.check(
            "init-only.seek-accepted",
            seek.is_ok(),
            format!("a seek to 2.0 s after only the init segments was refused: {seek:?}"),
        );
    }
    rig.play()?;
    std::thread::sleep(Duration::from_millis(500));
    let before = vstats(&rig).count;
    scn.check(
        "init-only.no-frames-before-data",
        before == 0,
        format!("{before} video frames reached the sink although no media segment was appended"),
    );
    scn.metric(
        "init-only.ready_state_before_data",
        format!("{:?}", rig.element_ready_state()),
    );
    reset_stats(&rig);
    let watch = Watch::start(&rig.video, Duration::from_secs(6), |s| s.count > 0);
    append_media(&rig, &feeds, 1..3)?;
    let appended = Instant::now();
    if seek_refused {
        reset_stats(&rig);
        rig.seek(t_ms(2_000))?;
    }
    let resumed = rig.wait_until(Duration::from_secs(3), |r| {
        crate::support::video_count(r) > 0
    });
    let fired = watch.finish();
    let latency = fired.map(|at| ms_between(appended, at));
    std::thread::sleep(Duration::from_millis(1_000));
    let after = vstats(&rig);
    scn.metric("init-only.first_frame_after_update_end_ms", latency)
        .metric("init-only.first_pts_ms", after.first_pts.map(ms_of));
    scn.check(
        "init-only.starts-within-1s",
        resumed && latency.is_some_and(|ms| ms <= 1_000),
        format!(
            "first frame {} after the UpdateEnd of the segments holding 2.0 s (limit 1000 ms, waited up to 3 s); {}",
            crate::support::fmt_ms(latency),
            diagnostics(&rig)
        ),
    );
    scn.check(
        "init-only.starts-at-target",
        after
            .first_pts
            .is_some_and(|pts| (1_900..=2_600).contains(&ms_of(pts))),
        format!(
            "the first frame after the seek to 2.0 s has timestamp {} (expected 1.9 s to 2.6 s: the keyframe at 2.0 s or the target)",
            fmt_opt(after.first_pts)
        ),
    );
    let verdict = judge_colours(&after.colour, nominal_colour, t_ms(40));
    scn.check(
        "init-only.picture",
        !after.colour.is_empty() && verdict.ok(),
        format!("{}: {}", verdict.summary(), verdict.wrong.join("; ")),
    );
    shutdown(scn, "c07-init-only", rig);
    Ok(())
}

/// How a seek beyond the data ended.
fn outcome_within(rig: &Rig, mark: usize, timeout: Duration) -> String {
    let found = rig.log.wait_for(timeout, |events| {
        events.iter().skip(mark).any(|timed| {
            matches!(
                timed.event,
                MseEvent::Play(PlayEvent::EndOfStream) | MseEvent::Play(PlayEvent::Error(_))
            )
        })
    });
    if !found {
        return format!("nothing within {timeout:?}");
    }
    rig.log
        .snapshot()
        .iter()
        .skip(mark)
        .find_map(|timed| match &timed.event {
            MseEvent::Play(PlayEvent::EndOfStream) => {
                Some(format!("end of stream after {} ms", timed.at.as_millis()))
            }
            MseEvent::Play(PlayEvent::Error(message)) => Some(format!("GstPlay error: {message}")),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn seek_past_duration(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let muxed = fixture("h264-aac")?;
    for (name, target_ms, must_end) in [
        ("past-duration-20s", 20_000u64, false),
        ("at-duration-8s", 8_000, true),
    ] {
        let rig = new_rig(scn, timed_config(ctx))?;
        let feeds = one_feed(&rig, &muxed)?;
        append_inits(&rig, &feeds)?;
        append_media(&rig, &feeds, 0..4)?;
        rig.backend.end_of_stream(EosError::None)?;
        rig.play()?;
        let playing = rig.wait_until(Duration::from_secs(8), |r| {
            r.position().is_some_and(|p| p >= t_ms(500))
        });
        if !scn.check(
            &format!("{name}.plays-before-seek"),
            playing,
            format!("playback did not start: {}", diagnostics(&rig)),
        ) {
            shutdown(scn, "c07-past", rig);
            continue;
        }
        let mark = rig.log.snapshot().len();
        let seek = rig.seek(t_ms(target_ms));
        scn.metric(&format!("{name}.seek_result"), format!("{seek:?}"));
        match seek {
            Err(error) => {
                scn.note(format!(
                    "API-FIT: GstPlay refused the seek to {target_ms} ms: {error}; the DOM must clamp seeks to the duration itself"
                ));
                if must_end {
                    scn.check(
                        &format!("{name}.ends"),
                        false,
                        format!("a seek to exactly the duration was refused: {error}"),
                    );
                }
            }
            Ok(()) => {
                let outcome = outcome_within(&rig, mark, Duration::from_secs(4));
                scn.metric(&format!("{name}.outcome"), outcome.clone());
                let defined = !outcome.starts_with("nothing");
                let ended = outcome.starts_with("end of stream");
                if must_end {
                    scn.check(
                        &format!("{name}.ends"),
                        ended,
                        format!(
                            "a seek to exactly the duration gave: {outcome}; {}",
                            diagnostics(&rig)
                        ),
                    );
                } else {
                    scn.check(
                        &format!("{name}.defined-outcome"),
                        defined,
                        format!(
                            "a seek past the duration gave: {outcome}; {}",
                            diagnostics(&rig)
                        ),
                    );
                }
            }
        }
        shutdown(scn, "c07-past", rig);
    }
    Ok(())
}
