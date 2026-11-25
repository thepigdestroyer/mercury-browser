// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `MseBackend`: the worker thread, the event posting, the [`MediaSourceBackend`] and [`FrameFeed`]
//! implementations.
//!
//! OWNER: item 3 of lane med-1. The two entry points `MseBackend::new` and
//! `MseBackend::is_type_supported` are the contract with the glue (item 5).
//!
//! # Threads and locks (decisions D8 to D10)
//!
//! * One **state lock** guards the whole [`MediaSource`] model (ready state, duration, the source
//!   buffers with their parsers and processors, the feed cursors). The script thread takes it for the
//!   synchronous checks and queries; the worker takes it for **one slice of an append** (at most
//!   [`APPEND_SLICE`] bytes, a fragment or a part of one) at a time, so a `buffered()` waits for at most
//!   one slice; a GStreamer thread takes it for a `pull`.
//! * One **worker thread** per MediaSource, named `mse-<n>`, started by the first job (an append, a
//!   remove, or an event that a synchronous call produced) and joined when the backend is dropped. It
//!   runs jobs in the order they were queued. A job is an append, a remove, or the delivery of events.
//! * **Every event is posted from the worker thread** through the [`EventSink`], never from the caller's
//!   thread and never while a lock of this module is held, so a closure may call any method of the
//!   backend (T02: no deadlock) and a slow closure never stalls the script thread. A synchronous call
//!   that must be heard (`set_duration`, `end_of_stream`) queues its events as a job.
//! * After `abort()`, `removeSourceBuffer` and `close()` return, no completion event of the aborted
//!   operation is delivered: each completion event carries the buffer's generation and the worker
//!   re-checks it, under an emission gate that those calls wait for (unless they run inside an event
//!   closure on the worker thread itself, which is already the only emitter).
//!
//! # What the DOM must know
//!
//! * `AppendDone`, `AppendError`, `RemoveDone`: `updating` is false and the data is in `buffered`.
//!   `AppendError` is followed by `SourceEnded { Some(Decode) }` for the append error algorithm.
//! * `SourceEnded` is posted every time the end of stream algorithm runs (an explicit `end_of_stream`
//!   too, and once per append error), `DurationChanged` for **every** change of the duration (an explicit
//!   `set_duration` too): the DOM fires `sourceended` and `durationchange` from the events and must not
//!   fire them itself.
//! * There is no `SourceOpen` event: an `append`, `remove`, `change_type` or attribute setter that finds
//!   the MediaSource `ended` makes it `open` again, and the DOM compares `ready_state()` before and after
//!   the call (`seeking` returns `true` for that case).
//! * After a decode error (`AppendError`, or `end_of_stream(Some(_))`) the media element has an error:
//!   `append` is `InvalidState` until `close()` and `open()` attach the MediaSource again.
//!
//! [`MediaSourceBackend`]: crate::api::MediaSourceBackend
//! [`FrameFeed`]: crate::api::FrameFeed
//! [`EventSink`]: crate::api::EventSink

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle, ThreadId};

use crate::api::{
    AppendMode, ElementReadyState, EosError, EventSink, FeedBatch, FrameFeed, Limits,
    MediaSourceBackend, MediaSourceReadyState, MemoryBudget, MseError, MseEvent, Ns,
    SourceBufferId, TimeRange, TrackConfig, TrackKind, TypeSupport,
};
use crate::media_source::{MediaSource, Outgoing, SliceStatus};
use crate::source_buffer::{APPEND_SLICE, AppendFailure};

/// Numbers the worker threads (`mse-1`, `mse-2`, ...).
static NEXT_WORKER: AtomicUsize = AtomicUsize::new(1);

type Wake = Arc<dyn Fn() + Send + Sync + 'static>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the worker does next.
enum Job {
    Append {
        id: SourceBufferId,
        generation: u64,
        data: Vec<u8>,
    },
    Remove {
        id: SourceBufferId,
        generation: u64,
        start: Ns,
        end: Option<Ns>,
    },
    /// Events that a synchronous call produced.
    Deliver(Vec<Outgoing>),
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    shutdown: bool,
    started: bool,
}

/// What the backend, its worker and its feed handles share.
struct Shared {
    state: Mutex<MediaSource>,
    queue: Mutex<Queue>,
    work: Condvar,
    events: EventSink,
    wake: Mutex<Option<Wake>>,
    /// Held while an event is being delivered; `abort` and friends pass through it.
    gate: Mutex<()>,
    emitter: Mutex<Option<ThreadId>>,
    shutdown: AtomicBool,
}

impl Shared {
    fn lock_state(&self) -> MutexGuard<'_, MediaSource> {
        lock(&self.state)
    }

    /// Runs `f` on the model and takes the outbox the call filled.
    fn with_state<R>(&self, f: impl FnOnce(&mut MediaSource) -> R) -> (R, Vec<Outgoing>) {
        let mut state = self.lock_state();
        let result = f(&mut state);
        let outbox = state.take_outbox();
        (result, outbox)
    }

    /// Waits for an event that is being delivered right now, so that nothing of an operation that was
    /// just cancelled arrives afterwards. A no-op on the emitting thread itself (an event closure that
    /// calls `abort`): it is the emitter, and the next event is checked before it is delivered.
    fn barrier(&self) {
        if *lock(&self.emitter) == Some(thread::current().id()) {
            return;
        }
        drop(lock(&self.gate));
    }

    /// Delivers an outbox: events in order, then one wake. Worker thread only.
    fn deliver(&self, outbox: Vec<Outgoing>) {
        let mut wake = false;
        for out in outbox {
            wake |= out.wake;
            if let Some(event) = out.event {
                self.emit(event, out.guard);
            }
        }
        if wake {
            self.call_wake();
        }
    }

    fn emit(&self, event: MseEvent, guard: Option<(SourceBufferId, u64)>) {
        if self.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let _gate = lock(&self.gate);
        if let Some((id, generation)) = guard
            && !self.lock_state().is_current(id, generation)
        {
            return;
        }
        *lock(&self.emitter) = Some(thread::current().id());
        let sink = Arc::clone(&self.events);
        if catch_unwind(AssertUnwindSafe(|| sink(event))).is_err() {
            log::error!("mercury-mse: an event closure panicked");
        }
        *lock(&self.emitter) = None;
    }

    fn call_wake(&self) {
        if self.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let wake = lock(&self.wake).clone();
        if let Some(wake) = wake
            && catch_unwind(AssertUnwindSafe(|| wake())).is_err()
        {
            log::error!("mercury-mse: the feed's wake closure panicked");
        }
    }

    fn run_job(&self, job: Job) {
        match job {
            Job::Deliver(outbox) => self.deliver(outbox),
            Job::Remove {
                id,
                generation,
                start,
                end,
            } => {
                let ((), outbox) = self.with_state(|s| s.run_remove(id, generation, start, end));
                self.deliver(outbox);
            }
            Job::Append {
                id,
                generation,
                data,
            } => self.run_append(id, generation, &data),
        }
    }

    /// The buffer append algorithm, slice by slice (decision D8): the state lock is held for one slice,
    /// the events of a slice are delivered after it is released.
    fn run_append(&self, id: SourceBufferId, generation: u64, data: &[u8]) {
        let mut failure: Option<AppendFailure> = None;
        for slice in data.chunks(APPEND_SLICE) {
            if self.shutdown.load(Ordering::SeqCst) {
                return;
            }
            let (status, outbox) = self.with_state(|state| {
                catch_unwind(AssertUnwindSafe(|| {
                    state.process_slice(id, generation, slice)
                }))
                .unwrap_or_else(|_| {
                    SliceStatus::Failed(AppendFailure::internal(
                        "a panic while processing an append",
                    ))
                })
            });
            self.deliver(outbox);
            match status {
                SliceStatus::Stale => return,
                SliceStatus::Ok => {}
                SliceStatus::Failed(f) => {
                    failure = Some(f);
                    break;
                }
            }
        }
        let ((), outbox) = self.with_state(|s| s.finish_append(id, generation, failure));
        self.deliver(outbox);
    }
}

fn worker_main(shared: Arc<Shared>) {
    loop {
        let job = {
            let mut queue = lock(&shared.queue);
            loop {
                if queue.shutdown {
                    return;
                }
                if let Some(job) = queue.jobs.pop_front() {
                    break job;
                }
                queue = shared
                    .work
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        shared.run_job(job);
    }
}

/// The backend of one MediaSource.
///
/// All methods take `&self` and may be called from any thread. Dropping it stops and joins the worker
/// thread (a drop that happens on the worker thread itself, from inside an event closure, lets the
/// thread end on its own) and frees everything the buffers hold.
pub struct MseBackend {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl MseBackend {
    /// Events go to `events` (called on the worker thread, never with a lock held); `types` answers
    /// decoder questions; `limits` is the quota.
    pub fn new(events: EventSink, types: Arc<dyn TypeSupport>, limits: Limits) -> MseBackend {
        log::debug!("mercury-mse: backend created");
        MseBackend {
            shared: Arc::new(Shared {
                state: Mutex::new(MediaSource::new(types, limits)),
                queue: Mutex::new(Queue::default()),
                work: Condvar::new(),
                events,
                wake: Mutex::new(None),
                gate: Mutex::new(()),
                emitter: Mutex::new(None),
                shutdown: AtomicBool::new(false),
            }),
            worker: Mutex::new(None),
        }
    }

    /// [`Self::new`] with a memory budget shared with other backends: every source buffer of this one
    /// charges it for the frames it stores, so the sum over all of them (all the pages of a content
    /// process, for the glue) cannot pass its cap (decision D17).
    pub fn with_budget(
        events: EventSink,
        types: Arc<dyn TypeSupport>,
        limits: Limits,
        budget: Arc<MemoryBudget>,
    ) -> MseBackend {
        let backend = MseBackend::new(events, types, limits);
        backend.shared.lock_state().set_budget(budget);
        backend
    }

    /// `MediaSource.isTypeSupported` (`mime::is_type_supported`).
    pub fn is_type_supported(mime: &str, types: &dyn TypeSupport) -> bool {
        crate::mime::is_type_supported(mime, types)
    }

    /// Queues jobs (in order) and starts the worker if this is the first.
    fn submit(&self, jobs: Vec<Job>) -> Result<(), MseError> {
        if jobs.is_empty() {
            return Ok(());
        }
        {
            let mut queue = lock(&self.shared.queue);
            if queue.shutdown {
                return Err(MseError::Other("the backend was shut down".into()));
            }
            let count = jobs.len();
            queue.jobs.extend(jobs);
            if !queue.started {
                let shared = Arc::clone(&self.shared);
                let n = NEXT_WORKER.fetch_add(1, Ordering::Relaxed);
                match thread::Builder::new()
                    .name(format!("mse-{n}"))
                    .spawn(move || worker_main(shared))
                {
                    Ok(handle) => {
                        queue.started = true;
                        *lock(&self.worker) = Some(handle);
                    }
                    Err(error) => {
                        for _ in 0..count {
                            queue.jobs.pop_back();
                        }
                        return Err(MseError::Other(format!(
                            "could not start the MSE worker thread: {error}"
                        )));
                    }
                }
            }
        }
        self.shared.work.notify_one();
        Ok(())
    }

    /// Hands events of a synchronous call to the worker.
    fn post(&self, outbox: Vec<Outgoing>) {
        if outbox.is_empty() {
            return;
        }
        if let Err(error) = self.submit(vec![Job::Deliver(outbox)]) {
            log::warn!("mercury-mse: events dropped: {error}");
        }
    }

    /// Runs a synchronous call on the model and posts the events it produced.
    fn call<R>(&self, f: impl FnOnce(&mut MediaSource) -> R) -> R {
        let (result, outbox) = self.shared.with_state(f);
        self.post(outbox);
        result
    }

    /// Runs a query that cannot change anything.
    fn query<R>(&self, f: impl FnOnce(&MediaSource) -> R) -> R {
        f(&self.shared.lock_state())
    }
}

impl Drop for MseBackend {
    fn drop(&mut self) {
        {
            let mut queue = lock(&self.shared.queue);
            queue.shutdown = true;
            queue.jobs.clear();
        }
        self.shared.shutdown.store(true, Ordering::SeqCst);
        self.shared.work.notify_all();
        let handle = lock(&self.worker).take();
        if let Some(handle) = handle
            && handle.thread().id() != thread::current().id()
        {
            let _ = handle.join();
        }
        // Free the buffers now: a feed handle may keep the shared state alive for a while.
        let mut state = self.shared.lock_state();
        state.close();
        drop(state.take_outbox());
        drop(state);
        *lock(&self.shared.wake) = None;
    }
}

impl MediaSourceBackend for MseBackend {
    fn open(&self) {
        self.call(|s| s.open());
    }

    fn close(&self) {
        self.call(|s| s.close());
        self.shared.barrier();
    }

    fn ready_state(&self) -> MediaSourceReadyState {
        self.query(MediaSource::ready_state)
    }

    fn add_source_buffer(&self, mime: &str) -> Result<SourceBufferId, MseError> {
        self.call(|s| s.add_source_buffer(mime))
    }

    fn remove_source_buffer(&self, id: SourceBufferId) -> Result<(), MseError> {
        let result = self.call(|s| s.remove_source_buffer(id));
        if result.is_ok() {
            self.shared.barrier();
        }
        result
    }

    fn source_buffer_count(&self) -> usize {
        self.query(MediaSource::source_buffer_count)
    }

    fn duration(&self) -> Option<f64> {
        self.query(MediaSource::duration)
    }

    fn set_duration(&self, secs: Option<f64>) -> Result<(), MseError> {
        self.call(|s| s.set_duration(secs))
    }

    fn end_of_stream(&self, error: Option<EosError>) -> Result<(), MseError> {
        self.call(|s| s.end_of_stream(error))
    }

    fn set_live_seekable_range(&self, start: f64, end: f64) -> Result<(), MseError> {
        self.call(|s| s.set_live_seekable_range(start, end))
    }

    fn clear_live_seekable_range(&self) -> Result<(), MseError> {
        self.call(|s| s.clear_live_seekable_range())
    }

    fn append(&self, id: SourceBufferId, data: Vec<u8>) -> Result<(), MseError> {
        let (prepared, outbox) = self.shared.with_state(|s| s.begin_append(id, data.len()));
        let generation = match prepared {
            Ok(generation) => generation,
            Err(error) => {
                self.post(outbox);
                return Err(error);
            }
        };
        let mut jobs = Vec::with_capacity(2);
        if !outbox.is_empty() {
            jobs.push(Job::Deliver(outbox));
        }
        jobs.push(Job::Append {
            id,
            generation,
            data,
        });
        if let Err(error) = self.submit(jobs) {
            self.call(|s| s.cancel_append(id));
            return Err(error);
        }
        Ok(())
    }

    fn abort(&self, id: SourceBufferId) -> Result<(), MseError> {
        let result = self.call(|s| s.abort(id));
        if result.is_ok() {
            self.shared.barrier();
        }
        result
    }

    fn remove(&self, id: SourceBufferId, start: f64, end: f64) -> Result<(), MseError> {
        let (prepared, outbox) = self.shared.with_state(|s| s.begin_remove(id, start, end));
        self.post(outbox);
        let (generation, start, end) = prepared?;
        let job = Job::Remove {
            id,
            generation,
            start,
            end,
        };
        if let Err(error) = self.submit(vec![job]) {
            self.call(|s| s.cancel_remove(id));
            return Err(error);
        }
        Ok(())
    }

    fn change_type(&self, id: SourceBufferId, mime: &str) -> Result<(), MseError> {
        self.call(|s| s.change_type(id, mime))
    }

    fn updating(&self, id: SourceBufferId) -> bool {
        self.query(|s| s.updating(id))
    }

    fn buffered(&self, id: SourceBufferId) -> Vec<TimeRange> {
        self.query(|s| s.buffered(id))
    }

    fn mode(&self, id: SourceBufferId) -> Result<AppendMode, MseError> {
        self.query(|s| s.mode(id))
    }

    fn set_mode(&self, id: SourceBufferId, mode: AppendMode) -> Result<(), MseError> {
        self.call(|s| s.set_mode(id, mode))
    }

    fn timestamp_offset(&self, id: SourceBufferId) -> Result<f64, MseError> {
        self.query(|s| s.timestamp_offset(id))
    }

    fn set_timestamp_offset(&self, id: SourceBufferId, secs: f64) -> Result<(), MseError> {
        self.call(|s| s.set_timestamp_offset(id, secs))
    }

    fn append_window_start(&self, id: SourceBufferId) -> Result<f64, MseError> {
        self.query(|s| s.append_window_start(id))
    }

    fn append_window_end(&self, id: SourceBufferId) -> Result<f64, MseError> {
        self.query(|s| s.append_window_end(id))
    }

    fn set_append_window_start(&self, id: SourceBufferId, secs: f64) -> Result<(), MseError> {
        self.call(|s| s.set_append_window_start(id, secs))
    }

    fn set_append_window_end(&self, id: SourceBufferId, secs: f64) -> Result<(), MseError> {
        self.call(|s| s.set_append_window_end(id, secs))
    }

    fn element_buffered(&self) -> Vec<TimeRange> {
        self.query(MediaSource::element_buffered)
    }

    fn seekable(&self) -> Vec<TimeRange> {
        self.query(MediaSource::seekable)
    }

    fn element_ready_state(&self, position: f64) -> ElementReadyState {
        self.query(|s| s.element_ready_state(position))
    }

    fn set_position(&self, secs: f64) {
        self.call(|s| s.set_position(secs));
    }

    fn seeking(&self, secs: f64) -> bool {
        self.call(|s| s.seeking(secs))
    }

    fn max_append_bytes(&self) -> usize {
        self.query(|s| s.max_append_bytes())
    }

    fn frame_feed(&self) -> Arc<dyn FrameFeed> {
        Arc::new(BackendFeed {
            shared: Arc::clone(&self.shared),
        })
    }
}

/// The pull interface GStreamer uses, over the same state as the backend.
struct BackendFeed {
    shared: Arc<Shared>,
}

impl FrameFeed for BackendFeed {
    fn tracks(&self) -> Vec<TrackConfig> {
        self.shared.lock_state().feed_tracks()
    }

    fn all_initialized(&self) -> bool {
        self.shared.lock_state().feed_all_initialized()
    }

    fn duration(&self) -> Option<Ns> {
        self.shared.lock_state().feed_duration()
    }

    fn seek(&self, position: Ns) {
        self.shared.lock_state().feed_seek(position);
    }

    fn pull(&self, kind: TrackKind, max_frames: usize) -> FeedBatch {
        self.shared.lock_state().feed_pull(kind, max_frames)
    }

    fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.shared.wake) = Some(wake);
    }
}

/// The backend is itself a feed, so that a caller holding the backend can hand the same object to the
/// GStreamer side (`Arc<MseBackend>` coerces to `Arc<dyn FrameFeed>`); `frame_feed()` gives a separate
/// handle over the same state.
impl FrameFeed for MseBackend {
    fn tracks(&self) -> Vec<TrackConfig> {
        self.shared.lock_state().feed_tracks()
    }

    fn all_initialized(&self) -> bool {
        self.shared.lock_state().feed_all_initialized()
    }

    fn duration(&self) -> Option<Ns> {
        self.shared.lock_state().feed_duration()
    }

    fn seek(&self, position: Ns) {
        self.shared.lock_state().feed_seek(position);
    }

    fn pull(&self, kind: TrackKind, max_frames: usize) -> FeedBatch {
        self.shared.lock_state().feed_pull(kind, max_frames)
    }

    fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.shared.wake) = Some(wake);
    }
}
