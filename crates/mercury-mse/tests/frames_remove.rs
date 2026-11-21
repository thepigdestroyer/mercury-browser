// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C10 at processor level: `remove(start, end)` and `buffered`. With a keyframe every 2 s and 8 s of
//! 25 fps video the exact answers are known: `remove(0, 4)` leaves [4, 8], `remove(6, +Infinity)` then
//! [4, 6], re-appending heals, `remove(1, 3)` removes the keyframe at 2 s so the frames up to the next
//! keyframe (4 s) go with it, and the removed part is never served by the feed.

mod frames_support;

mod frames_remove {
    use super::frames_support::*;
    use mercury_mse::{SourceBufferProcessor, TrackKind};

    const V: TrackKind = TrackKind::Video;
    const A: TrackKind = TrackKind::Audio;

    fn eight_seconds() -> SourceBufferProcessor {
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 8)).unwrap();
        p
    }

    #[test]
    fn c10_remove_head_tail_heal_and_ripple() {
        let mut p = eight_seconds();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);

        // remove(0, 4 s): exact edge at 4 s
        let removed = p.remove(0, Some(4 * SEC));
        assert_eq!(removed.frames, 100);
        assert_eq!(ms_ranges(&p.buffered()), [(4000, 8000)]);

        // remove(6 s, +Infinity)
        let removed = p.remove(6 * SEC, None);
        assert_eq!(removed.frames, 50);
        assert_eq!(ms_ranges(&p.buffered()), [(4000, 6000)]);

        // re-append the removed segments 0, 1 and 3: heals to [0, 8]
        p.process_frames(video_25fps(0, 4)).unwrap();
        p.process_frames(video_25fps(6, 2)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        assert_eq!(p.frame_count(V), 200);

        // remove(1 s, 3 s): the keyframe at 2 s goes, so the frames 3-4 s that depended on it go too
        let removed = p.remove(SEC, Some(3 * SEC));
        assert_eq!(removed.frames, 75, "1 s to 4 s");
        assert_eq!(ms_ranges(&p.buffered()), [(0, 1000), (4000, 8000)]);
        p.check_invariants().unwrap();
    }

    #[test]
    fn c10_the_removed_part_is_never_served() {
        let mut p = eight_seconds();
        p.remove(0, Some(4 * SEC));
        // a seek into the removed part finds nothing: no frame, the feed waits
        assert!(read_all(&p, V, SEC).is_empty());
        assert_eq!(p.feed_start(V, SEC), None);
        // a seek to 5 s starts at the keyframe at 4 s
        let frames = frames_of(&read_all(&p, V, 5 * SEC));
        assert_eq!(frames.len(), 100);
        assert_eq!(frames[0].pts, 4 * SEC);
        assert!(frames.iter().all(|f| f.pts >= 4 * SEC));
    }

    #[test]
    fn c10_ripple_after_an_unaligned_remove() {
        // remove(0.48 s, 0.52 s) starts mid-group: everything from 0.48 s up to the keyframe at 2 s goes.
        let mut p = eight_seconds();
        let removed = p.remove(secs(0.48), Some(secs(0.52)));
        assert_eq!(removed.frames, 38);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 480), (2000, 8000)]);
        // the frames before 0.48 s still decode: they start with the keyframe at 0; the feed serves
        // them and stops at the gap, and serves the rest from 2 s on
        let frames = frames_of(&read_all(&p, V, 0));
        assert_eq!(frames.len(), 12);
        assert!(frames[0].keyframe);
        assert_eq!(frames_of(&read_all(&p, V, 2 * SEC)).len(), 150);
        p.check_invariants().unwrap();
    }

    #[test]
    fn c10_remove_returns_the_bytes_at_once() {
        let mut p = eight_seconds();
        let before = p.bytes();
        let removed = p.remove(0, Some(4 * SEC));
        assert_eq!(removed.bytes, 2 * 8000 + 98 * 1000);
        assert_eq!(p.bytes(), before - removed.bytes);
        let removed = p.remove(0, None);
        assert_eq!(p.bytes(), 0);
        assert_eq!(removed.bytes, 2 * 8000 + 98 * 1000);
        assert!(p.buffered().is_empty());
        assert_eq!(p.highest_end(), None);
        p.check_invariants().unwrap();
    }

    #[test]
    fn c10_degenerate_removals_do_nothing() {
        let mut p = eight_seconds();
        let before = p.bytes();
        for (s, e) in [
            (4 * SEC, Some(4 * SEC)),
            (5 * SEC, Some(3 * SEC)),
            (20 * SEC, None),
            (20 * SEC, Some(30 * SEC)),
            (-5 * SEC, Some(-SEC)),
        ] {
            let r = p.remove(s, e);
            assert_eq!(r.frames, 0, "remove({s}, {e:?})");
        }
        assert_eq!(p.bytes(), before);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        // on an empty processor too
        let mut empty = SourceBufferProcessor::new(Default::default());
        assert_eq!(empty.remove(0, None).frames, 0);
    }

    #[test]
    fn c10_remove_to_a_late_end_takes_the_rest_of_the_track() {
        // no keyframe at or after the end: the rest of the buffer goes (the spec's "duration")
        let mut p = eight_seconds();
        let removed = p.remove(7 * SEC, Some(7_500 * MS));
        assert_eq!(removed.frames, 25);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 7000)]);
    }

    #[test]
    fn c10_audio_removal_is_exact_with_no_ripple() {
        // every audio frame is a random access point: nothing rippled, the edges are the frames'
        let mut p = processor(&[A]);
        p.process_frames(audio(0, 400, 20 * MS, 300)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        let removed = p.remove(SEC, Some(3 * SEC));
        assert_eq!(removed.frames, 100);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 1000), (3000, 8000)]);
        assert_eq!(p.bytes(), 300 * 300);
        // remove from the middle of a frame: frames that start in the range go, the one before stays
        p.remove(secs(3.01), Some(secs(3.05)));
        assert_eq!(
            ms_ranges(&p.buffered()),
            [(0, 1000), (3000, 3020), (3060, 8000)]
        );
    }

    #[test]
    fn c10_muxed_audio_and_video_remove_per_track() {
        let mut p = processor(&[V, A]);
        p.process_frames(video_25fps(0, 8)).unwrap();
        p.process_frames(audio(0, 400, 20 * MS, 300)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        // remove(1, 3): video ripples to 4 s, audio stops at 3 s: the intersection is [0,1] and [4,8]
        p.remove(SEC, Some(3 * SEC));
        assert_eq!(ms_ranges(&p.track_buffered(V)), [(0, 1000), (4000, 8000)]);
        assert_eq!(ms_ranges(&p.track_buffered(A)), [(0, 1000), (3000, 8000)]);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 1000), (4000, 8000)]);
        p.check_invariants().unwrap();
    }

    #[test]
    fn c10_removal_with_b_frames_leaves_no_orphans() {
        // decode order I P B B P B B P B B with reordering; removing the middle of a group takes the
        // frames after it in decode order up to the next keyframe, so nothing undecodable stays
        let mut p = processor(&[V]);
        p.process_frames(bframes(0, 3, 40 * MS, 500)).unwrap();
        // pts of group 0: 80 ms .. 440 ms; group 1 starts at pts 480 ms
        assert_eq!(ms_ranges(&p.buffered()), [(80, 1280)]);
        // remove the frames that start in [200 ms, 240 ms): that is P3 (pts 200 ms), and the keyframe at
        // 480 ms is the next random access point, so the removal takes every group 0 frame with a
        // presentation time from 200 ms on (P3 B4 B5 P6 B7 B8 P9) and, because B1 and B2 follow P3 in
        // decode order and may depend on it, those two as well: only the keyframe of group 0 stays
        let removed = p.remove(200 * MS, Some(240 * MS));
        assert_eq!(removed.frames, 9);
        assert_eq!(ms_ranges(&p.buffered()), [(80, 120), (480, 1280)]);
        // the feed from the start serves the keyframe and stops at the gap
        let head = frames_of(&read_all(&p, V, 0));
        assert_eq!(head.len(), 1);
        assert!(head[0].keyframe && head[0].pts == 80 * MS);
        // and from 480 ms the two whole groups, in decode order, each delta frame after its keyframe
        let frames = frames_of(&read_all(&p, V, 480 * MS));
        assert_eq!(frames.len(), 20);
        let mut seen_key = false;
        for f in &frames {
            seen_key |= f.keyframe;
            assert!(seen_key, "a delta frame without its keyframe at {}", f.pts);
        }
        p.check_invariants().unwrap();
    }

    #[test]
    fn c10_remove_then_append_continues_without_a_forced_keyframe() {
        // a removal that does not touch the last frame leaves the group state alone: the next segment,
        // which starts at the end of the last one, joins it
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 4)).unwrap();
        p.remove(0, Some(2 * SEC));
        let r = p.process_frames(video_25fps(4, 2)).unwrap();
        assert_eq!((r.accepted, r.dropped), (50, 0));
        assert_eq!(ms_ranges(&p.buffered()), [(2000, 6000)]);
    }

    #[test]
    fn c10_removing_the_last_frame_makes_the_next_group_start_at_a_keyframe() {
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 4)).unwrap();
        p.remove(2 * SEC, None);
        // the group is over: a stray delta frame is dropped, the keyframe after it starts a group again
        let mut frames = video(2 * SEC, 3, 40 * MS, 50, 8000, 1000);
        frames[0].keyframe = false;
        let r = p.process_frames(frames).unwrap();
        assert_eq!((r.accepted, r.dropped), (0, 3), "no random access point");
        let r = p.process_frames(video_25fps(2, 2)).unwrap();
        assert_eq!((r.accepted, r.dropped), (50, 0));
        assert_eq!(ms_ranges(&p.buffered()), [(0, 4000)]);
    }
}
