// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C08 at processor level (the spike's quality-switch scenario): frames of a new rendition replace the
//! old ones that overlap them (200 frames, not 250), `buffered` does not change, the feed emits the codec
//! configuration once per switch, and no gap larger than one frame appears.

mod frames_support;

mod frames_overwrite {
    use super::frames_support::*;
    use mercury_mse::{FeedItem, SourceBufferProcessor, TrackKind};

    const V: TrackKind = TrackKind::Video;

    fn low_init() -> mercury_mse::InitSegment {
        init(vec![config(V, 0)])
    }

    fn high_init() -> mercury_mse::InitSegment {
        init(vec![config(V, 1)])
    }

    /// The scenario: low 0-4 s buffered, then the high rendition over 2-6 s, then low again over 6-8 s.
    fn overwrite_scenario() -> SourceBufferProcessor {
        let mut p = SourceBufferProcessor::new(Default::default());
        p.init_segment(&low_init()).unwrap();
        p.process_frames(video_25fps(0, 4)).unwrap();
        p.init_segment(&high_init()).unwrap();
        p.process_frames(video_25fps(2, 4)).unwrap();
        p.init_segment(&low_init()).unwrap();
        p.process_frames(video_25fps(6, 2)).unwrap();
        p
    }

    #[test]
    fn c08_overwrite_replaces_200_frames_not_250() {
        let p = overwrite_scenario();
        // 50 low [0, 2) + 100 high [2, 6) + 50 low [6, 8); the old low [2, 4) is gone, not kept
        assert_eq!(p.frame_count(V), 200);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        assert_eq!(p.highest_end(), Some(8 * SEC));
        p.check_invariants().unwrap();
    }

    #[test]
    fn c08_buffered_is_unchanged_by_the_overwrite() {
        // Buffer [0, 8) with the low rendition, then replace [2, 6) with the high one.
        let mut p = SourceBufferProcessor::new(Default::default());
        p.init_segment(&low_init()).unwrap();
        p.process_frames(video_25fps(0, 8)).unwrap();
        let before = p.buffered();
        assert_eq!(ms_ranges(&before), [(0, 8000)]);
        let low_bytes = p.bytes();
        p.init_segment(&high_init()).unwrap();
        let r = p.process_frames(video_25fps(2, 4)).unwrap();
        assert_eq!((r.accepted, r.dropped), (100, 0));
        assert_eq!(p.buffered(), before, "buffered changed");
        assert_eq!(p.frame_count(V), 200);
        // the replaced 100 frames carry the same bytes (same synthetic sizes)
        assert_eq!(p.bytes(), low_bytes);
        p.check_invariants().unwrap();
    }

    #[test]
    fn c08_the_feed_emits_config_once_per_switch() {
        let p = overwrite_scenario();
        let items = read_all(&p, V, 0);
        let frames = frames_of(&items);
        assert_eq!(frames.len(), 200);
        // Config comes first, then once for the switch to high at 2 s and once for the switch back at 6 s.
        let positions: Vec<usize> = items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| matches!(it, FeedItem::Config(_)).then_some(i))
            .collect();
        assert_eq!(positions.len(), 3, "configs at {positions:?}");
        assert_eq!(positions[0], 0);
        // the second Config sits right before the 51st frame (2 s), the third before the 151st (6 s)
        assert_eq!(positions[1], 50 + 1);
        assert_eq!(positions[2], 150 + 2);
        let widths: Vec<u32> = items
            .iter()
            .filter_map(|it| match it {
                FeedItem::Config(c) => Some(c.width),
                _ => None,
            })
            .collect();
        assert_eq!(widths, [320, 640, 320]);
        // each Config is followed by a keyframe
        for pos in &positions {
            match &items[pos + 1] {
                FeedItem::Frame { frame, .. } => assert!(frame.keyframe),
                other => panic!("Config followed by {other:?}"),
            }
        }
    }

    #[test]
    fn c08_no_gap_larger_than_one_frame_and_decode_order() {
        let p = overwrite_scenario();
        let frames = frames_of(&read_all(&p, V, 0));
        assert!(largest_gap(&frames) <= 40 * MS);
        assert_eq!(largest_gap(&frames), 0);
        for w in frames.windows(2) {
            assert!(w[0].dts < w[1].dts, "decode order broken at {:?}", w[1].dts);
        }
        assert_eq!(frames.first().unwrap().pts, 0);
        assert_eq!(frames.last().unwrap().pts, 7_960 * MS);
        // only the first frame is a discontinuity
        let items = read_all(&p, V, 0);
        let discont = items
            .iter()
            .filter(|i| matches!(i, FeedItem::Frame { discont: true, .. }))
            .count();
        assert_eq!(discont, 1);
    }

    #[test]
    fn c08_alternating_every_segment_gives_three_switches() {
        // low, high, low, high on 2 s segments, each with its init segment: the spike's `alternate`
        let mut p = SourceBufferProcessor::new(Default::default());
        for i in 0..4i64 {
            let variant = (i % 2) as u8;
            p.init_segment(&init(vec![config(V, variant)])).unwrap();
            p.process_frames(video_25fps(i * 2, 2)).unwrap();
        }
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        assert_eq!(p.frame_count(V), 200);
        let items = read_all(&p, V, 0);
        assert_eq!(
            config_count(&items),
            4,
            "initial config plus three switches"
        );
        assert_eq!(frames_of(&items).len(), 200);
        assert_eq!(largest_gap(&frames_of(&items)), 0);
    }

    #[test]
    fn c08_an_init_segment_with_the_same_configuration_changes_nothing() {
        let mut p = SourceBufferProcessor::new(Default::default());
        let first = p.init_segment(&low_init()).unwrap();
        assert!(first.first && !first.config_changed);
        p.process_frames(video_25fps(0, 2)).unwrap();
        let again = p.init_segment(&low_init()).unwrap();
        assert!(!again.first && !again.config_changed);
        p.process_frames(video_25fps(2, 2)).unwrap();
        let items = read_all(&p, V, 0);
        assert_eq!(config_count(&items), 1);
        assert_eq!(frames_of(&items).len(), 100);
        let high = p.init_segment(&high_init()).unwrap();
        assert!(!high.first && high.config_changed);
    }

    #[test]
    fn c08_identical_reappend_replaces_without_growth() {
        let mut p = SourceBufferProcessor::new(Default::default());
        p.init_segment(&low_init()).unwrap();
        p.process_frames(video_25fps(0, 8)).unwrap();
        let bytes = p.bytes();
        for _ in 0..3 {
            // the same 2 s segment again, over and over
            p.process_frames(video_25fps(2, 2)).unwrap();
            assert_eq!(p.frame_count(V), 200);
            assert_eq!(p.bytes(), bytes);
            assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        }
        p.check_invariants().unwrap();
    }

    #[test]
    fn c08_new_data_that_ends_mid_group_removes_the_dependent_frames() {
        // New frames [1.0, 1.2) start a group in the middle of the old group [0, 2): the old frames that
        // followed 1.0 depended on the old frames the new ones replace, so they go up to the keyframe
        // at 2 s (the spec's "next random access point"): a gap [1.2, 2.0) is what is left.
        let mut p = SourceBufferProcessor::new(Default::default());
        p.init_segment(&low_init()).unwrap();
        p.process_frames(video_25fps(0, 4)).unwrap();
        p.init_segment(&high_init()).unwrap();
        let r = p
            .process_frames(video(SEC, 5, 40 * MS, 50, 8000, 1000))
            .unwrap();
        assert_eq!(r.accepted, 5);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 1200), (2000, 4000)]);
        assert_eq!(p.frame_count(V), 25 + 5 + 50);
        p.check_invariants().unwrap();
    }

    #[test]
    fn c08_new_keyframe_in_the_middle_of_an_old_group_keeps_the_earlier_frames() {
        // A keyframe at 3.0 inside the old group [2, 4): the old frames before it stay decodable.
        let mut p = SourceBufferProcessor::new(Default::default());
        p.init_segment(&low_init()).unwrap();
        p.process_frames(video_25fps(0, 4)).unwrap();
        p.init_segment(&high_init()).unwrap();
        // 3.0 to 4.0 as one new group
        p.process_frames(video(3 * SEC, 25, 40 * MS, 50, 8000, 1000))
            .unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 4000)]);
        assert_eq!(p.frame_count(V), 100);
        let frames = frames_of(&read_all(&p, V, 0));
        assert_eq!(frames.len(), 100);
        assert_eq!(largest_gap(&frames), 0);
        p.check_invariants().unwrap();
    }

    #[test]
    fn c08_a_cursor_in_the_replaced_range_restarts_at_the_new_keyframe() {
        let mut p = SourceBufferProcessor::new(Default::default());
        p.init_segment(&low_init()).unwrap();
        p.process_frames(video_25fps(0, 8)).unwrap();
        let mut cursor = p.cursor(V, 0);
        let mut out = Vec::new();
        // serve 75 frames: up to 3.0 s, in the middle of the group [2, 4)
        assert!(p.read(V, &mut cursor, 75, &mut out));
        assert_eq!(frames_of(&out).len(), 75);
        // the player switches up: the high rendition replaces [2, 6)
        p.init_segment(&high_init()).unwrap();
        p.process_frames(video_25fps(2, 4)).unwrap();
        let mut more = Vec::new();
        while p.read(V, &mut cursor, 64, &mut more) {}
        // the next frame served is the high keyframe at 2.0 (its group), flagged as a discontinuity,
        // preceded by the new configuration; then everything on to 8 s in decode order
        let FeedItem::Config(c) = &more[0] else {
            panic!("expected a Config first, got {:?}", more[0]);
        };
        assert_eq!(c.width, 640);
        let FeedItem::Frame { frame, discont } = &more[1] else {
            panic!("expected a frame");
        };
        assert!(frame.keyframe && *discont);
        assert_eq!(frame.pts, 2 * SEC);
        let frames = frames_of(&more);
        // 100 high frames [2, 6) and 50 low [6, 8)
        assert_eq!(frames.len(), 150);
        assert_eq!(config_count(&more), 2, "high, then low at 6 s");
    }
}
