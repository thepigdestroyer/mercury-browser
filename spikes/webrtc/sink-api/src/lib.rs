// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The contract between a capture stream (the `getUserMedia` streams of the media-capture lane)
//! and a WebRTC sender, written once and used by every spike crate of the sp-rtc lane.
//!
//! In the product this trait moves into the vendored `servo-media-streams` (`MediaStream` gets
//! `add_frame_sink` and `remove_frame_sink`; R14 section 4.6 point 3, media-capture review r2,
//! "WebRTC with libwebrtc"). Here it is a plain crate so that the producer side (GStreamer) and
//! the consumer side (libwebrtc through either binding crate) can be built and tested without
//! Servo. Nothing in this crate names GStreamer or libwebrtc.
//!
//! Conventions:
//! - video is I420 (three planes, 8 bits, chroma halved in both directions, rounding up);
//! - audio is interleaved signed 16-bit, 48 kHz, delivered in 10 ms blocks;
//! - timestamps are microseconds on the producer's clock, mapped to the consumer's clock with a
//!   [`ClockMap`];
//! - a sink is called from the producer's streaming thread and must not block;
//! - [`MediaFrameSink::ended`] is called exactly once, when the device is released (track
//!   `stop()`, page unload, tab teardown), so the WebRTC track ends when the camera light does.

use std::fmt;
use std::sync::{Arc, Mutex};

/// Audio sample rate the bridge delivers.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
/// Length of one audio block, in milliseconds (libwebrtc consumes audio in 10 ms blocks).
pub const AUDIO_BLOCK_MS: u32 = 10;
/// Samples per channel in one audio block at [`AUDIO_SAMPLE_RATE`].
pub const AUDIO_SAMPLES_PER_BLOCK: usize = (AUDIO_SAMPLE_RATE / 1000 * AUDIO_BLOCK_MS) as usize;

/// A microsecond timestamp. Its clock is named by whoever produced it: the producer's pipeline
/// clock before a [`ClockMap`] is applied, the consumer's clock afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Micros(pub i64);

/// Why a frame is not usable. Sinks may assume frames from a conforming producer are valid; the
/// tests of both sides call [`RawVideoFrame::validate`] and [`RawAudioFrame::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// Width or height is zero.
    EmptyFrame,
    /// A plane is shorter than its stride and the frame size require.
    PlaneTooShort {
        /// `"y"`, `"u"` or `"v"`.
        plane: &'static str,
        /// Bytes the plane must have at least.
        needed: usize,
        /// Bytes it has.
        got: usize,
    },
    /// A stride is smaller than the row it must hold.
    StrideTooSmall {
        /// `"y"`, `"u"` or `"v"`.
        plane: &'static str,
        /// Bytes a row needs.
        needed: usize,
        /// The stride given.
        got: usize,
    },
    /// The audio block does not hold 10 ms for its channel count and rate.
    WrongBlockLength {
        /// Samples (all channels) expected.
        expected: usize,
        /// Samples given.
        got: usize,
    },
    /// Zero channels or a sample rate that is not [`AUDIO_SAMPLE_RATE`].
    BadAudioFormat,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::EmptyFrame => write!(f, "the frame has no pixels"),
            FrameError::PlaneTooShort { plane, needed, got } => {
                write!(f, "the {plane} plane has {got} bytes, needs {needed}")
            }
            FrameError::StrideTooSmall { plane, needed, got } => {
                write!(f, "the {plane} stride is {got}, a row needs {needed}")
            }
            FrameError::WrongBlockLength { expected, got } => {
                write!(f, "the audio block has {got} samples, expected {expected}")
            }
            FrameError::BadAudioFormat => {
                write!(f, "the audio format is not 48 kHz with 1 or more channels")
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Width of a chroma plane for a luma width (rounds up).
pub const fn chroma_dim(luma: u32) -> u32 {
    luma.div_ceil(2)
}

/// One I420 video frame, borrowed from the producer for the duration of the call.
#[derive(Debug, Clone, Copy)]
pub struct RawVideoFrame<'a> {
    /// Luma width in pixels.
    pub width: u32,
    /// Luma height in pixels.
    pub height: u32,
    /// Luma plane.
    pub y: &'a [u8],
    /// First chroma plane (Cb).
    pub u: &'a [u8],
    /// Second chroma plane (Cr).
    pub v: &'a [u8],
    /// Bytes per luma row.
    pub stride_y: usize,
    /// Bytes per Cb row.
    pub stride_u: usize,
    /// Bytes per Cr row.
    pub stride_v: usize,
    /// Capture time on the producer's clock.
    pub timestamp: Micros,
}

impl RawVideoFrame<'_> {
    /// Bytes the three planes need together when tightly packed (strides equal to the row length).
    pub fn packed_len(width: u32, height: u32) -> usize {
        let (w, h) = (width as usize, height as usize);
        let (cw, ch) = (chroma_dim(width) as usize, chroma_dim(height) as usize);
        w * h + 2 * cw * ch
    }

    /// Checks sizes and strides. The last row of a plane does not need to be padded to the stride.
    pub fn validate(&self) -> Result<(), FrameError> {
        if self.width == 0 || self.height == 0 {
            return Err(FrameError::EmptyFrame);
        }
        let planes = [
            ("y", self.y.len(), self.stride_y, self.width, self.height),
            (
                "u",
                self.u.len(),
                self.stride_u,
                chroma_dim(self.width),
                chroma_dim(self.height),
            ),
            (
                "v",
                self.v.len(),
                self.stride_v,
                chroma_dim(self.width),
                chroma_dim(self.height),
            ),
        ];
        for (plane, len, stride, row, rows) in planes {
            let row = row as usize;
            if stride < row {
                return Err(FrameError::StrideTooSmall {
                    plane,
                    needed: row,
                    got: stride,
                });
            }
            let needed = stride * (rows as usize - 1) + row;
            if len < needed {
                return Err(FrameError::PlaneTooShort {
                    plane,
                    needed,
                    got: len,
                });
            }
        }
        Ok(())
    }
}

/// One block of interleaved signed 16-bit audio, borrowed for the duration of the call.
#[derive(Debug, Clone, Copy)]
pub struct RawAudioFrame<'a> {
    /// Interleaved samples, `channels` per sample time.
    pub samples: &'a [i16],
    /// Channel count (1 or 2 in practice).
    pub channels: u16,
    /// Sample rate; always [`AUDIO_SAMPLE_RATE`] from a conforming producer.
    pub sample_rate: u32,
    /// Capture time of the first sample, on the producer's clock.
    pub timestamp: Micros,
}

impl RawAudioFrame<'_> {
    /// Samples per channel.
    pub fn samples_per_channel(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    /// Checks that the block is 10 ms at 48 kHz.
    pub fn validate(&self) -> Result<(), FrameError> {
        if self.channels == 0 || self.sample_rate != AUDIO_SAMPLE_RATE {
            return Err(FrameError::BadAudioFormat);
        }
        let expected = AUDIO_SAMPLES_PER_BLOCK * usize::from(self.channels);
        if self.samples.len() != expected {
            return Err(FrameError::WrongBlockLength {
                expected,
                got: self.samples.len(),
            });
        }
        Ok(())
    }
}

/// Receives the frames of one capture stream. Every method has a default that ignores the call,
/// so an audio-only sink does not implement `video_frame`.
pub trait MediaFrameSink: Send + Sync {
    /// A video frame. Called on the producer's streaming thread; copy what you keep and return.
    fn video_frame(&self, _frame: RawVideoFrame<'_>) {}
    /// A 10 ms audio block. Called on the producer's streaming thread; copy what you keep.
    fn audio_frame(&self, _frame: RawAudioFrame<'_>) {}
    /// The stream ended: the device was released. Called exactly once per sink; nothing follows.
    fn ended(&self) {}
}

/// Identifies one registration in a [`SinkSet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameSinkId(pub u64);

/// The fan-out a stream keeps for its sinks: ids, delivery, and the exactly-once `ended`.
///
/// The product stream (`GStreamerMediaStream`) holds one of these next to its `tee`. Delivery
/// takes a snapshot of the registrations, so a sink may add or remove sinks from inside a
/// callback without deadlocking.
#[derive(Default)]
pub struct SinkSet {
    inner: Mutex<SinkSetInner>,
}

#[derive(Default)]
struct SinkSetInner {
    next_id: u64,
    ended: bool,
    sinks: Vec<(FrameSinkId, Arc<dyn MediaFrameSink>)>,
}

impl SinkSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a sink. On a stream that already ended the sink gets `ended()` at once and is not
    /// kept; the returned id is then still unique but names nothing.
    pub fn add(&self, sink: Arc<dyn MediaFrameSink>) -> FrameSinkId {
        let (id, already_ended) = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let id = FrameSinkId(inner.next_id);
            inner.next_id += 1;
            if inner.ended {
                (id, true)
            } else {
                inner.sinks.push((id, Arc::clone(&sink)));
                (id, false)
            }
        };
        if already_ended {
            sink.ended();
        }
        id
    }

    /// Removes a registration. Unknown ids are ignored. The removed sink gets no `ended()`: its
    /// owner asked to stop.
    pub fn remove(&self, id: FrameSinkId) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.sinks.retain(|(have, _)| *have != id);
    }

    /// Number of registered sinks.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sinks
            .len()
    }

    /// Whether no sink is registered (the producer can stop converting frames).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether [`SinkSet::end`] has run.
    pub fn has_ended(&self) -> bool {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).ended
    }

    fn snapshot(&self) -> Vec<Arc<dyn MediaFrameSink>> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.ended {
            return Vec::new();
        }
        inner
            .sinks
            .iter()
            .map(|(_, sink)| Arc::clone(sink))
            .collect()
    }

    /// Delivers a video frame to every sink. Nothing is delivered after `end`.
    pub fn video_frame(&self, frame: RawVideoFrame<'_>) {
        for sink in self.snapshot() {
            sink.video_frame(frame);
        }
    }

    /// Delivers an audio block to every sink. Nothing is delivered after `end`.
    pub fn audio_frame(&self, frame: RawAudioFrame<'_>) {
        for sink in self.snapshot() {
            sink.audio_frame(frame);
        }
    }

    /// Ends the stream: every registered sink gets `ended()` once and the set is emptied. Later
    /// calls do nothing. Returns whether this call was the one that ended it.
    pub fn end(&self) -> bool {
        let sinks = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.ended {
                return false;
            }
            inner.ended = true;
            std::mem::take(&mut inner.sinks)
        };
        for (_, sink) in sinks {
            sink.ended();
        }
        true
    }
}

/// Maps timestamps from the producer's clock to the consumer's. One anchor pair fixes the offset;
/// both clocks are assumed to run at the same rate (a spike measures whether that holds over a
/// few minutes before anyone adds drift correction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockMap {
    producer_anchor: Micros,
    consumer_anchor: Micros,
}

impl ClockMap {
    /// Anchors the map: `producer_anchor` on the producer's clock is `consumer_anchor` on the
    /// consumer's.
    pub fn new(producer_anchor: Micros, consumer_anchor: Micros) -> Self {
        Self {
            producer_anchor,
            consumer_anchor,
        }
    }

    /// Converts a producer timestamp (saturating at the ends of the `i64` range).
    pub fn to_consumer(&self, producer: Micros) -> Micros {
        let delta = producer.0.saturating_sub(self.producer_anchor.0);
        Micros(self.consumer_anchor.0.saturating_add(delta))
    }

    /// Converts a producer pipeline time in nanoseconds (GStreamer's unit).
    pub fn to_consumer_from_ns(&self, producer_ns: u64) -> Micros {
        let micros = i64::try_from(producer_ns / 1000).unwrap_or(i64::MAX);
        self.to_consumer(Micros(micros))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn video<'a>(w: u32, h: u32, y: &'a [u8], u: &'a [u8], v: &'a [u8]) -> RawVideoFrame<'a> {
        RawVideoFrame {
            width: w,
            height: h,
            y,
            u,
            v,
            stride_y: w as usize,
            stride_u: chroma_dim(w) as usize,
            stride_v: chroma_dim(w) as usize,
            timestamp: Micros(0),
        }
    }

    #[test]
    fn a_tightly_packed_frame_validates_and_odd_sizes_round_the_chroma_up() {
        let (y, u, v) = (vec![0u8; 5 * 3], vec![0u8; 3 * 2], vec![0u8; 3 * 2]);
        let frame = video(5, 3, &y, &u, &v);
        assert_eq!(frame.validate(), Ok(()));
        assert_eq!(RawVideoFrame::packed_len(5, 3), 15 + 12);
        assert_eq!(chroma_dim(5), 3);
        assert_eq!(chroma_dim(4), 2);
    }

    #[test]
    fn a_short_plane_or_a_small_stride_is_refused_and_the_last_row_may_lack_padding() {
        let (y, u, v) = (vec![0u8; 16], vec![0u8; 4], vec![0u8; 4]);
        let mut frame = video(4, 4, &y, &u, &v);
        assert_eq!(frame.validate(), Ok(()));
        frame.stride_y = 8; // needs 8 * 3 + 4 = 28 bytes
        assert_eq!(
            frame.validate(),
            Err(FrameError::PlaneTooShort {
                plane: "y",
                needed: 28,
                got: 16
            })
        );
        frame.stride_y = 3;
        assert_eq!(
            frame.validate(),
            Err(FrameError::StrideTooSmall {
                plane: "y",
                needed: 4,
                got: 3
            })
        );
        let y = vec![0u8; 16];
        let short_u = vec![0u8; 3];
        let frame = video(4, 4, &y, &short_u, &v);
        assert_eq!(
            frame.validate(),
            Err(FrameError::PlaneTooShort {
                plane: "u",
                needed: 4,
                got: 3
            })
        );
        let empty = video(0, 4, &y, &u, &v);
        assert_eq!(empty.validate(), Err(FrameError::EmptyFrame));
    }

    #[test]
    fn an_audio_block_is_ten_milliseconds_at_48_khz() {
        assert_eq!(AUDIO_SAMPLES_PER_BLOCK, 480);
        let mono = vec![0i16; 480];
        let block = RawAudioFrame {
            samples: &mono,
            channels: 1,
            sample_rate: 48_000,
            timestamp: Micros(0),
        };
        assert_eq!(block.validate(), Ok(()));
        assert_eq!(block.samples_per_channel(), 480);
        let stereo = vec![0i16; 960];
        let block = RawAudioFrame {
            samples: &stereo,
            channels: 2,
            sample_rate: 48_000,
            timestamp: Micros(0),
        };
        assert_eq!(block.validate(), Ok(()));
        assert_eq!(block.samples_per_channel(), 480);
        let wrong = RawAudioFrame {
            samples: &mono,
            channels: 2,
            sample_rate: 48_000,
            timestamp: Micros(0),
        };
        assert_eq!(
            wrong.validate(),
            Err(FrameError::WrongBlockLength {
                expected: 960,
                got: 480
            })
        );
        let rate = RawAudioFrame {
            samples: &mono,
            channels: 1,
            sample_rate: 44_100,
            timestamp: Micros(0),
        };
        assert_eq!(rate.validate(), Err(FrameError::BadAudioFormat));
        let none = RawAudioFrame {
            samples: &mono,
            channels: 0,
            sample_rate: 48_000,
            timestamp: Micros(0),
        };
        assert_eq!(none.validate(), Err(FrameError::BadAudioFormat));
    }

    #[derive(Default)]
    struct Counter {
        video: AtomicUsize,
        audio: AtomicUsize,
        ended: AtomicUsize,
    }

    impl MediaFrameSink for Counter {
        fn video_frame(&self, _: RawVideoFrame<'_>) {
            self.video.fetch_add(1, Ordering::SeqCst);
        }
        fn audio_frame(&self, _: RawAudioFrame<'_>) {
            self.audio.fetch_add(1, Ordering::SeqCst);
        }
        fn ended(&self) {
            self.ended.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn one_frame(set: &SinkSet) {
        let (y, u, v) = (vec![0u8; 4], vec![0u8; 1], vec![0u8; 1]);
        set.video_frame(video(2, 2, &y, &u, &v));
        let block = vec![0i16; 480];
        set.audio_frame(RawAudioFrame {
            samples: &block,
            channels: 1,
            sample_rate: 48_000,
            timestamp: Micros(0),
        });
    }

    #[test]
    fn frames_reach_every_sink_until_it_is_removed() {
        let set = SinkSet::new();
        let (a, b) = (Arc::new(Counter::default()), Arc::new(Counter::default()));
        let id_a = set.add(a.clone());
        set.add(b.clone());
        assert_eq!(set.len(), 2);
        one_frame(&set);
        set.remove(id_a);
        set.remove(id_a);
        one_frame(&set);
        assert_eq!(
            (
                a.video.load(Ordering::SeqCst),
                a.audio.load(Ordering::SeqCst)
            ),
            (1, 1)
        );
        assert_eq!(
            (
                b.video.load(Ordering::SeqCst),
                b.audio.load(Ordering::SeqCst)
            ),
            (2, 2)
        );
        assert_eq!(
            a.ended.load(Ordering::SeqCst),
            0,
            "a removed sink is not told the stream ended"
        );
    }

    #[test]
    fn ending_tells_each_sink_once_and_stops_delivery() {
        let set = SinkSet::new();
        let a = Arc::new(Counter::default());
        set.add(a.clone());
        assert!(set.end());
        assert!(!set.end());
        one_frame(&set);
        assert_eq!(a.ended.load(Ordering::SeqCst), 1);
        assert_eq!(a.video.load(Ordering::SeqCst), 0);
        assert!(set.has_ended());
        assert!(set.is_empty());
    }

    #[test]
    fn a_sink_added_after_the_end_is_told_at_once_and_not_kept() {
        let set = SinkSet::new();
        set.end();
        let late = Arc::new(Counter::default());
        let id = set.add(late.clone());
        assert_eq!(late.ended.load(Ordering::SeqCst), 1);
        assert!(set.is_empty());
        set.remove(id);
        assert_eq!(late.ended.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_sink_may_remove_itself_from_inside_a_callback() {
        struct SelfRemoving {
            set: Arc<SinkSet>,
            id: Mutex<Option<FrameSinkId>>,
            calls: AtomicUsize,
        }
        impl MediaFrameSink for SelfRemoving {
            fn video_frame(&self, _: RawVideoFrame<'_>) {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if let Some(id) = self.id.lock().unwrap().take() {
                    self.set.remove(id);
                }
            }
        }
        let set = Arc::new(SinkSet::new());
        let sink = Arc::new(SelfRemoving {
            set: set.clone(),
            id: Mutex::new(None),
            calls: AtomicUsize::new(0),
        });
        let id = set.add(sink.clone());
        *sink.id.lock().unwrap() = Some(id);
        one_frame(&set);
        one_frame(&set);
        assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
        assert!(set.is_empty());
    }

    #[test]
    fn the_clock_map_shifts_and_saturates() {
        let map = ClockMap::new(Micros(5_000_000), Micros(1_000));
        assert_eq!(map.to_consumer(Micros(5_000_000)), Micros(1_000));
        assert_eq!(map.to_consumer(Micros(5_033_333)), Micros(34_333));
        assert_eq!(
            map.to_consumer(Micros(4_000_000)),
            Micros(1_000 - 1_000_000)
        );
        assert_eq!(map.to_consumer_from_ns(5_033_333_999), Micros(34_333));
        let edge = ClockMap::new(Micros(0), Micros(i64::MAX - 1));
        assert_eq!(edge.to_consumer(Micros(10)), Micros(i64::MAX));
        assert_eq!(edge.to_consumer_from_ns(u64::MAX), Micros(i64::MAX));
    }
}
