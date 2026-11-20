// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The MediaSource model: ready state, duration, end of stream, the source buffer list, the element's
//! `buffered`, `seekable` and ready state, the seeking algorithm and the feed cursors.
//!
//! OWNER: item 3 of lane med-1. Written from the W3C Media Source Extensions text: 2.4 (the
//! MediaSource attributes and methods), 2.4.4 and 2.4.5 (duration change and end of stream), 3.1 (the
//! source buffer methods), 4 (the `HTMLMediaElement` extensions: `buffered`, `seekable`, the ready
//! state and the seeking algorithm). Ladybird's `MediaSource.cpp` and Firefox's `MediaSource.cpp` were
//! read for ideas only; no code of either is copied (brief D1).
//!
//! This is a plain state machine: no thread, no lock, no callback. Every method that changes something
//! the DOM must hear about pushes an [`Outgoing`] on the outbox; the backend takes the outbox after it
//! has released the state lock and delivers it from its worker thread (decisions D8 and D9). The spec's
//! "queue a task to fire an event" is that outbox.
//!
//! Where this deviates from the spec or from the brief it says so at the method: `set_duration(None)`
//! (NaN) is a `TypeError` as the spec has it; a duration set below the highest end time is rounded up
//! to it, and below the highest presentation timestamp is `InvalidState` (the spec's two steps), so the
//! older "truncation removes frames" rule never has a frame to remove; `end_of_stream` with nothing
//! buffered leaves the duration alone.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use crate::api::{
    ElementReadyState, EosError, FeedBatch, FeedState, Limits, MediaSourceReadyState, MseError,
    MseEvent, Ns, SourceBufferId, TimeRange, TrackConfig, TrackKind, TypeSupport, ns_to_secs,
    secs_to_ns,
};
use crate::mime::{ParsedType, parse_type};
use crate::processor::FeedCursor;
use crate::source_buffer::{AppendFailure, Busy, SourceBuffer, Unit};

/// More source buffers than this are refused with `QuotaExceededError`.
pub const MAX_SOURCE_BUFFERS: usize = 16;

/// How many removed source buffers are remembered, so that an id used after removal gives
/// `InvalidStateError` and not `NotFoundError`. Older ones are forgotten.
const MAX_TOMBSTONES: usize = 64;

const MS: Ns = 1_000_000;

/// A playback position this close before the start of a buffered range still counts as inside it (the
/// first frame of a stream often starts a few milliseconds after zero).
const START_FUZZ: Ns = 100 * MS;

/// Decision D13: `HaveFutureData` needs this much (or two frame durations) ahead of the position.
const FUTURE_AHEAD: Ns = 100 * MS;

/// Decision D13: `HaveEnoughData` needs this much ahead of the position (or the end of the stream).
const ENOUGH_AHEAD: Ns = 3_000 * MS;

/// Something for the worker thread to deliver after the state lock is released: an event for the DOM
/// and/or a nudge for the feed's wake closure.
#[derive(Debug)]
pub struct Outgoing {
    pub event: Option<MseEvent>,
    /// `(source buffer, generation)`: the event completes an operation of that buffer and is dropped when
    /// the buffer was aborted or removed since (decision D9: nothing arrives for an aborted append).
    pub guard: Option<(SourceBufferId, u64)>,
    /// Call the feed's wake closure (new frames, a new configuration, or the end of the stream).
    pub wake: bool,
}

/// What `process_slice` found.
#[derive(Debug, PartialEq, Eq)]
pub enum SliceStatus {
    /// The buffer was aborted, removed or the MediaSource closed: drop the rest of the append silently.
    Stale,
    Ok,
    Failed(AppendFailure),
}

/// Where the feed is, per kind of track (decision D2: the feed serves the first source buffer that has a
/// track of the kind).
#[derive(Debug, Default)]
struct FeedPlace {
    position: Ns,
    cursors: [Option<FeedCursor>; 3],
    suppliers: [Option<SourceBufferId>; 3],
    /// The configuration id the consumer last got, so that a cursor restarted by a seek emits a
    /// `Config` item only when the configuration at the new position differs.
    known_config: [Option<u32>; 3],
}

fn kind_index(kind: TrackKind) -> usize {
    match kind {
        TrackKind::Video => 0,
        TrackKind::Audio => 1,
        TrackKind::Text => 2,
    }
}

/// The MediaSource: everything one `MseBackend` knows.
pub struct MediaSource {
    types: Arc<dyn TypeSupport>,
    limits: Limits,
    ready_state: MediaSourceReadyState,
    /// Seconds; NaN until an initialization segment gives one; +Infinity for a live stream.
    duration: f64,
    live_seekable: Option<(Ns, Ns)>,
    position: Ns,
    /// The media element has an error (an append error, or `endOfStream` with an error): the spec's
    /// prepare append step 3 makes `appendBuffer` throw `InvalidStateError` until the MediaSource is
    /// attached again.
    errored: bool,
    buffers: Vec<SourceBuffer>,
    next_id: SourceBufferId,
    clock: u64,
    out: Vec<Outgoing>,
    feed: FeedPlace,
}

impl MediaSource {
    pub fn new(types: Arc<dyn TypeSupport>, limits: Limits) -> Self {
        MediaSource {
            types,
            limits,
            ready_state: MediaSourceReadyState::Closed,
            duration: f64::NAN,
            live_seekable: None,
            position: 0,
            errored: false,
            buffers: Vec::new(),
            next_id: 0,
            clock: 0,
            out: Vec::new(),
            feed: FeedPlace::default(),
        }
    }

    // ---- the outbox ----

    /// Everything pushed since the last call.
    pub fn take_outbox(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.out)
    }

    fn push_event(&mut self, event: MseEvent) {
        self.out.push(Outgoing {
            event: Some(event),
            guard: None,
            wake: false,
        });
    }

    fn push_guarded(&mut self, event: MseEvent, id: SourceBufferId, generation: u64) {
        self.out.push(Outgoing {
            event: Some(event),
            guard: Some((id, generation)),
            wake: false,
        });
    }

    fn push_wake(&mut self) {
        self.out.push(Outgoing {
            event: None,
            guard: None,
            wake: true,
        });
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// `true` while `generation` is still the one the buffer works under: the buffer exists, was not
    /// removed and was not aborted since.
    pub fn is_current(&self, id: SourceBufferId, generation: u64) -> bool {
        self.index(id)
            .is_some_and(|i| !self.buffers[i].removed && self.buffers[i].generation == generation)
    }

    // ---- lookups ----

    fn index(&self, id: SourceBufferId) -> Option<usize> {
        self.buffers.iter().position(|b| b.id == id)
    }

    /// The buffer's index; `NotFoundError` for an id that never existed (or was forgotten).
    fn known(&self, id: SourceBufferId) -> Result<usize, MseError> {
        self.index(id)
            .ok_or_else(|| MseError::NotFound(format!("there is no source buffer {id}")))
    }

    /// The index of a buffer that is in the list: `InvalidStateError` when it was removed.
    fn live_index(&self, id: SourceBufferId) -> Result<usize, MseError> {
        let i = self.known(id)?;
        if self.buffers[i].removed {
            return Err(MseError::InvalidState(format!(
                "the source buffer {id} was removed from its MediaSource"
            )));
        }
        Ok(i)
    }

    fn live(&self) -> impl Iterator<Item = &SourceBuffer> {
        self.buffers.iter().filter(|b| !b.removed)
    }

    /// The spec's `activeSourceBuffers`: buffers that received their initialization segment and have an
    /// audio or video track (a text track is not selected by default).
    fn active(&self) -> impl Iterator<Item = &SourceBuffer> {
        self.live().filter(|b| {
            b.processor.first_init_received()
                && (b.processor.has_track(TrackKind::Video)
                    || b.processor.has_track(TrackKind::Audio))
        })
    }

    pub fn source_buffer_count(&self) -> usize {
        self.live().count()
    }

    pub fn updating(&self, id: SourceBufferId) -> bool {
        self.index(id).is_some_and(|i| self.buffers[i].updating())
    }

    fn any_updating(&self) -> bool {
        self.live().any(SourceBuffer::updating)
    }

    pub fn ready_state(&self) -> MediaSourceReadyState {
        self.ready_state
    }

    /// Ended to open, as `appendBuffer`, `remove`, `changeType` and the attribute setters do. `true` when
    /// it reopened (the DOM compares the ready state before and after to fire `sourceopen`).
    fn reopen_if_ended(&mut self) -> bool {
        if self.ready_state == MediaSourceReadyState::Ended && !self.errored {
            self.ready_state = MediaSourceReadyState::Open;
            return true;
        }
        false
    }

    // ---- MediaSource: attach, detach ----

    /// Attached to a media element: closed to open.
    pub fn open(&mut self) {
        if self.ready_state == MediaSourceReadyState::Closed {
            self.ready_state = MediaSourceReadyState::Open;
            self.errored = false;
        }
    }

    /// Detached: every source buffer is dropped, the duration is NaN, the state is closed.
    pub fn close(&mut self) {
        self.ready_state = MediaSourceReadyState::Closed;
        for i in 0..self.buffers.len() {
            let generation = self.tick();
            self.buffers[i].discard(generation);
        }
        self.prune_tombstones();
        self.duration = f64::NAN;
        self.live_seekable = None;
        self.feed = FeedPlace {
            position: self.feed.position,
            ..FeedPlace::default()
        };
    }

    fn prune_tombstones(&mut self) {
        let removed = self.buffers.iter().filter(|b| b.removed).count();
        if removed > MAX_TOMBSTONES {
            let mut excess = removed - MAX_TOMBSTONES;
            self.buffers.retain(|b| {
                if b.removed && excess > 0 {
                    excess -= 1;
                    false
                } else {
                    true
                }
            });
        }
    }

    // ---- MediaSource: the source buffer list ----

    fn check_decoders(&self, parsed: &ParsedType) -> Result<(), MseError> {
        for codec in &parsed.codecs {
            if !self.types.has_decoder(codec.kind(), *codec) {
                return Err(MseError::NotSupported(format!(
                    "there is no decoder for {codec:?} ({:?})",
                    parsed.original
                )));
            }
        }
        Ok(())
    }

    /// `addSourceBuffer(type)`, in the spec's order: empty type, unsupported type, too many buffers,
    /// not open.
    pub fn add_source_buffer(&mut self, mime: &str) -> Result<SourceBufferId, MseError> {
        if mime.trim().is_empty() {
            return Err(MseError::Type("the type string is empty".into()));
        }
        let parsed = parse_type(mime)?;
        self.check_decoders(&parsed)?;
        if self.source_buffer_count() >= MAX_SOURCE_BUFFERS {
            return Err(MseError::QuotaExceeded(format!(
                "at most {MAX_SOURCE_BUFFERS} source buffers per MediaSource"
            )));
        }
        if self.ready_state != MediaSourceReadyState::Open {
            return Err(MseError::InvalidState(
                "the MediaSource's readyState is not open".into(),
            ));
        }
        let id = self.next_id;
        self.next_id += 1;
        let generation = self.tick();
        self.buffers
            .push(SourceBuffer::new(id, parsed, self.limits, generation));
        Ok(id)
    }

    /// `removeSourceBuffer`: the buffer is dropped (its memory is freed now), any work of the worker on
    /// it becomes stale.
    pub fn remove_source_buffer(&mut self, id: SourceBufferId) -> Result<(), MseError> {
        let i = self.known(id)?;
        if self.buffers[i].removed {
            return Err(MseError::NotFound(format!(
                "the source buffer {id} is not in the list of this MediaSource"
            )));
        }
        let generation = self.tick();
        self.buffers[i].discard(generation);
        self.prune_tombstones();
        Ok(())
    }

    // ---- MediaSource: duration, end of stream, live seekable range ----

    /// `duration`; `None` is NaN.
    pub fn duration(&self) -> Option<f64> {
        (!self.duration.is_nan()).then_some(self.duration)
    }

    fn duration_ns(&self) -> Option<Ns> {
        (!self.duration.is_nan()).then(|| dur_ns(self.duration))
    }

    /// The largest end time over every track buffer of every source buffer.
    fn highest_end(&self) -> Option<Ns> {
        self.live().filter_map(|b| b.processor.highest_end()).max()
    }

    /// An estimate of the largest presentation timestamp of any buffered frame: the largest end time
    /// minus one frame (the processor does not say which frame ends last).
    fn highest_pts(&self) -> Option<Ns> {
        self.live()
            .filter_map(|b| b.highest_end_and_pts().map(|(_, pts)| pts))
            .max()
    }

    /// The spec's duration change algorithm. Below the highest presentation timestamp is
    /// `InvalidStateError` ("duration reductions that would truncate buffered media are disallowed: use
    /// remove() first"); below the highest end time but not below that timestamp the duration is rounded
    /// up to the end time. The event is sent only when the duration really changes.
    fn duration_change(&mut self, new: f64) -> Result<(), MseError> {
        if self.duration == new {
            return Ok(());
        }
        let new_ns = dur_ns(new);
        if let Some(pts) = self.highest_pts() {
            if new_ns < pts {
                return Err(MseError::InvalidState(format!(
                    "the duration {new} is less than the highest presentation timestamp {} of the buffered \
                     frames; remove() the media first",
                    ns_to_secs(pts)
                )));
            }
        }
        let mut new = new;
        if let Some(end) = self.highest_end() {
            if new_ns < end {
                new = ns_to_secs(end);
            }
        }
        if self.duration != new {
            self.duration = new;
            self.push_event(MseEvent::DurationChanged(Some(new)));
        }
        Ok(())
    }

    /// The duration setter. NaN and negative values are `TypeError`, a MediaSource that is not open or
    /// has an updating buffer is `InvalidStateError`. (The brief's wording lets `None` work; the spec
    /// says NaN throws, and the spec is the authority.)
    pub fn set_duration(&mut self, secs: Option<f64>) -> Result<(), MseError> {
        let Some(value) = secs.filter(|v| !v.is_nan()) else {
            return Err(MseError::Type("the duration must not be NaN".into()));
        };
        if value < 0.0 {
            return Err(MseError::Type("the duration must not be negative".into()));
        }
        if self.ready_state != MediaSourceReadyState::Open {
            return Err(MseError::InvalidState(
                "the MediaSource's readyState is not open".into(),
            ));
        }
        if self.any_updating() {
            return Err(MseError::InvalidState("a source buffer is updating".into()));
        }
        self.duration_change(value)
    }

    /// `endOfStream(error)`: the spec's end of stream algorithm. One `SourceEnded` event per transition
    /// into `ended`; without an error the duration becomes the highest end time of the buffered data.
    pub fn end_of_stream(&mut self, error: Option<EosError>) -> Result<(), MseError> {
        if self.ready_state != MediaSourceReadyState::Open {
            return Err(MseError::InvalidState(
                "the MediaSource's readyState is not open".into(),
            ));
        }
        if self.any_updating() {
            return Err(MseError::InvalidState("a source buffer is updating".into()));
        }
        self.ready_state = MediaSourceReadyState::Ended;
        self.push_event(MseEvent::SourceEnded { error });
        match error {
            None => {
                if let Some(end) = self.highest_end() {
                    // Cannot fail: the new duration is at least the highest end time.
                    let _ = self.duration_change(ns_to_secs(end));
                }
            }
            Some(_) => self.errored = true,
        }
        self.push_wake();
        Ok(())
    }

    pub fn set_live_seekable_range(&mut self, start: f64, end: f64) -> Result<(), MseError> {
        if self.ready_state != MediaSourceReadyState::Open {
            return Err(MseError::InvalidState(
                "the MediaSource's readyState is not open".into(),
            ));
        }
        if start.is_nan() || end.is_nan() || start < 0.0 || start > end {
            return Err(MseError::Type(format!(
                "the live seekable range [{start}, {end}] is not 0 <= start <= end"
            )));
        }
        self.live_seekable = Some((secs_to_ns(start), secs_to_ns(end)));
        Ok(())
    }

    pub fn clear_live_seekable_range(&mut self) -> Result<(), MseError> {
        if self.ready_state != MediaSourceReadyState::Open {
            return Err(MseError::InvalidState(
                "the MediaSource's readyState is not open".into(),
            ));
        }
        self.live_seekable = None;
        Ok(())
    }

    // ---- SourceBuffer: attributes ----

    pub fn mode(&self, id: SourceBufferId) -> Result<crate::api::AppendMode, MseError> {
        Ok(self.buffers[self.known(id)?].processor.mode())
    }

    pub fn timestamp_offset(&self, id: SourceBufferId) -> Result<f64, MseError> {
        Ok(self.buffers[self.known(id)?].timestamp_offset)
    }

    pub fn append_window_start(&self, id: SourceBufferId) -> Result<f64, MseError> {
        Ok(self.buffers[self.known(id)?].window_start)
    }

    pub fn append_window_end(&self, id: SourceBufferId) -> Result<f64, MseError> {
        Ok(self.buffers[self.known(id)?].window_end)
    }

    /// Steps 1 to 2 of every attribute setter: removed, updating.
    fn settable(&self, id: SourceBufferId) -> Result<usize, MseError> {
        let i = self.live_index(id)?;
        if self.buffers[i].updating() {
            return Err(MseError::InvalidState(
                "the source buffer is updating".into(),
            ));
        }
        Ok(i)
    }

    /// The `mode` setter: after the checks, ended reopens, then PARSING_MEDIA_SEGMENT is refused.
    pub fn set_mode(
        &mut self,
        id: SourceBufferId,
        mode: crate::api::AppendMode,
    ) -> Result<(), MseError> {
        let i = self.settable(id)?;
        self.reopen_if_ended();
        if self.buffers[i].in_media_segment {
            return Err(MseError::InvalidState(
                "the source buffer is in the middle of a media segment".into(),
            ));
        }
        self.buffers[i].set_mode(mode);
        Ok(())
    }

    /// The `timestampOffset` setter (same order of checks as `mode`).
    pub fn set_timestamp_offset(&mut self, id: SourceBufferId, secs: f64) -> Result<(), MseError> {
        let i = self.settable(id)?;
        if !secs.is_finite() {
            return Err(MseError::Type(format!(
                "timestampOffset must be a finite number, not {secs}"
            )));
        }
        self.reopen_if_ended();
        if self.buffers[i].in_media_segment {
            return Err(MseError::InvalidState(
                "the source buffer is in the middle of a media segment".into(),
            ));
        }
        self.buffers[i].set_timestamp_offset(secs)
    }

    pub fn set_append_window_start(
        &mut self,
        id: SourceBufferId,
        secs: f64,
    ) -> Result<(), MseError> {
        let i = self.settable(id)?;
        self.buffers[i].set_window_start(secs)
    }

    pub fn set_append_window_end(&mut self, id: SourceBufferId, secs: f64) -> Result<(), MseError> {
        let i = self.settable(id)?;
        self.buffers[i].set_window_end(secs)
    }

    // ---- SourceBuffer: append, remove, abort, changeType ----

    /// The synchronous part of `appendBuffer` (the spec's prepare append): removed, updating, the media
    /// element's error, ended to open, coded frame eviction, the buffer full flag. Returns the
    /// generation the worker must carry. When eviction took data a `BufferedChanged` event is pushed
    /// even if the call then fails.
    pub fn begin_append(&mut self, id: SourceBufferId, len: usize) -> Result<u64, MseError> {
        let i = self.settable(id)?;
        if self.errored {
            return Err(MseError::InvalidState(
                "the media element has an error".into(),
            ));
        }
        self.reopen_if_ended();
        let position = self.position;
        let report = self.buffers[i].processor.evict_report(len, position);
        if report.removed.bytes > 0 || report.removed.frames > 0 {
            self.push_event(MseEvent::BufferedChanged { id });
        }
        self.buffers[i].buffer_full = !report.ok;
        if !report.ok {
            return Err(MseError::QuotaExceeded(format!(
                "the source buffer cannot hold {len} more bytes; remove() media it no longer needs"
            )));
        }
        self.buffers[i].busy = Some(Busy::Append);
        Ok(self.buffers[i].generation)
    }

    /// Undoes `begin_append` when the worker could not be started.
    pub fn cancel_append(&mut self, id: SourceBufferId) {
        if let Some(i) = self.index(id) {
            if self.buffers[i].busy == Some(Busy::Append) {
                self.buffers[i].busy = None;
            }
        }
    }

    /// The synchronous part of `remove(start, end)`; `end` may be +Infinity. Returns the generation and
    /// the range in nanoseconds (`None` is +Infinity) for the worker.
    pub fn begin_remove(
        &mut self,
        id: SourceBufferId,
        start: f64,
        end: f64,
    ) -> Result<(u64, Ns, Option<Ns>), MseError> {
        let i = self.settable(id)?;
        if self.duration.is_nan() {
            return Err(MseError::Type(
                "remove() needs a duration, and there is none yet".into(),
            ));
        }
        if start.is_nan() || start < 0.0 || start > self.duration {
            return Err(MseError::Type(format!(
                "the start {start} is not between 0 and the duration {}",
                self.duration
            )));
        }
        if end.is_nan() || end <= start {
            return Err(MseError::Type(format!(
                "the end {end} is not greater than the start {start}"
            )));
        }
        self.reopen_if_ended();
        self.buffers[i].busy = Some(Busy::Remove);
        let end = end.is_finite().then(|| secs_to_ns(end));
        Ok((self.buffers[i].generation, secs_to_ns(start), end))
    }

    pub fn cancel_remove(&mut self, id: SourceBufferId) {
        if let Some(i) = self.index(id) {
            if self.buffers[i].busy == Some(Busy::Remove) {
                self.buffers[i].busy = None;
            }
        }
    }

    /// `abort()`: an append in progress ends silently (the DOM fires `abort` and `updateend` itself and
    /// the worker's result becomes stale), the parser state is reset, the append window is reset. The
    /// range removal algorithm cannot be aborted.
    pub fn abort(&mut self, id: SourceBufferId) -> Result<(), MseError> {
        let i = self.live_index(id)?;
        if self.ready_state != MediaSourceReadyState::Open {
            return Err(MseError::InvalidState(
                "the MediaSource's readyState is not open".into(),
            ));
        }
        if self.buffers[i].busy == Some(Busy::Remove) {
            return Err(MseError::InvalidState(
                "the range removal algorithm is running".into(),
            ));
        }
        if self.buffers[i].busy == Some(Busy::Append) {
            let generation = self.tick();
            self.buffers[i].busy = None;
            self.buffers[i].generation = generation;
        }
        self.buffers[i].abort_reset();
        Ok(())
    }

    /// `changeType(type)`.
    pub fn change_type(&mut self, id: SourceBufferId, mime: &str) -> Result<(), MseError> {
        if mime.trim().is_empty() {
            return Err(MseError::Type("the type string is empty".into()));
        }
        let i = self.settable(id)?;
        let parsed = parse_type(mime)?;
        self.check_decoders(&parsed)?;
        self.reopen_if_ended();
        self.buffers[i].change_type(parsed);
        Ok(())
    }

    // ---- the worker's side of an append and a remove ----

    /// Runs one slice of an append (the worker holds the state lock for the call). Effects on the
    /// MediaSource that the slice caused are applied here: the duration from an initialization
    /// segment (when it is NaN) and from media beyond it, and the events.
    pub fn process_slice(
        &mut self,
        id: SourceBufferId,
        generation: u64,
        data: &[u8],
    ) -> SliceStatus {
        let Some(i) = self.index(id) else {
            return SliceStatus::Stale;
        };
        if self.buffers[i].removed
            || self.buffers[i].generation != generation
            || self.buffers[i].busy != Some(Busy::Append)
        {
            return SliceStatus::Stale;
        }
        let result = self.buffers[i].push_slice(data);
        let mut wake = false;
        for unit in result.units {
            match unit {
                Unit::Init { info, duration, .. } => {
                    if self.duration.is_nan() {
                        let new = duration.map_or(f64::INFINITY, ns_to_secs);
                        // Cannot fail: the buffers hold no frame that the new duration is below.
                        let _ = self.duration_change(new);
                    }
                    self.push_event(MseEvent::InitSegment { id, init: info });
                    wake = true;
                }
                Unit::Frames { accepted } => {
                    wake |= accepted > 0;
                    let group_end = self.buffers[i].processor.group_end_timestamp();
                    self.media_beyond_duration(group_end);
                }
                Unit::SegmentEnd => {}
            }
        }
        if wake {
            self.push_wake();
        }
        match result.failure {
            Some(failure) => SliceStatus::Failed(failure),
            None => SliceStatus::Ok,
        }
    }

    /// "If the media segment contains data beyond the current duration, run the duration change
    /// algorithm with the new duration set to the maximum of the current duration and the group end
    /// timestamp."
    fn media_beyond_duration(&mut self, group_end: Ns) {
        if let Some(current) = self.duration_ns() {
            if current != Ns::MAX && group_end > current {
                // Raising the duration cannot hit the InvalidState check.
                let _ = self.duration_change(ns_to_secs(group_end));
            }
        }
    }

    /// The end of an append: `AppendDone` when `failure` is `None`, else the append error algorithm
    /// (reset parser state, `AppendError`, and for a decode error the end of stream algorithm with the
    /// "decode" error: `SourceEnded`). `updating` is false before the event leaves.
    pub fn finish_append(
        &mut self,
        id: SourceBufferId,
        generation: u64,
        failure: Option<AppendFailure>,
    ) {
        let Some(i) = self.index(id) else { return };
        if self.buffers[i].removed
            || self.buffers[i].generation != generation
            || self.buffers[i].busy != Some(Busy::Append)
        {
            return;
        }
        self.buffers[i].busy = None;
        match failure {
            None => {
                self.push_guarded(MseEvent::AppendDone { id }, id, generation);
                self.push_wake();
            }
            Some(failure) => {
                log::debug!(
                    "mercury-mse: append to source buffer {id} failed: {}",
                    failure.message
                );
                self.buffers[i].reset_parser_state();
                self.push_guarded(
                    MseEvent::AppendError {
                        id,
                        kind: failure.kind,
                    },
                    id,
                    generation,
                );
                self.errored = true;
                if self.ready_state == MediaSourceReadyState::Open {
                    self.ready_state = MediaSourceReadyState::Ended;
                    self.push_event(MseEvent::SourceEnded {
                        error: Some(EosError::Decode),
                    });
                }
                self.push_wake();
            }
        }
    }

    /// The worker's part of `remove`: coded frame removal, then `RemoveDone` with `updating` false.
    pub fn run_remove(&mut self, id: SourceBufferId, generation: u64, start: Ns, end: Option<Ns>) {
        let Some(i) = self.index(id) else { return };
        if self.buffers[i].removed
            || self.buffers[i].generation != generation
            || self.buffers[i].busy != Some(Busy::Remove)
        {
            return;
        }
        let processor = &mut self.buffers[i].processor;
        if catch_unwind(AssertUnwindSafe(|| processor.remove(start, end))).is_err() {
            log::error!("mercury-mse: a panic while removing coded frames");
        }
        self.buffers[i].busy = None;
        self.push_guarded(MseEvent::RemoveDone { id }, id, generation);
    }

    // ---- SourceBuffer: buffered ----

    pub fn buffered(&self, id: SourceBufferId) -> Vec<TimeRange> {
        match self.index(id) {
            Some(i) if !self.buffers[i].removed => {
                self.buffers[i].buffered(self.ready_state == MediaSourceReadyState::Ended)
            }
            _ => Vec::new(),
        }
    }

    // ---- the media element's view ----

    /// `HTMLMediaElement.buffered`: the intersection of the active source buffers' ranges, with the last
    /// range of each extended to the highest end time when the stream has ended.
    pub fn element_buffered(&self) -> Vec<TimeRange> {
        let ended = self.ready_state == MediaSourceReadyState::Ended;
        let per_buffer: Vec<Vec<TimeRange>> = self.active().map(|b| b.buffered(ended)).collect();
        let Some(highest) = per_buffer
            .iter()
            .filter_map(|ranges| ranges.last())
            .map(|r| r.end)
            .max()
        else {
            return Vec::new();
        };
        let mut acc = vec![TimeRange::new(0, highest)];
        for mut ranges in per_buffer {
            if ended {
                if let Some(last) = ranges.last_mut() {
                    last.end = highest;
                }
            }
            acc = intersect(&acc, &ranges);
        }
        acc
    }

    /// `HTMLMediaElement.seekable`.
    pub fn seekable(&self) -> Vec<TimeRange> {
        let Some(duration) = self.duration_ns() else {
            return Vec::new();
        };
        if duration != Ns::MAX {
            return vec![TimeRange::new(0, duration)];
        }
        let buffered = self.element_buffered();
        if let Some((start, end)) = self.live_seekable {
            let start = buffered.first().map_or(start, |r| r.start.min(start));
            let end = buffered.last().map_or(end, |r| r.end.max(end));
            return vec![TimeRange::new(start, end)];
        }
        match buffered.last() {
            Some(last) => vec![TimeRange::new(0, last.end)],
            None => Vec::new(),
        }
    }

    /// Decision D13: `HaveNothing` until every source buffer has its initialization segment,
    /// `HaveMetadata` outside every range of the element's `buffered`, `HaveCurrentData` inside one,
    /// `HaveFutureData` with at least max(100 ms, two frame durations) ahead, `HaveEnoughData` with at
    /// least 3 s ahead or when the stream has ended.
    pub fn element_ready_state(&self, position: f64) -> ElementReadyState {
        if self.live().next().is_none() || self.live().any(|b| !b.processor.first_init_received()) {
            return ElementReadyState::HaveNothing;
        }
        let ended = self.ready_state == MediaSourceReadyState::Ended;
        let pos = secs_to_ns(position);
        let ranges = self.element_buffered();
        let last = ranges.len().saturating_sub(1);
        let Some(range) = ranges.iter().enumerate().find_map(|(i, r)| {
            let end_inclusive = ended && i == last;
            let inside = r.start.saturating_sub(START_FUZZ) <= pos
                && (pos < r.end || (end_inclusive && pos == r.end));
            inside.then_some(r)
        }) else {
            return ElementReadyState::HaveMetadata;
        };
        if ended {
            return ElementReadyState::HaveEnoughData;
        }
        let ahead = range.end.saturating_sub(pos);
        let frame = self
            .active()
            .map(SourceBuffer::frame_duration)
            .max()
            .unwrap_or(0);
        if ahead >= ENOUGH_AHEAD {
            ElementReadyState::HaveEnoughData
        } else if ahead >= FUTURE_AHEAD.max(frame.saturating_mul(2)) {
            ElementReadyState::HaveFutureData
        } else {
            ElementReadyState::HaveCurrentData
        }
    }

    /// The playback position, for eviction and the feed.
    pub fn set_position(&mut self, secs: f64) {
        self.position = secs_to_ns(secs);
    }

    pub fn position(&self) -> Ns {
        self.position
    }

    /// The spec's seeking algorithm, the part that is the MediaSource's: ended becomes open (`true`, the
    /// DOM fires `sourceopen`). Picking the random access point is the feed's: `feed_seek`.
    pub fn seeking(&mut self, secs: f64) -> bool {
        self.position = secs_to_ns(secs);
        self.reopen_if_ended()
    }

    // ---- the feed ----

    /// The source buffer that supplies this kind of track: the first one that has it.
    fn supplier(&self, kind: TrackKind) -> Option<usize> {
        self.buffers
            .iter()
            .position(|b| !b.removed && b.processor.has_track(kind))
    }

    pub fn feed_tracks(&self) -> Vec<TrackConfig> {
        let mut out = Vec::new();
        for kind in TrackKind::ALL {
            if let Some(i) = self.supplier(kind) {
                out.extend(
                    self.buffers[i]
                        .processor
                        .tracks()
                        .into_iter()
                        .filter(|t| t.kind == kind),
                );
            }
        }
        out
    }

    /// Every source buffer that exists has received its first initialization segment.
    pub fn feed_all_initialized(&self) -> bool {
        self.live().next().is_some() && self.live().all(|b| b.processor.first_init_received())
    }

    /// The duration for the pipeline; `None` for NaN and +Infinity.
    pub fn feed_duration(&self) -> Option<Ns> {
        self.duration_ns().filter(|d| *d != Ns::MAX)
    }

    /// The next pull of every kind starts at the random access point at or before `position`.
    pub fn feed_seek(&mut self, position: Ns) {
        self.feed.position = position;
        self.feed.cursors = [None, None, None];
    }

    /// Up to `max` frames of `kind` and the `Config` items in front of them.
    pub fn feed_pull(&mut self, kind: TrackKind, max: usize) -> FeedBatch {
        let ended = self.ready_state == MediaSourceReadyState::Ended;
        let slot = kind_index(kind);
        let Some(i) = self.supplier(kind) else {
            self.feed.cursors[slot] = None;
            self.feed.suppliers[slot] = None;
            return FeedBatch {
                items: Vec::new(),
                state: if ended {
                    FeedState::Ended
                } else {
                    FeedState::WaitingForData
                },
            };
        };
        let id = self.buffers[i].id;
        if self.feed.suppliers[slot] != Some(id) {
            self.feed.suppliers[slot] = Some(id);
            self.feed.cursors[slot] = None;
        }
        let position = self.feed.position;
        let known = self.feed.known_config[slot];
        let processor = &self.buffers[i].processor;
        let cursor = self.feed.cursors[slot]
            .get_or_insert_with(|| processor.cursor_with_config(kind, position, known));
        let mut items = Vec::new();
        let more = processor.read(kind, cursor, max, &mut items);
        if let Some(config) = cursor.config_id() {
            self.feed.known_config[slot] = Some(config);
        }
        let state = if more {
            FeedState::More
        } else if ended && exhausted(&self.buffers[i], kind, cursor, position) {
            FeedState::Ended
        } else {
            FeedState::WaitingForData
        };
        FeedBatch { items, state }
    }
}

/// Everything of this kind that is buffered was served, or (a cursor that has not started because the
/// data for the position never arrived) the position is at or past the end of the data.
fn exhausted(buffer: &SourceBuffer, kind: TrackKind, cursor: &FeedCursor, position: Ns) -> bool {
    let Some(track) = buffer.processor.track_buffer(kind) else {
        return true;
    };
    let Some(last_dts) = track.iter().next_back().map(|(_, stored)| stored.frame.dts) else {
        return true;
    };
    match cursor.last_dts() {
        Some(served) => served >= last_dts,
        None => track.highest_end().is_none_or(|end| position >= end),
    }
}

/// A duration in nanoseconds; +Infinity is `Ns::MAX`.
fn dur_ns(secs: f64) -> Ns {
    if secs.is_infinite() {
        Ns::MAX
    } else {
        secs_to_ns(secs)
    }
}

/// The intersection of two sorted lists of disjoint ranges.
fn intersect(a: &[TimeRange], b: &[TimeRange]) -> Vec<TimeRange> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let start = a[i].start.max(b[j].start);
        let end = a[i].end.min(b[j].end);
        if start < end {
            out.push(TimeRange::new(start, end));
        }
        if a[i].end < b[j].end {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(a: Ns, b: Ns) -> TimeRange {
        TimeRange::new(a, b)
    }

    #[test]
    fn intersection_of_ranges() {
        assert_eq!(
            intersect(&[r(0, 10)], &[r(2, 4), r(6, 12)]),
            vec![r(2, 4), r(6, 10)]
        );
        assert_eq!(intersect(&[r(0, 2)], &[r(2, 4)]), vec![]);
        assert_eq!(intersect(&[], &[r(0, 4)]), vec![]);
        assert_eq!(
            intersect(&[r(0, 4), r(6, 9)], &[r(3, 7)]),
            vec![r(3, 4), r(6, 7)]
        );
    }

    #[test]
    fn durations_in_nanoseconds() {
        assert_eq!(dur_ns(f64::INFINITY), Ns::MAX);
        assert_eq!(dur_ns(8.0), 8_000_000_000);
    }
}
