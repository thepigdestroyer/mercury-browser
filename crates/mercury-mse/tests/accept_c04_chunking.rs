// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C04 of the GstMSE spike at core level (lane med-1, item 3): the same stream appended under every
//! chunk plan (whole, 1 KiB, 64 KiB, mid-box and mid-sample cuts, one byte at a time) gives the same
//! `buffered`, the same duration, the same events and the same frames from the feed, and exactly one
//! completion event per append.

mod model_support;

use mercury_mse::api::{
    FeedState, InitInfo, MediaSourceBackend, MseEvent, Ns, SourceBufferId, TrackKind,
};
use mercury_mse_testkit::{ChunkPlan, chunks};
use model_support::*;

/// Everything about a played-through stream that must not depend on the chunking.
#[derive(Debug, PartialEq)]
struct Summary {
    buffered: Vec<(Ns, Ns)>,
    element_buffered: Vec<(Ns, Ns)>,
    duration: Option<f64>,
    inits: Vec<InitInfo>,
    duration_events: Vec<Option<f64>>,
    ended_events: usize,
    /// (kind, pts, dts, duration, keyframe, a checksum of the bytes) of every frame served.
    frames: Vec<(TrackKind, Ns, Ns, Ns, bool, u64)>,
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
    })
}

fn pieces(stream: &mercury_mse_testkit::SegmentedStream, plan: &ChunkPlan) -> usize {
    chunks(&stream.init.bytes, plan).len()
        + stream
            .media
            .iter()
            .map(|s| chunks(&s.bytes, plan).len())
            .sum::<usize>()
}

fn run(name: &str, plan: &ChunkPlan) -> Summary {
    let stream = load(name);
    let rig = Rig::new();
    let id: SourceBufferId = rig.add(&stream.mime);
    rig.append_stream(id, &stream, plan);
    // One completion event per accepted append, whatever the cuts.
    let expected = pieces(&stream, plan);
    assert_eq!(
        rig.rec.completions(id),
        expected,
        "{name} under {plan:?}: completions"
    );
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendDone { .. })),
        expected,
        "{name} under {plan:?}: every append is a success"
    );
    let duration_before = rig.backend.duration();
    let duration_events_before = rig.rec.count(|e| matches!(e, MseEvent::DurationChanged(_)));
    rig.backend.end_of_stream(None).unwrap();
    let duration_changes = usize::from(rig.backend.duration() != duration_before);
    // Let the worker deliver the events of the end of stream: SourceEnded, then the duration change.
    assert!(rig.rec.wait_for(WAIT, |log| {
        log.iter()
            .any(|r| matches!(r.event, MseEvent::SourceEnded { .. }))
            && log
                .iter()
                .filter(|r| matches!(r.event, MseEvent::DurationChanged(_)))
                .count()
                >= duration_events_before + duration_changes
    }));
    let feed = rig.backend.frame_feed();
    let mut frames = Vec::new();
    for kind in TrackKind::ALL {
        let d = drain(&*feed, kind);
        if matches!(kind, TrackKind::Text) {
            assert!(d.frames.is_empty());
            continue;
        }
        assert_eq!(d.state, FeedState::Ended, "{name} {kind:?}");
        frames.extend(d.frames.iter().map(|f| {
            (
                f.kind,
                f.pts,
                f.dts,
                f.duration,
                f.keyframe,
                checksum(&f.data),
            )
        }));
    }
    let events = rig.rec.events();
    let to_ns = |r: &mercury_mse::api::TimeRange| (r.start, r.end);
    Summary {
        buffered: rig.backend.buffered(id).iter().map(to_ns).collect(),
        element_buffered: rig.backend.element_buffered().iter().map(to_ns).collect(),
        duration: rig.backend.duration(),
        inits: events
            .iter()
            .filter_map(|e| match e {
                MseEvent::InitSegment { init, .. } => Some(init.clone()),
                _ => None,
            })
            .collect(),
        duration_events: events
            .iter()
            .filter_map(|e| match e {
                MseEvent::DurationChanged(d) => Some(*d),
                _ => None,
            })
            .collect(),
        ended_events: events
            .iter()
            .filter(|e| matches!(e, MseEvent::SourceEnded { .. }))
            .count(),
        frames,
    }
}

fn plans_for_big() -> Vec<ChunkPlan> {
    vec![
        ChunkPlan::Whole,
        ChunkPlan::Fixed(1024),
        ChunkPlan::Fixed(65_536),
        ChunkPlan::Sizes(vec![13, 4096, 1]),
    ]
}

fn same_under_every_plan(name: &str, plans: Vec<ChunkPlan>, expect_frames: usize) {
    let baseline = run(name, &ChunkPlan::Whole);
    assert_eq!(baseline.frames.len(), expect_frames, "{name}: frames");
    assert_eq!(baseline.inits.len(), 1, "{name}: one init segment event");
    assert_eq!(baseline.ended_events, 1);
    for plan in plans {
        if plan == ChunkPlan::Whole {
            continue;
        }
        let got = run(name, &plan);
        assert_eq!(
            got, baseline,
            "{name} under {plan:?} differs from the whole-segment run"
        );
    }
}

#[test]
fn c04_muxed_fmp4_under_every_chunk_plan() {
    same_under_every_plan("h264-aac", plans_for_big(), 200 + 375);
}

#[test]
fn c04_webm_under_every_chunk_plan() {
    same_under_every_plan("vp9-low", plans_for_big(), 200);
}

#[test]
fn c04_audio_only_webm_under_every_chunk_plan_including_one_byte() {
    let mut plans = plans_for_big();
    plans.push(ChunkPlan::Fixed(1));
    same_under_every_plan("opus", plans, 400);
}

#[test]
fn c04_video_fmp4_one_byte_at_a_time() {
    same_under_every_plan("h264-low", vec![ChunkPlan::Fixed(1)], 200);
}

#[test]
fn c04_bframes_keep_their_composition_offsets_under_chunking() {
    let whole = run("h264-bframes", &ChunkPlan::Whole);
    assert!(
        whole.frames.iter().any(|f| f.1 != f.2),
        "the B-frame stream has pts != dts"
    );
    let cut = run("h264-bframes", &ChunkPlan::Sizes(vec![13, 4096, 1]));
    assert_eq!(whole, cut);
}

#[test]
fn c04_an_init_segment_cut_in_two_is_one_init_event() {
    // The init segment split at an awkward byte: the first piece parses to nothing (still an
    // accepted append with its own AppendDone), the second one completes the segment.
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    let cut = 7;
    rig.append_ok(id, &s.init.bytes[..cut]);
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::InitSegment { .. })),
        0,
        "no init event for 7 bytes"
    );
    rig.append_ok(id, &s.init.bytes[cut..]);
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::InitSegment { .. })),
        1
    );
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendDone { .. })),
        2
    );
}
