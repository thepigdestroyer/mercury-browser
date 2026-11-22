// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Discontinuities (the spec's decode-timestamp rules), streams with B-frames, 90 kHz and 48 kHz timescales
//! without drift over a million frames, tolerance of tick rounding, and timestamps at the edge of the
//! numeric range.

mod frames_support;

mod frames_timing {
    use super::frames_support::*;
    use mercury_mse::{AppendMode, CodedFrame, TrackKind};
    use std::sync::Arc;

    const V: TrackKind = TrackKind::Video;
    const A: TrackKind = TrackKind::Audio;
    const T: TrackKind = TrackKind::Text;

    // ---- discontinuities ----

    #[test]
    fn a_gap_of_exactly_twice_the_frame_duration_is_not_a_discontinuity() {
        let mut p = processor(&[V]);
        p.process_frames(video(0, 25, 40 * MS, 50, 8000, 1000))
            .unwrap();
        // the last frame starts at 960 ms and lasts 40 ms; this delta frame starts 80 ms after it
        let mut next = video(960 * MS + 80 * MS, 1, 40 * MS, 50, 8000, 1000);
        next[0].keyframe = false;
        let r = p.process_frames(next).unwrap();
        assert_eq!(
            (r.accepted, r.dropped),
            (1, 0),
            "2 x the duration continues the group"
        );
    }

    #[test]
    fn a_gap_beyond_twice_the_frame_duration_starts_a_new_group() {
        let mut p = processor(&[V]);
        p.process_frames(video(0, 25, 40 * MS, 50, 8000, 1000))
            .unwrap();
        let mut next = video(960 * MS + 80 * MS + 1, 1, 40 * MS, 50, 8000, 1000);
        next[0].keyframe = false;
        let r = p.process_frames(next).unwrap();
        assert_eq!(
            (r.accepted, r.dropped),
            (0, 1),
            "a new group needs a keyframe"
        );
        // in segments mode the group end timestamp becomes the presentation time of that frame
        assert_eq!(p.group_end_timestamp(), 1_040 * MS + 1);
        // a keyframe starts it
        let r = p
            .process_frames(video(2 * SEC, 5, 40 * MS, 50, 8000, 1000))
            .unwrap();
        assert_eq!(r.accepted, 5);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 1000), (2000, 2200)]);
    }

    #[test]
    fn a_decode_timestamp_that_goes_back_starts_a_new_group() {
        // segments appended out of order: [4, 6) then [0, 2)
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(4, 2)).unwrap();
        let r = p.process_frames(video_25fps(0, 2)).unwrap();
        assert_eq!((r.accepted, r.dropped), (50, 0));
        assert_eq!(ms_ranges(&p.buffered()), [(0, 2000), (4000, 6000)]);
        // one frame back in the middle of a group is a discontinuity too: the delta frame is dropped
        let mut q = processor(&[V]);
        q.process_frames(video(0, 25, 40 * MS, 50, 8000, 1000))
            .unwrap();
        let mut back = video(400 * MS, 1, 40 * MS, 50, 8000, 1000);
        back[0].keyframe = false;
        let r = q.process_frames(back).unwrap();
        assert_eq!((r.accepted, r.dropped), (0, 1));
    }

    #[test]
    fn a_discontinuity_in_one_track_makes_every_track_wait_for_a_keyframe() {
        let mut p = processor(&[V, A]);
        p.process_frames(video(0, 25, 40 * MS, 50, 8000, 1000))
            .unwrap();
        p.process_frames(audio(0, 50, 20 * MS, 100)).unwrap();
        // audio jumps ahead by 4 s: the group is over for the video as well
        p.process_frames(audio(5 * SEC, 10, 20 * MS, 100)).unwrap();
        let mut delta = video(SEC, 3, 40 * MS, 50, 8000, 1000);
        for f in &mut delta {
            f.keyframe = false;
        }
        let r = p.process_frames(delta).unwrap();
        assert_eq!((r.accepted, r.dropped), (0, 3));
        // the video continues at its next keyframe
        let r = p
            .process_frames(video(SEC, 3, 40 * MS, 1, 8000, 1000))
            .unwrap();
        assert_eq!(r.accepted, 3);
    }

    #[test]
    fn frames_of_a_track_the_source_buffer_does_not_have_are_dropped() {
        let mut p = processor(&[V]);
        let r = p.process_frames(audio(0, 10, 20 * MS, 10)).unwrap();
        assert_eq!((r.accepted, r.dropped), (0, 10));
        assert!(p.buffered().is_empty());
    }

    #[test]
    fn text_tracks_count_for_the_highest_end_but_not_for_the_intersection() {
        let mut p = processor(&[V, T]);
        p.process_frames(video_25fps(0, 4)).unwrap();
        // two cues: [0.5, 1.5) and [9, 10)
        let cue = |start: i64, end: i64| CodedFrame {
            kind: T,
            track_id: 3,
            pts: start,
            dts: start,
            duration: end - start,
            keyframe: true,
            data: payload(20),
        };
        p.process_frames(vec![cue(secs(0.5), secs(1.5)), cue(9 * SEC, 10 * SEC)])
            .unwrap();
        assert_eq!(
            p.highest_end(),
            Some(10 * SEC),
            "the cue counts for the highest end time"
        );
        assert_eq!(
            ms_ranges(&p.buffered()),
            [(0, 4000)],
            "but not for the intersection"
        );
        assert_eq!(
            ms_ranges(&p.track_buffered(T)),
            [(500, 1500), (9000, 10000)]
        );
        // a source buffer with text only reports the text ranges
        let mut q = processor(&[T]);
        q.process_frames(vec![cue(0, SEC)]).unwrap();
        assert_eq!(ms_ranges(&q.buffered()), [(0, 1000)]);
    }

    // ---- buffered with several tracks ----

    #[test]
    fn buffered_is_the_intersection_of_the_tracks() {
        let mut p = processor(&[V, A]);
        p.process_frames(video_25fps(0, 8)).unwrap();
        p.process_frames(audio(0, 350, 20 * MS, 100)).unwrap(); // 7 s
        assert_eq!(ms_ranges(&p.track_buffered(V)), [(0, 8000)]);
        assert_eq!(ms_ranges(&p.track_buffered(A)), [(0, 7000)]);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 7000)]);
        // when the stream has ended the last range of each track stretches to the highest end time
        assert_eq!(ms_ranges(&p.buffered_with(true)), [(0, 8000)]);
        assert_eq!(p.highest_end(), Some(8 * SEC));
        // a hole in one track is a hole in the intersection
        p.remove(SEC, Some(2 * SEC));
        assert_eq!(ms_ranges(&p.buffered()).len(), 2);
    }

    #[test]
    fn a_gap_of_one_frame_is_hidden_and_a_larger_one_is_not() {
        let mut p = processor(&[A]);
        // 100 audio frames of 20 ms, a 20 ms hole, 100 more
        p.process_frames(audio(0, 100, 20 * MS, 10)).unwrap();
        p.process_frames(audio(2 * SEC + 20 * MS, 100, 20 * MS, 10))
            .unwrap();
        assert_eq!(
            ms_ranges(&p.track_buffered(A)),
            [(0, 4020)],
            "one frame of gap is hidden"
        );
        assert_eq!(ms_ranges(&p.buffered()), [(0, 4020)]);
        assert_eq!(
            p.track_ranges_exact(A).len(),
            2,
            "the real ranges keep the hole"
        );
        // a 40 ms hole is not
        let mut q = processor(&[A]);
        q.process_frames(audio(0, 100, 20 * MS, 10)).unwrap();
        q.process_frames(audio(2 * SEC + 40 * MS, 100, 20 * MS, 10))
            .unwrap();
        assert_eq!(ms_ranges(&q.buffered()), [(0, 2000), (2040, 4040)]);
    }

    #[test]
    fn the_gap_tolerance_is_capped() {
        // a slideshow track with 10 s frames does not join ranges 10 s apart
        let mut p = processor(&[V]);
        let long = |start: i64| CodedFrame {
            kind: V,
            track_id: 1,
            pts: start,
            dts: start,
            duration: 10 * SEC,
            keyframe: true,
            data: payload(10),
        };
        p.process_frames(vec![long(0)]).unwrap();
        p.process_frames(vec![long(25 * SEC)]).unwrap();
        // the gap is 15 s: shown as it is (the tolerance is at most 250 ms)
        assert_eq!(ms_ranges(&p.buffered()), [(0, 10_000), (25_000, 35_000)]);
    }

    // ---- B-frames ----

    #[test]
    fn b_frames_buffered_end_is_the_last_presentation_time_plus_its_duration() {
        let mut p = processor(&[V]);
        let frames = bframes(0, 5, 40 * MS, 500);
        assert_eq!(frames.len(), 50);
        let r = p.process_frames(frames.clone()).unwrap();
        assert_eq!(
            (r.accepted, r.dropped),
            (50, 0),
            "no frame of the group is lost to its own reordering"
        );
        // display index 49 shows at (49 + 2) * 40 ms; the end is its presentation time plus its duration
        let last_pts = frames.iter().map(|f| f.pts).max().unwrap();
        assert_eq!(last_pts, 2040 * MS);
        assert_eq!(p.highest_end(), Some(last_pts + 40 * MS));
        assert_eq!(ms_ranges(&p.buffered()), [(80, 2080)]);
        // the feed serves decode order, whose presentation times are not increasing
        let served = frames_of(&read_all(&p, V, 0));
        assert_eq!(served.len(), 50);
        assert!(served.windows(2).all(|w| w[0].dts < w[1].dts));
        assert!(served.windows(2).any(|w| w[0].pts > w[1].pts));
        assert_eq!(
            dts_pts_ms(&served[..4]),
            [(0, 80), (40, 200), (80, 120), (120, 160)]
        );
    }

    #[test]
    fn b_frames_in_chunks_cut_anywhere_give_the_same_result() {
        let frames = bframes(0, 6, 40 * MS, 500);
        let mut whole = processor(&[V]);
        whole.process_frames(frames.clone()).unwrap();
        for size in [1usize, 2, 3, 7, 10, 13] {
            let mut p = processor(&[V]);
            for chunk in frames.chunks(size) {
                p.process_frames(chunk.to_vec()).unwrap();
            }
            assert_eq!(p.buffered(), whole.buffered(), "chunks of {size}");
            assert_eq!(p.frame_count(V), 60, "chunks of {size}");
            assert_eq!(p.bytes(), whole.bytes(), "chunks of {size}");
        }
    }

    #[test]
    fn b_frames_overwritten_by_a_later_append_are_replaced_whole() {
        let mut p = processor(&[V]);
        p.process_frames(bframes(0, 4, 40 * MS, 500)).unwrap();
        // the same two middle groups again (groups 1 and 2 of a stream that starts 400 ms later)
        let again: Vec<_> = bframes(0, 4, 40 * MS, 500)
            .split_off(10)
            .into_iter()
            .take(20)
            .collect();
        let r = p.process_frames(again).unwrap();
        assert_eq!(r.accepted, 20);
        assert_eq!(p.frame_count(V), 40);
        assert_eq!(ms_ranges(&p.buffered()), [(80, 1680)]);
        p.check_invariants().unwrap();
    }

    // ---- timescales: no drift over a million frames ----

    /// Appends `total` frames of a stream in batches, dropping the oldest ones as it goes so that memory
    /// stays small, and checks after every batch that the data is one range that ends where the tick
    /// count says, to within a nanosecond.
    fn no_drift(kind: TrackKind, ticks_per_frame: i64, timescale: u32, gop: u64, total: u64) {
        const BATCH: u64 = 10_000;
        let shared: Arc<[u8]> = payload(8);
        let mut p = processor(&[kind]);
        let mut done = 0u64;
        let mut keep_from = 0u64;
        while done < total {
            let n = BATCH.min(total - done);
            let r = p
                .process_frames(ticked(
                    kind,
                    done,
                    n as usize,
                    ticks_per_frame,
                    timescale,
                    gop,
                    &shared,
                ))
                .unwrap();
            assert_eq!(r.dropped, 0, "frames dropped at {done}");
            done += n;
            let exact_end = ticks_to_ns(done as i64 * ticks_per_frame, timescale);
            let end = p.highest_end().unwrap();
            assert!(
                (end - exact_end).abs() <= 1,
                "drift {} ns after {done} frames",
                end - exact_end
            );
            assert_eq!(
                p.track_ranges_exact(kind).len(),
                1,
                "a tick-rounding gap split the range at {done}"
            );
            assert_eq!(p.buffered().len(), 1);
            // keep the last 50,000 frames: remove from the front up to a group boundary
            if done > keep_from + 60_000 {
                keep_from = ((done - 50_000) / gop) * gop;
                let start = ticks_to_ns(keep_from as i64 * ticks_per_frame, timescale);
                p.remove(0, Some(start));
            }
        }
        let exact_end = ticks_to_ns(total as i64 * ticks_per_frame, timescale);
        assert!((p.highest_end().unwrap() - exact_end).abs() <= 1);
        let first = ticks_to_ns(keep_from as i64 * ticks_per_frame, timescale);
        assert_eq!(p.buffered()[0].start, first);
        p.check_invariants().unwrap();
    }

    #[test]
    fn timescale_90khz_video_of_29_97_fps_does_not_drift_over_a_million_frames() {
        // 3003 ticks of 1/90000 s per frame, a keyframe every 30 frames
        no_drift(V, 3003, 90_000, 30, 1_000_000);
    }

    #[test]
    fn timescale_48khz_aac_does_not_drift_over_a_million_frames() {
        // 1024 samples of 1/48000 s per frame, every frame a random access point
        no_drift(A, 1024, 48_000, 1, 1_000_000);
    }

    #[test]
    fn timescale_44100_hz_and_1000_hz_do_not_drift_either() {
        no_drift(A, 1024, 44_100, 1, 200_000);
        no_drift(V, 33, 1000, 60, 200_000);
    }

    // ---- the edge of the numeric range ----

    #[test]
    fn extreme_timestamps_never_panic() {
        let mut p = processor(&[V, A]);
        let frame = |kind, pts: i64, dts: i64, duration: i64| CodedFrame {
            kind,
            track_id: 1,
            pts,
            dts,
            duration,
            keyframe: true,
            data: payload(1),
        };
        p.process_frames(vec![
            frame(V, i64::MAX - 5, i64::MAX - 5, 10),
            frame(V, i64::MIN, i64::MIN, 10),
            frame(A, 0, 0, i64::MAX),
            frame(A, 5, 5, -7),
            frame(V, 0, i64::MIN, i64::MAX),
        ])
        .unwrap();
        for offset in [i64::MIN, i64::MAX, 0, -1, 1] {
            p.set_timestamp_offset(offset);
            p.process_frames(vec![
                frame(V, i64::MAX, i64::MAX, 1),
                frame(A, i64::MIN, i64::MAX, 5),
            ])
            .unwrap();
        }
        p.set_mode(AppendMode::Sequence);
        p.set_append_window(i64::MIN, Some(i64::MAX));
        p.process_frames(vec![
            frame(V, i64::MAX, i64::MIN, i64::MAX),
            frame(V, 1, 1, 1),
        ])
        .unwrap();
        p.set_append_window(i64::MAX, Some(i64::MIN));
        p.process_frames(vec![frame(V, 1, 1, 1)]).unwrap();
        let _ = p.buffered();
        let _ = p.buffered_with(true);
        let _ = p.remove(i64::MIN, None);
        let _ = p.remove(i64::MIN, Some(i64::MAX));
        let _ = p.evict(usize::MAX, i64::MAX);
        let _ = p.evict(usize::MAX, i64::MIN);
        let _ = read_all(&p, V, 0);
        p.check_invariants().unwrap();
    }

    #[test]
    fn zero_duration_frames_are_stored_and_do_not_make_ranges() {
        let mut p = processor(&[V]);
        let mut frames = video(0, 5, 0, 50, 100, 100);
        frames[0].keyframe = true;
        let r = p.process_frames(frames).unwrap();
        assert_eq!(r.accepted, 5);
        assert!(
            p.buffered().is_empty(),
            "a zero-duration frame covers no time"
        );
        assert_eq!(p.frame_count(V), 5);
        assert_eq!(p.bytes(), 500 + 5 * OV);
        // eviction can still clear them (a buffer full of nothing)
        let mut q = processor_with(
            &[V],
            mercury_mse::Limits {
                video_bytes: 3 * (100 + OV),
                ..Default::default()
            },
        );
        q.process_frames(video(0, 5, 0, 1, 100, 100)).unwrap();
        assert!(q.evict(0, 100 * SEC));
        assert_eq!(q.frame_count(V), 3);
    }
}
