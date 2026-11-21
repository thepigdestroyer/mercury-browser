// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The cost of a long buffer: a 4 h live video buffer is about 350,000 frames. Insertion at the end and
//! removal at the front must stay cheap however long the buffer is, and asking for the buffered ranges
//! must never scan the frames. The numbers are printed (`--nocapture`) and recorded in the lane notes;
//! the assertions are loose enough for a loaded machine and tight enough to catch a quadratic
//! algorithm (which would take minutes here).

mod frames_support;

mod frames_perf {
    use super::frames_support::*;
    use mercury_mse::{Limits, SourceBufferProcessor, StoredFrame, TrackKind};
    use std::time::{Duration, Instant};

    const V: TrackKind = TrackKind::Video;
    const FRAME: i64 = 41_666_667; // 24 fps, in ns

    fn live(limits: Limits) -> SourceBufferProcessor {
        processor_with(&[V], limits)
    }

    /// `count` frames of 24 fps video from frame `first`, a keyframe every 48 frames (2 s), sharing one
    /// payload so that the test measures the store and not the allocator.
    fn frames(
        first: u64,
        count: usize,
        data: &std::sync::Arc<[u8]>,
    ) -> Vec<mercury_mse::CodedFrame> {
        (first..first + count as u64)
            .map(|i| {
                let t = (i as i64) * FRAME;
                mercury_mse::CodedFrame {
                    kind: V,
                    track_id: 1,
                    pts: t,
                    dts: t,
                    duration: FRAME,
                    keyframe: i % 48 == 0,
                    data: data.clone(),
                }
            })
            .collect()
    }

    #[test]
    fn a_four_hour_live_buffer_of_350000_frames_stays_cheap() {
        const TOTAL: u64 = 350_000;
        const SEGMENT: usize = 48;
        let data = payload(100);
        let mut p = live(Limits {
            video_bytes: usize::MAX / 2,
            ..Limits::default()
        });

        // insertion at the end, one 2 s segment at a time
        let started = Instant::now();
        let mut worst = Duration::ZERO;
        let mut i = 0u64;
        while i < TOTAL {
            let seg = frames(i, SEGMENT, &data);
            let t = Instant::now();
            let r = p.process_frames(seg).unwrap();
            worst = worst.max(t.elapsed());
            assert_eq!(r.dropped, 0);
            i += SEGMENT as u64;
        }
        let insert_total = started.elapsed();
        let frame_count = p.frame_count(V);
        println!(
            "insert: {frame_count} frames in {insert_total:?} ({:.2} us per frame), slowest 2 s segment {worst:?}",
            insert_total.as_secs_f64() * 1e6 / frame_count as f64
        );
        assert!(
            insert_total < Duration::from_secs(20),
            "insertion took {insert_total:?}"
        );
        assert!(
            worst < Duration::from_millis(200),
            "one segment took {worst:?}"
        );
        assert_eq!(p.buffered().len(), 1);
        println!(
            "stored frame: {} bytes in each index entry plus 24 for the presentation index",
            std::mem::size_of::<StoredFrame>() + 16
        );

        // buffered(), highest_end() and the cursor never scan the frames
        let t = Instant::now();
        for _ in 0..100_000 {
            std::hint::black_box(p.buffered());
            std::hint::black_box(p.highest_end());
        }
        let queries = t.elapsed();
        println!("100,000 x (buffered + highest_end) on {frame_count} frames: {queries:?}");
        assert!(queries < Duration::from_secs(2), "{queries:?}");

        let mid = 7_200 * SEC;
        let t = Instant::now();
        let mut total = 0;
        for k in 0..1000 {
            let items = {
                let mut cursor = p.cursor(V, mid + k * SEC / 10);
                let mut out = Vec::new();
                p.read(V, &mut cursor, 32, &mut out);
                out
            };
            total += items.len();
        }
        let seeks = t.elapsed();
        println!("1,000 seeks to the middle and 32 frames each: {seeks:?} ({total} items)");
        assert!(seeks < Duration::from_secs(2), "{seeks:?}");

        // the front: remove one segment at a time, the way a live player trims behind the position
        let t = Instant::now();
        let mut worst = Duration::ZERO;
        let mut freed = 0usize;
        for k in 1..=1000i64 {
            let step = Instant::now();
            let removed = p.remove(0, Some(k * 2 * SEC));
            worst = worst.max(step.elapsed());
            freed += removed.frames;
        }
        let front = t.elapsed();
        println!(
            "front removal: 1,000 x remove(0, t) freed {freed} frames in {front:?}, slowest {worst:?}"
        );
        assert!(freed >= 48_000);
        assert!(worst < Duration::from_millis(100), "{worst:?}");

        // eviction of a group at a time with the buffer exactly full: the steady state of a long live
        // session (every append needs one group's bytes, the oldest group goes)
        let held = p.bytes();
        p.set_limits(Limits {
            video_bytes: held,
            ..Limits::default()
        });
        let t = Instant::now();
        let mut evicted = 0;
        let mut next = TOTAL;
        for _ in 0..200 {
            let end = p.highest_end().unwrap();
            let report = p.evict_report(48 * 100, end - 60 * SEC);
            assert!(report.ok);
            evicted += report.removed.frames;
            p.process_frames(frames(next, 48, &data)).unwrap();
            next += 48;
        }
        println!(
            "200 appends into a full buffer of {held} bytes: {evicted} frames evicted in {:?}",
            t.elapsed()
        );
        assert_eq!(evicted, 200 * 48);
        p.check_invariants().unwrap();
    }

    #[test]
    fn a_long_live_session_with_a_sliding_window_does_not_slow_down() {
        // 12 hours of 2 s segments appended in a row with the oldest removed so that 1 h stays:
        // the time per step must not grow with the session length
        let data = payload(100);
        let mut p = live(Limits {
            video_bytes: usize::MAX / 2,
            ..Limits::default()
        });
        let mut slice_times = Vec::new();
        let mut i = 0u64;
        for slice in 0..12 {
            let t = Instant::now();
            for _ in 0..2_160 {
                p.process_frames(frames(i, 48, &data)).unwrap();
                i += 48;
                let now = (i as i64) * FRAME;
                p.remove(0, Some(now - 3_600 * SEC));
            }
            slice_times.push(t.elapsed());
            println!(
                "hour {slice}: {:?}, {} frames held",
                slice_times[slice],
                p.frame_count(V)
            );
        }
        // the medians of the first and the second half of the session (one slow hour is machine noise)
        let median = |slices: &[Duration]| {
            let mut v: Vec<f64> = slices.iter().map(Duration::as_secs_f64).collect();
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let early = median(&slice_times[..6]);
        let late = median(&slice_times[6..]);
        assert!(
            late < early * 3.0 + 0.2,
            "slowed down: {early:.3} s then {late:.3} s per hour"
        );
        assert!(p.frame_count(V) < 100_000);
        assert_eq!(p.buffered().len(), 1);
        p.check_invariants().unwrap();
    }
}
