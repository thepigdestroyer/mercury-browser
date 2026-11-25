// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C04 (chunking), C13 (abort and errors) at parser level, and T08 (protection data) by construction,
//! ported from the spike's `mse-conformance` and `mse-stress` scenarios to the new parsers: any chunking
//! gives the same events, garbage and wrong input end in a documented error and never a panic, a parser
//! is usable again after `reset`, an initialization segment may come twice, and `pssh` boxes come back
//! byte for byte.
//! Run with `cargo test -p mercury-mse --test 'parse_*'` (or the filter `parse_`).

mod parse_support;

mod parse_errors {
    use super::parse_support::*;
    use mercury_mse::api::{CodecConfig, Container, ParseError, TrackKind};
    use mercury_mse::parse::new_parser;
    use mercury_mse_testkit::{ChunkPlan, load, with_cenc_protection, with_pssh};

    // ---- C04: chunking ----

    #[test]
    fn c04_every_chunking_of_every_fixture_gives_the_same_events() {
        let mut rng = Rng(0xC04);
        for (name, stream) in all_streams() {
            let container = container_of(stream.container);
            let reference = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let all = stream.concat();
            // The five plans of the brief and thirty random ones, per segment and as one stream.
            let mut cuts = plans();
            cuts.extend((0..30).map(|_| random_plan(&mut rng)));
            for plan in &cuts {
                let fed = feed_stream(&stream, plan).unwrap();
                assert_same(
                    &format!("{name} by segment, {plan:?}"),
                    &fed.events,
                    &reference.events,
                );
                let one = parse_parts(container, &[&all], plan).unwrap();
                assert_same(
                    &format!("{name} as one stream, {plan:?}"),
                    &one,
                    &reference.events,
                );
            }
        }
    }

    #[test]
    fn c04_a_single_cut_anywhere_changes_nothing() {
        let mut rng = Rng(0xC04C);
        for (name, stream) in all_streams() {
            let container = container_of(stream.container);
            let all = stream.concat();
            let reference = parse_parts(container, &[&all], &ChunkPlan::Whole).unwrap();
            // Every byte of the first 300 (inside the init segment's headers) and 150 random ones.
            let mut points: Vec<usize> = (1..300.min(all.len())).collect();
            points.extend((0..150).map(|_| 1 + rng.below(all.len() - 1)));
            for at in points {
                let got =
                    parse_parts(container, &[&all[..at], &all[at..]], &ChunkPlan::Whole).unwrap();
                assert_same(&format!("{name} cut at {at}"), &got, &reference);
            }
        }
    }

    // ---- C13: abort and errors, at parser level ----

    fn documented(e: &ParseError) -> bool {
        matches!(
            e,
            ParseError::Malformed(_)
                | ParseError::Unsupported(_)
                | ParseError::MediaBeforeInit
                | ParseError::TooManyTracks
                | ParseError::WrongContainer(_)
        )
    }

    #[test]
    fn c13_garbage_is_refused_with_a_documented_error_and_never_panics() {
        let mut rng = Rng(0x13);
        let mut inputs: Vec<Vec<u8>> = vec![
            vec![],
            vec![0],
            vec![0; 10_000],
            vec![0xFF; 10_000],
            (0..=255u8).cycle().take(5000).collect(),
            b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n".to_vec(),
            b"<!DOCTYPE html><html><body>not a video</body></html>".to_vec(),
            b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec(),
            b"OggS\x00\x02".to_vec(),
        ];
        for n in [1usize, 3, 7, 8, 9, 15, 16, 17, 100, 1000, 20_000] {
            inputs.push(rng.bytes(n));
        }
        for container in [Container::Mp4, Container::WebM] {
            for (i, bytes) in inputs.iter().enumerate() {
                let reference = parse_parts(container, &[bytes], &ChunkPlan::Whole);
                if let Err(e) = &reference {
                    assert!(documented(e));
                }
                // The same under every chunking, error included.
                let again = same_under_all_plans(container, &[bytes]);
                assert_eq!(
                    again.is_err(),
                    reference.is_err(),
                    "{container:?} input {i}"
                );
                // After an error the parser keeps saying so until it is reset, and is then usable.
                let mut p = new_parser(container);
                let mut failed = None;
                for piece in bytes.chunks(50) {
                    if let Err(e) = p.push(piece) {
                        failed = Some(e);
                        break;
                    }
                }
                if let Some(e) = failed {
                    assert!(documented(&e));
                    assert_eq!(p.push(&[1, 2, 3]).unwrap_err(), e, "still failed");
                    assert_eq!(p.push(&[]).unwrap_err(), e, "even an empty push");
                    p.reset();
                    let (name, good) = if container == Container::Mp4 {
                        ("h264-low", load("h264-low").unwrap())
                    } else {
                        ("vp9-low", load("vp9-low").unwrap())
                    };
                    let events =
                        push_plan(p.as_mut(), &good.concat(), &ChunkPlan::Fixed(4096)).unwrap();
                    let want =
                        parse_parts(container, &[&good.concat()], &ChunkPlan::Whole).unwrap();
                    assert_same(&format!("{name} after garbage input {i}"), &events, &want);
                }
            }
        }
    }

    #[test]
    fn c13_a_media_segment_before_any_initialization_segment() {
        for name in [
            "h264-low",
            "aac",
            "h264-bframes",
            "vp9-low",
            "opus",
            "av1-low",
        ] {
            let stream = load(name).unwrap();
            let container = container_of(stream.container);
            for seg in [&stream.media[0], &stream.media[1]] {
                let e = error_of(same_under_all_plans(container, &[&seg.bytes]));
                assert_eq!(e, ParseError::MediaBeforeInit, "{name}");
            }
            // After a reset the parser does not magically know the configuration: still an error.
            let mut p = new_parser(container);
            assert!(p.push(&stream.media[0].bytes).is_err());
            p.reset();
            assert_eq!(
                p.push(&stream.media[0].bytes).unwrap_err(),
                ParseError::MediaBeforeInit
            );
            // But with the init segment the same bytes parse.
            p.reset();
            assert!(
                !push_plan(p.as_mut(), &stream.init.bytes, &ChunkPlan::Whole)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                !frames_of(
                    &push_plan(p.as_mut(), &stream.media[0].bytes, &ChunkPlan::Whole).unwrap()
                )
                .is_empty()
            );
        }
    }

    #[test]
    fn c13_bytes_of_the_other_container_are_refused() {
        let mp4 = load("h264-aac").unwrap();
        let webm = load("vp9-low").unwrap();
        // WebM into the MP4 parser: the EBML header names it.
        for bytes in [&webm.init.bytes, &webm.concat()] {
            let e = error_of(same_under_all_plans(Container::Mp4, &[bytes]));
            assert!(matches!(e, ParseError::WrongContainer(_)), "{e:?}");
        }
        // A bare Cluster is no valid MP4 box either (its first four bytes read as a giant size).
        let e = error_of(same_under_all_plans(
            Container::Mp4,
            &[&webm.media[0].bytes],
        ));
        assert!(
            matches!(e, ParseError::Malformed(_) | ParseError::WrongContainer(_)),
            "{e:?}"
        );
        // MP4 into the WebM parser: the init segment, a media segment, the whole file.
        for bytes in [&mp4.init.bytes, &mp4.media[0].bytes, &mp4.concat()] {
            let e = error_of(same_under_all_plans(Container::WebM, &[bytes]));
            assert!(matches!(e, ParseError::WrongContainer(_)), "{e:?}");
        }
        // Fewer than eight bytes decide nothing, so a cut at 5 bytes is no error yet.
        for (container, bytes) in [
            (Container::Mp4, &webm.init.bytes),
            (Container::WebM, &mp4.init.bytes),
        ] {
            let mut p = new_parser(container);
            assert!(p.push(&bytes[..5]).unwrap().is_empty());
            assert!(p.push(&bytes[5..8]).is_err());
        }
    }

    #[test]
    fn c13_a_truncated_segment_completed_later_equals_the_whole() {
        for (name, stream) in all_streams() {
            let container = container_of(stream.container);
            let reference = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            // Cut the second media segment in three places, finish it in a later append.
            let seg = &stream.media[1].bytes;
            for frac in [1usize, 2, 3] {
                let at = seg.len() * frac / 4;
                let mut p = new_parser(container);
                let mut events =
                    push_plan(p.as_mut(), &stream.init.bytes, &ChunkPlan::Whole).unwrap();
                events.extend(
                    push_plan(p.as_mut(), &stream.media[0].bytes, &ChunkPlan::Whole).unwrap(),
                );
                events.extend(push_plan(p.as_mut(), &seg[..at], &ChunkPlan::Whole).unwrap());
                events.extend(push_plan(p.as_mut(), &seg[at..], &ChunkPlan::Whole).unwrap());
                // The reference continues with the other segments; compare the prefix that was fed.
                let want = frames_of(&reference.events);
                let upto = frames_of(&events).len();
                assert_eq!(frames_of(&events), want[..upto], "{name} cut at {at}");
                assert_eq!(init_of(&events).len(), 1);
            }
        }
    }

    #[test]
    fn c13_an_aborted_segment_is_dropped_and_the_next_one_parses() {
        for (name, stream) in all_streams() {
            let container = container_of(stream.container);
            let reference = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let mut p = new_parser(container);
            push_plan(p.as_mut(), &stream.init.bytes, &ChunkPlan::Whole).unwrap();
            push_plan(p.as_mut(), &stream.media[0].bytes, &ChunkPlan::Whole).unwrap();
            // Segment 1 is cut in half and aborted (`abort()` resets the parser state).
            let half = stream.media[1].bytes.len() / 2;
            push_plan(
                p.as_mut(),
                &stream.media[1].bytes[..half],
                &ChunkPlan::Whole,
            )
            .unwrap();
            p.reset();
            // Segment 2 arrives in full and gives exactly the frames it gives without the accident.
            let got = frames_of(
                &push_plan(p.as_mut(), &stream.media[2].bytes, &ChunkPlan::Fixed(1000)).unwrap(),
            );
            assert_eq!(got, reference.per_segment[2], "{name}");
        }
    }

    #[test]
    fn c13_an_initialization_segment_may_come_twice() {
        for (name, stream) in all_streams() {
            let container = container_of(stream.container);
            let reference = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            // init, init: two identical Init events and nothing else.
            let twice =
                same_under_all_plans(container, &[&stream.init.bytes, &stream.init.bytes]).unwrap();
            let inits = init_of(&twice);
            assert_eq!(inits.len(), 2, "{name}");
            assert_eq!(inits[0], inits[1], "{name}");
            assert_eq!(inits[0], init_of(&reference.events)[0], "{name}");
            assert!(frames_of(&twice).is_empty());
            // init, segment, init, segment: the second init in the middle of a stream changes nothing.
            let mid = same_under_all_plans(
                container,
                &[
                    &stream.init.bytes,
                    &stream.media[0].bytes,
                    &stream.init.bytes,
                    &stream.media[1].bytes,
                ],
            )
            .unwrap();
            assert_eq!(init_of(&mid).len(), 2, "{name}");
            let want: Vec<Fr> = [
                reference.per_segment[0].clone(),
                reference.per_segment[1].clone(),
            ]
            .concat();
            let got = frames_of(&mid);
            // The WebM parser may hold the last frame of the open cluster; compare what both have.
            assert_eq!(got, want[..got.len()], "{name}");
            assert!(
                got.len() + 1 >= want.len(),
                "{name}: {} of {} frames",
                got.len(),
                want.len()
            );
        }
    }

    #[test]
    fn c13_a_quality_switch_sends_a_second_initialization_segment_with_another_configuration() {
        for (low, high) in [("h264-low", "h264-high"), ("vp9-low", "vp9-high")] {
            let (l, h) = (load(low).unwrap(), load(high).unwrap());
            let container = container_of(l.container);
            let events = same_under_all_plans(
                container,
                &[
                    &l.init.bytes,
                    &l.media[0].bytes,
                    &h.init.bytes,
                    &h.media[1].bytes,
                    &h.media[2].bytes,
                ],
            )
            .unwrap();
            let inits = init_of(&events);
            assert_eq!(inits.len(), 2);
            // The track configuration changed (H.264 by its avcC; VP9 in WebM has no codec private data,
            // so only the frame size says so: a consumer must compare the whole TrackConfig).
            assert_ne!(
                inits[0].tracks[0], inits[1].tracks[0],
                "{low} to {high}: the configuration changed"
            );
            assert_eq!(
                (inits[0].tracks[0].width, inits[1].tracks[0].width),
                (320, 640)
            );
            // The frames after the switch are the high rendition's, at the right times.
            let want = feed_stream(&h, &ChunkPlan::Whole).unwrap();
            let frames = frames_of(&events);
            let after: Vec<&Fr> = frames.iter().filter(|f| f.dts >= 2_000_000_000).collect();
            let want_after: Vec<&Fr> = want.per_segment[1]
                .iter()
                .chain(want.per_segment[2].iter())
                .collect();
            assert_eq!(after.len(), want_after.len().min(after.len()));
            for (a, b) in after.iter().zip(want_after) {
                assert_eq!(*a, b, "{low} to {high}");
            }
        }
    }

    #[test]
    fn c13_three_tracks_and_other_impossible_initialization_segments() {
        let mp4 = init_of_tracks(&[
            TrackSpec::video(1, 90_000),
            TrackSpec::audio(2, 48_000),
            TrackSpec::video(3, 90_000),
        ]);
        assert_eq!(
            error_of(same_under_all_plans(Container::Mp4, &[&mp4])),
            ParseError::TooManyTracks
        );
        let webm = webm_init(&[vp9_track(1, None), opus_track(2, None), vp8_track(3)]);
        assert_eq!(
            error_of(same_under_all_plans(Container::WebM, &[&webm])),
            ParseError::TooManyTracks
        );
        // After the error the parser is dead until reset, then fine.
        let mut p = new_parser(Container::Mp4);
        assert!(p.push(&mp4).is_err());
        assert!(p.push(&mp4).is_err());
        p.reset();
        assert_eq!(
            init_of(
                &push_plan(
                    p.as_mut(),
                    &init_of_tracks(&[TrackSpec::video(1, 90_000)]),
                    &ChunkPlan::Whole
                )
                .unwrap()
            )
            .len(),
            1
        );
    }

    // ---- T08: protection data by construction ----

    #[test]
    fn t08_pssh_boxes_come_back_byte_for_byte() {
        let systems: [[u8; 16]; 3] = [
            [
                0xED, 0xEF, 0x8B, 0xA9, 0x79, 0xD6, 0x4A, 0xCE, 0xA3, 0xC8, 0x27, 0xDC, 0xD5, 0x1D,
                0x21, 0xED,
            ], // Widevine
            [
                0x9A, 0x04, 0xF0, 0x79, 0x98, 0x40, 0x42, 0x86, 0xAB, 0x92, 0xE6, 0x5B, 0xE0, 0x88,
                0x5F, 0x95,
            ], // PlayReady
            [7; 16],
        ];
        let mut rng = Rng(0x7008);
        for name in ["h264-low", "aac", "h264-aac", "h264-bframes"] {
            let stream = load(name).unwrap();
            let clear = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let clear_init = init_of(&clear.events)[0].clone();
            assert!(clear_init.pssh.is_empty());
            for (i, system) in systems.iter().enumerate() {
                for len in [0usize, 1, 4, 1000, 65_000] {
                    let data = rng.bytes(len);
                    let protected = with_pssh(&stream.init, *system, &data).unwrap();
                    let n = 8 + 4 + 16 + 4 + len;
                    let expected = &protected.bytes[protected.bytes.len() - n..];
                    assert_eq!(&expected[4..8], b"pssh");
                    assert_eq!(
                        u32::from_be_bytes(expected[..4].try_into().unwrap()) as usize,
                        n
                    );
                    for plan in plans() {
                        let events =
                            parse_parts(Container::Mp4, &[&protected.bytes], &plan).unwrap();
                        let init = init_of(&events)[0].clone();
                        assert_eq!(
                            init.pssh,
                            vec![expected.to_vec()],
                            "{name} system {i} data {len} {plan:?}"
                        );
                        // Nothing else of the init segment changed.
                        assert_eq!(init.tracks, clear_init.tracks);
                        assert_eq!(init.duration, clear_init.duration);
                    }
                }
            }
            // Two boxes, in order.
            let one = with_pssh(&stream.init, systems[0], b"first").unwrap();
            let two = with_pssh(&one, systems[1], b"second!").unwrap();
            let init = init_of(
                &parse_parts(Container::Mp4, &[&two.bytes], &ChunkPlan::Fixed(97)).unwrap(),
            )[0]
            .clone();
            assert_eq!(init.pssh.len(), 2);
            assert_eq!(&init.pssh[0][12..28], &systems[0]);
            assert_eq!(&init.pssh[1][12..28], &systems[1]);
            assert!(init.pssh[1].ends_with(b"second!"));
            // The protected init segment still takes the clear fixture's media segments.
            let events = parse_parts(
                Container::Mp4,
                &[&two.bytes, &stream.media[0].bytes],
                &ChunkPlan::Whole,
            )
            .unwrap();
            assert_eq!(frames_of(&events), clear.per_segment[0], "{name}");
        }
    }

    #[test]
    fn t08_a_protected_sample_entry_is_read_as_the_codec_it_wraps() {
        let stream = load("h264-low").unwrap();
        let clear = init_of(&feed_stream(&stream, &ChunkPlan::Whole).unwrap().events)[0].clone();
        for iv in [8u8, 16] {
            let protected = with_cenc_protection(&stream.init, [9; 16], iv).unwrap();
            let events =
                parse_parts(Container::Mp4, &[&protected.bytes], &ChunkPlan::Fixed(33)).unwrap();
            let init = init_of(&events)[0].clone();
            assert_eq!(init.tracks[0].codec_string, "avc1.42C00C");
            assert!(matches!(init.tracks[0].codec, CodecConfig::H264 { .. }));
            assert_eq!(init.tracks[0].kind, TrackKind::Video);
            assert_eq!(init.tracks, clear.tracks, "iv {iv}");
        }
        // WebM carries protection in ContentEncodings: the testkit refuses to build it, so there is
        // nothing to compare; the WebM init has no pssh.
        assert!(with_pssh(&load("vp9-low").unwrap().init, [1; 16], b"x").is_err());
        let w = feed_stream(&load("vp9-low").unwrap(), &ChunkPlan::Whole).unwrap();
        assert!(init_of(&w.events)[0].pssh.is_empty());
    }
}
