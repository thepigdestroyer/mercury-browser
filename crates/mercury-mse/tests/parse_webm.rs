// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The WebM parser: the committed fixtures whole and under every chunk plan, and hand-built streams for
//! what the fixtures do not have (BlockGroups, known-size Clusters, durations from the next block, a
//! second initialization segment, lacing, three tracks).
//! Run with `cargo test -p mercury-mse --test 'parse_*'` (or the filter `parse_`).

mod parse_support;

mod parse_webm {
    use super::parse_support::*;
    use mercury_mse::api::{CodecConfig, Container, ParseError, TrackKind};
    use mercury_mse_testkit::{ChunkPlan, FIXTURES, load, webm_av1_config};

    const WEBM: [&str; 5] = ["vp9-low", "vp9-high", "opus", "av1-low", "av1-low-av1c"];
    const W: Container = Container::WebM;

    fn frame_count(name: &str) -> usize {
        if name == "opus" { 400 } else { 200 }
    }

    #[test]
    fn m2_every_webm_fixture_is_identical_under_every_chunk_plan() {
        for name in WEBM {
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
                let one = parse_parts(W, &[&all], &plan).unwrap();
                assert_same(
                    &format!("{name} as one stream, {plan:?}"),
                    &one,
                    &reference.events,
                );
                assert_eq!(fed.per_segment, reference.per_segment, "{name} {plan:?}");
            }
        }
    }

    #[test]
    fn frames_times_and_keyframes_match_the_fixtures() {
        for name in WEBM {
            let stream = load(name).unwrap();
            let fed = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let frames = frames_of(&fed.events);
            assert_eq!(frames.len(), frame_count(name), "{name}");
            assert!(fed.during_init.is_empty());
            let (kind, step) = if name == "opus" {
                (TrackKind::Audio, 20_000_000)
            } else {
                (TrackKind::Video, 40_000_000)
            };
            for (i, f) in frames.iter().enumerate() {
                assert_eq!(f.kind, kind);
                assert_eq!(
                    (f.dts, f.pts, f.duration),
                    (i as i64 * step, i as i64 * step, step),
                    "{name} frame {i}"
                );
                let key = if kind == TrackKind::Audio {
                    true
                } else {
                    i % 50 == 0
                };
                assert_eq!(f.keyframe, key, "{name} frame {i}");
            }
            for (k, seg) in stream.media.iter().enumerate() {
                let v = &fed.per_segment[k];
                let start = v.iter().map(|f| f.dts).min().unwrap();
                let end = v.iter().map(|f| f.dts + f.duration).max().unwrap();
                let (s, d) = (
                    seg.start_ms.unwrap() as i64,
                    seg.duration_ms.unwrap() as i64,
                );
                assert_eq!(ns_to_ms(start), s, "{name} segment {k} start");
                assert_eq!(ns_to_ms(end), s + d, "{name} segment {k} end");
            }
            // Every Cluster has an unknown size, so one ends when the next begins: the last one never does.
            assert_eq!(ends_of(&fed.events), stream.media.len() - 1, "{name}");
        }
    }

    #[test]
    fn init_facts_equal_the_fixtures_type_strings() {
        for name in WEBM {
            let stream = load(name).unwrap();
            let f = FIXTURES.iter().find(|f| f.name == name).unwrap();
            let fed = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
            let inits = init_of(&fed.events);
            assert_eq!(inits.len(), 1, "{name}");
            let init = inits[0];
            assert_eq!(init.container, Container::WebM);
            assert_eq!(init.tracks.len(), 1);
            let t = &init.tracks[0];
            assert_eq!((t.track_id, t.timescale), (1, 1000));
            let promised = &codecs_of(f.mime)[0];
            match &t.codec {
                CodecConfig::Vp9 { vpcc } => {
                    // libvpx's muxer writes no CodecPrivate, so the level is not known at the initialization
                    // segment: the short form stands for "any VP9" and the type string refines it.
                    assert!(promised.starts_with("vp09."), "{name}");
                    assert_eq!(t.codec_string, "vp9");
                    assert_eq!(*vpcc, None);
                }
                CodecConfig::Av1 { av1c } => {
                    assert_eq!(&t.codec_string, promised, "{name}");
                    let kit = webm_av1_config(&stream.init).unwrap().unwrap();
                    assert_eq!(
                        av1c[..4],
                        [kit.marker_version, kit.profile_level, kit.flags, kit.delay]
                    );
                    assert_eq!(
                        av1c[4..],
                        kit.config_obus[..],
                        "{name}: the sequence header OBU of CodecPrivate"
                    );
                    assert_eq!(av1c.len() > 4, name == "av1-low-av1c");
                }
                CodecConfig::Opus { head } => {
                    assert_eq!(&t.codec_string, promised);
                    assert_eq!(head.len(), 19);
                    assert!(
                        stream.init.bytes.windows(19).any(|w| w == head.as_slice()),
                        "OpusHead is in the file"
                    );
                    assert_eq!(&head[..8], b"OpusHead");
                    assert_eq!((t.channels, t.sample_rate), (1, 48_000));
                    assert_eq!(u16::from_le_bytes([head[10], head[11]]), 312, "pre-skip");
                }
                other => panic!("{name}: {other:?}"),
            }
            if t.kind == TrackKind::Video {
                assert_eq!((t.width, t.height), (f.width, f.height));
            }
            assert!(init.pssh.is_empty());
        }
    }

    // ---- hand-built streams ----

    fn two_clusters_without_defaults() -> Vec<u8> {
        // Video without a DefaultDuration: durations come from the next block of the track.
        let init = webm_init(&[vp9_track(1, None)]);
        let c0 = cluster(
            0,
            &[
                simple_block(1, 0, true, &payload(1, 10)),
                simple_block(1, 40, false, &payload(2, 10)),
                simple_block(1, 90, false, &payload(3, 10)),
            ],
        );
        let c1 = cluster_unknown(1000, &[simple_block(1, 0, true, &payload(4, 10))]);
        cat(&[&init, &c0, &c1])
    }

    #[test]
    fn durations_come_from_the_next_block_and_the_last_takes_the_one_before() {
        let stream = two_clusters_without_defaults();
        let events = same_under_all_plans(W, &[&stream]).unwrap();
        let frames = frames_of(&events);
        // The first cluster has a known size: it ends at its last byte, so its last frame is released with
        // the duration of the one before (50 ms); the second cluster is still open.
        assert_eq!(frames.len(), 3);
        let spans: Vec<_> = frames
            .iter()
            .map(|f| (ns_to_ms(f.pts), ns_to_ms(f.duration)))
            .collect();
        assert_eq!(spans, vec![(0, 40), (40, 50), (90, 50)]);
        assert_eq!(ends_of(&events), 1);
        // The frame of the open cluster is released when its cluster ends (here, when the next one begins);
        // the next one has a known size, so its single frame is released at its last byte.
        let more = cat(&[
            &stream,
            &cluster(2000, &[simple_block(1, 0, true, &payload(5, 4))]),
        ]);
        let events = same_under_all_plans(W, &[&more]).unwrap();
        let frames = frames_of(&events);
        assert_eq!(frames.len(), 5);
        assert_eq!(
            (ns_to_ms(frames[3].pts), ns_to_ms(frames[3].duration)),
            (1000, 50)
        );
        assert_eq!(
            (ns_to_ms(frames[4].pts), ns_to_ms(frames[4].duration)),
            (2000, 50)
        );
        assert_eq!(ends_of(&events), 3);
    }

    #[test]
    fn block_groups_give_durations_and_keyframes() {
        let init = webm_init(&[vp9_track(1, Some(40_000_000))]);
        let c = cluster(
            500,
            &[
                block_group(1, 0, &payload(1, 8), Some(33), None), // key, 33 ms
                block_group(1, 33, &payload(2, 8), Some(40), Some(-33)), // refers back: not key
                simple_block(1, 73, false, &payload(3, 8)),        // DefaultDuration
                block_group(1, 113, &payload(4, 8), None, None),   // key, DefaultDuration
            ],
        );
        let events = same_under_all_plans(W, &[&cat(&[&init, &c, &cluster(900, &[])])]).unwrap();
        let frames = frames_of(&events);
        let got: Vec<_> = frames
            .iter()
            .map(|f| (ns_to_ms(f.pts), ns_to_ms(f.duration), f.keyframe))
            .collect();
        assert_eq!(
            got,
            vec![
                (500, 33, true),
                (533, 40, false),
                (573, 40, false),
                (613, 40, true)
            ]
        );
        assert_eq!(frames[0].data, payload(1, 8));
        assert_eq!(frames[3].data, payload(4, 8));
    }

    #[test]
    fn every_audio_frame_is_a_keyframe_whatever_its_flags_say() {
        let init = webm_init(&[
            vp9_track(1, Some(40_000_000)),
            opus_track(2, Some(20_000_000)),
        ]);
        let c = cluster(
            0,
            &[
                simple_block(1, 0, false, &payload(1, 6)),
                simple_block(2, 0, false, &payload(2, 6)),
                block_group(2, 20, &payload(3, 6), None, Some(-20)),
            ],
        );
        let events = same_under_all_plans(W, &[&cat(&[&init, &c, &cluster(500, &[])])]).unwrap();
        let frames = frames_of(&events);
        let video: Vec<bool> = track_frames(&frames, TrackKind::Video)
            .iter()
            .map(|f| f.keyframe)
            .collect();
        let audio: Vec<bool> = track_frames(&frames, TrackKind::Audio)
            .iter()
            .map(|f| f.keyframe)
            .collect();
        assert_eq!(video, vec![false]);
        assert_eq!(audio, vec![true, true]);
    }

    #[test]
    fn an_unknown_size_cluster_ends_at_the_next_level_one_element() {
        let init = webm_init(&[vp9_track(1, Some(40_000_000))]);
        let c0 = cluster_unknown(0, &[simple_block(1, 0, true, &payload(1, 5))]);
        let cues = elem(CUES, &[0xBB, 0x80]);
        let c1 = cluster_unknown(40, &[simple_block(1, 0, true, &payload(2, 5))]);
        let events = same_under_all_plans(W, &[&cat(&[&init, &c0, &cues, &c1, &cues])]).unwrap();
        assert_eq!(frames_of(&events).len(), 2);
        assert_eq!(ends_of(&events), 2, "the Cues after each Cluster ended it");
        // The order of events: init, frame, end, frame, end.
        let shape: Vec<char> = events
            .iter()
            .map(|e| match e {
                Ev::Init(_) => 'I',
                Ev::Frame(_) => 'F',
                Ev::End => 'E',
            })
            .collect();
        assert_eq!(shape.iter().collect::<String>(), "IFEFE");
    }

    #[test]
    fn a_known_size_cluster_ends_where_it_says_and_a_second_init_replaces_the_tracks() {
        let init1 = webm_init(&[vp9_track(1, Some(40_000_000))]);
        let init2 = cat(&[
            &ebml_header("webm"),
            &elem_unknown(SEGMENT, &[]),
            &info_elem(1_000_000, Some(8000.0)),
            &tracks_elem(&[vp8_track(7)]),
        ]);
        let c0 = cluster(0, &[simple_block(1, 0, true, &payload(1, 5))]);
        let c1 = cluster(
            100,
            &[
                simple_block(7, 0, true, &payload(2, 5)),
                simple_block(7, 20, false, &payload(3, 5)),
            ],
        );
        let stream = cat(&[&init1, &c0, &init2, &c1]);
        let events = same_under_all_plans(W, &[&stream]).unwrap();
        let inits = init_of(&events);
        assert_eq!(inits.len(), 2);
        assert_eq!(inits[0].tracks[0].codec_string, "vp9");
        assert_eq!(
            (
                inits[1].tracks[0].codec_string.as_str(),
                inits[1].tracks[0].track_id
            ),
            ("vp8", 7)
        );
        let frames = frames_of(&events);
        assert_eq!(
            frames.iter().map(|f| f.track_id).collect::<Vec<_>>(),
            vec![1, 7, 7]
        );
        // The second track has no DefaultDuration: 20 ms from the next block, then the estimate.
        assert_eq!(
            (ns_to_ms(frames[1].duration), ns_to_ms(frames[2].duration)),
            (20, 20)
        );
        assert_eq!(ends_of(&events), 2);
        // A block of the first track after the second init is for a track the new init segment does not
        // describe (and did not skip as a subtitle track): the stream is damaged.
        let after = cat(&[
            &stream,
            &cluster(200, &[simple_block(1, 0, true, &payload(9, 3))]),
        ]);
        assert!(matches!(
            error_of(same_under_all_plans(W, &[&after])),
            ParseError::Malformed(_)
        ));
    }

    #[test]
    fn blocks_with_no_data_are_dropped_and_blocks_of_unknown_tracks_are_refused() {
        let init = webm_init(&[vp9_track(1, Some(40_000_000))]);
        let c = cluster(
            0,
            &[
                simple_block(1, 0, true, &payload(1, 5)),
                simple_block(1, 40, false, &[]),
                simple_block(1, 80, false, &payload(2, 5)),
            ],
        );
        let events = same_under_all_plans(W, &[&cat(&[&init, &c, &cluster(500, &[])])]).unwrap();
        let frames = frames_of(&events);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].pts, 80_000_000, "the empty block's time passes");
        let unknown = cluster(0, &[simple_block(9, 0, true, &payload(1, 5))]);
        assert!(matches!(
            error_of(same_under_all_plans(W, &[&cat(&[&init, &unknown])])),
            ParseError::Malformed(_)
        ));
    }

    #[test]
    fn timecode_scale_negative_offsets_and_the_declared_duration() {
        // 100 us per tick; a block at -3 ticks relative to the cluster; Duration in ticks.
        let init = cat(&[
            &ebml_header("webm"),
            &elem_unknown(SEGMENT, &[]),
            &info_elem(100_000, Some(12_345.0)),
            &tracks_elem(&[opus_track(2, Some(20_000_000))]),
        ]);
        let c = cluster(
            1000,
            &[
                simple_block(2, -3, true, &payload(1, 4)),
                simple_block(2, 197, true, &payload(2, 4)),
            ],
        );
        let events = same_under_all_plans(W, &[&cat(&[&init, &c, &cluster(5000, &[])])]).unwrap();
        let init = init_of(&events)[0].clone();
        assert_eq!(init.duration, Some(1_234_500_000));
        assert_eq!(init.tracks[0].timescale, 10_000);
        assert_eq!(
            (init.tracks[0].channels, init.tracks[0].sample_rate),
            (2, 48_000)
        );
        let frames = frames_of(&events);
        assert_eq!(frames[0].pts, (1000 - 3) * 100_000);
        assert_eq!(frames[1].pts, (1000 + 197) * 100_000);
    }

    #[test]
    fn the_initialization_segment_is_reported_without_a_cluster_and_in_either_order() {
        let tracks = tracks_elem(&[vp9_track(1, Some(40_000_000))]);
        let info = info_elem(1_000_000, None);
        let head = cat(&[&ebml_header("webm"), &elem_unknown(SEGMENT, &[])]);
        // Info then Tracks, Tracks then Info, and Info missing (the first Cluster reports it).
        for (parts, name) in [
            (cat(&[&head, &info, &tracks]), "info, tracks"),
            (cat(&[&head, &tracks, &info]), "tracks, info"),
            (
                cat(&[
                    &head,
                    &tracks,
                    &cluster(0, &[simple_block(1, 0, true, &[1, 2])]),
                ]),
                "no info",
            ),
        ] {
            let events = same_under_all_plans(W, &[&parts]).unwrap();
            assert_eq!(init_of(&events).len(), 1, "{name}");
            assert!(
                matches!(events[0], Ev::Init(_)),
                "{name}: the init comes first"
            );
        }
        // Tracks alone is not yet an initialization segment (Info may follow)...
        let events = same_under_all_plans(W, &[&cat(&[&head, &tracks])]).unwrap();
        assert!(events.is_empty());
        // ...and it is reported as soon as Info arrives, in a later append.
        let events = same_under_all_plans(W, &[&cat(&[&head, &tracks]), &info]).unwrap();
        assert_eq!(init_of(&events).len(), 1);
    }

    #[test]
    fn subtitle_tracks_and_their_blocks_are_ignored() {
        let sub = elem(
            TRACK_ENTRY,
            &cat(&[
                &uint_elem(0xD7, 3),
                &uint_elem(0x83, 17),
                &elem(0x86, b"S_TEXT/UTF8"),
            ]),
        );
        let init = webm_init(&[vp9_track(1, Some(40_000_000)), sub]);
        let c = cluster(
            0,
            &[
                simple_block(1, 0, true, &payload(1, 5)),
                simple_block(3, 0, true, b"hello"),
            ],
        );
        let events = same_under_all_plans(W, &[&cat(&[&init, &c, &cluster(100, &[])])]).unwrap();
        assert_eq!(init_of(&events)[0].tracks.len(), 1);
        assert_eq!(frames_of(&events).len(), 1);
    }

    #[test]
    fn vp9_codec_private_with_features_gives_the_long_codec_string() {
        let t = elem(
            TRACK_ENTRY,
            &cat(&[
                &uint_elem(0xD7, 1),
                &uint_elem(0x83, 1),
                &elem(0x86, b"V_VP9"),
                &elem(0x63A2, &[1, 1, 0, 2, 1, 21, 3, 1, 8]),
            ]),
        );
        let events = same_under_all_plans(W, &[&webm_init(&[t])]).unwrap();
        assert_eq!(init_of(&events)[0].tracks[0].codec_string, "vp09.00.21.08");
    }

    #[test]
    fn what_the_parser_refuses_in_webm() {
        let kinds = |r: Result<Vec<Ev>, ParseError>| match error_of(r) {
            ParseError::Unsupported(_) => "unsupported",
            ParseError::Malformed(_) => "malformed",
            ParseError::TooManyTracks => "too many tracks",
            ParseError::MediaBeforeInit => "media before init",
            ParseError::WrongContainer(_) => "wrong container",
        };
        let entry = |id: u64, ty: u64, codec: &[u8]| {
            elem(
                TRACK_ENTRY,
                &cat(&[
                    &uint_elem(0xD7, id),
                    &uint_elem(0x83, ty),
                    &elem(0x86, codec),
                ]),
            )
        };
        // Codecs this round does not feed.
        for codec in [
            &b"V_MPEG4/ISO/AVC"[..],
            b"A_VORBIS",
            b"A_AAC",
            b"A_FLAC",
            b"V_MPEGH/ISO/HEVC",
            b"A_AC3",
        ] {
            let ty = if codec.starts_with(b"V") { 1 } else { 2 };
            let r = same_under_all_plans(W, &[&webm_init(&[entry(1, ty, codec)])]);
            assert_eq!(
                kinds(r),
                "unsupported",
                "{}",
                String::from_utf8_lossy(codec)
            );
        }
        // AV1 needs its CodecPrivate; Opus needs its head.
        assert_eq!(
            kinds(same_under_all_plans(
                W,
                &[&webm_init(&[entry(1, 1, b"V_AV1")])]
            )),
            "unsupported"
        );
        assert_eq!(
            kinds(same_under_all_plans(
                W,
                &[&webm_init(&[entry(1, 2, b"A_OPUS")])]
            )),
            "malformed"
        );
        // A video codec in an audio track.
        assert_eq!(
            kinds(same_under_all_plans(
                W,
                &[&webm_init(&[entry(1, 2, b"V_VP8")])]
            )),
            "malformed"
        );
        // Three tracks, and two of one kind.
        let three = webm_init(&[vp9_track(1, None), vp8_track(2), opus_track(3, None)]);
        assert_eq!(kinds(same_under_all_plans(W, &[&three])), "too many tracks");
        let two_audio = webm_init(&[opus_track(1, None), opus_track(2, None)]);
        assert_eq!(
            kinds(same_under_all_plans(W, &[&two_audio])),
            "too many tracks"
        );
        // Same number twice, number zero.
        assert_eq!(
            kinds(same_under_all_plans(
                W,
                &[&webm_init(&[vp9_track(1, None), opus_track(1, None)])]
            )),
            "malformed"
        );
        assert_eq!(
            kinds(same_under_all_plans(
                W,
                &[&webm_init(&[vp9_track(0, None)])]
            )),
            "malformed"
        );
        // No usable track at all.
        let sub = elem(
            TRACK_ENTRY,
            &cat(&[
                &uint_elem(0xD7, 1),
                &uint_elem(0x83, 17),
                &elem(0x86, b"S_TEXT/UTF8"),
            ]),
        );
        assert_eq!(
            kinds(same_under_all_plans(W, &[&webm_init(&[sub])])),
            "unsupported"
        );
        // A document type that is neither webm nor matroska.
        let bad_doc = cat(&[&ebml_header("mkv3"), &elem_unknown(SEGMENT, &[])]);
        assert_eq!(kinds(same_under_all_plans(W, &[&bad_doc])), "unsupported");
        // Laced blocks.
        let init = webm_init(&[vp9_track(1, Some(40_000_000))]);
        let mut laced = simple_block(1, 0, true, &payload(1, 12));
        laced[5] |= 0x02; // element id, size, track, timecode (2), flags: Xiph lacing
        let c = cluster(0, &[laced]);
        assert_eq!(
            kinds(same_under_all_plans(W, &[&cat(&[&init, &c])])),
            "unsupported"
        );
        // A Cluster before any Tracks, with or without an EBML header.
        assert_eq!(
            kinds(same_under_all_plans(W, &[&cluster(0, &[])])),
            "media before init"
        );
        let head = cat(&[
            &ebml_header("webm"),
            &elem_unknown(SEGMENT, &[]),
            &info_elem(1_000_000, None),
        ]);
        assert_eq!(
            kinds(same_under_all_plans(W, &[&cat(&[&head, &cluster(0, &[])])])),
            "media before init"
        );
        // A block with no Cluster timecode before it.
        let c = elem(CLUSTER, &simple_block(1, 0, true, &[1]));
        assert_eq!(
            kinds(same_under_all_plans(W, &[&cat(&[&init, &c])])),
            "malformed"
        );
        // A child that runs past its Cluster.
        let short = {
            let mut v = cluster(0, &[simple_block(1, 0, true, &payload(1, 8))]);
            // Cluster size field: id(4) + size(1): shrink it by 4.
            v[4] -= 4;
            v
        };
        assert_eq!(
            kinds(same_under_all_plans(W, &[&cat(&[&init, &short])])),
            "malformed"
        );
    }

    #[test]
    fn memory_stays_bounded_whatever_the_declared_sizes() {
        use mercury_mse::api::ByteStreamParser;
        use mercury_mse::parse::webm::WebmParser;
        let init = webm_init(&[vp9_track(1, Some(40_000_000))]);
        // A Cluster that declares nearly 4 GiB and then delivers 24 MB of blocks of 100 KB: each block is
        // buffered until it is complete and then released, never more.
        let mut p = WebmParser::new();
        p.push(&init).unwrap();
        let mut cluster_header = id_bytes(CLUSTER);
        cluster_header.extend_from_slice(&[0x01, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xF0]);
        p.push(&cluster_header).unwrap();
        p.push(&uint_elem(0xE7, 0)).unwrap();
        let block = simple_block(1, 0, true, &payload(1, 100_000));
        let mut frames = 0;
        for _ in 0..24 {
            let mut mb = Vec::new();
            while mb.len() < 1 << 20 {
                mb.extend_from_slice(&block);
            }
            let events = p.push(&mb).unwrap();
            frames += frames_of(&flatten(events)).len();
            assert!(
                p.retained_bytes() <= block.len(),
                "{} bytes held",
                p.retained_bytes()
            );
        }
        assert!(frames > 200, "{frames} frames");
        // A block that declares 3 GB, an element of 64 MiB + 1 and an Info of 100 MB are refused at once,
        // before any of their bytes are kept.
        for (what, declared) in [
            ("block", 3_000_000_000u64),
            ("limit", (64 << 20) + 1),
            ("info", 100 << 20),
        ] {
            let mut p = WebmParser::new();
            let mut bytes = if what == "info" {
                Vec::new()
            } else {
                init.clone()
            };
            let id = match what {
                "info" => id_bytes(INFO),
                _ => id_bytes(SIMPLE_BLOCK),
            };
            if what != "info" {
                bytes.extend_from_slice(&id_bytes(CLUSTER));
                bytes.extend_from_slice(&[0xff]);
                bytes.extend_from_slice(&uint_elem(0xE7, 0));
            } else {
                bytes.extend_from_slice(&ebml_header("webm"));
                bytes.extend_from_slice(&elem_unknown(SEGMENT, &[]));
            }
            bytes.extend_from_slice(&id);
            bytes.extend_from_slice(&[0x01]);
            bytes.extend_from_slice(&declared.to_be_bytes()[1..]);
            bytes.extend_from_slice(&[0u8; 1000]);
            let mut seen = None;
            for piece in bytes.chunks(5) {
                if let Err(e) = p.push(piece) {
                    seen = Some(e);
                    break;
                }
            }
            assert!(
                matches!(seen, Some(ParseError::Malformed(_))),
                "{what}: {seen:?}"
            );
            assert!(
                p.retained_bytes() < 64,
                "{what}: {} bytes held",
                p.retained_bytes()
            );
        }
    }

    #[test]
    fn reset_drops_the_partial_cluster_and_the_held_frame_and_keeps_the_tracks() {
        let init = webm_init(&[vp9_track(1, None)]);
        let c0 = cluster(
            0,
            &[
                simple_block(1, 0, true, &payload(1, 6)),
                simple_block(1, 40, false, &payload(2, 6)),
            ],
        );
        let c1 = cluster(
            1000,
            &[
                simple_block(1, 0, true, &payload(3, 6)),
                simple_block(1, 40, false, &payload(4, 6)),
            ],
        );
        let mut p = mercury_mse::parse::new_parser(W);
        let mut out = push_plan(p.as_mut(), &init, &ChunkPlan::Whole).unwrap();
        // Cut c0 inside its second block, then reset (an abort).
        out.extend(push_plan(p.as_mut(), &c0[..c0.len() - 3], &ChunkPlan::Whole).unwrap());
        p.reset();
        out.extend(push_plan(p.as_mut(), &c1, &ChunkPlan::Whole).unwrap());
        // c1 is complete (known size): both its frames come out, the held ones included.
        let frames = frames_of(&out);
        let tail: Vec<_> = frames
            .iter()
            .map(|f| (ns_to_ms(f.pts), f.data[0]))
            .collect();
        assert!(tail.ends_with(&[(1000, 3), (1040, 4)]), "{tail:?}");
        assert_eq!(
            init_of(&out).len(),
            1,
            "the init segment is not repeated and is not lost"
        );
    }
}
