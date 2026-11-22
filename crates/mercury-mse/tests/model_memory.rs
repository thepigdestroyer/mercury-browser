// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The memory a page can make MSE use (lane med-1, review fixes S1 to S3): every stored frame is
//! charged its bytes plus a per-frame overhead, the first append is checked although no track buffer
//! exists yet, and all the MediaSources of a process share one budget.

mod model_support;
#[path = "parse_support/mod.rs"]
mod parse_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use mercury_mse::MseBackend;
use mercury_mse::api::{
    AllDecoders, AppendErrorKind, FRAME_OVERHEAD_BYTES, Limits, MediaSourceBackend, MemoryBudget,
    MseError, TrackKind,
};
use model_support::*;
use parse_support::{
    Run, RunSample, Tfhd, TrackSpec, fragment, init_of_tracks, payload, tfdt, tfhd, traf, trun,
};

const MIME: &str = "video/mp4; codecs=\"avc1.42C01E\"";

/// Small quotas: 128 KiB per kind (more than the raw bytes of the hostile appends below, so that the
/// size checks of `append` pass and only the accounting of the stored frames can stop them).
fn small_limits() -> Limits {
    Limits {
        video_bytes: 128 * 1024,
        audio_bytes: 128 * 1024,
        text_bytes: 128 * 1024,
        ..Limits::default()
    }
}

/// The initialization segment of one hand-built H.264 track (id 1, millisecond timescale).
fn init() -> Vec<u8> {
    init_of_tracks(&[TrackSpec::video(1, 1000)])
}

/// A `moof` + `mdat` of `count` one-byte samples of one millisecond each, starting at `start_ms`: a few
/// dozen bytes of metadata that stand for `count` stored frames.
fn tiny_samples(count: usize, start_ms: u64) -> Vec<u8> {
    fragment(
        |data_offset| {
            vec![traf(&[
                tfhd(&Tfhd {
                    track_id: 1,
                    default_duration: Some(1),
                    default_size: Some(1),
                    default_base_is_moof: true,
                    ..Tfhd::default()
                }),
                tfdt(1, start_ms),
                trun(&Run {
                    data_offset: Some(data_offset as i32),
                    samples: vec![RunSample::default(); count],
                    ..Run::default()
                }),
            ])]
        },
        &payload(1, count),
    )
}

/// The frames the feed holds, counted by pulling them all.
fn frames_held(backend: &dyn MediaSourceBackend) -> usize {
    let feed = backend.frame_feed();
    feed.seek(0);
    drain(&*feed, TrackKind::Video).frames.len()
}

#[test]
fn s1_a_moof_of_very_many_tiny_samples_hits_the_quota_not_the_heap() {
    // 100,000 one-byte samples: 100 KB of coded bytes, which the old accounting counted as 100 KB
    // against a 128 KiB quota (so it accepted them all), but which held 100,000 frames of heap. Each
    // frame now also counts its overhead.
    let rig = Rig::with_limits(small_limits());
    let id = rig.add(MIME);
    rig.append_ok(id, &init());
    let outcome = rig.append(id, &tiny_samples(100_000, 0));
    assert_eq!(outcome, Outcome::Error(AppendErrorKind::QuotaExceeded));
    // Whatever was stored fits the quota counted the same way.
    let held = frames_held(&*rig.backend);
    assert!(
        held * (1 + FRAME_OVERHEAD_BYTES) <= small_limits().video_bytes,
        "{held} frames are held against a quota of {} bytes",
        small_limits().video_bytes
    );
}

#[test]
fn s1_tiny_samples_appended_a_little_at_a_time_hit_the_quota_too() {
    // 200 frames per append (51 KB counted): the third append finds the buffer full, and evicts
    // nothing it may not (the position is 0), so the page gets QuotaExceededError.
    let rig = Rig::with_limits(small_limits());
    let id = rig.add(MIME);
    rig.append_ok(id, &init());
    rig.append_ok(id, &tiny_samples(200, 0));
    let mut refused = false;
    for i in 1..20u64 {
        match rig.append(id, &tiny_samples(200, i * 200)) {
            Outcome::Done => {}
            Outcome::Refused(MseError::QuotaExceeded(_))
            | Outcome::Error(AppendErrorKind::QuotaExceeded) => {
                refused = true;
                break;
            }
            other => panic!("append {i}: {other:?}"),
        }
    }
    assert!(
        refused,
        "4,000 frames were accepted against a 128 KiB quota"
    );
}

#[test]
fn s2_the_first_append_cannot_pass_what_a_source_buffer_may_hold() {
    let rig = Rig::with_limits(small_limits());
    let id = rig.add(MIME);
    let max = small_limits().max_append_bytes();
    assert_eq!(rig.backend.max_append_bytes(), max);
    // No initialization segment yet, so there is no track buffer for the old check to look at: an
    // append of more than the buffers could ever hold is refused at once, before it is parsed.
    let refused = rig.backend.append(id, vec![0u8; max + 1]);
    assert!(
        matches!(refused, Err(MseError::QuotaExceeded(_))),
        "{refused:?}"
    );
    // The refusal leaves the source buffer usable.
    rig.append_ok(id, &init());
}

#[test]
fn s2_an_init_segment_and_a_mountain_of_media_in_one_append_stops_at_the_quota() {
    // 40,000 samples (40 KB) pass the size check (under 192 KiB) but are 10 MB of frames.
    let rig = Rig::with_limits(small_limits());
    let id = rig.add(MIME);
    let mut one = init();
    one.extend_from_slice(&tiny_samples(40_000, 0));
    let outcome = rig.append(id, &one);
    assert_eq!(outcome, Outcome::Error(AppendErrorKind::QuotaExceeded));
    let held = frames_held(&*rig.backend);
    assert!(
        held * (1 + FRAME_OVERHEAD_BYTES) <= small_limits().video_bytes,
        "{held} frames held"
    );
}

#[test]
fn s3_all_the_media_sources_of_a_process_share_one_budget() {
    // Room for about 400 tiny frames in all, whatever each source's own quota says.
    let budget = MemoryBudget::new(400 * (1 + FRAME_OVERHEAD_BYTES));
    let make = || {
        let rec = Recorder::new();
        let backend = Arc::new(MseBackend::with_budget(
            rec.sink(),
            Arc::new(AllDecoders),
            Limits::default(),
            Arc::clone(&budget),
        ));
        backend.open();
        (Rig { backend, rec },)
    };
    let (a,) = make();
    let (b,) = make();
    let ida = a.add(MIME);
    let idb = b.add(MIME);
    a.append_ok(ida, &init());
    b.append_ok(idb, &init());
    a.append_ok(ida, &tiny_samples(300, 0));
    assert_eq!(budget.used(), 300 * (1 + FRAME_OVERHEAD_BYTES));
    // The second page's 300 frames do not fit what is left, and the first page's data is not its to
    // evict: it gets the quota error.
    let outcome = b.append(idb, &tiny_samples(300, 0));
    assert_eq!(outcome, Outcome::Error(AppendErrorKind::QuotaExceeded));
    assert!(
        budget.used() <= budget.cap(),
        "{} > {}",
        budget.used(),
        budget.cap()
    );
    // Removing the first page's data (or dropping its MediaSource) gives the bytes back at once.
    a.remove_ok(ida, 0.0, 1.0);
    assert_eq!(budget.used(), 0, "a remove frees its bytes at once");
    // The failed append ended the stream (the append error algorithm): a new page's MediaSource is
    // what continues.
    let (c,) = make();
    let idc = c.add(MIME);
    c.append_ok(idc, &init());
    c.append_ok(idc, &tiny_samples(300, 0));
    assert_eq!(budget.used(), 300 * (1 + FRAME_OVERHEAD_BYTES));
    drop(c);
    let deadline = Instant::now() + Duration::from_secs(3);
    while budget.used() != 0 {
        assert!(
            Instant::now() < deadline,
            "dropping a backend freed nothing"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn s3_the_first_append_of_a_source_buffer_checks_the_shared_budget_too() {
    let budget = MemoryBudget::new(10 * 1024);
    let rec = Recorder::new();
    let backend = Arc::new(MseBackend::with_budget(
        rec.sink(),
        Arc::new(AllDecoders),
        Limits::default(),
        Arc::clone(&budget),
    ));
    backend.open();
    let rig = Rig { backend, rec };
    let id = rig.add(MIME);
    let refused = rig.backend.append(id, vec![0u8; 20 * 1024]);
    assert!(
        matches!(refused, Err(MseError::QuotaExceeded(_))),
        "{refused:?}"
    );
}

#[test]
fn a_remove_gives_the_overhead_back_not_only_the_coded_bytes() {
    let budget = MemoryBudget::new(usize::MAX / 2);
    let rec = Recorder::new();
    let backend = Arc::new(MseBackend::with_budget(
        rec.sink(),
        Arc::new(AllDecoders),
        Limits::default(),
        Arc::clone(&budget),
    ));
    backend.open();
    let rig = Rig { backend, rec };
    let id = rig.add(MIME);
    rig.append_ok(id, &init());
    rig.append_ok(id, &tiny_samples(100, 0));
    assert_eq!(budget.used(), 100 * (1 + FRAME_OVERHEAD_BYTES));
    rig.remove_ok(id, 0.0, 1.0);
    assert_eq!(budget.used(), 0);
}
