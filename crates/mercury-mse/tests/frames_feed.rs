// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The feed cursor: frames in decode order from the random access point at or before a position
//! (C06's cursor half), `Config` items in front of the frames that need them, waiting for data that has
//! not arrived, and surviving removal and re-append between two reads (C07, C08, C10).

mod frames_support;

mod frames_feed {
    use super::frames_support::*;
    use mercury_mse::{FeedCursor, FeedItem, SourceBufferProcessor, TrackKind};

    const V: TrackKind = TrackKind::Video;
    const A: TrackKind = TrackKind::Audio;

    fn eight_seconds() -> SourceBufferProcessor {
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 8)).unwrap();
        p
    }

    fn read(
        p: &SourceBufferProcessor,
        cursor: &mut FeedCursor,
        max: usize,
    ) -> (Vec<FeedItem>, bool) {
        let mut out = Vec::new();
        let more = p.read(V, cursor, max, &mut out);
        (out, more)
    }

    #[test]
    fn the_whole_buffer_in_decode_order_with_one_config_and_one_discontinuity() {
        let p = eight_seconds();
        let items = read_all(&p, V, 0);
        assert!(matches!(items[0], FeedItem::Config(_)));
        let frames = frames_of(&items);
        assert_eq!(frames.len(), 200);
        assert!(frames.windows(2).all(|w| w[0].dts < w[1].dts));
        assert_eq!(config_count(&items), 1);
        let discont: Vec<bool> = items
            .iter()
            .filter_map(|i| match i {
                FeedItem::Frame { discont, .. } => Some(*discont),
                _ => None,
            })
            .collect();
        assert!(discont[0]);
        assert!(discont[1..].iter().all(|d| !d));
    }

    #[test]
    fn max_counts_frames_and_more_says_whether_to_read_again() {
        let p = eight_seconds();
        let mut cursor = p.cursor(V, 0);
        let (items, more) = read(&p, &mut cursor, 7);
        assert_eq!(frames_of(&items).len(), 7);
        assert_eq!(
            config_count(&items),
            1,
            "the Config does not count against max"
        );
        assert!(more);
        let mut total = 7;
        loop {
            let (items, more) = read(&p, &mut cursor, 64);
            total += frames_of(&items).len();
            assert_eq!(config_count(&items), 0, "no Config again");
            if !more {
                break;
            }
        }
        assert_eq!(total, 200);
        // nothing more until something is appended
        let (items, more) = read(&p, &mut cursor, 10);
        assert!(items.is_empty() && !more);
        assert!(cursor.is_started());
        assert_eq!(cursor.last_dts(), Some(7_960 * MS));
        // max 0 reads nothing and still says whether there is more
        let mut fresh = p.cursor(V, 0);
        let (items, more) = read(&p, &mut fresh, 0);
        assert!(items.is_empty() && more);
    }

    #[test]
    fn a_seek_starts_at_the_keyframe_at_or_before_the_position() {
        let p = eight_seconds();
        // C06: 5.0 s, back to 0.5 s, then 7.9 s
        for (position, first_pts, count) in [
            (secs(5.0), 4 * SEC, 100),
            (secs(0.5), 0, 200),
            (secs(7.9), 6 * SEC, 50),
            (secs(4.0), 4 * SEC, 100),
            (secs(3.99), 2 * SEC, 150),
        ] {
            let items = read_all(&p, V, position);
            let frames = frames_of(&items);
            assert_eq!(frames.len(), count, "position {position}");
            assert_eq!(frames[0].pts, first_pts, "position {position}");
            assert!(frames[0].keyframe);
            assert!(matches!(items[0], FeedItem::Config(_)));
            assert!(matches!(items[1], FeedItem::Frame { discont: true, .. }));
            assert_eq!(p.feed_start(V, position), Some(first_pts));
        }
    }

    #[test]
    fn a_position_in_the_last_frame_still_finds_it_and_one_past_the_end_waits() {
        let p = eight_seconds();
        assert_eq!(frames_of(&read_all(&p, V, secs(7.99))).len(), 50);
        // exactly at the end of the buffered data: not buffered
        assert!(read_all(&p, V, 8 * SEC).is_empty());
        assert!(read_all(&p, V, 9 * SEC).is_empty());
        assert_eq!(p.feed_start(V, 8 * SEC), None);
    }

    #[test]
    fn a_seek_to_an_unbuffered_range_waits_and_resumes_when_the_data_arrives() {
        // C07: seek to 5 s with only [0, 2) and [6, 8) buffered
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        p.process_frames(video_25fps(6, 2)).unwrap();
        let mut cursor = p.cursor(V, 5 * SEC);
        let (items, more) = read(&p, &mut cursor, 100);
        assert!(items.is_empty() && !more, "no frame and the position holds");
        assert!(!cursor.is_started());
        // the page appends 4-6 s: the cursor starts at the keyframe at 4 s and goes on to 8 s
        p.process_frames(video_25fps(4, 2)).unwrap();
        let mut all = Vec::new();
        loop {
            let (items, more) = read(&p, &mut cursor, 100);
            all.extend(items);
            if !more {
                break;
            }
        }
        let frames = frames_of(&all);
        assert_eq!(frames.len(), 100, "4 s to 8 s, contiguous now");
        assert_eq!(frames[0].pts, 4 * SEC);
        assert!(matches!(all[0], FeedItem::Config(_)));
    }

    #[test]
    fn the_feed_stops_at_a_gap_and_goes_on_when_it_is_filled() {
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        p.process_frames(video_25fps(4, 2)).unwrap();
        let mut cursor = p.cursor(V, 0);
        let mut got = 0;
        loop {
            let (items, more) = read(&p, &mut cursor, 100);
            got += frames_of(&items).len();
            if !more {
                break;
            }
        }
        assert_eq!(got, 50, "up to the gap [2, 4)");
        // filling the gap lets it through, without a discontinuity
        p.process_frames(video_25fps(2, 2)).unwrap();
        let (items, more) = read(&p, &mut cursor, 1000);
        let frames = frames_of(&items);
        assert_eq!(frames.len(), 100);
        assert_eq!(frames[0].pts, 2 * SEC);
        assert!(!more);
        assert!(
            items
                .iter()
                .all(|i| !matches!(i, FeedItem::Frame { discont: true, .. }))
        );
    }

    #[test]
    fn removal_between_two_reads_never_serves_what_was_removed() {
        // C10: the cursor is at 2.0 s when [4, 6) is removed and then re-appended
        let mut p = eight_seconds();
        let mut cursor = p.cursor(V, 0);
        let (items, _) = read(&p, &mut cursor, 50);
        assert_eq!(frames_of(&items).last().unwrap().pts, 1_960 * MS);
        p.remove(4 * SEC, Some(6 * SEC));
        let mut seen = Vec::new();
        loop {
            let (items, more) = read(&p, &mut cursor, 100);
            seen.extend(frames_of(&items));
            if !more {
                break;
            }
        }
        // serves 2 s to 4 s and then waits at the hole
        assert_eq!(seen.len(), 50);
        assert!(seen.iter().all(|f| f.pts < 4 * SEC));
        // re-append heals: the cursor continues from 4 s to 8 s
        p.process_frames(video_25fps(4, 2)).unwrap();
        let mut healed = Vec::new();
        loop {
            let (items, more) = read(&p, &mut cursor, 100);
            healed.extend(frames_of(&items));
            if !more {
                break;
            }
        }
        assert_eq!(healed.len(), 100);
        assert_eq!(healed[0].pts, 4 * SEC);
        assert_eq!(healed.last().unwrap().pts, 7_960 * MS);
    }

    #[test]
    fn removal_behind_the_cursor_changes_nothing_and_removal_of_its_frame_is_safe() {
        let mut p = eight_seconds();
        let mut cursor = p.cursor(V, 0);
        let (items, _) = read(&p, &mut cursor, 80);
        assert_eq!(frames_of(&items).len(), 80);
        // everything the cursor has served is removed, including its own last frame
        p.remove(0, Some(4 * SEC));
        let mut rest = Vec::new();
        loop {
            let (items, more) = read(&p, &mut cursor, 100);
            rest.extend(frames_of(&items));
            if !more {
                break;
            }
        }
        // the frames at 3.2-3.96 are gone with their group, the next keyframe is at 4 s: a gap of one
        // group is not crossed, so the cursor waits at 4 s... the frame after the cursor is the keyframe
        // at 4.0 s and the gap from 3.2 s is larger than the tolerance
        assert!(
            rest.is_empty(),
            "the cursor must not jump over a hole: {}",
            rest.len()
        );
        // a new cursor at 4 s serves everything that is left
        assert_eq!(frames_of(&read_all(&p, V, 4 * SEC)).len(), 100);
    }

    #[test]
    fn audio_cursors_are_per_track_and_all_frames_are_random_access_points() {
        let mut p = processor(&[V, A]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        p.process_frames(audio(0, 100, 20 * MS, 10)).unwrap();
        let audio_items = read_all(&p, A, secs(1.01));
        let frames = frames_of(&audio_items);
        // the audio frame shown at 1.01 s is the one that starts at 1.00 s
        assert_eq!(frames[0].pts, SEC);
        assert_eq!(frames.len(), 50);
        // the video cursor is independent of it
        assert_eq!(frames_of(&read_all(&p, V, secs(1.01))).len(), 50);
        // asking for a kind the buffer does not have gives nothing
        let mut cursor = p.cursor(TrackKind::Text, 0);
        let mut out = Vec::new();
        assert!(!p.read(TrackKind::Text, &mut cursor, 10, &mut out));
        assert!(out.is_empty());
    }

    #[test]
    fn a_seek_keeps_the_configuration_the_consumer_already_has() {
        let p = eight_seconds();
        let mut cursor = p.cursor(V, 0);
        let (items, _) = read(&p, &mut cursor, 10);
        assert_eq!(config_count(&items), 1);
        let known = cursor.config_id();
        assert!(known.is_some());
        cursor.seek(5 * SEC);
        assert!(!cursor.is_started());
        let (items, _) = read(&p, &mut cursor, 10);
        assert_eq!(
            config_count(&items),
            0,
            "same configuration, no Config again"
        );
        assert!(matches!(items[0], FeedItem::Frame { discont: true, .. }));
        // a cursor made for a consumer that has configuration `known` behaves the same
        let mut other = p.cursor_with_config(V, 0, known);
        let (items, _) = read(&p, &mut other, 10);
        assert_eq!(config_count(&items), 0);
        // but one that has another configuration gets it
        let mut third = p.cursor_with_config(V, 0, Some(99));
        let (items, _) = read(&p, &mut third, 10);
        assert_eq!(config_count(&items), 1);
    }

    #[test]
    fn b_frames_are_served_in_decode_order_from_their_keyframe() {
        let mut p = processor(&[V]);
        p.process_frames(bframes(0, 4, 40 * MS, 500)).unwrap();
        // groups start at pts 80, 480, 880, 1280 ms: a seek to 700 ms (a B-frame of group 1) starts at
        // the keyframe of group 1
        let frames = frames_of(&read_all(&p, V, 700 * MS));
        assert_eq!(frames.len(), 30);
        assert!(frames[0].keyframe);
        assert_eq!(frames[0].pts, 480 * MS);
        assert!(frames.windows(2).all(|w| w[0].dts < w[1].dts));
    }

    #[test]
    fn a_stream_that_starts_a_little_after_zero_is_reachable_from_zero() {
        // the first frame of a stream often starts a few milliseconds in (audio priming, B-frame offsets)
        let mut p = processor(&[A]);
        p.process_frames(audio(23 * MS, 100, 20 * MS, 10)).unwrap();
        assert_eq!(frames_of(&read_all(&p, A, 0)).len(), 100);
        // but a seek to 0.5 s with the data starting at 3 s is not
        let mut q = processor(&[A]);
        q.process_frames(audio(3 * SEC, 100, 20 * MS, 10)).unwrap();
        assert!(read_all(&q, A, 0).is_empty(), "3 s late is not a start gap");
        assert!(read_all(&q, A, secs(2.9)).is_empty());
        assert_eq!(frames_of(&read_all(&q, A, secs(3.0))).len(), 100);
        // and an audio frame 10 ms before a position inside the tolerance is found
        assert_eq!(frames_of(&read_all(&q, A, secs(3.0) - 10 * MS)).len(), 100);
    }

    #[test]
    fn a_cursor_is_exhausted_only_when_no_later_frame_is_stored() {
        // C11: the end of stream is for a cursor that served everything, not one waiting at a gap
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        p.process_frames(video_25fps(4, 2)).unwrap();
        let mut cursor = p.cursor(V, 0);
        // not started: frames end after the position
        assert!(!p.cursor_exhausted(V, &cursor));
        let (_, more) = read(&p, &mut cursor, 1000);
        assert!(!more, "stopped at the gap");
        assert!(
            !p.cursor_exhausted(V, &cursor),
            "[4, 6) is stored behind the gap"
        );
        // the data behind the gap is removed: nothing is left to serve
        p.remove(4 * SEC, None);
        assert!(p.cursor_exhausted(V, &cursor));
        // a position at or after everything stored
        assert!(p.cursor_exhausted(V, &p.cursor(V, 2 * SEC)));
        assert!(!p.cursor_exhausted(V, &p.cursor(V, SEC)));
        // a kind the buffer does not have is exhausted from the start
        assert!(p.cursor_exhausted(A, &p.cursor(A, 0)));
        // and appending again is seen
        p.process_frames(video_25fps(2, 2)).unwrap();
        assert!(!p.cursor_exhausted(V, &cursor));
    }

    #[test]
    fn frames_stored_with_the_same_decode_time_are_all_served() {
        // a replacement that leaves a stored frame with the same dts as a new one: both are in the store
        // and the feed gives each one once, in insertion order
        let mut p = processor(&[A]);
        p.process_frames(audio(0, 10, 20 * MS, 10)).unwrap();
        p.process_frames(audio(0, 10, 20 * MS, 20)).unwrap();
        // the second append replaced the first (same times): 10 frames of 20 bytes
        assert_eq!(p.frame_count(A), 10);
        assert_eq!(p.bytes(), 200);
        let frames = frames_of(&read_all(&p, A, 0));
        assert_eq!(frames.len(), 10);
        assert!(frames.iter().all(|f| f.data.len() == 20));
    }
}
