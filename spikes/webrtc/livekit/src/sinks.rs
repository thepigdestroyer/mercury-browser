// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `MediaFrameSink` adapters from a capture stream (the bridge crate, or in the product the
//! `GStreamerMediaStream`) to LiveKit's libwebrtc sources.
//!
//! Video: an I420 frame is copied into an `I420Buffer` (libwebrtc owns the buffer and the encoder
//! reads it later on its own thread, so the producer's memory cannot be lent) and handed to
//! `NativeVideoSource::capture_frame`. Audio: a 10 ms S16 block goes to
//! `NativeAudioSource::capture_frame`; the source is created with a queue of 0 ms, which is
//! libwebrtc's direct path, so the future the call returns is already complete when it is created
//! and one poll with a waker that does nothing is enough (no executor in the producer's thread).

use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use libwebrtc::audio_frame::AudioFrame;
use libwebrtc::audio_source::native::NativeAudioSource;
use libwebrtc::video_frame::{I420Buffer, NV12Buffer, VideoFrame, VideoRotation};
use libwebrtc::video_source::native::NativeVideoSource;
use mercury_frame_sink::{ClockMap, MediaFrameSink, Micros, RawAudioFrame, RawVideoFrame};

/// The consumer clock: microseconds on a monotonic clock that starts the first time it is asked.
/// libwebrtc's own `TimeMicros` is not exposed by the bindings; a video source only needs a
/// monotonic microsecond clock (its timestamp aligner maps the values onto its own).
pub fn consumer_now() -> Micros {
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    Micros(i64::try_from(start.elapsed().as_micros()).unwrap_or(i64::MAX))
}

/// Counters a sink keeps; read them with [`SinkCounters::snapshot`].
#[derive(Debug, Default)]
pub struct SinkCounters {
    frames: AtomicU64,
    rejected: AtomicU64,
    ended: AtomicU64,
    copy_ns: AtomicU64,
}

/// A copy of [`SinkCounters`] at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SinkSnapshot {
    /// Frames or blocks accepted by libwebrtc.
    pub frames: u64,
    /// Frames or blocks libwebrtc (or the format check) refused.
    pub rejected: u64,
    /// Times `ended` reached the sink (must be 0 or 1).
    pub ended: u64,
    /// Nanoseconds spent copying into libwebrtc's buffers (video only).
    pub copy_ns: u64,
}

impl SinkCounters {
    /// Reads all counters.
    pub fn snapshot(&self) -> SinkSnapshot {
        SinkSnapshot {
            frames: self.frames.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            ended: self.ended.load(Ordering::Relaxed),
            copy_ns: self.copy_ns.load(Ordering::Relaxed),
        }
    }
}

type EndedHook = Mutex<Option<Box<dyn FnOnce() + Send>>>;

fn run_ended_hook(hook: &EndedHook) {
    let taken = hook.lock().ok().and_then(|mut guard| guard.take());
    if let Some(hook) = taken {
        hook();
    }
}

/// Copies `rows` rows of `row_bytes` from a plane with one stride into a plane with another.
fn copy_plane(
    dst: &mut [u8],
    dst_stride: usize,
    src: &[u8],
    src_stride: usize,
    row_bytes: usize,
    rows: usize,
) {
    if rows == 0 {
        return;
    }
    if dst_stride == src_stride && dst_stride == row_bytes {
        let len = row_bytes * rows;
        dst[..len].copy_from_slice(&src[..len]);
        return;
    }
    for row in 0..rows {
        let d = row * dst_stride;
        let s = row * src_stride;
        dst[d..d + row_bytes].copy_from_slice(&src[s..s + row_bytes]);
    }
}

/// Sends the I420 frames of a capture stream into a [`NativeVideoSource`].
pub struct LiveKitVideoSink {
    source: NativeVideoSource,
    clock: Mutex<Option<ClockMap>>,
    counters: SinkCounters,
    on_ended: EndedHook,
}

impl LiveKitVideoSink {
    /// `clock` maps the producer's timestamps to [`consumer_now`]. `None` anchors the producer's
    /// first timestamp on the consumer clock at the moment the first frame arrives.
    pub fn new(source: NativeVideoSource, clock: Option<ClockMap>) -> Self {
        Self {
            source,
            clock: Mutex::new(clock),
            counters: SinkCounters::default(),
            on_ended: Mutex::new(None),
        }
    }

    /// Runs `hook` once when the producer says the stream ended (the device was released). The
    /// bindings cannot mark a local track ended, so the hook is where the caller disables the
    /// track or detaches it from its sender.
    pub fn set_on_ended(&self, hook: impl FnOnce() + Send + 'static) {
        if let Ok(mut guard) = self.on_ended.lock() {
            *guard = Some(Box::new(hook));
        }
    }

    /// The counters of this sink.
    pub fn counters(&self) -> &SinkCounters {
        &self.counters
    }

    fn consumer_timestamp(&self, producer: Micros) -> i64 {
        let mut guard = self.clock.lock().expect("clock lock");
        let map = guard.get_or_insert_with(|| ClockMap::new(producer, consumer_now()));
        map.to_consumer(producer).0
    }

    /// Pushes an NV12 frame (what a macOS camera or a hardware decoder gives) without converting it
    /// first; libwebrtc converts when it encodes. Used by the conversion measurement of t05.
    pub fn push_nv12(
        &self,
        width: u32,
        height: u32,
        y: &[u8],
        uv: &[u8],
        timestamp: Micros,
    ) -> bool {
        let mut buffer = NV12Buffer::new(width, height);
        let (stride_y, stride_uv) = buffer.strides();
        let (dst_y, dst_uv) = buffer.data_mut();
        copy_plane(
            dst_y,
            stride_y as usize,
            y,
            width as usize,
            width as usize,
            height as usize,
        );
        copy_plane(
            dst_uv,
            stride_uv as usize,
            uv,
            width as usize,
            width.div_ceil(2) as usize * 2,
            height.div_ceil(2) as usize,
        );
        let mut frame = VideoFrame::new(VideoRotation::VideoRotation0, buffer);
        frame.timestamp_us = self.consumer_timestamp(timestamp);
        self.source.capture_frame(&frame)
    }
}

impl MediaFrameSink for LiveKitVideoSink {
    fn video_frame(&self, frame: RawVideoFrame<'_>) {
        let started = Instant::now();
        let mut buffer = I420Buffer::new(frame.width, frame.height);
        let (stride_y, stride_u, stride_v) = buffer.strides();
        let (dst_y, dst_u, dst_v) = buffer.data_mut();
        let (w, h) = (frame.width as usize, frame.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        copy_plane(dst_y, stride_y as usize, frame.y, frame.stride_y, w, h);
        copy_plane(dst_u, stride_u as usize, frame.u, frame.stride_u, cw, ch);
        copy_plane(dst_v, stride_v as usize, frame.v, frame.stride_v, cw, ch);
        let mut out = VideoFrame::new(VideoRotation::VideoRotation0, buffer);
        out.timestamp_us = self.consumer_timestamp(frame.timestamp);
        let copied = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.counters.copy_ns.fetch_add(copied, Ordering::Relaxed);
        if self.source.capture_frame(&out) {
            self.counters.frames.fetch_add(1, Ordering::Relaxed);
        } else {
            self.counters.rejected.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn ended(&self) {
        self.counters.ended.fetch_add(1, Ordering::Relaxed);
        run_ended_hook(&self.on_ended);
    }
}

/// Sends the 10 ms blocks of a capture stream into a [`NativeAudioSource`] created with a queue
/// of 0 ms (the source then needs exactly 10 ms per call, which is what the bridge delivers).
pub struct LiveKitAudioSink {
    source: NativeAudioSource,
    counters: SinkCounters,
    on_ended: EndedHook,
}

impl LiveKitAudioSink {
    /// `source` must have been created with `queue_size_ms = 0`, 48 kHz and the stream's channel count.
    pub fn new(source: NativeAudioSource) -> Self {
        Self {
            source,
            counters: SinkCounters::default(),
            on_ended: Mutex::new(None),
        }
    }

    /// See [`LiveKitVideoSink::set_on_ended`].
    pub fn set_on_ended(&self, hook: impl FnOnce() + Send + 'static) {
        if let Ok(mut guard) = self.on_ended.lock() {
            *guard = Some(Box::new(hook));
        }
    }

    /// The counters of this sink.
    pub fn counters(&self) -> &SinkCounters {
        &self.counters
    }
}

/// Polls a future once with a waker that does nothing. `None` means it was not ready.
fn poll_once<F: Future>(future: F) -> Option<F::Output> {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}

impl MediaFrameSink for LiveKitAudioSink {
    fn audio_frame(&self, frame: RawAudioFrame<'_>) {
        let block = AudioFrame {
            data: std::borrow::Cow::Borrowed(frame.samples),
            sample_rate: frame.sample_rate,
            num_channels: u32::from(frame.channels),
            samples_per_channel: u32::try_from(frame.samples_per_channel()).unwrap_or(0),
        };
        match poll_once(self.source.capture_frame(&block)) {
            Some(Ok(())) => {
                self.counters.frames.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                self.counters.rejected.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn ended(&self) {
        self.counters.ended.fetch_add(1, Ordering::Relaxed);
        run_ended_hook(&self.on_ended);
    }
}
