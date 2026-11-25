// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T06 at processor level: memory and quota. Ten minutes of tiled frames against small limits: the held
//! bytes stay at the limit, eviction never touches `[position - keep_behind, ...)`, the buffer-full
//! flag is the answer when nothing can be evicted, and `remove` gives the bytes back at once.

mod frames_support;

mod frames_quota {
    use super::frames_support::*;
    use mercury_mse::{Limits, SourceBufferProcessor, TrackKind};
    use std::collections::BTreeSet;

    const V: TrackKind = TrackKind::Video;
    const A: TrackKind = TrackKind::Audio;

    /// One 2 s segment of 25 fps video: one group, 8000 + 49 * 1000 coded bytes and 50 frames' overhead.
    const SEGMENT_BYTES: usize = 57_000 + 50 * OV;

    fn segment(index: i64) -> Vec<mercury_mse::CodedFrame> {
        video_25fps(index * 2, 2)
    }

    fn limits(video_bytes: usize) -> Limits {
        Limits {
            video_bytes,
            audio_bytes: 100_000,
            text_bytes: 1_000,
            keep_behind: 5 * SEC,
            keep_ahead: 30 * SEC,
        }
    }

    /// The decode timestamps of the frames in the track whose presentation time is at or after `from`.
    fn frames_from(p: &SourceBufferProcessor, kind: TrackKind, from: i64) -> BTreeSet<i64> {
        p.track_buffer(kind)
            .unwrap()
            .iter()
            .filter(|(_, sf)| sf.frame.pts >= from)
            .map(|(k, _)| k.0)
            .collect()
    }

    #[test]
    fn t06_ten_minutes_of_tiled_frames_stay_at_the_limit() {
        const CAP: usize = 1_000_000;
        let mut p = processor_with(&[V], limits(CAP));
        let mut evicted_total = 0usize;
        let mut steady_min = usize::MAX;
        for i in 0..300i64 {
            // the page keeps about 20 s buffered ahead of the playback position
            let position = (i * 2 * SEC - 20 * SEC).max(0);
            let keep_from = position - 5 * SEC;
            let protected = frames_from(&p, V, keep_from);
            let report = p.evict_report(SEGMENT_BYTES, position);
            assert!(
                report.ok,
                "segment {i}: nothing to evict although data lies behind"
            );
            evicted_total += report.removed.bytes;
            // eviction never touches [position - 5 s, ...)
            let after = frames_from(&p, V, keep_from);
            assert_eq!(
                after, protected,
                "segment {i}: evicted data within keep_behind"
            );
            p.process_frames(segment(i)).unwrap();
            assert!(
                p.bytes() <= CAP,
                "segment {i}: {} bytes over {CAP}",
                p.bytes()
            );
            if i >= 40 {
                steady_min = steady_min.min(p.bytes());
            }
        }
        // whole groups only, so the buffer stays within two groups of the limit once it is full
        assert!(
            steady_min > CAP - 2 * SEGMENT_BYTES,
            "steady state holds only {steady_min} bytes"
        );
        assert_eq!(p.bytes() + evicted_total, 300 * SEGMENT_BYTES);
        // what is left is one range, starting at a keyframe, that covers the keep-behind line
        let ranges = p.buffered();
        assert_eq!(ranges.len(), 1);
        // the last position was 578 s, so the keep-behind line is at 573 s and the buffer starts before it
        assert!(ranges[0].start <= 573 * SEC);
        assert_eq!(ranges[0].end, 600 * SEC);
        let first = p
            .track_buffer(V)
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .1
            .frame
            .keyframe;
        assert!(first, "the oldest frame kept is a keyframe");
        p.check_invariants().unwrap();
    }

    #[test]
    fn t06_the_buffer_full_flag_when_nothing_can_be_evicted() {
        const CAP: usize = 1_000_000;
        // the playback is stuck at 0: everything buffered is within keep_ahead of it or in its range
        let mut p = processor_with(&[V], limits(CAP));
        let mut appended = 0;
        let mut first_refusal = None;
        for i in 0..40i64 {
            if !p.evict(SEGMENT_BYTES, 0) {
                first_refusal = Some(i);
                break;
            }
            p.process_frames(segment(i)).unwrap();
            appended += 1;
        }
        // 14 segments fit (977,200); the 15th needs 1,047,000
        assert_eq!(first_refusal, Some(14));
        assert_eq!(appended, 14);
        assert_eq!(p.bytes(), 14 * SEGMENT_BYTES);
        assert!(p.bytes() <= CAP);
        assert!(!p.has_room(SEGMENT_BYTES));
        // the page removes the first 10 s: the bytes are back at once
        let before = p.bytes();
        let removed = p.remove(0, Some(10 * SEC));
        assert_eq!(removed.bytes, 5 * SEGMENT_BYTES);
        assert_eq!(p.bytes(), before - 5 * SEGMENT_BYTES);
        assert!(p.has_room(SEGMENT_BYTES));
        assert!(p.evict(SEGMENT_BYTES, 10 * SEC), "room again");
        assert_eq!(
            p.bytes(),
            before - 5 * SEGMENT_BYTES,
            "nothing needed to go"
        );
        p.process_frames(segment(14)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(10_000, 30_000)]);
    }

    #[test]
    fn t06_an_append_bigger_than_the_capacity_never_fits() {
        let mut p = processor_with(&[V], limits(100_000));
        assert!(!p.evict(100_001, 0));
        assert!(p.evict(100_000, 0));
        p.process_frames(segment(0)).unwrap();
        assert!(
            !p.evict(100_000, 0),
            "57,000 are held and nothing is behind"
        );
    }

    #[test]
    fn t06_the_tail_beyond_keep_ahead_is_evicted_when_nothing_behind_can_go() {
        const CAP: usize = 1_000_000;
        let mut p = processor_with(&[V], limits(CAP));
        // a far range from before a seek: [600, 620) s
        for i in 300..310i64 {
            p.process_frames(segment(i)).unwrap();
        }
        // the page is back at 0 and has buffered [0, 10)
        for i in 0..5i64 {
            p.process_frames(segment(i)).unwrap();
        }
        assert_eq!(ms_ranges(&p.buffered()), [(0, 10_000), (600_000, 620_000)]);
        assert_eq!(p.bytes(), 15 * SEGMENT_BYTES);
        // room for 5 more segments: 15 held + 5 incoming over the quota of 14.3 segments needs 396,000
        // bytes freed, so 6 groups of the far range go (whole groups, from its end)
        let report = p.evict_report(5 * SEGMENT_BYTES, 0);
        assert!(report.ok);
        assert_eq!(report.removed.bytes, 6 * SEGMENT_BYTES);
        assert_eq!(report.removed.frames, 300);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 10_000), (600_000, 608_000)]);
        p.check_invariants().unwrap();
    }

    #[test]
    fn t06_the_tail_of_the_range_being_appended_to_is_left_alone() {
        // [0, 40 s) is one range with the append frontier at its end and the position inside it: its tail
        // lies beyond keep_ahead (30 s) but evicting it would leave a hole at the page's next segment
        const CAP: usize = 1_000_000;
        let mut p = processor_with(&[V], limits(CAP));
        for i in 0..14i64 {
            p.process_frames(segment(i)).unwrap();
        }
        let before = p.bytes();
        assert!(!p.evict(SEGMENT_BYTES, 0));
        assert_eq!(p.bytes(), before);
        // with the position in the middle there is data behind keep_behind to go instead
        let report = p.evict_report(SEGMENT_BYTES, 20 * SEC);
        assert!(report.ok);
        assert_eq!(report.removed.bytes, SEGMENT_BYTES);
        assert_eq!(ms_ranges(&p.buffered()), [(2_000, 28_000)]);
    }

    #[test]
    fn t06_eviction_takes_whole_groups_from_the_start_and_keeps_keyframes_first() {
        let mut p = processor_with(&[V], limits(560_000));
        for i in 0..8i64 {
            p.process_frames(segment(i)).unwrap();
        }
        // position 12 s: behind 7 s are free to go (groups ending at or before 7 s: 0-2, 2-4, 4-6)
        let report = p.evict_report(2 * SEGMENT_BYTES, 12 * SEC);
        assert!(report.ok);
        // 558,400 held + 139,600 incoming - 560,000 = 138,000 to free: two groups of 69,800
        assert_eq!(report.removed.bytes, 2 * SEGMENT_BYTES);
        assert_eq!(report.removed.frames, 100);
        assert_eq!(ms_ranges(&p.buffered()), [(4_000, 16_000)]);
        let first = p.track_buffer(V).unwrap().iter().next().unwrap().1;
        assert!(first.frame.keyframe);
        assert_eq!(first.frame.pts, 4 * SEC);
    }

    #[test]
    fn t06_a_group_that_straddles_the_keep_behind_line_stays_whole() {
        let mut p = processor_with(&[V], limits(350_000));
        for i in 0..5i64 {
            p.process_frames(segment(i)).unwrap();
        }
        // position 7 s: the line is at 2 s; the group [0, 2) ends exactly there and may go, [2, 4) may not
        let report = p.evict_report(SEGMENT_BYTES, 7 * SEC);
        assert!(report.ok);
        assert_eq!(ms_ranges(&p.buffered()), [(2_000, 10_000)]);
        // position 6.9 s: the line is at 1.9 s, no group ends before it: nothing goes
        let mut q = processor_with(&[V], limits(350_000));
        for i in 0..5i64 {
            q.process_frames(segment(i)).unwrap();
        }
        assert!(!q.evict(SEGMENT_BYTES, secs(6.9)));
        assert_eq!(ms_ranges(&q.buffered()), [(0, 10_000)]);
    }

    #[test]
    fn t06_each_kind_has_its_own_quota() {
        // audio holds 71,200 counted bytes (200 frames of 100 bytes and their overhead), video a lot more
        let mut p = processor_with(
            &[V, A],
            Limits {
                video_bytes: 50_000_000,
                audio_bytes: 200 * (100 + OV),
                ..limits(0)
            },
        );
        p.process_frames(video_25fps(0, 8)).unwrap();
        // 300 audio frames of 100 bytes, 20 ms each = 6 s, 300 * (100 + OV) counted: over the audio quota
        p.process_frames(audio(0, 300, 20 * MS, 100)).unwrap();
        assert_eq!(p.track_bytes(A), 300 * (100 + OV));
        let video_before = p.track_bytes(V);
        let report = p.evict_report(0, 8 * SEC);
        assert!(report.ok);
        assert_eq!(
            report.removed.bytes,
            100 * (100 + OV),
            "audio only, back to its quota"
        );
        assert_eq!(p.track_bytes(A), 200 * (100 + OV));
        assert_eq!(p.track_bytes(V), video_before);
        // the audio lost its start
        assert_eq!(ms_ranges(&p.track_buffered(A)), [(2000, 6000)]);
    }

    #[test]
    fn t06_evict_with_nothing_buffered_is_fine() {
        let mut empty = SourceBufferProcessor::new(Limits::default());
        assert!(
            empty.evict(1 << 30, 0),
            "no track buffers yet: nothing to evict"
        );
        let mut p = processor(&[V]);
        assert!(p.evict(0, 0));
        assert!(p.evict(10, 0));
        assert_eq!(p.capacity(), Limits::default().video_bytes);
        assert!(p.has_room(1000));
        assert!(!p.is_full());
    }

    #[test]
    fn t06_removing_the_last_frame_by_eviction_resets_the_group_state() {
        // position far to the right of an old range, and room wanted for a whole buffer: everything goes,
        // including the frame that was appended last, so the group is over and the next frame must be a
        // random access point
        const CAP: usize = 400_000;
        let mut p = processor_with(&[V], limits(CAP));
        for i in 0..6i64 {
            p.process_frames(segment(i)).unwrap();
        }
        let held = p.bytes();
        let report = p.evict_report(CAP, 200 * SEC);
        assert!(report.ok);
        assert_eq!(report.removed.bytes, held);
        assert_eq!(p.bytes(), 0);
        assert!(p.buffered().is_empty());
        // a stray delta frame continuing the old group is dropped
        let mut stray = video(12 * SEC, 2, 40 * MS, 50, 8000, 1000);
        stray[0].keyframe = false;
        let r = p.process_frames(stray).unwrap();
        assert_eq!((r.accepted, r.dropped), (0, 2));
        // and a keyframe starts the next group
        let r = p.process_frames(video_25fps(200, 2)).unwrap();
        assert_eq!(r.accepted, 50);
        p.check_invariants().unwrap();
    }

    #[test]
    fn t06_bytes_are_exact_after_every_operation() {
        let mut p = processor_with(&[V, A], Limits::default());
        let mut expected = 0usize;
        for i in 0..10i64 {
            let v = segment(i);
            let a = audio(i * 2 * SEC, 100, 20 * MS, 200);
            expected += v.iter().map(|f| f.data.len() + OV).sum::<usize>();
            expected += a.iter().map(|f| f.data.len() + OV).sum::<usize>();
            p.process_frames(v).unwrap();
            p.process_frames(a).unwrap();
            assert_eq!(p.bytes(), expected);
        }
        let removed = p.remove(4 * SEC, Some(12 * SEC));
        expected -= removed.bytes;
        assert_eq!(p.bytes(), expected);
        p.remove(0, None);
        assert_eq!(p.bytes(), 0);
    }
}
