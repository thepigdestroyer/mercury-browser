// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The sketch of MED-1's backend: everything the DOM's `MediaSource` and `SourceBuffer` need from
//! GstMSE, behind plain Rust types. No `gstreamer_mse` type appears in the public API, so the DOM
//! side (and servo-media's traits) never depend on the binding crate.
//!
//! OWNER: item 1 of lane sp-mse. Implement over `gstreamer-mse` 0.25. Do not copy servo/media#458
//! (MPL-2.0) into this file; read it for ideas only, MED-1 carries it in a file of its own.
//!
//! Every callback GstMSE makes is turned into an [`MseEvent`] pushed to the [`EventLog`] on whatever
//! thread GstMSE used (which is itself a result: record it, do not hide it by marshalling).
//!
//! What the spike has to find out through this API (the "API fit" questions; answers go in R17):
//! * `timestampOffset` is a signed double in the spec; GstMSE takes an unsigned `GstClockTime`.
//! * `duration` may be NaN or +Infinity (live); GstMSE has `GST_CLOCK_TIME_NONE`.
//! * `buffered` per source buffer exists; the element's `buffered`/`seekable` are the DOM's job.
//! * Track lists (`audioTracks`, `videoTracks`) and codec details are not in GstMSE's API.
//! * `appendBuffer` takes a `GstBuffer`: is there a copy from the page's `ArrayBuffer`, and is it safe to free it at once?
//! * `appendWindowEnd = +Infinity`, `remove(start, +Infinity)`.
//! * `QuotaExceededError`: when does GstMSE raise it, and can the page tell how much is buffered?

use gstreamer::ClockTime;

use crate::events::{EventLog, SourceBufferId};

/// `GstMediaSourceError`, plus the failures that are not MSE's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MseError {
    InvalidState(String),
    Type(String),
    NotSupported(String),
    NotFound(String),
    QuotaExceeded(String),
    /// A GError of another domain, or a refusal the rig itself makes (for example a negative offset).
    Other(String),
}

/// `MediaSource.readyState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSourceReadyState {
    Closed,
    Open,
    Ended,
}

/// The argument of `MediaSource.endOfStream()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EosError {
    None,
    Network,
    Decode,
}

/// `SourceBuffer.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendMode {
    Segments,
    Sequence,
}

/// One range of `SourceBuffer.buffered`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    pub start: ClockTime,
    pub end: ClockTime,
}

/// One `MediaSource` attached to one `msesrc`. All methods take `&self` and may be called from any
/// thread, which is what the DOM needs (script thread calls in, GStreamer threads call out); whether
/// GstMSE is safe under that is one of the things the spike measures.
pub struct MseBackend {
    // item 1: the gstreamer_mse::MediaSource, the attached MseSrc, the source buffers by id.
}

impl MseBackend {
    /// Creates a `GstMediaSource` in the `Closed` state and connects its signals to `log`.
    pub fn new(_log: EventLog) -> Result<Self, MseError> {
        todo!("item 1")
    }

    /// `MediaSource.isTypeSupported`, for one type string such as `video/mp4; codecs="avc1.42C01E"`.
    pub fn is_type_supported(_mime: &str) -> bool {
        todo!("item 1")
    }

    /// Attaches to the `msesrc` element that playbin3 made (`source-setup`) or that the rig put in
    /// the pipeline. Also connects `notify::ready-state` and `notify::duration` of the element to the log.
    pub fn attach(&self, _msesrc: &gstreamer::Element) -> Result<(), MseError> {
        todo!("item 1")
    }

    pub fn detach(&self) {
        todo!("item 1")
    }

    pub fn ready_state(&self) -> MediaSourceReadyState {
        todo!("item 1")
    }

    pub fn add_source_buffer(&self, _mime: &str) -> Result<SourceBufferId, MseError> {
        todo!("item 1")
    }

    pub fn remove_source_buffer(&self, _id: SourceBufferId) -> Result<(), MseError> {
        todo!("item 1")
    }

    /// `appendBuffer`: starts the update; `UpdateStart`, `Update`, `UpdateEnd` (or `Error`) follow in the log.
    pub fn append(&self, _id: SourceBufferId, _data: &[u8]) -> Result<(), MseError> {
        todo!("item 1")
    }

    pub fn updating(&self, _id: SourceBufferId) -> bool {
        todo!("item 1")
    }

    pub fn abort(&self, _id: SourceBufferId) -> Result<(), MseError> {
        todo!("item 1")
    }

    /// `SourceBuffer.remove(start, end)`; `end` of `None` is +Infinity.
    pub fn remove(
        &self,
        _id: SourceBufferId,
        _start: ClockTime,
        _end: Option<ClockTime>,
    ) -> Result<(), MseError> {
        todo!("item 1")
    }

    pub fn change_type(&self, _id: SourceBufferId, _mime: &str) -> Result<(), MseError> {
        todo!("item 1")
    }

    /// Signed nanoseconds, as the spec's double in seconds would be. A negative value must come back as
    /// `MseError::NotSupported` or `Other` with the reason if GstMSE cannot take it: say which in R17.
    pub fn set_timestamp_offset(
        &self,
        _id: SourceBufferId,
        _offset_ns: i64,
    ) -> Result<(), MseError> {
        todo!("item 1")
    }

    pub fn set_append_window(
        &self,
        _id: SourceBufferId,
        _start: ClockTime,
        _end: Option<ClockTime>,
    ) -> Result<(), MseError> {
        todo!("item 1")
    }

    pub fn set_append_mode(&self, _id: SourceBufferId, _mode: AppendMode) -> Result<(), MseError> {
        todo!("item 1")
    }

    pub fn buffered(&self, _id: SourceBufferId) -> Result<Vec<TimeRange>, MseError> {
        todo!("item 1")
    }

    pub fn source_buffer_count(&self) -> usize {
        todo!("item 1")
    }

    /// `None` is "unknown" (NaN) or, for a live stream, +Infinity: the spike finds out which.
    pub fn duration(&self) -> Option<ClockTime> {
        todo!("item 1")
    }

    pub fn set_duration(&self, _duration: Option<ClockTime>) -> Result<(), MseError> {
        todo!("item 1")
    }

    pub fn end_of_stream(&self, _error: EosError) -> Result<(), MseError> {
        todo!("item 1")
    }

    pub fn set_live_seekable_range(
        &self,
        _start: ClockTime,
        _end: ClockTime,
    ) -> Result<(), MseError> {
        todo!("item 1")
    }

    pub fn clear_live_seekable_range(&self) -> Result<(), MseError> {
        todo!("item 1")
    }
}
