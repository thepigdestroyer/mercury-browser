// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T01: threads and the main loop. Which thread emits each GstMSE and GstPlay callback, whether the
//! callback arrives at all with no GLib main loop (the default: Servo's backend runs none) or with
//! one (`--glib-main-loop`), and whether `append` blocks the calling thread.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use mse_rig::backend::EosError;
use mse_rig::events::{MseEvent, PlayEvent, current_thread_name};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::Rig;
use mse_segments::{SegmentedStream, tile};
use serde_json::json;

use crate::common::{
    APPEND_TIMEOUT, FIXTURE_FRAMES, Findings, Verdict, audio_enabled, event_kind, fast_config,
    fixture, new_rig, thread_table, thread_table_json, update_order_problems, video_frames,
};
use crate::stats::ms;

pub fn run(ctx: &Ctx) -> ScenarioResult {
    crate::common::run_isolated(
        "T01",
        "Threads and the main loop",
        ctx,
        "MSE_STRESS_T01_NESTED",
        std::time::Duration::from_secs(90),
        body,
    )
}

/// The callbacks an HTMLMediaElement and a MediaSource need from the backend. A missing one is a `Fail`.
const NEEDED: &[&str] = &[
    "SourceOpen",
    "SourceBufferAdded",
    "SourceBufferRemoved",
    "UpdateStart",
    "Update",
    "UpdateEnd",
    "Abort",
    "Error",
    "SourceEnded",
    "SourceClose",
    "ElementReadyState",
    "ElementDuration",
    "Play.SeekDone",
    "Play.EndOfStream",
];

/// Reported but not required: the DOM can poll or derive them.
const INFORMATIVE: &[&str] = &[
    "Play.StateChanged",
    "Play.DurationChanged",
    "Play.PositionUpdated",
    "Play.MediaInfoUpdated",
    "Play.Buffering",
];

fn body(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let video = fixture("h264-low")?;
    let audio = fixture("aac")?;
    let caller = current_thread_name();
    result.metric("calling_thread", caller.clone());
    let mut findings = Findings::new();

    // Part A: a normal play-through with a seek and the end of the stream.
    let rig = new_rig(fast_config(ctx))?;
    let vid = rig
        .backend
        .add_source_buffer(&video.mime)
        .map_err(|e| format!("add_source_buffer(video): {e:?}"))?;
    let aud = if audio_enabled() {
        Some(
            rig.backend
                .add_source_buffer(&audio.mime)
                .map_err(|e| format!("add_source_buffer(audio): {e:?}"))?,
        )
    } else {
        None
    };

    // Per append: how long the call took and how long until UpdateEnd.
    let mut call_ms = Vec::new();
    let mut end_ms = Vec::new();
    let mut timed_append = |rig: &Rig, id, bytes: &[u8]| -> Result<(), String> {
        let from = rig.log.len();
        let started = Instant::now();
        rig.backend
            .append(id, bytes)
            .map_err(|e| format!("append: {e:?}"))?;
        let call = started.elapsed();
        let finished = rig.log.wait_for(APPEND_TIMEOUT, |events| {
            events
                .get(from..)
                .is_some_and(|tail| tail.iter().any(|t| t.event == MseEvent::UpdateEnd(id)))
        });
        if !finished {
            return Err(format!(
                "no UpdateEnd within {} s after a {}-byte append",
                APPEND_TIMEOUT.as_secs(),
                bytes.len()
            ));
        }
        call_ms.push(ms(call));
        end_ms.push(ms(started.elapsed()));
        Ok(())
    };

    timed_append(&rig, vid, &video.init.bytes)?;
    if let Some(aud) = aud {
        timed_append(&rig, aud, &audio.init.bytes)?;
    }
    rig.play().map_err(|e| format!("play: {e}"))?;
    for index in 0..video.media.len().max(audio.media.len()) {
        if let Some(segment) = video.media.get(index) {
            timed_append(&rig, vid, &segment.bytes)?;
        }
        if let (Some(aud), Some(segment)) = (aud, audio.media.get(index)) {
            timed_append(&rig, aud, &segment.bytes)?;
        }
    }
    let wanted = FIXTURE_FRAMES - 2;
    rig.wait_until(Duration::from_secs(20), |rig| video_frames(rig) >= wanted);
    // A seek back into the buffered range, then the end of the stream.
    rig.seek(mse_rig::ClockTime::from_seconds(5))
        .map_err(|e| format!("seek: {e}"))?;
    rig.log.wait_for(Duration::from_secs(5), |events| {
        events
            .iter()
            .any(|t| matches!(t.event, MseEvent::Play(PlayEvent::SeekDone(_))))
    });
    rig.backend
        .end_of_stream(EosError::None)
        .map_err(|e| format!("end_of_stream: {e:?}"))?;
    rig.log.wait_for(Duration::from_secs(5), |events| {
        events.iter().any(|t| t.event == MseEvent::SourceEnded)
    });
    let eos = rig.wait_for_eos(Duration::from_secs(15));
    result.metric("part_a_eos_arrived", eos);
    std::thread::sleep(Duration::from_millis(300));
    let events_a = rig.log.snapshot();
    let order_problems: Vec<String> = std::iter::once(vid)
        .chain(aud)
        .flat_map(|id| update_order_problems(&events_a, id))
        .collect();
    result.metric("append_call_ms", crate::stats::summary_json(&call_ms));
    result.metric(
        "append_to_updateend_ms",
        crate::stats::summary_json(&end_ms),
    );
    let blocking = call_ms
        .iter()
        .zip(&end_ms)
        .filter(|(call, end)| **end > 0.0 && **call >= 0.8 * **end)
        .count();
    result.metric("small_appends_where_the_call_covers_the_update", blocking);
    result.note(format!(
        "append() of the fixture segments: {} of {} calls returned only when the update was (nearly) over, so the calling thread {}",
        blocking,
        call_ms.len(),
        if blocking * 2 > call_ms.len() { "is blocked for the whole update" } else { "is not blocked: the update runs on another thread" }
    ));

    // Part B: the callbacks a normal play-through does not make (abort, error, removal, close).
    let events_b = abnormal_callbacks(ctx, &video, result)?;
    let mut all = events_a.clone();
    all.extend(events_b.iter().cloned());
    result.metric("threads_by_callback", thread_table_json(&all));

    let mut arrived: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (kind, threads) in thread_table(&all) {
        arrived.insert(kind, threads.values().sum());
    }
    let mut missing = Vec::new();
    for kind in NEEDED {
        if !arrived.contains_key(kind) {
            missing.push((*kind).to_owned());
        }
    }
    let informative_missing: Vec<&str> = INFORMATIVE
        .iter()
        .copied()
        .filter(|kind| !arrived.contains_key(kind))
        .collect();
    result.metric("callbacks_missing", json!(missing));
    result.metric("informative_callbacks_missing", json!(informative_missing));
    result.metric("callbacks_seen", json!(arrived));

    // Which threads. The update callbacks on the calling thread would mean synchronous delivery.
    let mut synchronous = Vec::new();
    for (kind, threads) in thread_table(&all) {
        let names: Vec<&String> = threads.keys().collect();
        result.note(format!(
            "{kind}: {} on {:?}",
            threads.values().sum::<usize>(),
            names
        ));
        if threads.contains_key(&caller) {
            synchronous.push(kind);
        }
    }
    result.metric("callbacks_on_the_calling_thread", json!(synchronous));
    if !synchronous.is_empty() {
        result.note(format!(
            "these callbacks ran on the calling thread ({caller}), that is inside the call that caused them: {synchronous:?}. The DOM must not assume it can re-enter from them"
        ));
    }
    let own_threads: Vec<String> = thread_table(&all)
        .values()
        .flat_map(|threads| threads.keys().cloned())
        .filter(|name| *name != caller)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    result.metric("callback_threads_other_than_the_caller", json!(own_threads));

    if !order_problems.is_empty() {
        findings.fail(format!(
            "update event order: {} problem(s), for example {:?}",
            order_problems.len(),
            order_problems.iter().take(3).collect::<Vec<_>>()
        ));
    }
    if !missing.is_empty() {
        let mode = if ctx.glib_main_loop {
            "with a GLib main loop"
        } else {
            "with no GLib main loop"
        };
        findings.fail(format!(
            "callbacks that never arrived {mode}: {missing:?}. Consequence: {}",
            if ctx.glib_main_loop {
                "they do not arrive even with a loop, so the backend cannot report them at all"
            } else {
                "Servo's media backend runs no GLib main loop, so it must run one (a thread running glib::MainLoop on the default context) or these events cannot reach the DOM; compare with the --glib-main-loop run"
            }
        ));
    }
    result.note(format!(
        "arrival summary {mode}: needed callbacks seen {}/{}",
        NEEDED.len() - missing.len(),
        NEEDED.len(),
        mode = if ctx.glib_main_loop {
            "with a main loop"
        } else {
            "with no main loop"
        }
    ));
    result.metric(
        "part_a_pipeline_state_at_end",
        format!("{:?}", rig.pipeline_state()),
    );
    let _ = rig.shutdown();
    Ok(findings.verdict())
}

/// Triggers abort, error, source buffer removal and close on a second rig; returns its events.
fn abnormal_callbacks(
    ctx: &Ctx,
    video: &SegmentedStream,
    result: &mut ScenarioResult,
) -> Result<Vec<mse_rig::events::TimedEvent>, String> {
    let rig = new_rig(fast_config(ctx))?;
    let vid = rig
        .backend
        .add_source_buffer(&video.mime)
        .map_err(|e| format!("add_source_buffer: {e:?}"))?;
    rig.append_and_wait(vid, &video.init.bytes, APPEND_TIMEOUT)
        .map_err(|e| format!("init append: {e}"))?;

    // Abort: start a big append and abort it while the update is running.
    let mut aborted = false;
    let mut abort_note = String::new();
    for copies in [40usize, 100, 200] {
        let tiled = tile(video, copies).map_err(|e| format!("tile: {e}"))?;
        let bytes: Vec<u8> = tiled
            .media
            .iter()
            .flat_map(|s| s.bytes.iter().copied())
            .collect();
        let from = rig.log.len();
        rig.backend
            .append(vid, &bytes)
            .map_err(|e| format!("big append: {e:?}"))?;
        let updating = rig.backend.updating(vid);
        let abort_result = rig.backend.abort(vid);
        let arrived = rig.log.wait_for(Duration::from_secs(5), |events| {
            events
                .get(from..)
                .is_some_and(|tail| tail.iter().any(|t| t.event == MseEvent::Abort(vid)))
        });
        abort_note = format!(
            "abort after a {}-byte append: updating was {updating}, abort() gave {:?}, Abort event {}",
            bytes.len(),
            abort_result,
            if arrived { "arrived" } else { "did not arrive" }
        );
        if arrived {
            aborted = true;
            break;
        }
        rig.log.wait_for(Duration::from_secs(10), |events| {
            events.get(from..).is_some_and(|tail| {
                tail.iter()
                    .any(|t| matches!(t.event, MseEvent::UpdateEnd(id) if id == vid))
            })
        });
        if !updating {
            result.note(format!(
                "{abort_note}: the update was already over when append() returned, so the abort found nothing to abort; trying a bigger append"
            ));
        }
    }
    result.note(abort_note);
    result.metric("abort_event_arrived", aborted);

    // Error: bytes that are not a media segment.
    let garbage = vec![0x5Au8; 4096];
    let from = rig.log.len();
    let append = rig.backend.append(vid, &garbage);
    let error_arrived = rig.log.wait_for(Duration::from_secs(5), |events| {
        events
            .get(from..)
            .is_some_and(|tail| tail.iter().any(|t| t.event == MseEvent::Error(vid)))
    });
    result.note(format!(
        "garbage append: append() gave {append:?}, Error event {}",
        if error_arrived {
            "arrived"
        } else {
            "did not arrive"
        }
    ));
    result.metric("error_event_arrived", error_arrived);

    // Removal of a source buffer, then close by detaching.
    let removable = rig.backend.add_source_buffer(&video.mime);
    match removable {
        Ok(extra) => {
            if let Err(error) = rig.backend.remove_source_buffer(extra) {
                result.note(format!("remove_source_buffer({extra}) gave {error:?}"));
            }
        }
        Err(error) => {
            result.note(format!("a second add_source_buffer for the removal test gave {error:?} (state after the garbage append)"));
            // Try on the buffer that exists.
            if let Err(error) = rig.backend.remove_source_buffer(vid) {
                result.note(format!("remove_source_buffer({vid}) gave {error:?}"));
            }
        }
    }
    rig.log.wait_for(Duration::from_secs(2), |events| {
        events
            .iter()
            .any(|t| matches!(t.event, MseEvent::SourceBufferRemoved(_)))
    });
    rig.backend.detach();
    rig.log.wait_for(Duration::from_secs(3), |events| {
        events.iter().any(|t| t.event == MseEvent::SourceClose)
    });
    let events = rig.log.snapshot();
    let kinds: Vec<&str> = events.iter().map(|t| event_kind(&t.event)).collect();
    result.metric("part_b_event_kinds", json!(kinds));
    let _ = rig.shutdown();
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_needed_and_informative_lists_do_not_overlap() {
        for kind in NEEDED {
            assert!(!INFORMATIVE.contains(kind), "{kind} is in both lists");
        }
    }

    #[test]
    fn every_listed_kind_is_a_real_event_kind() {
        let known = [
            "SourceOpen",
            "SourceEnded",
            "SourceClose",
            "SourceBufferAdded",
            "SourceBufferRemoved",
            "UpdateStart",
            "Update",
            "UpdateEnd",
            "Abort",
            "Error",
            "ElementReadyState",
            "ElementDuration",
            "Play.StateChanged",
            "Play.Buffering",
            "Play.EndOfStream",
            "Play.SeekDone",
            "Play.DurationChanged",
            "Play.PositionUpdated",
            "Play.MediaInfoUpdated",
            "Play.Error",
            "Play.Warning",
        ];
        for kind in NEEDED.iter().chain(INFORMATIVE) {
            assert!(known.contains(kind), "{kind} is not an event kind");
        }
    }
}
