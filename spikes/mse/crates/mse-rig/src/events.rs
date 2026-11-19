// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The event log: every callback GstMSE and GstPlay make, with the time and the thread. A scenario
//! reads it to check order ("`updatestart` before `update` before `updateend`") and thread.
//! Complete (not a stub); item 1 may extend it.

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
}

#[derive(Debug, Clone)]
pub struct TimedEvent {
    /// Time since the log was created.
    pub at: Duration,
    /// `std::thread::current().name()`, or `unnamed-<id>`; GStreamer's threads have names such as `queue:src`.
    pub thread: String,
    pub event: MseEvent,
}

/// Cheap to clone; all clones share one log.
#[derive(Clone)]
pub struct EventLog {
    shared: Arc<(Mutex<Vec<TimedEvent>>, Condvar)>,
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
            t0: Instant::now(),
        }
    }

    /// Records `event` now, on the calling thread.
    pub fn push(&self, event: MseEvent) {
        let thread = std::thread::current();
        let thread = match thread.name() {
            Some(name) => name.to_owned(),
            None => format!("unnamed-{:?}", thread.id()),
        };
        let (lock, changed) = &*self.shared;
        lock.lock().unwrap().push(TimedEvent {
            at: self.t0.elapsed(),
            thread,
            event,
        });
        changed.notify_all();
    }

    pub fn snapshot(&self) -> Vec<TimedEvent> {
        self.shared.0.lock().unwrap().clone()
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
}
