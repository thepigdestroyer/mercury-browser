// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The processor on the real fixtures (parsed by item 1's parsers): the spike's scenarios C08, C10 and
//! C12 with `h264-low` and `h264-high`, `h264-bframes` for the composition offsets, muxed and split
//! audio and video, and WebM. The fixtures are 8 s at 25 fps with a keyframe every 2 s, four segments
//! of 2 s (the WebM audio has sixteen of 0.5 s).

mod frames_support;

mod frames_real {
    use super::frames_support::real::{Real, load};
    use super::frames_support::*;
    use mercury_mse::{AppendMode, FeedItem, SourceBufferProcessor, TrackKind};

    const V: TrackKind = TrackKind::Video;
    const A: TrackKind = TrackKind::Audio;

    fn fed(real: &Real) -> SourceBufferProcessor {
        let mut p = SourceBufferProcessor::new(Default::default());
        p.init_segment(&real.init).unwrap();
        p
    }

    /// Every segment of a fixture into a fresh processor.
    fn whole(name: &str) -> (Real, SourceBufferProcessor) {
        let real = load(name);
        let mut p = fed(&real);
        for seg in &real.segments {
            let r = p.process_frames(seg.clone()).unwrap();
            assert_eq!(r.dropped, 0, "{name}");
        }
        (real, p)
    }

    #[test]
    fn every_fixture_is_accepted_whole() {
        // (fixture, track, frames, buffered end in ms)
        for (name, kind, frames, end_ms) in [
            ("h264-low", V, 200, 8000),
            ("h264-high", V, 200, 8000),
            ("h264-bframes", V, 200, 8000),
            ("vp9-low", V, 200, 8000),
            ("vp9-high", V, 200, 8000),
            ("av1-low", V, 200, 8000),
            ("av1-low-av1c", V, 200, 8000),
            ("aac", A, 375, 8000),
            ("opus", A, 400, 8000),
        ] {
            let (_, p) = whole(name);
            assert_eq!(p.frame_count(kind), frames, "{name}");
            assert_eq!(ms_ranges(&p.buffered()), [(0, end_ms)], "{name}");
            p.check_invariants().unwrap();
        }
    }

    #[test]
    fn real_c03_muxed_audio_and_video_in_one_source_buffer() {
        // the spike: 200 video frames, 375 audio frames, buffered [0, 7.999 s] (the audio's end)
        let (_, p) = whole("h264-aac");
        assert_eq!(p.frame_count(V), 200);
        assert_eq!(p.frame_count(A), 375);
        assert_eq!(ms_ranges(&p.track_buffered(V)), [(0, 8000)]);
        assert_eq!(ms_ranges(&p.track_buffered(A)), [(0, 8000)]);
        let buffered = p.buffered();
        assert_eq!(buffered.len(), 1);
        assert_eq!(buffered[0].start, 0);
        // the intersection ends where the shorter track ends: 375 AAC frames of 1024 samples at 48 kHz
        let audio_end = p.track_buffered(A)[0].end;
        assert_eq!(buffered[0].end, audio_end.min(p.track_buffered(V)[0].end));
        assert!(
            (8 * SEC - buffered[0].end).abs() < 100 * 1000,
            "{buffered:?}"
        );
        // once the stream has ended the last ranges stretch to the highest end
        assert_eq!(ms_ranges(&p.buffered_with(true)), [(0, 8000)]);
        // the feed serves each track whole
        assert_eq!(frames_of(&read_all(&p, V, 0)).len(), 200);
        assert_eq!(frames_of(&read_all(&p, A, 0)).len(), 375);
    }

    #[test]
    fn real_c02_split_audio_and_video_are_two_source_buffers() {
        let (_, video) = whole("h264-low");
        let (_, audio) = whole("aac");
        assert_eq!(ms_ranges(&video.buffered()), [(0, 8000)]);
        assert_eq!(ms_ranges(&audio.buffered()), [(0, 8000)]);
        let v = frames_of(&read_all(&video, V, 0));
        let a = frames_of(&read_all(&audio, A, 0));
        assert_eq!((v.len(), a.len()), (200, 375));
        // times are exact multiples of the frame durations: 40 ms video, 1024/48000 s audio
        assert!(
            v.iter()
                .enumerate()
                .all(|(i, f)| f.pts == i as i64 * 40 * MS)
        );
        // (the fixture's `tfdt` values are a few ticks off the ideal 1024 samples per frame, so the
        // audio is compared to the ideal within a millisecond)
        assert!(
            a.iter()
                .enumerate()
                .all(|(i, f)| { (f.pts - ticks_to_ns(i as i64 * 1024, 48_000)).abs() < MS })
        );
        assert!(a.windows(2).all(|w| w[0].dts < w[1].dts));
    }

    #[test]
    fn real_c05_webm_vp9_and_opus() {
        let (_, vp9) = whole("vp9-low");
        let (_, opus) = whole("opus");
        assert_eq!(frames_of(&read_all(&vp9, V, 0)).len(), 200);
        assert_eq!(frames_of(&read_all(&opus, A, 0)).len(), 400);
        assert_eq!(ms_ranges(&vp9.buffered()), [(0, 8000)]);
        assert_eq!(ms_ranges(&opus.buffered()), [(0, 8000)]);
    }

    #[test]
    fn real_c08_quality_switch_overwrite() {
        // low segments 0-1 buffered, the high rendition appended over 2-6 s, low again for 6-8 s
        let low = load("h264-low");
        let high = load("h264-high");
        let mut p = fed(&low);
        p.process_frames(low.frames(0..2)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 4000)]);
        p.init_segment(&high.init).unwrap();
        p.process_frames(high.frames(1..3)).unwrap();
        p.init_segment(&low.init).unwrap();
        p.process_frames(low.frames(3..4)).unwrap();
        // 200 frames, not 250; buffered [0, 8]
        assert_eq!(p.frame_count(V), 200);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        let items = read_all(&p, V, 0);
        let frames = frames_of(&items);
        assert_eq!(frames.len(), 200);
        assert_eq!(largest_gap(&frames), 0);
        // three configurations: low, high at 2 s, low at 6 s (320x180 then 640x360 then 320x180)
        let sizes: Vec<(u32, u32)> = items
            .iter()
            .filter_map(|i| match i {
                FeedItem::Config(c) => Some((c.width, c.height)),
                _ => None,
            })
            .collect();
        let (lw, lh) = (low.init.tracks[0].width, low.init.tracks[0].height);
        let (hw, hh) = (high.init.tracks[0].width, high.init.tracks[0].height);
        assert_eq!(sizes, [(lw, lh), (hw, hh), (lw, lh)]);
        assert_ne!((lw, lh), (hw, hh));
    }

    #[test]
    fn real_c08_up_and_alternate() {
        let low = load("h264-low");
        let high = load("h264-high");
        // up: low 0-4 s, then high 4-8 s
        let mut p = fed(&low);
        p.process_frames(low.frames(0..2)).unwrap();
        p.init_segment(&high.init).unwrap();
        p.process_frames(high.frames(2..4)).unwrap();
        assert_eq!(p.frame_count(V), 200);
        assert_eq!(config_count(&read_all(&p, V, 0)), 2);
        // alternate: low, high, low, high with an init segment before each switch
        let mut q = fed(&low);
        for i in 0..4 {
            let source = if i % 2 == 0 { &low } else { &high };
            if i > 0 {
                q.init_segment(&source.init).unwrap();
            }
            q.process_frames(source.frames(i..i + 1)).unwrap();
        }
        assert_eq!(q.frame_count(V), 200);
        assert_eq!(ms_ranges(&q.buffered()), [(0, 8000)]);
        assert_eq!(config_count(&read_all(&q, V, 0)), 4);
    }

    #[test]
    fn real_c08_vp9_quality_switch_in_webm_differs_only_in_size() {
        let low = load("vp9-low");
        let high = load("vp9-high");
        let mut p = fed(&low);
        p.process_frames(low.frames(0..2)).unwrap();
        let out = p.init_segment(&high.init).unwrap();
        assert!(
            out.config_changed,
            "no configuration record, but another size"
        );
        p.process_frames(high.frames(2..4)).unwrap();
        assert_eq!(p.frame_count(V), 200);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        assert_eq!(config_count(&read_all(&p, V, 0)), 2);
    }

    #[test]
    fn real_c10_remove_head_tail_heal_and_ripple() {
        let real = load("h264-low");
        let mut p = fed(&real);
        p.process_frames(real.frames(0..4)).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        p.remove(0, Some(4 * SEC));
        assert_eq!(ms_ranges(&p.buffered()), [(4000, 8000)]);
        p.remove(6 * SEC, None);
        assert_eq!(ms_ranges(&p.buffered()), [(4000, 6000)]);
        for index in [0usize, 1, 3] {
            p.process_frames(real.segments[index].clone()).unwrap();
        }
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        p.remove(SEC, Some(3 * SEC));
        assert_eq!(ms_ranges(&p.buffered()), [(0, 1000), (4000, 8000)]);
        // the removed part does not play
        assert!(read_all(&p, V, 2 * SEC).is_empty());
        p.check_invariants().unwrap();
    }

    #[test]
    fn real_c10_remove_with_b_frames() {
        let real = load("h264-bframes");
        let frames = real.all_frames();
        assert!(
            frames.iter().any(|f| f.pts != f.dts),
            "the fixture reorders"
        );
        let mut p = fed(&real);
        p.process_frames(frames.clone()).unwrap();
        // buffered end = the last presentation time plus its duration
        let end = frames.iter().map(|f| f.pts + f.duration).max().unwrap();
        assert_eq!(p.highest_end(), Some(end));
        assert_eq!(p.buffered().last().unwrap().end, end);
        // the start is the earliest presentation time
        let start = frames.iter().map(|f| f.pts).min().unwrap();
        assert_eq!(p.buffered()[0].start, start.max(0));
        // remove(1, 3) ripples to the keyframe at 4 s in presentation time and leaves decodable groups
        p.remove(SEC, Some(3 * SEC));
        let ranges = ms_ranges(&p.buffered());
        assert_eq!(ranges.len(), 2, "{ranges:?}");
        assert!((ranges[0].1 - 1000).abs() <= 80, "{ranges:?}");
        assert_eq!(ranges[1], (4000, 8000));
        let served = frames_of(&read_all(&p, V, 4 * SEC));
        assert!(served[0].keyframe);
        p.check_invariants().unwrap();
    }

    #[test]
    fn real_h264_bframes_decode_order_and_buffered_end() {
        let real = load("h264-bframes");
        let mut p = fed(&real);
        p.process_frames(real.all_frames()).unwrap();
        let served = frames_of(&read_all(&p, V, 0));
        assert_eq!(served.len(), 200);
        // decode order is increasing in dts, the presentation order differs somewhere
        assert!(served.windows(2).all(|w| w[0].dts < w[1].dts));
        assert!(served.windows(2).any(|w| w[0].pts > w[1].pts));
        assert_eq!(
            p.highest_end(),
            Some(served.iter().map(|f| f.pts + f.duration).max().unwrap())
        );
    }

    #[test]
    fn real_c12_timestamp_offset_plus_8_s() {
        let real = load("h264-low");
        let mut p = fed(&real);
        p.set_timestamp_offset(8 * SEC);
        p.process_frames(real.all_frames()).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(8000, 16000)]);
        let served = frames_of(&read_all(&p, V, 8 * SEC));
        assert_eq!(served.len(), 200);
        assert_eq!(served[0].pts, 8 * SEC);
        assert_eq!(served[0].dts, 8 * SEC);
        assert_eq!(served.last().unwrap().pts, 15_960 * MS);
        // the same bytes moved by hand (the spike's shift_media_time) give the same ranges
        let mut q = fed(&real);
        let shifted: Vec<_> = real
            .all_frames()
            .into_iter()
            .map(|mut f| {
                f.pts += 8 * SEC;
                f.dts += 8 * SEC;
                f
            })
            .collect();
        q.process_frames(shifted).unwrap();
        assert_eq!(q.buffered(), p.buffered());
    }

    #[test]
    fn real_c12_negative_offset_and_sequence_and_window() {
        let real = load("h264-low");
        // -1 s: the frames before 0 and their dependents up to the keyframe at 2 s are dropped
        let mut p = fed(&real);
        p.set_timestamp_offset(-SEC);
        let r = p.process_frames(real.all_frames()).unwrap();
        assert_eq!((r.accepted, r.dropped), (150, 50));
        assert_eq!(ms_ranges(&p.buffered()), [(1000, 7000)]);

        // sequence mode: segment 0 (0-2 s), then segment 2 (4-6 s in the file) lands right after it
        let mut s = fed(&real);
        s.set_mode(AppendMode::Sequence);
        s.process_frames(real.segments[0].clone()).unwrap();
        s.process_frames(real.segments[2].clone()).unwrap();
        assert_eq!(ms_ranges(&s.buffered()), [(0, 4000)]);
        assert_eq!(s.frame_count(V), 100);
        assert_eq!(s.timestamp_offset(), -2 * SEC);

        // append window [2 s, 6 s]
        let mut w = fed(&real);
        w.set_append_window(2 * SEC, Some(6 * SEC));
        let r = w.process_frames(real.all_frames()).unwrap();
        assert_eq!((r.accepted, r.dropped), (100, 100));
        assert_eq!(ms_ranges(&w.buffered()), [(2000, 6000)]);
        // an end of +Infinity (the default) accepts everything
        let mut all = fed(&real);
        all.set_append_window(0, None);
        assert_eq!(all.process_frames(real.all_frames()).unwrap().accepted, 200);
    }

    #[test]
    fn real_webm_audio_sixteen_segments_remove_and_offset() {
        let real = load("opus");
        assert_eq!(real.segments.len(), 16);
        let mut p = fed(&real);
        p.set_timestamp_offset(2 * SEC);
        p.process_frames(real.all_frames()).unwrap();
        assert_eq!(ms_ranges(&p.buffered()), [(2000, 10000)]);
        p.remove(4 * SEC, Some(6 * SEC));
        assert_eq!(ms_ranges(&p.buffered()), [(2000, 4000), (6000, 10000)]);
        assert_eq!(p.frame_count(A), 400 - 100);
    }
}
