// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The real thing behind the feed: an `MseBackend` (worker thread, parsers, processor, track buffers)
//! that a test appends fixture segments to, the way a page does, and whose `FrameFeed` the source element
//! pulls from. `AppendDone` of each append is waited for before the next one goes in (as a DASH player
//! does), and a parse error fails the test at once.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use mercury_mse::{
    EosError, FrameFeed, Limits, MediaSourceBackend, MseBackend, MseEvent, SourceBufferId,
};
use mercury_mse_gst::GstTypeSupport;
use mercury_mse_testkit::{ChunkPlan, SegmentedStream, chunks};

const WAIT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Log {
    events: Vec<(Instant, MseEvent)>,
}

pub struct RealSource {
    pub backend: Arc<MseBackend>,
    log: Arc<(Mutex<Log>, Condvar)>,
}

impl RealSource {
    /// An open MediaSource with the given limits.
    pub fn with_limits(limits: Limits) -> RealSource {
        let log: Arc<(Mutex<Log>, Condvar)> = Arc::default();
        let sink = {
            let log = Arc::clone(&log);
            Arc::new(move |event: MseEvent| {
                let mut guard = log.0.lock().unwrap();
                guard.events.push((Instant::now(), event));
                log.1.notify_all();
            })
        };
        let backend = Arc::new(MseBackend::new(sink, Arc::new(GstTypeSupport), limits));
        backend.open();
        RealSource { backend, log }
    }

    pub fn new() -> RealSource {
        RealSource::with_limits(Limits::default())
    }

    pub fn feed(&self) -> Arc<dyn FrameFeed> {
        self.backend.frame_feed()
    }

    pub fn add(&self, mime: &str) -> SourceBufferId {
        self.backend
            .add_source_buffer(mime)
            .unwrap_or_else(|e| panic!("add_source_buffer({mime:?}): {e}"))
    }

    fn finished(&self, id: SourceBufferId) -> usize {
        self.log
            .0
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|(_, e)| {
                matches!(e, MseEvent::AppendDone { id: i, .. } | MseEvent::AppendError { id: i, .. } | MseEvent::RemoveDone { id: i, .. } if *i == id)
            })
            .count()
    }

    /// The time of the `n`th completion (0-based) of `id`, waiting for it.
    fn wait_completion(&self, id: SourceBufferId, n: usize) -> (Instant, MseEvent) {
        let deadline = Instant::now() + WAIT;
        let mut guard = self.log.0.lock().unwrap();
        loop {
            let found = guard
                .events
                .iter()
                .filter(|(_, e)| {
                    matches!(e, MseEvent::AppendDone { id: i, .. } | MseEvent::AppendError { id: i, .. } | MseEvent::RemoveDone { id: i, .. } if *i == id)
                })
                .nth(n)
                .cloned();
            if let Some(found) = found {
                return found;
            }
            let now = Instant::now();
            assert!(
                now < deadline,
                "no AppendDone for buffer {id} within {WAIT:?}"
            );
            guard = self.log.1.wait_timeout(guard, deadline - now).unwrap().0;
        }
    }

    /// Appends `bytes` and waits for its `AppendDone`; returns when that event arrived.
    pub fn append(&self, id: SourceBufferId, bytes: &[u8]) -> Instant {
        let before = self.finished(id);
        self.backend
            .append(id, bytes.to_vec())
            .unwrap_or_else(|e| panic!("append of {} bytes: {e}", bytes.len()));
        let (at, event) = self.wait_completion(id, before);
        assert!(
            matches!(event, MseEvent::AppendDone { .. }),
            "append failed: {event:?}"
        );
        at
    }

    pub fn append_plan(&self, id: SourceBufferId, bytes: &[u8], plan: &ChunkPlan) {
        for piece in chunks(bytes, plan) {
            self.append(id, piece);
        }
    }

    pub fn append_init(&self, id: SourceBufferId, stream: &SegmentedStream) {
        self.append(id, &stream.init.bytes);
    }

    /// Media segment `index` of `stream`; returns the time of its `AppendDone`.
    pub fn append_segment(
        &self,
        id: SourceBufferId,
        stream: &SegmentedStream,
        index: usize,
    ) -> Instant {
        self.append(id, &stream.media[index].bytes)
    }

    pub fn append_stream(&self, id: SourceBufferId, stream: &SegmentedStream, plan: &ChunkPlan) {
        self.append_plan(id, &stream.init.bytes, plan);
        for segment in &stream.media {
            self.append_plan(id, &segment.bytes, plan);
        }
    }

    pub fn add_and_append(&self, stream: &SegmentedStream) -> SourceBufferId {
        let id = self.add(&stream.mime);
        self.append_stream(id, stream, &ChunkPlan::Whole);
        id
    }

    pub fn end_of_stream(&self) {
        self.backend
            .end_of_stream(None::<EosError>)
            .expect("end_of_stream");
    }

    /// The events so far (for notes and failure messages).
    pub fn events(&self) -> Vec<MseEvent> {
        self.log
            .0
            .lock()
            .unwrap()
            .events
            .iter()
            .map(|(_, e)| e.clone())
            .collect()
    }
}

/// A fixture as a page appends it.
pub fn load(name: &str) -> SegmentedStream {
    mercury_mse_testkit::load(name).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// A fixture loaded once and shared by reference between threads (T12).
pub struct Loaded {
    pub mime: String,
    pub stream: SegmentedStream,
}

pub fn loaded(name: &str) -> Loaded {
    let stream = load(name);
    Loaded {
        mime: stream.mime.clone(),
        stream,
    }
}
