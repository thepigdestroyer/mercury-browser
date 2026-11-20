// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The coded-frame processor of one source buffer: the spec's coded frame processing, coded frame
//! removal and coded frame eviction algorithms over the track buffers, plus the feed cursor.
//!
//! OWNER: item 2 of lane med-1. **These signatures are the contract with item 3** (committed by the
//! lead as `todo!()`): implement them, keep them, add methods freely. If this file translates Ladybird's
//! `SourceBufferProcessor` or Firefox's `TrackBuffersManager` it carries that source's SPDX line and
//! header (brief D1); it may then be split so that only the translated part does.

use crate::api::CodedFrame;
use crate::api::{
    AppendMode, FeedItem, InitSegment, Limits, Ns, TimeRange, TrackConfig, TrackKind,
};

/// Why the processor refused something; both end the append as a decode error.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProcessError {
    #[error("decode error: {0}")]
    Decode(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// What `init_segment` found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InitOutcome {
    /// `true` for the first initialization segment since creation or since `change_type`.
    pub first: bool,
    /// The codec configuration of at least one track differs from the previous one (the feed will
    /// emit `FeedItem::Config` before the next frame of that track).
    pub config_changed: bool,
}

/// What `process_frames` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Processed {
    pub accepted: usize,
    /// Frames the append window, a missing random access point or the spec's checks dropped.
    pub dropped: usize,
}

/// What `remove` and `evict` freed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Removed {
    pub frames: usize,
    pub bytes: usize,
}

/// Where a feeder is in one track buffer. Opaque; survives removal and re-append between reads.
#[derive(Clone, Debug, Default)]
pub struct FeedCursor {
    _private: (),
}

/// The processing state of one source buffer: its track buffers, its group timestamps, its append
/// window and offset. Not thread safe: item 3 puts it behind its own lock.
#[derive(Debug)]
pub struct SourceBufferProcessor {
    _private: (),
}

impl SourceBufferProcessor {
    pub fn new(limits: Limits) -> Self {
        let _ = limits;
        todo!("item 2: the processor")
    }

    // ---- the SourceBuffer attributes the processing reads ----

    pub fn mode(&self) -> AppendMode {
        todo!("item 2")
    }

    /// In `Sequence` mode the spec sets the group start timestamp to the group end timestamp.
    pub fn set_mode(&mut self, mode: AppendMode) {
        let _ = mode;
        todo!("item 2")
    }

    pub fn timestamp_offset(&self) -> Ns {
        todo!("item 2")
    }

    pub fn set_timestamp_offset(&mut self, offset: Ns) {
        let _ = offset;
        todo!("item 2")
    }

    /// `(start, end)`; `end == None` is +Infinity.
    pub fn append_window(&self) -> (Ns, Option<Ns>) {
        todo!("item 2")
    }

    pub fn set_append_window(&mut self, start: Ns, end: Option<Ns>) {
        let _ = (start, end);
        todo!("item 2")
    }

    // ---- from the parser ----

    /// "Initialization segment received", the processor's part: the first one creates the track
    /// buffers; later ones must have the same track kinds; a changed configuration is allowed and
    /// recorded for the feed.
    pub fn init_segment(&mut self, init: &InitSegment) -> Result<InitOutcome, ProcessError> {
        let _ = init;
        todo!("item 2")
    }

    /// The coded frame processing algorithm over frames in decode order, with container times.
    pub fn process_frames(&mut self, frames: Vec<CodedFrame>) -> Result<Processed, ProcessError> {
        let _ = frames;
        todo!("item 2")
    }

    /// A media segment ended.
    pub fn end_of_segment(&mut self) {
        todo!("item 2")
    }

    /// "Reset parser state" (`abort()`, an append error): need-random-access-point on every track;
    /// in `Sequence` mode the group start becomes the group end.
    pub fn reset_parser_state(&mut self) {
        todo!("item 2")
    }

    /// `changeType`: the next initialization segment is a first one again.
    pub fn change_type(&mut self) {
        todo!("item 2")
    }

    // ---- removal, eviction, quota ----

    /// "Coded frame removal" for `[start, end)`; `end == None` is +Infinity. Ripples to the next
    /// random access point.
    pub fn remove(&mut self, start: Ns, end: Option<Ns>) -> Removed {
        let _ = (start, end);
        todo!("item 2")
    }

    /// "Coded frame eviction" (brief D6) to make room for `incoming_bytes` with the playback at
    /// `position`. `false` means there is still no room: the caller sets the buffer-full flag.
    pub fn evict(&mut self, incoming_bytes: usize, position: Ns) -> bool {
        let _ = (incoming_bytes, position);
        todo!("item 2")
    }

    /// Coded-frame bytes held, over all tracks.
    pub fn bytes(&self) -> usize {
        todo!("item 2")
    }

    // ---- queries ----

    /// `SourceBuffer.buffered`: for several tracks the intersection the spec describes.
    pub fn buffered(&self) -> Vec<TimeRange> {
        todo!("item 2")
    }

    pub fn track_buffered(&self, kind: TrackKind) -> Vec<TimeRange> {
        let _ = kind;
        todo!("item 2")
    }

    /// The highest end timestamp over the tracks, `None` when nothing is buffered.
    pub fn highest_end(&self) -> Option<Ns> {
        todo!("item 2")
    }

    /// The current configuration of each track.
    pub fn tracks(&self) -> Vec<TrackConfig> {
        todo!("item 2")
    }

    // ---- the feed ----

    /// A cursor at the random access point of `kind` at or before `position`.
    pub fn cursor(&self, kind: TrackKind, position: Ns) -> FeedCursor {
        let _ = (kind, position);
        todo!("item 2")
    }

    /// Appends up to `max` items (frames and the `Config` items in front of them) to `out`, in decode
    /// order, advancing `cursor`. `true` when more can be read now.
    pub fn read(
        &self,
        kind: TrackKind,
        cursor: &mut FeedCursor,
        max: usize,
        out: &mut Vec<FeedItem>,
    ) -> bool {
        let _ = (kind, cursor, max, out);
        todo!("item 2")
    }
}
