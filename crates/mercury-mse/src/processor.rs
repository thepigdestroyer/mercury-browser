// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The coded-frame processor of one source buffer: the spec's coded frame processing, coded frame
//! removal and coded frame eviction algorithms over the track buffers, plus the feed cursor.
//!
//! OWNER: item 2 of lane med-1. **These signatures are the contract with item 3**: the committed ones
//! are implemented unchanged; the methods and types below the "additions" markers are new and free to
//! use.
//!
//! Written from the W3C Media Source Extensions text (sections 5.5.2 reset parser state, 5.5.7
//! initialization segment received, 5.5.8 coded frame processing, 5.5.9 coded frame removal, 5.5.10 coded
//! frame eviction, and the `buffered` attribute of 3.1). The file is Mercury's own and no code was
//! translated from another project. Design references only, read while it was written: Ladybird's
//! `SourceBufferProcessor` (BSD-2-Clause, `Libraries/LibMedia/MediaSourceExtensions`, read at master
//! `cb3f4fafb851ded99295bcb2675044984ad38ca4` of 20 Nov 2026; the lane brief names the earlier master
//! `a390d9f1a0c0e8269cece5838e67e7c271576e7e` of the same day, which was not the one read), Firefox's
//! `TrackBuffersManager` (MPL-2.0), and Chrome's published per-kind quotas (12 MiB audio, 150 MiB
//! video, which are numbers, not expression). What the references gave: when the "last frame of the
//! group was removed" reset applies (w3c/media-source issue 290), the order of eviction (behind the
//! position first, then beyond it), and the one-frame fuzz of buffered ranges. The structure, data
//! structures (GOP-keyed `TrackBuffer`, the append frontier, the feed cursor) and control flow differ.
//! The file stays under Mercury's licence (brief D1), and `THIRD-PARTY-NOTICES.md` carries no entry for
//! it.
//!
//! # Times
//!
//! Everything is integer nanoseconds ([`Ns`]) and saturating, so no input can overflow. Frames come in
//! with container times and are stored on the presentation timeline (offset and mode applied).
//!
//! # Numbers published for the lane (brief D6; the integrator copies them to the report)
//!
//! * quota per track buffer, from [`Limits`]: video 150 MiB, audio 12 MiB, text 1 MiB of coded-frame
//!   bytes (the numbers Chrome uses); a source buffer's capacity is the sum over its tracks;
//! * eviction never removes data at or after `position - keep_behind` (5 s), and takes from the tail only
//!   data at or after `position + keep_ahead` (30 s) that is also after the ranges that hold the playback
//!   position and the most recent append (the page's frontier: Firefox never evicts from the interval in
//!   use either); both go a whole group of pictures at a time, so what stays decodes;
//! * a seek to the start of the media (position 0 or less) may land up to 1 s before the first frame
//!   (B-frame streams start a frame or two late; Chrome allows the same), any other position needs a
//!   frame within one gap tolerance;
//! * the gap hidden in a track's buffered ranges: the longest frame duration seen in the track, at most
//!   250 ms (Firefox's fuzz is the same, half a frame each side);
//! * the append window and the 1 microsecond rounding allowance: a frame is dropped for starting before
//!   `appendWindowStart` or ending after `appendWindowEnd` only when it is outside by more than
//!   [`WINDOW_ROUNDING`] (1 microsecond), because the window is a double in seconds and a frame time
//!   comes from a tick count, so the same instant can differ by a nanosecond or two.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::api::CodedFrame;
use crate::api::{
    AppendMode, FRAME_OVERHEAD_BYTES, FeedItem, InitSegment, Limits, MemoryBudget, Ns, TimeRange,
    TrackConfig, TrackKind,
};
use crate::ranges;
use crate::trackbuffer::{FrameKey, RemovedInfo, TrackBuffer};

/// A frame is outside the append window only when it is more than this far outside (1 microsecond).
pub const WINDOW_ROUNDING: Ns = 1_000;

/// A new frame replaces an overlapped video frame when it starts within this long of its start (the
/// spec's "1 microsecond" tolerance in the coded frame processing algorithm).
pub const OVERLAP_ROUNDING: Ns = 1_000;

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
///
/// A cursor is *unstarted* until its first read finds the frame to start from (the random access point
/// at or before the position); until the data for the position has arrived, reads return nothing and
/// the cursor keeps waiting. Once started it holds the last frame served, so removing or replacing
/// frames never makes it point at stale data: the next read continues after that frame's decode
/// timestamp, stops at a gap, and restarts at a random access point (flagging `discont`) when the
/// frames that follow belong to another group of pictures than the one being served.
#[derive(Clone, Debug, Default)]
pub struct FeedCursor {
    position: Ns,
    served: Option<Served>,
    config: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
struct Served {
    key: FrameKey,
    /// Decode timestamp plus duration of the last frame served.
    dts_end: Ns,
    gop: u64,
}

impl FeedCursor {
    /// An unstarted cursor that will start at the random access point at or before `position`. The
    /// first frame it serves is preceded by a `Config` item.
    pub fn new(position: Ns) -> Self {
        FeedCursor {
            position,
            served: None,
            config: None,
        }
    }

    /// Restarts the cursor at `position` (a seek). The configuration the consumer already has is
    /// remembered, so a `Config` item is emitted only when the new position needs another one.
    pub fn seek(&mut self, position: Ns) {
        self.position = position;
        self.served = None;
    }

    /// The position the cursor was created or last sought at.
    pub fn position(&self) -> Ns {
        self.position
    }

    /// `true` once a frame has been served since the cursor was created or sought.
    pub fn is_started(&self) -> bool {
        self.served.is_some()
    }

    /// The decode timestamp of the last frame served.
    pub fn last_dts(&self) -> Option<Ns> {
        self.served.map(|s| s.key.0)
    }

    /// The id of the codec configuration the consumer was last given.
    pub fn config_id(&self) -> Option<u32> {
        self.config
    }
}

/// What `evict_report` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvictReport {
    /// `true` when there is room for the incoming bytes now; `false` means the caller sets the spec's
    /// buffer full flag.
    pub ok: bool,
    pub removed: Removed,
}

/// The processing state of one source buffer: its track buffers, its group timestamps, its append
/// window and offset. Not thread safe: item 3 puts it behind its own lock.
#[derive(Debug)]
pub struct SourceBufferProcessor {
    limits: Limits,
    mode: AppendMode,
    timestamp_offset: Ns,
    window_start: Ns,
    window_end: Option<Ns>,
    group_start: Option<Ns>,
    group_end: Ns,
    generate_timestamps: bool,
    first_init: bool,
    pending_init: bool,
    tracks: BTreeMap<TrackKind, TrackBuffer>,
    /// The memory budget shared with the other source buffers (see [`MemoryBudget`]).
    budget: Option<Arc<MemoryBudget>>,
}

impl SourceBufferProcessor {
    pub fn new(limits: Limits) -> Self {
        SourceBufferProcessor {
            budget: None,
            limits,
            mode: AppendMode::Segments,
            timestamp_offset: 0,
            window_start: 0,
            window_end: None,
            group_start: None,
            group_end: 0,
            generate_timestamps: false,
            first_init: false,
            pending_init: false,
            tracks: BTreeMap::new(),
        }
    }

    // ---- the SourceBuffer attributes the processing reads ----

    pub fn mode(&self) -> AppendMode {
        self.mode
    }

    /// In `Sequence` mode the spec sets the group start timestamp to the group end timestamp.
    pub fn set_mode(&mut self, mode: AppendMode) {
        if mode == AppendMode::Sequence {
            self.group_start = Some(self.group_end);
        }
        self.mode = mode;
    }

    pub fn timestamp_offset(&self) -> Ns {
        self.timestamp_offset
    }

    /// In `Sequence` mode the spec also sets the group start timestamp to the new offset.
    pub fn set_timestamp_offset(&mut self, offset: Ns) {
        if self.mode == AppendMode::Sequence {
            self.group_start = Some(offset);
        }
        self.timestamp_offset = offset;
    }

    /// `(start, end)`; `end == None` is +Infinity.
    pub fn append_window(&self) -> (Ns, Option<Ns>) {
        (self.window_start, self.window_end)
    }

    pub fn set_append_window(&mut self, start: Ns, end: Option<Ns>) {
        self.window_start = start;
        self.window_end = end;
    }

    // ---- from the parser ----

    /// "Initialization segment received", the processor's part: the first one creates the track
    /// buffers; later ones must have the same track kinds; a changed configuration is allowed and
    /// recorded for the feed.
    pub fn init_segment(&mut self, init: &InitSegment) -> Result<InitOutcome, ProcessError> {
        if init.tracks.is_empty() {
            return Err(ProcessError::Decode(
                "the initialization segment has no audio, video or text track".into(),
            ));
        }
        let mut seen = [false; 3];
        for t in &init.tracks {
            let slot = &mut seen[kind_index(t.kind)];
            if *slot {
                return Err(ProcessError::Unsupported(format!(
                    "more than one {} track in the initialization segment",
                    t.kind.name()
                )));
            }
            *slot = true;
        }
        let first = !self.first_init || self.pending_init;
        if self.tracks.is_empty() {
            for t in &init.tracks {
                let mut buffer = TrackBuffer::new(t.kind, t.clone());
                if let Some(budget) = &self.budget {
                    buffer.set_budget(Arc::clone(budget));
                }
                self.tracks.insert(t.kind, buffer);
            }
            self.first_init = true;
            self.pending_init = false;
            return Ok(InitOutcome {
                first: true,
                config_changed: false,
            });
        }
        // The spec's checks when the first initialization segment was already received: the same
        // tracks (one per kind here, so the same kinds).
        if init.tracks.len() != self.tracks.len()
            || init
                .tracks
                .iter()
                .any(|t| !self.tracks.contains_key(&t.kind))
        {
            return Err(ProcessError::Decode(
                "the tracks of the initialization segment differ from the first one".into(),
            ));
        }
        let mut config_changed = false;
        for t in &init.tracks {
            if let Some(tb) = self.tracks.get_mut(&t.kind) {
                config_changed |= tb.set_config(t.clone());
            }
        }
        for tb in self.tracks.values_mut() {
            tb.need_random_access_point = true;
        }
        self.first_init = true;
        self.pending_init = false;
        Ok(InitOutcome {
            first,
            config_changed,
        })
    }

    /// The coded frame processing algorithm over frames in decode order, with container times.
    pub fn process_frames(&mut self, frames: Vec<CodedFrame>) -> Result<Processed, ProcessError> {
        if self.tracks.is_empty() || self.pending_init {
            return Err(ProcessError::Decode(
                "a media segment came before the initialization segment".into(),
            ));
        }
        let mut out = Processed::default();
        for frame in frames {
            if self.process_frame(frame) {
                out.accepted += 1;
            } else {
                out.dropped += 1;
            }
        }
        Ok(out)
    }

    /// One coded frame through the spec's algorithm; `true` when it was added to a track buffer.
    fn process_frame(&mut self, frame: CodedFrame) -> bool {
        let kind = frame.kind;
        if !self.tracks.contains_key(&kind) {
            return false;
        }
        let duration = frame.duration.max(0);
        let (mut pts, mut dts);
        // 1. Loop Top: the discontinuity step jumps back here.
        loop {
            (pts, dts) = if self.generate_timestamps {
                (0, 0)
            } else {
                (frame.pts, frame.dts)
            };
            // 3. Sequence mode with a group start timestamp: move the offset so that this frame starts there.
            if self.mode == AppendMode::Sequence
                && let Some(group_start) = self.group_start
            {
                self.timestamp_offset = group_start.saturating_sub(pts);
                self.group_end = group_start;
                for tb in self.tracks.values_mut() {
                    tb.need_random_access_point = true;
                }
                self.group_start = None;
            }
            // 4. Apply timestampOffset.
            if self.timestamp_offset != 0 {
                pts = pts.saturating_add(self.timestamp_offset);
                dts = dts.saturating_add(self.timestamp_offset);
            }
            // 6. A decode timestamp that goes back, or jumps by more than twice the last frame
            //    duration, starts a new coded frame group.
            let Some(tb) = self.tracks.get(&kind) else {
                return false;
            };
            let discontinuity = tb.last_decode_timestamp.is_some_and(|last| {
                dts < last
                    || dts.saturating_sub(last)
                        > tb.last_frame_duration.unwrap_or(0).saturating_mul(2)
            });
            if !discontinuity {
                break;
            }
            match self.mode {
                AppendMode::Segments => self.group_end = pts,
                AppendMode::Sequence => self.group_start = Some(self.group_end),
            }
            for tb in self.tracks.values_mut() {
                tb.unset_group_state();
                tb.need_random_access_point = true;
            }
        }
        // 7. Frame end timestamp.
        let frame_end = pts.saturating_add(duration);
        let window_start = self.window_start;
        let window_end = self.window_end;
        let Some(tb) = self.tracks.get_mut(&kind) else {
            return false;
        };
        // 8. Before the append window: drop it and wait for a random access point.
        if pts < window_start.saturating_sub(WINDOW_ROUNDING) {
            tb.need_random_access_point = true;
            return false;
        }
        // 9. After the append window.
        if window_end.is_some_and(|end| frame_end > end.saturating_add(WINDOW_ROUNDING)) {
            tb.need_random_access_point = true;
            return false;
        }
        // 10. A track buffer that needs a random access point drops everything else.
        if tb.need_random_access_point {
            if !frame.keyframe {
                return false;
            }
            tb.need_random_access_point = false;
        }
        // 13. Video: a frame that starts within a microsecond of the frame it overlaps replaces it.
        if kind == TrackKind::Video
            && tb.last_decode_timestamp.is_none()
            && let Some(key) = tb.frame_covering(pts)
        {
            let overlapped_pts = tb.get(&key).map_or(Ns::MAX, |sf| sf.frame.pts);
            if pts < overlapped_pts.saturating_add(OVERLAP_ROUNDING) {
                tb.remove_frame_with_dependents(&key);
            }
        }
        // 14. Remove the existing frames this one replaces; 15. and the frames that depended on them.
        let replaced = match tb.highest_end_timestamp {
            None => Some((pts, frame_end)),
            Some(highest) if highest <= pts => Some((highest, frame_end)),
            Some(_) => None,
        };
        if let Some((lo, hi)) = replaced
            && lo < hi
        {
            tb.remove_with_dependents(lo, Some(hi));
        }
        // 16. Add the frame.
        tb.insert(CodedFrame {
            pts,
            dts,
            duration,
            ..frame
        });
        // 17. to 19.
        tb.last_decode_timestamp = Some(dts);
        tb.last_frame_duration = Some(duration);
        if tb.highest_end_timestamp.is_none_or(|h| frame_end > h) {
            tb.highest_end_timestamp = Some(frame_end);
        }
        // 20.
        if frame_end > self.group_end {
            self.group_end = frame_end;
        }
        // 21.
        if self.generate_timestamps {
            self.timestamp_offset = frame_end;
        }
        true
    }

    /// A media segment ended. The spec's algorithms keep their group state across segments (a gap
    /// between segments is found by the discontinuity check of the next frame), so there is nothing to
    /// do; item 3 calls it for symmetry with the parser's `SegmentEnd`.
    pub fn end_of_segment(&mut self) {}

    /// "Reset parser state" (`abort()`, an append error): need-random-access-point on every track;
    /// in `Sequence` mode the group start becomes the group end.
    pub fn reset_parser_state(&mut self) {
        for tb in self.tracks.values_mut() {
            tb.unset_group_state();
            tb.need_random_access_point = true;
        }
        if self.mode == AppendMode::Sequence {
            self.group_start = Some(self.group_end);
        }
    }

    /// `changeType`: the next initialization segment is a first one again. The spec runs "reset parser
    /// state" in the same step.
    pub fn change_type(&mut self) {
        self.reset_parser_state();
        self.pending_init = true;
    }

    // ---- removal, eviction, quota ----

    /// "Coded frame removal" for `[start, end)`; `end == None` is +Infinity. Ripples to the next
    /// random access point.
    pub fn remove(&mut self, start: Ns, end: Option<Ns>) -> Removed {
        let mut total = Removed::default();
        for kind in TrackKind::ALL {
            let Some(tb) = self.tracks.get_mut(&kind) else {
                continue;
            };
            let info = tb.remove_range(start, end);
            total.frames += info.frames;
            total.bytes += info.bytes;
            if let Some(pts) = info.last_dts_hit {
                self.last_frame_removed(pts);
            }
        }
        total
    }

    /// The spec's removal step 3.1 to 3.5: a removed frame had the track's last decode timestamp, so the
    /// group is over.
    fn last_frame_removed(&mut self, pts: Ns) {
        match self.mode {
            AppendMode::Segments => self.group_end = pts,
            AppendMode::Sequence => self.group_start = Some(self.group_end),
        }
        for tb in self.tracks.values_mut() {
            tb.unset_group_state();
            tb.need_random_access_point = true;
        }
    }

    /// "Coded frame eviction" (brief D6) to make room for `incoming_bytes` with the playback at
    /// `position`. `false` means there is still no room: the caller sets the buffer-full flag.
    pub fn evict(&mut self, incoming_bytes: usize, position: Ns) -> bool {
        self.evict_report(incoming_bytes, position).ok
    }

    // ---- queries ----

    /// Coded-frame bytes held, over all tracks.
    pub fn bytes(&self) -> usize {
        self.tracks.values().map(TrackBuffer::bytes).sum()
    }

    /// `SourceBuffer.buffered`: for several tracks the intersection the spec describes.
    pub fn buffered(&self) -> Vec<TimeRange> {
        self.buffered_with(false)
    }

    pub fn track_buffered(&self, kind: TrackKind) -> Vec<TimeRange> {
        self.tracks
            .get(&kind)
            .map(TrackBuffer::coalesced_ranges)
            .unwrap_or_default()
    }

    /// The highest end timestamp over the tracks, `None` when nothing is buffered.
    pub fn highest_end(&self) -> Option<Ns> {
        self.tracks
            .values()
            .filter_map(TrackBuffer::highest_end)
            .max()
    }

    /// The current configuration of each track.
    pub fn tracks(&self) -> Vec<TrackConfig> {
        self.tracks
            .values()
            .map(|tb| tb.current_config().clone())
            .collect()
    }

    // ---- the feed ----

    /// A cursor at the random access point of `kind` at or before `position`.
    pub fn cursor(&self, kind: TrackKind, position: Ns) -> FeedCursor {
        let _ = kind;
        FeedCursor::new(position)
    }

    /// Appends up to `max` frames (and the `Config` items in front of them, which do not count) to
    /// `out`, in decode order, advancing `cursor`. `true` when more can be read now.
    ///
    /// A frame is `discont` when it is not the successor of the previous one: the first after the cursor
    /// started or was sought, and after a restart at a random access point (the frames after the cursor
    /// were replaced by another group of pictures; timestamps may then go backwards). The read stops
    /// at a gap larger than the track's gap tolerance, and starts only when the data for the position
    /// has arrived.
    pub fn read(
        &self,
        kind: TrackKind,
        cursor: &mut FeedCursor,
        max: usize,
        out: &mut Vec<FeedItem>,
    ) -> bool {
        let Some(tb) = self.tracks.get(&kind) else {
            return false;
        };
        let tol = tb.gap_tolerance();
        let mut served = 0usize;
        while served < max {
            // Which frame comes next, and does it follow the last one?
            let (mut key, mut discont) = match cursor.served {
                None => match tb.locate(cursor.position) {
                    Some(key) => (key, true),
                    None => return false,
                },
                Some(last) => match tb.next_after(&last.key) {
                    Some((key, sf)) => {
                        if sf.frame.dts.saturating_sub(last.dts_end) > tol {
                            return false;
                        }
                        (*key, false)
                    }
                    None => return false,
                },
            };
            let Some(mut sf) = tb.get(&key) else {
                return false;
            };
            if let Some(last) = cursor.served
                && !sf.frame.keyframe
                && sf.gop != last.gop
            {
                // The frames after the cursor are another group of pictures than the one being
                // served (they replaced it): restart at that group's random access point, or at the
                // next one when it is gone.
                let restart = tb
                    .find_gop_start(&key, sf.gop)
                    .or_else(|| tb.next_keyframe_after(&key));
                let Some(restart) = restart else {
                    return false;
                };
                let Some(restart_frame) = tb.get(&restart) else {
                    return false;
                };
                key = restart;
                sf = restart_frame;
                discont = true;
            }
            if cursor.config != Some(sf.config) {
                if let Some(config) = tb.config_by_id(sf.config) {
                    out.push(FeedItem::Config(config.clone()));
                }
                cursor.config = Some(sf.config);
            }
            out.push(FeedItem::Frame {
                frame: sf.frame.clone(),
                discont,
            });
            cursor.served = Some(Served {
                key,
                dts_end: sf.frame.dts.saturating_add(sf.frame.duration),
                gop: sf.gop,
            });
            served += 1;
        }
        // Is there more right now?
        match cursor.served {
            None => tb.locate(cursor.position).is_some(),
            Some(last) => tb
                .next_after(&last.key)
                .is_some_and(|(_, sf)| sf.frame.dts.saturating_sub(last.dts_end) <= tol),
        }
    }

    // ---- additions: state ----

    /// The quota numbers this processor evicts against.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
    }

    /// The spec's "generate timestamps flag" (byte streams without timestamps); item 3 sets it from the
    /// byte stream format. When true every frame is placed at the previous frame's end.
    pub fn set_generate_timestamps(&mut self, on: bool) {
        self.generate_timestamps = on;
    }

    pub fn generate_timestamps(&self) -> bool {
        self.generate_timestamps
    }

    /// The spec's "first initialization segment received flag".
    pub fn first_init_received(&self) -> bool {
        self.first_init
    }

    /// The spec's "pending initialization segment for changeType flag".
    pub fn pending_init_segment(&self) -> bool {
        self.pending_init
    }

    /// The spec's group start timestamp (`None` is unset).
    pub fn group_start_timestamp(&self) -> Option<Ns> {
        self.group_start
    }

    /// The spec's group end timestamp: item 3 raises the MediaSource duration to it after a media
    /// segment ("If the media segment contains data beyond the current duration").
    pub fn group_end_timestamp(&self) -> Ns {
        self.group_end
    }

    /// `true` when the source buffer has a track buffer of this kind.
    pub fn has_track(&self, kind: TrackKind) -> bool {
        self.tracks.contains_key(&kind)
    }

    /// The kinds that have a track buffer, in a fixed order.
    pub fn track_kinds(&self) -> Vec<TrackKind> {
        self.tracks.keys().copied().collect()
    }

    /// The track buffer of this kind (for inspection).
    pub fn track_buffer(&self, kind: TrackKind) -> Option<&TrackBuffer> {
        self.tracks.get(&kind)
    }

    /// Frames stored in this track.
    pub fn frame_count(&self, kind: TrackKind) -> usize {
        self.tracks.get(&kind).map_or(0, TrackBuffer::len)
    }

    /// Coded bytes stored in this track.
    pub fn track_bytes(&self, kind: TrackKind) -> usize {
        self.tracks.get(&kind).map_or(0, TrackBuffer::bytes)
    }

    /// Where the track's data really is, with no gap hidden (what the property tests compare).
    pub fn track_ranges_exact(&self, kind: TrackKind) -> Vec<TimeRange> {
        self.tracks
            .get(&kind)
            .map(|tb| tb.ranges().to_vec())
            .unwrap_or_default()
    }

    /// The id of the codec configuration the track's new frames are stored with.
    pub fn config_id(&self, kind: TrackKind) -> Option<u32> {
        self.tracks.get(&kind).map(TrackBuffer::config_id)
    }

    // ---- additions: buffered ----

    /// `SourceBuffer.buffered` with the MediaSource's state: when the stream has `ended` the last
    /// range of every audio and video track extends to the highest end time (the spec's step 4.2).
    pub fn buffered_with(&self, ended: bool) -> Vec<TimeRange> {
        let Some(highest) = self.highest_end() else {
            return Vec::new();
        };
        // Text tracks count for the highest end time but not for the intersection (unless the source
        // buffer has nothing else).
        let mut sources: Vec<&TrackBuffer> = self
            .tracks
            .values()
            .filter(|tb| matches!(tb.kind(), TrackKind::Video | TrackKind::Audio))
            .collect();
        if sources.is_empty() {
            sources = self.tracks.values().collect();
        }
        let mut acc = vec![TimeRange::new(0, highest)];
        for tb in sources {
            let mut track = tb.coalesced_ranges();
            if ended && let Some(last) = track.last_mut() {
                last.end = highest;
            }
            acc = ranges::intersect(&acc, &track);
        }
        acc
    }

    // ---- additions: quota ----

    /// Makes every track buffer, now and later, charge `budget` (the memory shared with the other
    /// source buffers).
    pub fn set_budget(&mut self, budget: Arc<MemoryBudget>) {
        for tb in self.tracks.values_mut() {
            tb.set_budget(Arc::clone(&budget));
        }
        self.budget = Some(budget);
    }

    /// The shared budget, if any.
    pub fn budget(&self) -> Option<&Arc<MemoryBudget>> {
        self.budget.as_ref()
    }

    /// No initialization segment has created a track buffer yet.
    pub fn is_uninitialized(&self) -> bool {
        self.tracks.is_empty()
    }

    /// The coded-frame bytes this source buffer may hold: the sum of the per-kind limits over its
    /// tracks.
    pub fn capacity(&self) -> usize {
        self.tracks
            .keys()
            .map(|k| self.limits.bytes_for(*k))
            .fold(0, usize::saturating_add)
    }

    /// `true` when `incoming_bytes` more would still fit.
    pub fn has_room(&self, incoming_bytes: usize) -> bool {
        self.bytes().saturating_add(incoming_bytes) <= self.capacity()
    }

    /// `true` when the buffer holds as much as it may ("this SourceBuffer is full and cannot accept more
    /// media data": the spec sets the buffer full flag then). An empty buffer is never full.
    pub fn is_full(&self) -> bool {
        let capacity = self.capacity();
        !self.tracks.is_empty() && self.bytes() >= capacity
    }

    /// Like [`Self::evict`], and says what was removed (item 3 sends `BufferedChanged` when it is not
    /// empty).
    ///
    /// The rule (brief D6): each track buffer keeps its own quota, and `incoming_bytes` is shared out
    /// over the tracks in proportion to their quotas (all of it for a single track). A track over its
    /// quota first gives up whole groups of pictures from the start of its buffered data, while the
    /// whole group ends at or before `position - keep_behind`; if that is not enough, whole groups from
    /// the end, while they start at or after `position + keep_ahead` and after the ranges that hold the
    /// position and the most recently appended frame (the append frontier: taking the tail of the range a
    /// page is appending to would leave a hole where its next segment goes). Nothing in
    /// `[position - keep_behind, position + keep_ahead)` is ever taken, and no part of a group is
    /// taken alone, so what stays decodes. A buffer that holds only the current range, with nothing
    /// behind the keep-behind line, therefore cannot evict: `ok` is `false` and the page must `remove`.
    pub fn evict_report(&mut self, incoming_bytes: usize, position: Ns) -> EvictReport {
        if self.tracks.is_empty() {
            return EvictReport {
                ok: true,
                removed: Removed::default(),
            };
        }
        let total_capacity = self.capacity();
        let mut incoming = [0usize; 3];
        for kind in TrackKind::ALL {
            if self.tracks.contains_key(&kind) {
                incoming[kind_slot(kind)] =
                    share_of(incoming_bytes, self.limits.bytes_for(kind), total_capacity);
            }
        }
        self.evict_for(incoming, position)
    }

    /// [`Self::evict_report`] for frames already parsed: each track is charged for its own frames
    /// (their bytes plus the per-frame overhead), so that a batch of video frames does not use up the
    /// audio quota. This is the check that bounds what one append can store, whatever it promised.
    pub fn evict_report_for_frames(&mut self, frames: &[CodedFrame], position: Ns) -> EvictReport {
        let mut incoming = [0usize; 3];
        for f in frames {
            let slot = kind_slot(f.kind);
            incoming[slot] =
                incoming[slot].saturating_add(f.data.len().saturating_add(FRAME_OVERHEAD_BYTES));
        }
        self.evict_for(incoming, position)
    }

    /// The eviction rule of [`Self::evict_report`] with the incoming bytes already split per track.
    fn evict_for(&mut self, incoming: [usize; 3], position: Ns) -> EvictReport {
        let mut report = EvictReport {
            ok: true,
            removed: Removed::default(),
        };
        let mut all_incoming = 0usize;
        for kind in TrackKind::ALL {
            if !self.tracks.contains_key(&kind) {
                continue;
            }
            let quota = self.limits.bytes_for(kind);
            let share = incoming[kind_slot(kind)];
            all_incoming = all_incoming.saturating_add(share);
            let held = self.track_bytes(kind);
            let need = held.saturating_add(share).saturating_sub(quota);
            if need > 0 {
                let info = self.evict_track(kind, need, position);
                report.removed.frames += info.frames;
                report.removed.bytes += info.bytes;
                if let Some(pts) = info.last_dts_hit {
                    self.last_frame_removed(pts);
                }
            }
            if self.track_bytes(kind).saturating_add(share) > quota {
                report.ok = false;
            }
        }
        // The shared budget (decision D17): when every source buffer of the process together would pass
        // it, this one gives up what its own rule allows, and the append is refused if that is not
        // enough.
        if let Some(budget) = self.budget.clone() {
            let excess = || {
                budget
                    .used()
                    .saturating_add(all_incoming)
                    .saturating_sub(budget.cap())
            };
            let mut need = excess();
            for kind in TrackKind::ALL {
                if need == 0 {
                    break;
                }
                if !self.tracks.contains_key(&kind) {
                    continue;
                }
                let info = self.evict_track(kind, need, position);
                report.removed.frames += info.frames;
                report.removed.bytes += info.bytes;
                if let Some(pts) = info.last_dts_hit {
                    self.last_frame_removed(pts);
                }
                need = excess();
            }
            if need > 0 {
                report.ok = false;
            }
        }
        report
    }

    /// Frees at least `need` bytes of one track if the rule allows.
    fn evict_track(&mut self, kind: TrackKind, need: usize, position: Ns) -> RemovedInfo {
        let cut = position.saturating_sub(self.limits.keep_behind);
        let limit = position.saturating_add(self.limits.keep_ahead);
        let mut total = RemovedInfo::default();
        let Some(tb) = self.tracks.get_mut(&kind) else {
            return total;
        };
        // From the start: groups that end before the keep-behind line.
        while total.bytes < need {
            let Some(gop) = tb.front_gop() else { break };
            if gop.max_end > cut {
                break;
            }
            let info = tb.remove_keys(&gop.keys);
            if info.frames == 0 {
                break;
            }
            total.add(&info);
        }
        // From the end: groups that start after the keep-ahead line and after the ranges that hold the
        // playback position and the append frontier (taking the tail of the range the page is
        // appending to would leave a hole right where its next segment goes; Firefox likewise never
        // evicts from the interval in use).
        let protected_end = tb
            .range_end_at(position)
            .into_iter()
            .chain(tb.append_frontier_end())
            .max()
            .unwrap_or(Ns::MIN);
        let line = limit.max(protected_end);
        while total.bytes < need {
            let Some(gop) = tb.back_gop() else { break };
            if gop.min_pts < line {
                break;
            }
            let info = tb.remove_keys(&gop.keys);
            if info.frames == 0 {
                break;
            }
            total.add(&info);
        }
        total
    }

    // ---- additions: the feed ----

    /// Like [`Self::cursor`], for a consumer that already has the codec configuration `known` (from
    /// an earlier cursor's [`FeedCursor::config_id`]): a `Config` item is emitted only when the first
    /// frame needs another one.
    pub fn cursor_with_config(
        &self,
        kind: TrackKind,
        position: Ns,
        known: Option<u32>,
    ) -> FeedCursor {
        let mut cursor = self.cursor(kind, position);
        cursor.config = known;
        cursor
    }

    /// Where the feed would start for `position`: the presentation time of the random access point at
    /// or before it, `None` when the data for the position has not arrived.
    pub fn feed_start(&self, kind: TrackKind, position: Ns) -> Option<Ns> {
        let tb = self.tracks.get(&kind)?;
        let key = tb.locate(position)?;
        tb.get(&key).map(|sf| sf.frame.pts)
    }

    /// `true` when nothing is stored that the cursor could ever serve without a new append: no frame
    /// follows the last one it served (or, for a cursor that has not started, nothing ends after its
    /// position). When the stream has ended this is "everything was served", the moment the feed sends
    /// the end of stream; a cursor that is waiting at a gap or for data that has not arrived is not
    /// exhausted while later frames are stored.
    pub fn cursor_exhausted(&self, kind: TrackKind, cursor: &FeedCursor) -> bool {
        let Some(tb) = self.tracks.get(&kind) else {
            return true;
        };
        match cursor.served {
            Some(last) => tb.next_after(&last.key).is_none(),
            None => tb.highest_end().is_none_or(|end| end <= cursor.position),
        }
    }

    /// Verifies every track buffer's invariants (tests).
    pub fn check_invariants(&self) -> Result<(), String> {
        for (kind, tb) in &self.tracks {
            tb.check_invariants()
                .map_err(|e| format!("{}: {e}", kind.name()))?;
        }
        Ok(())
    }
}

fn kind_index(kind: TrackKind) -> usize {
    match kind {
        TrackKind::Video => 0,
        TrackKind::Audio => 1,
        TrackKind::Text => 2,
    }
}

fn kind_slot(kind: TrackKind) -> usize {
    match kind {
        TrackKind::Video => 0,
        TrackKind::Audio => 1,
        TrackKind::Text => 2,
    }
}

/// `incoming` split in proportion `quota : total` (rounded up, so the shares cover all of it).
fn share_of(incoming: usize, quota: usize, total: usize) -> usize {
    if total == 0 {
        return incoming;
    }
    let numerator = incoming as u128 * quota as u128;
    let share = numerator.div_ceil(total as u128);
    usize::try_from(share).unwrap_or(usize::MAX)
}
