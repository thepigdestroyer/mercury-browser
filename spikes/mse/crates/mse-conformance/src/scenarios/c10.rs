// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C10 (P0): `remove` and `buffered`. The spec's removal rules (MSE "coded frame removal"): `remove(start, end)`
//! deletes the coded frames whose presentation timestamp is in [start, end), then everything that depended on a
//! deleted frame, up to the next random access point. With a keyframe every 2 s, 2 s segments and 8 s of
//! media (video only: one source buffer, two deadlock GstMSE 1.28.9, see C02), the exact answers are known:
//!
//! * after `remove(0, 4 s)`: [4, 8]; after `remove(6 s, +Infinity)`: [4, 6];
//! * re-appending the removed segments heals `buffered` to [0, 8];
//! * `remove(1 s, 3 s)` removes the keyframe at 2 s, so the frames 3-4 s that depend on it must go too:
//!   [0, 1] and [4, 8] (a remover that stops at 3 s leaves [3, 4] undecodable);
//! * a `remove` while an append is running is refused with `InvalidState`;
//! * the removed part does not play: after `remove(0, 4 s)` and a seek to 1 s nothing is rendered and the
//!   position holds, even though later data (4-8 s) is buffered.
//!
//! "Where `buffered` ends" is a convention (last timestamp, or last timestamp plus the frame's duration): the
//! baseline of the same run fixes it, and every later expectation moves with it, so only removal bugs are
//! reported. Edges are compared with 2 ms (frame times are exact). Every deviation is listed.

use std::time::{Duration, Instant};

use mse_rig::backend::{MseError, TimeRange};
use mse_rig::events::MseEvent;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::Rig;
use mse_segments::ChunkPlan;

use crate::probes::{hold_and_watch, keep_going};
use crate::support::{
    LOOSE_TOL_MS, NOMINAL_MS, Res, Scn, VIDEO_TOL_MS, append_inits, append_media, append_segment,
    buffered, compare_ranges, diagnostics, dur_ms, errors_in, event_order_problems, fast_config,
    fixture, fmt_opt, fmt_ranges, matches_ranges, ms_of, new_rig, one_feed, ranges_json,
    remove_and_wait, reset_stats, run_scenario, shutdown, t_ms, timed_config, update_ends, vstats,
    wait_buffered,
};

pub const TITLE: &str = "remove and buffered: head, tail to +Infinity, heal, ripple to the next keyframe, remove while updating, removed part does not play";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C10", TITLE, true, body)
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let outcome = removal_rules(ctx, scn);
    keep_going(scn, "removal-rules", outcome)?;
    let outcome = remove_while_updating(ctx, scn);
    keep_going(scn, "remove-while-updating", outcome)?;
    let outcome = removed_does_not_play(ctx, scn);
    keep_going(scn, "removed-does-not-play", outcome)?;
    Ok(())
}

/// One step: the ranges of the source buffer against the spec's, with every deviation listed.
fn check_step(scn: &mut Scn, rig: &Rig, step: &str, id: usize, expected: &[(i64, i64)]) -> Res<()> {
    let ranges = buffered(rig, id)?;
    scn.metric(&format!("{step}.buffered"), ranges_json(&ranges));
    let deviations = compare_ranges(
        &format!("buffered after {step}"),
        &ranges,
        expected,
        VIDEO_TOL_MS,
    );
    scn.check(
        &format!("{step}.buffered"),
        deviations.is_empty(),
        format!(
            "buffered {}: {}",
            fmt_ranges(&ranges),
            deviations.join("; ")
        ),
    );
    Ok(())
}

fn end_of(ranges: &[TimeRange]) -> i64 {
    ranges.last().map_or(NOMINAL_MS, |r| ms_of(r.end))
}

fn removal_rules(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let stream = fixture("h264-low")?;
    let rig = new_rig(scn, fast_config(ctx))?;
    let [feed] = one_feed(&rig, &stream)?;
    let id = feed.id;
    append_inits(&rig, &[feed])?;
    append_media(&rig, &[feed], 0..4)?;

    // The baseline fixes the "end" convention.
    let base_wait = wait_buffered(
        &rig,
        id,
        Duration::from_secs(5),
        matches_ranges(vec![(0, NOMINAL_MS)], LOOSE_TOL_MS),
    )?;
    let base = base_wait.settled.clone();
    scn.metric(
        "baseline.buffered_immediate",
        ranges_json(&base_wait.immediate),
    )
    .metric("baseline.buffered_lag_ms", dur_ms(base_wait.elapsed));
    scn.check(
        "baseline.visible-at-updateend",
        base_wait.immediate_ok,
        format!(
            "right after appending everything buffered was {} (expected [0.000-8.000]); it took {} ms to show all the data",
            fmt_ranges(&base_wait.immediate),
            dur_ms(base_wait.elapsed)
        ),
    );
    let convention = end_of(&base) - NOMINAL_MS;
    scn.metric("baseline.buffered", ranges_json(&base))
        .metric("baseline.end_convention_ms", convention);
    let problems = compare_ranges("baseline", &base, &[(0, NOMINAL_MS)], LOOSE_TOL_MS);
    if !scn.check(
        "baseline.buffered",
        problems.is_empty(),
        format!(
            "after appending everything buffered is {} (expected one range [0, 8 s]): {}",
            fmt_ranges(&base),
            problems.join("; ")
        ),
    ) {
        shutdown(scn, "c10-rules", rig);
        return Ok(());
    }
    let cv = convention;

    // 1. remove(0, 4 s)
    let before = update_ends(&rig.log, id);
    remove_and_wait(&rig, id, t_ms(0), Some(t_ms(4_000)))?;
    scn.check(
        "remove-head.update-end",
        update_ends(&rig.log, id) == before + 1,
        "remove did not produce exactly one UpdateEnd",
    );
    check_step(
        scn,
        &rig,
        "remove-head-0-4s",
        id,
        &[(4_000, NOMINAL_MS + cv)],
    )?;

    // 2. remove(6 s, +Infinity)
    remove_and_wait(&rig, id, t_ms(6_000), None)?;
    check_step(
        scn,
        &rig,
        "remove-tail-6s-to-infinity",
        id,
        &[(4_000, 6_000 + cv)],
    )?;

    // 3. heal: re-append the removed segments 0, 1 and 3
    for index in [0usize, 1, 3] {
        append_segment(&rig, id, &stream.media[index], &ChunkPlan::Whole)?;
    }
    check_step(
        scn,
        &rig,
        "heal-after-reappend",
        id,
        &[(0, NOMINAL_MS + cv)],
    )?;

    // 4. remove(1 s, 3 s): the keyframe at 2 s goes, so video must go on to the next keyframe (4 s)
    remove_and_wait(&rig, id, t_ms(1_000), Some(t_ms(3_000)))?;
    check_step(
        scn,
        &rig,
        "ripple-remove-1-3s",
        id,
        &[(0, 1_000 + cv), (4_000, NOMINAL_MS + cv)],
    )?;

    // Every remove and append went through UpdateStart -> Update -> UpdateEnd.
    let events = rig.log.snapshot();
    let order = event_order_problems(&events, &[id], false);
    scn.check("event-order", order.is_empty(), order.join("; "));
    let errors = errors_in(&events);
    scn.check(
        "no-errors",
        errors.is_empty(),
        format!("error or abort events: {}", errors.join("; ")),
    );
    shutdown(scn, "c10-rules", rig);
    Ok(())
}

fn remove_while_updating(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let low = fixture("h264-low")?;
    let rig = new_rig(scn, fast_config(ctx))?;
    let [feed] = one_feed(&rig, &low)?;
    let id = feed.id;
    append_inits(&rig, &[feed])?;
    let mut conclusive = false;
    for (attempt, times) in [8usize, 40, 160].into_iter().enumerate() {
        let tiled = mse_segments::tile(&low, times)?;
        let big: Vec<u8> = tiled
            .media
            .iter()
            .flat_map(|s| s.bytes.iter().copied())
            .collect();
        let before = update_ends(&rig.log, id);
        rig.backend.append(id, &big)?;
        let updating = rig.backend.updating(id);
        scn.metric(&format!("while-updating.attempt{attempt}.bytes"), big.len())
            .metric(
                &format!("while-updating.attempt{attempt}.updating_after_append"),
                updating,
            );
        if updating {
            let result = rig.backend.remove(id, t_ms(0), Some(t_ms(1_000)));
            scn.metric(
                &format!("while-updating.attempt{attempt}.remove_result"),
                format!("{result:?}"),
            );
            scn.check(
                "remove-while-updating.invalid-state",
                matches!(result, Err(MseError::InvalidState(_))),
                format!(
                    "remove() while an append of {} bytes was still updating returned {result:?} (spec: InvalidStateError)",
                    big.len()
                ),
            );
            conclusive = true;
        }
        let ended = rig.log.wait_for(Duration::from_secs(60), |events| {
            events
                .iter()
                .filter(|t| matches!(t.event, MseEvent::UpdateEnd(i) if i == id))
                .count()
                > before
        });
        scn.check(
            &format!("remove-while-updating.attempt{attempt}.append-ends"),
            ended,
            format!(
                "the {}-byte append never reached UpdateEnd; {}",
                big.len(),
                diagnostics(&rig)
            ),
        );
        if conclusive || !ended {
            break;
        }
    }
    if !conclusive {
        scn.note(
            "remove while updating is inconclusive: every append (up to 160 tiled copies, 24 MB) had already finished when append() returned, so updating was never observable as true",
        );
    }
    scn.metric("remove-while-updating.conclusive", conclusive);
    shutdown(scn, "c10-updating", rig);
    Ok(())
}

fn removed_does_not_play(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let stream = fixture("h264-low")?;
    let rig = new_rig(scn, timed_config(ctx))?;
    let [feed] = one_feed(&rig, &stream)?;
    append_inits(&rig, &[feed])?;
    append_media(&rig, &[feed], 0..4)?;
    remove_and_wait(&rig, feed.id, t_ms(0), Some(t_ms(4_000)))?;

    // Informational: play from 0 with no flush. Frames that were already downstream when the removal happened
    // (the preroll picture, decoder and queue contents) may still show; they are counted, not judged.
    reset_stats(&rig);
    rig.play()?;
    std::thread::sleep(Duration::from_millis(1_200));
    let leak = vstats(&rig);
    scn.metric("removed.frames_without_flush", leak.count)
        .metric(
            "removed.first_pts_without_flush_ms",
            leak.first_pts.map(ms_of),
        )
        .metric(
            "removed.last_pts_without_flush_ms",
            leak.last_pts.map(ms_of),
        );
    if leak.first_pts.is_some_and(|pts| ms_of(pts) < 4_000) {
        scn.note(format!(
            "playing from 0 after remove(0, 4 s) without a flush still rendered {} frame(s), pts {} to {}: data that was already downstream of msesrc is not taken back (informational)",
            leak.count,
            fmt_opt(leak.first_pts),
            fmt_opt(leak.last_pts)
        ));
    }

    // The judged part: pause, settle, flush by seeking into the removed range, play and watch for 3.6 s.
    rig.pause()?;
    std::thread::sleep(Duration::from_millis(300));
    reset_stats(&rig);
    rig.seek(t_ms(1_000))?;
    rig.play()?;
    let started = Instant::now();
    let hold = hold_and_watch(&rig, Duration::from_millis(3_600), t_ms(0));
    let after = vstats(&rig);
    scn.metric("removed.frames_after_seek_into_removed", after.count)
        .metric("removed.max_position_ms", hold.max_position.map(ms_of))
        .metric(
            "removed.hold_ms",
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
    scn.check(
        "removed-range-does-not-play",
        after.count == 0,
        format!(
            "{} video frame(s) were rendered (pts {} to {}) within 3.6 s of seeking to 1.0 s, where [0, 4 s) was removed and 4-8 s is buffered; nothing may play until data for 1.0 s is appended; {}",
            after.count,
            fmt_opt(after.first_pts),
            fmt_opt(after.last_pts),
            diagnostics(&rig)
        ),
    );
    scn.check(
        "removed-range-position-holds",
        hold.max_position.is_none_or(|p| ms_of(p) <= 1_300),
        format!(
            "the position reached {} after seeking to 1.0 s into the removed range (it must hold at 1.0 s)",
            fmt_opt(hold.max_position)
        ),
    );
    shutdown(scn, "c10-plays", rig);
    Ok(())
}
