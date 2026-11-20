// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Helpers shared by the conformance scenarios: the scenario recorder (`Scn`), the three ways a
//! scenario can stop (`Abort`), how to read an `EventLog`, colour classification of the red/blue
//! fixtures, range and time formatting, feeding segments to source buffers, and a few small tools
//! (a watcher thread, a deterministic byte generator).
//!
//! The rules the helpers enforce (the brief's section 6 and 8):
//! * `Fail` means "the scenario ran correctly and GstMSE did the wrong thing"; every refusal, timeout
//!   or odd state that comes from GstMSE or the pipeline turns into `Abort::Fail` with the text of what
//!   happened (events, times, counts, error text). Nothing from GstMSE is swallowed.
//! * `Error` means the harness itself broke (set-up failure, missing P0 fixture, a splitter error).
//! * A check is never weakened to get a `Pass`: tolerances here are the ones the scenario table names
//!   (one frame for video edges, one audio frame for audio edges) and each is written down where it is used.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mse_rig::ClockTime;
use mse_rig::RigError;
use mse_rig::backend::{MseError, TimeRange};
use mse_rig::env::PluginMode;
use mse_rig::events::{
    ElementReadyState, EventLog, MseEvent, PlayEvent, SourceBufferId, TimedEvent,
};
use mse_rig::frames::{ColourSample, FrameStats, SharedStats};
use mse_rig::harness::Ctx;
use mse_rig::report::{ScenarioResult, Status};
use mse_rig::rig::{Rig, RigConfig, Shape};
use mse_segments::{ChunkPlan, Segment, SegmentedStream, SplitError};

// ---------------------------------------------------------------------------------------------
// Fixture facts (brief section 5): 8 s, 25 fps, keyframe every 2 s, segments of 2 s, red then blue.
// ---------------------------------------------------------------------------------------------

/// The nominal length of every fixture, in milliseconds.
pub const NOMINAL_MS: i64 = 8_000;
/// 25 fps for 8 s.
pub const NOMINAL_FRAMES: u64 = 200;
/// The picture turns from red to blue here (frame 100).
pub const FLIP_MS: u64 = 4_000;
/// One video frame, in milliseconds.
pub const FRAME_MS: i64 = 40;
/// Tolerance of a video range edge: removal and appends are on exact frame times.
pub const VIDEO_TOL_MS: i64 = 2;
/// Tolerance of the "whole stream is buffered" checks (C02..C05): more than a frame would be a bug,
/// two frames of slack allow for either convention of "end = last timestamp" or "end = last + duration".
pub const LOOSE_TOL_MS: i64 = 100;
/// How long one append may take to reach `UpdateEnd`.
pub const APPEND_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------------------------
// How a scenario stops
// ---------------------------------------------------------------------------------------------

/// Why a scenario function stopped before its end. `?` converts the rig's errors with the mapping
/// the brief asks for: a GstMSE refusal, timeout or pipeline failure is a finding (`Fail`); only a
/// set-up failure or a splitter problem is a harness `Error`.
#[derive(Debug)]
pub enum Abort {
    /// GstMSE (or the pipeline around it) did the wrong thing; the text says exactly what.
    Fail(String),
    /// The harness broke.
    Error(String),
    /// The scenario cannot run here (a missing optional fixture or plugin); the text says why.
    Skip(String),
}

pub type Res<T> = Result<T, Abort>;

impl Abort {
    /// Prefixes the message with what the scenario was doing.
    pub fn context(self, what: &str) -> Abort {
        match self {
            Abort::Fail(m) => Abort::Fail(format!("{what}: {m}")),
            Abort::Error(m) => Abort::Error(format!("{what}: {m}")),
            Abort::Skip(m) => Abort::Skip(format!("{what}: {m}")),
        }
    }
}

impl From<RigError> for Abort {
    fn from(error: RigError) -> Abort {
        match error {
            RigError::Setup(message) => Abort::Error(format!("rig set-up failed: {message}")),
            RigError::Mse(error) => Abort::Fail(format!("GstMSE refused the call: {error:?}")),
            RigError::Pipeline(message) => Abort::Fail(format!("pipeline: {message}")),
            RigError::Timeout(after, what) => {
                Abort::Fail(format!("timed out after {after:?} waiting for {what}"))
            }
        }
    }
}

impl From<MseError> for Abort {
    fn from(error: MseError) -> Abort {
        Abort::Fail(format!("GstMSE refused the call: {error:?}"))
    }
}

impl From<SplitError> for Abort {
    fn from(error: SplitError) -> Abort {
        Abort::Error(format!("segment splitter: {error}"))
    }
}

/// Reads a committed fixture; failing to load a P0/P1 fixture is a harness problem (`Error`).
pub fn fixture(name: &str) -> Res<SegmentedStream> {
    mse_segments::load(name).map_err(|error| Abort::Error(format!("fixture {name:?}: {error}")))
}

// ---------------------------------------------------------------------------------------------
// The recorder
// ---------------------------------------------------------------------------------------------

/// A scenario's result under construction: metrics, notes, and the list of failed checks.
pub struct Scn {
    result: ScenarioResult,
    failures: Vec<String>,
    started: Instant,
    rig_notes: BTreeSet<String>,
}

pub fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Bare => "bare",
        Shape::PlayBin => "playbin",
    }
}

pub fn plugin_mode_name(mode: &PluginMode) -> &'static str {
    match mode {
        PluginMode::SystemPath => "system",
        PluginMode::ServoStyle { .. } => "servo",
    }
}

impl Scn {
    /// Starts a result and records which shape, plugin mode and main-loop setting this run used.
    pub fn new(ctx: &Ctx, id: &str, title: &str) -> Scn {
        let mut result = ScenarioResult::new(id, title);
        result
            .metric("shape", shape_name(ctx.shape))
            .metric("plugin_mode", plugin_mode_name(&ctx.plugin_mode))
            .metric("glib_main_loop", ctx.glib_main_loop)
            .metric("gst_version", ctx.info.version.clone())
            .metric("arch", ctx.info.arch);
        result.note(format!(
            "ran with --shape {} --plugin-mode {} --glib-main-loop {} on GStreamer {} ({})",
            shape_name(ctx.shape),
            plugin_mode_name(&ctx.plugin_mode),
            if ctx.glib_main_loop { "on" } else { "off" },
            ctx.info.version,
            ctx.info.arch
        ));
        if !ctx.info.failed_plugins.is_empty() {
            result.note(format!(
                "plugins that failed to load in this mode: {:?}",
                ctx.info.failed_plugins
            ));
        }
        Scn {
            result,
            failures: Vec::new(),
            started: Instant::now(),
            rig_notes: BTreeSet::new(),
        }
    }

    pub fn metric(&mut self, name: &str, value: impl Into<serde_json::Value>) -> &mut Self {
        self.result.metric(name, value);
        self
    }

    pub fn note(&mut self, line: impl Into<String>) -> &mut Self {
        self.result.note(line);
        self
    }

    /// Records a note once per distinct text (used for per-rig set-up lines).
    pub fn note_once(&mut self, line: impl Into<String>) {
        let line = line.into();
        if self.rig_notes.insert(line.clone()) {
            self.result.note(line);
        }
    }

    /// Records one check as the metric `check.<name>`; a failed one also becomes a `FAIL` note and
    /// makes the scenario `Fail`. `detail` says what was measured.
    pub fn check(&mut self, name: &str, ok: bool, detail: impl Into<String>) -> bool {
        let detail = detail.into();
        self.result.metric(&format!("check.{name}"), ok);
        if !ok {
            self.result.note(format!("FAIL {name}: {detail}"));
            self.failures.push(name.to_owned());
        }
        ok
    }

    /// Records a finding that is not tied to a check (the scenario becomes `Fail`).
    pub fn finding(&mut self, line: impl Into<String>) {
        let line = line.into();
        self.result.note(format!("FINDING {line}"));
        self.failures.push(line);
    }

    #[cfg(test)]
    pub fn has_failed(&self) -> bool {
        !self.failures.is_empty()
    }

    /// A recorder that needs no `Ctx`, for unit tests of the helpers.
    #[cfg(test)]
    pub fn for_tests() -> Scn {
        Scn {
            result: ScenarioResult::new("X00", "test"),
            failures: Vec::new(),
            started: Instant::now(),
            rig_notes: BTreeSet::new(),
        }
    }

    /// Closes the result. `Ok` is `Pass` unless a check failed; an `Abort` decides the status itself.
    pub fn finish(self, outcome: Res<()>) -> ScenarioResult {
        let Scn {
            mut result,
            failures,
            started,
            ..
        } = self;
        result.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        result.status = match outcome {
            Ok(()) if failures.is_empty() => Status::Pass,
            Ok(()) => Status::Fail,
            Err(Abort::Fail(message)) => {
                result.note(format!("STOPPED by a GstMSE failure: {message}"));
                Status::Fail
            }
            Err(Abort::Error(message)) => {
                result.note(format!("HARNESS ERROR: {message}"));
                Status::Error
            }
            Err(Abort::Skip(message)) => {
                result.note(format!("SKIPPED: {message}"));
                if failures.is_empty() {
                    Status::Skip
                } else {
                    Status::Fail
                }
            }
        };
        result
    }
}

/// Runs `body` and builds the `ScenarioResult`; scenarios that need a rig are `Skip` when the `mse`
/// plugin is not loaded in this plugin mode (that is itself a finding for EA0, written in the note).
pub fn run_scenario(
    ctx: &Ctx,
    id: &str,
    title: &str,
    needs_msesrc: bool,
    body: impl FnOnce(&Ctx, &mut Scn) -> Res<()>,
) -> ScenarioResult {
    let mut scn = Scn::new(ctx, id, title);
    if needs_msesrc && !ctx.info.msesrc_available {
        let reason = format!(
            "msesrc is not available in plugin mode {}: libgstmse.dylib is missing from the plugin list \
             (EA0 must add `mse` to Mercury's GStreamer plugin list)",
            plugin_mode_name(&ctx.plugin_mode)
        );
        return scn.finish(Err(Abort::Skip(reason)));
    }
    eprintln!("[{id}] start");
    let outcome = body(ctx, &mut scn);
    eprintln!("[{id}] done");
    scn.finish(outcome)
}

// ---------------------------------------------------------------------------------------------
// Time and range formatting
// ---------------------------------------------------------------------------------------------

pub fn t_ms(milliseconds: u64) -> ClockTime {
    ClockTime::from_mseconds(milliseconds)
}

/// Nanoseconds to whole milliseconds, signed (so that differences can go negative).
pub fn ms_of(time: ClockTime) -> i64 {
    i64::try_from(time.nseconds() / 1_000_000).unwrap_or(i64::MAX)
}

pub fn secs(time: ClockTime) -> f64 {
    time.nseconds() as f64 / 1e9
}

pub fn fmt_t(time: ClockTime) -> String {
    format!("{:.3}s", secs(time))
}

pub fn fmt_opt(time: Option<ClockTime>) -> String {
    match time {
        Some(time) => fmt_t(time),
        None => "none".to_owned(),
    }
}

/// `Some(ms)` as `"123 ms"`, `None` as `"never"`: for the numbers of "how long until the first frame".
pub fn fmt_ms(value: Option<i64>) -> String {
    value.map_or_else(
        || "never (no frame arrived in time)".to_owned(),
        |ms| format!("{ms} ms"),
    )
}

pub fn fmt_ranges(ranges: &[TimeRange]) -> String {
    if ranges.is_empty() {
        return "[]".to_owned();
    }
    ranges
        .iter()
        .map(|range| format!("[{:.3}-{:.3}]", secs(range.start), secs(range.end)))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn ranges_json(ranges: &[TimeRange]) -> serde_json::Value {
    serde_json::Value::Array(
        ranges
            .iter()
            .map(|range| serde_json::json!([ms_of(range.start), ms_of(range.end)]))
            .collect(),
    )
}

pub fn dur_ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 10_000.0).round() / 10.0
}

pub fn ms_between(from: Instant, to: Instant) -> i64 {
    if to >= from {
        i64::try_from((to - from).as_millis()).unwrap_or(i64::MAX)
    } else {
        -i64::try_from((from - to).as_millis()).unwrap_or(i64::MAX)
    }
}

/// Compares `got` with the `expected` ranges (start and end in milliseconds) within `tol_ms`; the
/// returned lines are the deviations, empty when the ranges match.
pub fn compare_ranges(
    label: &str,
    got: &[TimeRange],
    expected: &[(i64, i64)],
    tol_ms: i64,
) -> Vec<String> {
    let mut deviations = Vec::new();
    if got.len() != expected.len() {
        deviations.push(format!(
            "{label}: got {} range(s) {} but the spec gives {} range(s) {:?}",
            got.len(),
            fmt_ranges(got),
            expected.len(),
            expected
                .iter()
                .map(|(start, end)| format!(
                    "[{:.3}-{:.3}]",
                    *start as f64 / 1e3,
                    *end as f64 / 1e3
                ))
                .collect::<Vec<_>>()
        ));
        return deviations;
    }
    for (index, (range, (start, end))) in got.iter().zip(expected).enumerate() {
        for (edge, value, wanted) in [
            ("start", ms_of(range.start), *start),
            ("end", ms_of(range.end), *end),
        ] {
            let delta = value - wanted;
            if delta.abs() > tol_ms {
                deviations.push(format!(
                    "{label}: range {index} {edge} is {value} ms, the spec gives {wanted} ms ({delta:+} ms, tolerance {tol_ms} ms)"
                ));
            }
        }
    }
    deviations
}

// ---------------------------------------------------------------------------------------------
// Colours: the fixtures are solid red for the first 4 s and solid blue for the last 4 s
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Red,
    Blue,
    Other,
}

/// Classifies the mean colour of a frame. A black ball moves over the solid background, which lowers
/// the mean a little, and codecs shift the primaries, so "red" means red dominates by a clear margin.
pub fn classify(rgb: [u8; 3]) -> Colour {
    let [red, green, blue] = rgb.map(i32::from);
    if red > blue + 80 && red > green + 80 {
        Colour::Red
    } else if blue > red + 80 && blue > green + 80 {
        Colour::Blue
    } else {
        Colour::Other
    }
}

/// The colour a frame of the unshifted fixture has at media time `pts`.
pub fn nominal_colour(pts: ClockTime) -> Colour {
    if ms_of(pts) < i64::try_from(FLIP_MS).unwrap_or(i64::MAX) {
        Colour::Red
    } else {
        Colour::Blue
    }
}

/// The nominal colour of a fixture that was moved by `offset_ms` (timestampOffset, shift_media_time).
pub fn shifted_colour(offset_ms: i64) -> impl Fn(ClockTime) -> Colour {
    move |pts| {
        nominal_colour(t_ms(
            u64::try_from((ms_of(pts) - offset_ms).max(0)).unwrap_or(0),
        ))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ColourVerdict {
    pub samples: usize,
    pub red: usize,
    pub blue: usize,
    pub other: usize,
    /// One line per sample whose colour is wrong.
    pub wrong: Vec<String>,
}

impl ColourVerdict {
    pub fn ok(&self) -> bool {
        self.wrong.is_empty()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} samples: {} red, {} blue, {} other, {} wrong",
            self.samples,
            self.red,
            self.blue,
            self.other,
            self.wrong.len()
        )
    }
}

/// Judges every sample against `expected(pts)`. A sample within one frame of a flip may be either side
/// (`slack` is that frame): the frame boundary is not where a player could tell the difference.
pub fn judge_colours(
    samples: &[ColourSample],
    expected: impl Fn(ClockTime) -> Colour,
    slack: ClockTime,
) -> ColourVerdict {
    let mut verdict = ColourVerdict {
        samples: samples.len(),
        ..ColourVerdict::default()
    };
    for sample in samples {
        let seen = classify(sample.rgb);
        match seen {
            Colour::Red => verdict.red += 1,
            Colour::Blue => verdict.blue += 1,
            Colour::Other => verdict.other += 1,
        }
        let before = expected(sample.pts.saturating_sub(slack));
        let after = expected(sample.pts + slack);
        if seen != before && seen != after {
            verdict.wrong.push(format!(
                "frame at {} is {:?} (mean rgb {:?}), expected {:?}",
                fmt_t(sample.pts),
                seen,
                sample.rgb,
                expected(sample.pts)
            ));
        }
    }
    verdict
}

// ---------------------------------------------------------------------------------------------
// Reading the event log
// ---------------------------------------------------------------------------------------------

pub fn update_ends(log: &EventLog, id: SourceBufferId) -> usize {
    log.count(|event| matches!(event, MseEvent::UpdateEnd(i) if *i == id))
}

pub fn eos_seen(log: &EventLog) -> bool {
    log.count(|event| matches!(event, MseEvent::Play(PlayEvent::EndOfStream))) > 0
}

pub fn seek_dones(log: &EventLog) -> usize {
    log.count(|event| matches!(event, MseEvent::Play(PlayEvent::SeekDone(_))))
}

/// Every `Error`/`Abort` of a source buffer and every GstPlay error, as readable lines.
pub fn errors_in(events: &[TimedEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|timed| {
            let what = match &timed.event {
                MseEvent::Error(id) => format!("source buffer {id} Error event"),
                MseEvent::Abort(id) => format!("source buffer {id} Abort event"),
                MseEvent::Play(PlayEvent::Error(message)) => format!("GstPlay error: {message}"),
                _ => return None,
            };
            Some(format!(
                "+{} ms [{}] {what}",
                timed.at.as_millis(),
                timed.thread
            ))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdatePhase {
    Idle,
    Started,
    Updated,
    Terminated,
}

/// Checks that each listed source buffer's events follow `UpdateStart -> Update -> UpdateEnd`,
/// cycle after cycle, and that no cycle is left open. `Error` and `Abort` events are problems unless
/// `allow_error_abort`, in which case `UpdateStart -> (Error|Abort) -> UpdateEnd` is also a cycle.
pub fn event_order_problems(
    events: &[TimedEvent],
    ids: &[SourceBufferId],
    allow_error_abort: bool,
) -> Vec<String> {
    let mut problems = Vec::new();
    for &id in ids {
        let mut phase = UpdatePhase::Idle;
        let mut cycles = 0usize;
        for timed in events {
            let name = match &timed.event {
                MseEvent::UpdateStart(i) if *i == id => "UpdateStart",
                MseEvent::Update(i) if *i == id => "Update",
                MseEvent::UpdateEnd(i) if *i == id => "UpdateEnd",
                MseEvent::Abort(i) if *i == id => "Abort",
                MseEvent::Error(i) if *i == id => "Error",
                _ => continue,
            };
            let next = match (phase, name) {
                (UpdatePhase::Idle, "UpdateStart") => Some(UpdatePhase::Started),
                (UpdatePhase::Started, "Update") => Some(UpdatePhase::Updated),
                (UpdatePhase::Updated, "UpdateEnd") => {
                    cycles += 1;
                    Some(UpdatePhase::Idle)
                }
                (UpdatePhase::Started, "Abort" | "Error") if allow_error_abort => {
                    Some(UpdatePhase::Terminated)
                }
                (UpdatePhase::Terminated, "UpdateEnd") => {
                    cycles += 1;
                    Some(UpdatePhase::Idle)
                }
                _ => None,
            };
            match next {
                Some(next) => phase = next,
                None => {
                    problems.push(format!(
                        "source buffer {id}: {name} at +{} ms [{}] arrived while the append was {phase:?} (cycle {cycles})",
                        timed.at.as_millis(),
                        timed.thread
                    ));
                    break;
                }
            }
        }
        if problems.is_empty() && phase != UpdatePhase::Idle {
            problems.push(format!(
                "source buffer {id}: the last append is still {phase:?} (no UpdateEnd) after {cycles} complete cycle(s)"
            ));
        }
    }
    problems
}

/// The distinct thread names that delivered events matching `predicate`.
pub fn threads_of(
    events: &[TimedEvent],
    predicate: impl Fn(&MseEvent) -> bool,
) -> BTreeSet<String> {
    events
        .iter()
        .filter(|timed| predicate(&timed.event))
        .map(|timed| timed.thread.clone())
        .collect()
}

pub fn ready_state_events(
    events: &[TimedEvent],
    from: usize,
) -> Vec<(Duration, ElementReadyState)> {
    events
        .iter()
        .skip(from)
        .filter_map(|timed| match &timed.event {
            MseEvent::ElementReadyState(state) => Some((timed.at, *state)),
            _ => None,
        })
        .collect()
}

pub fn fmt_ready_states(states: &[(Duration, ElementReadyState)]) -> String {
    if states.is_empty() {
        return "none".to_owned();
    }
    states
        .iter()
        .map(|(at, state)| format!("{state:?}@{}ms", at.as_millis()))
        .collect::<Vec<_>>()
        .join(" -> ")
}

// ---------------------------------------------------------------------------------------------
// The rig: configuration, state snapshots, diagnostics
// ---------------------------------------------------------------------------------------------

/// Fast mode: sinks do not synchronise to the clock (decode as fast as possible).
pub fn fast_config(ctx: &Ctx) -> RigConfig {
    RigConfig {
        sync: false,
        ..ctx.rig_config()
    }
}

/// Real-time mode, for the scenarios about timing (at most 12 s of media).
pub fn timed_config(ctx: &Ctx) -> RigConfig {
    RigConfig {
        sync: true,
        ..ctx.rig_config()
    }
}

pub fn new_rig(scn: &mut Scn, config: RigConfig) -> Res<Rig> {
    scn.note_once(format!(
        "rig: sync={} video={} audio={} prefer_decoder={:?}",
        config.sync, config.video, config.audio, config.prefer_decoder
    ));
    Ok(Rig::new(config)?)
}

pub fn vstats(rig: &Rig) -> FrameStats {
    rig.video
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub fn astats(rig: &Rig) -> FrameStats {
    rig.audio
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub fn reset_stats(rig: &Rig) {
    FrameStats::reset(&rig.video);
    FrameStats::reset(&rig.audio);
}

pub fn video_count(rig: &Rig) -> u64 {
    rig.video
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .count
}

/// Media time covered by an audio sink: first to last timestamp (the last buffer's own length is not in it).
pub fn span_ms(stats: &FrameStats) -> i64 {
    match (stats.first_pts, stats.last_pts) {
        (Some(first), Some(last)) => ms_of(last) - ms_of(first),
        _ => 0,
    }
}

/// Runs `f` on a helper thread and waits at most `timeout` for its answer: a call into GstMSE that blocks on a
/// lock a deadlocked thread holds then costs the timeout and not the scenario. A thread that never returns is
/// left behind (it holds nothing of ours).
pub fn call_with_timeout<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("conformance-call".to_owned())
        .spawn(move || {
            let _ = sender.send(f());
        })
        .ok()?;
    receiver.recv_timeout(timeout).ok()
}

/// Waits up to `timeout` for more than `before` `UpdateEnd` events of source buffer `id` (`before` is
/// `update_ends` read before the operation was started).
pub fn wait_update_end(rig: &Rig, id: SourceBufferId, before: usize, timeout: Duration) -> bool {
    rig.log.wait_for(timeout, |events| {
        events
            .iter()
            .filter(|timed| matches!(timed.event, MseEvent::UpdateEnd(i) if i == id))
            .count()
            > before
    })
}

/// Runs a call on the rig's backend on a helper thread and gives up after `timeout`: GstMSE 1.28.9 can block a call
/// for ever after some misuse (an append in the ended state), and one blocked call must not take the scenario
/// (and its other results) with it. `None` means the call did not return in time.
pub fn guarded<T: Send + 'static>(
    rig: &Rig,
    timeout: Duration,
    call: impl FnOnce(mse_rig::backend::MseBackend) -> T + Send + 'static,
) -> Option<T> {
    let backend = rig.backend.clone();
    call_with_timeout(timeout, move || call(backend))
}

/// `(n-video, n-audio)` of the `msesrc`: how many tracks GstMSE has exposed as pads.
pub fn track_counts(rig: &Rig) -> Option<(u32, u32)> {
    let element = rig.msesrc()?;
    call_with_timeout(Duration::from_secs(2), move || {
        use gstreamer::prelude::ObjectExt;
        (
            element.property::<u32>("n-video"),
            element.property::<u32>("n-audio"),
        )
    })
}

/// What `wait_tracks` saw.
#[derive(Debug, Clone, Copy)]
pub struct TrackWait {
    /// The element has exactly the expected number of video and audio tracks and is at least `HaveMetadata`.
    pub reached: bool,
    pub counts: Option<(u32, u32)>,
    pub ready: ElementReadyState,
    pub waited: Duration,
}

/// Waits up to `timeout` for the `msesrc` to expose `video` video and `audio` audio tracks.
pub fn wait_tracks(rig: &Rig, video: u32, audio: u32, timeout: Duration) -> TrackWait {
    let started = Instant::now();
    loop {
        let counts = track_counts(rig);
        let ready = rig.element_ready_state();
        let reached = counts == Some((video, audio)) && ready >= ElementReadyState::HaveMetadata;
        if reached || started.elapsed() >= timeout {
            return TrackWait {
                reached,
                counts,
                ready,
                waited: started.elapsed(),
            };
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A one-paragraph snapshot of everything observable, for the `notes` of a failure.
pub fn diagnostics(rig: &Rig) -> String {
    let video = vstats(rig);
    let audio = astats(rig);
    let events = rig.log.snapshot();
    let tail = events
        .iter()
        .rev()
        .take(12)
        .rev()
        .map(|timed| {
            format!(
                "+{}ms [{}] {:?}",
                timed.at.as_millis(),
                timed.thread,
                timed.event
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "position={} element-ready-state={:?} element-duration={} mediasource={:?} duration={} \
         video(count={} first={} last={}) audio(count={} first={} last={}) last events: {tail}",
        fmt_opt(rig.position()),
        rig.element_ready_state(),
        fmt_opt(rig.element_duration()),
        rig.backend.ready_state(),
        fmt_opt(rig.backend.duration()),
        video.count,
        fmt_opt(video.first_pts),
        fmt_opt(video.last_pts),
        audio.count,
        fmt_opt(audio.first_pts),
        fmt_opt(audio.last_pts),
    )
}

/// The first `max` events of the log, one per line: for the notes of a run that did not finish.
pub fn timeline(rig: &Rig, max: usize) -> String {
    rig.log
        .snapshot()
        .iter()
        .filter(|timed| !matches!(timed.event, MseEvent::Play(PlayEvent::PositionUpdated(_))))
        .take(max)
        .map(|timed| {
            format!(
                "+{}ms [{}] {:?}",
                timed.at.as_millis(),
                timed.thread,
                timed.event
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Shuts the rig down and records what teardown said (never swallowed).
pub fn shutdown(scn: &mut Scn, label: &str, rig: Rig) {
    match rig.shutdown() {
        Ok(info) => {
            scn.metric(&format!("shutdown_ms.{label}"), dur_ms(info.took));
            for message in info.messages {
                scn.note(format!("teardown message ({label}): {message}"));
            }
        }
        Err(error) => {
            scn.finding(format!("{label}: shutdown failed: {error}"));
        }
    }
}

/// What `wait_eos` found.
#[derive(Debug, Clone, Copy)]
pub struct EosWait {
    /// The `EndOfStream` event arrived (the rig posts it in both shapes).
    pub reached: bool,
    pub took: Duration,
}

/// Waits for the end of the stream: the `EndOfStream` event in the log.
pub fn wait_eos(rig: &Rig, timeout: Duration) -> EosWait {
    let started = Instant::now();
    let reached = rig.wait_for_eos(timeout);
    EosWait {
        reached,
        took: started.elapsed(),
    }
}

// ---------------------------------------------------------------------------------------------
// Feeding segments
// ---------------------------------------------------------------------------------------------

/// How the bytes of one segment are cut into appends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chunking {
    Whole,
    Fixed(usize),
    /// 13 bytes (mid-box), 4096 bytes (mid-sample), the rest, then a 1-byte tail; the init segment is
    /// cut 7 bytes in.
    MidCuts,
}

impl Chunking {
    pub fn name(&self) -> String {
        match self {
            Chunking::Whole => "whole".to_owned(),
            Chunking::Fixed(size) => format!("fixed-{size}"),
            Chunking::MidCuts => "mid-cuts".to_owned(),
        }
    }

    pub fn plan_for_init(&self, len: usize) -> ChunkPlan {
        match self {
            Chunking::Whole => ChunkPlan::Whole,
            Chunking::Fixed(size) => ChunkPlan::Fixed(*size),
            Chunking::MidCuts if len > 7 => ChunkPlan::Sizes(vec![7, len - 7]),
            Chunking::MidCuts => ChunkPlan::Whole,
        }
    }

    pub fn plan_for_media(&self, len: usize) -> ChunkPlan {
        match self {
            Chunking::Whole => ChunkPlan::Whole,
            Chunking::Fixed(size) => ChunkPlan::Fixed(*size),
            Chunking::MidCuts => mid_cut_plan(len),
        }
    }
}

/// `13`, `4096`, the rest, `1`: a cut inside a box header, one inside a sample, and a one-byte tail.
/// `ChunkPlan::Sizes` repeats its last size, so the sizes add up to exactly `len` and the 1 is used once.
pub fn mid_cut_plan(len: usize) -> ChunkPlan {
    if len > 13 + 4096 + 2 {
        ChunkPlan::Sizes(vec![13, 4096, len - 13 - 4096 - 1, 1])
    } else if len > 14 {
        ChunkPlan::Sizes(vec![13, len - 13 - 1, 1])
    } else {
        ChunkPlan::Whole
    }
}

/// One stream going into one source buffer.
#[derive(Clone, Copy)]
pub struct Feed<'a> {
    pub id: SourceBufferId,
    pub stream: &'a SegmentedStream,
    pub chunking: Chunking,
}

pub fn add_source_buffer(rig: &Rig, stream: &SegmentedStream) -> Res<SourceBufferId> {
    rig.backend
        .add_source_buffer(&stream.mime)
        .map_err(|error| {
            Abort::Fail(format!(
                "add_source_buffer({:?}) was refused: {error:?}",
                stream.mime
            ))
        })
}

/// Appends one segment as the chunks `plan` describes, each waited to its `UpdateEnd`. Returns the number
/// of appends and the time the whole segment took.
pub fn append_segment(
    rig: &Rig,
    id: SourceBufferId,
    segment: &Segment,
    plan: &ChunkPlan,
) -> Res<(usize, Duration)> {
    let started = Instant::now();
    let pieces = mse_segments::chunks(&segment.bytes, plan);
    let count = pieces.len();
    for piece in pieces {
        rig.append_and_wait(id, piece, APPEND_TIMEOUT)?;
    }
    Ok((count, started.elapsed()))
}

/// Init segments of all feeds, in order. Returns the number of appends made.
pub fn append_inits(rig: &Rig, feeds: &[Feed]) -> Res<usize> {
    let mut appends = 0;
    for feed in feeds {
        let plan = feed.chunking.plan_for_init(feed.stream.init.bytes.len());
        let (count, _) = append_segment(rig, feed.id, &feed.stream.init, &plan)
            .map_err(|e| e.context(&format!("init segment of {}", feed.stream.mime)))?;
        appends += count;
    }
    Ok(appends)
}

/// Media segments `range` of every feed, interleaved by segment index (video 0, audio 0, video 1, ...),
/// the order a player with two buffers produces.
pub fn append_media(rig: &Rig, feeds: &[Feed], range: std::ops::Range<usize>) -> Res<usize> {
    let mut appends = 0;
    for index in range {
        for feed in feeds {
            let Some(segment) = feed.stream.media.get(index) else {
                continue;
            };
            let plan = feed.chunking.plan_for_media(segment.bytes.len());
            let (count, _) = append_segment(rig, feed.id, segment, &plan).map_err(|e| {
                e.context(&format!("media segment {index} of {}", feed.stream.mime))
            })?;
            appends += count;
        }
    }
    Ok(appends)
}

/// One source buffer for `stream` (a muxed or single-track fixture), as a one-element feed list: GstMSE 1.28.9
/// deadlocks with two source buffers on one `MediaSource`, so every scenario except C02 and C05 plays one.
pub fn one_feed<'a>(rig: &Rig, stream: &'a SegmentedStream) -> Res<[Feed<'a>; 1]> {
    Ok([Feed {
        id: add_source_buffer(rig, stream)?,
        stream,
        chunking: Chunking::Whole,
    }])
}

/// `SourceBuffer.remove` and the wait for its `UpdateEnd`.
pub fn remove_and_wait(
    rig: &Rig,
    id: SourceBufferId,
    start: ClockTime,
    end: Option<ClockTime>,
) -> Res<Duration> {
    let before = update_ends(&rig.log, id);
    let started = Instant::now();
    rig.backend.remove(id, start, end)?;
    let done = rig.log.wait_for(APPEND_TIMEOUT, |events| {
        events
            .iter()
            .filter(|timed| matches!(timed.event, MseEvent::UpdateEnd(i) if i == id))
            .count()
            > before
    });
    if !done {
        return Err(Abort::Fail(format!(
            "remove({}, {}) on source buffer {id} returned Ok but no UpdateEnd followed within {APPEND_TIMEOUT:?}; {}",
            fmt_t(start),
            end.map_or("+Infinity".to_owned(), fmt_t),
            diagnostics(rig)
        )));
    }
    Ok(started.elapsed())
}

/// `buffered` of one source buffer, with a failure turned into a recorded `Abort::Fail`.
pub fn buffered(rig: &Rig, id: SourceBufferId) -> Res<Vec<TimeRange>> {
    rig.backend
        .buffered(id)
        .map_err(|error| Abort::Fail(format!("buffered({id}) failed: {error:?}")))
}

/// What `wait_buffered` saw: `buffered` right away (what a page reads in its `updateend` handler) and after
/// GstMSE had time to catch up.
#[derive(Debug, Clone)]
pub struct BufferedWait {
    /// The first read, immediately after the call that changed the buffer.
    pub immediate: Vec<TimeRange>,
    /// The last read: the first one `done` accepted, or the last one before the timeout.
    pub settled: Vec<TimeRange>,
    /// How long it took until `done` accepted a read (the timeout when it never did).
    pub elapsed: Duration,
    /// `done` accepted the immediate read.
    pub immediate_ok: bool,
}

/// Reads `buffered` of `id` now and then every 20 ms until `done` accepts it or `timeout` passes. GstMSE 1.28.9
/// finishes an append (`UpdateEnd`) before the data shows in `buffered`, so every judgment of `buffered` goes
/// through this and the lag is a finding of its own, not noise in the others.
pub fn wait_buffered(
    rig: &Rig,
    id: SourceBufferId,
    timeout: Duration,
    done: impl Fn(&[TimeRange]) -> bool,
) -> Res<BufferedWait> {
    let started = Instant::now();
    let immediate = buffered(rig, id)?;
    let immediate_ok = done(&immediate);
    let mut current = immediate.clone();
    let mut converged = immediate_ok;
    while !converged && started.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(20));
        current = buffered(rig, id)?;
        converged = done(&current);
    }
    Ok(BufferedWait {
        immediate,
        settled: current,
        elapsed: started.elapsed(),
        immediate_ok,
    })
}

/// A `done` predicate for `wait_buffered`: the ranges equal `expected` (milliseconds) within `tol_ms`.
pub fn matches_ranges(expected: Vec<(i64, i64)>, tol_ms: i64) -> impl Fn(&[TimeRange]) -> bool {
    move |ranges| compare_ranges("", ranges, &expected, tol_ms).is_empty()
}

/// Waits for `buffered` of `id` to equal `expected` (milliseconds, within `tol_ms`) for up to 3 s, records the
/// immediate and settled reads and the lag as metrics, and checks the settled read as `<name>`. Returns the wait.
pub fn check_buffered(
    scn: &mut Scn,
    rig: &Rig,
    name: &str,
    id: SourceBufferId,
    expected: &[(i64, i64)],
    tol_ms: i64,
) -> Res<BufferedWait> {
    let wait = wait_buffered(
        rig,
        id,
        Duration::from_secs(3),
        matches_ranges(expected.to_vec(), tol_ms),
    )?;
    scn.metric(&format!("{name}.immediate"), ranges_json(&wait.immediate))
        .metric(&format!("{name}.settled"), ranges_json(&wait.settled))
        .metric(&format!("{name}.lag_ms"), dur_ms(wait.elapsed));
    let deviations = compare_ranges(name, &wait.settled, expected, tol_ms);
    scn.check(
        name,
        deviations.is_empty(),
        format!(
            "buffered is {} after {} ms (immediately after the append: {}): {}",
            fmt_ranges(&wait.settled),
            dur_ms(wait.elapsed),
            fmt_ranges(&wait.immediate),
            deviations.join("; ")
        ),
    );
    Ok(wait)
}

/// Starts playback and waits for the end of the stream.
pub fn play_to_end(rig: &Rig, timeout: Duration) -> Res<EosWait> {
    rig.play()?;
    Ok(wait_eos(rig, timeout))
}

// ---------------------------------------------------------------------------------------------
// A watcher thread: when did a sink first satisfy a condition (millisecond resolution)?
// ---------------------------------------------------------------------------------------------

/// Polls a sink's statistics every millisecond on a thread of its own and remembers the instant a
/// condition first held. `finish` stops and joins the thread, so no scenario leaves one behind.
pub struct Watch {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Option<Instant>>>,
}

impl Watch {
    pub fn start(
        stats: &SharedStats,
        max: Duration,
        condition: impl Fn(&FrameStats) -> bool + Send + 'static,
    ) -> Watch {
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::clone(stats);
        let flag = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("conformance-watch".to_owned())
            .spawn(move || {
                let deadline = Instant::now() + max;
                while !flag.load(Ordering::Relaxed) && Instant::now() < deadline {
                    let held = {
                        let guard = stats.lock().unwrap_or_else(PoisonError::into_inner);
                        condition(&guard)
                    };
                    if held {
                        return Some(Instant::now());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                None
            })
            .ok();
        Watch { stop, handle }
    }

    /// Stops the thread and returns the instant the condition first held, if it did.
    pub fn finish(mut self) -> Option<Instant> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .and_then(|handle| handle.join().ok())
            .flatten()
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Deterministic bytes (garbage for C13)
// ---------------------------------------------------------------------------------------------

/// xorshift64*: a fixed seed gives the same garbage on every run and on both architectures.
#[derive(Debug, Clone)]
pub struct XorShift(u64);

impl XorShift {
    pub fn new(seed: u64) -> XorShift {
        XorShift(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            out.extend_from_slice(&self.next_u64().to_le_bytes());
        }
        out.truncate(len);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(at_ms: u64, event: MseEvent) -> TimedEvent {
        TimedEvent {
            at: Duration::from_millis(at_ms),
            thread: "test".to_owned(),
            event,
        }
    }

    fn cycle(start: u64, id: SourceBufferId) -> Vec<TimedEvent> {
        vec![
            timed(start, MseEvent::UpdateStart(id)),
            timed(start + 1, MseEvent::Update(id)),
            timed(start + 2, MseEvent::UpdateEnd(id)),
        ]
    }

    #[test]
    fn classify_finds_red_and_blue_with_a_ball_in_the_mean() {
        assert_eq!(classify([236, 12, 14]), Colour::Red);
        assert_eq!(classify([210, 40, 35]), Colour::Red);
        assert_eq!(classify([12, 20, 240]), Colour::Blue);
        assert_eq!(classify([30, 60, 200]), Colour::Blue);
        assert_eq!(classify([128, 128, 128]), Colour::Other);
        assert_eq!(classify([0, 0, 0]), Colour::Other);
        assert_eq!(classify([0, 255, 0]), Colour::Other);
    }

    #[test]
    fn nominal_colour_flips_at_four_seconds() {
        assert_eq!(nominal_colour(t_ms(0)), Colour::Red);
        assert_eq!(nominal_colour(t_ms(3_960)), Colour::Red);
        assert_eq!(nominal_colour(t_ms(4_000)), Colour::Blue);
        assert_eq!(nominal_colour(t_ms(7_960)), Colour::Blue);
        let shifted = shifted_colour(8_000);
        assert_eq!(shifted(t_ms(8_000)), Colour::Red);
        assert_eq!(shifted(t_ms(11_999)), Colour::Red);
        assert_eq!(shifted(t_ms(12_000)), Colour::Blue);
        assert_eq!(shifted(t_ms(100)), Colour::Red);
    }

    #[test]
    fn judge_colours_reports_wrong_samples_and_tolerates_the_flip_frame() {
        let sample = |ms: u64, rgb: [u8; 3]| ColourSample { pts: t_ms(ms), rgb };
        let samples = vec![
            sample(0, [230, 10, 10]),
            sample(1_000, [230, 10, 10]),
            sample(4_000, [230, 10, 10]), // the flip frame: either side is accepted
            sample(5_000, [10, 10, 230]),
            sample(6_000, [230, 10, 10]),   // wrong: red after the flip
            sample(7_000, [100, 100, 100]), // wrong: neither
        ];
        let verdict = judge_colours(&samples, nominal_colour, t_ms(40));
        assert_eq!(verdict.samples, 6);
        assert_eq!(verdict.wrong.len(), 2);
        assert!(verdict.wrong[0].contains("6.000s"));
        assert!(verdict.wrong[1].contains("7.000s"));
        assert_eq!((verdict.red, verdict.blue, verdict.other), (4, 1, 1));
        assert!(!verdict.ok());
        assert!(judge_colours(&samples[..4], nominal_colour, t_ms(40)).ok());
    }

    #[test]
    fn compare_ranges_lists_every_deviation() {
        let range = |start: u64, end: u64| TimeRange {
            start: t_ms(start),
            end: t_ms(end),
        };
        let good = [range(4_000, 8_000)];
        assert!(compare_ranges("v", &good, &[(4_000, 8_000)], 2).is_empty());
        assert!(compare_ranges("v", &[range(4_001, 8_000)], &[(4_000, 8_000)], 2).is_empty());
        let off = compare_ranges("v", &[range(3_960, 8_000)], &[(4_000, 8_000)], 2);
        assert_eq!(off.len(), 1);
        assert!(off[0].contains("start") && off[0].contains("-40 ms"));
        let count = compare_ranges(
            "v",
            &[range(0, 1_000), range(4_000, 8_000)],
            &[(4_000, 8_000)],
            2,
        );
        assert_eq!(count.len(), 1);
        assert!(count[0].contains("2 range(s)") && count[0].contains("1 range(s)"));
        assert_eq!(
            fmt_ranges(&[range(0, 1_500), range(2_000, 3_000)]),
            "[0.000-1.500] [2.000-3.000]"
        );
        assert_eq!(fmt_ranges(&[]), "[]");
    }

    #[test]
    fn event_order_accepts_clean_cycles_and_flags_the_rest() {
        let mut clean = cycle(0, 0);
        clean.extend(cycle(10, 1));
        clean.extend(cycle(20, 0));
        assert!(event_order_problems(&clean, &[0, 1], false).is_empty());

        // UpdateEnd before Update.
        let wrong = vec![
            timed(0, MseEvent::UpdateStart(0)),
            timed(1, MseEvent::UpdateEnd(0)),
        ];
        let problems = event_order_problems(&wrong, &[0], false);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("UpdateEnd"));

        // An open cycle is a problem.
        let open = vec![timed(0, MseEvent::UpdateStart(0))];
        assert!(event_order_problems(&open, &[0], false)[0].contains("still Started"));

        // Error is a problem unless allowed; then UpdateStart, Error, UpdateEnd is a cycle.
        let error = vec![
            timed(0, MseEvent::UpdateStart(0)),
            timed(1, MseEvent::Error(0)),
            timed(2, MseEvent::UpdateEnd(0)),
        ];
        assert_eq!(event_order_problems(&error, &[0], false).len(), 1);
        assert!(event_order_problems(&error, &[0], true).is_empty());
        assert_eq!(errors_in(&error).len(), 1);
    }

    #[test]
    fn ready_state_events_and_threads_are_read_from_the_log() {
        let events = vec![
            timed(
                5,
                MseEvent::ElementReadyState(ElementReadyState::HaveMetadata),
            ),
            TimedEvent {
                at: Duration::from_millis(9),
                thread: "gst".to_owned(),
                event: MseEvent::ElementReadyState(ElementReadyState::HaveEnoughData),
            },
            timed(12, MseEvent::SourceEnded),
        ];
        let states = ready_state_events(&events, 0);
        assert_eq!(states.len(), 2);
        assert_eq!(
            fmt_ready_states(&states),
            "HaveMetadata@5ms -> HaveEnoughData@9ms"
        );
        assert_eq!(ready_state_events(&events, 1).len(), 1);
        assert_eq!(fmt_ready_states(&[]), "none");
        let threads = threads_of(&events, |e| matches!(e, MseEvent::ElementReadyState(_)));
        assert_eq!(threads.len(), 2);
    }

    #[test]
    fn mid_cut_plan_adds_up_and_uses_the_tail_once() {
        for len in [20_000usize, 4_200, 4_111, 100, 15, 14, 3] {
            let plan = mid_cut_plan(len);
            let total: usize = match &plan {
                ChunkPlan::Whole => len,
                ChunkPlan::Sizes(sizes) => sizes.iter().sum(),
                ChunkPlan::Fixed(_) => unreachable!("mid_cut_plan never makes a Fixed plan"),
            };
            assert_eq!(total, len, "plan for {len}: {plan:?}");
        }
        assert_eq!(
            mid_cut_plan(10_000),
            ChunkPlan::Sizes(vec![13, 4096, 10_000 - 13 - 4096 - 1, 1])
        );
        assert_eq!(
            Chunking::MidCuts.plan_for_init(500),
            ChunkPlan::Sizes(vec![7, 493])
        );
        assert_eq!(
            Chunking::Fixed(1024).plan_for_media(10),
            ChunkPlan::Fixed(1024)
        );
        assert_eq!(Chunking::Whole.plan_for_init(10), ChunkPlan::Whole);
        assert_eq!(Chunking::MidCuts.name(), "mid-cuts");
        assert_eq!(Chunking::Fixed(65536).name(), "fixed-65536");
    }

    #[test]
    fn xorshift_is_deterministic_and_never_zero_seeded() {
        let mut a = XorShift::new(7);
        let mut b = XorShift::new(7);
        assert_eq!(a.bytes(33), b.bytes(33));
        assert_ne!(XorShift::new(7).bytes(16), XorShift::new(8).bytes(16));
        assert_eq!(XorShift::new(0).bytes(5).len(), 5);
        assert_ne!(XorShift::new(0).next_u64(), 0);
    }

    #[test]
    fn time_helpers_round_trip() {
        assert_eq!(ms_of(t_ms(1_234)), 1_234);
        assert_eq!(fmt_t(t_ms(4_000)), "4.000s");
        assert_eq!(fmt_opt(None), "none");
        assert!((secs(t_ms(2_500)) - 2.5).abs() < 1e-9);
        assert_eq!(dur_ms(Duration::from_micros(1_234_567)), 1234.6);
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(30);
        assert_eq!(ms_between(t0, t1), 30);
        assert_eq!(ms_between(t1, t0), -30);
    }

    #[test]
    fn abort_context_and_conversions_keep_the_class() {
        let abort = Abort::Fail("x".into()).context("while appending");
        assert!(matches!(&abort, Abort::Fail(m) if m == "while appending: x"));
        assert!(matches!(
            Abort::from(RigError::Setup("no registry".into())),
            Abort::Error(_)
        ));
        assert!(matches!(
            Abort::from(RigError::Timeout(Duration::from_secs(1), "UpdateEnd".into())),
            Abort::Fail(m) if m.contains("UpdateEnd")
        ));
        assert!(matches!(
            Abort::from(MseError::QuotaExceeded("full".into())),
            Abort::Fail(m) if m.contains("QuotaExceeded")
        ));
        assert!(matches!(
            Abort::from(SplitError::Truncated),
            Abort::Error(_)
        ));
    }

    #[test]
    fn scn_finish_maps_checks_and_aborts_to_statuses() {
        fn scn() -> Scn {
            Scn {
                result: ScenarioResult::new("X00", "test"),
                failures: Vec::new(),
                started: Instant::now(),
                rig_notes: BTreeSet::new(),
            }
        }
        let mut passing = scn();
        assert!(passing.check("a", true, "fine"));
        assert_eq!(passing.finish(Ok(())).status, Status::Pass);

        let mut failing = scn();
        assert!(!failing.check("b", false, "went wrong"));
        let result = failing.finish(Ok(()));
        assert_eq!(result.status, Status::Fail);
        assert_eq!(result.metrics["check.b"], false);
        assert!(result.notes.iter().any(|n| n == "FAIL b: went wrong"));

        assert_eq!(
            scn().finish(Err(Abort::Fail("hang".into()))).status,
            Status::Fail
        );
        assert_eq!(
            scn().finish(Err(Abort::Error("bug".into()))).status,
            Status::Error
        );
        assert_eq!(
            scn().finish(Err(Abort::Skip("no fixture".into()))).status,
            Status::Skip
        );
        let mut mixed = scn();
        mixed.check("c", false, "bad");
        assert_eq!(
            mixed.finish(Err(Abort::Skip("later".into()))).status,
            Status::Fail
        );
        let mut finding = scn();
        finding.finding("something");
        assert_eq!(finding.finish(Ok(())).status, Status::Fail);
    }

    #[test]
    fn note_once_deduplicates() {
        let mut scn = Scn {
            result: ScenarioResult::new("X00", "test"),
            failures: Vec::new(),
            started: Instant::now(),
            rig_notes: BTreeSet::new(),
        };
        scn.note_once("rig: sync=false");
        scn.note_once("rig: sync=false");
        scn.note_once("rig: sync=true");
        assert_eq!(scn.finish(Ok(())).notes.len(), 2);
    }
}
