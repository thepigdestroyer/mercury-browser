// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C14 (P1): the event and state mapping. One real-time run (`sync=true`, about 6 s of media) that goes
//! through everything an HTMLMediaElement reports:
//!
//! - both init segments;
//! - segments 0-1 of both tracks (4 s buffered);
//! - `play()`, playback to the end of the buffered data and a 1.2 s stall (the underrun);
//! - segment 2 appended (recovery);
//! - segment 3 appended and a seek inside the buffered range to 6.5 s;
//! - `end_of_stream` and the end.
//!
//! From the event log (every callback with its thread and time) the table of the brief is built: for each
//! HTMLMediaElement event (`loadedmetadata`, `loadeddata`, `canplay`, `canplaythrough`, `waiting`, `playing`,
//! `seeking`, `seeked`, `timeupdate`, `durationchange`, `ended`, `stalled`) which GstMSE or GstPlay signal drives
//! it, when it first appears and how often, or that none does (then the DOM computes it from `buffered` and the
//! position). The table is in the metrics (`event_mapping`, `timeline`, `threads`) and in the notes, ready to
//! paste into R17.
//!
//! Pass when the run went through every phase and every row of the table has an answer (a driver or "none").
//! A missing driver is not a failure here: it is the answer.

use std::collections::BTreeMap;
use std::time::Duration;

use mse_rig::backend::EosError;
use mse_rig::events::{ElementReadyState, MseEvent, PlayEvent, TimedEvent};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use serde_json::json;

use crate::probes::keep_going;
use crate::support::{
    Res, Scn, append_inits, append_media, diagnostics, fixture, fmt_opt, fmt_ready_states, new_rig,
    one_feed, ready_state_events, run_scenario, shutdown, t_ms, threads_of, timed_config, wait_eos,
};

pub const TITLE: &str =
    "event and state mapping: which GstMSE/GstPlay signal drives each HTMLMediaElement event";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C14", TITLE, true, body)
}

/// Indices into the event log at the phase boundaries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Marks {
    pub init_start: usize,
    pub after_init: usize,
    pub media_start: usize,
    pub play: usize,
    pub stall: usize,
    pub resume: usize,
    pub seek: usize,
    pub eos: usize,
    pub end: usize,
}

/// One candidate signal for an HTMLMediaElement event: a predicate over events in a window of the log.
struct Candidate {
    signal: &'static str,
    from: usize,
    to: usize,
    matches: fn(&MseEvent) -> bool,
}

struct RowSpec {
    event: &'static str,
    meaning: &'static str,
    candidates: Vec<Candidate>,
    /// What the DOM does when no signal drives it.
    fallback: &'static str,
}

/// One row of the answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub event: &'static str,
    pub meaning: &'static str,
    /// (signal, first seen in ms since the log started, count) for each candidate that appeared.
    pub drivers: Vec<(&'static str, u128, usize)>,
    pub answer: String,
}

fn ready_at_least(state: ElementReadyState) -> fn(&MseEvent) -> bool {
    // fn pointers cannot capture: one function per state.
    match state {
        ElementReadyState::HaveNothing => |e| {
            matches!(
                e,
                MseEvent::ElementReadyState(ElementReadyState::HaveNothing)
            )
        },
        ElementReadyState::HaveMetadata => |e| {
            matches!(
                e,
                MseEvent::ElementReadyState(ElementReadyState::HaveMetadata)
            )
        },
        ElementReadyState::HaveCurrentData => |e| {
            matches!(
                e,
                MseEvent::ElementReadyState(ElementReadyState::HaveCurrentData)
            )
        },
        ElementReadyState::HaveFutureData => |e| {
            matches!(
                e,
                MseEvent::ElementReadyState(ElementReadyState::HaveFutureData)
            )
        },
        ElementReadyState::HaveEnoughData => |e| {
            matches!(
                e,
                MseEvent::ElementReadyState(ElementReadyState::HaveEnoughData)
            )
        },
    }
}

fn specs(m: &Marks) -> Vec<RowSpec> {
    let all_end = m.end;
    vec![
        RowSpec {
            event: "loadedmetadata",
            meaning: "readyState reaches HAVE_METADATA after the init segments",
            candidates: vec![
                Candidate {
                    signal: "msesrc notify::ready-state = HaveMetadata",
                    from: m.init_start,
                    to: all_end,
                    matches: ready_at_least(ElementReadyState::HaveMetadata),
                },
                Candidate {
                    signal: "GstPlay media-info-updated",
                    from: m.init_start,
                    to: all_end,
                    matches: |e| matches!(e, MseEvent::Play(PlayEvent::MediaInfoUpdated)),
                },
                Candidate {
                    signal: "GstPlay duration-changed",
                    from: m.init_start,
                    to: all_end,
                    matches: |e| matches!(e, MseEvent::Play(PlayEvent::DurationChanged(Some(_)))),
                },
            ],
            fallback: "the DOM fires it itself when the init segment has been parsed (UpdateEnd of the first append)",
        },
        RowSpec {
            event: "loadeddata",
            meaning: "readyState reaches HAVE_CURRENT_DATA (first frame available)",
            candidates: vec![
                Candidate {
                    signal: "msesrc notify::ready-state = HaveCurrentData",
                    from: m.init_start,
                    to: all_end,
                    matches: ready_at_least(ElementReadyState::HaveCurrentData),
                },
                Candidate {
                    signal: "GstPlay state-changed = Paused (preroll done)",
                    from: m.media_start,
                    to: all_end,
                    matches: |e| matches!(e, MseEvent::Play(PlayEvent::StateChanged(s)) if s == "Paused"),
                },
            ],
            fallback: "the DOM computes it: buffered covers the position and a frame was prerolled",
        },
        RowSpec {
            event: "canplay",
            meaning: "readyState reaches HAVE_FUTURE_DATA",
            candidates: vec![Candidate {
                signal: "msesrc notify::ready-state = HaveFutureData",
                from: m.init_start,
                to: all_end,
                matches: ready_at_least(ElementReadyState::HaveFutureData),
            }],
            fallback: "the DOM computes it from buffered ahead of the position",
        },
        RowSpec {
            event: "canplaythrough",
            meaning: "readyState reaches HAVE_ENOUGH_DATA",
            candidates: vec![Candidate {
                signal: "msesrc notify::ready-state = HaveEnoughData",
                from: m.init_start,
                to: all_end,
                matches: ready_at_least(ElementReadyState::HaveEnoughData),
            }],
            fallback: "the DOM computes it from buffered ahead of the position",
        },
        RowSpec {
            event: "waiting",
            meaning: "playback stops for lack of data (readyState falls below HAVE_FUTURE_DATA while playing)",
            candidates: vec![
                Candidate {
                    signal: "msesrc notify::ready-state falls (HaveMetadata or HaveCurrentData) during the stall",
                    from: m.stall,
                    to: m.resume,
                    matches: |e| {
                        matches!(
                            e,
                            MseEvent::ElementReadyState(
                                ElementReadyState::HaveMetadata
                                    | ElementReadyState::HaveCurrentData
                            )
                        )
                    },
                },
                Candidate {
                    signal: "GstPlay buffering < 100 during the stall",
                    from: m.stall,
                    to: m.resume,
                    matches: |e| matches!(e, MseEvent::Play(PlayEvent::Buffering(p)) if *p < 100),
                },
                Candidate {
                    signal: "GstPlay state-changed = Paused during the stall",
                    from: m.stall,
                    to: m.resume,
                    matches: |e| matches!(e, MseEvent::Play(PlayEvent::StateChanged(s)) if s == "Paused"),
                },
            ],
            fallback: "the DOM polls position against buffered at the end of the buffered range",
        },
        RowSpec {
            event: "playing",
            meaning: "playback starts or resumes",
            candidates: vec![Candidate {
                signal: "GstPlay state-changed = Playing",
                from: m.play,
                to: all_end,
                matches: |e| matches!(e, MseEvent::Play(PlayEvent::StateChanged(s)) if s == "Playing"),
            }],
            fallback: "the DOM fires it when it asked for play and readyState >= HAVE_FUTURE_DATA",
        },
        RowSpec {
            event: "seeking",
            meaning: "a seek starts",
            candidates: vec![],
            fallback: "no signal: the DOM sets it when currentTime is assigned (GstPlay has no seek-started message)",
        },
        RowSpec {
            event: "seeked",
            meaning: "a seek completes",
            candidates: vec![Candidate {
                signal: "GstPlay seek-done (Bare: bus AsyncDone after a seek)",
                from: m.seek,
                to: all_end,
                matches: |e| matches!(e, MseEvent::Play(PlayEvent::SeekDone(_))),
            }],
            fallback: "the DOM computes it: position reached the target and data is available",
        },
        RowSpec {
            event: "timeupdate",
            meaning: "the position changed",
            candidates: vec![
                Candidate {
                    signal: "GstPlay position-updated (every 500 ms in the rig, as Servo configures it)",
                    from: m.play,
                    to: all_end,
                    matches: |e| matches!(e, MseEvent::Play(PlayEvent::PositionUpdated(_))),
                },
                Candidate {
                    signal: "msesrc notify::position (only when logged)",
                    from: m.play,
                    to: all_end,
                    matches: |e| matches!(e, MseEvent::ElementPosition(_)),
                },
            ],
            fallback: "the DOM polls the pipeline position (query_position) on a timer",
        },
        RowSpec {
            event: "durationchange",
            meaning: "the duration changed",
            candidates: vec![
                Candidate {
                    signal: "msesrc notify::duration",
                    from: m.init_start,
                    to: all_end,
                    matches: |e| matches!(e, MseEvent::ElementDuration(_)),
                },
                Candidate {
                    signal: "GstPlay duration-changed",
                    from: m.init_start,
                    to: all_end,
                    matches: |e| matches!(e, MseEvent::Play(PlayEvent::DurationChanged(_))),
                },
            ],
            fallback: "the DOM fires it from MediaSource.duration changes",
        },
        RowSpec {
            event: "ended",
            meaning: "playback reached the end of the stream",
            candidates: vec![Candidate {
                signal: "GstPlay end-of-stream (bus EOS)",
                from: m.eos,
                to: all_end,
                matches: |e| matches!(e, MseEvent::Play(PlayEvent::EndOfStream)),
            }],
            fallback: "the DOM computes it: position at duration and the MediaSource ended",
        },
        RowSpec {
            event: "stalled",
            meaning: "the user agent is trying to fetch media but data is not arriving",
            candidates: vec![],
            fallback: "no signal: it is about the network fetch, which the page performs; the DOM never fires it for MSE-fed media",
        },
    ]
}

/// Builds the table from the log.
pub fn build_mapping(events: &[TimedEvent], marks: &Marks) -> Vec<Row> {
    specs(marks)
        .into_iter()
        .map(|spec| {
            let mut drivers = Vec::new();
            for candidate in &spec.candidates {
                let window = events
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        *index >= candidate.from && *index < candidate.to.min(events.len())
                    })
                    .map(|(_, timed)| timed)
                    .filter(|timed| (candidate.matches)(&timed.event))
                    .collect::<Vec<_>>();
                if let Some(first) = window.first() {
                    drivers.push((candidate.signal, first.at.as_millis(), window.len()));
                }
            }
            let answer = if drivers.is_empty() {
                format!("none: {}", spec.fallback)
            } else {
                drivers
                    .iter()
                    .map(|(signal, at, count)| format!("{signal} (first at +{at} ms, {count}x)"))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            Row {
                event: spec.event,
                meaning: spec.meaning,
                drivers,
                answer,
            }
        })
        .collect()
}

/// The median gap between consecutive `PositionUpdated` events, in ms.
pub fn position_update_interval_ms(events: &[TimedEvent]) -> Option<u128> {
    let times: Vec<u128> = events
        .iter()
        .filter(|timed| matches!(timed.event, MseEvent::Play(PlayEvent::PositionUpdated(_))))
        .map(|timed| timed.at.as_millis())
        .collect();
    let mut gaps: Vec<u128> = times.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    gaps.get(gaps.len() / 2).copied()
}

fn kind_of(event: &MseEvent) -> &'static str {
    match event {
        MseEvent::SourceOpen => "SourceOpen",
        MseEvent::SourceEnded => "SourceEnded",
        MseEvent::SourceClose => "SourceClose",
        MseEvent::SourceBufferAdded(_) => "SourceBufferAdded",
        MseEvent::SourceBufferRemoved(_) => "SourceBufferRemoved",
        MseEvent::UpdateStart(_) => "UpdateStart",
        MseEvent::Update(_) => "Update",
        MseEvent::UpdateEnd(_) => "UpdateEnd",
        MseEvent::Abort(_) => "Abort",
        MseEvent::Error(_) => "Error",
        MseEvent::ElementReadyState(_) => "ElementReadyState",
        MseEvent::ElementDuration(_) => "ElementDuration",
        MseEvent::ElementPosition(_) => "ElementPosition",
        MseEvent::Play(PlayEvent::StateChanged(_)) => "Play.StateChanged",
        MseEvent::Play(PlayEvent::Buffering(_)) => "Play.Buffering",
        MseEvent::Play(PlayEvent::EndOfStream) => "Play.EndOfStream",
        MseEvent::Play(PlayEvent::SeekDone(_)) => "Play.SeekDone",
        MseEvent::Play(PlayEvent::DurationChanged(_)) => "Play.DurationChanged",
        MseEvent::Play(PlayEvent::PositionUpdated(_)) => "Play.PositionUpdated",
        MseEvent::Play(PlayEvent::MediaInfoUpdated) => "Play.MediaInfoUpdated",
        MseEvent::Play(PlayEvent::Error(_)) => "Play.Error",
        MseEvent::Play(PlayEvent::Warning(_)) => "Play.Warning",
    }
}

/// Which threads delivered each kind of event.
pub fn threads_by_kind(events: &[TimedEvent]) -> BTreeMap<&'static str, Vec<String>> {
    let mut by_kind: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for timed in events {
        by_kind.entry(kind_of(&timed.event)).or_default();
    }
    for (kind, threads) in &mut by_kind {
        let wanted = *kind;
        *threads = threads_of(events, |e| kind_of(e) == wanted)
            .into_iter()
            .collect();
    }
    by_kind
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    main_run(ctx, scn)?;
    let outcome = position_rate(ctx, scn);
    keep_going(scn, "position-rate", outcome)?;
    Ok(())
}

/// The `position` property of `msesrc` (what a `timeupdate` could be driven by): how often it changes while
/// playing. It is only logged after `set_log_position(true)` before attach, which `Rig::new` has already done
/// by the time it returns, so this run detaches and attaches again before anything else happens. A separate
/// run so that the detach/attach cannot disturb the table above.
fn position_rate(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let muxed = fixture("h264-aac")?;
    let rig = new_rig(scn, timed_config(ctx))?;
    let Some(msesrc) = rig.msesrc() else {
        scn.note("position-rate: the rig has no msesrc element to re-attach to; skipped");
        shutdown(scn, "c14-position", rig);
        return Ok(());
    };
    rig.backend.detach();
    rig.backend.set_log_position(true);
    rig.backend.attach(&msesrc)?;
    rig.log.clear();
    let feeds = one_feed(&rig, &muxed)?;
    append_inits(&rig, &feeds)?;
    append_media(&rig, &feeds, 0..2)?;
    rig.play()?;
    std::thread::sleep(Duration::from_millis(2_500));
    let events = rig.log.snapshot();
    let times: Vec<u128> = events
        .iter()
        .filter(|t| matches!(t.event, MseEvent::ElementPosition(_)))
        .map(|t| t.at.as_millis())
        .collect();
    let mut gaps: Vec<u128> = times.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    let median = gaps.get(gaps.len() / 2).copied();
    let threads = threads_by_kind(&events)
        .get("ElementPosition")
        .cloned()
        .unwrap_or_default();
    scn.metric("position_notify.count_in_2_5s", times.len())
        .metric("position_notify.median_gap_ms", median.map(|m| m as u64))
        .metric("position_notify.threads", threads.clone());
    scn.note(format!(
        "msesrc notify::position while playing for 2.5 s: {} notifications, median gap {} ms, delivered on {:?}; GstPlay position-updated is the 500 ms timer the table shows",
        times.len(),
        median.map_or("n/a".to_owned(), |m| m.to_string()),
        threads
    ));
    shutdown(scn, "c14-position", rig);
    Ok(())
}

fn main_run(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let muxed = fixture("h264-aac")?;
    let rig = new_rig(scn, timed_config(ctx))?;
    let feeds = one_feed(&rig, &muxed)?;
    let mut marks = Marks::default();
    let len = |rig: &mse_rig::rig::Rig| rig.log.snapshot().len();

    marks.init_start = len(&rig);
    append_inits(&rig, &feeds)?;
    rig.wait_until(Duration::from_millis(500), |r| {
        r.element_duration().is_some()
    });
    marks.after_init = len(&rig);
    marks.media_start = len(&rig);
    append_media(&rig, &feeds, 0..2)?;

    marks.play = len(&rig);
    rig.play()?;
    let reached_end_of_buffered = rig.wait_until(Duration::from_secs(10), |r| {
        r.position().is_some_and(|p| p >= t_ms(3_800))
    });
    marks.stall = len(&rig);
    std::thread::sleep(Duration::from_millis(1_200));
    let stalled_at = rig.position();

    marks.resume = len(&rig);
    append_media(&rig, &feeds, 2..3)?;
    let resumed = rig.wait_until(Duration::from_secs(5), |r| {
        r.position().is_some_and(|p| p >= t_ms(4_400))
    });
    append_media(&rig, &feeds, 3..4)?;
    marks.seek = len(&rig);
    rig.seek(t_ms(6_500))?;
    let seek_done = rig.log.wait_for(Duration::from_secs(4), |events| {
        events
            .iter()
            .skip(marks.seek)
            .any(|t| matches!(t.event, MseEvent::Play(PlayEvent::SeekDone(_))))
    });
    std::thread::sleep(Duration::from_millis(600));
    marks.eos = len(&rig);
    rig.backend.end_of_stream(EosError::None)?;
    let eos = wait_eos(&rig, Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(300));
    marks.end = len(&rig);

    let events = rig.log.snapshot();
    scn.check(
        "plays-through-every-phase",
        reached_end_of_buffered && resumed && eos.reached,
        format!(
            "the run did not go through every phase: reached the end of the buffered data {reached_end_of_buffered}, resumed after the append {resumed}, end of stream {}; position at the stall {}; {}",
            eos.reached,
            fmt_opt(stalled_at),
            diagnostics(&rig)
        ),
    );
    scn.metric(
        "phases.stall_position_ms",
        stalled_at.map(crate::support::ms_of),
    )
    .metric("phases.seek_done", seek_done);

    let rows = build_mapping(&events, &marks);
    scn.metric(
        "event_mapping",
        rows.iter()
            .map(|row| {
                json!({
                    "html_event": row.event,
                    "meaning": row.meaning,
                    "drivers": row.drivers.iter()
                        .map(|(signal, at, count)| json!({"signal": signal, "first_ms": at, "count": count}))
                        .collect::<Vec<_>>(),
                    "answer": row.answer,
                })
            })
            .collect::<Vec<_>>(),
    );
    for row in &rows {
        scn.note(format!("{} <- {}", row.event, row.answer));
    }
    scn.check(
        "table-complete",
        rows.len() == 12 && rows.iter().all(|row| !row.answer.is_empty()),
        "a row of the table has no answer",
    );

    // Ready-state transitions in each phase, the update events, durations, threads, timeline.
    let phase_states = |from: usize, to: usize| {
        fmt_ready_states(&ready_state_events(
            &events[from.min(events.len())..to.min(events.len())],
            0,
        ))
    };
    scn.metric(
        "ready_states.init",
        phase_states(marks.init_start, marks.after_init),
    )
    .metric(
        "ready_states.first_media",
        phase_states(marks.media_start, marks.play),
    )
    .metric(
        "ready_states.play_to_stall",
        phase_states(marks.play, marks.stall),
    )
    .metric(
        "ready_states.stall",
        phase_states(marks.stall, marks.resume),
    )
    .metric(
        "ready_states.recovery_and_seek",
        phase_states(marks.resume, marks.eos),
    )
    .metric("ready_states.end", phase_states(marks.eos, marks.end));
    scn.metric(
        "position_update_interval_ms",
        position_update_interval_ms(&events).map(|ms| ms as u64),
    );
    scn.metric(
        "threads",
        threads_by_kind(&events)
            .into_iter()
            .map(|(kind, threads)| (kind.to_owned(), json!(threads)))
            .collect::<serde_json::Map<_, _>>(),
    );
    scn.metric(
        "timeline",
        events
            .iter()
            .filter(|t| !matches!(t.event, MseEvent::Play(PlayEvent::PositionUpdated(_))))
            .take(120)
            .map(|t| format!("+{} ms [{}] {:?}", t.at.as_millis(), t.thread, t.event))
            .collect::<Vec<_>>(),
    );
    scn.metric(
        "marks",
        json!({
            "init_start": marks.init_start, "after_init": marks.after_init, "play": marks.play,
            "stall": marks.stall, "resume": marks.resume, "seek": marks.seek, "eos": marks.eos, "end": marks.end,
        }),
    );
    shutdown(scn, "c14", rig);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(ms: u64, event: MseEvent) -> TimedEvent {
        TimedEvent {
            at: Duration::from_millis(ms),
            thread: format!("t{}", ms % 2),
            event,
        }
    }

    fn sample_log() -> (Vec<TimedEvent>, Marks) {
        let events = vec![
            at(1, MseEvent::SourceOpen),
            at(
                10,
                MseEvent::ElementReadyState(ElementReadyState::HaveMetadata),
            ),
            at(11, MseEvent::ElementDuration(Some(t_ms(8_000)))),
            at(
                20,
                MseEvent::ElementReadyState(ElementReadyState::HaveCurrentData),
            ),
            at(
                30,
                MseEvent::Play(PlayEvent::StateChanged("Playing".into())),
            ),
            at(
                40,
                MseEvent::ElementReadyState(ElementReadyState::HaveEnoughData),
            ),
            at(
                500,
                MseEvent::Play(PlayEvent::PositionUpdated(Some(t_ms(500)))),
            ),
            at(
                1_000,
                MseEvent::Play(PlayEvent::PositionUpdated(Some(t_ms(1_000)))),
            ),
            at(
                1_500,
                MseEvent::Play(PlayEvent::PositionUpdated(Some(t_ms(1_500)))),
            ),
            at(
                4_000,
                MseEvent::ElementReadyState(ElementReadyState::HaveCurrentData),
            ),
            at(
                5_000,
                MseEvent::ElementReadyState(ElementReadyState::HaveEnoughData),
            ),
            at(
                6_000,
                MseEvent::Play(PlayEvent::SeekDone(Some(t_ms(6_500)))),
            ),
            at(7_000, MseEvent::Play(PlayEvent::EndOfStream)),
        ];
        let marks = Marks {
            init_start: 0,
            after_init: 3,
            media_start: 3,
            play: 4,
            stall: 9,
            resume: 10,
            seek: 11,
            eos: 12,
            end: 13,
        };
        (events, marks)
    }

    #[test]
    fn the_table_has_twelve_rows_and_names_the_signals_that_appeared() {
        let (events, marks) = sample_log();
        let rows = build_mapping(&events, &marks);
        assert_eq!(rows.len(), 12);
        let by_name = |name: &str| rows.iter().find(|r| r.event == name).unwrap();
        assert!(
            by_name("loadedmetadata")
                .answer
                .contains("HaveMetadata (first at +10 ms")
        );
        assert!(
            by_name("canplaythrough")
                .answer
                .contains("HaveEnoughData (first at +40 ms, 2x)")
        );
        assert!(by_name("playing").answer.contains("Playing"));
        assert!(
            by_name("seeked")
                .answer
                .contains("seek-done (Bare: bus AsyncDone after a seek) (first at +6000 ms")
        );
        assert!(by_name("ended").answer.contains("end-of-stream"));
        assert!(by_name("timeupdate").answer.contains("3x"));
        assert!(
            by_name("durationchange")
                .answer
                .contains("msesrc notify::duration")
        );
        // The stall window is [9, 10): the fall to HaveCurrentData at 4000 ms is the waiting driver.
        assert!(by_name("waiting").answer.contains("falls"));
        assert!(by_name("seeking").answer.starts_with("none:"));
        assert!(by_name("stalled").answer.starts_with("none:"));
        assert!(rows.iter().all(|r| !r.answer.is_empty()));
    }

    #[test]
    fn a_missing_signal_is_answered_with_none_and_the_dom_fallback() {
        let rows = build_mapping(&[], &Marks::default());
        assert_eq!(rows.len(), 12);
        assert!(rows.iter().all(|r| r.answer.starts_with("none: ")));
        assert!(rows.iter().all(|r| r.drivers.is_empty()));
    }

    #[test]
    fn windows_keep_a_signal_in_its_phase() {
        let (events, mut marks) = sample_log();
        // Move the stall window so that it holds no ready-state fall: waiting has no driver.
        marks.stall = 5;
        marks.resume = 8;
        let rows = build_mapping(&events, &marks);
        let waiting = rows.iter().find(|r| r.event == "waiting").unwrap();
        assert!(waiting.answer.starts_with("none:"), "{}", waiting.answer);
    }

    #[test]
    fn the_position_update_interval_is_the_median_gap() {
        let (events, _) = sample_log();
        assert_eq!(position_update_interval_ms(&events), Some(500));
        assert_eq!(position_update_interval_ms(&[]), None);
    }

    #[test]
    fn threads_are_listed_per_event_kind() {
        let (events, _) = sample_log();
        let threads = threads_by_kind(&events);
        assert_eq!(threads["SourceOpen"], vec!["t1".to_owned()]);
        assert_eq!(threads["Play.PositionUpdated"], vec!["t0".to_owned()]);
        assert_eq!(threads["ElementReadyState"], vec!["t0".to_owned()]);
        assert!(!threads.contains_key("Error"));
    }
}
