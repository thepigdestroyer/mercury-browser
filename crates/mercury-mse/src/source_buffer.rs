// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! One SourceBuffer: a byte-stream parser, a [`SourceBufferProcessor`] and the attributes and flags of
//! the spec's append state machine (`updating`, the buffer full flag, the append state, the generation
//! counter that makes a late worker result harmless).
//!
//! OWNER: item 3 of lane med-1. Written from the W3C Media Source Extensions text: 3.1 the SourceBuffer
//! attributes, 3.5.1 the segment parser loop, 3.5.2 reset parser state, 3.5.3 append error, 3.5.7
//! initialization segment received (the parts that belong to one buffer). Ladybird's `SourceBuffer.cpp`
//! and Firefox's `SourceBuffer.cpp` were read for ideas only; no code of either is copied (brief D1).
//!
//! This type knows nothing of the MediaSource it belongs to (ready state, duration, other buffers, the
//! event outbox): `MediaSource` wraps it, does those checks and the duration algorithms, and the backend
//! drives the work from a worker thread. What happens in one slice of an append is
//! [`SourceBuffer::push_slice`].

use crate::api::{
    AppendErrorKind, AppendMode, ByteStreamParser, Container, InitInfo, InitSegment, Limits,
    MseError, Ns, ParseError, ParseEvent, SourceBufferId, TimeRange, TrackInfo, TrackKind,
    ns_to_secs, secs_to_ns,
};
use crate::mime::ParsedType;
use crate::parse::new_parser;
use crate::processor::{ProcessError, SourceBufferProcessor};

/// An append is parsed in slices of this many bytes, so that the worker holds the state lock for one
/// parsed unit (a fragment or a part of one) at a time and a `buffered()` call on the script thread
/// waits at most that long (decision D8). Any chunking of a byte stream gives the same frames, so the
/// slicing is invisible to the page.
pub const APPEND_SLICE: usize = 64 * 1024;

/// What the buffer is busy with. The spec's `updating` is `busy.is_some()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    /// The buffer append algorithm is running (`appendBuffer`).
    Append,
    /// The range removal algorithm is running (`remove`).
    Remove,
}

/// Why an append failed: the kind the DOM is told and a message for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendFailure {
    pub kind: AppendErrorKind,
    pub message: String,
}

impl AppendFailure {
    pub fn decode(message: impl Into<String>) -> Self {
        AppendFailure {
            kind: AppendErrorKind::Decode,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        AppendFailure {
            kind: AppendErrorKind::Internal,
            message: message.into(),
        }
    }

    fn from_parse(error: ParseError) -> Self {
        let kind = match error {
            ParseError::Unsupported(_) | ParseError::TooManyTracks => AppendErrorKind::Unsupported,
            ParseError::Malformed(_)
            | ParseError::MediaBeforeInit
            | ParseError::WrongContainer(_) => AppendErrorKind::Decode,
        };
        AppendFailure {
            kind,
            message: error.to_string(),
        }
    }

    fn from_process(error: ProcessError) -> Self {
        let kind = match error {
            ProcessError::Decode(_) => AppendErrorKind::Decode,
            ProcessError::Unsupported(_) => AppendErrorKind::Unsupported,
        };
        AppendFailure {
            kind,
            message: error.to_string(),
        }
    }
}

/// What the parser's events became once the processor had seen them.
#[derive(Clone, Debug, PartialEq)]
pub enum Unit {
    /// An initialization segment was accepted ("initialization segment received").
    Init {
        info: InitInfo,
        /// The duration the segment declares, in nanoseconds (`None` when absent or zero).
        duration: Option<Ns>,
        /// The track configuration changed (a quality switch inside one source buffer).
        config_changed: bool,
    },
    /// A batch of coded frames went through coded frame processing.
    Frames { accepted: usize },
    /// A media segment ended.
    SegmentEnd,
}

/// The outcome of one slice: what was done before a failure, and the failure if there was one. The
/// units are kept even when the slice fails, because their effects on the processor have happened.
#[derive(Debug, Default)]
pub struct SliceResult {
    pub units: Vec<Unit>,
    pub failure: Option<AppendFailure>,
}

/// One SourceBuffer of a MediaSource.
pub struct SourceBuffer {
    pub id: SourceBufferId,
    /// Removed from the MediaSource (`removeSourceBuffer`, or the MediaSource closed). Its processor
    /// and parser are empty; the attributes are kept so that the getters still answer.
    pub removed: bool,
    /// The type the buffer was created or last changed with.
    pub declared: ParsedType,
    parser: Box<dyn ByteStreamParser>,
    pub processor: SourceBufferProcessor,
    pub busy: Option<Busy>,
    /// Bumped by everything that must make the worker's current result stale (`abort`, removal,
    /// closing); the worker carries the value it started with and drops its result when it differs.
    pub generation: u64,
    /// The spec's buffer full flag: the last eviction could not make room.
    pub buffer_full: bool,
    /// The spec's append state is PARSING_MEDIA_SEGMENT (MP4 only: a WebM Cluster of unknown size ends
    /// when the next element arrives, so the parser cannot say it is between segments).
    pub in_media_segment: bool,
    pub timestamp_offset: f64,
    pub window_start: f64,
    /// `f64::INFINITY` for +Infinity.
    pub window_end: f64,
    video_frame_duration: Ns,
    audio_frame_duration: Ns,
    limits: Limits,
}

impl SourceBuffer {
    pub fn new(id: SourceBufferId, declared: ParsedType, limits: Limits, generation: u64) -> Self {
        let parser = new_parser(declared.container);
        SourceBuffer {
            id,
            removed: false,
            declared,
            parser,
            processor: SourceBufferProcessor::new(limits),
            busy: None,
            generation,
            buffer_full: false,
            in_media_segment: false,
            timestamp_offset: 0.0,
            window_start: 0.0,
            window_end: f64::INFINITY,
            video_frame_duration: 0,
            audio_frame_duration: 0,
            limits,
        }
    }

    /// `updating`.
    pub fn updating(&self) -> bool {
        self.busy.is_some()
    }

    /// Drops everything the buffer holds (it was removed, or its MediaSource closed) and makes any
    /// result of the worker stale.
    pub fn discard(&mut self, generation: u64) {
        self.removed = true;
        self.busy = None;
        self.generation = generation;
        self.buffer_full = false;
        self.in_media_segment = false;
        self.processor = SourceBufferProcessor::new(self.limits);
        self.parser = new_parser(self.declared.container);
    }

    /// The duration of one frame, for the readiness rule of decision D13 and for estimating the
    /// highest presentation timestamp from the highest end time: the last video frame's if there is
    /// one, else the last audio frame's, else 0.
    pub fn frame_duration(&self) -> Ns {
        if self.video_frame_duration > 0 {
            self.video_frame_duration
        } else {
            self.audio_frame_duration
        }
    }

    /// The largest end time over the tracks and an estimate of the largest presentation timestamp (one
    /// frame before it), `None` when nothing is buffered.
    pub fn highest_end_and_pts(&self) -> Option<(Ns, Ns)> {
        let end = self.processor.highest_end()?;
        Some((end, end.saturating_sub(self.frame_duration()).max(0)))
    }

    /// `SourceBuffer.buffered`, with the MediaSource's ended extension.
    pub fn buffered(&self, ended: bool) -> Vec<TimeRange> {
        self.processor.buffered_with(ended)
    }

    // ---- the append state machine ----

    /// Runs one slice of an append through the parser and the processor. The parser and processor
    /// refuse bad input with an error and never panic; the caller wraps this in `catch_unwind` anyway.
    pub fn push_slice(&mut self, data: &[u8]) -> SliceResult {
        let mut result = SliceResult::default();
        let events = match self.parser.push(data) {
            Ok(events) => events,
            Err(error) => {
                result.failure = Some(AppendFailure::from_parse(error));
                return result;
            }
        };
        for event in events {
            match event {
                ParseEvent::Init(init) => {
                    if let Err(failure) = self.init_segment(&init, &mut result.units) {
                        result.failure = Some(failure);
                        return result;
                    }
                }
                ParseEvent::Frames(frames) => {
                    for frame in &frames {
                        match frame.kind {
                            TrackKind::Video => self.video_frame_duration = frame.duration,
                            TrackKind::Audio => self.audio_frame_duration = frame.duration,
                            TrackKind::Text => {}
                        }
                    }
                    match self.processor.process_frames(frames) {
                        Ok(done) => {
                            result.units.push(Unit::Frames {
                                accepted: done.accepted,
                            });
                            self.in_media_segment = self.declared.container == Container::Mp4;
                        }
                        Err(error) => {
                            result.failure = Some(AppendFailure::from_process(error));
                            return result;
                        }
                    }
                }
                ParseEvent::SegmentEnd => {
                    self.processor.end_of_segment();
                    result.units.push(Unit::SegmentEnd);
                    self.in_media_segment = false;
                }
            }
        }
        result
    }

    /// The spec's "initialization segment received", the parts that belong to one buffer: the declared
    /// type must match the tracks (Chrome's rule: a type with codecs promises exactly those kinds), and
    /// the processor creates or checks its track buffers.
    fn init_segment(
        &mut self,
        init: &InitSegment,
        units: &mut Vec<Unit>,
    ) -> Result<(), AppendFailure> {
        if !self.declared.codecs.is_empty() {
            for kind in [TrackKind::Video, TrackKind::Audio] {
                let declared = self.declared.kinds.contains(&kind);
                let present = init.tracks.iter().any(|t| t.kind == kind);
                if declared != present {
                    return Err(AppendFailure::decode(format!(
                        "the initialization segment has {} {} track but the type {:?} {}",
                        if present { "a" } else { "no" },
                        kind.name(),
                        self.declared.original,
                        if declared {
                            "promises one"
                        } else {
                            "does not promise one"
                        }
                    )));
                }
            }
        }
        let outcome = self
            .processor
            .init_segment(init)
            .map_err(AppendFailure::from_process)?;
        self.in_media_segment = false;
        units.push(Unit::Init {
            info: init_info(init, outcome.first),
            duration: init.duration.filter(|d| *d > 0),
            config_changed: outcome.config_changed,
        });
        Ok(())
    }

    /// "Reset parser state" plus the window reset `abort()` does: the parser drops a partial segment
    /// and keeps its initialization configuration, the processor wants a random access point on every
    /// track, `appendWindowStart` is the presentation start time and `appendWindowEnd` +Infinity.
    pub fn abort_reset(&mut self) {
        self.reset_parser_state();
        self.window_start = 0.0;
        self.window_end = f64::INFINITY;
        self.apply_window();
    }

    /// The append error algorithm's step 1, "reset parser state", without touching the window.
    pub fn reset_parser_state(&mut self) {
        self.parser.reset();
        self.processor.reset_parser_state();
        self.in_media_segment = false;
    }

    /// `changeType`: a fresh parser for the new container (the next initialization segment is required
    /// and may be of another container), the processor wants a new first initialization segment.
    pub fn change_type(&mut self, declared: ParsedType) {
        self.parser = new_parser(declared.container);
        self.declared = declared;
        self.processor.change_type();
        self.in_media_segment = false;
    }

    // ---- attributes (value checks only; state checks are the MediaSource's) ----

    pub fn set_mode(&mut self, mode: AppendMode) {
        self.processor.set_mode(mode);
    }

    /// `timestampOffset`: what the page set, or in `Sequence` mode what the coded frame processing
    /// algorithm moved it to (the processor owns the live value; the `f64` the page set is returned
    /// as it was while the processor still agrees with it, so a getter round-trips exactly).
    pub fn timestamp_offset_secs(&self) -> f64 {
        let live = self.processor.timestamp_offset();
        if live == secs_to_ns(self.timestamp_offset) {
            self.timestamp_offset
        } else {
            ns_to_secs(live)
        }
    }

    pub fn set_timestamp_offset(&mut self, secs: f64) -> Result<(), MseError> {
        if !secs.is_finite() {
            return Err(MseError::Type(format!(
                "timestampOffset must be a finite number, not {secs}"
            )));
        }
        self.timestamp_offset = secs;
        self.processor.set_timestamp_offset(secs_to_ns(secs));
        Ok(())
    }

    pub fn set_window_start(&mut self, secs: f64) -> Result<(), MseError> {
        if secs.is_nan() || secs < 0.0 || secs >= self.window_end {
            return Err(MseError::Type(format!(
                "appendWindowStart {secs} must be at least 0 and less than appendWindowEnd {}",
                self.window_end
            )));
        }
        self.window_start = secs;
        self.apply_window();
        Ok(())
    }

    pub fn set_window_end(&mut self, secs: f64) -> Result<(), MseError> {
        if secs.is_nan() || secs <= self.window_start {
            return Err(MseError::Type(format!(
                "appendWindowEnd {secs} must be greater than appendWindowStart {}",
                self.window_start
            )));
        }
        self.window_end = secs;
        self.apply_window();
        Ok(())
    }

    fn apply_window(&mut self) {
        let end = self
            .window_end
            .is_finite()
            .then(|| secs_to_ns(self.window_end));
        self.processor
            .set_append_window(secs_to_ns(self.window_start), end);
    }
}

/// What the DOM learns from an initialization segment.
fn init_info(init: &InitSegment, first: bool) -> InitInfo {
    InitInfo {
        tracks: init
            .tracks
            .iter()
            .map(|t| TrackInfo {
                kind: t.kind,
                id: t.track_id,
                codec: t.codec_string.clone(),
                language: t.language.clone(),
                label: String::new(),
                width: t.width,
                height: t.height,
                channels: t.channels,
                sample_rate: t.sample_rate,
                default: t.default,
            })
            .collect(),
        duration: init.duration.filter(|d| *d > 0).map(ns_to_secs),
        first,
        protected: !init.pssh.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_map_to_the_documented_kinds() {
        assert_eq!(
            AppendFailure::from_parse(ParseError::MediaBeforeInit).kind,
            AppendErrorKind::Decode
        );
        assert_eq!(
            AppendFailure::from_parse(ParseError::TooManyTracks).kind,
            AppendErrorKind::Unsupported
        );
        assert_eq!(
            AppendFailure::from_process(ProcessError::Unsupported("x".into())).kind,
            AppendErrorKind::Unsupported
        );
        assert_eq!(
            AppendFailure::from_process(ProcessError::Decode("x".into())).kind,
            AppendErrorKind::Decode
        );
    }
}
