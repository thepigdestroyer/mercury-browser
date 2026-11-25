// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The fragmented MP4 parser on the committed fixtures: every fixture, whole and under every chunk plan,
//! gives the same initialization segment and frames, with times that match the testkit's segment times.
//! Run with `cargo test -p mercury-mse --test 'parse_*'` (or the filter `parse_`).

mod parse_support;

mod parse_fmp4 {
    use super::parse_support::*;
    use mercury_mse::api::{CodecConfig, Container, TrackKind};
    use mercury_mse_testkit::{ChunkPlan, FIXTURES, load};

    const MP4: [&str; 5] = ["h264-low", "h264-high", "aac", "h264-aac", "h264-bframes"];

    /// Everything the committed fixtures promise about frames: counts per track, contiguity, the end.
    fn counts(name: &str) -> (usize, usize) {
        match name {
            "h264-low" | "h264-high" | "h264-bframes" => (200, 0),
            "aac" => (0, 375),
            "h264-aac" => (200, 375),
            other => panic!("unexpected fixture {other}"),
        }
    }

    #[test]
    fn m1_h264_low_init_and_frames_under_every_chunk_plan() {
        let stream = load("h264-low").unwrap();
        let reference = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
        let inits = init_of(&reference.events);
        assert_eq!(inits.len(), 1, "one initialization segment");
        let init = inits[0];
        assert_eq!(init.container, Container::Mp4);
        assert_eq!(init.tracks.len(), 1);
        let t = &init.tracks[0];
        assert_eq!(t.kind, TrackKind::Video);
        assert_eq!(t.codec_string, "avc1.42C00C");
        assert_eq!(
            (t.width, t.height, t.track_id, t.timescale),
            (320, 180, 1, 2500)
        );
        assert!(
            matches!(&t.codec, CodecConfig::H264 { avcc } if avcc[..4] == [1, 0x42, 0xC0, 0x0C])
        );
        assert!(init.pssh.is_empty());

        let frames = frames_of(&reference.events);
        assert_eq!(frames.len(), 200);
        assert_eq!(frames.iter().filter(|f| f.keyframe).count(), 4);
        assert_eq!(ends_of(&reference.events), 4);
        assert!(
            reference.during_init.is_empty(),
            "no frame comes out of an init segment"
        );

        for plan in plans() {
            let fed = feed_stream(&stream, &plan).unwrap();
            assert_same(
                &format!("h264-low segment by segment, {plan:?}"),
                &fed.events,
                &reference.events,
            );
            let whole_stream = parse_parts(Container::Mp4, &[&stream.concat()], &plan).unwrap();
            assert_same(
                &format!("h264-low as one stream, {plan:?}"),
                &whole_stream,
                &reference.events,
            );
        }
    }

    #[test]
    fn every_mp4_fixture_is_identical_under_every_chunk_plan() {
        for name in MP4 {
            let stream = load(name).unwrap();
            let reference = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let all = stream.concat();
            for plan in plans() {
                let fed = feed_stream(&stream, &plan).unwrap();
                assert_same(
                    &format!("{name} by segment, {plan:?}"),
                    &fed.events,
                    &reference.events,
                );
                let one = parse_parts(Container::Mp4, &[&all], &plan).unwrap();
                assert_same(
                    &format!("{name} as one stream, {plan:?}"),
                    &one,
                    &reference.events,
                );
                // The frames of one segment come out before the next segment's first byte: a cut inside a
                // segment never delays a frame past the end of the segment.
                assert_eq!(fed.per_segment.len(), stream.media.len());
                for (k, v) in fed.per_segment.iter().enumerate() {
                    assert_eq!(v, &reference.per_segment[k], "{name} segment {k}, {plan:?}");
                }
            }
        }
    }

    #[test]
    fn frame_counts_times_and_keyframes_match_the_fixtures() {
        for name in MP4 {
            let stream = load(name).unwrap();
            let fed = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let frames = frames_of(&fed.events);
            let video = track_frames(&frames, TrackKind::Video);
            let audio = track_frames(&frames, TrackKind::Audio);
            assert_eq!((video.len(), audio.len()), counts(name), "{name}");

            // Each track is gapless: a frame starts where the one before it ended, in exact nanoseconds.
            // (Across two fragments the muxer's own `tfdt` may differ from the sum of the durations by one
            // sample of the track timescale, which the processor tolerates: 21 us for 48 kHz audio.)
            for (kind, fs) in [("video", &video), ("audio", &audio)] {
                for pair in fs.windows(2) {
                    let gap = pair[1].dts - (pair[0].dts + pair[0].duration);
                    assert!(
                        gap.abs() <= 20_834,
                        "{name} {kind}: {gap} ns between two frames"
                    );
                }
                if let Some(first) = fs.first() {
                    assert_eq!(first.dts, 0, "{name} {kind} starts at 0");
                }
                if let Some(last) = fs.last() {
                    assert!(
                        (last.dts + last.duration - 8_000_000_000).abs() <= 20_834,
                        "{name} {kind} ends at 8 s"
                    );
                }
            }
            // Video: 40 ms frames, a keyframe every 50. Audio: every frame is one.
            assert!(video.iter().all(|f| f.duration == 40_000_000), "{name}");
            for (i, f) in video.iter().enumerate() {
                assert_eq!(f.keyframe, i % 50 == 0, "{name} video frame {i}");
            }
            assert!(audio.iter().all(|f| f.keyframe), "{name}");
            // AAC: 1024 samples at 48 kHz, so 21,333,333 or 21,333,334 ns, never accumulating error.
            assert!(
                audio
                    .iter()
                    .all(|f| (21_333_333..=21_333_334).contains(&f.duration)),
                "{name}"
            );
            assert!(frames.iter().all(|f| !f.data.is_empty()));

            // The segments of the testkit: where each starts and how long it is, within a millisecond of
            // rounding. The muxed file's two tracks start a few ms apart; the segment spans both.
            for (k, seg) in stream.media.iter().enumerate() {
                let v = &fed.per_segment[k];
                let start = v.iter().map(|f| f.dts).min().unwrap();
                let end = v.iter().map(|f| f.dts + f.duration).max().unwrap();
                let (s, d) = (
                    seg.start_ms.unwrap() as i64,
                    seg.duration_ms.unwrap() as i64,
                );
                assert!(
                    (ns_to_ms(start) - s).abs() <= 1,
                    "{name} segment {k} starts at {} ms, not {s}",
                    ns_to_ms(start)
                );
                assert!(
                    (ns_to_ms(end) - (s + d)).abs() <= 1,
                    "{name} segment {k} ends at {} ms, not {}",
                    ns_to_ms(end),
                    s + d
                );
            }
            assert_eq!(
                ends_of(&fed.events),
                stream.media.len(),
                "{name}: one SegmentEnd per fragment"
            );
        }
    }

    #[test]
    fn h264_bframes_have_composition_offsets_in_both_directions() {
        let stream = load("h264-bframes").unwrap();
        let frames = frames_of(&feed_stream(&stream, &ChunkPlan::Whole).unwrap().events);
        assert_eq!(frames.len(), 200);
        let different = frames.iter().filter(|f| f.pts != f.dts).count();
        assert!(
            different >= 150,
            "{different} of 200 frames have pts != dts"
        );
        // This muxer writes `trun` version 1: the offsets are signed, so a frame can present before it
        // decodes (against the decode timeline that starts at 0).
        assert!(
            frames.iter().any(|f| f.pts < f.dts),
            "negative composition offsets"
        );
        assert!(
            frames.iter().any(|f| f.pts > f.dts),
            "positive composition offsets"
        );
        // Reordering within a group never leaves the set of times: pts and dts are permutations.
        let mut pts: Vec<_> = frames.iter().map(|f| f.pts).collect();
        let mut dts: Vec<_> = frames.iter().map(|f| f.dts).collect();
        pts.sort_unstable();
        dts.sort_unstable();
        assert_eq!(pts, dts);
        // The presentation of the last frame ends where the decode timeline ends.
        let end = frames.iter().map(|f| f.pts + f.duration).max().unwrap();
        assert_eq!(end, 8_000_000_000);
        // The keyframe presents first.
        assert!(frames[0].keyframe && frames[0].pts == 0 && frames[0].dts == 0);
    }

    #[test]
    fn init_facts_equal_the_fixtures_type_strings() {
        for name in MP4 {
            let stream = load(name).unwrap();
            let f = FIXTURES.iter().find(|f| f.name == name).unwrap();
            let fed = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let inits = init_of(&fed.events);
            assert_eq!(inits.len(), 1);
            let init = inits[0];
            let from_parser: Vec<&str> = init
                .tracks
                .iter()
                .map(|t| t.codec_string.as_str())
                .collect();
            let from_type = codecs_of(f.mime);
            assert_eq!(
                from_parser,
                from_type.iter().map(String::as_str).collect::<Vec<_>>(),
                "{name}: {}",
                f.mime
            );
            assert_eq!(
                codecs_of(&stream.mime),
                from_type,
                "{name}: the testkit derives the same string"
            );
            for t in &init.tracks {
                assert_ne!(t.track_id, 0);
                assert!(t.timescale > 0);
                assert!(t.language.is_empty(), "und is no language");
                assert!(t.default);
                match t.kind {
                    TrackKind::Video => {
                        assert_eq!((t.width, t.height), (f.width, f.height), "{name}")
                    }
                    _ => assert_eq!(
                        (t.width, t.height, t.sample_rate, t.channels),
                        (0, 0, 48_000, 1),
                        "{name}"
                    ),
                }
            }
            assert_eq!(
                init.tracks
                    .iter()
                    .filter(|t| t.kind == TrackKind::Video)
                    .count(),
                usize::from(counts(name).0 > 0)
            );
            assert_eq!(
                init.tracks
                    .iter()
                    .filter(|t| t.kind == TrackKind::Audio)
                    .count(),
                usize::from(counts(name).1 > 0)
            );
        }
    }

    /// The first `"<tag>"` box in `bytes` and its payload, found by scanning (independent of the parser).
    fn scan_box<'a>(bytes: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
        let at = bytes
            .windows(4)
            .position(|w| w == tag)
            .unwrap_or_else(|| panic!("no {tag:?} box"));
        let size = u32::from_be_bytes(bytes[at - 4..at].try_into().unwrap()) as usize;
        &bytes[at + 4..at - 4 + size]
    }

    #[test]
    fn the_configuration_bytes_are_the_ones_in_the_file() {
        for name in ["h264-low", "h264-high", "h264-bframes", "h264-aac"] {
            let stream = load(name).unwrap();
            let fed = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let init = init_of(&fed.events)[0].clone();
            let video = init
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Video)
                .unwrap();
            let CodecConfig::H264 { avcc } = &video.codec else {
                panic!("{name}: not H.264")
            };
            assert_eq!(
                avcc.as_slice(),
                scan_box(&stream.init.bytes, b"avcC"),
                "{name}: avcC"
            );
        }
        for name in ["aac", "h264-aac"] {
            let stream = load(name).unwrap();
            let fed = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let init = init_of(&fed.events)[0].clone();
            let audio = init
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Audio)
                .unwrap();
            let CodecConfig::Aac { asc } = &audio.codec else {
                panic!("{name}: not AAC")
            };
            // The ASC is the DecoderSpecificInfo at the end of the esds box.
            let esds = scan_box(&stream.init.bytes, b"esds");
            let mut tagged = vec![0x05, asc.len() as u8];
            tagged.extend_from_slice(asc);
            assert!(
                esds.windows(tagged.len()).any(|w| w == tagged.as_slice()),
                "{name}: ASC {asc:?} is the DecoderSpecificInfo of the esds"
            );
            assert_eq!(asc.as_slice(), [0x11, 0x88], "AAC-LC, 48 kHz, mono");
        }
    }

    #[test]
    fn reset_between_segments_keeps_the_initialization_configuration() {
        let stream = load("h264-bframes").unwrap();
        let mut parser = mercury_mse::parse::new_parser(Container::Mp4);
        let mut out = push_plan(parser.as_mut(), &stream.init.bytes, &ChunkPlan::Whole).unwrap();
        // Half of segment 0, then a reset (an abort), then segment 1 in full: its frames still come out
        // with the right times because the configuration (timescale, trex) survives.
        let half = stream.media[0].bytes.len() / 2;
        out.extend(
            push_plan(
                parser.as_mut(),
                &stream.media[0].bytes[..half],
                &ChunkPlan::Whole,
            )
            .unwrap(),
        );
        parser.reset();
        out.extend(push_plan(parser.as_mut(), &stream.media[1].bytes, &ChunkPlan::Whole).unwrap());
        let frames = frames_of(&out);
        let reference = frames_of(&feed_stream(&stream, &ChunkPlan::Whole).unwrap().events);
        let second: Vec<_> = reference[50..100].to_vec();
        let got: Vec<_> = frames.iter().rev().take(50).rev().cloned().collect();
        assert_eq!(
            got, second,
            "segment 1 after an aborted segment 0 is what it is without"
        );
        assert!(init_of(&out).len() == 1);
    }
}
