// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Shared helpers: GStreamer initialisation, the process clock, element construction, and the
//! "am I inside a sink callback" flag that keeps re-entrant calls from deadlocking.

use std::cell::Cell;
use std::sync::{Arc, Mutex, Once};

use gstreamer as gst;
use gstreamer::prelude::*;
use mercury_frame_sink::Micros;

use crate::BridgeError;

static INIT: Once = Once::new();
static INIT_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Initialises GStreamer once per process. Every constructor calls it; calling it first only
/// moves the cost (plugin registry scan) to a moment of your choosing.
pub fn init() -> Result<(), BridgeError> {
    INIT.call_once(|| {
        if let Err(error) = gst::init() {
            *INIT_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
        }
    });
    match INIT_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        Some(error) => Err(BridgeError::Init(error)),
        None => Ok(()),
    }
}

/// The clock every bridge pipeline uses: GStreamer's monotonic system clock, the same one the
/// product's `servo-media-gstreamer` pins its pipelines to.
pub(crate) fn system_clock() -> gst::Clock {
    gst::SystemClock::obtain()
}

/// Microseconds from a clock time (truncating).
pub(crate) fn micros(time: gst::ClockTime) -> Micros {
    Micros(i64::try_from(time.nseconds() / 1000).unwrap_or(i64::MAX))
}

/// The pipeline clock now, in microseconds, on the scale of frame timestamps (what
/// `CaptureStream::producer_now` returns, for code that has no stream at hand, such as a sink
/// measuring the delay between capture and delivery).
pub fn clock_now() -> Micros {
    micros(system_clock().time())
}

/// A new pipeline on the process clock. Its base time is the clock time of the moment it goes
/// PLAYING (the pipeline's default), so a live source's running time starts at zero when the
/// first sink is added and `base time + running time` is the clock time of the capture.
pub(crate) fn new_pipeline(name: &str) -> gst::Pipeline {
    let pipeline = gst::Pipeline::with_name(name);
    pipeline.use_clock(Some(&system_clock()));
    pipeline
}

/// Makes an element, with a readable error when its plugin is missing.
pub(crate) fn make(factory: &'static str) -> Result<gst::Element, BridgeError> {
    gst::ElementFactory::make(factory)
        .build()
        .map_err(|_| BridgeError::MissingElement(factory))
}

/// The timestamp of a sample's buffer on the pipeline clock: the element's base time (the pipeline
/// hands its own to every element when it goes PLAYING) plus the running time of the buffer PTS
/// (the segment applied; the raw PTS when there is no usable segment). A buffer without PTS gets
/// the clock time now.
pub(crate) fn sample_timestamp(
    element: &gst::Element,
    sample: &gst::Sample,
    buffer: &gst::BufferRef,
) -> Micros {
    let Some(pts) = buffer.pts() else {
        return clock_now();
    };
    let running = sample
        .segment()
        .and_then(|segment| segment.downcast_ref::<gst::ClockTime>())
        .and_then(|segment| segment.to_running_time(pts))
        .unwrap_or(pts);
    let base = element.base_time().unwrap_or(gst::ClockTime::ZERO);
    micros(base + running)
}

thread_local! {
    static IN_SINK_CALL: Cell<u32> = const { Cell::new(0) };
}

/// Marks the current thread as inside a sink callback for its lifetime.
pub(crate) struct SinkCallGuard;

impl SinkCallGuard {
    pub(crate) fn enter() -> Self {
        IN_SINK_CALL.with(|depth| depth.set(depth.get() + 1));
        SinkCallGuard
    }
}

impl Drop for SinkCallGuard {
    fn drop(&mut self) {
        IN_SINK_CALL.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Whether the calling thread is inside a sink callback (a streaming thread of a branch). Stopping
/// a pipeline or tearing a branch down from there would wait for the very thread that asks, so
/// those calls are handed to a short-lived helper thread instead.
pub(crate) fn in_sink_call() -> bool {
    IN_SINK_CALL.with(|depth| depth.get() > 0)
}

/// Runs `job` on a short-lived thread; runs it here when no thread can be started.
pub(crate) fn run_detached(name: &str, job: impl FnOnce() + Send + 'static) {
    let job = Arc::new(Mutex::new(Some(job)));
    let for_thread = Arc::clone(&job);
    let spawned = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            if let Some(job) = for_thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
                job();
            }
        });
    if spawned.is_err()
        && let Some(job) = job.lock().unwrap_or_else(|e| e.into_inner()).take()
    {
        job();
    }
}
