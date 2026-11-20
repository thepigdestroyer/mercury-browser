// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The event log: every callback GstMSE and GstPlay make, with the time and the thread. A scenario
//! reads it to check order ("`updatestart` before `update` before `updateend`") and thread.
//! Complete (not a stub); item 1 extended it with a hook list, an elapsed clock and the real OS
//! thread name (see [`current_thread_name`]).

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gstreamer::ClockTime;

/// A source buffer, numbered in the order it was added (0, 1, 2, ...); never reused.
pub type SourceBufferId = usize;

/// `MseSrc::ready-state`, which is the media element's `readyState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ElementReadyState {
    HaveNothing,
    HaveMetadata,
    HaveCurrentData,
    HaveFutureData,
    HaveEnoughData,
}

/// `GstPlay` messages, in the PlayBin shape only.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayEvent {
    StateChanged(String),
    Buffering(u8),
    EndOfStream,
    SeekDone(Option<ClockTime>),
    DurationChanged(Option<ClockTime>),
    PositionUpdated(Option<ClockTime>),
    MediaInfoUpdated,
    Error(String),
    Warning(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum MseEvent {
    SourceOpen,
    SourceEnded,
    SourceClose,
    SourceBufferAdded(SourceBufferId),
    SourceBufferRemoved(SourceBufferId),
    UpdateStart(SourceBufferId),
    Update(SourceBufferId),
    UpdateEnd(SourceBufferId),
    Abort(SourceBufferId),
    Error(SourceBufferId),
    /// `notify::ready-state` on `msesrc`.
    ElementReadyState(ElementReadyState),
    /// `notify::duration` on `msesrc` (`None` is "unknown").
    ElementDuration(Option<ClockTime>),
    Play(PlayEvent),
    /// `notify::position` on `msesrc`. Only logged after `MseBackend::set_log_position(true)`: it can
    /// fire for every buffer, so it is off by default (C14 turns it on to measure the update rate).
    ElementPosition(Option<ClockTime>),
}

#[derive(Debug, Clone)]
pub struct TimedEvent {
    /// Time since the log was created.
    pub at: Duration,
    /// The OS name of the emitting thread (see [`current_thread_name`]); GStreamer's threads have
    /// names such as `queue:src` or `GstPlay`, Rust threads the name given to the builder.
    pub thread: String,
    pub event: MseEvent,
}

/// A callback run on the emitting thread for every event, after it is in the log and with no lock held.
/// A hook may call back into the log and into the backend: that is how re-entrancy (T02) is exercised.
pub type EventHook = Arc<dyn Fn(&TimedEvent) + Send + Sync + 'static>;

/// The name of the calling thread. A thread that GStreamer or GLib created is unknown to Rust's
/// `std::thread::current()` (it has no name there), so the name is read from the OS
/// (`pthread_getname_np`, which also returns the names Rust sets). Falls back to the Rust name and
/// then to `unnamed-<id>`.
pub fn current_thread_name() -> String {
    let mut buffer = [0 as libc::c_char; 64];
    // SAFETY: `pthread_self()` is the calling thread and `buffer` is writable for its whole length;
    // the call writes a NUL-terminated string that fits in it, or fails and writes nothing we read.
    let status = unsafe {
        libc::pthread_getname_np(libc::pthread_self(), buffer.as_mut_ptr(), buffer.len())
    };
    if status == 0 {
        // SAFETY: on success the buffer holds a NUL-terminated string (written by the call above).
        let name = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
        if !name.is_empty() {
            return name.into_owned();
        }
    }
    let thread = std::thread::current();
    match thread.name() {
        Some(name) => name.to_owned(),
        None => format!("unnamed-{:?}", thread.id()),
    }
}

/// Cheap to clone; all clones share one log.
#[derive(Clone)]
pub struct EventLog {
    shared: Arc<(Mutex<Vec<TimedEvent>>, Condvar)>,
    hooks: Arc<Mutex<Vec<EventHook>>>,
    t0: Instant,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    pub fn new() -> Self {
        EventLog {
            shared: Arc::new((Mutex::new(Vec::new()), Condvar::new())),
            hooks: Arc::new(Mutex::new(Vec::new())),
            t0: Instant::now(),
        }
    }

    /// Records `event` now, on the calling thread, then runs the hooks on that thread.
    pub fn push(&self, event: MseEvent) {
        let timed = TimedEvent {
            at: self.t0.elapsed(),
            thread: current_thread_name(),
            event,
        };
        let (lock, changed) = &*self.shared;
        lock.lock().unwrap().push(timed.clone());
        changed.notify_all();
        // Hooks run with no lock held, so that they can use the log (and anything else) freely.
        let hooks: Vec<EventHook> = self.hooks.lock().unwrap().clone();
        for hook in hooks {
            hook(&timed);
        }
    }

    /// Registers a callback for every event pushed from now on, on the thread that pushed it.
    pub fn add_hook(&self, hook: impl Fn(&TimedEvent) + Send + Sync + 'static) {
        self.hooks.lock().unwrap().push(Arc::new(hook));
    }

    /// Removes every hook (a hook that calls into the backend should be removed before teardown).
    pub fn clear_hooks(&self) {
        self.hooks.lock().unwrap().clear();
    }

    /// Time since the log was created: the clock `TimedEvent::at` and the counting sinks' arrival
    /// times use, so that events and frames can be put on one timeline.
    pub fn elapsed(&self) -> Duration {
        self.t0.elapsed()
    }

    /// How many events are recorded.
    pub fn len(&self) -> usize {
        self.shared.0.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn snapshot(&self) -> Vec<TimedEvent> {
        self.shared.0.lock().unwrap().clone()
    }

    /// The events recorded at index `from` and later (`snapshot()[from..]` without copying the rest).
    pub fn snapshot_from(&self, from: usize) -> Vec<TimedEvent> {
        let events = self.shared.0.lock().unwrap();
        events
            .get(from..)
            .map(<[TimedEvent]>::to_vec)
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        self.shared.0.lock().unwrap().clear();
    }

    /// Blocks until `predicate` is true of the log or `timeout` passes; returns whether it became true.
    /// The predicate runs under the log's lock: it must not call back into the log.
    pub fn wait_for(&self, timeout: Duration, predicate: impl Fn(&[TimedEvent]) -> bool) -> bool {
        let (lock, changed) = &*self.shared;
        let deadline = Instant::now() + timeout;
        let mut events = lock.lock().unwrap();
        loop {
            if predicate(&events) {
                return true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            events = changed.wait_timeout(events, left).unwrap().0;
        }
    }

    /// How many recorded events satisfy `predicate`.
    pub fn count(&self, predicate: impl Fn(&MseEvent) -> bool) -> usize {
        self.shared
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|timed| predicate(&timed.event))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_for_sees_an_event_pushed_from_another_thread() {
        let log = EventLog::new();
        let other = log.clone();
        let handle = std::thread::Builder::new()
            .name("pusher".into())
            .spawn(move || other.push(MseEvent::SourceOpen))
            .unwrap();
        assert!(log.wait_for(Duration::from_secs(5), |events| {
            events.iter().any(|e| e.event == MseEvent::SourceOpen)
        }));
        handle.join().unwrap();
        assert_eq!(log.snapshot()[0].thread, "pusher");
        assert_eq!(log.count(|e| *e == MseEvent::SourceOpen), 1);
    }

    #[test]
    fn wait_for_times_out() {
        let log = EventLog::new();
        assert!(!log.wait_for(Duration::from_millis(20), |events| !events.is_empty()));
    }

    #[test]
    fn the_thread_name_is_the_os_name_of_a_named_thread() {
        let name = std::thread::Builder::new()
            .name("os-name-test".into())
            .spawn(current_thread_name)
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(name, "os-name-test");
    }

    #[test]
    fn an_unnamed_thread_gets_a_stable_fallback_name() {
        let name = std::thread::spawn(current_thread_name).join().unwrap();
        // macOS leaves the pthread name empty for it; the fallback names the Rust thread id.
        assert!(name.starts_with("unnamed-"), "{name}");
    }

    #[test]
    fn hooks_run_on_the_pushing_thread_after_the_event_is_logged_and_can_use_the_log() {
        let log = EventLog::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let seen = seen.clone();
            let inner = log.clone();
            log.add_hook(move |timed| {
                // The event is already in the log, and the hook may call back into it.
                seen.lock()
                    .unwrap()
                    .push((timed.event.clone(), inner.len(), timed.thread.clone()));
            });
        }
        let other = log.clone();
        std::thread::Builder::new()
            .name("hooked".into())
            .spawn(move || other.push(MseEvent::UpdateEnd(3)))
            .unwrap()
            .join()
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.as_slice(),
            &[(MseEvent::UpdateEnd(3), 1, "hooked".to_owned())]
        );
        log.clear_hooks();
        log.push(MseEvent::SourceOpen);
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn snapshot_from_returns_the_tail_and_tolerates_a_start_past_the_end() {
        let log = EventLog::new();
        log.push(MseEvent::SourceOpen);
        log.push(MseEvent::UpdateStart(0));
        log.push(MseEvent::UpdateEnd(0));
        assert_eq!(log.len(), 3);
        let tail = log.snapshot_from(1);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[1].event, MseEvent::UpdateEnd(0));
        assert!(log.snapshot_from(9).is_empty());
        assert!(!log.is_empty());
    }
}
