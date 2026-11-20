// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T02: re-entrancy. From inside each callback (`UpdateEnd`, `Update`, `Error`, `SourceOpen`, the
//! element's ready-state notification, a pad probe on `msesrc`'s streaming thread) the scenario calls
//! each of `append`, `buffered`, `remove`, `set_duration`, `add_source_buffer`, `end_of_stream`, `abort`
//! and `detach`, 6 x 8 = 48 cells, each with a 2 s watchdog. The output is the exact set that
//! deadlocks or errors: what the DOM must defer to the script thread.
//!
//! Every cell runs in a process of its own (a nested run of this executable, see `sub`), because a
//! deadlocked cell can leave a lock held inside GstMSE for good.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use mse_rig::ClockTime;
use mse_rig::backend::EosError;
use mse_rig::events::{MseEvent, SourceBufferId, TimedEvent, current_thread_name};
use mse_rig::harness::Ctx;
use mse_rig::report::{ScenarioResult, Status};
use mse_rig::rig::Rig;
use serde_json::json;

use crate::common::ManagedRig;
use crate::common::{
    APPEND_TIMEOUT, Findings, Verdict, fast_config, fixture, new_rig, run_scenario,
};
use crate::stats::ms;
use crate::sub::{SubOutcome, SubSpec, part, run_all};
use crate::watchdog::{Done, WatchError, call_with_timeout};

const CELL_VARIABLE: &str = "MSE_STRESS_T02_CELL";

/// How long one call may take from inside a callback before it counts as a deadlock.
const CALL_WATCHDOG: Duration = Duration::from_secs(2);

/// How long to wait for a callback to fire at all.
const FIRE_TIMEOUT: Duration = Duration::from_secs(6);

pub const HOOKS: &[&str] = &[
    "UpdateEnd",
    "Update",
    "Error",
    "SourceOpen",
    "ReadyState",
    "StreamingProbe",
];
pub const CALLS: &[&str] = &[
    "append",
    "buffered",
    "remove",
    "set_duration",
    "add_source_buffer",
    "end_of_stream",
    "abort",
    "detach",
];

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(
        "T02",
        "Re-entrancy from inside callbacks",
        ctx,
        |ctx, result| match part(CELL_VARIABLE) {
            Some(cell) => run_cell(ctx, result, &cell),
            None => run_matrix(ctx, result),
        },
    )
}

/// `UpdateEnd:append` -> (`UpdateEnd`, `append`).
pub fn parse_cell(text: &str) -> Option<(&'static str, &'static str)> {
    let (hook, call) = text.split_once(':')?;
    let hook = HOOKS.iter().find(|h| **h == hook)?;
    let call = CALLS.iter().find(|c| **c == call)?;
    Some((hook, call))
}

/// The cell names in matrix order.
pub fn all_cells() -> Vec<String> {
    HOOKS
        .iter()
        .flat_map(|hook| CALLS.iter().map(move |call| format!("{hook}:{call}")))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The matrix driver.

fn run_matrix(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let cells = all_cells();
    let specs: Vec<SubSpec> = cells
        .iter()
        .map(|cell| SubSpec {
            id: "T02".to_owned(),
            tag: cell.replace(':', "-"),
            env: vec![(CELL_VARIABLE.to_owned(), cell.clone())],
            iterations: None,
            seed: None,
            deadline: Duration::from_secs(45),
        })
        .collect();
    let outcomes = run_all(ctx, &specs, 4, Duration::from_secs(235));

    let mut matrix: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut findings = Findings::new();
    let mut deadlocks = Vec::new();
    let mut errors = Vec::new();
    let mut crashes = Vec::new();
    let mut never_fired = Vec::new();
    let mut not_run: Vec<String> = Vec::new();
    let mut threads: BTreeMap<String, String> = BTreeMap::new();
    for (cell, outcome) in cells.iter().zip(outcomes) {
        let Some((hook, call)) = parse_cell(cell) else {
            continue;
        };
        let label = match outcome {
            SubOutcome::Report {
                result: cell_result,
                exit,
            } => {
                if cell_result.status == Status::Error {
                    return Err(format!(
                        "cell {cell} reported a harness error (exit {exit}): {}",
                        cell_result.notes.join(" | ")
                    ));
                }
                if let Some(thread) = cell_result
                    .metrics
                    .get("callback_thread")
                    .and_then(|v| v.as_str())
                {
                    threads.insert(hook.to_owned(), thread.to_owned());
                }
                cell_result
                    .metrics
                    .get("outcome")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned()
            }
            SubOutcome::NotRun => "not-run".to_owned(),
            other => format!("crash: {}", other.describe()),
        };
        let pair = format!("{call} from {hook}");
        if label == "deadlock" {
            deadlocks.push(pair);
        } else if label.starts_with("crash") {
            crashes.push(format!("{pair}: {label}"));
        } else if label.starts_with("err:") {
            errors.push(format!("{pair}: {}", label.trim_start_matches("err:")));
        } else if label == "never-fired" {
            never_fired.push(hook.to_owned());
        } else if label == "not-run" {
            not_run.push(pair.clone());
        }
        matrix
            .entry(hook.to_owned())
            .or_default()
            .insert(call.to_owned(), label);
    }
    never_fired.sort();
    never_fired.dedup();

    for hook in HOOKS {
        let row = matrix.get(*hook).cloned().unwrap_or_default();
        let cells: Vec<String> = CALLS
            .iter()
            .map(|call| {
                format!(
                    "{call}={}",
                    short(row.get(*call).map_or("-", String::as_str))
                )
            })
            .collect();
        result.note(format!(
            "from {hook} (thread {}): {}",
            threads.get(*hook).map_or("?", String::as_str),
            cells.join(" ")
        ));
    }
    result.metric("matrix", json!(matrix));
    result.metric("callback_threads", json!(threads));
    result.metric("deadlocks", json!(deadlocks));
    result.metric("errors", json!(errors));
    result.metric("crashes", json!(crashes));
    result.metric("callbacks_that_never_fired", json!(never_fired));
    result.metric("cells_not_run", json!(not_run));
    if !not_run.is_empty() {
        result.note(format!(
            "INCOMPLETE: {} of {} cells were not run because the time budget ran out: {not_run:?}",
            not_run.len(),
            cells.len()
        ));
    }
    result.metric("cells", cells.len());
    result.metric(
        "cells_ok",
        matrix
            .values()
            .flat_map(|row| row.values())
            .filter(|l| *l == "ok")
            .count(),
    );
    if !errors.is_empty() {
        result.note(format!(
            "calls refused with an MSE error (documented behaviour, not a hang): {}",
            errors.join("; ")
        ));
    }
    if !never_fired.is_empty() {
        result.note(format!(
            "callbacks that never fired, so their cells were skipped: {never_fired:?}"
        ));
    }
    for pair in &deadlocks {
        findings.fail(format!(
            "deadlock: `{pair}`; the DOM must not make this call from the callback (post it to the media element task source and make it from the script thread)"
        ));
    }
    for crash in &crashes {
        findings.fail(format!("process died or hung: {crash}"));
    }
    if findings.is_clean() {
        result.note("no cell deadlocked or crashed: every call returned within 2 s from every callback that fired");
    }
    Ok(findings.verdict())
}

fn short(label: &str) -> String {
    match label {
        "ok" => "ok".to_owned(),
        "deadlock" => "DEADLOCK".to_owned(),
        "never-fired" => "n/a".to_owned(),
        other => {
            if let Some(rest) = other.strip_prefix("err:") {
                let kind: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
                format!("err({kind})")
            } else {
                other.chars().take(24).collect()
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// One cell.

/// What the callback needs to make its call (owned, so it can live inside the callback).
struct Bits {
    video_mime: String,
    audio_mime: String,
    init_bytes: Vec<u8>,
    next_bytes: Vec<u8>,
    /// The source buffer to use; `None` in the `SourceOpen` cell, where the callback makes its own.
    buffer: Mutex<Option<SourceBufferId>>,
}

struct CellState {
    fired: AtomicBool,
    done: Done,
    outcome: Mutex<Option<String>>,
    thread: Mutex<Option<String>>,
    call_ms: Mutex<Option<f64>>,
}

impl CellState {
    fn new() -> Arc<Self> {
        Arc::new(CellState {
            fired: AtomicBool::new(false),
            done: Done::new(),
            outcome: Mutex::new(None),
            thread: Mutex::new(None),
            call_ms: Mutex::new(None),
        })
    }
}

fn describe_result<T: std::fmt::Debug, E: std::fmt::Debug>(result: Result<T, E>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(error) => format!("err:{error:?}"),
    }
}

/// Makes the call under test on the backend. Runs on whatever thread the callback arrived on.
fn make_call(rig: &Rig, call: &str, bits: &Bits) -> String {
    let needs_buffer = matches!(call, "append" | "buffered" | "remove" | "abort");
    let buffer = {
        let mut slot = bits.buffer.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() && needs_buffer {
            // The SourceOpen cell: the callback adds the buffer the call needs. If that deadlocks,
            // this cell reports it as the deadlocking call and says so.
            match rig.backend.add_source_buffer(&bits.video_mime) {
                Ok(id) => *slot = Some(id),
                Err(error) => {
                    return format!("err:precondition add_source_buffer failed: {error:?}");
                }
            }
        }
        *slot
    };
    let backend = &rig.backend;
    match call {
        "append" => match buffer {
            Some(id) => {
                let data = if bits.next_bytes.is_empty() {
                    &bits.init_bytes
                } else {
                    &bits.next_bytes
                };
                describe_result(backend.append(id, data))
            }
            None => "err:no source buffer".to_owned(),
        },
        "buffered" => match buffer {
            Some(id) => describe_result(backend.buffered(id)),
            None => "err:no source buffer".to_owned(),
        },
        "remove" => match buffer {
            Some(id) => describe_result(backend.remove(
                id,
                ClockTime::ZERO,
                Some(ClockTime::from_seconds(1)),
            )),
            None => "err:no source buffer".to_owned(),
        },
        "set_duration" => describe_result(backend.set_duration(Some(ClockTime::from_seconds(10)))),
        "add_source_buffer" => describe_result(backend.add_source_buffer(&bits.audio_mime)),
        "end_of_stream" => describe_result(backend.end_of_stream(EosError::None)),
        "abort" => match buffer {
            Some(id) => describe_result(backend.abort(id)),
            None => "err:no source buffer".to_owned(),
        },
        "detach" => {
            backend.detach();
            "ok".to_owned()
        }
        other => format!("err:unknown call {other}"),
    }
}

/// The callback body: runs once, on the callback's thread.
fn fire(state: &CellState, rig: &Weak<ManagedRig>, call: &'static str, bits: &Bits) {
    if state.fired.swap(true, Ordering::SeqCst) {
        return;
    }
    *state.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(current_thread_name());
    let Some(rig) = rig.upgrade() else {
        *state.outcome.lock().unwrap_or_else(|e| e.into_inner()) =
            Some("err:rig already gone".to_owned());
        state.done.start();
        state.done.finish();
        return;
    };
    state.done.start();
    let started = std::time::Instant::now();
    let outcome = make_call(&rig, call, bits);
    *state.call_ms.lock().unwrap_or_else(|e| e.into_inner()) = Some(ms(started.elapsed()));
    *state.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
    state.done.finish();
}

/// Runs `f` against the rig on a helper thread with the 2 s watchdog.
fn watched<T: Send + 'static>(
    what: &str,
    rig: &Arc<ManagedRig>,
    f: impl FnOnce(&Rig) -> T + Send + 'static,
) -> Result<T, WatchError> {
    let rig = Arc::clone(rig);
    call_with_timeout(what, CALL_WATCHDOG, move || f(&rig))
}

fn run_cell(ctx: &Ctx, result: &mut ScenarioResult, cell: &str) -> Result<Verdict, String> {
    let (hook, call) = parse_cell(cell).ok_or_else(|| format!("unknown cell {cell:?}"))?;
    result.metric("cell", cell);
    result.metric("hook", hook);
    result.metric("call", call);
    let video = fixture("h264-low")?;
    let audio = fixture("aac")?;
    let rig = Arc::new(new_rig(fast_config(ctx))?);
    let state = CellState::new();
    let bits = Arc::new(Bits {
        video_mime: video.mime.clone(),
        audio_mime: audio.mime.clone(),
        init_bytes: video.init.bytes.clone(),
        next_bytes: video
            .media
            .first()
            .map(|s| s.bytes.clone())
            .unwrap_or_default(),
        buffer: Mutex::new(None),
    });
    let weak = Arc::downgrade(&rig);

    // The source buffer the call works on (the SourceOpen cell makes its own inside the callback).
    if hook != "SourceOpen" {
        let id = rig
            .backend
            .add_source_buffer(&video.mime)
            .map_err(|e| format!("add_source_buffer(video): {e:?}"))?;
        *bits.buffer.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
    }
    let vid = *bits.buffer.lock().unwrap_or_else(|e| e.into_inner());

    // Arm the callback.
    match hook {
        "UpdateEnd" | "Update" | "Error" | "ReadyState" | "SourceOpen" => {
            let (state, bits, weak) = (Arc::clone(&state), Arc::clone(&bits), weak.clone());
            let wanted = hook;
            rig.log.add_hook(move |timed: &TimedEvent| {
                let matches = match (&timed.event, wanted) {
                    (MseEvent::UpdateEnd(id), "UpdateEnd") => Some(*id) == vid,
                    (MseEvent::Update(id), "Update") => Some(*id) == vid,
                    (MseEvent::Error(id), "Error") => Some(*id) == vid,
                    (MseEvent::ElementReadyState(_), "ReadyState") => true,
                    (MseEvent::SourceOpen, "SourceOpen") => true,
                    _ => false,
                };
                if matches {
                    fire(&state, &weak, call, &bits);
                }
            });
        }
        "StreamingProbe" => {
            let msesrc = rig
                .msesrc()
                .ok_or_else(|| "the rig has no msesrc element".to_owned())?;
            let (state, bits, weak) = (Arc::clone(&state), Arc::clone(&bits), weak.clone());
            msesrc.connect_pad_added(move |_, pad| {
                let (state, bits, weak) = (Arc::clone(&state), Arc::clone(&bits), weak.clone());
                pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                    fire(&state, &weak, call, &bits);
                    gst::PadProbeReturn::Remove
                });
            });
        }
        other => return Err(format!("unknown hook {other}")),
    }

    // Drive the rig so that the callback fires.
    let mut trouble: Vec<String> = Vec::new();
    let mut drive =
        |what: &str, f: Box<dyn FnOnce(&Rig) -> String + Send>| match watched(what, &rig, f) {
            Ok(text) if text.is_empty() => {}
            Ok(text) => trouble.push(format!("{what}: {text}")),
            Err(error) => trouble.push(error.to_string()),
        };
    let init = video.init.bytes.clone();
    let media: Vec<Vec<u8>> = video
        .media
        .iter()
        .take(2)
        .map(|s| s.bytes.clone())
        .collect();
    match hook {
        "UpdateEnd" | "Update" | "ReadyState" | "StreamingProbe" => {
            let id = vid.ok_or("no source buffer")?;
            drive(
                "driver append(init)",
                Box::new(move |rig| describe_unit(rig.backend.append(id, &init))),
            );
            if matches!(hook, "ReadyState" | "StreamingProbe") {
                wait_idle(&rig, id);
                for (index, bytes) in media.into_iter().enumerate() {
                    let what = format!("driver append(media {index})");
                    drive(
                        &what,
                        Box::new(move |rig| describe_unit(rig.backend.append(id, &bytes))),
                    );
                    wait_idle(&rig, id);
                }
                drive("driver play", Box::new(|rig| describe_unit(rig.play())));
            }
        }
        "Error" => {
            let id = vid.ok_or("no source buffer")?;
            drive(
                "driver append(init)",
                Box::new(move |rig| describe_unit(rig.backend.append(id, &init))),
            );
            wait_idle(&rig, id);
            drive(
                "driver append(garbage)",
                Box::new(move |rig| describe_unit(rig.backend.append(id, &vec![0x5Au8; 4096]))),
            );
        }
        "SourceOpen" => {
            // The SourceOpen of Rig::new is over: close the source and open it again with the hook armed.
            drive(
                "driver detach",
                Box::new(|rig| {
                    rig.backend.detach();
                    String::new()
                }),
            );
            let msesrc = rig.msesrc().ok_or("the rig has no msesrc element")?;
            drive(
                "driver attach",
                Box::new(move |rig| describe_unit(rig.backend.attach(&msesrc))),
            );
        }
        _ => {}
    }

    // Did the callback fire, and did the call return?
    let fired = poll_until(FIRE_TIMEOUT, || state.done.has_started());
    let outcome;
    let mut blocks_other_threads = None;
    if !fired {
        outcome = "never-fired".to_owned();
        result.note(format!(
            "the {hook} callback never fired in {} s (driver trouble: {trouble:?})",
            FIRE_TIMEOUT.as_secs()
        ));
    } else {
        match state.done.wait(CALL_WATCHDOG) {
            Some(_) => {
                outcome = state
                    .outcome
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                    .unwrap_or_else(|| "err:no outcome".to_owned());
            }
            None => {
                outcome = "deadlock".to_owned();
                // Does the stuck callback also block calls from other threads?
                let probe = watched("main-thread buffered()+ready_state()", &rig, move |rig| {
                    let _ = rig.backend.ready_state();
                    vid.map(|id| rig.backend.updating(id))
                });
                blocks_other_threads = Some(probe.is_err());
                result.note(format!(
                    "DEADLOCK: `{call}` called from the {hook} callback (thread {}) did not return within {} s; a call from another thread afterwards {}",
                    state.thread.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default(),
                    CALL_WATCHDOG.as_secs(),
                    if probe.is_err() { "also hung: the lock is global to the MediaSource" } else { "still returned: only the calling thread is stuck" }
                ));
            }
        }
    }
    result.metric("outcome", outcome.clone());
    result.metric(
        "callback_thread",
        state
            .thread
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    );
    result.metric(
        "call_ms",
        state
            .call_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|v| json!(v)),
    );
    result.metric("blocks_other_threads", blocks_other_threads);
    if !trouble.is_empty() {
        result.note(format!("driver messages: {}", trouble.join("; ")));
    }
    result.note(format!("`{call}` from the {hook} callback: {outcome}"));

    // Tidy up unless something is stuck (the harness kills the process).
    if outcome != "deadlock" {
        rig.log.clear_hooks();
        if let Ok(rig) = Arc::try_unwrap(rig) {
            let _ = rig.shutdown();
        }
    }
    Ok(match outcome.as_str() {
        "deadlock" => Verdict::Fail(vec![format!("deadlock in {call} from {hook}")]),
        "never-fired" => Verdict::Skip(format!(
            "the {hook} callback never fired, so {call} could not be tried from it"
        )),
        _ => Verdict::Pass,
    })
}

fn describe_unit<T, E: std::fmt::Debug>(result: Result<T, E>) -> String {
    match result {
        Ok(_) => String::new(),
        Err(error) => format!("{error:?}"),
    }
}

/// Waits until source buffer `id` is not updating (the driver's own pacing; bounded).
fn wait_idle(rig: &Arc<ManagedRig>, id: SourceBufferId) {
    let _ = watched("driver wait for updating=false", rig, move |rig| {
        rig.wait_until(APPEND_TIMEOUT.min(Duration::from_secs(1)), |rig| {
            !rig.backend.updating(id)
        })
    });
}

fn poll_until(timeout: Duration, condition: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matrix_is_six_hooks_by_eight_calls() {
        let cells = all_cells();
        assert_eq!(cells.len(), 48);
        assert_eq!(cells[0], "UpdateEnd:append");
        assert_eq!(cells[47], "StreamingProbe:detach");
        let mut unique = cells.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 48);
    }

    #[test]
    fn every_cell_name_parses_back() {
        for cell in all_cells() {
            let (hook, call) = parse_cell(&cell).unwrap_or_else(|| panic!("{cell} does not parse"));
            assert_eq!(format!("{hook}:{call}"), cell);
        }
        assert_eq!(parse_cell("UpdateEnd"), None);
        assert_eq!(parse_cell("Nope:append"), None);
        assert_eq!(parse_cell("Update:nope"), None);
    }

    #[test]
    fn short_labels_are_compact() {
        assert_eq!(short("ok"), "ok");
        assert_eq!(short("deadlock"), "DEADLOCK");
        assert_eq!(short("never-fired"), "n/a");
        assert_eq!(short("err:InvalidState(\"x\")"), "err(InvalidState)");
    }

    #[test]
    fn describe_result_names_the_error() {
        assert_eq!(describe_result::<(), String>(Ok(())), "ok");
        assert_eq!(
            describe_result::<(), String>(Err("bad".into())),
            "err:\"bad\""
        );
        assert_eq!(describe_unit::<(), String>(Ok(())), "");
    }

    #[test]
    fn poll_until_stops_on_the_condition() {
        let flag = AtomicBool::new(false);
        assert!(!poll_until(Duration::from_millis(30), || flag.load(Ordering::SeqCst)));
        flag.store(true, Ordering::SeqCst);
        assert!(poll_until(Duration::from_millis(30), || flag.load(Ordering::SeqCst)));
    }
}
