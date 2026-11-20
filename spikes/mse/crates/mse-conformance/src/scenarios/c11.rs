// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C11 (P0): end of stream and duration. Four sub-runs, each in a fresh rig, video only (one source buffer: two deadlock GstMSE 1.28.9, see C02), fast mode:
//!
//! * `eos-none`: `end_of_stream(None)` after all data. The `MediaSource` goes closed -> open -> ended
//!   (events `SourceOpen`, `SourceEnded`); the element reaches the end (`PlayEvent::EndOfStream`) only after
//!   the last buffered frame has been rendered; an append after `ended` reopens it (`SourceOpen` again); detach
//!   closes it (`SourceClose`).
//! * `eos-error`: `end_of_stream(Network)` and `(Decode)` on fresh rigs. The call is accepted, the state is
//!   `ended`, and what surfaces (bus error, GstPlay error, nothing) is recorded: when nothing does, the DOM must
//!   raise the `MediaError` itself.
//! * `append-after-ended`: segments 0-1, `end_of_stream`, then segments 2-3 appended in the `ended` state
//!   (the spec reopens), then `end_of_stream` again; all 200 frames must play to the end.
//! * `duration`: duration before any init segment and after each step, then `set_duration` smaller than the
//!   buffered end (spec: `InvalidState`), after removing the excess (spec: accepted), larger (accepted),
//!   `None`, while an append is updating (`InvalidState`), in the `ended` state (`InvalidState`); the element's
//!   duration notification must follow a change.

use std::time::Duration;

use mse_rig::backend::{EosError, MediaSourceReadyState, MseBackend, MseError};
use mse_rig::events::{EventLog, MseEvent};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;

use crate::probes::keep_going;
use crate::support::{
    LOOSE_TOL_MS, NOMINAL_FRAMES, Res, Scn, append_inits, append_media, diagnostics, dur_ms,
    errors_in, fast_config, fixture, fmt_opt, fmt_ranges, guarded, judge_colours, matches_ranges,
    ms_of, new_rig, nominal_colour, one_feed, ranges_json, remove_and_wait, run_scenario, shutdown,
    t_ms, update_ends, vstats, wait_buffered, wait_eos, wait_update_end,
};

pub const TITLE: &str =
    "end of stream and duration: ended state, error EOS, append after ended, set_duration outcomes";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C11", TITLE, true, body)
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let outcome = eos_none(ctx, scn);
    keep_going(scn, "eos-none", outcome)?;
    let outcome = eos_error(ctx, scn);
    keep_going(scn, "eos-error", outcome)?;
    let outcome = append_after_ended(ctx, scn);
    keep_going(scn, "append-after-ended", outcome)?;
    let outcome = durations(ctx, scn);
    keep_going(scn, "duration", outcome)?;
    Ok(())
}

fn lifecycle(log: &EventLog) -> Vec<&'static str> {
    log.snapshot()
        .iter()
        .filter_map(|timed| match timed.event {
            MseEvent::SourceOpen => Some("open"),
            MseEvent::SourceEnded => Some("ended"),
            MseEvent::SourceClose => Some("close"),
            _ => None,
        })
        .collect()
}

fn wait_lifecycle(log: &EventLog, wanted: &[&'static str]) -> bool {
    let wanted = wanted.to_vec();
    log.wait_for(Duration::from_secs(2), move |events| {
        let seen: Vec<&str> = events
            .iter()
            .filter_map(|timed| match timed.event {
                MseEvent::SourceOpen => Some("open"),
                MseEvent::SourceEnded => Some("ended"),
                MseEvent::SourceClose => Some("close"),
                _ => None,
            })
            .collect();
        seen == wanted
    })
}

fn eos_none(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let video = fixture("h264-low")?;

    // closed: a MediaSource that is not attached to anything
    let probe = MseBackend::new(EventLog::new())?;
    scn.check(
        "eos-none.closed-before-attach",
        probe.ready_state() == MediaSourceReadyState::Closed,
        format!(
            "a new MediaSource is {:?}, expected Closed",
            probe.ready_state()
        ),
    );
    drop(probe);

    let rig = new_rig(scn, fast_config(ctx))?;
    let feeds = one_feed(&rig, &video)?;
    scn.check(
        "eos-none.open-after-attach",
        rig.backend.ready_state() == MediaSourceReadyState::Open,
        format!(
            "after attach the MediaSource is {:?}, expected Open",
            rig.backend.ready_state()
        ),
    );
    append_inits(&rig, &feeds)?;
    append_media(&rig, &feeds, 0..4)?;
    rig.backend.end_of_stream(EosError::None)?;
    let ended_event = wait_lifecycle(&rig.log, &["open", "ended"]);
    scn.check(
        "eos-none.ended",
        ended_event && rig.backend.ready_state() == MediaSourceReadyState::Ended,
        format!(
            "after end_of_stream(None) the MediaSource is {:?} and its events are {:?} (expected Ended and [open, ended])",
            rig.backend.ready_state(),
            lifecycle(&rig.log)
        ),
    );
    rig.play()?;
    let eos = wait_eos(&rig, Duration::from_secs(20));
    let at_eos = vstats(&rig);
    scn.metric("eos-none.video_frames_at_eos", at_eos.count)
        .metric(
            "eos-none.video_last_pts_at_eos_ms",
            at_eos.last_pts.map(ms_of),
        );
    scn.check(
        "eos-none.element-eos",
        eos.reached,
        format!(
            "no EndOfStream after end_of_stream(None) and play: {}",
            diagnostics(&rig)
        ),
    );
    if eos.reached {
        scn.check(
            "eos-none.eos-after-last-frame",
            at_eos.count.abs_diff(NOMINAL_FRAMES) <= 2
                && at_eos.last_pts.is_some_and(|pts| ms_of(pts) >= 7_900),
            format!(
                "at the moment EndOfStream arrived the video sink had {} frames (last pts {}): the stream ended before the last buffered frame was rendered",
                at_eos.count,
                fmt_opt(at_eos.last_pts)
            ),
        );
    }

    // append after ended: reopens
    let before_append = update_ends(&rig.log, feeds[0].id);
    let result = rig.backend.append(feeds[0].id, &video.media[3].bytes);
    scn.metric("eos-none.append_after_ended_result", format!("{result:?}"));
    let ok = scn.check(
        "eos-none.append-after-ended-accepted",
        result.is_ok(),
        format!(
            "append in the ended state was refused: {result:?} (spec: it reopens the MediaSource)"
        ),
    );
    if ok {
        let reopened = wait_lifecycle(&rig.log, &["open", "ended", "open"]);
        scn.check(
            "eos-none.reopened",
            reopened && rig.backend.ready_state() == MediaSourceReadyState::Open,
            format!(
                "after an append in the ended state the MediaSource is {:?} and its events are {:?} (expected Open and [open, ended, open])",
                rig.backend.ready_state(),
                lifecycle(&rig.log)
            ),
        );
        let completed = wait_update_end(&rig, feeds[0].id, before_append, Duration::from_secs(5));
        scn.check(
            "eos-none.append-after-ended-completes",
            completed,
            "the append made in the ended state never reached UpdateEnd within 5 s (the MediaSource did reopen)",
        );
        if completed {
            let again = guarded(&rig, Duration::from_secs(5), |backend| {
                backend.end_of_stream(EosError::None)
            });
            scn.check(
                "eos-none.ended-again",
                matches!(again, Some(Ok(())))
                    && wait_lifecycle(&rig.log, &["open", "ended", "open", "ended"]),
                format!(
                    "the second end_of_stream returned {again:?} and the events are {:?}",
                    lifecycle(&rig.log)
                ),
            );
        }
    }
    let detached = guarded(&rig, Duration::from_secs(5), |backend| backend.detach());
    scn.check(
        "eos-none.detach-returns",
        detached.is_some(),
        "detach() did not return within 5 s",
    );
    let closed = wait_lifecycle(&rig.log, &["open", "ended", "open", "ended", "close"])
        || wait_lifecycle(&rig.log, &["open", "ended", "close"]);
    scn.check(
        "eos-none.closed-after-detach",
        closed && rig.backend.ready_state() == MediaSourceReadyState::Closed,
        format!(
            "after detach the MediaSource is {:?} and its events are {:?} (expected Closed and a trailing close)",
            rig.backend.ready_state(),
            lifecycle(&rig.log)
        ),
    );
    scn.metric("eos-none.lifecycle", lifecycle(&rig.log));
    shutdown(scn, "c11-eos-none", rig);
    Ok(())
}

fn eos_error(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let video = fixture("h264-low")?;
    for (name, kind) in [("network", EosError::Network), ("decode", EosError::Decode)] {
        let rig = new_rig(scn, fast_config(ctx))?;
        let feeds = one_feed(&rig, &video)?;
        append_inits(&rig, &feeds)?;
        append_media(&rig, &feeds, 0..2)?;
        rig.play()?;
        rig.wait_until(Duration::from_secs(8), |r| {
            crate::support::video_count(r) >= 50
        });
        let mark = rig.log.snapshot().len();
        let result = rig.backend.end_of_stream(kind);
        scn.metric(&format!("eos-error.{name}.result"), format!("{result:?}"));
        scn.check(
            &format!("eos-error.{name}.accepted"),
            result.is_ok(),
            format!("end_of_stream({kind:?}) was refused: {result:?}"),
        );
        std::thread::sleep(Duration::from_millis(1_500));
        let after: Vec<String> = rig
            .log
            .snapshot()
            .iter()
            .skip(mark)
            .filter(|timed| {
                !matches!(
                    timed.event,
                    MseEvent::Play(mse_rig::events::PlayEvent::PositionUpdated(_))
                )
            })
            .map(|timed| {
                format!(
                    "+{} ms [{}] {:?}",
                    timed.at.as_millis(),
                    timed.thread,
                    timed.event
                )
            })
            .collect();
        let errors = errors_in(&rig.log.snapshot()[mark.min(rig.log.snapshot().len())..]);
        scn.metric(&format!("eos-error.{name}.events_after"), after.clone())
            .metric(
                &format!("eos-error.{name}.error_surfaced"),
                !errors.is_empty(),
            )
            .metric(
                &format!("eos-error.{name}.ready_state"),
                format!("{:?}", rig.backend.ready_state()),
            );
        scn.check(
            &format!("eos-error.{name}.ended"),
            rig.backend.ready_state() == MediaSourceReadyState::Ended,
            format!(
                "after end_of_stream({kind:?}) the MediaSource is {:?}, expected Ended",
                rig.backend.ready_state()
            ),
        );
        if errors.is_empty() {
            scn.note(format!(
                "end_of_stream({kind:?}): no error reached the bus, GstPlay or the source buffers (events after the call: {}); the DOM must raise MEDIA_ERR_{} on the element itself",
                after.join("; "),
                name.to_uppercase()
            ));
        } else {
            scn.note(format!(
                "end_of_stream({kind:?}) surfaced: {}",
                errors.join("; ")
            ));
        }
        shutdown(scn, &format!("c11-eos-{name}"), rig);
    }
    Ok(())
}

fn append_after_ended(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let video = fixture("h264-low")?;
    let rig = new_rig(scn, fast_config(ctx))?;
    let feeds = one_feed(&rig, &video)?;
    append_inits(&rig, &feeds)?;
    append_media(&rig, &feeds, 0..2)?;
    rig.backend.end_of_stream(EosError::None)?;
    scn.check(
        "append-after-ended.ended",
        wait_lifecycle(&rig.log, &["open", "ended"]),
        format!(
            "after end_of_stream the events are {:?}",
            lifecycle(&rig.log)
        ),
    );
    let more = append_media(&rig, &feeds, 2..3);
    if let Err(error) = more {
        scn.finding(format!(
            "appending segment 2 in the ended state failed: {error:?} (spec: the append reopens the MediaSource)"
        ));
        shutdown(scn, "c11-after-ended", rig);
        return Ok(());
    }
    scn.check(
        "append-after-ended.reopened",
        rig.backend.ready_state() == MediaSourceReadyState::Open
            && lifecycle(&rig.log).ends_with(&["ended", "open"]),
        format!(
            "after an append in the ended state the MediaSource is {:?}, events {:?} (expected Open and a second open)",
            rig.backend.ready_state(),
            lifecycle(&rig.log)
        ),
    );
    append_media(&rig, &feeds, 3..4)?;
    rig.backend.end_of_stream(EosError::None)?;
    rig.play()?;
    let eos = wait_eos(&rig, Duration::from_secs(20));
    let v = vstats(&rig);
    scn.metric("append-after-ended.video_frames", v.count);
    scn.check(
        "append-after-ended.eos",
        eos.reached,
        format!("no EndOfStream: {}", diagnostics(&rig)),
    );
    scn.check(
        "append-after-ended.all-frames",
        v.count.abs_diff(NOMINAL_FRAMES) <= 2,
        format!(
            "{} frames played (expected {NOMINAL_FRAMES} +- 2): data appended after ended did not all play; {}",
            v.count,
            diagnostics(&rig)
        ),
    );
    let verdict = judge_colours(&v.colour, nominal_colour, t_ms(40));
    scn.check(
        "append-after-ended.colour",
        v.colour.len() >= 4 && verdict.ok(),
        format!("{}: {}", verdict.summary(), verdict.wrong.join("; ")),
    );
    shutdown(scn, "c11-after-ended", rig);
    Ok(())
}

fn durations(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let video = fixture("h264-low")?;
    let rig = new_rig(scn, fast_config(ctx))?;
    let feeds = one_feed(&rig, &video)?;
    let ids = (feeds[0].id,);

    let before_init = (rig.backend.duration(), rig.element_duration());
    scn.metric("duration.before_init_backend", before_init.0.map(ms_of))
        .metric("duration.before_init_element", before_init.1.map(ms_of));
    append_inits(&rig, &feeds)?;
    rig.wait_until(Duration::from_millis(300), |r| {
        r.element_duration().is_some()
    });
    let after_init = (rig.backend.duration(), rig.element_duration());
    scn.metric("duration.after_init_backend", after_init.0.map(ms_of))
        .metric("duration.after_init_element", after_init.1.map(ms_of));
    append_media(&rig, &feeds, 0..4)?;
    // GstMSE finishes appends before the data shows in `buffered` and in the duration: let it settle first,
    // so that the `set_duration` rules below are judged against 8 s of buffered data.
    let settle = wait_buffered(
        &rig,
        feeds[0].id,
        Duration::from_secs(5),
        matches_ranges(vec![(0, 8_000)], LOOSE_TOL_MS),
    )?;
    scn.metric(
        "duration.buffered_at_updateend",
        ranges_json(&settle.immediate),
    )
    .metric("duration.buffered_settle_ms", dur_ms(settle.elapsed));
    scn.check(
        "duration.buffered-visible-at-updateend",
        settle.immediate_ok,
        format!(
            "right after appending everything buffered was {}, it showed [0.000-8.000] {} ms later",
            fmt_ranges(&settle.immediate),
            dur_ms(settle.elapsed)
        ),
    );
    rig.wait_until(Duration::from_secs(3), |r| {
        r.element_duration().is_some_and(|d| ms_of(d) >= 7_900)
    });
    let after_media = (rig.backend.duration(), rig.element_duration());
    scn.metric("duration.after_media_backend", after_media.0.map(ms_of))
        .metric("duration.after_media_element", after_media.1.map(ms_of));
    scn.note(format!(
        "duration (MediaSource / element): before any init segment {} / {}; after the init segments {} / {}; after all media {} / {} (none is unknown or NaN)",
        fmt_opt(before_init.0),
        fmt_opt(before_init.1),
        fmt_opt(after_init.0),
        fmt_opt(after_init.1),
        fmt_opt(after_media.0),
        fmt_opt(after_media.1),
    ));

    // smaller than the buffered end: spec says InvalidState
    let result = rig.backend.set_duration(Some(t_ms(2_000)));
    scn.metric("duration.set_2s_with_8s_buffered", format!("{result:?}"));
    scn.check(
        "duration.smaller-than-buffered-refused",
        matches!(result, Err(MseError::InvalidState(_))),
        format!(
            "set_duration(2 s) with data buffered to 8 s returned {result:?} (spec: InvalidStateError); the duration is now {}",
            fmt_opt(rig.backend.duration())
        ),
    );

    // after removing the excess: accepted
    remove_and_wait(&rig, ids.0, t_ms(2_000), None)?;
    let result = rig.backend.set_duration(Some(t_ms(2_000)));
    scn.metric("duration.set_2s_after_removal", format!("{result:?}"));
    scn.check(
        "duration.smaller-after-removal-accepted",
        result.is_ok() && rig.backend.duration().is_some_and(|d| (ms_of(d) - 2_000).abs() <= 5),
        format!(
            "set_duration(2 s) after removing everything from 2 s on returned {result:?} and the duration is {} (spec: accepted)",
            fmt_opt(rig.backend.duration())
        ),
    );

    // larger: accepted, and the element hears about it
    let before = rig.log.snapshot().len();
    let result = rig.backend.set_duration(Some(t_ms(20_000)));
    let notified = rig.log.wait_for(Duration::from_secs(1), |events| {
        events.iter().skip(before).any(|timed| {
            matches!(&timed.event, MseEvent::ElementDuration(Some(d)) if (ms_of(*d) - 20_000).abs() <= 5)
        })
    });
    scn.metric("duration.set_20s", format!("{result:?}"))
        .metric("duration.set_20s_element_notified", notified);
    scn.check(
        "duration.larger-accepted",
        result.is_ok()
            && rig
                .backend
                .duration()
                .is_some_and(|d| (ms_of(d) - 20_000).abs() <= 5),
        format!(
            "set_duration(20 s) returned {result:?} and the duration is {} (spec: accepted)",
            fmt_opt(rig.backend.duration())
        ),
    );
    scn.check(
        "duration.element-notified",
        notified,
        format!(
            "no ElementDuration(20 s) notification within 1 s of set_duration(20 s); the element reports {}",
            fmt_opt(rig.element_duration())
        ),
    );

    // None: unknown / +Infinity (recorded)
    let result = rig.backend.set_duration(None);
    scn.metric("duration.set_none", format!("{result:?}"))
        .metric(
            "duration.after_set_none_backend",
            rig.backend.duration().map(ms_of),
        );
    scn.note(format!(
        "set_duration(None) (NaN or +Infinity: the API cannot tell them apart) returned {result:?}; the duration afterwards is {}",
        fmt_opt(rig.backend.duration())
    ));

    // while an append is updating
    let tiled = mse_segments::tile(&video, 40)?;
    let big: Vec<u8> = tiled
        .media
        .iter()
        .flat_map(|s| s.bytes.iter().copied())
        .collect();
    let before_ends = update_ends(&rig.log, ids.0);
    rig.backend.append(ids.0, &big)?;
    if rig.backend.updating(ids.0) {
        let result = rig.backend.set_duration(Some(t_ms(400_000)));
        scn.metric("duration.set_while_updating", format!("{result:?}"));
        scn.check(
            "duration.set-while-updating-refused",
            matches!(result, Err(MseError::InvalidState(_))),
            format!("set_duration() while an append was updating returned {result:?} (spec: InvalidStateError)"),
        );
    } else {
        scn.note("set_duration while updating is inconclusive: the 6 MB append had finished when append() returned");
    }
    rig.log.wait_for(Duration::from_secs(60), |events| {
        events
            .iter()
            .filter(|t| matches!(t.event, MseEvent::UpdateEnd(i) if i == ids.0))
            .count()
            > before_ends
    });

    // in the ended state
    rig.backend.end_of_stream(EosError::None)?;
    let result = rig.backend.set_duration(Some(t_ms(1_000)));
    scn.metric("duration.set_in_ended_state", format!("{result:?}"));
    scn.check(
        "duration.set-in-ended-state-refused",
        matches!(result, Err(MseError::InvalidState(_))),
        format!("set_duration() in the ended state returned {result:?} (spec: InvalidStateError, the state is not open)"),
    );
    shutdown(scn, "c11-duration", rig);
    Ok(())
}
