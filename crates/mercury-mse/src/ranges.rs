// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Sets of time ranges: normalisation, union, intersection.
//!
//! OWNER: item 2 of lane med-1. The three functions are the contract with item 3 (the media element's
//! `buffered` is the intersection over source buffers); item 2 may add more.

use crate::api::TimeRange;

/// Sorts, drops empty ranges and merges overlapping or touching ones.
pub fn normalize(ranges: &[TimeRange]) -> Vec<TimeRange> {
    let _ = ranges;
    todo!("item 2: ranges")
}

/// The union of two normalised range sets (normalised).
pub fn union(a: &[TimeRange], b: &[TimeRange]) -> Vec<TimeRange> {
    let _ = (a, b);
    todo!("item 2: ranges")
}

/// The intersection of two normalised range sets (normalised).
pub fn intersect(a: &[TimeRange], b: &[TimeRange]) -> Vec<TimeRange> {
    let _ = (a, b);
    todo!("item 2: ranges")
}
