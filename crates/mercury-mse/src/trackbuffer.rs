// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The track buffer: one sorted store of coded frames per track kind, and the per-track variables of the
//! W3C MSE "coded frame processing" algorithm (last decode timestamp, last frame duration, highest end
//! timestamp, need random access point flag).
//!
//! OWNER: item 2 of lane med-1. Nothing outside `processor.rs` needs this API (item 3 goes through
//! `SourceBufferProcessor`).
//!
//! Written from the W3C Media Source Extensions text. Ladybird's `SourceBufferProcessor`/`TrackBuffer`
//! (BSD-2-Clause) and Firefox's `TrackBuffersManager` (MPL-2.0) were read for ideas only (the one-frame
//! gap tolerance of buffered ranges, whole-GOP eviction, tracking the "last frame removed" case); no code
//! of either is copied, so the file stays under Mercury's licence (brief D1).
//!
//! # Storage and cost
//!
//! Frames live in two ordered indexes over the same set:
//!
//! * `frames`, a `BTreeMap` keyed by `(decode timestamp, sequence number)`: decode order, which is the
//!   order the feed serves and the order in which "the next random access point" is found;
//! * `by_pts`, a `BTreeSet` of `(presentation timestamp, key)`: the order in which the spec removes
//!   ("all coded frames with a presentation timestamp in [a, b)") and in which a seek looks a frame up.
//!
//! Insertion, removal of one frame, `locate` and "the next frame after this key" are O(log n); removing
//! the front or the back of the buffer is O(k log n) for k frames. The buffered ranges are kept as a
//! sorted `Vec` of intervals updated incrementally (O(log m) at the end, O(frames removed) after a
//! removal), so asking for them never scans the frames. A live 4 h video buffer is about 350,000 frames;
//! `tests/frames_perf.rs` measures it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound::{Excluded, Unbounded};

use crate::api::{CodedFrame, Ns, TimeRange, TrackConfig, TrackKind};
use crate::ranges;

/// Presentation intervals that are this close (1 microsecond, which is the spec's own rounding
/// allowance) are one range: two tick-rounded frame times can differ by a nanosecond or two.
pub const MERGE_EPSILON: Ns = 1_000;

/// The largest gap between two frames that [`TrackBuffer::coalesced_ranges`] hides (250 ms). The gap
/// tolerance is the longest frame duration seen in the track, capped here so that a slideshow track with
/// ten second frames does not join ranges ten seconds apart.
pub const MAX_GAP_TOLERANCE: Ns = 250_000_000;

/// How far before the first buffered frame a seek to the very start of the media (position 0 or less) may
/// land and still start the feed at that frame (1 s, the fudge Chrome allows).
pub const SEEK_TO_START_FUDGE: Ns = 1_000_000_000;

/// How many distinct codec configurations one track buffer remembers before it prunes the unused ones.
const MAX_CONFIGS: usize = 64;

/// The position of a frame in decode order. `(decode timestamp, sequence number)`: the sequence number
/// is unique per inserted frame, so equal decode timestamps never collide and an older frame sorts
/// before a newer one.
pub type FrameKey = (Ns, u64);

/// A frame in a track buffer: the coded frame on the presentation timeline, the codec configuration it
/// is decoded with, and the group of pictures it belongs to.
#[derive(Clone, Debug)]
pub struct StoredFrame {
    pub frame: CodedFrame,
    /// The id of the codec configuration (see [`TrackBuffer::config_by_id`]) current when it was stored.
    pub config: u32,
    /// The sequence number of the random access point this frame is decoded from (its own for a
    /// random access point). Two frames with different `gop` were not stored as one run.
    pub gop: u64,
}

impl StoredFrame {
    /// The end of the presentation interval.
    pub fn end(&self) -> Ns {
        self.frame.pts.saturating_add(self.frame.duration)
    }

    /// Coded bytes.
    pub fn size(&self) -> usize {
        self.frame.data.len()
    }
}

/// What a removal took out of a track buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemovedInfo {
    pub frames: usize,
    pub bytes: usize,
    /// The presentation timestamp of a removed frame whose decode timestamp equals the track's last
    /// decode timestamp (the spec's trigger for "the last frame of the group was removed").
    pub last_dts_hit: Option<Ns>,
}

impl RemovedInfo {
    /// Adds another removal's counts (the first hit wins).
    pub fn add(&mut self, other: &RemovedInfo) {
        self.frames += other.frames;
        self.bytes += other.bytes;
        if self.last_dts_hit.is_none() {
            self.last_dts_hit = other.last_dts_hit;
        }
    }
}

/// One group of pictures: a random access point and the frames that follow it up to the next one.
#[derive(Clone, Debug)]
pub struct GopInfo {
    pub keys: Vec<FrameKey>,
    pub bytes: usize,
    /// The earliest presentation timestamp in the group.
    pub min_pts: Ns,
    /// The latest end of a presentation interval in the group.
    pub max_end: Ns,
}

impl GopInfo {
    fn new() -> Self {
        GopInfo {
            keys: Vec::new(),
            bytes: 0,
            min_pts: Ns::MAX,
            max_end: Ns::MIN,
        }
    }

    fn push(&mut self, key: FrameKey, sf: &StoredFrame) {
        self.keys.push(key);
        self.bytes += sf.size();
        self.min_pts = self.min_pts.min(sf.frame.pts);
        self.max_end = self.max_end.max(sf.end());
    }
}

/// `true` when two track configurations decode the same way (a new codec configuration is a change
/// only when one of these differs). The whole configuration counts, not only the codec's own record:
/// VP9 in WebM has no configuration record (`Vp9 { vpcc: None }` at every resolution), so two renditions
/// of it differ only in their size. Language, label, default flag, track id and timescale do not matter.
pub fn same_decoder_config(a: &TrackConfig, b: &TrackConfig) -> bool {
    a.kind == b.kind
        && a.codec == b.codec
        && a.codec_string == b.codec_string
        && a.width == b.width
        && a.height == b.height
        && a.sample_rate == b.sample_rate
        && a.channels == b.channels
}

/// The coded frames of one track and the spec variables that go with them.
#[derive(Debug)]
pub struct TrackBuffer {
    kind: TrackKind,
    frames: BTreeMap<FrameKey, StoredFrame>,
    by_pts: BTreeSet<(Ns, FrameKey)>,
    next_seq: u64,
    bytes: usize,
    /// Union of the frames' presentation intervals, merged within [`MERGE_EPSILON`].
    ranges: Vec<TimeRange>,
    /// The longest frame duration since the buffer was last empty.
    longest: Ns,
    /// The presentation timestamp of the frame stored last.
    last_inserted_pts: Option<Ns>,
    current_gop: u64,
    configs: Vec<(u32, TrackConfig)>,
    current_config: u32,
    next_config_id: u32,

    /// <https://w3c.github.io/media-source/#last-decode-timestamp>
    pub last_decode_timestamp: Option<Ns>,
    /// <https://w3c.github.io/media-source/#last-frame-duration>
    pub last_frame_duration: Option<Ns>,
    /// <https://w3c.github.io/media-source/#highest-end-timestamp>
    pub highest_end_timestamp: Option<Ns>,
    /// <https://w3c.github.io/media-source/#need-RAP-flag>; initially true.
    pub need_random_access_point: bool,
}

impl TrackBuffer {
    /// An empty track buffer with its first codec configuration.
    pub fn new(kind: TrackKind, config: TrackConfig) -> Self {
        TrackBuffer {
            kind,
            frames: BTreeMap::new(),
            by_pts: BTreeSet::new(),
            next_seq: 0,
            bytes: 0,
            ranges: Vec::new(),
            longest: 0,
            last_inserted_pts: None,
            current_gop: 0,
            configs: vec![(0, config)],
            current_config: 0,
            next_config_id: 1,
            last_decode_timestamp: None,
            last_frame_duration: None,
            highest_end_timestamp: None,
            need_random_access_point: true,
        }
    }

    pub fn kind(&self) -> TrackKind {
        self.kind
    }

    /// Frames stored.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Coded bytes stored.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    // ---- the spec's per-track variables ----

    /// Unsets the last decode timestamp, the last frame duration and the highest end timestamp.
    pub fn unset_group_state(&mut self) {
        self.last_decode_timestamp = None;
        self.last_frame_duration = None;
        self.highest_end_timestamp = None;
    }

    // ---- codec configurations ----

    /// The id of the configuration new frames are stored with.
    pub fn config_id(&self) -> u32 {
        self.current_config
    }

    /// The configuration new frames are stored with.
    pub fn current_config(&self) -> &TrackConfig {
        self.config_by_id(self.current_config)
            .unwrap_or(&self.configs[0].1)
    }

    /// The configuration with this id, if the buffer still remembers it.
    pub fn config_by_id(&self, id: u32) -> Option<&TrackConfig> {
        self.configs.iter().find(|(i, _)| *i == id).map(|(_, c)| c)
    }

    /// Makes `config` the configuration of the frames stored from now on. Returns `true` when it decodes
    /// differently from the current one (the feed will emit a `Config` item before the next frame).
    pub fn set_config(&mut self, config: TrackConfig) -> bool {
        let current = self.current_config;
        if let Some(slot) = self.configs.iter_mut().find(|(i, _)| *i == current)
            && same_decoder_config(&slot.1, &config)
        {
            // Same decoding: keep the id, refresh the descriptive fields (language, track id).
            slot.1 = config;
            return false;
        }
        if let Some((id, slot)) = self
            .configs
            .iter_mut()
            .find(|(_, c)| same_decoder_config(c, &config))
        {
            *slot = config;
            self.current_config = *id;
            return true;
        }
        if self.configs.len() >= MAX_CONFIGS {
            self.prune_configs();
        }
        let id = self.next_config_id;
        self.next_config_id += 1;
        self.configs.push((id, config));
        self.current_config = id;
        true
    }

    /// Forgets the configurations no stored frame uses (the current one stays).
    fn prune_configs(&mut self) {
        let used: BTreeSet<u32> = self.frames.values().map(|sf| sf.config).collect();
        let current = self.current_config;
        self.configs
            .retain(|(id, _)| *id == current || used.contains(id));
    }

    // ---- insertion ----

    /// Stores a frame (already on the presentation timeline) in decode order and returns its key.
    pub fn insert(&mut self, frame: CodedFrame) -> FrameKey {
        let seq = self.next_seq;
        self.next_seq += 1;
        let key = (frame.dts, seq);
        if frame.keyframe {
            self.current_gop = seq;
        }
        let end = frame.pts.saturating_add(frame.duration);
        self.bytes += frame.data.len();
        self.longest = self.longest.max(frame.duration);
        self.last_inserted_pts = Some(frame.pts);
        ranges::insert_interval(
            &mut self.ranges,
            TimeRange::new(frame.pts, end),
            MERGE_EPSILON,
        );
        self.by_pts.insert((frame.pts, key));
        self.frames.insert(
            key,
            StoredFrame {
                frame,
                config: self.current_config,
                gop: self.current_gop,
            },
        );
        key
    }

    // ---- ranges ----

    /// The union of the stored frames' presentation intervals (merged within 1 microsecond): the
    /// spec's "track buffer ranges".
    pub fn ranges(&self) -> &[TimeRange] {
        &self.ranges
    }

    /// The largest gap [`Self::coalesced_ranges`] hides: the longest frame duration, at most
    /// [`MAX_GAP_TOLERANCE`].
    pub fn gap_tolerance(&self) -> Ns {
        self.longest.clamp(0, MAX_GAP_TOLERANCE)
    }

    /// The ranges the page sees: [`Self::ranges`] with the gaps of at most one frame closed, as the
    /// spec allows ("coalescing adjacent ranges separated by a gap smaller than 2 times the maximum
    /// frame duration") and as Firefox does, so that a one-frame rounding gap does not split a range.
    pub fn coalesced_ranges(&self) -> Vec<TimeRange> {
        ranges::coalesce(&self.ranges, self.gap_tolerance())
    }

    /// The end of the last range.
    pub fn highest_end(&self) -> Option<Ns> {
        ranges::last_end(&self.ranges)
    }

    /// The end of the range that holds the instant `t`, if one does.
    pub fn range_end_at(&self, t: Ns) -> Option<Ns> {
        ranges::find(&self.ranges, t).map(|i| self.ranges[i].end)
    }

    /// The end of the range that holds the frame stored last: the append frontier, which eviction from
    /// the end leaves alone.
    pub fn append_frontier_end(&self) -> Option<Ns> {
        self.last_inserted_pts.and_then(|t| self.range_end_at(t))
    }

    /// Rebuilds the ranges that touch the window `[lo, hi)` from the frames that remain, after frames
    /// whose intervals were inside the window were removed. The ranges outside the window stay.
    fn refresh_ranges(&mut self, lo: Ns, hi: Ns) {
        if self.frames.is_empty() {
            self.ranges.clear();
            return;
        }
        let eps = MERGE_EPSILON;
        let first = self
            .ranges
            .partition_point(|r| r.end.saturating_add(eps) < lo);
        let last = self
            .ranges
            .partition_point(|r| r.start <= hi.saturating_add(eps));
        let mut mid: Vec<TimeRange> = Vec::new();
        if first < last {
            let a = self.ranges[first];
            if a.start < lo {
                ranges::insert_interval(&mut mid, TimeRange::new(a.start, lo), eps);
            }
        }
        // Remaining frames that overlap or touch the window start no earlier than `lo - longest`; the
        // ones that end before it are in ranges the window does not reach and stay out of `mid`.
        let from = (lo.saturating_sub(self.longest), (Ns::MIN, 0));
        for (pts, key) in self.by_pts.range(from..) {
            if *pts >= hi {
                break;
            }
            let end = self.frames[key].end();
            if *pts < end && end.saturating_add(eps) >= lo {
                ranges::insert_interval(&mut mid, TimeRange::new(*pts, end), eps);
            }
        }
        if first < last {
            // The part of the old range after the window starts at the first remaining frame at or
            // after it (that frame's start is a true edge; the window's end may sit a nanosecond before).
            let b = self.ranges[last - 1];
            if b.end > hi {
                let after = self.by_pts.range((hi, (Ns::MIN, 0))..).next();
                if let Some((start, _)) = after.filter(|(p, _)| *p < b.end) {
                    ranges::insert_interval(&mut mid, TimeRange::new(*start, b.end), eps);
                }
            }
        }
        self.ranges.splice(first..last, mid);
    }

    /// Recomputes the ranges from scratch (tests compare it with the incremental ones).
    pub fn recompute_ranges(&self) -> Vec<TimeRange> {
        let mut out = Vec::new();
        for (pts, key) in &self.by_pts {
            let end = self.frames[key].end();
            ranges::insert_interval(&mut out, TimeRange::new(*pts, end), MERGE_EPSILON);
        }
        out
    }

    // ---- lookups ----

    pub fn get(&self, key: &FrameKey) -> Option<&StoredFrame> {
        self.frames.get(key)
    }

    /// Every frame in decode order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&FrameKey, &StoredFrame)> {
        self.frames.iter()
    }

    /// The frame that follows `key` in decode order.
    pub fn next_after(&self, key: &FrameKey) -> Option<(&FrameKey, &StoredFrame)> {
        self.frames.range((Excluded(*key), Unbounded)).next()
    }

    /// The first random access point after `key` in decode order.
    pub fn next_keyframe_after(&self, key: &FrameKey) -> Option<FrameKey> {
        self.frames
            .range((Excluded(*key), Unbounded))
            .find(|(_, sf)| sf.frame.keyframe)
            .map(|(k, _)| *k)
    }

    /// The frame with the greatest presentation timestamp at or before `pts` whose presentation
    /// interval contains `pts`.
    pub fn frame_covering(&self, pts: Ns) -> Option<FrameKey> {
        let (p, key) = self
            .by_pts
            .range(..=(pts, (Ns::MAX, u64::MAX)))
            .next_back()?;
        let end = self.frames[key].end();
        (*p <= pts && pts < end).then_some(*key)
    }

    /// The random access point to start feeding from for `position`: the latest random access point in
    /// decode order, at or before the frame that is shown at `position`, whose presentation time is not
    /// after `position`. The position must lie in a buffered range, or at most one frame-tolerance
    /// before one (the first frame of a stream often starts a few milliseconds after zero); `None`
    /// otherwise: the data for that position has not arrived (or was removed).
    pub fn locate(&self, position: Ns) -> Option<FrameKey> {
        let tol = self.gap_tolerance();
        let idx = self.ranges.partition_point(|r| r.end <= position);
        let range = self.ranges.get(idx)?;
        // A seek to the start of the media may land up to a second before the first frame (B-frame
        // streams start a frame or two after their first decode time; Chrome allows the same).
        let slack = if position <= 0 && idx == 0 {
            tol.max(SEEK_TO_START_FUDGE)
        } else {
            tol
        };
        if range.start.saturating_sub(slack) > position {
            return None;
        }
        let upper = (position, (Ns::MAX, u64::MAX));
        let target = match self.by_pts.range(..=upper).next_back() {
            Some((pts, key)) if *pts >= range.start => *key,
            _ => self.by_pts.range((range.start, (Ns::MIN, 0))..).next()?.1,
        };
        let target_pts = self.frames.get(&target)?.frame.pts;
        let limit = position.max(target_pts);
        self.frames
            .range(..=target)
            .rev()
            .find(|(_, sf)| sf.frame.keyframe && sf.frame.pts <= limit)
            .map(|(k, _)| *k)
            .or(Some(target))
    }

    /// The random access point `gop` that the non-key frame at `key` is decoded from, if it is still
    /// in the buffer right before it (walking back in decode order, stopping at a different random
    /// access point).
    pub fn find_gop_start(&self, key: &FrameKey, gop: u64) -> Option<FrameKey> {
        for (k, sf) in self.frames.range(..=*key).rev() {
            if sf.frame.keyframe {
                return (k.1 == gop).then_some(*k);
            }
        }
        None
    }

    // ---- removal ----

    /// Removes these frames and updates the byte count and the ranges.
    pub fn remove_keys(&mut self, keys: &[FrameKey]) -> RemovedInfo {
        let mut info = RemovedInfo::default();
        let (mut lo, mut hi) = (Ns::MAX, Ns::MIN);
        for key in keys {
            let Some(sf) = self.frames.remove(key) else {
                continue;
            };
            self.by_pts.remove(&(sf.frame.pts, *key));
            let size = sf.size();
            self.bytes -= size;
            info.frames += 1;
            info.bytes += size;
            if info.last_dts_hit.is_none() && self.last_decode_timestamp == Some(sf.frame.dts) {
                info.last_dts_hit = Some(sf.frame.pts);
            }
            let end = sf.end();
            if sf.frame.pts < end {
                lo = lo.min(sf.frame.pts);
                hi = hi.max(end);
            }
        }
        if info.frames > 0 {
            if self.frames.is_empty() {
                self.reset_when_empty();
            } else if lo < hi {
                self.refresh_ranges(lo, hi);
            }
        }
        info
    }

    /// The buffer holds nothing: the ranges, the longest frame and the unused configurations go.
    fn reset_when_empty(&mut self) {
        self.ranges.clear();
        self.longest = 0;
        self.last_inserted_pts = None;
        let current = self.current_config;
        self.configs.retain(|(id, _)| *id == current);
    }

    /// Removes every frame and returns what that took out.
    pub fn clear(&mut self) -> RemovedInfo {
        let keys: Vec<FrameKey> = self.frames.keys().copied().collect();
        self.remove_keys(&keys)
    }

    /// Adds to `keys` (the frames being removed) every frame that may depend on one of them: for each
    /// removed frame, the frames after it in decode order up to (not including) the next random access
    /// point. The spec calls this "a conservative estimate": it assumes every frame between a removed
    /// frame and the next random access point depended on it. Sorts and deduplicates `keys`.
    fn add_dependents(&self, keys: &mut Vec<FrameKey>) {
        keys.sort_unstable();
        keys.dedup();
        let mut extra: Vec<FrameKey> = Vec::new();
        // Where the last scan stopped: the key of the random access point (or the end of the buffer).
        let mut scanned_to: Option<FrameKey> = None;
        for k in keys.iter() {
            if scanned_to.is_some_and(|end| *k < end) {
                continue;
            }
            let mut stop = (Ns::MAX, u64::MAX);
            for (nk, sf) in self.frames.range((Excluded(*k), Unbounded)) {
                if sf.frame.keyframe {
                    stop = *nk;
                    break;
                }
                extra.push(*nk);
            }
            scanned_to = Some(stop);
        }
        keys.extend(extra);
    }

    /// Removes the frames with a presentation timestamp in `[lo, hi)` (`hi == None` is +Infinity) and
    /// every frame that depends on them (the frames after each removed one, up to the next random access point).
    pub fn remove_with_dependents(&mut self, lo: Ns, hi: Option<Ns>) -> RemovedInfo {
        let mut keys: Vec<FrameKey> = Vec::new();
        for (pts, key) in self.by_pts.range((lo, (Ns::MIN, 0))..) {
            if hi.is_some_and(|h| *pts >= h) {
                break;
            }
            keys.push(*key);
        }
        if keys.is_empty() {
            return RemovedInfo::default();
        }
        self.add_dependents(&mut keys);
        self.remove_keys(&keys)
    }

    /// Removes one frame and every frame that depends on it (the frames after it in decode order up to
    /// the next random access point).
    pub fn remove_frame_with_dependents(&mut self, key: &FrameKey) -> RemovedInfo {
        let mut keys = vec![*key];
        self.add_dependents(&mut keys);
        self.remove_keys(&keys)
    }

    /// The spec's "coded frame removal" for one track buffer: removes the frames that start in
    /// `[start, remove end)`, where `remove end` is the presentation timestamp of the first random
    /// access point at or after `end` (+Infinity when `end` is None or no such point exists), and the
    /// frames that depend on them.
    pub fn remove_range(&mut self, start: Ns, end: Option<Ns>) -> RemovedInfo {
        if end.is_some_and(|e| e <= start) {
            return RemovedInfo::default();
        }
        let remove_end = end.and_then(|e| {
            self.by_pts
                .range((e, (Ns::MIN, 0))..)
                .find(|(_, key)| self.frames[key].frame.keyframe)
                .map(|(pts, _)| *pts)
        });
        self.remove_with_dependents(start, remove_end)
    }

    // ---- groups of pictures (eviction) ----

    /// The first group of pictures in decode order.
    pub fn front_gop(&self) -> Option<GopInfo> {
        let mut info = GopInfo::new();
        for (k, sf) in &self.frames {
            if sf.frame.keyframe && !info.keys.is_empty() {
                break;
            }
            info.push(*k, sf);
        }
        (!info.keys.is_empty()).then_some(info)
    }

    /// The last group of pictures in decode order.
    pub fn back_gop(&self) -> Option<GopInfo> {
        let mut info = GopInfo::new();
        for (k, sf) in self.frames.iter().rev() {
            info.push(*k, sf);
            if sf.frame.keyframe {
                break;
            }
        }
        info.keys.reverse();
        (!info.keys.is_empty()).then_some(info)
    }

    // ---- checks ----

    /// Verifies the internal invariants (the two indexes agree, the byte count, the incremental ranges
    /// equal the recomputed ones, the ranges are normalised). Returns the first violation. For tests.
    pub fn check_invariants(&self) -> Result<(), String> {
        if self.frames.len() != self.by_pts.len() {
            return Err(format!(
                "{} frames but {} presentation entries",
                self.frames.len(),
                self.by_pts.len()
            ));
        }
        let mut bytes = 0usize;
        for (pts, key) in &self.by_pts {
            let sf = self
                .frames
                .get(key)
                .ok_or_else(|| format!("presentation entry {key:?} has no frame"))?;
            if sf.frame.pts != *pts || sf.frame.dts != key.0 {
                return Err(format!("entry {key:?} disagrees with its frame"));
            }
            bytes += sf.size();
        }
        if bytes != self.bytes {
            return Err(format!("bytes {} but frames hold {bytes}", self.bytes));
        }
        if !ranges::is_normalized(&self.ranges) {
            return Err(format!("ranges are not normalised: {:?}", self.ranges));
        }
        // The incremental ranges may differ from a from-scratch union by the merge epsilon at an edge
        // (a removal that leaves a gap keeps the old end of the range before it); never by more.
        let fresh = self.recompute_ranges();
        let close = fresh.len() == self.ranges.len()
            && fresh.iter().zip(&self.ranges).all(|(a, b)| {
                (a.start - b.start).abs() <= MERGE_EPSILON && (a.end - b.end).abs() <= MERGE_EPSILON
            });
        if !close {
            return Err(format!(
                "incremental ranges {:?} differ from recomputed {fresh:?}",
                self.ranges
            ));
        }
        if self.frames.is_empty() && (self.longest != 0 || !self.ranges.is_empty()) {
            return Err("empty buffer with leftovers".to_string());
        }
        Ok(())
    }
}
