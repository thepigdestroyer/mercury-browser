// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C01, type support: the 60 strings of the GstMSE spike (R17 sections 3 and 8.1, `mse-conformance`
//! scenario C01) against `parse_type` and `is_type_supported`, with the answer decision D4 gives for each,
//! and every fixture's type string. Where GstMSE was "too lax" (R17 finding 13), the string is refused here.
//! Run with `cargo test -p mercury-mse --test 'parse_*'` (or the filter `parse_`).

mod parse_support;

mod parse_types {
    use super::parse_support::*;
    use mercury_mse::api::{AllDecoders, CodecId, Container, MseError, TrackKind, TypeSupport};
    use mercury_mse::mime::{is_type_supported, parse_type};
    use mercury_mse_testkit::{ChunkPlan, FIXTURES, load};

    /// What a string must give.
    #[derive(Clone, Copy, Debug)]
    enum Want {
        /// Supported, with its container and codecs.
        Yes(Container, &'static [CodecId]),
        /// `MseError::Type`.
        SyntaxError,
        /// `MseError::NotSupported` whose message contains this.
        NotSupported(&'static str),
    }
    use Want::{NotSupported, SyntaxError, Yes};

    const MP4: Container = Container::Mp4;
    const WEBM: Container = Container::WebM;
    use CodecId::{Aac, Ac3, Av1, Eac3, Flac, H264, Hevc, Mp3, Opus, Vorbis, Vp8, Vp9};

    /// The table of `mse-conformance/src/scenarios/c01.rs`, string for string, with what D4 makes of it.
    /// The second column groups them as the spike did.
    #[rustfmt::skip]
    const TABLE: &[(&str, Want)] = &[
        // H.264 in MP4 (8 of 8 supported)
        (r#"video/mp4; codecs="avc1.42E01E""#, Yes(MP4, &[H264])),
        (r#"video/mp4; codecs="avc1.42C01E""#, Yes(MP4, &[H264])),
        (r#"video/mp4; codecs="avc1.4D401F""#, Yes(MP4, &[H264])),
        (r#"video/mp4; codecs="avc1.64001F""#, Yes(MP4, &[H264])),
        (r#"video/mp4; codecs="avc1.640028""#, Yes(MP4, &[H264])),
        (r#"video/mp4; codecs="avc3.640028""#, Yes(MP4, &[H264])),
        (r#"video/mp4; codecs="avc1.4D4028""#, Yes(MP4, &[H264])),
        (r#"video/mp4; codecs="avc1.640033""#, Yes(MP4, &[H264])),
        // HEVC: out in round 1 (D4), in since lane med-1b (D7)
        (r#"video/mp4; codecs="hev1.1.6.L93.B0""#, Yes(MP4, &[Hevc])),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0""#, Yes(MP4, &[Hevc])),
        (r#"video/mp4; codecs="hvc1.2.4.L153.B0""#, Yes(MP4, &[Hevc])),
        // AV1 (4 of 4)
        (r#"video/mp4; codecs="av01.0.05M.08""#, Yes(MP4, &[Av1])),
        (r#"video/mp4; codecs="av01.0.08M.10""#, Yes(MP4, &[Av1])),
        (r#"video/webm; codecs="av01.0.05M.08""#, Yes(WEBM, &[Av1])),
        (r#"video/webm; codecs="av01.0.08M.10""#, Yes(WEBM, &[Av1])),
        // VP8 / VP9
        (r#"video/webm; codecs="vp09.00.10.08""#, Yes(WEBM, &[Vp9])),
        (r#"video/webm; codecs="vp09.00.51.08""#, Yes(WEBM, &[Vp9])),
        (r#"video/webm; codecs="vp09.02.10.10""#, Yes(WEBM, &[Vp9])),
        (r#"video/webm; codecs="vp9""#, Yes(WEBM, &[Vp9])),
        (r#"video/webm; codecs="vp8""#, Yes(WEBM, &[Vp8])),
        (r#"video/mp4; codecs="vp09.00.10.08""#, Yes(MP4, &[Vp9])),
        // audio in MP4
        (r#"audio/mp4; codecs="mp4a.40.2""#, Yes(MP4, &[Aac])),
        (r#"audio/mp4; codecs="mp4a.40.5""#, Yes(MP4, &[Aac])),
        (r#"audio/mp4; codecs="mp4a.40.29""#, Yes(MP4, &[Aac])),
        (r#"audio/mp4; codecs="mp4a.69""#, Yes(MP4, &[Mp3])),
        (r#"audio/mp4; codecs="mp4a.6B""#, Yes(MP4, &[Mp3])),
        // The two strings players ask that the registry can decode: refused in round 1 with a note (D4),
        // accepted since lane med-1b (D7).
        (r#"audio/mp4; codecs="ac-3""#, Yes(MP4, &[Ac3])),
        (r#"audio/mp4; codecs="ec-3""#, Yes(MP4, &[Eac3])),
        (r#"audio/mp4; codecs="opus""#, Yes(MP4, &[Opus])),
        (r#"audio/mp4; codecs="flac""#, Yes(MP4, &[Flac])),
        (r#"audio/mp4; codecs="mp3""#, Yes(MP4, &[Mp3])),
        // audio in WebM
        (r#"audio/webm; codecs="opus""#, Yes(WEBM, &[Opus])),
        (r#"audio/webm; codecs="vorbis""#, Yes(WEBM, &[Vorbis])),
        // elementary and transport streams: out (D4, still out in D7, with the note)
        (r#"audio/mpeg"#, NotSupported("audio/mpeg")),
        (r#"audio/mpeg; codecs="mp3""#, NotSupported("audio/mpeg")),
        (r#"audio/aac"#, NotSupported("audio/aac")),
        (r#"video/mp2t; codecs="avc1.42E01E,mp4a.40.2""#, NotSupported("video/mp2t")),
        (r#"video/mp2t"#, NotSupported("video/mp2t")),
        // muxed pairs, in any order, with or without the space
        (r#"video/mp4; codecs="avc1.42E01E, mp4a.40.2""#, Yes(MP4, &[H264, Aac])),
        (r#"video/mp4; codecs="avc1.64001F,mp4a.40.2""#, Yes(MP4, &[H264, Aac])),
        (r#"video/mp4; codecs="mp4a.40.2, avc1.42E01E""#, Yes(MP4, &[Aac, H264])),
        (r#"video/webm; codecs="vp09.00.10.08, opus""#, Yes(WEBM, &[Vp9, Opus])),
        (r#"video/webm; codecs="vp8, vorbis""#, Yes(WEBM, &[Vp8, Vorbis])),
        // no codecs parameter at all
        (r#"video/mp4"#, Yes(MP4, &[])),
        (r#"audio/mp4"#, Yes(MP4, &[])),
        (r#"video/webm"#, Yes(WEBM, &[])),
        (r#"audio/webm"#, Yes(WEBM, &[])),
        // syntax variants
        (r#"VIDEO/MP4; codecs="avc1.42E01E""#, Yes(MP4, &[H264])),
        (r#"video/mp4; codecs=avc1.42E01E"#, Yes(MP4, &[H264])),
        (r#"video/mp4; codecs="avc1.42E01E"; foo=bar"#, Yes(MP4, &[H264])),
        // H.264 in WebM and a video codec in an audio type: GstMSE said yes, which was wrong
        (r#"video/webm; codecs="avc1.42E01E""#, NotSupported("not supported in video/webm")),
        (r#"audio/mp4; codecs="avc1.42E01E""#, NotSupported("video codec in an audio type")),
        // bogus
        (r#"video/foo"#, NotSupported("video/foo")),
        (r#"video/mp4; codecs="foo.bar""#, NotSupported("not recognised")),
        (r#""#, SyntaxError),
        (r#"video/mp4; codecs="avc1.42E01E, avc1.64001F""#, NotSupported("more than one video codec")),
        (r#"video/mp4; codecs=""#, SyntaxError),
        (r#"application/octet-stream"#, NotSupported("application/octet-stream")),
        (r#"text/plain"#, NotSupported("text/plain")),
        (r#"video/mp4; codecs="""#, NotSupported("empty")),
    ];

    /// The ten strings of the fixtures (`FIXTURES`) and the ones the testkit derives from the files: 18
    /// distinct strings in R17's count; here every one of both kinds.
    #[test]
    fn c01_every_fixture_type_string_is_supported_and_names_what_the_parser_finds() {
        let mut checked = 0;
        for f in FIXTURES {
            let stream = load(f.name).unwrap();
            for mime in [f.mime, stream.mime.as_str()] {
                assert!(is_type_supported(mime, &AllDecoders), "{}: {mime}", f.name);
                let t = parse_type(mime).unwrap();
                assert_eq!(t.original, mime);
                checked += 1;
                // What the type promises is what the init segment holds.
                let fed = feed_stream(&stream, &ChunkPlan::Whole).unwrap();
                let init = init_of(&fed.events)[0].clone();
                let kinds: Vec<TrackKind> = init.tracks.iter().map(|t| t.kind).collect();
                let mut promised = t.kinds.clone();
                let mut found = kinds.clone();
                promised.sort();
                found.sort();
                assert_eq!(promised, found, "{}: {mime}", f.name);
                let ids: Vec<CodecId> = init.tracks.iter().map(|t| t.codec.id()).collect();
                let mut want = t.codecs.clone();
                let mut got = ids;
                want.sort_by_key(|c| format!("{c:?}"));
                got.sort_by_key(|c| format!("{c:?}"));
                assert_eq!(want, got, "{}: {mime}", f.name);
                assert_eq!(t.container, container_of(stream.container));
            }
        }
        assert_eq!(checked, 34, "17 fixtures, two strings each");
    }

    #[test]
    fn c01_the_sixty_strings_of_the_spike() {
        assert_eq!(TABLE.len(), 60, "the table is the spike's table");
        let unique: std::collections::BTreeSet<&str> = TABLE.iter().map(|(m, _)| *m).collect();
        assert_eq!(unique.len(), 60, "no string twice");
        let mut supported = 0;
        let mut notes = Vec::new();
        for (mime, want) in TABLE {
            let got = parse_type(mime);
            match (want, &got) {
                (Yes(container, codecs), Ok(t)) => {
                    supported += 1;
                    assert_eq!(t.container, *container, "{mime}");
                    assert_eq!(&t.codecs, codecs, "{mime}");
                    assert!(is_type_supported(mime, &AllDecoders), "{mime}");
                    // Kinds: from the codecs, else from the major type.
                    if codecs.is_empty() {
                        let major = if mime.to_ascii_lowercase().starts_with("audio") {
                            TrackKind::Audio
                        } else {
                            TrackKind::Video
                        };
                        assert_eq!(t.kinds, vec![major], "{mime}");
                    } else {
                        let mut kinds = Vec::new();
                        for c in *codecs {
                            if !kinds.contains(&c.kind()) {
                                kinds.push(c.kind());
                            }
                        }
                        assert_eq!(t.kinds, kinds, "{mime}");
                    }
                }
                (SyntaxError, Err(MseError::Type(_))) => {}
                (NotSupported(word), Err(MseError::NotSupported(msg))) => {
                    assert!(
                        msg.contains(word),
                        "{mime}: {msg:?} should mention {word:?}"
                    );
                    // The elementary and transport streams are refused with the D7 note.
                    if msg.contains("D7") {
                        notes.push(format!("{mime}: {msg}"));
                    }
                }
                (want, got) => panic!("{mime}: wanted {want:?}, got {got:?}"),
            }
            if !matches!(want, Yes(..)) {
                assert!(
                    !is_type_supported(mime, &AllDecoders),
                    "{mime} must be refused"
                );
            }
        }
        assert_eq!(
            supported, 45,
            "45 of 60 are supported since decision D7 (34 in round 1; GstMSE said 54: it was too lax)"
        );
        // What is still refused as a stream format says so, not silently: MPEG audio, ADTS and MPEG-TS
        // streams (five strings of the table).
        assert_eq!(notes.len(), 5, "{notes:?}");
        for n in &notes {
            assert!(n.contains("decision D7"), "{n}");
            println!("NOTE C01: {n}");
        }
    }

    /// What decision D7 adds to the table of the spike: the new codecs in every container and
    /// combination, and the strings that must stay refused next to them (a wrong container for the
    /// codec, a malformed HEVC string, near misses of the registered names).
    #[rustfmt::skip]
    const D7_TABLE: &[(&str, Want)] = &[
        // HEVC in MP4 (hvc1 and hev1, profiles, tiers, levels, constraint bytes)
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0""#, Yes(MP4, &[Hevc])),
        (r#"video/mp4; codecs="hvc1.1.6.L90""#, Yes(MP4, &[Hevc])),
        (r#"video/mp4; codecs="hvc1.2.4.L120.90""#, Yes(MP4, &[Hevc])),
        (r#"video/mp4; codecs="hvc1.2.4.H153.B0""#, Yes(MP4, &[Hevc])),
        (r#"video/mp4; codecs="hev1.1.6.L93.B0""#, Yes(MP4, &[Hevc])),
        (r#"video/mp4; codecs="hvc1.A1.6.L93""#, Yes(MP4, &[Hevc])),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0.00.00.00.00.80""#, Yes(MP4, &[Hevc])),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0, mp4a.40.2""#, Yes(MP4, &[Hevc, Aac])),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0, ec-3""#, Yes(MP4, &[Hevc, Eac3])),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0, ac-3""#, Yes(MP4, &[Hevc, Ac3])),
        (r#"video/mp4; codecs="avc1.640028, ec-3""#, Yes(MP4, &[H264, Eac3])),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0, flac""#, Yes(MP4, &[Hevc, Flac])),
        // audio codecs in MP4
        (r#"audio/mp4; codecs="mp4a.6B""#, Yes(MP4, &[Mp3])),
        (r#"audio/mp4; codecs="mp4a.6b""#, Yes(MP4, &[Mp3])),
        (r#"audio/mp4; codecs="mp4a.69""#, Yes(MP4, &[Mp3])),
        (r#"audio/mp4; codecs="mp3""#, Yes(MP4, &[Mp3])),
        (r#"audio/mp4; codecs="flac""#, Yes(MP4, &[Flac])),
        (r#"audio/mp4; codecs="ac-3""#, Yes(MP4, &[Ac3])),
        (r#"audio/mp4; codecs="ec-3""#, Yes(MP4, &[Eac3])),
        (r#"audio/mp4; codecs="EC-3""#, Yes(MP4, &[Eac3])),
        // audio codecs in WebM
        (r#"audio/webm; codecs="vorbis""#, Yes(WEBM, &[Vorbis])),
        (r#"audio/webm; codecs="flac""#, Yes(WEBM, &[Flac])),
        (r#"audio/webm; codecs="ac-3""#, Yes(WEBM, &[Ac3])),
        (r#"audio/webm; codecs="ec-3""#, Yes(WEBM, &[Eac3])),
        (r#"video/webm; codecs="vp9, vorbis""#, Yes(WEBM, &[Vp9, Vorbis])),
        (r#"video/webm; codecs="av01.0.05M.08, flac""#, Yes(WEBM, &[Av1, Flac])),
        // the wrong container for a codec
        (r#"video/webm; codecs="hvc1.1.6.L93.B0""#, NotSupported("not supported in video/webm")),
        (r#"video/webm; codecs="hev1.1.6.L93.B0""#, NotSupported("not supported in video/webm")),
        (r#"audio/webm; codecs="mp3""#, NotSupported("not supported in audio/webm")),
        (r#"audio/webm; codecs="mp4a.6B""#, NotSupported("not supported in audio/webm")),
        (r#"audio/mp4; codecs="vorbis""#, NotSupported("not supported in audio/mp4")),
        (r#"audio/mp4; codecs="hvc1.1.6.L93.B0""#, NotSupported("video codec in an audio type")),
        // two codecs of one kind
        (r#"audio/mp4; codecs="ac-3, ec-3""#, NotSupported("more than one audio codec")),
        (r#"audio/mp4; codecs="mp3, flac""#, NotSupported("more than one audio codec")),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0, avc1.640028""#, NotSupported("more than one video codec")),
        // malformed HEVC strings
        (r#"video/mp4; codecs="hvc1""#, NotSupported("is not valid")),
        (r#"video/mp4; codecs="hvc1.1.6""#, NotSupported("is not valid")),
        (r#"video/mp4; codecs="hvc1.1.6.L0""#, NotSupported("is not valid")),
        (r#"video/mp4; codecs="hvc1.1.6.L256""#, NotSupported("is not valid")),
        (r#"video/mp4; codecs="hvc1.1.6.M93""#, NotSupported("is not valid")),
        (r#"video/mp4; codecs="hvc1.32.6.L93""#, NotSupported("is not valid")),
        (r#"video/mp4; codecs="hvc1.1.G.L93""#, NotSupported("is not valid")),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0.B0.B0.B0.B0.B0.B0""#, NotSupported("is not valid")),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B00""#, NotSupported("is not valid")),
        // names that are not the registered ones
        (r#"audio/mp4; codecs="ac3""#, NotSupported("registered names")),
        (r#"audio/mp4; codecs="ec3""#, NotSupported("registered names")),
        (r#"audio/mp4; codecs="flac.1""#, NotSupported("not recognised")),
        (r#"audio/mp4; codecs="mp4a.A5""#, NotSupported("AC-3")),
        (r#"audio/mp4; codecs="mp4a.A6""#, NotSupported("E-AC-3")),
        (r#"video/mp4; codecs="dvh1.05.06""#, NotSupported("Dolby Vision")),
        (r#"video/mp4; codecs="dvhe.05.06""#, NotSupported("Dolby Vision")),
        // still out (D7): raw MPEG audio and ADTS streams and MPEG-TS
        (r#"audio/mpeg"#, NotSupported("D7")),
        (r#"audio/mp3"#, NotSupported("D7")),
        (r#"audio/aac"#, NotSupported("D7")),
        (r#"audio/x-aac"#, NotSupported("D7")),
        (r#"video/mp2t; codecs="hvc1.1.6.L93.B0""#, NotSupported("D7")),
        (r#"audio/mp2t; codecs="mp4a.40.2""#, NotSupported("D7")),
    ];

    #[test]
    fn c01_the_strings_of_decision_d7() {
        let unique: std::collections::BTreeSet<&str> = D7_TABLE.iter().map(|(m, _)| *m).collect();
        assert_eq!(unique.len(), D7_TABLE.len(), "no string twice");
        for (mime, want) in D7_TABLE {
            let got = parse_type(mime);
            match (want, &got) {
                (Yes(container, codecs), Ok(t)) => {
                    assert_eq!(t.container, *container, "{mime}");
                    assert_eq!(&t.codecs, codecs, "{mime}");
                    assert!(is_type_supported(mime, &AllDecoders), "{mime}");
                }
                (NotSupported(word), Err(MseError::NotSupported(msg))) => {
                    assert!(
                        msg.contains(word),
                        "{mime}: {msg:?} should mention {word:?}"
                    );
                    assert!(
                        !is_type_supported(mime, &AllDecoders),
                        "{mime} must be refused"
                    );
                }
                (want, got) => panic!("{mime}: wanted {want:?}, got {got:?}"),
            }
        }
    }

    #[test]
    fn d7_codecs_need_a_decoder_each() {
        struct Only(Vec<CodecId>);
        impl TypeSupport for Only {
            fn has_decoder(&self, _kind: TrackKind, codec: CodecId) -> bool {
                self.0.contains(&codec)
            }
        }
        for (mime, codec) in [
            (r#"video/mp4; codecs="hvc1.1.6.L93.B0""#, Hevc),
            (r#"audio/mp4; codecs="ac-3""#, Ac3),
            (r#"audio/mp4; codecs="ec-3""#, Eac3),
            (r#"audio/mp4; codecs="mp3""#, Mp3),
            (r#"audio/mp4; codecs="flac""#, Flac),
            (r#"audio/webm; codecs="vorbis""#, Vorbis),
        ] {
            assert!(is_type_supported(mime, &Only(vec![codec])), "{mime}");
            assert!(
                !is_type_supported(mime, &Only(vec![H264, Aac, Opus])),
                "{mime}"
            );
        }
    }

    #[test]
    fn c01_the_too_lax_cases_of_r17_are_refused() {
        for (mime, why) in [
            ("video/foo", "an unknown media type"),
            (
                r#"video/mp4; codecs="avc1.42E01E, avc1.64001F""#,
                "two video codecs",
            ),
            (r#"video/mp4; codecs=""#, "a truncated codecs parameter"),
            (r#"video/mp4; codecs="avc1.42E01E"#, "an unterminated quote"),
            (
                r#"audio/mp4; codecs="avc1.42E01E""#,
                "a video codec in audio/mp4",
            ),
            (r#"audio/webm; codecs="vp9""#, "a video codec in audio/webm"),
            (r#"video/webm; codecs="avc1.42E01E""#, "H.264 in WebM"),
            (r#"video/mp4; codecs="""#, "an empty codecs list"),
            (r#"video/mp4; codecs="mp4a.40.2, opus""#, "two audio codecs"),
            ("", "the empty string"),
            ("video", "no subtype"),
            ("video/mp4; codecs", "a parameter without a value"),
        ] {
            assert!(!is_type_supported(mime, &AllDecoders), "{why}: {mime:?}");
            let e = parse_type(mime).unwrap_err();
            // What addSourceBuffer throws: TypeError for the empty string and syntax errors, NotSupportedError
            // for the rest.
            let name = e.dom_exception_name();
            let expected = if mime.is_empty()
                || mime == "video"
                || mime == "video/mp4; codecs"
                || mime == r#"video/mp4; codecs=""#
                || mime == r#"video/mp4; codecs="avc1.42E01E"# {
                "TypeError"
            } else {
                "NotSupportedError"
            };
            assert_eq!(name, expected, "{why}: {mime:?} gave {e}");
        }
    }

    #[test]
    fn is_type_supported_follows_the_platforms_decoders() {
        struct Without(CodecId);
        impl TypeSupport for Without {
            fn has_decoder(&self, _kind: TrackKind, codec: CodecId) -> bool {
                codec != self.0
            }
        }
        let av1 = r#"video/webm; codecs="av01.0.05M.08""#;
        assert!(is_type_supported(av1, &AllDecoders));
        assert!(!is_type_supported(av1, &Without(CodecId::Av1)));
        assert!(is_type_supported(av1, &Without(CodecId::Opus)));
        let both = r#"video/mp4; codecs="avc1.42E01E, mp4a.40.2""#;
        assert!(!is_type_supported(both, &Without(CodecId::Aac)));
        assert!(!is_type_supported(both, &Without(CodecId::H264)));
        // HEVC is a type of this build now; it needs a decoder like every other codec.
        let hevc = r#"video/mp4; codecs="hvc1.1.6.L93.B0""#;
        assert!(is_type_supported(hevc, &AllDecoders));
        assert!(!is_type_supported(hevc, &Without(CodecId::Hevc)));
        // A type that is refused stays refused whatever the platform has.
        assert!(!is_type_supported("audio/mpeg", &AllDecoders));
        assert!(!is_type_supported(
            r#"video/mp4; codecs="dvh1.05.06""#,
            &AllDecoders
        ));
    }
}
