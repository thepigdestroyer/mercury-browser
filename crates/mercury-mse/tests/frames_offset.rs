// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C12 at processor level: `timestampOffset` (positive and negative), sequence mode, the append window
//! and its rounding, and the generate timestamps flag.

mod frames_support;

mod frames_offset {
    use super::frames_support::*;
    use mercury_mse::{AppendMode, SourceBufferProcessor, TrackKind};
    use std::sync::Arc;

    const V: TrackKind = TrackKind::Video;
    const A: TrackKind = TrackKind::Audio;

    // ---- timestampOffset ----

    #[test]
    fn c12_offset_plus_8_s_gives_8_to_16() {
        let mut p = processor(&[V]);
        p.set_timestamp_offset(8 * SEC);
        let r = p.process_frames(video_25fps(0, 8)).unwrap();
        assert_eq!((r.accepted, r.dropped), (200, 0));
        assert_eq!(ms_ranges(&p.buffered()), [(8000, 16000)]);
        let frames = frames_of(&read_all(&p, V, 8 * SEC));
        assert_eq!(frames.len(), 200);
        assert_eq!((frames[0].pts, frames[0].dts), (8 * SEC, 8 * SEC));
        assert_eq!(frames[199].pts, 15_960 * MS);
        assert_eq!(
            p.timestamp_offset(),
            8 * SEC,
            "the attribute is untouched in segments mode"
        );
        // set before the init segment or after it: both orders are picked up
        let mut q = SourceBufferProcessor::new(Default::default());
        q.set_timestamp_offset(8 * SEC);
        q.init_segment(&init_of(&[V])).unwrap();
        q.process_frames(video_25fps(0, 8)).unwrap();
        assert_eq!(q.buffered(), p.buffered());
    }

    #[test]
    fn c12_the_offset_applies_to_audio_and_to_each_append() {
        let mut p = processor(&[A]);
        p.set_timestamp_offset(secs(2.5));
        p.process_frames(audio(0, 100, 20 * MS, 100)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(2500, 4500)]);
        // a period change: the same media time again with another offset
        p.set_timestamp_offset(secs(10.0));
        p.process_frames(audio(0, 100, 20 * MS, 100)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(2500, 4500), (10000, 12000)]);
        // 3 s, then back: the offset can go down
        p.set_timestamp_offset(secs(4.5));
        p.process_frames(audio(0, 100, 20 * MS, 100)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(2500, 6500), (10000, 12000)]);
    }

    #[test]
    fn c12_negative_offset_drops_what_falls_before_zero() {
        let mut p = processor(&[V]);
        p.set_timestamp_offset(-SEC);
        let r = p.process_frames(video_25fps(0, 8)).unwrap();
        // 25 frames fall before 0 and the 25 after them have no keyframe: 50 dropped, as the spec says
        assert_eq!((r.accepted, r.dropped), (150, 50));
        assert_eq!(ms_ranges(&p.buffered()), [(1000, 7000)]);
        let frames = frames_of(&read_all(&p, V, SEC));
        assert_eq!(frames.len(), 150);
        assert!(frames[0].keyframe);
        assert_eq!(frames[0].pts, SEC);
        p.check_invariants().unwrap();
    }

    #[test]
    fn c12_audio_with_a_negative_offset_keeps_what_is_after_zero() {
        let mut p = processor(&[A]);
        p.set_timestamp_offset(-secs(0.5));
        let r = p.process_frames(audio(0, 100, 20 * MS, 100)).unwrap();
        // every audio frame is a random access point: 25 fall before zero, 75 stay
        assert_eq!((r.accepted, r.dropped), (75, 25));
        assert_eq!(ms_ranges(&p.buffered()), [(0, 1500)]);
    }

    // ---- sequence mode ----

    #[test]
    fn c12_sequence_mode_joins_segments_back_to_back() {
        // segment 0 is 0-2 s in the file and segment 2 is 4-6 s: in sequence mode the second one lands
        // right after the first, 2-4 s
        let mut p = processor(&[V]);
        p.set_mode(AppendMode::Sequence);
        p.process_frames(video_25fps(0, 2)).unwrap();
        p.process_frames(video_25fps(4, 2)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 4000)]);
        assert_eq!(p.frame_count(V), 100);
        assert_eq!(
            p.timestamp_offset(),
            -2 * SEC,
            "the offset follows the group"
        );
        assert_eq!(p.group_end_timestamp(), 4 * SEC);
        let frames = frames_of(&read_all(&p, V, 0));
        assert_eq!(frames.len(), 100);
        assert_eq!(largest_gap(&frames), 0);
        // the picture order: red then blue in the spike is "frames 0-49 then frames 100-149 of the file"
        assert!(frames[50].keyframe);
    }

    #[test]
    fn c12_sequence_mode_starts_at_the_group_start() {
        let mut p = processor(&[V]);
        p.set_mode(AppendMode::Sequence);
        // the first segment of the file starts at 10 s: placed at 0
        p.process_frames(video_25fps(10, 2)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 2000)]);
        // setting the offset in sequence mode sets the group start: the next segment goes there
        p.set_timestamp_offset(5 * SEC);
        p.process_frames(video_25fps(12, 2)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 2000), (5000, 7000)]);
        // and a reset (abort) in sequence mode continues at the group end
        p.reset_parser_state();
        p.process_frames(video_25fps(100, 2)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 2000), (5000, 9000)]);
    }

    #[test]
    fn c12_switching_to_sequence_mode_starts_at_the_group_end() {
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        p.set_mode(AppendMode::Sequence);
        assert_eq!(p.group_start_timestamp(), Some(2 * SEC));
        p.process_frames(video_25fps(50, 2)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 4000)]);
        p.set_mode(AppendMode::Segments);
        assert_eq!(p.mode(), AppendMode::Segments);
    }

    // ---- append window ----

    #[test]
    fn c12_append_window_drops_what_is_outside() {
        let mut p = processor(&[V]);
        p.set_append_window(2 * SEC, Some(6 * SEC));
        assert_eq!(p.append_window(), (2 * SEC, Some(6 * SEC)));
        let r = p.process_frames(video_25fps(0, 8)).unwrap();
        // frames [0, 2) start before the window and [6, 8) end after it; the window holds 100 frames
        assert_eq!((r.accepted, r.dropped), (100, 100));
        assert_eq!(ms_ranges(&p.buffered()), [(2000, 6000)]);
        assert_eq!(p.frame_count(V), 100);
    }

    #[test]
    fn c12_append_window_start_in_the_middle_of_a_group_waits_for_a_keyframe() {
        let mut p = processor(&[V]);
        p.set_append_window(3 * SEC, Some(5 * SEC));
        let r = p.process_frames(video_25fps(0, 8)).unwrap();
        // 3.0-4.0 are delta frames of the group at 2 s: no keyframe yet, dropped; the group at 4 s is
        // kept up to 5.0 s (frame ends after 5 s dropped)
        assert_eq!(r.accepted, 25);
        assert_eq!(ms_ranges(&p.buffered()), [(4000, 5000)]);
    }

    #[test]
    fn c12_a_frame_that_ends_after_the_window_end_is_dropped_whole() {
        // the spec drops (it does not cut) a frame that is only partly inside the window
        let mut p = processor(&[A]);
        p.set_append_window(0, Some(secs(0.05)));
        let r = p.process_frames(audio(0, 10, 20 * MS, 100)).unwrap();
        // frames [0,20) [20,40) fit; [40,60) ends after 50 ms
        assert_eq!((r.accepted, r.dropped), (2, 8));
        assert_eq!(ms_ranges(&p.buffered()), [(0, 40)]);
        // and one that starts before the window start is dropped, the next kept
        let mut q = processor(&[A]);
        q.set_append_window(secs(0.01), None);
        let r = q.process_frames(audio(0, 5, 20 * MS, 100)).unwrap();
        assert_eq!((r.accepted, r.dropped), (4, 1));
        assert_eq!(ms_ranges(&q.buffered()), [(20, 100)]);
    }

    #[test]
    fn c12_append_window_end_rounding_keeps_a_frame_that_ends_a_nanosecond_late() {
        // 29.97 fps at a 90 kHz timescale: frame 28 starts at tick 84084 and lasts 3003 ticks, and its
        // start plus duration, each rounded to nanoseconds on its own, is one nanosecond after the
        // end tick 87087 rounded: 967,633,334 against 967,633,333
        let shared: Arc<[u8]> = payload(100);
        let frames = ticked(V, 0, 29, 3003, 90_000, 30, &shared);
        let last = frames.last().unwrap();
        let exact_end = ticks_to_ns(29 * 3003, 90_000);
        assert_eq!(last.pts + last.duration, exact_end + 1, "the premise");
        // the page sets appendWindowEnd to the end of the last frame, as the double nearest to it
        let window_end = secs(87087.0 / 90_000.0);
        assert_eq!(window_end, exact_end);
        let mut p = processor(&[V]);
        p.set_append_window(0, Some(window_end));
        let r = p.process_frames(frames.clone()).unwrap();
        assert_eq!((r.accepted, r.dropped), (29, 0), "the last frame must stay");
        assert_eq!(p.highest_end(), Some(exact_end + 1));
        // but a frame a real 2 microseconds beyond the window is dropped
        let mut q = processor(&[V]);
        q.set_append_window(0, Some(window_end - 2_000));
        let r = q.process_frames(frames).unwrap();
        assert_eq!(r.dropped, 1);
    }

    #[test]
    fn c12_append_window_start_rounding_keeps_a_keyframe_a_nanosecond_early() {
        let mut p = processor(&[V]);
        p.set_append_window(2 * SEC, None);
        // a keyframe 1 ns before the window start (a rounding artefact) is not dropped
        let mut frames = video(2 * SEC - 1, 25, 40 * MS, 50, 8000, 1000);
        frames[0].keyframe = true;
        let r = p.process_frames(frames).unwrap();
        assert_eq!((r.accepted, r.dropped), (25, 0));
        // 2 microseconds early is outside
        let mut q = processor(&[V]);
        q.set_append_window(2 * SEC, None);
        let r = q
            .process_frames(video(2 * SEC - 2_000, 25, 40 * MS, 50, 8000, 1000))
            .unwrap();
        assert_eq!(
            r.accepted, 0,
            "the keyframe is dropped and the deltas have no keyframe"
        );
    }

    #[test]
    fn c12_an_inverted_or_infinite_window_is_stored_as_given() {
        // the DOM refuses start > end (a TypeError); the processor just keeps the numbers
        let mut p = processor(&[V]);
        p.set_append_window(0, None);
        assert_eq!(p.append_window(), (0, None));
        p.set_append_window(5 * SEC, Some(9 * SEC));
        assert_eq!(p.append_window(), (5 * SEC, Some(9 * SEC)));
    }

    // ---- generate timestamps ----

    #[test]
    fn c12_generate_timestamps_places_each_frame_after_the_previous_one() {
        // a byte stream without timestamps (every frame says 0): frames are laid end to end
        let mut p = processor(&[A]);
        p.set_generate_timestamps(true);
        p.set_mode(AppendMode::Sequence);
        let frames: Vec<_> = audio(0, 50, 20 * MS, 10)
            .into_iter()
            .map(|mut f| {
                f.pts = 0;
                f.dts = 0;
                f
            })
            .collect();
        let r = p.process_frames(frames).unwrap();
        assert_eq!(r.accepted, 50);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 1000)]);
        assert_eq!(p.timestamp_offset(), SEC);
    }
}
