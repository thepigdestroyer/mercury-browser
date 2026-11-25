// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Shared helpers of the `model_*` and `accept_*` integration tests of lane med-1 (item 3, `model`): an
//! event recorder that remembers on which thread each event arrived, a rig that wraps a real
//! `MseBackend` (real worker thread, real parser, real processor), helpers that append fixture streams
//! under a chunk plan and wait for the events, a feed drain, and a few measuring tools. Not a test
//! file itself: each `tests/model_*.rs` and `tests/accept_*.rs` has `mod model_support;`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use mercury_mse::MseBackend;
use mercury_mse::api::{
    AllDecoders, AppendErrorKind, CodedFrame, EosError, EventSink, FeedItem, FeedState, FrameFeed,
    Limits, MediaSourceBackend, MseError, MseEvent, Ns, SourceBufferId, TimeRange, TrackConfig,
    TrackKind,
};
use mercury_mse_testkit::{ChunkPlan, SegmentedStream, chunks};

pub const SEC: Ns = 1_000_000_000;
pub const MS: Ns = 1_000_000;

/// How long a test waits for an event before it calls it a failure (a hang is a failure, never a wait).
pub const WAIT: Duration = Duration::from_secs(10);

// ----------------------------------------------------------------------------------------------
// The recorder

/// One event as it arrived.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub seq: usize,
    pub at: Duration,
    pub thread: ThreadId,
    pub thread_name: String,
    pub event: MseEvent,
}

type Hook = Arc<dyn Fn(&MseEvent) + Send + Sync>;

/// Collects every event; hooks run on the emitting thread before the event is recorded and with no lock
/// of the recorder held (so a hook may call the backend and the recorder).
pub struct Recorder {
    inner: Mutex<Inner>,
    cv: Condvar,
    start: Instant,
    hooks: Mutex<Vec<Hook>>,
}

#[derive(Default)]
struct Inner {
    log: Vec<Recorded>,
    /// What each append or remove of a buffer ended in, in order (so a test finds the outcome of its
    /// own append without scanning the log).
    completions: HashMap<SourceBufferId, Vec<Outcome>>,
}

impl Recorder {
    pub fn new() -> Arc<Recorder> {
        Arc::new(Recorder {
            inner: Mutex::new(Inner::default()),
            cv: Condvar::new(),
            start: Instant::now(),
            hooks: Mutex::new(Vec::new()),
        })
    }

    pub fn sink(self: &Arc<Self>) -> EventSink {
        let rec = Arc::clone(self);
        Arc::new(move |event| rec.record(event))
    }

    /// The hooks run first, on the emitting thread, with no lock of the recorder held; the event becomes
    /// visible to the test only after them. A hook therefore sees the backend as the DOM would when the
    /// event is delivered: a test that reacts to the event cannot have started another operation yet.
    fn record(&self, event: MseEvent) {
        let thread = thread::current();
        let hooks: Vec<Hook> = self.hooks.lock().unwrap().clone();
        for hook in hooks {
            hook(&event);
        }
        {
            let mut inner = self.inner.lock().unwrap();
            let seq = inner.log.len();
            match &event {
                MseEvent::AppendDone { id } | MseEvent::RemoveDone { id } => {
                    inner
                        .completions
                        .entry(*id)
                        .or_default()
                        .push(Outcome::Done);
                }
                MseEvent::AppendError { id, kind } => {
                    inner
                        .completions
                        .entry(*id)
                        .or_default()
                        .push(Outcome::Error(*kind));
                }
                _ => {}
            }
            inner.log.push(Recorded {
                seq,
                at: self.start.elapsed(),
                thread: thread.id(),
                thread_name: thread.name().unwrap_or("").to_string(),
                event,
            });
        }
        self.cv.notify_all();
    }

    pub fn add_hook(&self, hook: impl Fn(&MseEvent) + Send + Sync + 'static) {
        self.hooks.lock().unwrap().push(Arc::new(hook));
    }

    pub fn clear_hooks(&self) {
        self.hooks.lock().unwrap().clear();
    }

    pub fn snapshot(&self) -> Vec<Recorded> {
        self.inner.lock().unwrap().log.clone()
    }

    pub fn events(&self) -> Vec<MseEvent> {
        self.snapshot().into_iter().map(|r| r.event).collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().log.len()
    }

    pub fn count(&self, pred: impl Fn(&MseEvent) -> bool) -> usize {
        self.inner
            .lock()
            .unwrap()
            .log
            .iter()
            .filter(|r| pred(&r.event))
            .count()
    }

    /// Waits until `pred` holds over the events so far, or the timeout.
    pub fn wait_for(&self, timeout: Duration, pred: impl Fn(&[Recorded]) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut inner = self.inner.lock().unwrap();
        loop {
            if pred(&inner.log) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            inner = self.cv.wait_timeout(inner, deadline - now).unwrap().0;
        }
    }

    /// Appends and removes that finished for `id`: `AppendDone`, `AppendError` and `RemoveDone`.
    pub fn completions(&self, id: SourceBufferId) -> usize {
        self.inner
            .lock()
            .unwrap()
            .completions
            .get(&id)
            .map_or(0, Vec::len)
    }

    /// Waits for the `index`-th (from 0) completion of `id` and returns what it was.
    pub fn wait_completion(
        &self,
        id: SourceBufferId,
        index: usize,
        timeout: Duration,
    ) -> Option<Outcome> {
        let deadline = Instant::now() + timeout;
        let mut inner = self.inner.lock().unwrap();
        loop {
            if let Some(outcome) = inner.completions.get(&id).and_then(|v| v.get(index)) {
                return Some(outcome.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            inner = self.cv.wait_timeout(inner, deadline - now).unwrap().0;
        }
    }
}

// ----------------------------------------------------------------------------------------------
// The rig

/// What an append did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Error(AppendErrorKind),
    /// `append` itself refused (the synchronous checks).
    Refused(MseError),
    /// No completion event within [`WAIT`].
    Timeout,
}

/// A backend, its recorder, and the helpers that drive them.
pub struct Rig {
    pub backend: Arc<MseBackend>,
    pub rec: Arc<Recorder>,
}

impl Rig {
    /// An open MediaSource with the default limits.
    pub fn new() -> Rig {
        Rig::with_limits(Limits::default())
    }

    pub fn with_limits(limits: Limits) -> Rig {
        let rig = Rig::closed(limits);
        rig.backend.open();
        rig
    }

    /// A MediaSource that is still closed (not attached to a media element).
    pub fn closed(limits: Limits) -> Rig {
        let rec = Recorder::new();
        let backend = Arc::new(MseBackend::new(rec.sink(), Arc::new(AllDecoders), limits));
        Rig { backend, rec }
    }

    pub fn add(&self, mime: &str) -> SourceBufferId {
        self.backend
            .add_source_buffer(mime)
            .unwrap_or_else(|e| panic!("add_source_buffer({mime:?}): {e}"))
    }

    /// Appends `bytes` and waits for the completion event of this append.
    pub fn append(&self, id: SourceBufferId, bytes: &[u8]) -> Outcome {
        let before = self.rec.completions(id);
        if let Err(error) = self.backend.append(id, bytes.to_vec()) {
            return Outcome::Refused(error);
        }
        self.rec
            .wait_completion(id, before, WAIT)
            .unwrap_or(Outcome::Timeout)
    }

    /// Appends and expects `AppendDone`.
    pub fn append_ok(&self, id: SourceBufferId, bytes: &[u8]) {
        assert_eq!(
            self.append(id, bytes),
            Outcome::Done,
            "append of {} bytes",
            bytes.len()
        );
    }

    /// Appends `bytes` cut by `plan`, one append per piece, each waited for.
    pub fn append_plan(&self, id: SourceBufferId, bytes: &[u8], plan: &ChunkPlan) {
        for piece in chunks(bytes, plan) {
            self.append_ok(id, piece);
        }
    }

    /// Appends the init segment and every media segment of `stream` under `plan`.
    pub fn append_stream(&self, id: SourceBufferId, stream: &SegmentedStream, plan: &ChunkPlan) {
        self.append_plan(id, &stream.init.bytes, plan);
        for segment in &stream.media {
            self.append_plan(id, &segment.bytes, plan);
        }
    }

    /// Adds a source buffer for `stream` and appends all of it.
    pub fn add_and_append(&self, stream: &SegmentedStream) -> SourceBufferId {
        let id = self.add(&stream.mime);
        self.append_stream(id, stream, &ChunkPlan::Whole);
        id
    }

    /// `remove` and waits for `RemoveDone`.
    pub fn remove_ok(&self, id: SourceBufferId, start: f64, end: f64) {
        let before = self.rec.completions(id);
        self.backend
            .remove(id, start, end)
            .unwrap_or_else(|e| panic!("remove({start}, {end}): {e}"));
        assert!(
            self.rec.wait_completion(id, before, WAIT).is_some(),
            "RemoveDone did not arrive"
        );
    }

    /// Waits until `updating` is false for every id.
    pub fn wait_idle(&self, ids: &[SourceBufferId]) {
        let deadline = Instant::now() + WAIT;
        while ids.iter().any(|id| self.backend.updating(*id)) {
            assert!(Instant::now() < deadline, "still updating after {WAIT:?}");
            thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn buffered_secs(&self, id: SourceBufferId) -> Vec<(f64, f64)> {
        secs(&self.backend.buffered(id))
    }
}

/// The ranges as `(start, end)` seconds.
pub fn secs(ranges: &[TimeRange]) -> Vec<(f64, f64)> {
    ranges
        .iter()
        .map(|r| (r.start as f64 / 1e9, r.end as f64 / 1e9))
        .collect()
}

/// Asserts that `got` has the ranges of `want`, every edge within `tol_ms`.
#[track_caller]
pub fn assert_ranges(label: &str, got: &[TimeRange], want: &[(f64, f64)], tol_ms: f64) {
    let got = secs(got);
    assert_eq!(
        got.len(),
        want.len(),
        "{label}: {got:?} has another number of ranges than {want:?}"
    );
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g.0 - w.0).abs() * 1e3 <= tol_ms && (g.1 - w.1).abs() * 1e3 <= tol_ms,
            "{label}: {got:?} is not {want:?} within {tol_ms} ms"
        );
    }
}

pub fn load(name: &str) -> SegmentedStream {
    mercury_mse_testkit::load(name).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

// ----------------------------------------------------------------------------------------------
// The feed

/// What a feed drain collected for one kind.
#[derive(Debug)]
pub struct Drained {
    pub frames: Vec<CodedFrame>,
    pub discont: Vec<bool>,
    pub configs: Vec<TrackConfig>,
    pub state: FeedState,
}

/// Pulls `kind` until the feed stops saying `More`.
pub fn drain(feed: &dyn FrameFeed, kind: TrackKind) -> Drained {
    let mut out = Drained {
        frames: Vec::new(),
        discont: Vec::new(),
        configs: Vec::new(),
        state: FeedState::WaitingForData,
    };
    for _ in 0..1_000_000 {
        let batch = feed.pull(kind, 64);
        for item in batch.items {
            match item {
                FeedItem::Config(config) => out.configs.push(config),
                FeedItem::Frame { frame, discont } => {
                    out.frames.push(frame);
                    out.discont.push(discont);
                }
            }
        }
        out.state = batch.state;
        if batch.state != FeedState::More {
            return out;
        }
    }
    panic!("the feed never stopped saying More");
}

/// Frames in decode order, with presentation times that follow their predecessors within `slack`
/// (B-frames reorder presentation but never by more than a few frames).
#[track_caller]
pub fn assert_in_order(label: &str, frames: &[CodedFrame]) {
    for pair in frames.windows(2) {
        assert!(
            pair[1].dts >= pair[0].dts,
            "{label}: decode order broken: {} then {}",
            pair[0].dts,
            pair[1].dts
        );
    }
}

// ----------------------------------------------------------------------------------------------
// Measuring

/// The number of threads of this process, `None` when the platform gives no way to count them.
pub fn thread_count() -> Option<usize> {
    if let Ok(dir) = std::fs::read_dir("/proc/self/task") {
        return Some(dir.count());
    }
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-M", "-p", &pid])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let lines = String::from_utf8_lossy(&output.stdout).lines().count();
    // One header line, then one line per thread.
    lines.checked_sub(1)
}

/// The number of open file descriptors of this process, `None` when the platform gives no way to count
/// them (the count includes the descriptor of the directory listing, a constant offset).
pub fn fd_count() -> Option<usize> {
    for dir in ["/proc/self/fd", "/dev/fd"] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            return Some(entries.count());
        }
    }
    None
}

/// A small deterministic generator (xorshift64*), so that a failing seed replays.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next() >> 24) as u8).collect()
    }
}

/// Runs `f` on a helper thread and fails if it does not finish in `limit`: a deadlock is a failure
/// with a name, never a hung test run.
pub fn watchdog<T: Send + 'static>(
    label: &str,
    limit: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name(format!("watched-{label}"))
        .spawn(move || {
            let _ = tx.send(f());
        })
        .expect("spawn");
    match rx.recv_timeout(limit) {
        Ok(value) => {
            let _ = handle.join();
            value
        }
        // The closure ended without a value: it panicked. Say why.
        Err(mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Ok(()) => panic!("{label}: the watched closure returned no value"),
            Err(payload) => std::panic::resume_unwind(payload),
        },
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("{label}: no answer within {limit:?} (a deadlock or a hang)");
        }
    }
}

pub fn eos_name(error: Option<EosError>) -> &'static str {
    match error {
        None => "none",
        Some(EosError::Network) => "network",
        Some(EosError::Decode) => "decode",
    }
}

pub fn backend_dyn(rig: &Rig) -> Arc<dyn MediaSourceBackend> {
    rig.backend.clone()
}
