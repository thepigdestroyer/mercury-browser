// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Item 1 `parse`: incremental byte-stream parsers for fragmented MP4 and WebM.
//!
//! OWNER: item 1 of lane med-1 (brief section 6). The signature of [`new_parser`] is the contract with
//! item 3; the trait it returns is `crate::api::ByteStreamParser`.
//!
//! # How both parsers work
//!
//! Each parser keeps the bytes it has not finished with in a [`ByteQueue`] addressed by absolute stream
//! offset, and a small state machine that is advanced as far as the bytes allow after every `push`. A
//! decision is taken as soon as the bytes it needs are there and never earlier or later, so the sequence
//! of events (and the error, if any) depends on the bytes only and not on how they were cut into
//! `push` calls: one byte at a time gives exactly what one big call gives.
//!
//! * Boxes and elements the parser needs whole (`moov`, `moof`, the EBML header, `Info`, `Tracks`, a
//!   block) are buffered up to [`MAX_BOX_BYTES`]; a larger declared size is refused before any byte is
//!   kept. `mdat` and `Cluster` are consumed incrementally: a sample or block is cut out of the queue as
//!   soon as its last byte has arrived and the bytes before it are dropped.
//! * Input is fed to the state machine in slices of [`FEED_SLICE`], so a huge `push` never makes the
//!   queue hold more than the data the state machine is still waiting on.
//! * Whatever is kept never exceeds [`MAX_RETAINED_BYTES`] plus one slice; a stream that would need more
//!   (sample data far behind or ahead of the box that names it) is refused as `Malformed`.
//! * Time is exact: container ticks convert to [`Ns`] with one rounding, never by accumulating
//!   rounded durations ([`ticks_to_ns`]).
//! * After an error the parser returns that error from every `push` until `reset`; the events that were
//!   produced earlier in the failing `push` are dropped with it (the trait cannot return both).

pub mod bmff;
pub mod config;
pub mod ebml;
pub mod mp4;
pub mod webm;

use std::sync::Arc;

use crate::api::{
    ByteStreamParser, CodedFrame, Container, InitSegment, Ns, ParseError, ParseEvent,
};

/// The largest box or element the parsers buffer or skip (`mdat` and `Cluster` excepted).
pub const MAX_BOX_BYTES: u64 = 64 * 1024 * 1024;

/// The most samples one `moof` may describe (a 4 GiB `sample_count` with no per-sample fields would
/// otherwise allocate without bound).
pub const MAX_SAMPLES_PER_FRAGMENT: usize = 500_000;

/// The most input bytes a parser takes into its queue before it runs its state machine.
pub const FEED_SLICE: usize = 256 * 1024;

/// The most bytes a parser retains between two `push` calls (two maximal boxes).
pub const MAX_RETAINED_BYTES: usize = 2 * MAX_BOX_BYTES as usize;

/// A parser for `container`, ready for its first byte.
pub fn new_parser(container: Container) -> Box<dyn ByteStreamParser> {
    match container {
        Container::Mp4 => Box::new(mp4::Mp4Parser::new()),
        Container::WebM => Box::new(webm::WebmParser::new()),
    }
}

/// The most bytes of one codec configuration record (`avcC`, `vpcC`, `av1C`, the AudioSpecificConfig,
/// `dOps`, a WebM `CodecPrivate`) the parsers keep. Real records are a few hundred bytes (an `avcC` with
/// many parameter sets stays under 2 KB); the track buffer keeps up to 64 configurations and every
/// `Config` the feed serves is copied into caps, so an unbounded record multiplied by them was a
/// memory amplification.
pub const MAX_CODEC_CONFIG_BYTES: usize = 64 * 1024;

/// The most bytes of `pssh` boxes copied out of one initialization segment (they are only kept for
/// EME, which does not exist yet; a real `moov` carries a few hundred bytes of them). The boxes past
/// the cap are dropped, not an error: the stream plays without them.
pub const MAX_PSSH_BYTES: usize = 1024 * 1024;

pub(crate) type PResult<T> = Result<T, ParseError>;

pub(crate) fn malformed(message: impl Into<String>) -> ParseError {
    ParseError::Malformed(message.into())
}

pub(crate) fn unsupported(message: impl Into<String>) -> ParseError {
    ParseError::Unsupported(message.into())
}

/// `ticks` at `timescale` ticks per second, in nanoseconds, rounded to the nearest (halves away from
/// zero). Every conversion starts from the absolute tick count, so no error accumulates over a long
/// stream: a 90 kHz tick and a 48 kHz sample survive the round trip.
pub(crate) fn ticks_to_ns(ticks: i64, timescale: u32) -> PResult<Ns> {
    if timescale == 0 {
        return Err(malformed("a timescale of 0"));
    }
    let ts = i128::from(timescale);
    let n = i128::from(ticks) * 1_000_000_000;
    let ns = if n >= 0 {
        (n + ts / 2) / ts
    } else {
        -((-n + ts / 2) / ts)
    };
    Ns::try_from(ns).map_err(|_| malformed("a timestamp does not fit in 64 bits of nanoseconds"))
}

/// Bytes received and not yet dropped, addressed by absolute stream offset.
#[derive(Debug, Default)]
pub(crate) struct ByteQueue {
    buf: Vec<u8>,
    head: usize,
    /// Absolute offset of `buf[head]`.
    base: u64,
}

impl ByteQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Absolute offset of the first byte held.
    #[cfg(test)]
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Absolute offset just past the last byte received.
    pub fn end(&self) -> u64 {
        self.base + (self.buf.len() - self.head) as u64
    }

    /// Bytes held.
    pub fn len(&self) -> usize {
        self.buf.len() - self.head
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// `[from, to)` when all of it is held.
    pub fn slice(&self, from: u64, to: u64) -> Option<&[u8]> {
        if from < self.base || to > self.end() || from > to {
            return None;
        }
        let a = self.head + (from - self.base) as usize;
        let b = self.head + (to - self.base) as usize;
        self.buf.get(a..b)
    }

    /// Up to `max` bytes from `from`, whatever is held of them.
    pub fn peek(&self, from: u64, max: usize) -> &[u8] {
        if from < self.base || from >= self.end() {
            return &[];
        }
        let a = self.head + (from - self.base) as usize;
        let b = a.saturating_add(max).min(self.buf.len());
        self.buf.get(a..b).unwrap_or(&[])
    }

    /// Drops the bytes before `abs` (and does nothing for an offset already behind the base).
    pub fn discard_before(&mut self, abs: u64) {
        let abs = abs.min(self.end());
        if abs <= self.base {
            return;
        }
        self.head += (abs - self.base) as usize;
        self.base = abs;
        if self.head == self.buf.len() {
            self.buf.clear();
            self.head = 0;
        } else if self.head >= 64 * 1024 && self.head >= self.buf.len() / 2 {
            self.buf.drain(..self.head);
            self.head = 0;
        }
    }

    /// Forgets everything held; the next byte received is at `abs`.
    pub fn clear_to(&mut self, abs: u64) {
        self.buf.clear();
        self.head = 0;
        self.base = abs;
    }
}

/// Collects what one `push` produces, keeping the order `Init`, `Frames`, `SegmentEnd` and merging
/// consecutive frames into one batch.
#[derive(Debug, Default)]
pub(crate) struct Output {
    events: Vec<ParseEvent>,
    frames: Vec<CodedFrame>,
}

impl Output {
    pub fn frame(&mut self, frame: CodedFrame) {
        self.frames.push(frame);
    }

    pub fn init(&mut self, init: InitSegment) {
        self.flush_frames();
        self.events.push(ParseEvent::Init(init));
    }

    pub fn segment_end(&mut self) {
        self.flush_frames();
        self.events.push(ParseEvent::SegmentEnd);
    }

    fn flush_frames(&mut self) {
        if !self.frames.is_empty() {
            self.events
                .push(ParseEvent::Frames(std::mem::take(&mut self.frames)));
        }
    }

    pub fn finish(mut self) -> Vec<ParseEvent> {
        self.flush_frames();
        self.events
    }
}

/// A frame body cut out of a queue.
pub(crate) fn frame_data(bytes: &[u8]) -> Arc<[u8]> {
    Arc::from(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_addresses_bytes_by_absolute_offset() {
        let mut q = ByteQueue::new();
        q.push(&[1, 2, 3, 4, 5]);
        assert_eq!((q.base(), q.end(), q.len()), (0, 5, 5));
        assert_eq!(q.slice(1, 4), Some(&[2u8, 3, 4][..]));
        assert_eq!(q.slice(1, 6), None);
        assert_eq!(q.peek(3, 10), &[4, 5]);
        q.discard_before(2);
        assert_eq!((q.base(), q.end(), q.len()), (2, 5, 3));
        assert_eq!(q.slice(1, 3), None, "bytes behind the base are gone");
        assert_eq!(q.slice(2, 5), Some(&[3u8, 4, 5][..]));
        q.push(&[6]);
        assert_eq!(q.slice(4, 6), Some(&[5u8, 6][..]));
        q.discard_before(100);
        assert_eq!((q.base(), q.end(), q.len()), (6, 6, 0));
        q.clear_to(1000);
        q.push(&[9]);
        assert_eq!((q.base(), q.end()), (1000, 1001));
    }

    #[test]
    fn queue_compacts_without_changing_addresses() {
        let mut q = ByteQueue::new();
        let data: Vec<u8> = (0..=255u8).cycle().take(300_000).collect();
        q.push(&data);
        q.discard_before(200_000);
        assert_eq!(q.len(), 100_000);
        assert_eq!(q.slice(200_000, 200_004), Some(&data[200_000..200_004]));
        assert_eq!(q.peek(299_998, 10), &data[299_998..]);
    }

    #[test]
    fn ticks_convert_exactly_and_round_once() {
        // 90 kHz: one tick is 11,111.1 ns; an hour of them must not drift.
        assert_eq!(ticks_to_ns(90_000, 90_000).unwrap(), 1_000_000_000);
        assert_eq!(
            ticks_to_ns(90_000 * 3600, 90_000).unwrap(),
            3_600_000_000_000
        );
        assert_eq!(ticks_to_ns(1, 90_000).unwrap(), 11_111);
        assert_eq!(ticks_to_ns(2, 90_000).unwrap(), 22_222);
        assert_eq!(ticks_to_ns(3, 90_000).unwrap(), 33_333);
        // 48 kHz: 1024 samples are 21,333,333.33 ns.
        assert_eq!(ticks_to_ns(1024, 48_000).unwrap(), 21_333_333);
        assert_eq!(
            ticks_to_ns(48_000 * 7200, 48_000).unwrap(),
            7_200_000_000_000
        );
        // Halves go away from zero.
        assert_eq!(ticks_to_ns(1, 2_000_000_000).unwrap(), 1);
        assert_eq!(ticks_to_ns(-1, 2_000_000_000).unwrap(), -1);
        assert!(ticks_to_ns(i64::MAX, 1).is_err());
        assert!(ticks_to_ns(5, 0).is_err());
    }

    #[test]
    fn output_keeps_order_and_merges_frames() {
        use crate::api::TrackKind;
        let f = || CodedFrame {
            kind: TrackKind::Video,
            track_id: 1,
            pts: 0,
            dts: 0,
            duration: 1,
            keyframe: true,
            data: Arc::from(&[0u8][..]),
        };
        let mut o = Output::default();
        o.frame(f());
        o.frame(f());
        o.segment_end();
        o.frame(f());
        let events = o.finish();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], ParseEvent::Frames(v) if v.len() == 2));
        assert!(matches!(events[1], ParseEvent::SegmentEnd));
        assert!(matches!(&events[2], ParseEvent::Frames(v) if v.len() == 1));
    }
}
