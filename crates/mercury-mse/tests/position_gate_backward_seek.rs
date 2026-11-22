// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A backward seek under a small quota, with the real `MseBackend` (med-1b review fix, finding 2).
//!
//! The element calls `set_position(target)` and the MediaSource's seeking algorithm when a seek starts, and
//! the player keeps reporting its position every 100 ms until the seek completes. The first of those reports
//! after the seek began still carries the position from before it (the old, later one). The backend uses the
//! position for eviction: with the stale one, a backward seek makes the data around the target look like
//! more than `keep_behind` in the past, and the next append under quota pressure evicts it. The element
//! therefore passes every report through `forward_position(seeking, position)`.

mod model_support;

use mercury_mse::api::{Limits, MediaSourceBackend, TimeRange, secs_to_ns};
use mercury_mse::media_state::forward_position;
use mercury_mse_testkit::SegmentedStream;
use model_support::*;

/// Segments of the tiled stream the buffer holds when the seek happens (2 s each: 12 s).
const HELD_SEGMENTS: usize = 6;
/// Where the player was (and where its stale report says it still is), seconds.
const STALE_POSITION: f64 = 11.5;
/// The seek target, seconds: inside the first group of pictures.
const TARGET: f64 = 1.0;

/// Four 2 s segments of `h264-low`, twice: 16 s of video in eight segments.
fn stream() -> SegmentedStream {
    mercury_mse_testkit::tile(&load("h264-low"), 2).expect("tile")
}

/// A video quota that holds exactly the first [`HELD_SEGMENTS`] segments (bytes plus the per-frame charge).
fn quota_for(stream: &SegmentedStream) -> usize {
    let held = &stream.media[..HELD_SEGMENTS];
    let bytes: usize = held.iter().map(|s| s.bytes.len()).sum();
    // 25 frames/s x 2 s per segment, each charged FRAME_OVERHEAD_BYTES on top of its payload; the sum of
    // the segments' bytes already counts every payload byte (and more: box headers), so this is a ceiling
    // that cannot take one more segment.
    bytes + HELD_SEGMENTS * 50 * mercury_mse::api::FRAME_OVERHEAD_BYTES
}

fn covers(ranges: &[TimeRange], secs: f64) -> bool {
    let ns = secs_to_ns(secs);
    ranges.iter().any(|r| r.start <= ns && ns < r.end)
}

/// What the element does for a position report from the player.
fn position_report(backend: &dyn MediaSourceBackend, seeking: bool, position: f64) {
    if let Some(position) = forward_position(seeking, position) {
        backend.set_position(position);
    }
}

/// Plays forward to [`STALE_POSITION`] with [`HELD_SEGMENTS`] segments buffered, then seeks back to
/// [`TARGET`]: the element's seek hook sets the target position and runs the MediaSource's seeking. Returns
/// the rig, the buffer, and the stream.
fn after_backward_seek(
    report_stale_position: impl Fn(&dyn MediaSourceBackend),
) -> (Rig, mercury_mse::api::SourceBufferId, SegmentedStream) {
    let stream = stream();
    let rig = Rig::with_limits(Limits {
        video_bytes: quota_for(&stream),
        ..Limits::default()
    });
    let id = rig.add(&stream.mime);
    rig.append_ok(id, &stream.init.bytes);
    for segment in &stream.media[..HELD_SEGMENTS] {
        rig.append_ok(id, &segment.bytes);
    }
    assert_ranges(
        "before the seek",
        &rig.backend.buffered(id),
        &[(0.0, 12.0)],
        100.0,
    );

    rig.backend.set_position(STALE_POSITION);
    // The seek begins: target position, then the MediaSource's seeking algorithm.
    rig.backend.set_position(TARGET);
    rig.backend.seeking(TARGET);
    // The next position report of the player is still the old one.
    report_stale_position(&*rig.backend);
    (rig, id, stream)
}

#[test]
fn without_the_gate_a_stale_position_after_a_backward_seek_evicts_the_data_at_the_target() {
    // The reason for the gate: forward every report, as the element did before the fix.
    let (rig, id, stream) = after_backward_seek(|backend| backend.set_position(STALE_POSITION));
    // The page keeps buffering ahead; the buffer is full, so this append needs eviction.
    let outcome = rig.append(id, &stream.media[HELD_SEGMENTS].bytes);
    assert_eq!(outcome, Outcome::Done, "the append only fits by evicting");
    let buffered = rig.backend.buffered(id);
    assert!(
        !covers(&buffered, TARGET),
        "the data at the seek target {TARGET} s survived the stale position: {:?}",
        secs(&buffered)
    );
}

#[test]
fn a_stale_position_is_not_forwarded_while_seeking_so_the_data_at_the_target_survives() {
    let (rig, id, stream) = after_backward_seek(|backend| {
        position_report(backend, true, STALE_POSITION);
    });
    // The same append under the same pressure. The backend knows the position is the target, so nothing
    // near it may go (nothing behind the target is old enough, nothing ahead is past the keep-ahead line):
    // the page gets a quota error and would have to remove ranges itself.
    let outcome = rig.append(id, &stream.media[HELD_SEGMENTS].bytes);
    let buffered = rig.backend.buffered(id);
    assert!(
        covers(&buffered, TARGET),
        "the data at the seek target {TARGET} s was evicted ({outcome:?}): {:?}",
        secs(&buffered)
    );
    assert_ranges("after the append", &buffered, &[(0.0, 12.0)], 100.0);
    assert!(
        covers(&buffered, TARGET + 2.0) && covers(&buffered, 11.0),
        "the rest of the buffer is untouched too: {:?}",
        secs(&buffered)
    );
}

#[test]
fn once_the_seek_has_completed_the_players_positions_are_forwarded_again() {
    let (rig, id, stream) = after_backward_seek(|backend| {
        position_report(backend, true, STALE_POSITION);
    });
    // The seek is done: the player reports from the target, and later from further on. At 9 s the first
    // group is more than keep_behind (5 s) in the past, so eviction may take it again.
    position_report(&*rig.backend, false, TARGET);
    position_report(&*rig.backend, false, 9.0);
    let outcome = rig.append(id, &stream.media[HELD_SEGMENTS].bytes);
    assert_eq!(outcome, Outcome::Done);
    assert!(
        !covers(&rig.backend.buffered(id), 0.5),
        "forwarding must resume after the seek: {:?}",
        rig.buffered_secs(id)
    );
}
