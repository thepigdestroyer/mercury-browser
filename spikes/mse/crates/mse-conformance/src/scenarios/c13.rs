// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C13 (P1): abort and errors. Eight cases, each in a fresh rig, each ending in a defined state or recorded
//! as a finding. "Defined" is the brief's: an `Error` event with `updating == false`, or an `InvalidState`
//! refusal, never a hang (an append that is still updating after 5 s) and never a crash (a crash kills the
//! child process: the progress lines on stderr, `[C13] case ...`, name the case that was running).
//!
//! * `abort-large`: `abort` during a large append (8 tiled copies, 1.2 MB); then the buffer must accept data again.
//! * `garbage`: 4096 bytes of deterministic noise as the first append.
//! * `media-before-init`: a media segment before any init segment.
//! * `webm-into-mp4`: a WebM init segment into an MP4 source buffer.
//! * `truncated`: a segment cut in half, then its remainder (must work: buffered [0, 2 s], all frames play);
//!   and a half segment followed by `abort` and a clean re-append.
//! * `init-twice`: the same init segment appended twice (a switch to the same rendition); everything plays.
//! * `append-to-removed`: `append` to a removed source buffer is refused (`InvalidState`).
//! * `append-while-updating`: a second append during an update is refused (`InvalidState`).

use std::time::Duration;

use mse_rig::backend::{EosError, MseError};
use mse_rig::events::MseEvent;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::Rig;
use mse_segments::SegmentedStream;

use crate::isolate;
use crate::probes::keep_going;
use crate::support::{
    Abort, Chunking, Feed, LOOSE_TOL_MS, NOMINAL_FRAMES, Res, Scn, XorShift, add_source_buffer,
    append_inits, append_media, check_buffered, diagnostics, errors_in, fast_config, fixture,
    judge_colours, new_rig, nominal_colour, run_scenario, shutdown, t_ms, update_ends, vstats,
    wait_eos,
};

pub const TITLE: &str = "abort and errors: abort mid-append, garbage, media before init, wrong container, truncation, init twice, removed buffer, append while updating";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C13", TITLE, true, body)
}

type CaseFn = fn(&Ctx, &mut Scn, &SegmentedStream) -> Res<()>;

/// How long an append may take before it counts as stuck.
const SETTLE: Duration = Duration::from_secs(5);

/// How long one case may take in its own process.
const CASE_TIMEOUT: Duration = Duration::from_secs(60);

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let low = fixture("h264-low")?;
    let cases: [(&str, CaseFn); 8] = [
        ("truncated", truncated),
        ("init-twice", init_twice),
        ("append-to-removed", append_to_removed),
        ("append-while-updating", append_while_updating),
        ("abort-large", abort_large),
        ("garbage", garbage),
        ("media-before-init", media_before_init),
        ("webm-into-mp4", webm_into_mp4),
    ];
    // A re-run of this scenario for one case (see `isolate`): run just that case here.
    if let Some(wanted) = isolate::selected_case() {
        let Some((name, case)) = cases.iter().find(|(name, _)| *name == wanted) else {
            return Err(Abort::Error(format!("C13 has no case {wanted:?}")));
        };
        eprintln!("[C13] case {name}");
        return case(ctx, scn, &low);
    }
    // Every case in a process of its own: a crash or hang in GstMSE costs that case only.
    for (name, _) in cases {
        eprintln!("[C13] case {name} (own process)");
        let outcome = isolate::run_case_in_child(ctx, scn, "C13", name, CASE_TIMEOUT);
        keep_going(scn, name, outcome)?;
    }
    Ok(())
}

/// What one append did.
#[derive(Debug)]
enum Observed {
    /// `append` returned an error at once.
    Refused(MseError),
    /// An `Error` or `Abort` event arrived and then `UpdateEnd`.
    Errored { updating: bool },
    /// `UpdateEnd` without an error.
    Completed { updating: bool },
    /// No `UpdateEnd` within the settle time.
    Stuck,
}

impl Observed {
    fn describe(&self) -> String {
        match self {
            Observed::Refused(error) => format!("refused at once: {error:?}"),
            Observed::Errored { updating } => {
                format!("Error event then UpdateEnd (updating={updating})")
            }
            Observed::Completed { updating } => {
                format!("UpdateEnd without an error (updating={updating})")
            }
            Observed::Stuck => format!("no UpdateEnd within {SETTLE:?}"),
        }
    }

    /// An error was raised in one of the two accepted ways.
    fn defined_error(&self) -> bool {
        matches!(
            self,
            Observed::Errored { updating: false } | Observed::Refused(MseError::InvalidState(_))
        )
    }

    fn completed_cleanly(&self) -> bool {
        matches!(self, Observed::Completed { updating: false })
    }
}

fn observe(rig: &Rig, id: usize, bytes: &[u8]) -> Observed {
    let mark = rig.log.snapshot().len();
    if let Err(error) = rig.backend.append(id, bytes) {
        return Observed::Refused(error);
    }
    let done = rig.log.wait_for(SETTLE, |events| {
        events
            .iter()
            .skip(mark)
            .any(|timed| matches!(timed.event, MseEvent::UpdateEnd(i) if i == id))
    });
    if !done {
        return Observed::Stuck;
    }
    let errored = rig
        .log
        .snapshot()
        .iter()
        .skip(mark)
        .any(|timed| matches!(timed.event, MseEvent::Error(i) | MseEvent::Abort(i) if i == id));
    let updating = rig.backend.updating(id);
    if errored {
        Observed::Errored { updating }
    } else {
        Observed::Completed { updating }
    }
}

fn single_feed<'a>(rig: &Rig, stream: &'a SegmentedStream) -> Res<Feed<'a>> {
    Ok(Feed {
        id: add_source_buffer(rig, stream)?,
        stream,
        chunking: Chunking::Whole,
    })
}

/// The shared judgment of the three "bad data" cases, then what state the MediaSource is left in.
fn judge_bad_data(
    scn: &mut Scn,
    rig: &Rig,
    case: &str,
    observed: &Observed,
    id: usize,
    good_init: &[u8],
) {
    scn.metric(&format!("{case}.outcome"), observed.describe());
    scn.check(
        &format!("{case}.defined-error"),
        observed.defined_error(),
        format!(
            "bad data ended as: {} (a defined outcome is an Error event with updating false, or an InvalidState refusal); {}",
            observed.describe(),
            diagnostics(rig)
        ),
    );
    scn.metric(
        &format!("{case}.mediasource_state_after"),
        format!("{:?}", rig.backend.ready_state()),
    );
    if !matches!(observed, Observed::Stuck) {
        let recovery = observe(rig, id, good_init);
        scn.metric(
            &format!("{case}.good_init_after_error"),
            recovery.describe(),
        );
        scn.note(format!(
            "{case}: after the bad append the MediaSource is {:?}; a good init segment appended next was: {}",
            rig.backend.ready_state(),
            recovery.describe()
        ));
    }
}

fn garbage(ctx: &Ctx, scn: &mut Scn, low: &SegmentedStream) -> Res<()> {
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = single_feed(&rig, low)?;
    let noise = XorShift::new(0x00C1_3C13).bytes(4096);
    let observed = observe(&rig, feed.id, &noise);
    judge_bad_data(scn, &rig, "garbage", &observed, feed.id, &low.init.bytes);
    shutdown(scn, "c13-garbage", rig);
    Ok(())
}

fn media_before_init(ctx: &Ctx, scn: &mut Scn, low: &SegmentedStream) -> Res<()> {
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = single_feed(&rig, low)?;
    let observed = observe(&rig, feed.id, &low.media[0].bytes);
    judge_bad_data(
        scn,
        &rig,
        "media-before-init",
        &observed,
        feed.id,
        &low.init.bytes,
    );
    shutdown(scn, "c13-media-first", rig);
    Ok(())
}

fn webm_into_mp4(ctx: &Ctx, scn: &mut Scn, low: &SegmentedStream) -> Res<()> {
    let vp9 = mse_segments::load("vp9-low")
        .map_err(|error| Abort::Skip(format!("fixture vp9-low is not available: {error}")))?;
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = single_feed(&rig, low)?;
    let observed = observe(&rig, feed.id, &vp9.init.bytes);
    judge_bad_data(
        scn,
        &rig,
        "webm-into-mp4",
        &observed,
        feed.id,
        &low.init.bytes,
    );
    shutdown(scn, "c13-webm-into-mp4", rig);
    Ok(())
}

fn truncated(ctx: &Ctx, scn: &mut Scn, low: &SegmentedStream) -> Res<()> {
    // Part 1: half a segment, then the rest, then the remaining segments; everything must play.
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = single_feed(&rig, low)?;
    append_inits(&rig, &[feed])?;
    let segment = &low.media[0].bytes;
    let cut = segment.len() / 2 + 3; // inside a sample, not on a box edge
    let first = observe(&rig, feed.id, &segment[..cut]);
    scn.metric("truncated.first_half", first.describe());
    scn.check(
        "truncated.first-half-accepted",
        first.completed_cleanly(),
        format!(
            "the first {cut} of {} bytes of a segment: {} (a partial segment is normal input)",
            segment.len(),
            first.describe()
        ),
    );
    let second = observe(&rig, feed.id, &segment[cut..]);
    scn.metric("truncated.remainder", second.describe());
    scn.check(
        "truncated.remainder-accepted",
        second.completed_cleanly(),
        format!(
            "the remaining {} bytes of the segment: {}",
            segment.len() - cut,
            second.describe()
        ),
    );
    check_buffered(
        scn,
        &rig,
        "truncated.buffered",
        feed.id,
        &[(0, 2_000)],
        LOOSE_TOL_MS,
    )?;
    append_media(&rig, &[feed], 1..4)?;
    rig.backend.end_of_stream(EosError::None)?;
    rig.play()?;
    let eos = wait_eos(&rig, Duration::from_secs(20));
    let stats = vstats(&rig);
    scn.metric("truncated.video_frames", stats.count);
    scn.check(
        "truncated.plays-all",
        eos.reached && stats.count.abs_diff(NOMINAL_FRAMES) <= 2,
        format!(
            "{} frames reached the sink (expected {NOMINAL_FRAMES} +- 2), end of stream {}; {}",
            stats.count,
            eos.reached,
            diagnostics(&rig)
        ),
    );
    let verdict = judge_colours(&stats.colour, nominal_colour, t_ms(40));
    scn.check(
        "truncated.picture",
        stats.colour.len() >= 4 && verdict.ok(),
        format!("{}: {}", verdict.summary(), verdict.wrong.join("; ")),
    );
    shutdown(scn, "c13-truncated", rig);

    // Part 2: half a segment, abort, then a clean start must work.
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = single_feed(&rig, low)?;
    append_inits(&rig, &[feed])?;
    let half = observe(&rig, feed.id, &segment[..cut]);
    scn.metric("truncated-abort.half", half.describe());
    let abort = rig.backend.abort(feed.id);
    scn.metric("truncated-abort.abort_result", format!("{abort:?}"));
    scn.check(
        "truncated-abort.abort-accepted",
        abort.is_ok(),
        format!("abort() after a partial segment returned {abort:?}"),
    );
    let again = observe(&rig, feed.id, &low.media[0].bytes);
    scn.metric("truncated-abort.clean_media_after_abort", again.describe());
    let recovered = if again.completed_cleanly() {
        true
    } else {
        // The parser may want its init segment again after an abort; that is allowed by the spec (it is not
        // required to keep one) but must work.
        let init = observe(&rig, feed.id, &low.init.bytes);
        let retry = observe(&rig, feed.id, &low.media[0].bytes);
        scn.note(format!(
            "truncated-abort: a whole media segment right after abort was {}; init again: {}; media again: {}",
            again.describe(),
            init.describe(),
            retry.describe()
        ));
        init.completed_cleanly() && retry.completed_cleanly()
    };
    check_buffered(
        scn,
        &rig,
        "truncated-abort.buffered",
        feed.id,
        &[(0, 2_000)],
        LOOSE_TOL_MS,
    )?;
    scn.check(
        "truncated-abort.recovers",
        recovered,
        "after half a segment and abort, a clean media segment (or init plus media) was not accepted",
    );
    shutdown(scn, "c13-truncated-abort", rig);
    Ok(())
}

fn init_twice(ctx: &Ctx, scn: &mut Scn, low: &SegmentedStream) -> Res<()> {
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = single_feed(&rig, low)?;
    append_inits(&rig, &[feed])?;
    let second = observe(&rig, feed.id, &low.init.bytes);
    scn.metric("init-twice.second_init", second.describe());
    scn.check(
        "init-twice.second-init-accepted",
        second.completed_cleanly(),
        format!(
            "the same init segment appended a second time: {}",
            second.describe()
        ),
    );
    append_media(&rig, &[feed], 0..4)?;
    rig.backend.end_of_stream(EosError::None)?;
    rig.play()?;
    let eos = wait_eos(&rig, Duration::from_secs(20));
    let stats = vstats(&rig);
    scn.metric("init-twice.video_frames", stats.count);
    scn.check(
        "init-twice.plays-all",
        eos.reached && stats.count.abs_diff(NOMINAL_FRAMES) <= 2,
        format!(
            "{} frames reached the sink (expected {NOMINAL_FRAMES} +- 2), end of stream {}; {}",
            stats.count,
            eos.reached,
            diagnostics(&rig)
        ),
    );
    let errors = errors_in(&rig.log.snapshot());
    scn.check(
        "init-twice.no-errors",
        errors.is_empty(),
        format!("error or abort events: {}", errors.join("; ")),
    );
    shutdown(scn, "c13-init-twice", rig);
    Ok(())
}

fn append_to_removed(ctx: &Ctx, scn: &mut Scn, low: &SegmentedStream) -> Res<()> {
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = single_feed(&rig, low)?;
    append_inits(&rig, &[feed])?;
    let removed = rig.backend.remove_source_buffer(feed.id);
    scn.metric("append-to-removed.remove_result", format!("{removed:?}"));
    scn.check(
        "append-to-removed.remove-accepted",
        removed.is_ok(),
        format!("remove_source_buffer returned {removed:?}"),
    );
    let announced = rig.log.wait_for(Duration::from_secs(1), |events| {
        events
            .iter()
            .any(|timed| matches!(timed.event, MseEvent::SourceBufferRemoved(i) if i == feed.id))
    });
    scn.check(
        "append-to-removed.removed-event",
        announced && rig.backend.source_buffer_count() == 0,
        format!(
            "after remove_source_buffer: SourceBufferRemoved event {announced}, {} source buffer(s) remain",
            rig.backend.source_buffer_count()
        ),
    );
    let result = rig.backend.append(feed.id, &low.media[0].bytes);
    scn.metric("append-to-removed.append_result", format!("{result:?}"));
    scn.check(
        "append-to-removed.append-refused",
        matches!(
            result,
            Err(MseError::InvalidState(_) | MseError::NotFound(_))
        ),
        format!("append to a removed source buffer returned {result:?} (spec: InvalidStateError)"),
    );
    if let Err(MseError::NotFound(_)) = result {
        scn.note("API-FIT: append to a removed source buffer is NotFound; the spec throws InvalidStateError");
    }
    shutdown(scn, "c13-removed", rig);
    Ok(())
}

fn append_while_updating(ctx: &Ctx, scn: &mut Scn, low: &SegmentedStream) -> Res<()> {
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = single_feed(&rig, low)?;
    append_inits(&rig, &[feed])?;
    let mut conclusive = false;
    for (attempt, times) in [8usize, 40, 160].into_iter().enumerate() {
        let tiled = mse_segments::tile(low, times)?;
        let big: Vec<u8> = tiled
            .media
            .iter()
            .flat_map(|s| s.bytes.iter().copied())
            .collect();
        let before = update_ends(&rig.log, feed.id);
        rig.backend.append(feed.id, &big)?;
        if rig.backend.updating(feed.id) {
            let second = rig.backend.append(feed.id, &low.media[0].bytes);
            scn.metric("append-while-updating.second_append", format!("{second:?}"));
            scn.check(
                "append-while-updating.refused",
                matches!(second, Err(MseError::InvalidState(_))),
                format!("a second append during an update of {} bytes returned {second:?} (spec: InvalidStateError)", big.len()),
            );
            conclusive = true;
        }
        let ended = rig.log.wait_for(Duration::from_secs(60), |events| {
            events
                .iter()
                .filter(|t| matches!(t.event, MseEvent::UpdateEnd(i) if i == feed.id))
                .count()
                > before
        });
        scn.check(
            &format!("append-while-updating.attempt{attempt}.ends"),
            ended && !rig.backend.updating(feed.id),
            format!(
                "the large append did not end cleanly; {}",
                diagnostics(&rig)
            ),
        );
        if conclusive || !ended {
            break;
        }
    }
    scn.metric("append-while-updating.conclusive", conclusive);
    if !conclusive {
        scn.note("append while updating is inconclusive: every append (up to 24 MB) had finished before append() returned");
    }
    shutdown(scn, "c13-while-updating", rig);
    Ok(())
}

fn abort_large(ctx: &Ctx, scn: &mut Scn, low: &SegmentedStream) -> Res<()> {
    let rig = new_rig(scn, fast_config(ctx))?;
    let feed = single_feed(&rig, low)?;
    append_inits(&rig, &[feed])?;
    let tiled = mse_segments::tile(low, 8)?;
    let big: Vec<u8> = tiled
        .media
        .iter()
        .flat_map(|s| s.bytes.iter().copied())
        .collect();
    let mut conclusive = false;
    for attempt in 0..5 {
        let mark = rig.log.snapshot().len();
        rig.backend.append(feed.id, &big)?;
        let updating = rig.backend.updating(feed.id);
        let aborted = rig.backend.abort(feed.id);
        scn.metric(
            &format!("abort-large.attempt{attempt}.updating_at_abort"),
            updating,
        )
        .metric(
            &format!("abort-large.attempt{attempt}.abort_result"),
            format!("{aborted:?}"),
        );
        if !updating {
            // The append finished before abort could run: wait it out and try again.
            rig.log.wait_for(Duration::from_secs(10), |events| {
                events
                    .iter()
                    .skip(mark)
                    .any(|t| matches!(t.event, MseEvent::UpdateEnd(i) if i == feed.id))
            });
            continue;
        }
        conclusive = true;
        scn.check(
            "abort-large.abort-accepted",
            aborted.is_ok(),
            format!("abort() during an update returned {aborted:?}"),
        );
        let settled = rig.wait_until(Duration::from_secs(2), |r| !r.backend.updating(feed.id));
        scn.check(
            "abort-large.not-updating-after",
            settled,
            format!(
                "updating is still true 2 s after abort(); {}",
                diagnostics(&rig)
            ),
        );
        let events = rig.log.snapshot();
        let saw_abort = events
            .iter()
            .skip(mark)
            .any(|t| matches!(t.event, MseEvent::Abort(i) if i == feed.id));
        let ended = events
            .iter()
            .skip(mark)
            .any(|t| matches!(t.event, MseEvent::UpdateEnd(i) if i == feed.id));
        scn.check(
            "abort-large.abort-and-updateend-events",
            saw_abort && ended,
            format!("abort during an update produced Abort event {saw_abort} and UpdateEnd {ended} (spec: abort, then updateend)"),
        );
        // The buffer must take data again.
        let fresh = observe(&rig, feed.id, &low.media[0].bytes);
        let usable = if fresh.completed_cleanly() {
            true
        } else {
            let init = observe(&rig, feed.id, &low.init.bytes);
            let retry = observe(&rig, feed.id, &low.media[0].bytes);
            scn.note(format!(
                "abort-large: a media segment right after abort was {}; init again {}; media again {}",
                fresh.describe(),
                init.describe(),
                retry.describe()
            ));
            init.completed_cleanly() && retry.completed_cleanly()
        };
        scn.check(
            "abort-large.usable-after",
            usable,
            format!(
                "the source buffer did not accept data after abort(): {}",
                fresh.describe()
            ),
        );
        break;
    }
    scn.metric("abort-large.conclusive", conclusive);
    if !conclusive {
        scn.note("abort during an append is inconclusive: five 1.2 MB appends all finished before abort() could be called");
    }
    shutdown(scn, "c13-abort", rig);
    Ok(())
}
