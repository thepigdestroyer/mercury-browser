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
    use CodecId::{Aac, Av1, H264, Opus, Vp8, Vp9};

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
        // HEVC: out in round 1 (D4)
        (r#"video/mp4; codecs="hev1.1.6.L93.B0""#, NotSupported("HEVC")),
        (r#"video/mp4; codecs="hvc1.1.6.L93.B0""#, NotSupported("HEVC")),
        (r#"video/mp4; codecs="hvc1.2.4.L153.B0""#, NotSupported("HEVC")),
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
        (r#"audio/mp4; codecs="mp4a.69""#, NotSupported("MP3")),
        (r#"audio/mp4; codecs="mp4a.6B""#, NotSupported("MP3")),
        // The two strings players ask that the registry could decode: refused for now with a note (D4).
        (r#"audio/mp4; codecs="ac-3""#, NotSupported("AC-3")),
        (r#"audio/mp4; codecs="ec-3""#, NotSupported("E-AC-3")),
        (r#"audio/mp4; codecs="opus""#, Yes(MP4, &[Opus])),
        (r#"audio/mp4; codecs="flac""#, NotSupported("FLAC")),
        (r#"audio/mp4; codecs="mp3""#, NotSupported("MP3")),
        // audio in WebM
        (r#"audio/webm; codecs="opus""#, Yes(WEBM, &[Opus])),
        (r#"audio/webm; codecs="vorbis""#, NotSupported("Vorbis")),
        // elementary and transport streams: out (D4)
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
        (r#"video/webm; codecs="vp8, vorbis""#, NotSupported("Vorbis")),
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
        assert_eq!(checked, 20);
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
                    if msg.contains("AC-3") {
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
            supported, 34,
            "34 of 60 are supported (GstMSE said 54: it was too lax and refused AC-3 and E-AC-3)"
        );
        // AC-3 and E-AC-3 are refused with a note, not silently: they are the two strings players ask
        // that the registry could decode (R17 8.1); EA2 decides.
        assert_eq!(notes.len(), 2, "{notes:?}");
        for n in &notes {
            assert!(n.contains("decision D4") && n.contains("EA2"), "{n}");
            println!("NOTE C01: {n}");
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
        // A refused type stays refused whatever the platform has: HEVC has a decoder in the registry.
        assert!(!is_type_supported(
            r#"video/mp4; codecs="hvc1.1.6.L93.B0""#,
            &AllDecoders
        ));
    }
}
