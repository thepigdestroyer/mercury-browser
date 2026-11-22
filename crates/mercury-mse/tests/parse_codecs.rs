// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The codecs of lane med-1b's decision D7 in the parsers: HEVC, AC-3, E-AC-3, FLAC and MP3 in fragmented
//! MP4, FLAC, Vorbis, AC-3 and E-AC-3 in WebM. Two kinds of test: the seven committed fixtures (made with
//! `gst-launch-1.0` by `gen.sh d7`) whole and under every chunk plan, and hand-built boxes and elements
//! for what the fixtures do not have (other profiles and rates, truncated and damaged records, zero
//! channels, the 64 KiB cap, a changed configuration). Run with `cargo test -p mercury-mse --test parse_codecs`.

mod parse_support;

mod parse_codecs {
    use super::parse_support::*;
    use mercury_mse::api::{
        CodecConfig, CodecId, Container, InitSegment, ParseError, TrackConfig, TrackKind,
    };
    use mercury_mse_testkit::load;

    const M: Container = Container::Mp4;
    const W: Container = Container::WebM;

    // ------------------------------------------------------------------------------------------
    // The committed fixtures

    /// Parses a fixture as a page appends it (the init segment, then each media segment) under every
    /// chunk plan, asserts all of them give the same events, and returns the init segment, the frames
    /// and the number of segment ends. Facts every fixture shares are checked here.
    fn run_fixture(name: &str) -> (InitSegment, Vec<Fr>) {
        let stream = load(name).unwrap();
        let mut parts: Vec<&[u8]> = vec![&stream.init.bytes];
        parts.extend(stream.media.iter().map(|m| m.bytes.as_slice()));
        let events = same_under_all_plans(container_of(stream.container), &parts)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let inits = init_of(&events);
        assert_eq!(inits.len(), 1, "{name}");
        let frames = frames_of(&events);
        assert!(!frames.is_empty(), "{name}");
        // One segment end per media segment; a WebM stream of Clusters of unknown size (streamable
        // muxing) ends the last Cluster only when another element follows it, as for the other fixtures.
        let open_last = usize::from(stream.container == mercury_mse_testkit::Container::WebM);
        assert_eq!(
            ends_of(&events),
            stream.media.len() - open_last,
            "{name}: one segment end per media segment"
        );
        assert_eq!(inits[0].tracks.len(), 1, "{name}");
        let kind = inits[0].tracks[0].kind;
        assert!(frames.iter().all(|f| f.kind == kind), "{name}");
        for pair in frames.windows(2) {
            assert!(pair[0].dts <= pair[1].dts, "{name}: decode order");
        }
        assert!(frames.iter().all(|f| !f.data.is_empty()), "{name}");
        // The first Vorbis packet decodes to no samples and webmmux writes it with a BlockDuration of 0 (a
        // priming packet); every other frame of every fixture lasts.
        let zero: Vec<(usize, i64, usize)> = frames
            .iter()
            .enumerate()
            .filter(|(i, f)| f.duration <= 0 && !(name == "vorbis" && *i == 0))
            .map(|(i, f)| (i, f.pts, f.data.len()))
            .collect();
        assert!(
            zero.is_empty(),
            "{name}: frames with no duration (index, pts, bytes): {zero:?}"
        );
        (inits[0].clone(), frames)
    }

    fn total_seconds(frames: &[Fr]) -> f64 {
        frames.iter().map(|f| f.duration).sum::<i64>() as f64 / 1e9
    }

    #[test]
    fn the_hevc_fixture_gives_hvcc_frames_and_a_codec_string() {
        let (init, frames) = run_fixture("hevc-low");
        let t = &init.tracks[0];
        assert_eq!(t.codec_string, "hvc1.1.6.L60.B0");
        assert_eq!((t.kind, t.width, t.height), (TrackKind::Video, 320, 180));
        let CodecConfig::Hevc { hvcc } = &t.codec else {
            panic!("{:?}", t.codec)
        };
        assert_eq!(hvcc[0], 1);
        // The record carries the three parameter set arrays of an hvc1 stream: VPS, SPS, PPS.
        let mut types = Vec::new();
        let mut at = 23;
        for _ in 0..hvcc[22] {
            types.push(hvcc[at] & 0x3f);
            let count = usize::from(u16::from_be_bytes([hvcc[at + 1], hvcc[at + 2]]));
            at += 3;
            for _ in 0..count {
                at += 2 + usize::from(u16::from_be_bytes([hvcc[at], hvcc[at + 1]]));
            }
        }
        assert_eq!(types, vec![32, 33, 34]);
        assert_eq!(at, hvcc.len());
        // 8 s at 25 fps, a key frame every 50 frames, no reordering.
        assert_eq!(frames.len(), 200);
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.keyframe, i % 50 == 0, "frame {i}");
            assert_eq!(f.pts, f.dts, "frame {i}: no B-frames");
            assert_eq!(f.duration, 40_000_000, "frame {i}");
            assert_eq!(f.dts, i as i64 * 40_000_000, "frame {i}");
        }
    }

    #[test]
    fn the_ac3_fixture_gives_dac3_and_250_frames_of_32_ms() {
        let (init, frames) = run_fixture("ac3");
        let t = &init.tracks[0];
        assert_eq!(t.codec_string, "ac-3");
        assert_eq!(
            (t.kind, t.sample_rate, t.channels),
            (TrackKind::Audio, 48_000, 1)
        );
        let CodecConfig::Other { codec, config } = &t.codec else {
            panic!("{:?}", t.codec)
        };
        assert_eq!(
            (*codec, config.len()),
            (CodecId::Ac3, 3),
            "a dac3 payload is 3 bytes"
        );
        assert_eq!(frames.len(), 250);
        for (i, f) in frames.iter().enumerate() {
            assert!(f.keyframe, "frame {i}: every audio frame is a sync sample");
            assert_eq!(f.duration, 32_000_000, "frame {i}");
            assert_eq!(f.dts, i as i64 * 32_000_000, "frame {i}");
            // An AC-3 frame starts with the syncword 0x0B77.
            assert_eq!(&f.data[..2], &[0x0B, 0x77], "frame {i}");
        }
    }

    #[test]
    fn the_eac3_fixture_gives_dec3_and_250_frames_of_32_ms() {
        let (init, frames) = run_fixture("eac3");
        let t = &init.tracks[0];
        assert_eq!(t.codec_string, "ec-3");
        assert_eq!(
            (t.kind, t.sample_rate, t.channels),
            (TrackKind::Audio, 48_000, 1)
        );
        let CodecConfig::Other { codec, config } = &t.codec else {
            panic!("{:?}", t.codec)
        };
        assert_eq!(*codec, CodecId::Eac3);
        assert!(
            config.len() >= 5,
            "a dec3 payload has the 2-byte header and 3 bytes per substream"
        );
        assert_eq!(frames.len(), 250);
        for (i, f) in frames.iter().enumerate() {
            assert!(f.keyframe, "frame {i}");
            assert_eq!(f.duration, 32_000_000, "frame {i}");
            assert_eq!(&f.data[..2], &[0x0B, 0x77], "frame {i}");
        }
    }

    #[test]
    fn the_flac_fixture_gives_the_metadata_blocks_without_the_marker() {
        let (init, frames) = run_fixture("flac");
        let t = &init.tracks[0];
        assert_eq!(t.codec_string, "flac");
        // 16 kHz mono (a full-band sine at 48 kHz does not fit the fixture budget).
        assert_eq!(
            (t.kind, t.sample_rate, t.channels),
            (TrackKind::Audio, 16_000, 1)
        );
        let CodecConfig::Other { codec, config } = &t.codec else {
            panic!("{:?}", t.codec)
        };
        assert_eq!(*codec, CodecId::Flac);
        // STREAMINFO first: not-last flag + type 0, length 34; no `fLaC` marker.
        assert_eq!(&config[..4], &[0x00, 0x00, 0x00, 34]);
        assert_ne!(&config[..4], b"fLaC");
        assert!(
            (total_seconds(&frames) - 8.0).abs() < 0.3,
            "{}",
            total_seconds(&frames)
        );
        assert!(frames.iter().all(|f| f.keyframe));
        // A FLAC frame starts with the sync code 0xFFF8 or 0xFFF9.
        assert!(
            frames
                .iter()
                .all(|f| f.data[0] == 0xFF && f.data[1] & 0xFE == 0xF8)
        );
    }

    #[test]
    fn the_mp3_fixture_gives_an_empty_config_and_335_frames_of_24_ms() {
        let (init, frames) = run_fixture("mp3");
        let t = &init.tracks[0];
        assert_eq!(t.codec_string, "mp4a.6B");
        assert_eq!(
            (t.kind, t.sample_rate, t.channels),
            (TrackKind::Audio, 48_000, 1)
        );
        assert_eq!(
            t.codec,
            CodecConfig::Other {
                codec: CodecId::Mp3,
                config: Vec::new()
            }
        );
        // 334 frames went in; mp4mux writes one more (the encoder's last, flushed frame).
        assert_eq!(frames.len(), 335);
        for (i, f) in frames.iter().enumerate() {
            assert!(f.keyframe, "frame {i}");
            assert_eq!(f.duration, 24_000_000, "frame {i}");
            // An MPEG audio frame starts with 11 set bits.
            assert!(f.data[0] == 0xFF && f.data[1] & 0xE0 == 0xE0, "frame {i}");
        }
    }

    #[test]
    fn the_vorbis_fixture_keeps_the_xiph_laced_headers() {
        let (init, frames) = run_fixture("vorbis");
        let t = &init.tracks[0];
        assert_eq!(t.codec_string, "vorbis");
        assert_eq!(
            (t.kind, t.sample_rate, t.channels),
            (TrackKind::Audio, 48_000, 1)
        );
        let CodecConfig::Other { codec, config } = &t.codec else {
            panic!("{:?}", t.codec)
        };
        assert_eq!(*codec, CodecId::Vorbis);
        assert_eq!(config[0], 2, "three packets: the count byte is 2");
        // The identification, comment and setup headers are all in it: their signatures.
        for kind in [1u8, 3, 5] {
            let sig = [kind, b'v', b'o', b'r', b'b', b'i', b's'];
            assert!(
                config.windows(7).any(|w| w == sig),
                "header packet type {kind}"
            );
        }
        assert!(
            (total_seconds(&frames) - 8.0).abs() < 0.2,
            "{}",
            total_seconds(&frames)
        );
        assert!(frames.iter().all(|f| f.keyframe));
    }

    #[test]
    fn the_flac_in_matroska_fixture_gives_the_same_blocks() {
        let (init, frames) = run_fixture("flac-webm");
        let t = &init.tracks[0];
        assert_eq!(t.codec_string, "flac");
        assert_eq!(
            (t.kind, t.sample_rate, t.channels),
            (TrackKind::Audio, 16_000, 1)
        );
        let CodecConfig::Other { codec, config } = &t.codec else {
            panic!("{:?}", t.codec)
        };
        assert_eq!(*codec, CodecId::Flac);
        assert_eq!(&config[..4], &[0x00, 0x00, 0x00, 34]);
        // The same encoder settings as the MP4 one: the two files carry the same STREAMINFO.
        let (mp4, _) = run_fixture("flac");
        let CodecConfig::Other {
            config: mp4_config, ..
        } = &mp4.tracks[0].codec
        else {
            panic!()
        };
        assert_eq!(&config[..38], &mp4_config[..38]);
        assert!(
            (total_seconds(&frames) - 8.0).abs() < 0.3,
            "{}",
            total_seconds(&frames)
        );
    }

    #[test]
    fn the_codec_strings_of_the_fixtures_are_the_ones_the_types_accept() {
        use mercury_mse::api::AllDecoders;
        use mercury_mse::mime::{is_type_supported, parse_type};
        for (name, codec) in [
            ("hevc-low", CodecId::Hevc),
            ("ac3", CodecId::Ac3),
            ("eac3", CodecId::Eac3),
            ("flac", CodecId::Flac),
            ("mp3", CodecId::Mp3),
            ("vorbis", CodecId::Vorbis),
            ("flac-webm", CodecId::Flac),
        ] {
            let stream = load(name).unwrap();
            assert!(
                is_type_supported(&stream.mime, &AllDecoders),
                "{name}: {}",
                stream.mime
            );
            let t = parse_type(&stream.mime).unwrap();
            assert_eq!(t.codecs, vec![codec], "{name}");
            let (init, _) = run_fixture(name);
            // What the init segment says is what the type string promises, string for string.
            assert_eq!(
                codecs_of(&stream.mime),
                vec![init.tracks[0].codec_string.clone()],
                "{name}"
            );
        }
    }

    // ------------------------------------------------------------------------------------------
    // Hand-built MP4 sample entries

    fn entry_track(handler: &'static str, timescale: u32, entry: Vec<u8>) -> TrackSpec {
        TrackSpec {
            id: 1,
            handler,
            timescale,
            entry,
        }
    }

    fn init_for(handler: &'static str, ts: u32, entry: Vec<u8>) -> Result<TrackConfig, ParseError> {
        let stream = init_of_tracks(&[entry_track(handler, ts, entry)]);
        same_under_all_plans(M, &[&stream]).map(|events| init_of(&events)[0].tracks[0].clone())
    }

    fn track(handler: &'static str, ts: u32, entry: Vec<u8>) -> TrackConfig {
        init_for(handler, ts, entry).unwrap_or_else(|e| panic!("{e}"))
    }

    fn refusal(handler: &'static str, ts: u32, entry: Vec<u8>) -> ParseError {
        init_for(handler, ts, entry).expect_err("a refusal")
    }

    /// Packs `(value, bit count)` fields into bytes, most significant bit first.
    fn pack(fields: &[(u32, usize)]) -> Vec<u8> {
        let mut bits = Vec::new();
        for &(v, n) in fields {
            for i in (0..n).rev() {
                bits.push((v >> i) & 1);
            }
        }
        bits.chunks(8)
            .map(|c| {
                c.iter()
                    .enumerate()
                    .fold(0u8, |a, (i, &b)| a | ((b as u8) << (7 - i)))
            })
            .collect()
    }

    /// An `hvcC` record: the 23 fixed bytes and one array per NAL unit `(type, bytes)`.
    fn hvcc_record(
        profile_byte: u8,
        compat: u32,
        constraints: [u8; 6],
        level: u8,
        nals: &[(u8, &[u8])],
    ) -> Vec<u8> {
        let mut v = vec![1, profile_byte];
        v.extend_from_slice(&compat.to_be_bytes());
        v.extend_from_slice(&constraints);
        v.push(level);
        v.extend_from_slice(&[0xF0, 0x00, 0xFC, 0xFD, 0xF8, 0xF8, 0, 0, 0x0F]);
        v.push(nals.len() as u8);
        assert_eq!(v.len(), 23);
        for (kind, nal) in nals {
            v.push(0x80 | kind);
            v.extend_from_slice(&1u16.to_be_bytes());
            v.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            v.extend_from_slice(nal);
        }
        v
    }

    fn main_hvcc() -> Vec<u8> {
        hvcc_record(
            0x01,
            0x6000_0000,
            [0xB0, 0, 0, 0, 0, 0],
            93,
            &[
                (32, &[0x40, 0x01, 0x0C]),
                (33, &[0x42, 0x01, 0x01, 0x01]),
                (34, &[0x44, 0x01]),
            ],
        )
    }

    fn hevc_entry(typ: &str, hvcc: &[u8]) -> Vec<u8> {
        visual_entry(typ, 1920, 1080, &bx("hvcC", hvcc))
    }

    #[test]
    fn hevc_entries_give_the_hvcc_and_the_rfc_6381_string() {
        let t = track("vide", 90_000, hevc_entry("hvc1", &main_hvcc()));
        assert_eq!(t.codec_string, "hvc1.1.6.L93.B0");
        assert_eq!(
            (t.kind, t.width, t.height, t.timescale),
            (TrackKind::Video, 1920, 1080, 90_000)
        );
        assert_eq!(t.codec, CodecConfig::Hevc { hvcc: main_hvcc() });
        // hev1 keeps its own name in the string.
        assert_eq!(
            track("vide", 90_000, hevc_entry("hev1", &main_hvcc())).codec_string,
            "hev1.1.6.L93.B0"
        );
        // Main 10, high tier, level 5.1, a space letter, trailing constraint bytes dropped, inner ones kept.
        let main10 = hvcc_record(
            0x22,
            0x2000_0000,
            [0x90, 0, 0x01, 0, 0, 0],
            153,
            &[(33, &[0x42])],
        );
        assert_eq!(
            track("vide", 90_000, hevc_entry("hvc1", &main10)).codec_string,
            "hvc1.2.4.H153.90.0.1"
        );
        let spaced = hvcc_record(0x81, 0x6000_0000, [0; 6], 93, &[]);
        assert_eq!(
            track("vide", 90_000, hevc_entry("hvc1", &spaced)).codec_string,
            "hvc1.B1.6.L93"
        );
        // All 32 compatibility flags are written bit-reversed, without leading zeros.
        let all = hvcc_record(0x01, 0x0000_0001, [0; 6], 1, &[]);
        assert_eq!(
            track("vide", 90_000, hevc_entry("hvc1", &all)).codec_string,
            "hvc1.1.80000000.L1"
        );
        let none = hvcc_record(0x01, 0, [0; 6], 1, &[]);
        assert_eq!(
            track("vide", 90_000, hevc_entry("hvc1", &none)).codec_string,
            "hvc1.1.0.L1"
        );
    }

    #[test]
    fn a_truncated_or_damaged_hvcc_is_malformed() {
        let good = main_hvcc();
        // Every cut of the record, from empty to one byte short, is refused: the fixed part is 23 bytes
        // and the arrays have to end inside the record.
        for cut in 0..good.len() {
            let e = refusal("vide", 90_000, hevc_entry("hvc1", &good[..cut]));
            assert!(matches!(e, ParseError::Malformed(_)), "cut at {cut}: {e:?}");
        }
        // Not version 1.
        let mut v2 = good.clone();
        v2[0] = 2;
        assert!(matches!(
            refusal("vide", 90_000, hevc_entry("hvc1", &v2)),
            ParseError::Malformed(_)
        ));
        // An array that claims more NAL units than the record holds.
        let mut many = good.clone();
        many[24] = 0xFF;
        assert!(matches!(
            refusal("vide", 90_000, hevc_entry("hvc1", &many)),
            ParseError::Malformed(_)
        ));
        // A NAL unit longer than the rest of the record.
        let mut long = good.clone();
        long[26] = 0x7F;
        assert!(matches!(
            refusal("vide", 90_000, hevc_entry("hvc1", &long)),
            ParseError::Malformed(_)
        ));
        // No hvcC box at all; an hvc1 entry in a sound track.
        assert!(matches!(
            refusal("vide", 90_000, visual_entry("hvc1", 640, 360, &[])),
            ParseError::Malformed(_)
        ));
        assert!(matches!(
            refusal("soun", 48_000, hevc_entry("hvc1", &good)),
            ParseError::Malformed(_)
        ));
    }

    #[test]
    fn dolby_vision_and_ac4_are_unsupported_with_a_note() {
        for (typ, word) in [("dvh1", "Dolby Vision"), ("dvhe", "Dolby Vision")] {
            match refusal("vide", 90_000, hevc_entry(typ, &main_hvcc())) {
                ParseError::Unsupported(m) => assert!(m.contains(word), "{typ}: {m}"),
                other => panic!("{typ}: {other:?}"),
            }
        }
        assert!(matches!(
            refusal(
                "soun",
                48_000,
                audio_entry("ac-4", 2, 48_000, &bx("dac4", &[0; 8]))
            ),
            ParseError::Unsupported(_)
        ));
    }

    /// A `dac3` payload: sample rate code, bsid 8, bsmod 0, `acmod`, `lfeon`, bit rate code 10.
    fn dac3(fscod: u32, acmod: u32, lfeon: u32) -> Vec<u8> {
        pack(&[
            (fscod, 2),
            (8, 5),
            (0, 3),
            (acmod, 3),
            (lfeon, 1),
            (10, 5),
            (0, 5),
        ])
    }

    #[test]
    fn ac3_entries_take_rate_and_channels_from_the_dac3() {
        for (fscod, acmod, lfeon, rate, channels) in [
            (0, 2, 0, 48_000, 2),
            (1, 2, 0, 44_100, 2),
            (2, 1, 0, 32_000, 1),
            (0, 7, 1, 48_000, 6),
            (0, 0, 0, 48_000, 2), // 1+1: two mono programs
            (0, 3, 1, 48_000, 4),
            (0, 6, 0, 48_000, 4),
        ] {
            let config = dac3(fscod, acmod, lfeon);
            assert_eq!(config.len(), 3);
            // The entry says something else on purpose: the configuration wins.
            let t = track(
                "soun",
                48_000,
                audio_entry("ac-3", 9, 8_000, &bx("dac3", &config)),
            );
            assert_eq!(t.codec_string, "ac-3");
            assert_eq!(
                (t.sample_rate, t.channels, t.kind),
                (rate, channels, TrackKind::Audio),
                "{fscod} {acmod} {lfeon}"
            );
            assert_eq!(
                t.codec,
                CodecConfig::Other {
                    codec: CodecId::Ac3,
                    config
                }
            );
        }
    }

    #[test]
    fn a_broken_dac3_is_malformed() {
        let good = dac3(0, 2, 0);
        for cut in 0..good.len() {
            let e = refusal(
                "soun",
                48_000,
                audio_entry("ac-3", 2, 48_000, &bx("dac3", &good[..cut])),
            );
            assert!(matches!(e, ParseError::Malformed(_)), "cut at {cut}: {e:?}");
        }
        // The reserved sample rate code.
        let e = refusal(
            "soun",
            48_000,
            audio_entry("ac-3", 2, 48_000, &bx("dac3", &dac3(3, 2, 0))),
        );
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
        // No dac3 box; an ac-3 entry in a video track.
        let e = refusal("soun", 48_000, audio_entry("ac-3", 2, 48_000, &[]));
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
        let e = refusal(
            "vide",
            90_000,
            audio_entry("ac-3", 2, 48_000, &bx("dac3", &good)),
        );
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
    }

    /// A `dec3` payload with the given independent substreams `(fscod, acmod, lfeon, num_dep_sub)`.
    fn dec3(substreams: &[(u32, u32, u32, u32)]) -> Vec<u8> {
        let mut fields = vec![(384, 13), (substreams.len() as u32 - 1, 3)];
        for &(fscod, acmod, lfeon, deps) in substreams {
            fields.extend_from_slice(&[
                (fscod, 2),
                (16, 5),
                (0, 1),
                (0, 1),
                (0, 3),
                (acmod, 3),
                (lfeon, 1),
                (0, 3),
                (deps, 4),
            ]);
            if deps > 0 {
                fields.push((0b0_0000_0010, 9));
            } else {
                fields.push((0, 1));
            }
        }
        pack(&fields)
    }

    #[test]
    fn eac3_entries_take_the_rate_from_the_dec3_and_prefer_the_entrys_channels() {
        // 5.1, no dependent substream: 5 bytes. The entry's channel count (6) is kept.
        let config = dec3(&[(0, 7, 1, 0)]);
        assert_eq!(config.len(), 5);
        let t = track(
            "soun",
            48_000,
            audio_entry("ec-3", 6, 44_100, &bx("dec3", &config)),
        );
        assert_eq!(t.codec_string, "ec-3");
        assert_eq!((t.sample_rate, t.channels), (48_000, 6));
        assert_eq!(
            t.codec,
            CodecConfig::Other {
                codec: CodecId::Eac3,
                config
            }
        );
        // With no channel count in the entry, the first substream's own channels are used.
        let t = track(
            "soun",
            48_000,
            audio_entry("ec-3", 0, 48_000, &bx("dec3", &dec3(&[(1, 2, 0, 0)]))),
        );
        assert_eq!((t.sample_rate, t.channels), (44_100, 2));
        // A dependent substream adds 9 bits and a second independent substream 3 bytes; both are read.
        let long = dec3(&[(0, 7, 1, 1), (0, 2, 0, 0)]);
        assert_eq!(
            long.len(),
            2 + 4 + 3,
            "two substreams: 32 + 24 bits after the 16-bit header"
        );
        let t = track(
            "soun",
            48_000,
            audio_entry("ec-3", 8, 48_000, &bx("dec3", &long)),
        );
        assert_eq!((t.sample_rate, t.channels), (48_000, 8));
    }

    #[test]
    fn a_broken_dec3_is_malformed() {
        let good = dec3(&[(0, 7, 1, 0), (0, 2, 0, 0)]);
        for cut in 0..good.len() {
            let e = refusal(
                "soun",
                48_000,
                audio_entry("ec-3", 2, 48_000, &bx("dec3", &good[..cut])),
            );
            assert!(matches!(e, ParseError::Malformed(_)), "cut at {cut}: {e:?}");
        }
        let e = refusal(
            "soun",
            48_000,
            audio_entry("ec-3", 2, 48_000, &bx("dec3", &dec3(&[(3, 2, 0, 0)]))),
        );
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
        let e = refusal("soun", 48_000, audio_entry("ec-3", 2, 48_000, &[]));
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
    }

    /// The 34-byte STREAMINFO block (without its header).
    fn streaminfo(rate: u32, channels: u32, bits: u32) -> Vec<u8> {
        let mut v = vec![0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0];
        let packed =
            (u64::from(rate) << 44) | (u64::from(channels - 1) << 41) | (u64::from(bits - 1) << 36);
        v.extend_from_slice(&packed.to_be_bytes());
        v.extend_from_slice(&[0; 16]);
        assert_eq!(v.len(), 34);
        v
    }

    fn flac_block(last: bool, kind: u8, data: &[u8]) -> Vec<u8> {
        let header = (u32::from(last) << 31) | (u32::from(kind) << 24) | data.len() as u32;
        let mut v = header.to_be_bytes().to_vec();
        v.extend_from_slice(data);
        v
    }

    fn dfla(blocks: &[u8]) -> Vec<u8> {
        full("dfLa", 0, 0, blocks)
    }

    #[test]
    fn flac_entries_take_rate_and_channels_from_streaminfo() {
        let blocks = cat(&[
            &flac_block(false, 0, &streaminfo(44_100, 2, 16)),
            &flac_block(true, 4, &[0, 0, 0, 0, 0, 0, 0, 0]),
        ]);
        let t = track(
            "soun",
            44_100,
            audio_entry("fLaC", 7, 8_000, &dfla(&blocks)),
        );
        assert_eq!(t.codec_string, "flac");
        assert_eq!(
            (t.sample_rate, t.channels, t.kind),
            (44_100, 2, TrackKind::Audio)
        );
        assert_eq!(
            t.codec,
            CodecConfig::Other {
                codec: CodecId::Flac,
                config: blocks
            }
        );
        // A rate over 65,535 Hz does not fit the entry's 16.16 field (it holds 0): STREAMINFO has it.
        let hi = cat(&[&flac_block(true, 0, &streaminfo(96_000, 6, 24))]);
        let t = track("soun", 96_000, audio_entry("fLaC", 6, 0, &dfla(&hi)));
        assert_eq!((t.sample_rate, t.channels), (96_000, 6));
        // The STREAMINFO alone, with or without the last-block flag, is a record.
        let alone = flac_block(false, 0, &streaminfo(48_000, 1, 16));
        assert_eq!(
            track(
                "soun",
                48_000,
                audio_entry("fLaC", 1, 48_000, &dfla(&alone))
            )
            .sample_rate,
            48_000
        );
    }

    #[test]
    fn a_broken_dfla_is_malformed() {
        let good = cat(&[
            &flac_block(false, 0, &streaminfo(48_000, 2, 16)),
            &flac_block(true, 3, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
        ]);
        let entry = |blocks: &[u8]| audio_entry("fLaC", 2, 48_000, &dfla(blocks));
        // Every cut that ends inside a block, or before the first is complete.
        for cut in (0..good.len()).filter(|&c| c != 38) {
            let e = refusal("soun", 48_000, entry(&good[..cut]));
            assert!(matches!(e, ParseError::Malformed(_)), "cut at {cut}: {e:?}");
        }
        // Cut exactly after STREAMINFO: the record is complete (no last-block flag is required).
        assert_eq!(
            track("soun", 48_000, entry(&good[..38])).sample_rate,
            48_000
        );
        // STREAMINFO of 33 bytes; the first block is a comment block; the reserved block type 127; a sample
        // rate of 0; a dfLa box that is shorter than its version and flags.
        let short = flac_block(true, 0, &streaminfo(48_000, 2, 16)[..33]);
        let comment_first = flac_block(true, 4, &[0; 8]);
        let reserved = cat(&[
            &flac_block(false, 0, &streaminfo(48_000, 2, 16)),
            &flac_block(true, 127, &[0; 4]),
        ]);
        let zero_rate = flac_block(true, 0, &streaminfo(0, 2, 16));
        for (what, blocks) in [
            ("short", short),
            ("comment first", comment_first),
            ("type 127", reserved),
            ("rate 0", zero_rate),
        ] {
            let e = refusal("soun", 48_000, entry(&blocks));
            assert!(matches!(e, ParseError::Malformed(_)), "{what}: {e:?}");
        }
        let e = refusal(
            "soun",
            48_000,
            audio_entry("fLaC", 2, 48_000, &bx("dfLa", &[0, 0])),
        );
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
        let e = refusal("soun", 48_000, audio_entry("fLaC", 2, 48_000, &[]));
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
    }

    fn esds_with_object_type(oti: u8) -> Vec<u8> {
        let mut esds = esds_aac();
        let at = esds.iter().position(|&b| b == 0x40).unwrap();
        esds[at] = oti;
        esds
    }

    #[test]
    fn mp3_entries_have_no_config_and_the_object_type_names_the_string() {
        for (oti, string) in [(0x6B, "mp4a.6B"), (0x69, "mp4a.69")] {
            let t = track(
                "soun",
                44_100,
                audio_entry("mp4a", 2, 44_100, &esds_with_object_type(oti)),
            );
            assert_eq!(t.codec_string, string);
            assert_eq!(
                (t.sample_rate, t.channels, t.kind),
                (44_100, 2, TrackKind::Audio)
            );
            assert_eq!(
                t.codec,
                CodecConfig::Other {
                    codec: CodecId::Mp3,
                    config: Vec::new()
                }
            );
        }
        // AAC keeps working, and MPEG-2 AAC (object type 0x66) is still refused.
        let aac = track("soun", 48_000, mp4a_entry());
        assert_eq!(aac.codec_string, "mp4a.40.2");
        let e = refusal(
            "soun",
            48_000,
            audio_entry("mp4a", 2, 44_100, &esds_with_object_type(0x66)),
        );
        assert!(matches!(e, ParseError::Unsupported(_)), "{e:?}");
    }

    #[test]
    fn an_mp3_entry_without_channels_or_rate_is_malformed() {
        let esds = esds_with_object_type(0x6B);
        let e = refusal("soun", 44_100, audio_entry("mp4a", 0, 44_100, &esds));
        assert!(
            matches!(e, ParseError::Malformed(_)),
            "zero channels: {e:?}"
        );
        let e = refusal("soun", 44_100, audio_entry("mp4a", 2, 0, &esds));
        assert!(matches!(e, ParseError::Malformed(_)), "zero rate: {e:?}");
        // Opus already refused zero channels: the same rule for every codec of the new table.
        let mut dops = vec![0, 0];
        dops.extend_from_slice(&312u16.to_be_bytes());
        dops.extend_from_slice(&48_000u32.to_be_bytes());
        dops.extend_from_slice(&0u16.to_be_bytes());
        dops.push(0);
        let e = refusal(
            "soun",
            48_000,
            audio_entry("Opus", 0, 48_000, &bx("dOps", &dops)),
        );
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
    }

    #[test]
    fn a_changed_configuration_is_a_changed_track_config() {
        // The same codec string, a different record: TrackConfig compares unequal, which is how the
        // source buffer notices that the decoder must be set up again (the rule of round 1 for H.264).
        let a = main_hvcc();
        let mut b = a.clone();
        let last = b.len() - 1;
        b[last] ^= 0x01; // one byte of the PPS
        let ta = track("vide", 90_000, hevc_entry("hvc1", &a));
        let tb = track("vide", 90_000, hevc_entry("hvc1", &b));
        assert_eq!(ta.codec_string, tb.codec_string);
        assert_ne!(ta, tb, "hvcC");
        assert_eq!(
            ta,
            track("vide", 90_000, hevc_entry("hvc1", &a)),
            "an identical record is no change"
        );
        // hvc1 against hev1 is a change too (the string differs).
        assert_ne!(ta, track("vide", 90_000, hevc_entry("hev1", &a)));
        // The audio configurations.
        let t1 = track(
            "soun",
            48_000,
            audio_entry("ac-3", 2, 48_000, &bx("dac3", &dac3(0, 2, 0))),
        );
        let t2 = track(
            "soun",
            48_000,
            audio_entry("ac-3", 2, 48_000, &bx("dac3", &dac3(0, 7, 1))),
        );
        assert_ne!(t1, t2, "dac3");
        let e1 = track(
            "soun",
            48_000,
            audio_entry("ec-3", 2, 48_000, &bx("dec3", &dec3(&[(0, 2, 0, 0)]))),
        );
        let e2 = track(
            "soun",
            48_000,
            audio_entry("ec-3", 2, 48_000, &bx("dec3", &dec3(&[(0, 2, 1, 0)]))),
        );
        assert_ne!(e1, e2, "dec3");
        let f1 = track(
            "soun",
            48_000,
            audio_entry(
                "fLaC",
                2,
                48_000,
                &dfla(&flac_block(true, 0, &streaminfo(48_000, 2, 16))),
            ),
        );
        let f2 = track(
            "soun",
            48_000,
            audio_entry(
                "fLaC",
                2,
                48_000,
                &dfla(&flac_block(true, 0, &streaminfo(48_000, 2, 24))),
            ),
        );
        assert_eq!((f1.sample_rate, f1.channels), (f2.sample_rate, f2.channels));
        assert_ne!(f1, f2, "dfLa");
    }

    // ------------------------------------------------------------------------------------------
    // The media segments of the new entries, hand-built: the parser needs nothing but the sample sizes

    #[test]
    fn frames_of_the_new_codecs_come_out_with_exact_times() {
        // An ac-3 track (timescale 48 kHz, 1536 ticks per frame) and an hvc1 track (90 kHz, 3600 ticks).
        let init = init_of_tracks(&[
            TrackSpec {
                id: 1,
                handler: "vide",
                timescale: 90_000,
                entry: hevc_entry("hvc1", &main_hvcc()),
            },
            TrackSpec {
                id: 2,
                handler: "soun",
                timescale: 48_000,
                entry: audio_entry("ac-3", 2, 48_000, &bx("dac3", &dac3(0, 2, 0))),
            },
        ]);
        let data = cat(&[
            &payload(1, 40),
            &payload(2, 30),
            &payload(3, 20),
            &payload(4, 20),
        ]);
        let seg = fragment(
            |off| {
                let video = Run {
                    data_offset: Some(off as i32),
                    has_duration: true,
                    has_size: true,
                    has_flags: true,
                    samples: vec![
                        RunSample {
                            duration: 3600,
                            size: 40,
                            flags: 0x0200_0000,
                            cto: 0,
                        },
                        RunSample {
                            duration: 3600,
                            size: 30,
                            flags: 0x0101_0000,
                            cto: 0,
                        },
                    ],
                    ..Run::default()
                };
                let audio = Run {
                    data_offset: Some(off as i32 + 70),
                    has_duration: true,
                    has_size: true,
                    samples: vec![
                        RunSample {
                            duration: 1536,
                            size: 20,
                            flags: 0,
                            cto: 0,
                        },
                        RunSample {
                            duration: 1536,
                            size: 20,
                            flags: 0,
                            cto: 0,
                        },
                    ],
                    ..Run::default()
                };
                let base = Tfhd {
                    default_base_is_moof: true,
                    ..Tfhd::default()
                };
                vec![
                    traf(&[
                        tfhd(&Tfhd {
                            track_id: 1,
                            ..base.clone()
                        }),
                        tfdt(0, 0),
                        trun(&video),
                    ]),
                    traf(&[
                        tfhd(&Tfhd {
                            track_id: 2,
                            ..base.clone()
                        }),
                        tfdt(0, 48_000),
                        trun(&audio),
                    ]),
                ]
            },
            &data,
        );
        let events = same_under_all_plans(M, &[&init, &seg]).unwrap();
        let frames = frames_of(&events);
        assert_eq!(frames.len(), 4);
        let video = track_frames(&frames, TrackKind::Video);
        assert_eq!(
            (video[0].dts, video[1].dts, video[0].duration),
            (0, 40_000_000, 40_000_000)
        );
        assert_eq!((video[0].keyframe, video[1].keyframe), (true, false));
        let audio = track_frames(&frames, TrackKind::Audio);
        assert_eq!((audio[0].dts, audio[1].dts), (1_000_000_000, 1_032_000_000));
        assert!(audio.iter().all(|f| f.keyframe && f.duration == 32_000_000));
        assert_eq!(audio[0].data, payload(3, 20));
    }

    // ------------------------------------------------------------------------------------------
    // Hand-built WebM tracks

    /// A `TrackEntry` of an audio track.
    fn audio_track(
        number: u64,
        codec_id: &str,
        private: Option<&[u8]>,
        audio: Option<(f64, u64)>,
    ) -> Vec<u8> {
        let mut p = cat(&[
            &uint_elem(0xD7, number),
            &uint_elem(0x73C5, number),
            &uint_elem(0x83, 2),
            &elem(0x86, codec_id.as_bytes()),
        ]);
        if let Some(private) = private {
            p.extend_from_slice(&elem(0x63A2, private));
        }
        if let Some((rate, channels)) = audio {
            p.extend_from_slice(&elem(
                0xE1,
                &cat(&[&float_elem(0xB5, rate), &uint_elem(0x9F, channels)]),
            ));
        }
        elem(TRACK_ENTRY, &p)
    }

    fn webm_track(entry: Vec<u8>) -> Result<TrackConfig, ParseError> {
        let stream = webm_init(&[entry]);
        same_under_all_plans(W, &[&stream]).map(|events| init_of(&events)[0].tracks[0].clone())
    }

    fn webm_refusal(entry: Vec<u8>) -> ParseError {
        webm_track(entry).expect_err("a refusal")
    }

    fn lacing(n: usize) -> Vec<u8> {
        let mut v = vec![255; n / 255];
        v.push((n % 255) as u8);
        v
    }

    /// A Vorbis `CodecPrivate`: count byte, Xiph lacing of the first two sizes, three packets. The comment
    /// header is padded by `comment_pad` bytes (the parser reads its size and its type, not its content).
    fn vorbis_private(channels: u8, rate: u32, comment_pad: usize, setup_len: usize) -> Vec<u8> {
        let mut ident = vec![1];
        ident.extend_from_slice(b"vorbis");
        ident.extend_from_slice(&0u32.to_le_bytes());
        ident.push(channels);
        ident.extend_from_slice(&rate.to_le_bytes());
        ident.extend_from_slice(&[0; 12]);
        ident.extend_from_slice(&[0xB8, 1]);
        assert_eq!(ident.len(), 30);
        let mut comment = vec![3];
        comment.extend_from_slice(b"vorbis");
        comment.extend_from_slice(&[0; 8]);
        comment.extend(std::iter::repeat_n(0x20, comment_pad));
        comment.push(1);
        let mut setup = vec![5];
        setup.extend_from_slice(b"vorbis");
        setup.extend(std::iter::repeat_n(0x55, setup_len));
        let mut v = vec![2];
        v.extend_from_slice(&lacing(ident.len()));
        v.extend_from_slice(&lacing(comment.len()));
        v.extend_from_slice(&ident);
        v.extend_from_slice(&comment);
        v.extend_from_slice(&setup);
        v
    }

    #[test]
    fn a_vorbis_track_keeps_its_codec_private_and_reads_rate_and_channels_from_it() {
        // A comment header over 255 bytes needs a run of 255s in the lacing, one over 510 two of them.
        for (pad, setup) in [(0usize, 10usize), (300, 300), (600, 5000)] {
            let private = vorbis_private(2, 44_100, pad, setup);
            // The Audio element says something else: the identification header wins.
            let t = webm_track(audio_track(
                1,
                "A_VORBIS",
                Some(&private),
                Some((8_000.0, 1)),
            ))
            .unwrap();
            assert_eq!(t.codec_string, "vorbis");
            assert_eq!(
                (t.sample_rate, t.channels, t.kind),
                (44_100, 2, TrackKind::Audio)
            );
            assert_eq!(
                t.codec,
                CodecConfig::Other {
                    codec: CodecId::Vorbis,
                    config: private
                }
            );
        }
        // No Audio element at all is fine for Vorbis.
        let t = webm_track(audio_track(
            1,
            "A_VORBIS",
            Some(&vorbis_private(1, 48_000, 0, 8)),
            None,
        ))
        .unwrap();
        assert_eq!((t.sample_rate, t.channels), (48_000, 1));
    }

    #[test]
    fn a_broken_vorbis_codec_private_is_malformed() {
        let good = vorbis_private(2, 44_100, 0, 40);
        let track = |p: &[u8]| audio_track(1, "A_VORBIS", Some(p), Some((44_100.0, 2)));
        // The setup header is the last packet and has no length of its own: a cut inside it is still a
        // record as long as its 7 signature bytes are there, so the cuts stop before that.
        for cut in 0..good.len() - 40 {
            let e = webm_refusal(track(&good[..cut]));
            assert!(matches!(e, ParseError::Malformed(_)), "cut at {cut}: {e:?}");
        }
        // Not three packets; a header of the wrong type; no vorbis signature; no channels; rate 0; a
        // lacing run that never ends.
        let mut two = good.clone();
        two[0] = 1;
        let mut wrong_type = good.clone();
        wrong_type[1 + 1 + 1] = 3; // the first packet's type byte
        let mut no_sig = good.clone();
        no_sig[4] = b'x';
        let no_channels = vorbis_private(0, 44_100, 0, 40);
        let no_rate = vorbis_private(2, 0, 0, 40);
        let endless = vec![2, 255, 255, 255];
        for (what, p) in [
            ("two", two),
            ("wrong type", wrong_type),
            ("no signature", no_sig),
            ("no channels", no_channels),
            ("no rate", no_rate),
            ("endless lacing", endless),
        ] {
            let e = webm_refusal(track(&p));
            assert!(matches!(e, ParseError::Malformed(_)), "{what}: {e:?}");
        }
        // A missing CodecPrivate.
        let e = webm_refusal(audio_track(1, "A_VORBIS", None, Some((44_100.0, 2))));
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
    }

    #[test]
    fn a_flac_track_strips_the_marker_and_reads_streaminfo() {
        let blocks = cat(&[
            &flac_block(false, 0, &streaminfo(44_100, 2, 16)),
            &flac_block(true, 4, &[0; 12]),
        ]);
        let marked = cat(&[b"fLaC", &blocks]);
        for private in [&marked, &blocks] {
            let t =
                webm_track(audio_track(1, "A_FLAC", Some(private), Some((8_000.0, 7)))).unwrap();
            assert_eq!(t.codec_string, "flac");
            assert_eq!(
                (t.sample_rate, t.channels, t.kind),
                (44_100, 2, TrackKind::Audio)
            );
            assert_eq!(
                t.codec,
                CodecConfig::Other {
                    codec: CodecId::Flac,
                    config: blocks.clone()
                }
            );
        }
        for cut in 0..blocks.len() {
            if cut == 38 {
                continue; // exactly STREAMINFO: complete
            }
            let e = webm_refusal(audio_track(
                1,
                "A_FLAC",
                Some(&marked[..4 + cut]),
                Some((44_100.0, 2)),
            ));
            assert!(matches!(e, ParseError::Malformed(_)), "cut at {cut}: {e:?}");
        }
        for private in [&b""[..], b"fLaC"] {
            let e = webm_refusal(audio_track(1, "A_FLAC", Some(private), Some((44_100.0, 2))));
            assert!(matches!(e, ParseError::Malformed(_)), "{private:?}: {e:?}");
        }
        let e = webm_refusal(audio_track(1, "A_FLAC", None, Some((44_100.0, 2))));
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
    }

    #[test]
    fn ac3_and_eac3_tracks_take_rate_and_channels_from_the_audio_element() {
        for (id, codec, string) in [
            ("A_AC3", CodecId::Ac3, "ac-3"),
            ("A_EAC3", CodecId::Eac3, "ec-3"),
        ] {
            let t = webm_track(audio_track(1, id, None, Some((48_000.0, 6)))).unwrap();
            assert_eq!(t.codec_string, string);
            assert_eq!(
                (t.sample_rate, t.channels, t.kind),
                (48_000, 6, TrackKind::Audio)
            );
            assert_eq!(
                t.codec,
                CodecConfig::Other {
                    codec,
                    config: Vec::new()
                }
            );
            // No Audio element, no channels, no usable rate: nothing for the decoder's caps.
            for audio in [
                None,
                Some((48_000.0, 0)),
                Some((0.0, 2)),
                Some((f64::NAN, 2)),
                Some((1.0e9, 2)),
            ] {
                let e = webm_refusal(audio_track(1, id, None, audio));
                assert!(
                    matches!(e, ParseError::Malformed(_)),
                    "{id} {audio:?}: {e:?}"
                );
            }
        }
    }

    #[test]
    fn hevc_in_webm_is_refused_with_a_note_and_the_codecs_stay_in_their_kind() {
        let hevc = elem(
            TRACK_ENTRY,
            &cat(&[
                &uint_elem(0xD7, 1),
                &uint_elem(0x83, 1),
                &elem(0x86, b"V_MPEGH/ISO/HEVC"),
                &elem(0x63A2, &main_hvcc()),
            ]),
        );
        match webm_refusal(hevc) {
            ParseError::Unsupported(m) => assert!(m.contains("not a WebM"), "{m}"),
            other => panic!("{other:?}"),
        }
        // A_FLAC in a video track, A_VORBIS in a video track.
        for id in ["A_FLAC", "A_VORBIS", "A_AC3"] {
            let e = webm_refusal(elem(
                TRACK_ENTRY,
                &cat(&[
                    &uint_elem(0xD7, 1),
                    &uint_elem(0x83, 1),
                    &elem(0x86, id.as_bytes()),
                ]),
            ));
            assert!(matches!(e, ParseError::Malformed(_)), "{id}: {e:?}");
        }
    }

    #[test]
    fn frames_of_a_webm_vorbis_track_come_out_with_block_times() {
        let init = webm_init(&[audio_track(
            1,
            "A_VORBIS",
            Some(&vorbis_private(2, 44_100, 0, 20)),
            Some((44_100.0, 2)),
        )]);
        let cluster = cluster(
            1000,
            &[
                simple_block(1, 0, true, &payload(1, 30)),
                simple_block(1, 20, true, &payload(2, 31)),
                simple_block(1, 45, true, &payload(3, 32)),
            ],
        );
        let events = same_under_all_plans(W, &[&init, &cluster]).unwrap();
        let frames = frames_of(&events);
        assert_eq!(frames.len(), 3);
        assert_eq!(
            frames.iter().map(|f| f.pts).collect::<Vec<_>>(),
            vec![1_000_000_000, 1_020_000_000, 1_045_000_000]
        );
        // A frame lasts until the next one; the last repeats the one before.
        assert_eq!(
            frames.iter().map(|f| f.duration).collect::<Vec<_>>(),
            vec![20_000_000, 25_000_000, 25_000_000]
        );
        assert!(
            frames
                .iter()
                .all(|f| f.keyframe && f.kind == TrackKind::Audio)
        );
        assert_eq!(frames[2].data, payload(3, 32));
    }
}
