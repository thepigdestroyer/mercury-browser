// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What every stress scenario needs: rig configurations, fixtures, appending whole streams, reading
//! the counting sinks, event-order checks and the verdict plumbing.
//!
//! Verdicts: a scenario function returns `Result<Verdict, String>`; `Err` is a harness problem
//! (status `Error`: the fixture is missing, the rig could not be built), `Ok(Verdict::Fail(..))` is a
//! finding about GstMSE (status `Fail`).

use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::Mutex;
use std::time::Duration;

use mse_rig::RigError;
use mse_rig::backend::EosError;
use mse_rig::events::{MseEvent, SourceBufferId, TimedEvent};
use mse_rig::frames::SharedStats;
use mse_rig::harness::Ctx;
use mse_rig::report::{ScenarioResult, Status};
use mse_rig::rig::{Rig, RigConfig, ShutdownInfo, TeardownOrder};
use mse_segments::SegmentedStream;
use serde_json::json;

/// 8 s at 25 fps: what C02 expects at the video sink (plus or minus [`FRAME_TOLERANCE`]).
pub const FIXTURE_FRAMES: u64 = 200;
pub const FRAME_TOLERANCE: u64 = 2;

/// How long one append may take before the scenario calls it stuck.
pub const APPEND_TIMEOUT: Duration = Duration::from_secs(10);

/// A scenario's outcome when the scenario itself ran correctly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    /// GstMSE misbehaved: the reasons, one per line, go into the notes.
    Fail(Vec<String>),
    Skip(String),
}

/// Runs `body`, fills `status`, `notes` and `elapsed_ms`. `Err` becomes `Status::Error`.
pub fn run_scenario(
    id: &str,
    title: &str,
    ctx: &Ctx,
    body: impl FnOnce(&Ctx, &mut ScenarioResult) -> Result<Verdict, String>,
) -> ScenarioResult {
    let started = std::time::Instant::now();
    let mut result = ScenarioResult::new(id, title);
    note_config(&mut result, ctx);
    result.metric(
        "tracks",
        if audio_enabled() {
            "video and audio"
        } else {
            "video only (MSE_STRESS_TRACKS=video)"
        },
    );
    let outcome = if !ctx.info.msesrc_available {
        Ok(Verdict::Skip(
            "msesrc is not available in this run (the mse plugin did not load)".to_owned(),
        ))
    } else {
        body(ctx, &mut result)
    };
    match outcome {
        Ok(Verdict::Pass) => result.status = Status::Pass,
        Ok(Verdict::Fail(reasons)) => {
            result.status = Status::Fail;
            for reason in reasons {
                result.note(format!("FAIL: {reason}"));
            }
        }
        Ok(Verdict::Skip(reason)) => {
            result.status = Status::Skip;
            result.note(format!("SKIP: {reason}"));
        }
        Err(problem) => {
            result.status = Status::Error;
            result.note(format!("ERROR (harness): {problem}"));
        }
    }
    let deadlocks = take_deadlocks();
    if !deadlocks.is_empty() {
        if result.status == Status::Error {
            result.note("the harness error above came with a deadlock: the pipeline was probably stuck, which is the finding");
        }
        result.status = Status::Fail;
        for deadlock in &deadlocks {
            result.note(format!("FAIL: {deadlock}"));
        }
        result.metric("deadlocks", serde_json::json!(deadlocks));
    }
    result.elapsed_ms = started.elapsed().as_millis() as u64;
    result
}

/// Accumulates failure reasons; `verdict()` is `Pass` when none were added.
#[derive(Debug, Default)]
pub struct Findings {
    reasons: Vec<String>,
}

impl Findings {
    pub fn new() -> Self {
        Findings::default()
    }

    pub fn fail(&mut self, reason: impl Into<String>) {
        self.reasons.push(reason.into());
    }

    /// Adds a failure when `condition` is false.
    pub fn check(&mut self, condition: bool, reason: impl Into<String>) {
        if !condition {
            self.fail(reason);
        }
    }

    pub fn is_clean(&self) -> bool {
        self.reasons.is_empty()
    }

    #[cfg(test)]
    pub fn reasons(&self) -> &[String] {
        &self.reasons
    }

    pub fn verdict(self) -> Verdict {
        if self.reasons.is_empty() {
            Verdict::Pass
        } else {
            Verdict::Fail(self.reasons)
        }
    }
}

/// `shape=playbin plugin-mode=system glib-main-loop=false`: the first note of every result.
pub fn config_line(ctx: &Ctx) -> String {
    format!(
        "ran with shape={} plugin-mode={} glib-main-loop={} on {} (GStreamer {})",
        ctx.shape.label(),
        crate::sub::plugin_mode_arg(&ctx.plugin_mode),
        ctx.glib_main_loop,
        ctx.info.arch,
        ctx.info.version
    )
}

pub fn note_config(result: &mut ScenarioResult, ctx: &Ctx) {
    result.note(config_line(ctx));
    result.metric("shape", ctx.shape.label());
    result.metric("plugin_mode", crate::sub::plugin_mode_arg(&ctx.plugin_mode));
    result.metric("glib_main_loop", ctx.glib_main_loop);
    for (plugin, why) in &ctx.info.failed_plugins {
        result.note(format!("plugin {plugin} did not load: {why}"));
    }
}

/// Whether the A/V scenarios feed an audio source buffer as well as the video one. GstMSE 1.28.9 deadlocks
/// with two source buffers on this Mac (see `stress-findings.md`), so `MSE_STRESS_TRACKS=video` runs the
/// same scenarios with the video buffer alone to get their numbers; the default is both.
pub fn audio_enabled() -> bool {
    !matches!(std::env::var("MSE_STRESS_TRACKS").as_deref(), Ok("video"))
}

/// Adds the video source buffer and, unless `MSE_STRESS_TRACKS=video`, the audio one; returns their ids
/// (video first).
pub fn add_buffers(
    rig: &Rig,
    video_mime: &str,
    audio_mime: &str,
) -> Result<Vec<SourceBufferId>, String> {
    let mut ids = vec![
        rig.backend
            .add_source_buffer(video_mime)
            .map_err(|e| format!("add_source_buffer(video): {e:?}"))?,
    ];
    if audio_enabled() {
        ids.push(
            rig.backend
                .add_source_buffer(audio_mime)
                .map_err(|e| format!("add_source_buffer(audio): {e:?}"))?,
        );
    }
    Ok(ids)
}

/// The default fast mode of the brief: decode as fast as possible.
pub fn fast_config(ctx: &Ctx) -> RigConfig {
    RigConfig {
        sync: false,
        audio: audio_enabled(),
        ..ctx.rig_config()
    }
}

/// Real time: the sinks wait for the clock.
pub fn realtime_config(ctx: &Ctx) -> RigConfig {
    RigConfig {
        sync: true,
        audio: audio_enabled(),
        ..ctx.rig_config()
    }
}

pub fn fixture(name: &str) -> Result<SegmentedStream, String> {
    mse_segments::load(name).map_err(|error| format!("fixture {name}: {error}"))
}

pub fn rig_error(what: &str, error: RigError) -> String {
    format!("{what}: {error}")
}

pub fn new_rig(config: RigConfig) -> Result<ManagedRig, String> {
    trace("Rig::new");
    Rig::new(config)
        .map(|rig| ManagedRig { rig: Some(rig) })
        .map_err(|error| rig_error("Rig::new", error))
}

/// Deadlocks found while a scenario ran (a teardown that did not return); `run_scenario` turns them into
/// `Fail` findings. A hang in GstMSE must not cost the whole scenario, or the child process.
static DEADLOCKS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn record_deadlock(text: String) {
    eprintln!("mse-stress: DEADLOCK: {text}");
    DEADLOCKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(text);
}

pub fn take_deadlocks() -> Vec<String> {
    std::mem::take(&mut *DEADLOCKS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// How long a teardown may take before it counts as a deadlock. After the first one has been seen in this
/// process the next ones are expected to hang as well, and [`SHUTDOWN_TIMEOUT_AFTER_A_DEADLOCK`] is used so that
/// a scenario with many rigs finishes.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);
pub const SHUTDOWN_TIMEOUT_AFTER_A_DEADLOCK: Duration = Duration::from_secs(3);

/// A [`Rig`] whose teardown cannot hang the scenario: `shutdown`, `shutdown_with` and `Drop` run the
/// rig's own teardown on a helper thread with a watchdog. If it does not return within
/// [`SHUTDOWN_TIMEOUT`], the deadlock is recorded (the scenario then ends as `Fail`) and the stuck
/// thread is left behind.
pub struct ManagedRig {
    rig: Option<Rig>,
}

impl Deref for ManagedRig {
    type Target = Rig;

    fn deref(&self) -> &Rig {
        self.rig
            .as_ref()
            .expect("the rig is only taken by shutdown or drop")
    }
}

fn guarded_teardown(rig: Rig, order: Option<TeardownOrder>) -> Result<ShutdownInfo, String> {
    let what = match order {
        Some(order) => format!("Rig teardown ({order:?})"),
        None => "Rig::shutdown".to_owned(),
    };
    let limit = if DEADLOCKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty()
    {
        SHUTDOWN_TIMEOUT
    } else {
        SHUTDOWN_TIMEOUT_AFTER_A_DEADLOCK
    };
    trace(&what);
    let outcome = crate::watchdog::call_with_timeout(&what, limit, move || match order {
        Some(order) => rig.shutdown_with(order),
        None => rig.shutdown(),
    });
    match outcome {
        Ok(Ok(info)) => Ok(info),
        Ok(Err(error)) => Err(format!("the pipeline refused to go to NULL: {error}")),
        Err(watch) => {
            let text = format!(
                "{watch}: set_state(NULL) of the pipeline never returned (GstMSE teardown deadlock)"
            );
            record_deadlock(text.clone());
            Err(text)
        }
    }
}

impl ManagedRig {
    pub fn shutdown(mut self) -> Result<ShutdownInfo, String> {
        match self.rig.take() {
            Some(rig) => guarded_teardown(rig, None),
            None => Err("already shut down".to_owned()),
        }
    }

    pub fn shutdown_with(mut self, order: TeardownOrder) -> Result<ShutdownInfo, String> {
        match self.rig.take() {
            Some(rig) => guarded_teardown(rig, Some(order)),
            None => Err("already shut down".to_owned()),
        }
    }
}

impl Drop for ManagedRig {
    fn drop(&mut self) {
        if let Some(rig) = self.rig.take() {
            let _ = guarded_teardown(rig, None);
        }
    }
}

/// What a counting sink saw, in plain numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Snapshot {
    pub count: u64,
    pub bytes: u64,
    pub first_pts_ms: Option<f64>,
    pub last_pts_ms: Option<f64>,
    pub out_of_order: u64,
    pub max_gap_ms: Option<f64>,
    pub caps_changes: usize,
    pub flushes: u64,
}

impl Snapshot {
    pub fn read(stats: &SharedStats) -> Snapshot {
        let stats = stats.lock().unwrap_or_else(|e| e.into_inner());
        let to_ms = |t: Option<mse_rig::ClockTime>| t.map(|t| t.nseconds() as f64 / 1e6);
        Snapshot {
            count: stats.count,
            bytes: stats.bytes,
            first_pts_ms: to_ms(stats.first_pts),
            last_pts_ms: to_ms(stats.last_pts),
            out_of_order: stats.out_of_order,
            max_gap_ms: to_ms(stats.max_gap),
            caps_changes: stats.caps_changes.len(),
            flushes: stats.flushes,
        }
    }

    /// Last minus first timestamp, in milliseconds.
    pub fn span_ms(&self) -> Option<f64> {
        Some(self.last_pts_ms? - self.first_pts_ms?)
    }

    /// Adds `<prefix>_count`, `_bytes`, `_span_ms`, `_out_of_order`, `_max_gap_ms`, `_caps_changes`
    /// and `_flushes` to `result`.
    pub fn record(&self, result: &mut ScenarioResult, prefix: &str) {
        result.metric(&format!("{prefix}_count"), self.count);
        result.metric(&format!("{prefix}_bytes"), self.bytes);
        result.metric(
            &format!("{prefix}_span_ms"),
            crate::stats::round3(self.span_ms()),
        );
        result.metric(&format!("{prefix}_out_of_order"), self.out_of_order);
        result.metric(
            &format!("{prefix}_max_gap_ms"),
            crate::stats::round3(self.max_gap_ms),
        );
        result.metric(&format!("{prefix}_caps_changes"), self.caps_changes);
        result.metric(&format!("{prefix}_flushes"), self.flushes);
    }
}

pub fn video_frames(rig: &Rig) -> u64 {
    rig.video.lock().unwrap_or_else(|e| e.into_inner()).count
}

pub fn audio_buffers(rig: &Rig) -> u64 {
    rig.audio.lock().unwrap_or_else(|e| e.into_inner()).count
}

/// What `play_through` measured.
#[derive(Debug, Clone, Copy, Default)]
pub struct Played {
    pub video: Snapshot,
    pub audio: Snapshot,
    pub reached_expected: bool,
}

/// What a play-through needs: the frames to expect at the video sink (0 when there is no video).
#[derive(Debug, Clone, Copy)]
pub struct Expect {
    pub video_frames: u64,
}

/// Adds one source buffer per stream, appends the initialisation segments, starts playback, appends
/// the media segments round-robin (video, audio, video, audio...), ends the stream and waits for the
/// expected number of video frames (within [`FRAME_TOLERANCE`]) or `wait` to pass.
pub fn play_through(
    rig: &Rig,
    video: Option<&SegmentedStream>,
    audio: Option<&SegmentedStream>,
    expect: Expect,
    wait: Duration,
) -> Result<Played, String> {
    let streams: Vec<&SegmentedStream> = video
        .into_iter()
        .chain(audio.filter(|_| audio_enabled()))
        .collect();
    let mut ids = Vec::new();
    for stream in &streams {
        let id = rig
            .backend
            .add_source_buffer(&stream.mime)
            .map_err(|e| format!("add_source_buffer({}): {e:?}", stream.mime))?;
        ids.push(id);
    }
    for (stream, id) in streams.iter().zip(&ids) {
        rig.append_and_wait(*id, &stream.init.bytes, APPEND_TIMEOUT)
            .map_err(|e| rig_error("append init segment", e))?;
    }
    rig.play().map_err(|e| rig_error("play", e))?;
    let longest = streams.iter().map(|s| s.media.len()).max().unwrap_or(0);
    for index in 0..longest {
        for (stream, id) in streams.iter().zip(&ids) {
            if let Some(segment) = stream.media.get(index) {
                rig.append_and_wait(*id, &segment.bytes, APPEND_TIMEOUT)
                    .map_err(|e| rig_error("append media segment", e))?;
            }
        }
    }
    rig.backend
        .end_of_stream(EosError::None)
        .map_err(|e| format!("end_of_stream: {e:?}"))?;
    let wanted = expect.video_frames.saturating_sub(FRAME_TOLERANCE);
    let reached = rig.wait_until(wait, |rig| video_frames(rig) >= wanted);
    // Let late frames and the audio tail arrive before reading.
    std::thread::sleep(Duration::from_millis(300));
    Ok(Played {
        video: Snapshot::read(&rig.video),
        audio: Snapshot::read(&rig.audio),
        reached_expected: reached || expect.video_frames == 0,
    })
}

/// A progress line on standard error. The harness keeps the last lines of a child's standard error in the
/// notes of a result that has no result line, so after a crash or a hang the notes say where it was.
pub fn trace(message: &str) {
    eprintln!("mse-stress: {message}");
}

/// The short name of an event kind, without its payload: `UpdateEnd`, `ElementReadyState`, ...
pub fn event_kind(event: &MseEvent) -> &'static str {
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
        MseEvent::Play(play) => match play {
            mse_rig::events::PlayEvent::StateChanged(_) => "Play.StateChanged",
            mse_rig::events::PlayEvent::Buffering(_) => "Play.Buffering",
            mse_rig::events::PlayEvent::EndOfStream => "Play.EndOfStream",
            mse_rig::events::PlayEvent::SeekDone(_) => "Play.SeekDone",
            mse_rig::events::PlayEvent::DurationChanged(_) => "Play.DurationChanged",
            mse_rig::events::PlayEvent::PositionUpdated(_) => "Play.PositionUpdated",
            mse_rig::events::PlayEvent::MediaInfoUpdated => "Play.MediaInfoUpdated",
            mse_rig::events::PlayEvent::Error(_) => "Play.Error",
            mse_rig::events::PlayEvent::Warning(_) => "Play.Warning",
        },
    }
}

/// Event kind -> thread name -> how many events of that kind came on that thread.
pub fn thread_table(events: &[TimedEvent]) -> BTreeMap<&'static str, BTreeMap<String, usize>> {
    let mut table: BTreeMap<&'static str, BTreeMap<String, usize>> = BTreeMap::new();
    for timed in events {
        *table
            .entry(event_kind(&timed.event))
            .or_default()
            .entry(timed.thread.clone())
            .or_default() += 1;
    }
    table
}

/// The thread table as JSON (`{"UpdateEnd": {"thread-a": 3}}`).
pub fn thread_table_json(events: &[TimedEvent]) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    for (kind, threads) in thread_table(events) {
        object.insert(kind.to_owned(), json!(threads));
    }
    serde_json::Value::Object(object)
}

/// Checks that, for source buffer `id`, every update is `UpdateStart`, `Update`, `UpdateEnd` in that order
/// (an `Error` or `Abort` may replace `Update`; `UpdateEnd` always closes). Returns the problems found.
pub fn update_order_problems(events: &[TimedEvent], id: SourceBufferId) -> Vec<String> {
    let mut problems = Vec::new();
    // 0 = idle, 1 = after UpdateStart, 2 = after Update/Error/Abort.
    let mut state = 0;
    for timed in events {
        let (name, matches) = match &timed.event {
            MseEvent::UpdateStart(other) => ("UpdateStart", *other == id),
            MseEvent::Update(other) => ("Update", *other == id),
            MseEvent::Error(other) => ("Error", *other == id),
            MseEvent::Abort(other) => ("Abort", *other == id),
            MseEvent::UpdateEnd(other) => ("UpdateEnd", *other == id),
            _ => continue,
        };
        if !matches {
            continue;
        }
        let ok = match (state, name) {
            (0, "UpdateStart") => {
                state = 1;
                true
            }
            (1, "Update" | "Error" | "Abort") => {
                state = 2;
                true
            }
            (1 | 2, "UpdateEnd") => {
                state = 0;
                true
            }
            _ => false,
        };
        if !ok {
            problems.push(format!(
                "source buffer {id}: {name} at {:.1} ms is out of order (state {state})",
                timed.at.as_secs_f64() * 1000.0
            ));
        }
    }
    problems
}

/// The most updates of source buffer `id` that were running at the same time in `events` (an update runs from
/// `UpdateStart` to `UpdateEnd`, `Abort` or `Error`): the spec allows one, so 2 means an append was
/// accepted while another was still updating.
pub fn max_update_overlap(events: &[TimedEvent], id: SourceBufferId) -> usize {
    let mut running = 0usize;
    let mut most = 0usize;
    for timed in events {
        match &timed.event {
            MseEvent::UpdateStart(other) if *other == id => {
                running += 1;
                most = most.max(running);
            }
            MseEvent::UpdateEnd(other) if *other == id => running = running.saturating_sub(1),
            _ => {}
        }
    }
    most
}

/// Polls `buffered` of `id` until its last range ends at or after `end_s` (less 50 ms) or `timeout` passes.
/// GstMSE fires `UpdateEnd` before the appended data is in `buffered` (T05, `settle`): any caller that
/// needs to see the data (a `remove`, a range check) has to wait like this. Returns whether it got there.
pub fn wait_buffered_end(rig: &Rig, id: SourceBufferId, end_s: f64, timeout: Duration) -> bool {
    rig.wait_until(timeout, |rig| {
        rig.backend
            .buffered(id)
            .ok()
            .and_then(|ranges| ranges.last().map(|r| r.end.nseconds() as f64 / 1e9))
            .is_some_and(|end| end + 0.05 >= end_s)
    })
}

/// Polls `buffered` of `id` until it has not changed for `quiet` (at most `max`); returns the ranges as
/// (start, end) in seconds.
pub fn settled_ranges(
    rig: &Rig,
    id: SourceBufferId,
    quiet: Duration,
    max: Duration,
) -> Vec<(f64, f64)> {
    let read = || -> Vec<(f64, f64)> {
        rig.backend
            .buffered(id)
            .map(|ranges| {
                ranges
                    .iter()
                    .map(|r| {
                        (
                            r.start.nseconds() as f64 / 1e9,
                            r.end.nseconds() as f64 / 1e9,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let started = std::time::Instant::now();
    let mut last = read();
    let mut stable_since = std::time::Instant::now();
    while started.elapsed() < max && stable_since.elapsed() < quiet {
        std::thread::sleep(Duration::from_millis(5));
        let now = read();
        if now != last {
            last = now;
            stable_since = std::time::Instant::now();
        }
    }
    last
}

/// `video count within the C02 tolerance of the expected frames`.
pub fn frames_ok(count: u64, expected: u64) -> bool {
    count + FRAME_TOLERANCE >= expected && count <= expected + FRAME_TOLERANCE
}

/// A short text for notes: `12.3 MB/s`.
pub fn mb_per_s(bytes: usize, took: Duration) -> f64 {
    if took.is_zero() {
        return f64::INFINITY;
    }
    bytes as f64 / 1e6 / took.as_secs_f64()
}

/// What a scenario body is: the same signature `run_scenario` takes.
pub type Body = fn(&Ctx, &mut ScenarioResult) -> Result<Verdict, String>;

/// Runs `body` in a nested run of this executable (the variable `nested_variable` tells the nested run to
/// do the work itself), once more if the process died, and merges the result. GstMSE can crash a process
/// some time after a rig was shut down (T04), which would cost a scenario all its numbers; a scenario that
/// uses this loses them only when it dies on both attempts, and says so. `budget` bounds the two attempts
/// together (the harness has a limit on the whole scenario).
pub fn run_isolated(
    id: &str,
    title: &str,
    ctx: &Ctx,
    nested_variable: &str,
    budget: Duration,
    body: Body,
) -> ScenarioResult {
    use crate::sub::{SubOutcome, SubSpec, crash_summaries_since, part, run_all};
    if part(nested_variable).is_some() {
        return run_scenario(id, title, ctx, body);
    }
    let nested_variable = nested_variable.to_owned();
    let id_owned = id.to_owned();
    run_scenario(id, title, ctx, move |ctx, result| {
        let began = std::time::Instant::now();
        let since = std::time::SystemTime::now();
        let mut crashes: Vec<String> = Vec::new();
        for attempt in 1..=2 {
            let left = budget.saturating_sub(began.elapsed());
            if attempt == 2 && left < Duration::from_secs(20) {
                break;
            }
            let spec = SubSpec {
                id: id_owned.clone(),
                tag: format!("isolated-{attempt}"),
                env: vec![(nested_variable.clone(), "1".to_owned())],
                iterations: Some(ctx.iterations),
                seed: Some(ctx.seed),
                deadline: left,
            };
            let outcome = run_all(ctx, &[spec], 1, left).into_iter().next();
            match outcome {
                Some(SubOutcome::Report {
                    result: nested,
                    exit,
                }) => {
                    let panicked = nested
                        .notes
                        .iter()
                        .any(|n| n.to_lowercase().contains("panic"));
                    if nested.status == Status::Error && !panicked {
                        crashes.push(format!(
                            "attempt {attempt} died ({exit}): {}",
                            nested.notes.join(" | ")
                        ));
                        continue;
                    }
                    if nested.status == Status::Error {
                        return Err(nested.notes.join(" | "));
                    }
                    for (key, value) in &nested.metrics {
                        if !matches!(
                            key.as_str(),
                            "shape" | "plugin_mode" | "glib_main_loop" | "tracks"
                        ) {
                            result.metric(key, value.clone());
                        }
                    }
                    let mut reasons = Vec::new();
                    for note in &nested.notes {
                        if let Some(reason) = note.strip_prefix("FAIL: ") {
                            reasons.push(reason.to_owned());
                        } else if !note.starts_with("ran with ") {
                            result.note(note.clone());
                        }
                    }
                    if !crashes.is_empty() {
                        result.metric("crashed_attempts", serde_json::json!(crashes));
                        for crash in &crashes {
                            result.note(format!("an earlier attempt of this scenario died (GstMSE teardown crash, see T04): {crash}"));
                        }
                        for summary in crash_summaries_since(since) {
                            result.note(format!("macOS {summary}"));
                        }
                    }
                    return Ok(match nested.status {
                        Status::Pass => Verdict::Pass,
                        Status::Skip => {
                            Verdict::Skip(nested.notes.last().cloned().unwrap_or_default())
                        }
                        _ => Verdict::Fail(reasons),
                    });
                }
                Some(other) => {
                    let hung = matches!(other, SubOutcome::TimedOut { .. });
                    crashes.push(format!("attempt {attempt}: {}", other.describe()));
                    if hung {
                        // A hang is not a crash: the same hang would eat the second attempt too.
                        break;
                    }
                }
                None => crashes.push(format!("attempt {attempt}: nothing ran")),
            }
        }
        result.metric("crashed_attempts", serde_json::json!(crashes));
        let mut reasons = vec![format!(
            "the scenario's process died on every attempt (a GstMSE crash, see T04): {}",
            crashes.join("; ")
        )];
        for summary in crash_summaries_since(since) {
            reasons.push(format!("macOS {summary}"));
        }
        Ok(Verdict::Fail(reasons))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(at_ms: u64, event: MseEvent) -> TimedEvent {
        TimedEvent {
            at: Duration::from_millis(at_ms),
            thread: "t".to_owned(),
            event,
        }
    }

    #[test]
    fn a_normal_update_has_no_problems() {
        let events = vec![
            event(1, MseEvent::UpdateStart(0)),
            event(2, MseEvent::UpdateStart(1)),
            event(3, MseEvent::Update(0)),
            event(4, MseEvent::UpdateEnd(0)),
            event(5, MseEvent::Update(1)),
            event(6, MseEvent::UpdateEnd(1)),
        ];
        assert!(update_order_problems(&events, 0).is_empty());
        assert!(update_order_problems(&events, 1).is_empty());
    }

    #[test]
    fn a_missing_update_start_is_reported() {
        let events = vec![
            event(1, MseEvent::Update(0)),
            event(2, MseEvent::UpdateEnd(0)),
        ];
        let problems = update_order_problems(&events, 0);
        assert!(!problems.is_empty(), "{problems:?}");
        assert!(problems[0].contains("Update"));
    }

    #[test]
    fn an_error_may_replace_the_update_event() {
        let events = vec![
            event(1, MseEvent::UpdateStart(2)),
            event(2, MseEvent::Error(2)),
            event(3, MseEvent::UpdateEnd(2)),
            event(4, MseEvent::UpdateStart(2)),
            event(5, MseEvent::Abort(2)),
            event(6, MseEvent::UpdateEnd(2)),
        ];
        assert!(update_order_problems(&events, 2).is_empty());
    }

    #[test]
    fn two_update_starts_in_a_row_are_reported() {
        let events = vec![
            event(1, MseEvent::UpdateStart(0)),
            event(2, MseEvent::UpdateStart(0)),
        ];
        assert_eq!(update_order_problems(&events, 0).len(), 1);
    }

    #[test]
    fn the_thread_table_groups_by_kind_and_thread() {
        let mut other = event(2, MseEvent::UpdateEnd(1));
        other.thread = "u".to_owned();
        let events = vec![
            event(1, MseEvent::UpdateEnd(0)),
            other,
            event(3, MseEvent::SourceOpen),
        ];
        let table = thread_table(&events);
        assert_eq!(table["UpdateEnd"]["t"], 1);
        assert_eq!(table["UpdateEnd"]["u"], 1);
        assert_eq!(table["SourceOpen"]["t"], 1);
        let json = thread_table_json(&events);
        assert_eq!(json["UpdateEnd"]["u"], 1);
    }

    #[test]
    fn every_event_has_a_kind_name() {
        assert_eq!(event_kind(&MseEvent::SourceOpen), "SourceOpen");
        assert_eq!(
            event_kind(&MseEvent::Play(mse_rig::events::PlayEvent::EndOfStream)),
            "Play.EndOfStream"
        );
        assert_eq!(
            event_kind(&MseEvent::ElementDuration(None)),
            "ElementDuration"
        );
    }

    #[test]
    fn overlapping_updates_are_counted() {
        let serial = vec![
            event(1, MseEvent::UpdateStart(0)),
            event(2, MseEvent::UpdateEnd(0)),
            event(3, MseEvent::UpdateStart(0)),
            event(4, MseEvent::UpdateEnd(0)),
        ];
        assert_eq!(max_update_overlap(&serial, 0), 1);
        let overlapping = vec![
            event(1, MseEvent::UpdateStart(0)),
            event(2, MseEvent::UpdateStart(0)),
            event(3, MseEvent::UpdateStart(1)),
            event(4, MseEvent::UpdateEnd(0)),
            event(5, MseEvent::UpdateEnd(0)),
        ];
        assert_eq!(max_update_overlap(&overlapping, 0), 2);
        assert_eq!(max_update_overlap(&overlapping, 1), 1);
        assert_eq!(max_update_overlap(&[], 0), 0);
    }

    #[test]
    fn the_frame_tolerance_is_two_either_way() {
        assert!(frames_ok(200, 200));
        assert!(frames_ok(198, 200));
        assert!(frames_ok(202, 200));
        assert!(!frames_ok(197, 200));
        assert!(!frames_ok(203, 200));
    }

    #[test]
    fn findings_become_a_verdict() {
        let mut findings = Findings::new();
        findings.check(true, "never shown");
        assert!(findings.is_clean());
        assert_eq!(findings.verdict(), Verdict::Pass);
        let mut findings = Findings::new();
        findings.check(false, "boom");
        findings.fail("bang");
        assert_eq!(findings.reasons().len(), 2);
        assert_eq!(
            findings.verdict(),
            Verdict::Fail(vec!["boom".into(), "bang".into()])
        );
    }

    #[test]
    fn mb_per_s_is_bytes_over_time() {
        assert_eq!(mb_per_s(10_000_000, Duration::from_secs(1)), 10.0);
        assert!(mb_per_s(1, Duration::ZERO).is_infinite());
    }

    #[test]
    fn snapshots_report_their_span() {
        let snapshot = Snapshot {
            first_pts_ms: Some(10.0),
            last_pts_ms: Some(7990.0),
            ..Snapshot::default()
        };
        assert_eq!(snapshot.span_ms(), Some(7980.0));
        assert_eq!(Snapshot::default().span_ms(), None);
    }
}
