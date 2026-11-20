// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C12 (P1): `timestampOffset`, append window and sequence mode, each in a fresh rig on `h264-low` (video
//! only), fast mode.
//!
//! * `offset-plus-8s`: `set_timestamp_offset(+8 s)` then the whole stream: `buffered` [8, 16], the picture plays
//!   with timestamps 8-16 s, and the result equals the same bytes moved by `shift_media_time` (+8000 ms) and
//!   appended with offset 0.
//! * `offset-before-init`: the same +8 s offset set before the init segment instead of after it (the order a page uses
//!   decides whether the offset is picked up; with GstMSE 1.28.9 neither order does).
//! * `sequence`: `AppendMode::Sequence`, segment 0 (0-2 s) then segment 2 (tfdt 4 s): the second is placed
//!   right after the first, `buffered` [0, 4], and the picture shows red then blue.
//! * `window`: append window [2 s, 6 s], whole stream: `buffered` [2, 6] and 100 frames; an inverted window is
//!   refused; a window end of +Infinity (the default) is accepted.
//! * `negative-offset`: `set_timestamp_offset(-1 s)`. GstMSE's offset is unsigned: the result (accepted or
//!   refused, with the error class) is recorded as an API-fit finding. If it is accepted it must be applied
//!   correctly: [1, 7] (the frames before 0 and their dependants up to the keyframe at 2 s are dropped).

use std::time::Duration;

use mse_rig::backend::{AppendMode, EosError, MseError};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::Rig;
use mse_segments::ChunkPlan;

use crate::probes::keep_going;
use crate::support::{
    Chunking, Colour, Feed, LOOSE_TOL_MS, Res, Scn, add_source_buffer, append_inits, append_media,
    append_segment, compare_ranges, diagnostics, errors_in, fast_config, fixture, fmt_opt,
    fmt_ranges, judge_colours, ms_of, new_rig, nominal_colour, ranges_json, run_scenario,
    shifted_colour, shutdown, t_ms, vstats, wait_eos,
};
use crate::support::{check_buffered, matches_ranges, wait_buffered};

pub const TITLE: &str =
    "timestampOffset +8 s and -1 s, sequence mode, append window [2 s, 6 s], +Infinity window";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C12", TITLE, true, body)
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let low = fixture("h264-low")?;
    let outcome = offset_plus_8s(ctx, scn, &low);
    keep_going(scn, "offset-plus-8s", outcome)?;
    let outcome = offset_before_init(ctx, scn, &low);
    keep_going(scn, "offset-before-init", outcome)?;
    let outcome = sequence_mode(ctx, scn, &low);
    keep_going(scn, "sequence", outcome)?;
    let outcome = append_window(ctx, scn, &low);
    keep_going(scn, "window", outcome)?;
    let outcome = negative_offset(ctx, scn, &low);
    keep_going(scn, "negative-offset", outcome)?;
    Ok(())
}

fn class(error: &MseError) -> &'static str {
    match error {
        MseError::InvalidState(_) => "InvalidState",
        MseError::Type(_) => "Type",
        MseError::NotSupported(_) => "NotSupported",
        MseError::NotFound(_) => "NotFound",
        MseError::QuotaExceeded(_) => "QuotaExceeded",
        MseError::Other(_) => "Other",
    }
}

/// Plays whatever is buffered to the end and returns the video statistics.
fn play_out(scn: &mut Scn, rig: &Rig, label: &str) -> Res<mse_rig::frames::FrameStats> {
    rig.backend.end_of_stream(EosError::None)?;
    rig.play()?;
    let eos = wait_eos(rig, Duration::from_secs(20));
    scn.check(
        &format!("{label}.eos"),
        eos.reached,
        format!("the stream did not end: {}", diagnostics(rig)),
    );
    Ok(vstats(rig))
}

fn offset_plus_8s(ctx: &Ctx, scn: &mut Scn, low: &mse_segments::SegmentedStream) -> Res<()> {
    let label = "offset-plus-8s";
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = Feed {
        id: add_source_buffer(&rig, low)?,
        stream: low,
        chunking: Chunking::Whole,
    };
    append_inits(&rig, &[feed])?;
    let set = rig.backend.set_timestamp_offset(feed.id, 8_000_000_000);
    scn.metric(&format!("{label}.set_result"), format!("{set:?}"));
    if let Err(error) = &set {
        scn.finding(format!(
            "set_timestamp_offset(+8 s) was refused: {error:?}; a positive offset is the common case (periods, ad insertion)"
        ));
        shutdown(scn, "c12-offset", rig);
        return Ok(());
    }
    let offset_readback = rig.backend.timestamp_offset(feed.id);
    scn.metric(
        &format!("{label}.offset_read_back"),
        format!("{offset_readback:?}"),
    );
    append_media(&rig, &[feed], 0..4)?;
    let offset_ranges = check_buffered(
        scn,
        &rig,
        &format!("{label}.buffered"),
        feed.id,
        &[(8_000, 16_000)],
        LOOSE_TOL_MS,
    )?
    .settled;
    let stats = play_out(scn, &rig, label)?;
    scn.metric(&format!("{label}.video_frames"), stats.count)
        .metric(&format!("{label}.first_pts_ms"), stats.first_pts.map(ms_of))
        .metric(&format!("{label}.last_pts_ms"), stats.last_pts.map(ms_of));
    scn.check(
        &format!("{label}.plays-at-offset"),
        stats.count.abs_diff(200) <= 2
            && stats
                .first_pts
                .is_some_and(|p| (7_900..=8_100).contains(&ms_of(p)))
            && stats
                .last_pts
                .is_some_and(|p| (15_800..=16_050).contains(&ms_of(p))),
        format!(
            "{} frames with timestamps {} to {} (expected 200 +- 2 frames from 8.0 s to 15.96 s)",
            stats.count,
            fmt_opt(stats.first_pts),
            fmt_opt(stats.last_pts)
        ),
    );
    let verdict = judge_colours(&stats.colour, shifted_colour(8_000), t_ms(40));
    scn.check(
        &format!("{label}.picture"),
        stats.colour.len() >= 4 && verdict.ok(),
        format!("{}: {}", verdict.summary(), verdict.wrong.join("; ")),
    );
    let duration = rig.element_duration();
    scn.check(
        &format!("{label}.duration"),
        duration.is_some_and(|d| (ms_of(d) - 16_000).abs() <= LOOSE_TOL_MS),
        format!("the element's duration is {} after appending 8 s at offset 8 s and ending the stream (expected 16 s)", fmt_opt(duration)),
    );
    shutdown(scn, "c12-offset", rig);

    // The same bytes moved in time by the segment writer, appended with offset 0.
    let mut shifted = low.clone();
    mse_segments::shift_media_time(&mut shifted, 8_000)?;
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = Feed {
        id: add_source_buffer(&rig, &shifted)?,
        stream: &shifted,
        chunking: Chunking::Whole,
    };
    append_inits(&rig, &[feed])?;
    append_media(&rig, &[feed], 0..4)?;
    let expected: Vec<(i64, i64)> = offset_ranges
        .iter()
        .map(|r| (ms_of(r.start), ms_of(r.end)))
        .collect();
    let shifted_ranges = wait_buffered(
        &rig,
        feed.id,
        Duration::from_secs(3),
        matches_ranges(expected.clone(), 2),
    )?
    .settled;
    scn.metric(
        &format!("{label}.shift_media_time_buffered"),
        ranges_json(&shifted_ranges),
    );
    let deviations = compare_ranges(
        "shift_media_time vs timestampOffset",
        &shifted_ranges,
        &expected,
        2,
    );
    scn.check(
        &format!("{label}.same-as-shift-media-time"),
        deviations.is_empty(),
        format!(
            "set_timestamp_offset(+8 s) gives {}, the same data shifted by 8000 ms with offset 0 gives {}: {}",
            fmt_ranges(&offset_ranges),
            fmt_ranges(&shifted_ranges),
            deviations.join("; ")
        ),
    );
    shutdown(scn, "c12-shifted", rig);
    Ok(())
}

/// The same +8 s offset, set before the init segment instead of after it: tells whether the offset is picked up
/// only when the source buffer is first fed (the order a page uses decides whether it works).
fn offset_before_init(ctx: &Ctx, scn: &mut Scn, low: &mse_segments::SegmentedStream) -> Res<()> {
    let label = "offset-before-init";
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = Feed {
        id: add_source_buffer(&rig, low)?,
        stream: low,
        chunking: Chunking::Whole,
    };
    let set = rig.backend.set_timestamp_offset(feed.id, 8_000_000_000);
    scn.metric(&format!("{label}.set_result"), format!("{set:?}"));
    if set.is_err() {
        scn.note(format!(
            "{label}: set_timestamp_offset before the init segment was refused: {set:?}"
        ));
        shutdown(scn, "c12-offset-first", rig);
        return Ok(());
    }
    append_inits(&rig, &[feed])?;
    append_media(&rig, &[feed], 0..4)?;
    scn.metric(
        &format!("{label}.offset_read_back"),
        format!("{:?}", rig.backend.timestamp_offset(feed.id)),
    );
    check_buffered(
        scn,
        &rig,
        &format!("{label}.buffered"),
        feed.id,
        &[(8_000, 16_000)],
        LOOSE_TOL_MS,
    )?;
    shutdown(scn, "c12-offset-first", rig);
    Ok(())
}

fn sequence_mode(ctx: &Ctx, scn: &mut Scn, low: &mse_segments::SegmentedStream) -> Res<()> {
    let label = "sequence";
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = Feed {
        id: add_source_buffer(&rig, low)?,
        stream: low,
        chunking: Chunking::Whole,
    };
    let mode = rig.backend.set_append_mode(feed.id, AppendMode::Sequence);
    scn.metric(&format!("{label}.set_mode_result"), format!("{mode:?}"));
    if let Err(error) = &mode {
        scn.finding(format!("set_append_mode(Sequence) was refused: {error:?}"));
        shutdown(scn, "c12-sequence", rig);
        return Ok(());
    }
    append_inits(&rig, &[feed])?;
    // segment 0 is 0-2 s, segment 2 is 4-6 s in the file: in sequence mode it must land at 2-4 s.
    append_segment(&rig, feed.id, &low.media[0], &ChunkPlan::Whole)?;
    append_segment(&rig, feed.id, &low.media[2], &ChunkPlan::Whole)?;
    check_buffered(
        scn,
        &rig,
        &format!("{label}.back-to-back"),
        feed.id,
        &[(0, 4_000)],
        LOOSE_TOL_MS,
    )?;
    let stats = play_out(scn, &rig, label)?;
    scn.metric(&format!("{label}.video_frames"), stats.count);
    let expected = |pts: mse_rig::ClockTime| {
        if ms_of(pts) < 2_000 {
            Colour::Red
        } else {
            Colour::Blue
        }
    };
    let verdict = judge_colours(&stats.colour, expected, t_ms(40));
    scn.check(
        &format!("{label}.picture"),
        stats.count.abs_diff(100) <= 2 && stats.colour.len() >= 3 && verdict.ok(),
        format!(
            "{} frames (expected 100 +- 2); {}: {}",
            stats.count,
            verdict.summary(),
            verdict.wrong.join("; ")
        ),
    );
    shutdown(scn, "c12-sequence", rig);
    Ok(())
}

fn append_window(ctx: &Ctx, scn: &mut Scn, low: &mse_segments::SegmentedStream) -> Res<()> {
    let label = "window";
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = Feed {
        id: add_source_buffer(&rig, low)?,
        stream: low,
        chunking: Chunking::Whole,
    };
    // +Infinity is the default end: setting it explicitly must be accepted.
    let infinity = rig.backend.set_append_window(feed.id, t_ms(0), None);
    scn.metric(&format!("{label}.infinity_result"), format!("{infinity:?}"));
    scn.check(
        &format!("{label}.infinity-accepted"),
        infinity.is_ok(),
        format!(
            "set_append_window(0, +Infinity) (the default window, which players set explicitly) returned {infinity:?}; the wrapper has to skip the call"
        ),
    );
    // an inverted window is a TypeError
    let inverted = rig
        .backend
        .set_append_window(feed.id, t_ms(6_000), Some(t_ms(2_000)));
    scn.metric(&format!("{label}.inverted_result"), format!("{inverted:?}"));
    scn.check(
        &format!("{label}.inverted-refused"),
        inverted.is_err(),
        "set_append_window(6 s, 2 s) was accepted (spec: TypeError: the start must be below the end)",
    );
    if let Err(error) = &inverted
        && class(error) != "Type"
    {
        scn.note(format!(
                "API-FIT: an inverted append window is refused with {} ({error:?}); the spec throws TypeError",
                class(error)
            ));
    }
    let set = rig
        .backend
        .set_append_window(feed.id, t_ms(2_000), Some(t_ms(6_000)));
    scn.metric(&format!("{label}.set_result"), format!("{set:?}"));
    if let Err(error) = &set {
        scn.finding(format!(
            "set_append_window(2 s, 6 s) was refused: {error:?}"
        ));
        shutdown(scn, "c12-window", rig);
        return Ok(());
    }
    append_inits(&rig, &[feed])?;
    append_media(&rig, &[feed], 0..4)?;
    check_buffered(
        scn,
        &rig,
        &format!("{label}.buffered"),
        feed.id,
        &[(2_000, 6_000)],
        LOOSE_TOL_MS,
    )?;
    let stats = play_out(scn, &rig, label)?;
    scn.metric(&format!("{label}.video_frames"), stats.count)
        .metric(&format!("{label}.first_pts_ms"), stats.first_pts.map(ms_of));
    let verdict = judge_colours(&stats.colour, nominal_colour, t_ms(40));
    scn.check(
        &format!("{label}.plays-the-window"),
        stats.count.abs_diff(100) <= 2
            && stats
                .first_pts
                .is_some_and(|p| (1_900..=2_100).contains(&ms_of(p)))
            && verdict.ok(),
        format!(
            "{} frames from {} to {} (expected 100 +- 2 frames from 2.0 s); {}",
            stats.count,
            fmt_opt(stats.first_pts),
            fmt_opt(stats.last_pts),
            verdict.summary()
        ),
    );
    let errors = errors_in(&rig.log.snapshot());
    scn.check(
        &format!("{label}.no-errors"),
        errors.is_empty(),
        format!("error or abort events: {}", errors.join("; ")),
    );
    shutdown(scn, "c12-window", rig);
    Ok(())
}

fn negative_offset(ctx: &Ctx, scn: &mut Scn, low: &mse_segments::SegmentedStream) -> Res<()> {
    let label = "negative-offset";
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = Feed {
        id: add_source_buffer(&rig, low)?,
        stream: low,
        chunking: Chunking::Whole,
    };
    append_inits(&rig, &[feed])?;
    let set = rig.backend.set_timestamp_offset(feed.id, -1_000_000_000);
    scn.metric(&format!("{label}.set_result"), format!("{set:?}"));
    match set {
        Err(error) => {
            scn.note(format!(
                "API-FIT: set_timestamp_offset(-1 s) is refused with {} ({error:?}); GstMSE's offset is an unsigned GstClockTime, so a negative offset needs a wrapper (shift the media timeline by a constant instead) or the DOM must throw",
                class(&error)
            ));
        }
        Ok(()) => {
            append_media(&rig, &[feed], 0..4)?;
            check_buffered(
                scn,
                &rig,
                &format!("{label}.applied-correctly"),
                feed.id,
                &[(1_000, 7_000)],
                LOOSE_TOL_MS,
            )?;
        }
    }
    shutdown(scn, "c12-negative", rig);
    Ok(())
}
