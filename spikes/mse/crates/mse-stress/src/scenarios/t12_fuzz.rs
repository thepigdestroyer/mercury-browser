// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T12: fuzz. From a seed, random sequences of 300 operations from {add, append (random chunking),
//! seek, remove, abort, set_duration, end_of_stream, change_type, detach, play, pause}; 50 seeds, each
//! in a process of its own (a nested run of this executable with `--only T12 --seed N`). No crash,
//! no hang over 10 s (a hang prints the seed and the last 10 operations), and errors only of the
//! documented kinds (`InvalidState`, `Type`, `NotSupported`, `NotFound`, `QuotaExceeded`).
//!
//! Replay one seed: `MSE_STRESS_T12_SEEDS=1 mse-stress --only T12 --seed N`.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mse_rig::ClockTime;
use mse_rig::RigError;
use mse_rig::backend::{EosError, MseError};
use mse_rig::events::{MseEvent, PlayEvent, SourceBufferId};
use mse_rig::harness::Ctx;
use mse_rig::report::{ScenarioResult, Status};
use mse_segments::{ChunkPlan, SegmentedStream, chunks};
use serde_json::json;

use crate::common::{
    Findings, ManagedRig, Verdict, fast_config, new_rig, realtime_config, run_scenario,
};
use crate::rng::Rng;
use crate::sub::{SubOutcome, SubSpec, part, run_all};

pub const SEED_RUN_VARIABLE: &str = "MSE_STRESS_T12_SEED_RUN";
pub const PROGRESS_VARIABLE: &str = "MSE_STRESS_T12_PROGRESS";
pub const SEEDS_VARIABLE: &str = "MSE_STRESS_T12_SEEDS";

pub const DEFAULT_SEEDS: u64 = 50;
pub const OPERATIONS: usize = 300;

/// A single operation that takes longer than this is a hang.
pub const HANG: Duration = Duration::from_secs(10);

/// The fixtures the fuzzer draws from (missing ones are left out).
const BANK: &[&str] = &["h264-low", "aac", "vp9-low", "opus", "h264-high"];

/// Operation weights, in the order of `OP_NAMES`.
const WEIGHTS: [u32; 11] = [8, 40, 8, 8, 5, 5, 4, 4, 1, 6, 4];
const OP_NAMES: [&str; 11] = [
    "add",
    "append",
    "seek",
    "remove",
    "abort",
    "set_duration",
    "end_of_stream",
    "change_type",
    "detach",
    "play",
    "pause",
];

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario("T12", "Fuzz", ctx, |ctx, result| {
        match part(SEED_RUN_VARIABLE) {
            Some(_) => run_seed(ctx, result),
            None => run_seeds(ctx, result),
        }
    })
}

/// The seeds of a run: `count` consecutive values from `base` (wrapping).
pub fn seeds(base: u64, count: u64) -> Vec<u64> {
    (0..count).map(|i| base.wrapping_add(i)).collect()
}

// ---------------------------------------------------------------------------------------------
// The driver: one nested run per seed.

fn run_seeds(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let count = part(SEEDS_VARIABLE)
        .and_then(|text| text.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SEEDS);
    let base = ctx.seed;
    let list = seeds(base, count);
    result.note(format!(
        "seeds {base}..{} ({count} seeds, {OPERATIONS} operations each); replay one with MSE_STRESS_T12_SEEDS=1 and --seed N",
        base.wrapping_add(count).wrapping_sub(1)
    ));
    result.metric("base_seed", base);
    result.metric("seeds", count);
    let specs: Vec<SubSpec> = list
        .iter()
        .map(|seed| SubSpec {
            id: "T12".to_owned(),
            tag: format!("seed-{seed}"),
            env: vec![
                (SEED_RUN_VARIABLE.to_owned(), "1".to_owned()),
                (
                    PROGRESS_VARIABLE.to_owned(),
                    ctx.scratch
                        .join(format!("progress-{seed}.txt"))
                        .display()
                        .to_string(),
                ),
            ],
            iterations: None,
            seed: Some(*seed),
            deadline: Duration::from_secs(150),
        })
        .collect();
    let since = std::time::SystemTime::now();
    let outcomes = run_all(ctx, &specs, 6, Duration::from_secs(245));
    let mut findings = Findings::new();
    let mut clean = 0u64;
    let mut totals: BTreeMap<String, u64> = BTreeMap::new();
    let mut failing = Vec::new();
    let mut not_run: Vec<u64> = Vec::new();
    for (seed, outcome) in list.iter().zip(outcomes) {
        let progress = std::fs::read_to_string(ctx.scratch.join(format!("progress-{seed}.txt")))
            .unwrap_or_default();
        let last_lines: Vec<&str> = progress.lines().collect();
        match outcome {
            SubOutcome::Report { result: sub, exit } => match sub.status {
                Status::Pass => {
                    clean += 1;
                    if let Some(map) = sub.metrics.get("outcomes").and_then(|v| v.as_object()) {
                        for (key, value) in map {
                            *totals.entry(key.clone()).or_default() += value.as_u64().unwrap_or(0);
                        }
                    }
                }
                Status::Fail => {
                    failing.push(*seed);
                    for note in &sub.notes {
                        if let Some(reason) = note.strip_prefix("FAIL: ") {
                            findings.fail(format!("seed {seed}: {reason}"));
                        }
                    }
                }
                Status::Skip => {
                    result.note(format!("seed {seed} skipped: {}", sub.notes.join(" | ")));
                }
                Status::Error => {
                    if sub.notes.iter().any(|n| n.to_lowercase().contains("panic")) {
                        return Err(format!("seed {seed}: {}", sub.notes.join(" | ")));
                    }
                    failing.push(*seed);
                    findings.fail(format!(
                        "seed {seed}: the process died without a result ({exit}); last operations: {last_lines:?}"
                    ));
                }
            },
            SubOutcome::NotRun => not_run.push(*seed),
            other => {
                failing.push(*seed);
                findings.fail(format!(
                    "seed {seed}: {}; last operations: {last_lines:?}",
                    other.describe()
                ));
            }
        }
    }
    result.metric("clean_seeds", clean);
    let crashes = crate::sub::crash_summaries_since(since);
    result.metric("crash_reports", json!(crashes));
    for crash in &crashes {
        result.note(format!("macOS {crash}"));
    }
    result.metric("seeds_not_run", json!(not_run));
    if !not_run.is_empty() {
        result.note(format!(
            "INCOMPLETE: {} of {count} seeds were not run because the time budget ran out (first: {:?})",
            not_run.len(),
            not_run.first()
        ));
    }
    result.metric("failing_seeds", json!(failing));
    result.metric("outcome_totals", json!(totals));
    result.note(format!(
        "{clean} of {count} seeds ran clean; outcomes over the clean seeds: {totals:?}"
    ));
    Ok(findings.verdict())
}

// ---------------------------------------------------------------------------------------------
// One seed.

/// What an operation's call gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// One of the documented refusals.
    Refused(&'static str),
    /// An error of another domain: a finding.
    Undocumented(String),
    /// The pipeline (not MSE) refused: recorded, not a finding.
    PipelineRefused(String),
}

pub fn classify_mse<T>(result: Result<T, MseError>) -> Outcome {
    match result {
        Ok(_) => Outcome::Ok,
        Err(MseError::InvalidState(_)) => Outcome::Refused("InvalidState"),
        Err(MseError::Type(_)) => Outcome::Refused("Type"),
        Err(MseError::NotSupported(_)) => Outcome::Refused("NotSupported"),
        Err(MseError::NotFound(_)) => Outcome::Refused("NotFound"),
        Err(MseError::QuotaExceeded(_)) => Outcome::Refused("QuotaExceeded"),
        Err(MseError::Other(text)) => Outcome::Undocumented(text),
    }
}

pub fn classify_rig<T>(result: Result<T, RigError>) -> Outcome {
    match result {
        Ok(_) => Outcome::Ok,
        Err(RigError::Mse(error)) => classify_mse::<()>(Err(error)),
        Err(other) => Outcome::PipelineRefused(other.to_string()),
    }
}

impl Outcome {
    fn label(&self) -> String {
        match self {
            Outcome::Ok => "ok".to_owned(),
            Outcome::Refused(kind) => format!("refused:{kind}"),
            Outcome::Undocumented(_) => "UNDOCUMENTED".to_owned(),
            Outcome::PipelineRefused(_) => "pipeline-refused".to_owned(),
        }
    }
}

/// The operations in flight and the last ten, mirrored to a file so a crash leaves them behind.
struct Journal {
    ring: Mutex<VecDeque<String>>,
    current: Mutex<(usize, String, Instant)>,
    finished: AtomicUsize,
    path: Option<PathBuf>,
}

impl Journal {
    fn begin(&self, number: usize, description: &str) {
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) =
            (number, description.to_owned(), Instant::now());
        let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        ring.push_back(format!("#{number} {description}"));
        while ring.len() > 10 {
            ring.pop_front();
        }
        if let Some(path) = &self.path {
            let text: Vec<String> = ring.iter().cloned().collect();
            let _ = std::fs::write(path, text.join("\n"));
        }
    }

    fn end(&self, outcome: &Outcome) {
        self.finished.fetch_add(1, Ordering::SeqCst);
        let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = ring.back_mut() {
            last.push_str(&format!(" -> {}", outcome.label()));
        }
    }

    fn last_ops(&self) -> Vec<String> {
        self.ring
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

struct Buf {
    id: SourceBufferId,
    stream: usize,
    inited: bool,
    next: usize,
}

struct World {
    rig: ManagedRig,
    bank: Vec<(&'static str, SegmentedStream)>,
    buffers: Vec<Buf>,
    detached: bool,
    duration_ms: u64,
}

/// A chunk plan for `len` bytes with at most about 200 pieces.
pub fn random_plan(rng: &mut Rng, len: usize) -> ChunkPlan {
    let floor = len / 200 + 1;
    match rng.weighted(&[60, 25, 15]) {
        0 => ChunkPlan::Whole,
        1 => ChunkPlan::Fixed((rng.range(256, 65_536) as usize).max(floor)),
        _ => ChunkPlan::Sizes(vec![
            rng.range(1, 100) as usize,
            rng.range(100, 5000) as usize,
            (rng.range(1, 30_000) as usize).max(floor),
        ]),
    }
}

fn plan_text(plan: &ChunkPlan) -> String {
    match plan {
        ChunkPlan::Whole => "whole".to_owned(),
        ChunkPlan::Fixed(n) => format!("fixed({n})"),
        ChunkPlan::Sizes(v) => format!("sizes({v:?})"),
    }
}

fn pick_buffer(world: &World, rng: &mut Rng) -> Option<usize> {
    if world.buffers.is_empty() {
        None
    } else {
        Some(rng.below(world.buffers.len() as u64) as usize)
    }
}

/// Performs operation `kind` and returns its description and outcome.
fn operate(world: &mut World, rng: &mut Rng, kind: usize) -> (String, Outcome) {
    match kind {
        0 => {
            let stream = rng.below(world.bank.len() as u64) as usize;
            let (name, data) = &world.bank[stream];
            let outcome = match world.rig.backend.add_source_buffer(&data.mime) {
                Ok(id) => {
                    world.buffers.push(Buf {
                        id,
                        stream,
                        inited: false,
                        next: 0,
                    });
                    Outcome::Ok
                }
                Err(error) => classify_mse::<()>(Err(error)),
            };
            (format!("add stream={name}"), outcome)
        }
        1 => {
            let Some(index) = pick_buffer(world, rng) else {
                return ("append (no source buffer yet)".to_owned(), Outcome::Ok);
            };
            let way = rng.weighted(&[70, 10, 10, 10]);
            let (id, stream_index) = (world.buffers[index].id, world.buffers[index].stream);
            let stream_len = world.bank[stream_index].1.media.len().max(1);
            let (what, bytes): (String, Vec<u8>) = match way {
                0 => {
                    let buffer = &mut world.buffers[index];
                    let data = &world.bank[stream_index].1;
                    if buffer.inited {
                        let at = buffer.next % stream_len;
                        buffer.next += 1;
                        (
                            format!("next(media {at})"),
                            data.media
                                .get(at)
                                .map(|s| s.bytes.clone())
                                .unwrap_or_default(),
                        )
                    } else {
                        buffer.inited = true;
                        ("init".to_owned(), data.init.bytes.clone())
                    }
                }
                1 => {
                    let at = rng.below(stream_len as u64) as usize;
                    (
                        format!("random(media {at})"),
                        world.bank[stream_index]
                            .1
                            .media
                            .get(at)
                            .map(|s| s.bytes.clone())
                            .unwrap_or_default(),
                    )
                }
                2 => (
                    "init again".to_owned(),
                    world.bank[stream_index].1.init.bytes.clone(),
                ),
                _ => {
                    let other = rng.below(world.bank.len() as u64) as usize;
                    let data = &world.bank[other].1;
                    let at = rng.below(data.media.len() as u64 + 1) as usize;
                    let bytes = if at == 0 {
                        data.init.bytes.clone()
                    } else {
                        data.media
                            .get(at - 1)
                            .map(|s| s.bytes.clone())
                            .unwrap_or_default()
                    };
                    (format!("mismatched({} {})", world.bank[other].0, at), bytes)
                }
            };
            let plan = random_plan(rng, bytes.len());
            let desc = format!(
                "append buf={id} stream={} {what} plan={} bytes={}",
                world.bank[stream_index].0,
                plan_text(&plan),
                bytes.len()
            );
            let mut outcome = Outcome::Ok;
            for piece in chunks(&bytes, &plan) {
                outcome = classify_mse(world.rig.backend.append(id, piece));
                if outcome != Outcome::Ok {
                    break;
                }
                world
                    .rig
                    .wait_until(Duration::from_secs(3), |rig| !rig.backend.updating(id));
            }
            (desc, outcome)
        }
        2 => {
            let to = rng.below(world.duration_ms + 2000);
            (
                format!("seek to {to} ms"),
                classify_rig(world.rig.seek(ClockTime::from_mseconds(to))),
            )
        }
        3 => {
            let Some(index) = pick_buffer(world, rng) else {
                return ("remove (no source buffer yet)".to_owned(), Outcome::Ok);
            };
            let id = world.buffers[index].id;
            let start = rng.below(10_000);
            let end = if rng.chance(20) {
                None
            } else {
                Some(start + rng.below(8000))
            };
            (
                format!("remove buf={id} {start} ms..{end:?}"),
                classify_mse(world.rig.backend.remove(
                    id,
                    ClockTime::from_mseconds(start),
                    end.map(ClockTime::from_mseconds),
                )),
            )
        }
        4 => {
            let Some(index) = pick_buffer(world, rng) else {
                return ("abort (no source buffer yet)".to_owned(), Outcome::Ok);
            };
            let id = world.buffers[index].id;
            (
                format!("abort buf={id}"),
                classify_mse(world.rig.backend.abort(id)),
            )
        }
        5 => {
            let duration = if rng.chance(20) {
                None
            } else {
                Some(rng.below(20_000))
            };
            (
                format!("set_duration {duration:?} ms"),
                classify_mse(
                    world
                        .rig
                        .backend
                        .set_duration(duration.map(ClockTime::from_mseconds)),
                ),
            )
        }
        6 => {
            let error = match rng.weighted(&[70, 15, 15]) {
                0 => EosError::None,
                1 => EosError::Network,
                _ => EosError::Decode,
            };
            (
                format!("end_of_stream {error:?}"),
                classify_mse(world.rig.backend.end_of_stream(error)),
            )
        }
        7 => {
            let Some(index) = pick_buffer(world, rng) else {
                return ("change_type (no source buffer yet)".to_owned(), Outcome::Ok);
            };
            let id = world.buffers[index].id;
            let target = rng.below(world.bank.len() as u64) as usize;
            let (name, data) = &world.bank[target];
            (
                format!("change_type buf={id} to {name}"),
                classify_mse(world.rig.backend.change_type(id, &data.mime)),
            )
        }
        8 => {
            if world.detached {
                ("detach (already detached)".to_owned(), Outcome::Ok)
            } else {
                world.rig.backend.detach();
                world.detached = true;
                ("detach".to_owned(), Outcome::Ok)
            }
        }
        9 => ("play".to_owned(), classify_rig(world.rig.play())),
        _ => ("pause".to_owned(), classify_rig(world.rig.pause())),
    }
}

struct SeedReport {
    operations: usize,
    outcomes: BTreeMap<String, u64>,
    by_operation: BTreeMap<&'static str, u64>,
    undocumented: Vec<String>,
    pipeline_refusals: Vec<String>,
    play_errors: usize,
}

fn fuzz_worker(ctx: &Ctx, seed: u64, journal: &Journal) -> Result<SeedReport, String> {
    let mut bank = Vec::new();
    for name in BANK {
        if let Ok(stream) = mse_segments::load(name) {
            bank.push((*name, stream));
        }
    }
    if bank.len() < 2 {
        return Err(format!(
            "only {} fixtures of the bank are available",
            bank.len()
        ));
    }
    let duration_ms = bank
        .iter()
        .filter_map(|(_, s)| s.duration_ms())
        .max()
        .unwrap_or(8000);
    let config = if seed.is_multiple_of(2) {
        fast_config(ctx)
    } else {
        realtime_config(ctx)
    };
    journal.begin(0, "Rig::new");
    let rig = new_rig(config)?;
    let mut world = World {
        rig,
        bank,
        buffers: Vec::new(),
        detached: false,
        duration_ms,
    };
    let mut rng = Rng::new(seed);
    let mut report = SeedReport {
        operations: 0,
        outcomes: BTreeMap::new(),
        by_operation: BTreeMap::new(),
        undocumented: Vec::new(),
        pipeline_refusals: Vec::new(),
        play_errors: 0,
    };
    for number in 1..=OPERATIONS {
        let kind = rng.weighted(&WEIGHTS);
        // The description is known only after the operation has chosen its parameters, so the journal
        // first says which kind is running, then (below) what it was.
        journal.begin(
            number,
            &format!("{} (parameters not chosen yet)", OP_NAMES[kind]),
        );
        let (description, outcome) = operate(&mut world, &mut rng, kind);
        journal.begin(number, &description);
        journal.end(&outcome);
        report.operations += 1;
        *report.outcomes.entry(outcome.label()).or_default() += 1;
        *report.by_operation.entry(OP_NAMES[kind]).or_default() += 1;
        match &outcome {
            Outcome::Undocumented(text) => report
                .undocumented
                .push(format!("#{number} {description}: {text}")),
            Outcome::PipelineRefused(text) if report.pipeline_refusals.len() < 5 => {
                report
                    .pipeline_refusals
                    .push(format!("#{number} {description}: {text}"));
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(rng.below(4)));
    }
    journal.begin(OPERATIONS + 1, "shutdown");
    report.play_errors = world
        .rig
        .log
        .count(|e| matches!(e, MseEvent::Play(PlayEvent::Error(_))));
    let _ = world.rig.shutdown();
    journal.end(&Outcome::Ok);
    Ok(report)
}

fn run_seed(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let seed = ctx.seed;
    let progress_path = part(PROGRESS_VARIABLE).map(PathBuf::from);
    let journal = Arc::new(Journal {
        ring: Mutex::new(VecDeque::new()),
        current: Mutex::new((0, String::new(), Instant::now())),
        finished: AtomicUsize::new(0),
        path: progress_path,
    });
    result.metric("seed", seed);
    result.note(format!(
        "seed {seed}: {OPERATIONS} operations, sync={}",
        !seed.is_multiple_of(2)
    ));
    let (sender, receiver) = mpsc::channel();
    let (worker_ctx, worker_journal) = (ctx.clone(), Arc::clone(&journal));
    std::thread::Builder::new()
        .name("t12-fuzz".into())
        .spawn(move || {
            let _ = sender.send(fuzz_worker(&worker_ctx, seed, &worker_journal));
        })
        .map_err(|e| format!("cannot start the fuzz thread: {e}"))?;

    let mut findings = Findings::new();
    loop {
        match receiver.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(report)) => {
                result.metric("operations", report.operations);
                result.metric("outcomes", json!(report.outcomes));
                result.metric("by_operation", json!(report.by_operation));
                result.metric("pipeline_refusals", json!(report.pipeline_refusals));
                result.metric("play_errors", report.play_errors);
                result.metric("last_ops", json!(journal.last_ops()));
                result.note(format!(
                    "{} operations done; outcomes {:?}; {} pipeline error message(s)",
                    report.operations, report.outcomes, report.play_errors
                ));
                for undocumented in &report.undocumented {
                    findings.fail(format!("an error of an undocumented kind: {undocumented}"));
                }
                if !findings.is_clean() {
                    findings.fail(format!("last operations: {:?}", journal.last_ops()));
                }
                return Ok(findings.verdict());
            }
            Ok(Err(error)) => return Err(format!("seed {seed}: {error}")),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let (number, description, since) = journal
                    .current
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if since.elapsed() > HANG {
                    result.metric("last_ops", json!(journal.last_ops()));
                    findings.fail(format!(
                        "hang: operation #{number} `{description}` did not return within {} s; last 10 operations: {:?}",
                        HANG.as_secs(),
                        journal.last_ops()
                    ));
                    return Ok(findings.verdict());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(format!(
                    "seed {seed}: the fuzz thread panicked; last operations: {:?}",
                    journal.last_ops()
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_consecutive() {
        assert_eq!(seeds(10, 3), [10, 11, 12]);
        assert_eq!(seeds(u64::MAX, 2), [u64::MAX, 0]);
        assert_eq!(seeds(5, 0), Vec::<u64>::new());
    }

    #[test]
    fn the_operation_tables_agree() {
        assert_eq!(WEIGHTS.len(), OP_NAMES.len());
        assert_eq!(WEIGHTS.iter().sum::<u32>(), 93);
        assert_eq!(OP_NAMES[1], "append");
        assert_eq!(DEFAULT_SEEDS, 50);
        assert_eq!(OPERATIONS, 300);
    }

    #[test]
    fn documented_errors_are_refusals_and_other_is_a_finding() {
        assert_eq!(
            classify_mse::<()>(Err(MseError::InvalidState("x".into()))),
            Outcome::Refused("InvalidState")
        );
        assert_eq!(
            classify_mse::<()>(Err(MseError::QuotaExceeded("x".into()))),
            Outcome::Refused("QuotaExceeded")
        );
        assert_eq!(classify_mse(Ok(1)), Outcome::Ok);
        assert_eq!(
            classify_mse::<()>(Err(MseError::Other("boom".into()))),
            Outcome::Undocumented("boom".into())
        );
        assert!(matches!(
            classify_rig::<()>(Err(RigError::Pipeline("no".into()))),
            Outcome::PipelineRefused(_)
        ));
        assert_eq!(
            classify_rig::<()>(Err(RigError::Mse(MseError::NotFound("x".into())))),
            Outcome::Refused("NotFound")
        );
    }

    #[test]
    fn plans_never_cut_into_more_than_about_two_hundred_pieces() {
        let mut rng = Rng::new(3);
        for _ in 0..500 {
            let len = rng.range(1, 400_000) as usize;
            let plan = random_plan(&mut rng, len);
            let data = vec![0u8; len];
            let pieces = chunks(&data, &plan).len();
            assert!(
                pieces <= 260,
                "{plan:?} on {len} bytes gave {pieces} pieces"
            );
        }
    }

    #[test]
    fn the_journal_keeps_the_last_ten_operations() {
        let journal = Journal {
            ring: Mutex::new(VecDeque::new()),
            current: Mutex::new((0, String::new(), Instant::now())),
            finished: AtomicUsize::new(0),
            path: None,
        };
        for number in 1..=15 {
            journal.begin(number, "op");
            journal.end(&Outcome::Refused("InvalidState"));
        }
        let ops = journal.last_ops();
        assert_eq!(ops.len(), 10);
        assert!(ops[0].starts_with("#6 "));
        assert!(ops[9].ends_with("refused:InvalidState"));
        assert_eq!(journal.finished.load(Ordering::SeqCst), 15);
    }

    #[test]
    fn the_journal_is_mirrored_to_a_file() {
        let path = std::env::temp_dir().join(format!(
            "mse-stress-journal-test-{}.txt",
            std::process::id()
        ));
        let journal = Journal {
            ring: Mutex::new(VecDeque::new()),
            current: Mutex::new((0, String::new(), Instant::now())),
            finished: AtomicUsize::new(0),
            path: Some(path.clone()),
        };
        journal.begin(1, "append buf=0");
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(text, "#1 append buf=0");
    }
}
