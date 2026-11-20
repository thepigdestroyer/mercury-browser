// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T04: lifecycle and teardown. N (200 unless `--iterations` says otherwise) times: create a rig,
//! attach, add source buffers, append, play, tear down, in seven teardown orders; resident memory,
//! file descriptors and threads are recorded per order and the RSS growth per iteration is fitted.
//!
//! Each order runs in a nested run of this executable (`sub`), all seven in parallel, so that a crash
//! or hang in one order is a finding about that order and costs nothing else. Thread and descriptor
//! counts are compared with the values after a short warm-up (lazily created pool threads and
//! caches are not leaks); the values before the first rig are reported too.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mse_rig::ClockTime;
use mse_rig::harness::Ctx;
use mse_rig::report::{ScenarioResult, Status};
use mse_rig::rig::TeardownOrder;
use mse_segments::{SegmentedStream, tile};
use serde_json::json;

use crate::common::{
    APPEND_TIMEOUT, Findings, ManagedRig, Verdict, add_buffers, fast_config, fixture, new_rig,
    run_scenario, video_frames,
};
use crate::proc::{self, Sample};
use crate::stats::{ms, slope_per_index, summary_json};
use crate::sub::{SubOutcome, SubSpec, part, run_all};

const ORDER_VARIABLE: &str = "MSE_STRESS_T04_ORDER";
/// Where a nested run writes `iteration N stage S` at every step, so that a crash leaves its place behind.
const PROGRESS_VARIABLE: &str = "MSE_STRESS_T04_PROGRESS";

/// The default N of the brief.
pub const DEFAULT_ITERATIONS: usize = 200;

/// RSS growth per iteration above which the order is reported as leaking (kilobytes).
pub const LEAK_SLOPE_KB: f64 = 1024.0;

/// Threads and descriptors may differ from the warm baseline by this much at the end.
pub const COUNT_SLACK: usize = 2;

/// A stall of this long in one stage is reported as a hang.
const STALL: Duration = Duration::from_secs(30);

/// Wall-clock budget of one order (the scenario's own limit is 300 s; the orders run in parallel).
const ORDER_BUDGET: Duration = Duration::from_secs(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// Pipeline to NULL, then drop the backend.
    NullThenBackend,
    /// Drop the backend, then take the pipeline to NULL.
    BackendThenNull,
    /// A large append is running; detach, then NULL, then drop.
    DetachDuringAppend,
    /// A large append is running; the pipeline goes to NULL.
    NullDuringAppend,
    /// A seek to an unbuffered position is pending; the pipeline goes to NULL.
    NullWithSeekPending,
    /// Drop everything without setting NULL.
    DropWithoutNull,
    /// Detach, then attach the same MediaSource to the `msesrc` of a second pipeline.
    ReattachToSecondElement,
}

pub const ORDERS: [Order; 7] = [
    Order::NullThenBackend,
    Order::BackendThenNull,
    Order::DetachDuringAppend,
    Order::NullDuringAppend,
    Order::NullWithSeekPending,
    Order::DropWithoutNull,
    Order::ReattachToSecondElement,
];

impl Order {
    pub fn label(self) -> &'static str {
        match self {
            Order::NullThenBackend => "null-then-drop-backend",
            Order::BackendThenNull => "drop-backend-then-null",
            Order::DetachDuringAppend => "detach-during-append",
            Order::NullDuringAppend => "null-during-append",
            Order::NullWithSeekPending => "null-with-seek-pending",
            Order::DropWithoutNull => "drop-without-null",
            Order::ReattachToSecondElement => "reattach-to-second-element",
        }
    }

    pub fn index(self) -> usize {
        ORDERS.iter().position(|order| *order == self).unwrap_or(0)
    }

    fn needs_big_append(self) -> bool {
        matches!(self, Order::DetachDuringAppend | Order::NullDuringAppend)
    }
}

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(
        "T04",
        "Lifecycle and teardown",
        ctx,
        |ctx, result| match part(ORDER_VARIABLE) {
            Some(index) => {
                let order = index
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| ORDERS.get(i).copied())
                    .ok_or_else(|| {
                        format!("{ORDER_VARIABLE}={index:?} is not an order index 0..6")
                    })?;
                run_order(ctx, result, order)
            }
            None => run_orders(ctx, result),
        },
    )
}

pub fn iterations_for(ctx: &Ctx) -> usize {
    if ctx.iterations > 1 {
        ctx.iterations
    } else {
        DEFAULT_ITERATIONS
    }
}

/// The warm-up length for `n` iterations.
pub fn warmup_for(n: usize) -> usize {
    (n / 20).clamp(3, 10)
}

// ---------------------------------------------------------------------------------------------
// The driver: seven nested runs.

fn run_orders(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let n = iterations_for(ctx);
    result.metric("iterations_requested", n);
    let specs: Vec<SubSpec> = ORDERS
        .iter()
        .map(|order| SubSpec {
            id: "T04".to_owned(),
            tag: format!("order-{}", order.index()),
            env: vec![
                (ORDER_VARIABLE.to_owned(), order.index().to_string()),
                (
                    PROGRESS_VARIABLE.to_owned(),
                    ctx.scratch
                        .join(format!("progress-order-{}.txt", order.index()))
                        .display()
                        .to_string(),
                ),
            ],
            iterations: Some(n),
            seed: None,
            deadline: Duration::from_secs(280),
        })
        .collect();
    let since = std::time::SystemTime::now();
    let outcomes = run_all(ctx, &specs, ORDERS.len(), Duration::from_secs(285));
    let mut findings = Findings::new();
    for (order, outcome) in ORDERS.iter().zip(outcomes) {
        let label = order.label();
        let last_seen = std::fs::read_to_string(
            ctx.scratch
                .join(format!("progress-order-{}.txt", order.index())),
        )
        .unwrap_or_default();
        match outcome {
            SubOutcome::Report { result: sub, exit } => {
                if sub.status == Status::Error
                    && !sub.notes.iter().any(|n| n.to_lowercase().contains("panic"))
                {
                    findings.fail(format!(
                        "order {label}: the process died without a result ({exit}) at {last_seen:?}: {}",
                        sub.notes.join(" | ")
                    ));
                    result.note(format!(
                        "[{label}] died: exit {exit}; last progress {last_seen:?}; {}",
                        sub.notes.join(" | ")
                    ));
                    continue;
                }
                if sub.status == Status::Error {
                    return Err(format!("order {label}: {}", sub.notes.join(" | ")));
                }
                for (key, value) in &sub.metrics {
                    if matches!(key.as_str(), "shape" | "plugin_mode" | "glib_main_loop") {
                        continue;
                    }
                    result.metric(&format!("o{}_{label}_{key}", order.index()), value.clone());
                }
                for note in &sub.notes {
                    if let Some(reason) = note.strip_prefix("FAIL: ") {
                        findings.fail(format!("order {label}: {reason}"));
                    } else if !note.starts_with("ran with ") {
                        result.note(format!("[{label}] {note}"));
                    }
                }
                result.note(format!("[{label}] sub-run status {:?}", sub.status));
            }
            other => {
                findings.fail(format!(
                    "order {label}: {} (last progress {last_seen:?})",
                    other.describe()
                ));
                result.note(format!(
                    "[{label}] {} (last progress {last_seen:?})",
                    other.describe()
                ));
            }
        }
    }
    let crashes = crate::sub::crash_summaries_since(since);
    result.metric("crash_reports", json!(crashes));
    for crash in &crashes {
        result.note(format!("macOS {crash}"));
    }
    Ok(findings.verdict())
}

// ---------------------------------------------------------------------------------------------
// One order, in this process.

/// What the worker thread reports.
struct OrderRun {
    iterations_done: usize,
    rss_kb: Vec<u64>,
    iteration_ms: Vec<f64>,
    shutdown_ms: Vec<f64>,
    cold: Sample,
    warm: Sample,
    end: Sample,
    shutdown_messages: usize,
    reattach_plays: Option<usize>,
    notes: Vec<String>,
    budget_exhausted: bool,
}

struct Progress {
    iteration: AtomicUsize,
    stage: Mutex<String>,
    changed: Mutex<Instant>,
    path: Option<std::path::PathBuf>,
}

impl Progress {
    fn set(&self, iteration: usize, stage: &str) {
        self.iteration.store(iteration, Ordering::SeqCst);
        *self.stage.lock().unwrap_or_else(|e| e.into_inner()) = stage.to_owned();
        *self.changed.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        if let Some(path) = &self.path {
            let _ = std::fs::write(path, format!("iteration {iteration} stage {stage}"));
        }
    }
}

fn run_order(ctx: &Ctx, result: &mut ScenarioResult, order: Order) -> Result<Verdict, String> {
    let n = iterations_for(ctx);
    let video = fixture("h264-low")?;
    let audio = fixture("aac")?;
    let big: Vec<u8> = if order.needs_big_append() {
        tile(&video, 40)
            .map_err(|e| format!("tile: {e}"))?
            .media
            .iter()
            .flat_map(|s| s.bytes.iter().copied())
            .collect()
    } else {
        Vec::new()
    };
    result.metric("order", order.label());
    result.metric("iterations_requested", n);
    result.metric("big_append_bytes", big.len());

    let progress = Arc::new(Progress {
        iteration: AtomicUsize::new(0),
        stage: Mutex::new("start".to_owned()),
        changed: Mutex::new(Instant::now()),
        path: part(PROGRESS_VARIABLE).map(std::path::PathBuf::from),
    });
    let (sender, receiver) = mpsc::channel();
    let worker_ctx = ctx.clone();
    let worker_progress = Arc::clone(&progress);
    std::thread::Builder::new()
        .name("t04-worker".into())
        .spawn(move || {
            let outcome = order_loop(
                &worker_ctx,
                order,
                n,
                &video,
                &audio,
                &big,
                &worker_progress,
            );
            let _ = sender.send(outcome);
        })
        .map_err(|e| format!("cannot start the worker thread: {e}"))?;

    let finished = loop {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(outcome) => break Ok(outcome),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let idle = progress
                    .changed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .elapsed();
                if idle > STALL {
                    break Err(format!(
                        "hang: iteration {} stuck in stage `{}` for {} s",
                        progress.iteration.load(Ordering::SeqCst),
                        progress.stage.lock().unwrap_or_else(|e| e.into_inner()),
                        idle.as_secs()
                    ));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err("the worker thread panicked".to_owned());
            }
        }
    };
    let mut findings = Findings::new();
    let run = match finished {
        Ok(Ok(run)) => run,
        Ok(Err(why)) => {
            findings.fail(why);
            return Ok(findings.verdict());
        }
        Err(hang) => {
            findings.fail(hang);
            return Ok(findings.verdict());
        }
    };

    let warm_n = warmup_for(n).min(run.rss_kb.len().saturating_sub(2));
    let rss_after_warmup: Vec<f64> = run
        .rss_kb
        .iter()
        .skip(warm_n)
        .map(|kb| *kb as f64)
        .collect();
    let slope = slope_per_index(&rss_after_warmup);
    result.metric("iterations_completed", run.iterations_done);
    result.metric("warmup_iterations", warm_n);
    result.metric("rss_kb_cold", run.cold.rss_kb);
    result.metric("rss_kb_warm", run.warm.rss_kb);
    result.metric("rss_kb_end", run.end.rss_kb);
    result.metric("rss_slope_kb_per_iteration", crate::stats::round3(slope));
    result.metric("threads_cold", run.cold.threads);
    result.metric("threads_warm", run.warm.threads);
    result.metric("threads_end", run.end.threads);
    result.metric("fds_cold", run.cold.fds);
    result.metric("fds_warm", run.warm.fds);
    result.metric("fds_end", run.end.fds);
    result.metric("iteration_ms", summary_json(&run.iteration_ms));
    result.metric("shutdown_ms", summary_json(&run.shutdown_ms));
    result.metric("shutdown_messages", run.shutdown_messages);
    result.metric(
        "reattached_element_played_in_iterations",
        run.reattach_plays,
    );
    result.metric("budget_exhausted", run.budget_exhausted);
    for note in &run.notes {
        result.note(note.clone());
    }
    result.note(format!(
        "{}: {} iterations; RSS {} KB cold, {} KB after warm-up, {} KB at the end, slope {} KB per iteration; threads {}/{}/{}; fds {}/{}/{} (cold/warm/end)",
        order.label(),
        run.iterations_done,
        run.cold.rss_kb,
        run.warm.rss_kb,
        run.end.rss_kb,
        slope.map_or("n/a".to_owned(), |s| format!("{s:.1}")),
        run.cold.threads,
        run.warm.threads,
        run.end.threads,
        run.cold.fds,
        run.warm.fds,
        run.end.fds,
    ));
    if run.budget_exhausted {
        result.note(format!(
            "the {} s budget ended the loop after {} of {n} iterations",
            ORDER_BUDGET.as_secs(),
            run.iterations_done
        ));
    }
    if run.iterations_done < n && !run.budget_exhausted {
        findings.fail(format!(
            "only {} of {n} iterations completed",
            run.iterations_done
        ));
    }
    if let Some(slope) = slope {
        findings.check(
            slope < LEAK_SLOPE_KB,
            format!(
                "{}: RSS grows by {slope:.0} KB per iteration (limit {LEAK_SLOPE_KB:.0} KB): a leak caused by this teardown order",
                order.label()
            ),
        );
    }
    findings.check(
        run.end.threads <= run.warm.threads + COUNT_SLACK,
        format!(
            "{}: threads did not return: {} after warm-up, {} at the end (slack {COUNT_SLACK})",
            order.label(),
            run.warm.threads,
            run.end.threads
        ),
    );
    findings.check(
        run.end.fds <= run.warm.fds + COUNT_SLACK,
        format!(
            "{}: file descriptors did not return: {} after warm-up, {} at the end (slack {COUNT_SLACK})",
            order.label(),
            run.warm.fds,
            run.end.fds
        ),
    );
    Ok(findings.verdict())
}

fn order_loop(
    ctx: &Ctx,
    order: Order,
    n: usize,
    video: &SegmentedStream,
    audio: &SegmentedStream,
    big: &[u8],
    progress: &Progress,
) -> Result<OrderRun, String> {
    let cold = proc::sample()?;
    let warm_n = warmup_for(n);
    let started = Instant::now();
    let mut rss_kb = Vec::new();
    let mut iteration_ms = Vec::new();
    let mut shutdown_ms = Vec::new();
    let mut shutdown_messages = 0;
    let mut notes = Vec::new();
    let mut reattach_plays = 0usize;
    let mut warm = cold;
    let mut budget_exhausted = false;
    for iteration in 0..n {
        if started.elapsed() > ORDER_BUDGET {
            budget_exhausted = true;
            break;
        }
        let begun = Instant::now();
        let outcome =
            one_iteration(ctx, order, video, audio, big, progress, iteration).map_err(|why| {
                format!(
                    "{} iteration {iteration}: {why} (RSS {} KB)",
                    order.label(),
                    proc::rss_kb().unwrap_or(0)
                )
            })?;
        shutdown_ms.push(ms(outcome.shutdown));
        shutdown_messages += outcome.messages;
        if outcome.reattached_and_played {
            reattach_plays += 1;
        }
        if let Some(note) = outcome.note
            && notes.len() < 5
        {
            notes.push(note);
        }
        iteration_ms.push(ms(begun.elapsed()));
        progress.set(iteration, "sample");
        rss_kb.push(proc::rss_kb()?);
        if iteration + 1 == warm_n {
            warm = proc::sample()?;
        }
    }
    let iterations_done = rss_kb.len();
    progress.set(iterations_done, "settle");
    // Give background threads a moment to finish before the closing sample.
    let mut end = proc::sample()?;
    for _ in 0..16 {
        if end.threads <= warm.threads + COUNT_SLACK {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
        end = proc::sample()?;
    }
    if iterations_done < warm_n {
        warm = cold;
    }
    Ok(OrderRun {
        iterations_done,
        rss_kb,
        iteration_ms,
        shutdown_ms,
        cold,
        warm,
        end,
        shutdown_messages,
        reattach_plays: (order == Order::ReattachToSecondElement).then_some(reattach_plays),
        notes,
        budget_exhausted,
    })
}

struct IterationOutcome {
    shutdown: Duration,
    messages: usize,
    reattached_and_played: bool,
    note: Option<String>,
}

fn one_iteration(
    ctx: &Ctx,
    order: Order,
    video: &SegmentedStream,
    audio: &SegmentedStream,
    big: &[u8],
    progress: &Progress,
    iteration: usize,
) -> Result<IterationOutcome, String> {
    progress.set(iteration, "Rig::new");
    let rig = new_rig(fast_config(ctx))?;
    progress.set(iteration, "add_source_buffer");
    let ids = add_buffers(&rig, &video.mime, &audio.mime)?;
    let vid = ids[0];
    progress.set(iteration, "append");
    let first_video = video
        .media
        .first()
        .map(|s| s.bytes.as_slice())
        .unwrap_or_default();
    let first_audio = audio
        .media
        .first()
        .map(|s| s.bytes.as_slice())
        .unwrap_or_default();
    let inits = [video.init.bytes.as_slice(), audio.init.bytes.as_slice()];
    let firsts = [first_video, first_audio];
    for bytes in [inits, firsts] {
        for (id, bytes) in ids.iter().zip(bytes) {
            rig.append_and_wait(*id, bytes, APPEND_TIMEOUT)
                .map_err(|e| format!("append of {} bytes: {e}", bytes.len()))?;
        }
    }
    progress.set(iteration, "play");
    rig.play().map_err(|e| format!("play: {e}"))?;
    // "Play 0.3 s": until three frames reached the sink, at most 0.3 s.
    rig.wait_until(Duration::from_millis(300), |rig| video_frames(rig) >= 3);

    // MSE_STRESS_T04_QUIET_MS: a quiet period before the teardown. GstMSE's source-buffer task parks in a
    // 1 s timed wait; a teardown that comes less than a second after the last activity crashes (a calling
    // discipline found with `examples/deadlock_repro --pre-null-ms`).
    if let Some(quiet) = part("MSE_STRESS_T04_QUIET_MS").and_then(|v| v.parse::<u64>().ok()) {
        progress.set(iteration, "quiet period");
        std::thread::sleep(Duration::from_millis(quiet));
    }
    progress.set(iteration, "teardown");
    let mut note = None;
    let mut reattached_and_played = false;
    let started = Instant::now();
    let info = match order {
        Order::NullThenBackend => rig.shutdown_with(TeardownOrder::PipelineNullThenBackend),
        Order::BackendThenNull => rig.shutdown_with(TeardownOrder::BackendThenPipelineNull),
        Order::DetachDuringAppend => {
            let started_append = rig.backend.append(vid, big);
            if iteration == 0 {
                note = Some(format!(
                    "the big append ({} bytes) was started with result {started_append:?}",
                    big.len()
                ));
            }
            rig.shutdown_with(TeardownOrder::DetachThenNull)
        }
        Order::NullDuringAppend => {
            let started_append = rig.backend.append(vid, big);
            if iteration == 0 {
                note = Some(format!(
                    "the big append ({} bytes) was started with result {started_append:?}",
                    big.len()
                ));
            }
            rig.shutdown_with(TeardownOrder::PipelineNullThenBackend)
        }
        Order::NullWithSeekPending => {
            let seek = rig.seek(ClockTime::from_seconds(6));
            if iteration == 0 {
                note = Some(format!(
                    "seek to 6 s (not buffered) requested with result {seek:?}"
                ));
            }
            rig.shutdown_with(TeardownOrder::PipelineNullThenBackend)
        }
        Order::DropWithoutNull => rig.shutdown_with(TeardownOrder::DropWithoutNull),
        Order::ReattachToSecondElement => {
            progress.set(iteration, "reattach");
            let outcome = reattach(ctx, rig, video, progress, iteration)?;
            reattached_and_played = outcome.0;
            if iteration == 0 {
                note = Some(outcome.1);
            }
            return Ok(IterationOutcome {
                shutdown: started.elapsed(),
                messages: outcome.2,
                reattached_and_played,
                note,
            });
        }
    };
    let took = started.elapsed();
    let info = info.map_err(|e| format!("shutdown refused: {e}"))?;
    Ok(IterationOutcome {
        shutdown: took,
        messages: info.messages.len(),
        reattached_and_played,
        note,
    })
}

/// Order 7: detach the first rig's MediaSource, attach it to the `msesrc` of a second pipeline whose
/// own MediaSource was detached first, append to it and see whether the second pipeline plays.
fn reattach(
    ctx: &Ctx,
    rig: ManagedRig,
    video: &SegmentedStream,
    progress: &Progress,
    iteration: usize,
) -> Result<(bool, String, usize), String> {
    rig.backend.detach();
    let second = new_rig(fast_config(ctx))?;
    second.backend.detach();
    let target = second
        .msesrc()
        .ok_or_else(|| "the second rig has no msesrc".to_owned())?;
    progress.set(iteration, "attach to the second element");
    let attached = rig.backend.attach(&target);
    let mut played = false;
    let mut text =
        format!("re-attach of the first MediaSource to the second element gave {attached:?}");
    if attached.is_ok() {
        let opened = rig.wait_until(Duration::from_secs(2), |rig| {
            rig.backend.ready_state() == mse_rig::backend::MediaSourceReadyState::Open
        });
        text.push_str(&format!("; ready state open again: {opened}"));
        if opened {
            progress.set(iteration, "append to the second element");
            match rig.backend.add_source_buffer(&video.mime) {
                Ok(id) => {
                    let appended = rig
                        .append_and_wait(id, &video.init.bytes, APPEND_TIMEOUT)
                        .and_then(|_| {
                            video.media.first().map_or(Ok(()), |segment| {
                                rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT)
                                    .map(|_| ())
                            })
                        });
                    text.push_str(&format!("; appends gave {appended:?}"));
                    if appended.is_ok() {
                        let _ = second.play();
                        played =
                            second.wait_until(Duration::from_secs(3), |s| video_frames(s) >= 3);
                        text.push_str(&format!("; the second element played: {played}"));
                    }
                }
                Err(error) => text.push_str(&format!(
                    "; add_source_buffer on the re-attached source gave {error:?}"
                )),
            }
        }
    }
    progress.set(iteration, "teardown");
    let mut messages = 0;
    if let Ok(info) = second.shutdown() {
        messages += info.messages.len();
    }
    if let Ok(info) = rig.shutdown() {
        messages += info.messages.len();
    }
    Ok((played, text, messages))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seven_orders_have_distinct_labels_and_indexes() {
        let mut labels: Vec<&str> = ORDERS.iter().map(|o| o.label()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 7);
        for (i, order) in ORDERS.iter().enumerate() {
            assert_eq!(order.index(), i);
        }
    }

    #[test]
    fn only_the_append_orders_need_the_big_append() {
        let needing: Vec<&str> = ORDERS
            .iter()
            .filter(|o| o.needs_big_append())
            .map(|o| o.label())
            .collect();
        assert_eq!(needing, ["detach-during-append", "null-during-append"]);
    }

    #[test]
    fn warm_up_is_a_twentieth_between_three_and_ten() {
        assert_eq!(warmup_for(200), 10);
        assert_eq!(warmup_for(40), 3);
        assert_eq!(warmup_for(1), 3);
        assert_eq!(warmup_for(100), 5);
    }

    #[test]
    fn the_progress_record_remembers_the_stage() {
        let progress = Progress {
            iteration: AtomicUsize::new(0),
            stage: Mutex::new(String::new()),
            changed: Mutex::new(Instant::now()),
            path: None,
        };
        progress.set(7, "append");
        assert_eq!(progress.iteration.load(Ordering::SeqCst), 7);
        assert_eq!(*progress.stage.lock().unwrap(), "append");
    }

    #[test]
    fn the_default_iteration_count_is_the_briefs() {
        assert_eq!(DEFAULT_ITERATIONS, 200);
        assert_eq!(json!(LEAK_SLOPE_KB), json!(1024.0));
    }
}
