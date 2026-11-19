// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `MediaFrameSink` adapters: what a capture stream of the media-capture lane calls on its
//! streaming thread, turned into libwebrtc calls.
//!
//! Video goes into the `AdaptedVideoTrackSource` of a [`crate::video::LocalVideo`]; audio goes
//! into the virtual device of [`crate::audio`]. When the stream ends (`ended()`), the local track
//! is disabled: the wrapper cannot mark a local track `Ended` (no `set_state`), so the product
//! ends a sender by `remove_track` and a renegotiation (the remote side sees `on_remove_track`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use mercury_frame_sink::{ClockMap, MediaFrameSink, Micros, RawAudioFrame, RawVideoFrame};
use shiguredo_webrtc::MediaStreamTrack;

use crate::audio::VirtualAudio;
use crate::video::{LocalVideo, Push, VideoPusher};

/// libwebrtc's clock now, in microseconds (millisecond resolution: `rtc::TimeMillis`). Anchor a
/// [`ClockMap`] with `(stream.producer_now(), consumer_now())`.
pub fn consumer_now() -> Micros {
    Micros(shiguredo_webrtc::time_millis() * 1000)
}

/// What a sink did, readable from any thread.
#[derive(Debug, Default)]
pub struct SinkCounters {
    /// Frames or blocks the producer delivered.
    pub received: AtomicU64,
    /// Given to libwebrtc.
    pub pushed: AtomicU64,
    /// Dropped by libwebrtc's adapter, or refused by the device (nobody recording).
    pub dropped: AtomicU64,
    /// Failed validation.
    pub invalid: AtomicU64,
    /// Calls to `ended()` (must end at 1).
    pub ended: AtomicU64,
    /// Frames or blocks that arrived after `ended()`.
    pub after_end: AtomicU64,
}

impl SinkCounters {
    /// `received/pushed/dropped/invalid/ended/after_end` for a log line.
    pub fn summary(&self) -> String {
        let get = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        format!(
            "received={} pushed={} dropped={} invalid={} ended={} after_end={}",
            get(&self.received),
            get(&self.pushed),
            get(&self.dropped),
            get(&self.invalid),
            get(&self.ended),
            get(&self.after_end),
        )
    }
}

fn disable(track: &Mutex<Option<MediaStreamTrack>>) {
    let guard: MutexGuard<'_, Option<MediaStreamTrack>> = track
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(track) = guard.as_ref() {
        track.set_enabled(false);
    }
}

/// Video frames of a capture stream into a local video source.
pub struct ShiguredoVideoSink {
    pusher: VideoPusher,
    clock: ClockMap,
    counters: Arc<SinkCounters>,
    track: Mutex<Option<MediaStreamTrack>>,
}

impl ShiguredoVideoSink {
    /// A sink feeding `video`; `clock` maps the producer's timestamps to libwebrtc's.
    pub fn new(video: &LocalVideo, clock: ClockMap) -> Arc<ShiguredoVideoSink> {
        Arc::new(ShiguredoVideoSink {
            pusher: video.pusher(),
            clock,
            counters: Arc::new(SinkCounters::default()),
            track: Mutex::new(Some(video.media_track())),
        })
    }

    /// The counters.
    pub fn counters(&self) -> Arc<SinkCounters> {
        Arc::clone(&self.counters)
    }
}

impl MediaFrameSink for ShiguredoVideoSink {
    fn video_frame(&self, frame: RawVideoFrame<'_>) {
        let counters = &self.counters;
        counters.received.fetch_add(1, Ordering::Relaxed);
        if counters.ended.load(Ordering::Acquire) > 0 {
            counters.after_end.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let timestamp = self.clock.to_consumer(frame.timestamp).0;
        let counter = match self.pusher.push(&frame, timestamp) {
            Push::Pushed => &counters.pushed,
            Push::Dropped => &counters.dropped,
            Push::Invalid => &counters.invalid,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn ended(&self) {
        self.counters.ended.fetch_add(1, Ordering::AcqRel);
        disable(&self.track);
    }
}

/// Audio blocks of a capture stream into the virtual audio device.
pub struct ShiguredoAudioSink {
    audio: VirtualAudio,
    clock: ClockMap,
    counters: Arc<SinkCounters>,
    track: Mutex<Option<MediaStreamTrack>>,
}

impl ShiguredoAudioSink {
    /// A sink feeding `audio`. `track`, when given, is disabled on `ended()`.
    pub fn new(
        audio: VirtualAudio,
        clock: ClockMap,
        track: Option<MediaStreamTrack>,
    ) -> Arc<ShiguredoAudioSink> {
        Arc::new(ShiguredoAudioSink {
            audio,
            clock,
            counters: Arc::new(SinkCounters::default()),
            track: Mutex::new(track),
        })
    }

    /// The counters.
    pub fn counters(&self) -> Arc<SinkCounters> {
        Arc::clone(&self.counters)
    }
}

impl MediaFrameSink for ShiguredoAudioSink {
    fn audio_frame(&self, frame: RawAudioFrame<'_>) {
        let counters = &self.counters;
        counters.received.fetch_add(1, Ordering::Relaxed);
        if counters.ended.load(Ordering::Acquire) > 0 {
            counters.after_end.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if frame.validate().is_err() {
            counters.invalid.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let capture_time = self.clock.to_consumer(frame.timestamp);
        let counter = if self
            .audio
            .push_block(frame.samples, frame.channels, Some(capture_time))
        {
            &counters.pushed
        } else {
            &counters.dropped
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn ended(&self) {
        self.counters.ended.fetch_add(1, Ordering::AcqRel);
        disable(&self.track);
    }
}
