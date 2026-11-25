// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Type strings: `addSourceBuffer(type)`, `changeType(type)` and `MediaSource.isTypeSupported(type)`.
//!
//! OWNER: item 1 of lane med-1 (EA2 core). The signatures are the contract with item 3 and with the
//! glue (`MseBackend::is_type_supported`). Decision D4 says which containers and codecs are accepted.
//!
//! Written from RFC 6838 (media type syntax), RFC 6381 (codec strings), the MSE `isTypeSupported` and
//! `addSourceBuffer` steps and the codec registrations (`avc1`/`avc3`, `vp09`, `av01`, `mp4a.40.x`,
//! `opus`). The 60 strings of R17 section 8.1 are the test table (`tests/parse_types.rs`).
//!
//! Two kinds of refusal, as in the spec: a string that is not a media type with parameters at all (an
//! empty string, no `/`, an unterminated quote, an empty entry in the `codecs` list) is
//! [`MseError::Type`]; a string that is well formed but names something this build does not feed is
//! [`MseError::NotSupported`], and the message says which part. What is accepted:
//!
//! | container | codecs |
//! |---|---|
//! | `video/mp4`, `audio/mp4` | `avc1.PPCCLL`, `avc3.PPCCLL`, `vp09[.PP.LL.DD...]`, `vp9`, `av01.P.LLT.DD...`, `mp4a.40.2`, `mp4a.40.5`, `mp4a.40.29`, `opus` |
//! | `video/webm`, `audio/webm` | `vp8`, `vp08...`, `vp09...`, `vp9`, `av01...`, `opus` |
//!
//! At most one video and one audio codec (decision D3); no video codec in `audio/*`. HEVC, AC-3,
//! E-AC-3, MP3 (`mp4a.69`, `mp4a.6B`, `mp3`), FLAC, Vorbis, MPEG-TS and the raw `audio/mpeg` and
//! `audio/aac` streams are refused (decision D4). Codec names and the hex digits in them are matched
//! without regard to case; parameters other than `codecs` are accepted and ignored; when `codecs` is
//! given twice the first wins.

use crate::api::{CodecId, Container, MseError, TrackKind, TypeSupport};

/// A parsed MIME type with codecs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedType {
    pub container: Container,
    /// The kinds of track the type promises (from the codecs, or from the major type when `codecs` is
    /// absent).
    pub kinds: Vec<TrackKind>,
    /// The codecs named, in order; empty when the type has no `codecs` parameter.
    pub codecs: Vec<CodecId>,
    /// The type as written, trimmed (the DOM does not need it normalised).
    pub original: String,
}

/// Parses a type string. `MseError::Type` for a syntax error or an empty string,
/// `MseError::NotSupported` for a container or codec that decision D4 excludes, and for two video
/// codecs, a video codec in `audio/*` and the other combinations that cannot be one source buffer.
pub fn parse_type(mime: &str) -> Result<ParsedType, MseError> {
    let original = mime.trim_matches(is_space).to_string();
    if original.is_empty() {
        return Err(MseError::Type("the type string is empty".into()));
    }
    let (essence, rest) = match original.split_once(';') {
        Some((e, r)) => (e.trim_matches(is_space), Some(r)),
        None => (original.as_str(), None),
    };
    let (major, minor) = essence
        .split_once('/')
        .ok_or_else(|| MseError::Type(format!("{essence:?} is not of the form type/subtype")))?;
    if !is_token(major) || !is_token(minor) {
        return Err(MseError::Type(format!(
            "{essence:?} is not a valid media type"
        )));
    }
    let params = match rest {
        Some(r) => parse_parameters(r)?,
        None => Vec::new(),
    };
    let codecs_value = params
        .iter()
        .find(|(name, _)| name == "codecs")
        .map(|(_, v)| v.as_str());

    let major = major.to_ascii_lowercase();
    let minor = minor.to_ascii_lowercase();
    let container = match (major.as_str(), minor.as_str()) {
        ("video" | "audio", "mp4") => Container::Mp4,
        ("video" | "audio", "webm") => Container::WebM,
        _ => {
            return Err(MseError::NotSupported(format!(
                "the media type {major}/{minor} is not supported"
            )));
        }
    };

    let mut codecs = Vec::new();
    if let Some(value) = codecs_value {
        if value.trim_matches(is_space).is_empty() {
            return Err(MseError::NotSupported("the codecs list is empty".into()));
        }
        for item in value.split(',') {
            let item = item.trim_matches(is_space);
            if item.is_empty() {
                return Err(MseError::Type("the codecs list has an empty entry".into()));
            }
            let id = classify(item)?;
            if !container_has(container, id) {
                return Err(MseError::NotSupported(format!(
                    "the codec {item:?} is not supported in {major}/{minor}"
                )));
            }
            codecs.push(id);
        }
    }

    let mut kinds: Vec<TrackKind> = Vec::new();
    for id in &codecs {
        if !kinds.contains(&id.kind()) {
            kinds.push(id.kind());
        }
    }
    for kind in [TrackKind::Video, TrackKind::Audio] {
        if codecs.iter().filter(|c| c.kind() == kind).count() > 1 {
            return Err(MseError::NotSupported(format!(
                "more than one {} codec in one source buffer (decision D3)",
                kind.name()
            )));
        }
    }
    if major == "audio" && kinds.contains(&TrackKind::Video) {
        return Err(MseError::NotSupported(
            "a video codec in an audio type".into(),
        ));
    }
    if codecs.is_empty() {
        kinds.push(if major == "audio" {
            TrackKind::Audio
        } else {
            TrackKind::Video
        });
    }
    Ok(ParsedType {
        container,
        kinds,
        codecs,
        original,
    })
}

/// `MediaSource.isTypeSupported`: [`parse_type`] succeeds and `types` has a decoder for every codec.
pub fn is_type_supported(mime: &str, types: &dyn TypeSupport) -> bool {
    match parse_type(mime) {
        Ok(p) => p.codecs.iter().all(|&c| types.has_decoder(c.kind(), c)),
        Err(_) => false,
    }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// An RFC 7230 token: one or more of the characters below.
fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// The parameters after the first `;`: `(lower-case name, value)` in order. Values are tokens, anything
/// up to the next `;` (lenient, so an unquoted `codecs=a,b` works), or quoted strings with `\` escapes.
fn parse_parameters(s: &str) -> Result<Vec<(String, String)>, MseError> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.next_if(|&c| is_space(c) || c == ';').is_some() {}
        if chars.peek().is_none() {
            return Ok(out);
        }
        let mut name = String::new();
        while let Some(c) = chars.next_if(|&c| c != '=' && c != ';') {
            name.push(c);
        }
        let name = name.trim_matches(is_space).to_ascii_lowercase();
        if chars.next_if_eq(&'=').is_none() || !is_token(&name) {
            return Err(MseError::Type(format!(
                "the parameter {name:?} has no value or a bad name"
            )));
        }
        while chars.next_if(|&c| is_space(c)).is_some() {}
        let mut value = String::new();
        if chars.next_if_eq(&'"').is_some() {
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next() {
                        Some(e) => value.push(e),
                        None => break,
                    },
                    _ => value.push(c),
                }
            }
            if !closed {
                return Err(MseError::Type(format!(
                    "the quoted value of {name:?} is not terminated"
                )));
            }
            while chars.next_if(|&c| is_space(c)).is_some() {}
            if chars.peek().is_some_and(|&c| c != ';') {
                return Err(MseError::Type(format!(
                    "text after the quoted value of {name:?}"
                )));
            }
        } else {
            while let Some(c) = chars.next_if(|&c| c != ';') {
                value.push(c);
            }
            value = value.trim_matches(is_space).to_string();
            if value.is_empty() {
                return Err(MseError::Type(format!(
                    "the parameter {name:?} has no value"
                )));
            }
        }
        out.push((name, value));
    }
}

/// Which codecs each container byte stream format is parsed for (decision D4).
fn container_has(container: Container, codec: CodecId) -> bool {
    match container {
        Container::Mp4 => matches!(
            codec,
            CodecId::H264 | CodecId::Vp9 | CodecId::Av1 | CodecId::Aac | CodecId::Opus
        ),
        Container::WebM => matches!(
            codec,
            CodecId::Vp8 | CodecId::Vp9 | CodecId::Av1 | CodecId::Opus
        ),
    }
}

fn refused(codec: &str, what: &str) -> MseError {
    MseError::NotSupported(format!(
        "the codec {codec:?} ({what}) is not supported yet (decision D4)"
    ))
}

fn bad_codec(codec: &str, why: &str) -> MseError {
    MseError::NotSupported(format!("the codec string {codec:?} is not valid: {why}"))
}

fn decimal_2(s: &str) -> Option<u32> {
    if s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

/// `PP.LL.DD` and the optional `.CC.cp.tc.mc.FF` of `vp09` and `vp08` strings.
fn check_vp_fields(codec: &str, rest: &str) -> Result<(), MseError> {
    let f: Vec<&str> = rest.split('.').collect();
    if f.len() != 3 && f.len() != 8 {
        return Err(bad_codec(codec, "expected PP.LL.DD or all eight fields"));
    }
    if f.iter().any(|x| decimal_2(x).is_none()) {
        return Err(bad_codec(codec, "every field is two decimal digits"));
    }
    let n = |i: usize| decimal_2(f[i]).unwrap_or(0);
    if n(0) > 3 || ![8, 10, 12].contains(&n(2)) {
        return Err(bad_codec(codec, "profile 0 to 3 and bit depth 8, 10 or 12"));
    }
    Ok(())
}

/// The codec a codec string names, when this build feeds it.
fn classify(codec: &str) -> Result<CodecId, MseError> {
    let lower = codec.to_ascii_lowercase();
    let (family, rest) = match lower.split_once('.') {
        Some((f, r)) => (f, Some(r)),
        None => (lower.as_str(), None),
    };
    match family {
        "avc1" | "avc3" => match rest {
            Some(r) if r.len() == 6 && r.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Ok(CodecId::H264)
            }
            _ => Err(bad_codec(
                codec,
                "avc1 and avc3 need profile, constraints and level as six hex digits",
            )),
        },
        "hev1" | "hvc1" | "dvh1" | "dvhe" => Err(refused(codec, "HEVC")),
        "vp09" => {
            if let Some(r) = rest {
                check_vp_fields(codec, r)?;
            }
            Ok(CodecId::Vp9)
        }
        "vp9" if rest.is_none() => Ok(CodecId::Vp9),
        "vp08" => {
            if let Some(r) = rest {
                check_vp_fields(codec, r)?;
            }
            Ok(CodecId::Vp8)
        }
        "vp8" if rest.is_none() => Ok(CodecId::Vp8),
        "av01" => {
            let f: Vec<&str> = rest.unwrap_or("").split('.').collect();
            let profile_ok = f.first().is_some_and(|p| matches!(*p, "0" | "1" | "2"));
            let level_ok = f.get(1).is_some_and(|l| {
                l.len() == 3
                    && l.is_ascii()
                    && l[..2].bytes().all(|b| b.is_ascii_digit())
                    && l[..2].parse::<u32>().is_ok_and(|v| v < 32)
                    && matches!(&l[2..], "m" | "h")
            });
            let depth_ok = f.get(2).is_some_and(|d| matches!(*d, "08" | "10" | "12"));
            if profile_ok && level_ok && depth_ok {
                Ok(CodecId::Av1)
            } else {
                Err(bad_codec(codec, "expected av01.P.LLT.DD"))
            }
        }
        "mp4a" => match rest {
            Some(r) if r.starts_with("40.") => match r[3..].parse::<u32>() {
                Ok(2 | 5 | 29) if !r[3..].starts_with('+') => Ok(CodecId::Aac),
                Ok(_) => Err(refused(codec, "an AAC variant other than LC, HE and HE v2")),
                Err(_) => Err(bad_codec(codec, "the audio object type is not a number")),
            },
            Some("69" | "6b") => Err(refused(codec, "MP3")),
            Some("a5") => Err(refused(codec, "AC-3")),
            Some("a6") => Err(refused(codec, "E-AC-3")),
            _ => Err(refused(
                codec,
                "an MPEG-4 audio object type that is not AAC",
            )),
        },
        "opus" if rest.is_none() => Ok(CodecId::Opus),
        "mp3" => Err(refused(codec, "MP3")),
        "flac" => Err(refused(codec, "FLAC")),
        "vorbis" => Err(refused(codec, "Vorbis")),
        "ac-3" | "ac3" => Err(refused(codec, "AC-3; EA2 decides")),
        "ec-3" | "ec3" => Err(refused(codec, "E-AC-3; EA2 decides")),
        _ => Err(MseError::NotSupported(format!(
            "the codec {codec:?} is not recognised"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AllDecoders;

    fn ok(s: &str) -> ParsedType {
        parse_type(s).unwrap_or_else(|e| panic!("{s:?}: {e}"))
    }

    #[test]
    fn a_plain_type_gets_its_kinds_from_the_major_type() {
        let p = ok("video/mp4");
        assert_eq!(
            (p.container, p.kinds, p.codecs),
            (Container::Mp4, vec![TrackKind::Video], vec![])
        );
        assert_eq!(ok("audio/webm").kinds, vec![TrackKind::Audio]);
        assert_eq!(ok("  Video/WebM  ").original, "Video/WebM");
    }

    #[test]
    fn codecs_come_in_order_with_their_kinds() {
        let p = ok(r#"video/mp4; codecs="mp4a.40.2, avc1.42E01E""#);
        assert_eq!(p.codecs, vec![CodecId::Aac, CodecId::H264]);
        assert_eq!(p.kinds, vec![TrackKind::Audio, TrackKind::Video]);
        let p = ok(r#"audio/mp4;codecs=opus"#);
        assert_eq!(p.codecs, vec![CodecId::Opus]);
    }

    #[test]
    fn parameter_syntax() {
        // Whitespace, case, quoting with escapes, first `codecs` wins, other parameters ignored.
        assert_eq!(
            ok(r#"video/mp4 ;  CODECS = "avc1.42E01E" ; a=b"#).codecs,
            vec![CodecId::H264]
        );
        assert_eq!(
            ok(r#"video/mp4; codecs="avc1.42E01E"; codecs="vp09.00.10.08""#).codecs,
            vec![CodecId::H264]
        );
        assert_eq!(
            ok(r#"video/mp4; x="a\"b;c"; codecs=avc1.42E01E"#).codecs,
            vec![CodecId::H264]
        );
        assert_eq!(ok("video/mp4;").codecs, vec![]);
        assert_eq!(ok("video/mp4; ; ;").codecs, vec![]);
        for bad in [
            "",
            "   ",
            "video",
            "video/",
            "/mp4",
            "video/mp4/x",
            "vid eo/mp4",
            r#"video/mp4; codecs="avc1.42E01E"#,
            "video/mp4; codecs",
            "video/mp4; =x",
            "video/mp4; codecs=",
            r#"video/mp4; codecs="a" b"#,
            r#"video/mp4; codecs="avc1.42E01E,""#,
            r#"video/mp4; codecs="avc1.42E01E,,mp4a.40.2""#,
        ] {
            assert!(
                matches!(parse_type(bad), Err(MseError::Type(_))),
                "{bad:?} is a syntax error: {:?}",
                parse_type(bad)
            );
        }
    }

    #[test]
    fn codec_strings_are_validated() {
        for good in [
            "avc1.42E01E",
            "AVC1.42e01e",
            "avc3.640028",
            "vp09.00.10.08",
            "vp09.02.10.10.01.09.16.09.01",
            "vp09",
            "av01.0.05M.08",
            "av01.2.08H.12.0.110.09.16.09.0",
            "mp4a.40.2",
            "mp4a.40.5",
            "mp4a.40.29",
            "opus",
        ] {
            let mime = format!(r#"video/mp4; codecs="{good}""#);
            let r = parse_type(&mime);
            // vp09 and av01 and opus are fine in mp4 too; avc and aac are mp4 only.
            assert!(r.is_ok(), "{good}: {r:?}");
        }
        for bad in [
            "avc1",
            "avc1.42E01",
            "avc1.42E01G",
            "avc1.42E01E00",
            "vp09.00.10",
            "vp09.4.10.08",
            "vp09.00.10.09",
            "vp09.00.1x.08",
            "av01",
            "av01.0.05X.08",
            "av01.3.05M.08",
            "av01.0.32M.08",
            "av01.0.05M.09",
            "mp4a",
            "mp4a.40",
            "mp4a.40.x",
            "mp4a.40.34",
            "mp4a.40.+2",
            "opus.1",
            "vp9.1",
            "foo.bar",
        ] {
            let mime = format!(r#"video/mp4; codecs="{bad}""#);
            assert!(
                matches!(parse_type(&mime), Err(MseError::NotSupported(_))),
                "{bad}: {:?}",
                parse_type(&mime)
            );
        }
    }

    #[test]
    fn what_decision_d4_leaves_out_says_so() {
        for (codec, word) in [
            ("hvc1.1.6.L93.B0", "HEVC"),
            ("hev1.1.6.L93.B0", "HEVC"),
            ("mp4a.69", "MP3"),
            ("mp4a.6B", "MP3"),
            ("mp3", "MP3"),
            ("flac", "FLAC"),
            ("vorbis", "Vorbis"),
            ("ac-3", "AC-3"),
            ("ec-3", "E-AC-3"),
        ] {
            let mime = format!(r#"audio/mp4; codecs="{codec}""#);
            match parse_type(&mime) {
                Err(MseError::NotSupported(m)) => {
                    assert!(m.contains(word) && m.contains("D4"), "{codec}: {m}")
                }
                other => panic!("{codec}: {other:?}"),
            }
        }
    }

    #[test]
    fn combinations_are_checked() {
        for bad in [
            r#"video/mp4; codecs="avc1.42E01E, avc1.64001F""#,
            r#"video/mp4; codecs="avc1.42E01E, vp09.00.10.08""#,
            r#"video/mp4; codecs="mp4a.40.2, opus""#,
            r#"audio/mp4; codecs="avc1.42E01E""#,
            r#"audio/webm; codecs="vp9""#,
            r#"video/webm; codecs="avc1.42E01E""#,
            r#"video/webm; codecs="mp4a.40.2""#,
            r#"video/mp4; codecs="vp8""#,
            r#"video/mp4; codecs="""#,
            r#"video/mp4; codecs=" ""#,
            "video/foo",
            "audio/mpeg",
            "audio/aac",
            "video/mp2t",
            "application/octet-stream",
            "text/plain",
            "application/mp4",
        ] {
            assert!(
                matches!(parse_type(bad), Err(MseError::NotSupported(_))),
                "{bad:?}: {:?}",
                parse_type(bad)
            );
        }
        let ok = ok(r#"video/webm; codecs="vp9, opus""#);
        assert_eq!(ok.kinds, vec![TrackKind::Video, TrackKind::Audio]);
    }

    #[test]
    fn is_type_supported_asks_the_platform_for_every_codec() {
        struct OnlyVideo;
        impl TypeSupport for OnlyVideo {
            fn has_decoder(&self, kind: TrackKind, _codec: CodecId) -> bool {
                kind == TrackKind::Video
            }
        }
        let both = r#"video/mp4; codecs="avc1.42E01E, mp4a.40.2""#;
        assert!(is_type_supported(both, &AllDecoders));
        assert!(!is_type_supported(both, &OnlyVideo));
        assert!(is_type_supported(
            r#"video/mp4; codecs="avc1.42E01E""#,
            &OnlyVideo
        ));
        assert!(
            is_type_supported("video/mp4", &OnlyVideo),
            "no codecs, nothing to ask"
        );
        assert!(!is_type_supported("", &AllDecoders));
        assert!(!is_type_supported("video/foo", &AllDecoders));
    }
}
