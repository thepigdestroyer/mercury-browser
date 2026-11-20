// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C08 (P0): quality switch inside one source buffer, the way DASH does it: append the new rendition's
//! init segment, then its media segments, at a keyframe boundary. Three sub-runs, each in a fresh rig, video
//! only (one source buffer: two deadlock GstMSE 1.28.9, see C02), fast mode (decode as fast as possible, so
//! frame counts and gaps are exact):
//!
//! * `up`: play `h264-low` segments 0-1 (the decoder idles at 4 s waiting for data, as a player that is
//!   ahead of its buffer does), then append `h264-high`'s init segment and segments 2-3. Records the delay of
//!   the first 640x360 frame after the switch (from the start of the init append, and from the `UpdateEnd`
//!   of the first high media segment; negative means the frame was already at the sink).
//! * `alternate`: low, high, low, high on every segment (aggressive adaptation: three switches, each with
//!   its init segment), appended before playing.
//! * `overwrite`: low segments 0-1 are buffered, then the high rendition is appended over 2-6 s (the player
//!   replaces already buffered data at a segment boundary: a switch in the middle of what is buffered) and low
//!   again for 6-8 s.
//!
//! Every sub-run: 200 +- 2 video frames; no gap between frames larger than 42 ms (1 frame + 2 ms); the sink
//! saw exactly the expected sequence of picture sizes (so the change happened once per switch, not more);
//! the colours stay right; no error events and the update events keep their order.

use std::time::{Duration, Instant};

use mse_rig::backend::EosError;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::Rig;
use mse_segments::{ChunkPlan, SegmentedStream};

use crate::probes::{fmt_sizes, keep_going, size_sequence};
use crate::support::{
    FRAME_MS, LOOSE_TOL_MS, NOMINAL_FRAMES, NOMINAL_MS, Res, Scn, Watch, append_inits,
    append_segment, check_buffered, diagnostics, errors_in, event_order_problems, fast_config,
    fixture, fmt_opt, judge_colours, ms_between, ms_of, new_rig, nominal_colour, one_feed,
    run_scenario, shutdown, t_ms, video_count, vstats, wait_eos,
};

pub const TITLE: &str = "quality switch in one source buffer: low to high, alternating every segment, overwrite in the middle";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C08", TITLE, true, body)
}

struct Streams {
    low: SegmentedStream,
    high: SegmentedStream,
    low_size: (u32, u32),
    high_size: (u32, u32),
}

fn size_of(name: &str) -> Res<(u32, u32)> {
    mse_segments::fixture(name)
        .map(|f| (f.width, f.height))
        .ok_or_else(|| crate::support::Abort::Error(format!("no fixture {name:?}")))
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let streams = Streams {
        low: fixture("h264-low")?,
        high: fixture("h264-high")?,
        low_size: size_of("h264-low")?,
        high_size: size_of("h264-high")?,
    };
    let outcome = switch_up(ctx, scn, &streams);
    keep_going(scn, "up", outcome)?;
    let outcome = alternate(ctx, scn, &streams);
    keep_going(scn, "alternate", outcome)?;
    let outcome = overwrite(ctx, scn, &streams);
    keep_going(scn, "overwrite", outcome)?;
    Ok(())
}

/// What the sink and the log saw, judged by the sub-run's expectations (shared with C09, which does not
/// know the sizes: `expected_sizes` is `None` there).
pub(crate) fn judge_switch(
    scn: &mut Scn,
    label: &str,
    rig: &Rig,
    ids: &[usize],
    expected_sizes: Option<&[(u32, u32)]>,
) {
    let video = vstats(rig);
    let events = rig.log.snapshot();
    let sizes = size_sequence(&video);
    scn.metric(&format!("{label}.video_frames"), video.count)
        .metric(
            &format!("{label}.video_max_gap_ms"),
            video.max_gap.map(ms_of),
        )
        .metric(&format!("{label}.caps_changes"), video.caps_changes.len())
        .metric(&format!("{label}.sizes"), fmt_sizes(&sizes))
        .metric(&format!("{label}.decoders"), rig.chosen_decoders());
    scn.check(
        &format!("{label}.frames"),
        video.count.abs_diff(NOMINAL_FRAMES) <= 2,
        format!(
            "{} video frames reached the sink, expected {NOMINAL_FRAMES} +- 2 (first pts {}, last pts {})",
            video.count,
            fmt_opt(video.first_pts),
            fmt_opt(video.last_pts)
        ),
    );
    scn.check(
        &format!("{label}.max-gap"),
        video.max_gap.is_none_or(|gap| ms_of(gap) <= FRAME_MS + 2),
        format!(
            "the largest gap between consecutive video timestamps is {} ms (limit one frame, {} ms + 2 ms)",
            video.max_gap.map_or(0, ms_of),
            FRAME_MS
        ),
    );
    if let Some(expected_sizes) = expected_sizes {
        let expected: Vec<Option<(u32, u32)>> = expected_sizes.iter().map(|s| Some(*s)).collect();
        scn.check(
            &format!("{label}.caps-sequence"),
            sizes == expected,
            format!(
                "the video sink saw the picture sizes {} (caps changes: {}), expected {} ({} change(s))",
                fmt_sizes(&sizes),
                video
                    .caps_changes
                    .iter()
                    .map(|c| c.caps.as_str())
                    .collect::<Vec<_>>()
                    .join(" | "),
                fmt_sizes(&expected),
                expected.len().saturating_sub(1)
            ),
        );
    }
    let verdict = judge_colours(&video.colour, nominal_colour, t_ms(40));
    scn.check(
        &format!("{label}.colour"),
        video.colour.len() >= 4 && verdict.ok(),
        format!("{}: {}", verdict.summary(), verdict.wrong.join("; ")),
    );
    let errors = errors_in(&events);
    scn.check(
        &format!("{label}.no-errors"),
        errors.is_empty(),
        format!("error or abort events: {}", errors.join("; ")),
    );
    let order = event_order_problems(&events, ids, false);
    scn.check(
        &format!("{label}.event-order"),
        order.is_empty(),
        order.join("; "),
    );
}

fn switch_up(ctx: &Ctx, scn: &mut Scn, s: &Streams) -> Res<()> {
    let label = "up";
    let rig = new_rig(scn, fast_config(ctx))?;
    let [feed] = one_feed(&rig, &s.low)?;
    append_inits(&rig, &[feed])?;
    append_segment(&rig, feed.id, &s.low.media[0], &ChunkPlan::Whole)?;
    append_segment(&rig, feed.id, &s.low.media[1], &ChunkPlan::Whole)?;
    rig.play()?;
    let first_half = rig.wait_until(Duration::from_secs(10), |r| video_count(r) >= 98);
    if !scn.check(
        &format!("{label}.first-half-plays"),
        first_half,
        format!(
            "the first 4 s (100 frames) did not reach the sink before the switch: {}",
            diagnostics(&rig)
        ),
    ) {
        shutdown(scn, "c08-up", rig);
        return Ok(());
    }

    // The switch: the new rendition's init segment, then its media segments from the next boundary (4 s).
    let new_size = s.high_size;
    let watch = Watch::start(&rig.video, Duration::from_secs(15), move |stats| {
        stats
            .caps_changes
            .last()
            .is_some_and(|c| crate::probes::caps_size(&c.caps) == Some(new_size))
    });
    let started = Instant::now();
    append_segment(&rig, feed.id, &s.high.init, &ChunkPlan::Whole)?;
    let init_done = Instant::now();
    append_segment(&rig, feed.id, &s.high.media[2], &ChunkPlan::Whole)?;
    let first_media_done = Instant::now();
    append_segment(&rig, feed.id, &s.high.media[3], &ChunkPlan::Whole)?;
    rig.backend.end_of_stream(EosError::None)?;
    let eos = wait_eos(&rig, Duration::from_secs(20));
    let fired = watch.finish();
    scn.metric(
        &format!("{label}.first_new_size_frame_after_init_append_start_ms"),
        fired.map(|at| ms_between(started, at)),
    )
    .metric(
        &format!("{label}.first_new_size_frame_after_init_update_end_ms"),
        fired.map(|at| ms_between(init_done, at)),
    )
    .metric(
        &format!("{label}.first_new_size_frame_after_first_media_update_end_ms"),
        fired.map(|at| ms_between(first_media_done, at)),
    );
    scn.check(
        &format!("{label}.eos"),
        eos.reached,
        format!(
            "the stream did not end after the switch: {}",
            diagnostics(&rig)
        ),
    );
    scn.check(
        &format!("{label}.new-size-arrives"),
        fired.is_some(),
        format!(
            "no {}x{} frame reached the sink within 15 s of the switch: {}",
            new_size.0,
            new_size.1,
            diagnostics(&rig)
        ),
    );
    judge_switch(
        scn,
        label,
        &rig,
        &[feed.id],
        Some(&[s.low_size, s.high_size]),
    );
    check_buffered(
        scn,
        &rig,
        &format!("{label}.buffered"),
        feed.id,
        &[(0, NOMINAL_MS)],
        LOOSE_TOL_MS,
    )?;
    shutdown(scn, "c08-up", rig);
    Ok(())
}

fn alternate(ctx: &Ctx, scn: &mut Scn, s: &Streams) -> Res<()> {
    let label = "alternate";
    let rig = new_rig(scn, fast_config(ctx))?;
    let [feed] = one_feed(&rig, &s.low)?;
    append_inits(&rig, &[feed])?;
    // low 0, then for each next segment: the other rendition's init segment, then its segment.
    for index in 0..4 {
        let stream = if index % 2 == 0 { &s.low } else { &s.high };
        if index > 0 {
            append_segment(&rig, feed.id, &stream.init, &ChunkPlan::Whole)?;
        }
        append_segment(&rig, feed.id, &stream.media[index], &ChunkPlan::Whole)?;
    }
    rig.backend.end_of_stream(EosError::None)?;
    rig.play()?;
    let eos = wait_eos(&rig, Duration::from_secs(20));
    scn.check(
        &format!("{label}.eos"),
        eos.reached,
        format!("the stream did not end: {}", diagnostics(&rig)),
    );
    judge_switch(
        scn,
        label,
        &rig,
        &[feed.id],
        Some(&[s.low_size, s.high_size, s.low_size, s.high_size]),
    );
    shutdown(scn, "c08-alternate", rig);
    Ok(())
}

fn overwrite(ctx: &Ctx, scn: &mut Scn, s: &Streams) -> Res<()> {
    let label = "overwrite";
    let rig = new_rig(scn, fast_config(ctx))?;
    let [feed] = one_feed(&rig, &s.low)?;
    append_inits(&rig, &[feed])?;
    // low 0-4 s buffered; the player now decides to replace 2-6 s by the high rendition, then goes back to low.
    append_segment(&rig, feed.id, &s.low.media[0], &ChunkPlan::Whole)?;
    append_segment(&rig, feed.id, &s.low.media[1], &ChunkPlan::Whole)?;
    append_segment(&rig, feed.id, &s.high.init, &ChunkPlan::Whole)?;
    append_segment(&rig, feed.id, &s.high.media[1], &ChunkPlan::Whole)?;
    append_segment(&rig, feed.id, &s.high.media[2], &ChunkPlan::Whole)?;
    append_segment(&rig, feed.id, &s.low.init, &ChunkPlan::Whole)?;
    append_segment(&rig, feed.id, &s.low.media[3], &ChunkPlan::Whole)?;
    check_buffered(
        scn,
        &rig,
        &format!("{label}.buffered"),
        feed.id,
        &[(0, NOMINAL_MS)],
        LOOSE_TOL_MS,
    )?;
    rig.backend.end_of_stream(EosError::None)?;
    rig.play()?;
    let eos = wait_eos(&rig, Duration::from_secs(20));
    scn.check(
        &format!("{label}.eos"),
        eos.reached,
        format!("the stream did not end: {}", diagnostics(&rig)),
    );
    judge_switch(
        scn,
        label,
        &rig,
        &[feed.id],
        Some(&[s.low_size, s.high_size, s.low_size]),
    );
    shutdown(scn, "c08-overwrite", rig);
    Ok(())
}
