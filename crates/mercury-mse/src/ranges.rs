// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Sets of time ranges: normalisation, union, intersection, difference and gap coalescing.
//!
//! OWNER: item 2 of lane med-1. `normalize`, `union` and `intersect` are the contract with item 3 (the
//! media element's `buffered` is the intersection over source buffers); the rest is what the track
//! buffers need and is free for anyone to use.
//!
//! Every function takes and returns ranges as half-open intervals `[start, end)` in nanoseconds. A
//! *normalised* set is sorted by start, has no empty range and no two ranges that overlap or touch. All
//! arithmetic is saturating or comparison-only, so no input (including `i64::MAX` ends, which stand for
//! +Infinity) can overflow.

use crate::api::{Ns, TimeRange};

/// `true` when the set is sorted, has no empty range, and no two ranges overlap or touch.
pub fn is_normalized(ranges: &[TimeRange]) -> bool {
    ranges.iter().all(|r| r.start < r.end) && ranges.windows(2).all(|w| w[0].end < w[1].start)
}

/// Sorts, drops empty ranges and merges overlapping or touching ones.
pub fn normalize(ranges: &[TimeRange]) -> Vec<TimeRange> {
    let mut sorted: Vec<TimeRange> = ranges.iter().copied().filter(|r| r.start < r.end).collect();
    sorted.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<TimeRange> = Vec::with_capacity(sorted.len());
    for r in sorted {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// The union of two normalised range sets (normalised).
pub fn union(a: &[TimeRange], b: &[TimeRange]) -> Vec<TimeRange> {
    let mut out: Vec<TimeRange> = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        let take_a = j >= b.len() || (i < a.len() && a[i].start <= b[j].start);
        let next = if take_a {
            i += 1;
            a[i - 1]
        } else {
            j += 1;
            b[j - 1]
        };
        if next.start >= next.end {
            continue;
        }
        match out.last_mut() {
            Some(last) if next.start <= last.end => last.end = last.end.max(next.end),
            _ => out.push(next),
        }
    }
    out
}

/// The intersection of two normalised range sets (normalised).
pub fn intersect(a: &[TimeRange], b: &[TimeRange]) -> Vec<TimeRange> {
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

/// `a` without the parts that are in `b` (both normalised; the result is normalised).
pub fn subtract(a: &[TimeRange], b: &[TimeRange]) -> Vec<TimeRange> {
    let mut out = Vec::new();
    let mut j = 0;
    for r in a {
        let mut start = r.start;
        // Cuts that end before this range starts also end before every later range starts.
        while j < b.len() && b[j].end <= start {
            j += 1;
        }
        let mut k = j;
        while k < b.len() && b[k].start < r.end {
            if b[k].start > start {
                out.push(TimeRange::new(start, b[k].start));
            }
            start = start.max(b[k].end);
            if start >= r.end {
                break;
            }
            k += 1;
        }
        if start < r.end {
            out.push(TimeRange::new(start, r.end));
        }
    }
    out
}

/// Merges the ranges of a normalised set that are separated by a gap of at most `max_gap` (the
/// one-frame tolerance of a track's buffered ranges). A `max_gap` of zero or less changes nothing.
pub fn coalesce(ranges: &[TimeRange], max_gap: Ns) -> Vec<TimeRange> {
    let max_gap = max_gap.max(0);
    let mut out: Vec<TimeRange> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start.saturating_sub(last.end) <= max_gap => {
                last.end = last.end.max(r.end);
            }
            _ => out.push(*r),
        }
    }
    out
}

/// Adds one interval to a normalised set in place, merging it with whatever it overlaps or touches and
/// with ranges at most `epsilon` away. Appending at the end is O(log n); an empty interval does nothing.
pub fn insert_interval(ranges: &mut Vec<TimeRange>, interval: TimeRange, epsilon: Ns) {
    if interval.start >= interval.end {
        return;
    }
    let epsilon = epsilon.max(0);
    // The first range whose end reaches the new start (within epsilon).
    let first = ranges.partition_point(|r| r.end.saturating_add(epsilon) < interval.start);
    // One past the last range that starts no later than the new end (within epsilon).
    let last = ranges.partition_point(|r| r.start <= interval.end.saturating_add(epsilon));
    if first >= last {
        ranges.insert(first, interval);
        return;
    }
    let start = ranges[first].start.min(interval.start);
    let end = ranges[last - 1].end.max(interval.end);
    ranges[first] = TimeRange::new(start, end);
    ranges.drain(first + 1..last);
}

/// The index of the range containing `t` (`start <= t < end`) in a normalised set.
pub fn find(ranges: &[TimeRange], t: Ns) -> Option<usize> {
    let after = ranges.partition_point(|r| r.end <= t);
    ranges.get(after).filter(|r| r.start <= t).map(|_| after)
}

/// `true` when `t` is inside some range of a normalised set.
pub fn contains(ranges: &[TimeRange], t: Ns) -> bool {
    find(ranges, t).is_some()
}

/// The total length of a normalised set, saturating.
pub fn total_len(ranges: &[TimeRange]) -> Ns {
    ranges.iter().fold(0, |sum, r| {
        sum.saturating_add(r.end.saturating_sub(r.start))
    })
}

/// The end of the last range, or `None` for an empty set.
pub fn last_end(ranges: &[TimeRange]) -> Option<Ns> {
    ranges.last().map(|r| r.end)
}

/// The start of the first range, or `None` for an empty set.
pub fn first_start(ranges: &[TimeRange]) -> Option<Ns> {
    ranges.first().map(|r| r.start)
}
