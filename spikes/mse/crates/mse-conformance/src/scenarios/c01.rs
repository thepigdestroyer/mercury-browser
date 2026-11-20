// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C01 (P0): the type-support table. `is_type_supported` for every type string a player might ask
//! (H.264, HEVC, AV1, VP8/VP9, AAC, MP3, AC-3, E-AC-3, Opus, FLAC, Vorbis; MP4, WebM, MPEG audio, ADTS,
//! MPEG-TS; muxed pairs; no codecs at all; bogus strings), the registry's decoders and demuxers next to
//! each answer, and `add_source_buffer` with unsupported types. The whole table goes into the metric
//! `type_table`: it feeds EA2 (the codec strings `canPlayType`/`isTypeSupported` must answer).
//!
//! Pass when every fixture MIME is supported, the strings in the table that a player needs and the fixtures
//! cover are supported, every bogus string is refused (by `is_type_supported` and by `add_source_buffer`,
//! with `NotSupported` or `Type`). Everything else in the table is recorded, with two kinds of note:
//! "yes but no decoder in the registry" and "no where a player would ask".

use std::str::FromStr;

use mse_rig::backend::{MseBackend, MseError};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use serde_json::json;

use crate::support::{Abort, Res, Scn, fast_config, new_rig, run_scenario, shutdown};

pub const TITLE: &str = "type support table: is_type_supported, decoders in the registry, add_source_buffer with bad types";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C01", TITLE, false, body)
}

/// What the scenario requires of one answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// A player needs this and the fixtures cover it: `false` is a failure.
    Yes,
    /// Bogus: `true` is a failure.
    No,
    /// Recorded only (feeds EA2).
    Info,
}

impl Expect {
    fn name(self) -> &'static str {
        match self {
            Expect::Yes => "yes",
            Expect::No => "no",
            Expect::Info => "info",
        }
    }
}

pub struct Case {
    pub mime: &'static str,
    pub expect: Expect,
    /// Who asks about this string in practice ("" when only the spec or nobody does).
    pub asked_by: &'static str,
}

const fn case(mime: &'static str, expect: Expect, asked_by: &'static str) -> Case {
    Case {
        mime,
        expect,
        asked_by,
    }
}

/// The table. `Expect::Yes` is limited to what the fixtures prove and the minimum a player needs;
/// everything else is `Info`, so that a "no" for HEVC or E-AC-3 is data and not a verdict.
pub const CASES: &[Case] = &[
    // H.264 in MP4 (YouTube, every DASH/HLS player)
    case(
        r#"video/mp4; codecs="avc1.42E01E""#,
        Expect::Yes,
        "youtube, dash.js, shaka, hls.js",
    ),
    case(
        r#"video/mp4; codecs="avc1.42C01E""#,
        Expect::Yes,
        "dash.js, shaka",
    ),
    case(
        r#"video/mp4; codecs="avc1.4D401F""#,
        Expect::Yes,
        "youtube, dash.js, shaka, hls.js",
    ),
    case(
        r#"video/mp4; codecs="avc1.64001F""#,
        Expect::Yes,
        "youtube, dash.js, shaka, hls.js",
    ),
    case(
        r#"video/mp4; codecs="avc1.640028""#,
        Expect::Yes,
        "youtube, dash.js, shaka, hls.js",
    ),
    case(
        r#"video/mp4; codecs="avc3.640028""#,
        Expect::Info,
        "dash.js, shaka",
    ),
    case(
        r#"video/mp4; codecs="avc1.4D4028""#,
        Expect::Info,
        "youtube",
    ),
    case(
        r#"video/mp4; codecs="avc1.640033""#,
        Expect::Info,
        "youtube (level 5.1)",
    ),
    // HEVC
    case(
        r#"video/mp4; codecs="hev1.1.6.L93.B0""#,
        Expect::Info,
        "youtube, shaka, hls.js",
    ),
    case(
        r#"video/mp4; codecs="hvc1.1.6.L93.B0""#,
        Expect::Info,
        "shaka, hls.js",
    ),
    case(
        r#"video/mp4; codecs="hvc1.2.4.L153.B0""#,
        Expect::Info,
        "youtube (HDR)",
    ),
    // AV1
    case(
        r#"video/mp4; codecs="av01.0.05M.08""#,
        Expect::Info,
        "youtube, dash.js, shaka",
    ),
    case(
        r#"video/mp4; codecs="av01.0.08M.10""#,
        Expect::Info,
        "youtube (HDR)",
    ),
    case(
        r#"video/webm; codecs="av01.0.05M.08""#,
        Expect::Info,
        "youtube",
    ),
    case(
        r#"video/webm; codecs="av01.0.08M.10""#,
        Expect::Info,
        "youtube (HDR)",
    ),
    // VP8 / VP9
    case(
        r#"video/webm; codecs="vp09.00.10.08""#,
        Expect::Yes,
        "youtube, dash.js, shaka",
    ),
    case(
        r#"video/webm; codecs="vp09.00.51.08""#,
        Expect::Info,
        "youtube (level 5.1)",
    ),
    case(
        r#"video/webm; codecs="vp09.02.10.10""#,
        Expect::Info,
        "youtube (HDR, profile 2)",
    ),
    case(r#"video/webm; codecs="vp9""#, Expect::Info, "older players"),
    case(r#"video/webm; codecs="vp8""#, Expect::Info, "older players"),
    case(
        r#"video/mp4; codecs="vp09.00.10.08""#,
        Expect::Info,
        "shaka",
    ),
    // audio in MP4
    case(
        r#"audio/mp4; codecs="mp4a.40.2""#,
        Expect::Yes,
        "youtube, dash.js, shaka, hls.js",
    ),
    case(
        r#"audio/mp4; codecs="mp4a.40.5""#,
        Expect::Info,
        "dash.js, shaka (HE-AAC)",
    ),
    case(
        r#"audio/mp4; codecs="mp4a.40.29""#,
        Expect::Info,
        "hls.js (HE-AAC v2)",
    ),
    case(
        r#"audio/mp4; codecs="mp4a.69""#,
        Expect::Info,
        "hls.js (MP3 in MP4)",
    ),
    case(
        r#"audio/mp4; codecs="mp4a.6B""#,
        Expect::Info,
        "hls.js (MP3 in MP4)",
    ),
    case(r#"audio/mp4; codecs="ac-3""#, Expect::Info, "shaka, hls.js"),
    case(r#"audio/mp4; codecs="ec-3""#, Expect::Info, "shaka, hls.js"),
    case(r#"audio/mp4; codecs="opus""#, Expect::Info, "shaka, hls.js"),
    case(r#"audio/mp4; codecs="flac""#, Expect::Info, "shaka"),
    case(r#"audio/mp4; codecs="mp3""#, Expect::Info, "hls.js"),
    // audio in WebM
    case(
        r#"audio/webm; codecs="opus""#,
        Expect::Yes,
        "youtube, dash.js, shaka",
    ),
    case(
        r#"audio/webm; codecs="vorbis""#,
        Expect::Info,
        "older players",
    ),
    // elementary and transport streams
    case(r#"audio/mpeg"#, Expect::Info, "hls.js"),
    case(r#"audio/mpeg; codecs="mp3""#, Expect::Info, "hls.js"),
    case(r#"audio/aac"#, Expect::Info, "hls.js (ADTS)"),
    case(
        r#"video/mp2t; codecs="avc1.42E01E,mp4a.40.2""#,
        Expect::Info,
        "hls.js (older versions)",
    ),
    case(r#"video/mp2t"#, Expect::Info, "hls.js (older versions)"),
    // muxed pairs
    case(
        r#"video/mp4; codecs="avc1.42E01E, mp4a.40.2""#,
        Expect::Yes,
        "hls.js, dash.js (muxed)",
    ),
    case(
        r#"video/mp4; codecs="avc1.64001F,mp4a.40.2""#,
        Expect::Info,
        "hls.js (no space after the comma)",
    ),
    case(
        r#"video/mp4; codecs="mp4a.40.2, avc1.42E01E""#,
        Expect::Info,
        "any order",
    ),
    case(
        r#"video/webm; codecs="vp09.00.10.08, opus""#,
        Expect::Info,
        "muxed WebM",
    ),
    case(
        r#"video/webm; codecs="vp8, vorbis""#,
        Expect::Info,
        "older muxed WebM",
    ),
    // no codecs parameter
    case(r#"video/mp4"#, Expect::Info, "sites that omit codecs"),
    case(r#"audio/mp4"#, Expect::Info, "sites that omit codecs"),
    case(r#"video/webm"#, Expect::Info, "sites that omit codecs"),
    case(r#"audio/webm"#, Expect::Info, "sites that omit codecs"),
    // syntax variants
    case(
        r#"VIDEO/MP4; codecs="avc1.42E01E""#,
        Expect::Info,
        "MIME types are case-insensitive",
    ),
    case(
        r#"video/mp4; codecs=avc1.42E01E"#,
        Expect::Info,
        "unquoted codecs value",
    ),
    case(
        r#"video/mp4; codecs="avc1.42E01E"; foo=bar"#,
        Expect::Info,
        "extra parameter",
    ),
    case(
        r#"video/webm; codecs="avc1.42E01E""#,
        Expect::Info,
        "H.264 in WebM",
    ),
    case(
        r#"audio/mp4; codecs="avc1.42E01E""#,
        Expect::Info,
        "video codec in an audio type",
    ),
    // bogus
    case(r#"video/foo"#, Expect::No, ""),
    case(r#"video/mp4; codecs="foo.bar""#, Expect::No, ""),
    case(r#""#, Expect::No, ""),
    case(
        r#"video/mp4; codecs="avc1.42E01E, avc1.64001F""#,
        Expect::No,
        "two video codecs",
    ),
    case(r#"video/mp4; codecs=""#, Expect::No, ""),
    case(r#"application/octet-stream"#, Expect::No, ""),
    case(r#"text/plain"#, Expect::No, ""),
    case(r#"video/mp4; codecs="""#, Expect::Info, "empty codecs list"),
];

/// The codec names of a type string's `codecs` parameter.
pub(crate) fn parse_codecs(mime: &str) -> Vec<String> {
    let lower = mime.to_ascii_lowercase();
    let Some(at) = lower.find("codecs=") else {
        return Vec::new();
    };
    let rest = &mime[at + "codecs=".len()..];
    let value = if let Some(stripped) = rest.strip_prefix('"') {
        stripped.split('"').next().unwrap_or("")
    } else {
        rest.split(';').next().unwrap_or("")
    };
    value
        .split(',')
        .map(|codec| codec.trim().to_owned())
        .filter(|codec| !codec.is_empty())
        .collect()
}

/// The `type/subtype` of a type string, lower-cased.
pub fn container_type(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// The GStreamer caps a decoder must accept for `codec` (an RFC 6381 string), when it is one we know.
pub(crate) fn codec_caps(codec: &str) -> Option<&'static str> {
    let lower = codec.to_ascii_lowercase();
    let lower = lower.as_str();
    Some(match lower {
        c if c.starts_with("avc1") || c.starts_with("avc3") => "video/x-h264",
        c if c.starts_with("hev1") || c.starts_with("hvc1") => "video/x-h265",
        c if c.starts_with("av01") => "video/x-av1",
        c if c.starts_with("vp09") || c == "vp9" => "video/x-vp9",
        c if c.starts_with("vp08") || c == "vp8" => "video/x-vp8",
        "mp4a.69" | "mp4a.6b" | "mp3" => "audio/mpeg, mpegversion=(int)1, layer=(int)3",
        c if c.starts_with("mp4a.40") => "audio/mpeg, mpegversion=(int)4",
        "ac-3" => "audio/x-ac3",
        "ec-3" => "audio/x-eac3",
        "opus" => "audio/x-opus",
        "flac" => "audio/x-flac",
        "vorbis" => "audio/x-vorbis",
        _ => return None,
    })
}

/// The element factories that must be there to demux or parse a container type.
pub fn container_elements(container: &str) -> &'static [&'static str] {
    match container {
        "video/mp4" | "audio/mp4" => &["qtdemux"],
        "video/webm" | "audio/webm" => &["matroskademux"],
        "audio/mpeg" => &["mpegaudioparse"],
        "audio/aac" => &["aacparse"],
        "video/mp2t" => &["tsdemux"],
        _ => &[],
    }
}

/// Decoders in the registry that accept `caps`, best rank first, as `name:rank`. Rank 0 (NONE) factories are
/// left out: they are never auto-plugged (the generic `decodebin` bins are among them).
fn decoders_for(caps: &str) -> Vec<String> {
    let Ok(caps) = gstreamer::Caps::from_str(caps) else {
        return Vec::new();
    };
    use gstreamer::prelude::*;
    let mut found: Vec<(i32, String)> = gstreamer::ElementFactory::factories_with_type(
        gstreamer::ElementFactoryType::DECODER,
        gstreamer::Rank::MARGINAL,
    )
    .into_iter()
    .filter(|factory| factory.can_sink_any_caps(&caps))
    .map(|factory| (i32::from(factory.rank()), factory.name().to_string()))
    .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found
        .into_iter()
        .map(|(rank, name)| format!("{name}:{rank}"))
        .collect()
}

fn element_exists(name: &str) -> bool {
    gstreamer::ElementFactory::find(name).is_some()
}

/// The registry facts that go next to one answer.
struct Facts {
    /// One entry per codec: `codec -> decoders`.
    decoders: Vec<(String, Vec<String>)>,
    unknown_codecs: Vec<String>,
    container_elements: Vec<(String, bool)>,
}

fn facts_for(mime: &str) -> Facts {
    let mut decoders = Vec::new();
    let mut unknown = Vec::new();
    for codec in parse_codecs(mime) {
        match codec_caps(&codec) {
            Some(caps) => decoders.push((codec, decoders_for(caps))),
            None => unknown.push(codec),
        }
    }
    let container_elements = container_elements(&container_type(mime))
        .iter()
        .map(|name| ((*name).to_owned(), element_exists(name)))
        .collect();
    Facts {
        decoders,
        unknown_codecs: unknown,
        container_elements,
    }
}

impl Facts {
    /// The codecs of the string for which the registry has no decoder, and demuxers that are missing.
    fn missing(&self) -> Vec<String> {
        let mut missing: Vec<String> = self
            .decoders
            .iter()
            .filter(|(_, found)| found.is_empty())
            .map(|(codec, _)| format!("no decoder for {codec}"))
            .collect();
        missing.extend(
            self.container_elements
                .iter()
                .filter(|(_, present)| !present)
                .map(|(name, _)| format!("no {name}")),
        );
        missing
    }
}

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let mut table = Vec::new();
    let mut must_support = Vec::new();
    let mut must_reject = Vec::new();
    let mut no_but_decoder = Vec::new();
    let mut yes_without_decoder = Vec::new();
    let mut no_where_asked = Vec::new();
    let mut supported_count = 0usize;

    for case in CASES {
        let supported = MseBackend::is_type_supported(case.mime);
        if supported {
            supported_count += 1;
        }
        let facts = facts_for(case.mime);
        let missing = facts.missing();
        match case.expect {
            Expect::Yes if !supported => must_support.push(format!("{:?}", case.mime)),
            Expect::No if supported => must_reject.push(format!("{:?}", case.mime)),
            _ => {}
        }
        if !supported
            && case.expect != Expect::No
            && facts.missing().is_empty()
            && !facts.decoders.is_empty()
        {
            no_but_decoder.push(format!(
                "{:?}: decoders exist in the registry ({})",
                case.mime,
                facts
                    .decoders
                    .iter()
                    .map(|(codec, found)| format!("{codec}: {}", found.join("/")))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        if supported && !missing.is_empty() && case.expect != Expect::No {
            yes_without_decoder.push(format!("{:?}: {}", case.mime, missing.join(", ")));
        }
        if !supported && !case.asked_by.is_empty() && case.expect != Expect::No {
            no_where_asked.push(format!("{:?} (asked by {})", case.mime, case.asked_by));
        }
        table.push(json!({
            "mime": case.mime,
            "supported": supported,
            "expect": case.expect.name(),
            "asked_by": case.asked_by,
            "decoders": facts.decoders.iter()
                .map(|(codec, found)| json!({"codec": codec, "decoders": found}))
                .collect::<Vec<_>>(),
            "codecs_not_mapped": facts.unknown_codecs,
            "container_elements": facts.container_elements.iter()
                .map(|(name, present)| json!({"element": name, "present": present}))
                .collect::<Vec<_>>(),
        }));
    }
    scn.metric("types_total", CASES.len())
        .metric("types_supported", supported_count)
        .metric("types_unsupported", CASES.len() - supported_count)
        .metric("type_table", table)
        .metric("yes_without_decoder", yes_without_decoder.clone())
        .metric("no_where_players_ask", no_where_asked.clone())
        .metric("no_but_decoder_exists", no_but_decoder.clone());
    for line in &no_but_decoder {
        scn.note(format!("NO although the registry has a decoder: {line}"));
    }
    for line in &yes_without_decoder {
        scn.note(format!(
            "YES but nothing in the registry can play it: {line}"
        ));
    }
    for line in &no_where_asked {
        scn.note(format!("NO where a player would ask: {line}"));
    }
    scn.note(format!(
        "{supported_count} of {} type strings are supported",
        CASES.len()
    ));
    scn.check(
        "required-types-supported",
        must_support.is_empty(),
        format!(
            "GstMSE says no to types a player needs and the fixtures cover: {}",
            must_support.join("; ")
        ),
    );
    scn.check(
        "bogus-types-rejected",
        must_reject.is_empty(),
        format!(
            "GstMSE says yes to types that are not media types it can play (the DOM must validate type strings itself): {}",
            must_reject.join("; ")
        ),
    );

    fixture_mimes(scn)?;

    if ctx.info.msesrc_available {
        add_source_buffer_with_bad_types(ctx, scn)?;
    } else {
        scn.note(
            "add_source_buffer with bad types was not run: msesrc is not loaded in this plugin mode",
        );
    }
    Ok(())
}

/// Every fixture's MIME, as written in `FIXTURES` and as derived from its init segment, is supported.
fn fixture_mimes(scn: &mut Scn) -> Res<()> {
    let mut refused = Vec::new();
    let mut checked = 0usize;
    for fixture in mse_segments::FIXTURES {
        checked += 1;
        if !MseBackend::is_type_supported(fixture.mime) {
            refused.push(format!(
                "{} ({:?}, from FIXTURES)",
                fixture.name, fixture.mime
            ));
        }
        match mse_segments::load(fixture.name) {
            Ok(stream) => {
                checked += 1;
                if !MseBackend::is_type_supported(&stream.mime) {
                    refused.push(format!(
                        "{} ({:?}, derived from its init segment)",
                        fixture.name, stream.mime
                    ));
                }
                if stream.mime != fixture.mime {
                    scn.note(format!(
                        "fixture {}: FIXTURES says {:?}, the init segment gives {:?}",
                        fixture.name, fixture.mime, stream.mime
                    ));
                }
            }
            Err(error) => {
                return Err(Abort::Error(format!(
                    "fixture {:?} does not load: {error}",
                    fixture.name
                )));
            }
        }
    }
    scn.metric("fixture_mimes_checked", checked);
    if mse_segments::FIXTURES.is_empty() {
        return Err(Abort::Error(
            "mse_segments::FIXTURES is empty: the fixtures are not in place".to_owned(),
        ));
    }
    scn.check(
        "fixture-mimes-supported",
        refused.is_empty(),
        format!("GstMSE refuses fixture types: {}", refused.join("; ")),
    );
    Ok(())
}

/// The class `addSourceBuffer` should throw for a string, from the spec: an empty type is a `TypeError`,
/// an unsupported one a `NotSupportedError`.
fn spec_class(mime: &str) -> &'static str {
    if mime.is_empty() {
        "Type"
    } else {
        "NotSupported"
    }
}

fn class_name(error: &MseError) -> &'static str {
    match error {
        MseError::InvalidState(_) => "InvalidState",
        MseError::Type(_) => "Type",
        MseError::NotSupported(_) => "NotSupported",
        MseError::NotFound(_) => "NotFound",
        MseError::QuotaExceeded(_) => "QuotaExceeded",
        MseError::Other(_) => "Other",
    }
}

fn add_source_buffer_with_bad_types(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let rig = new_rig(scn, fast_config(ctx))?;
    let bad = [
        "video/foo",
        "",
        r#"video/mp4; codecs="foo.bar""#,
        r#"video/mp4; codecs="avc1.42E01E, avc1.64001F""#,
        "application/octet-stream",
    ];
    let mut problems = Vec::new();
    let mut classes = Vec::new();
    for mime in bad {
        match rig.backend.add_source_buffer(mime) {
            Ok(id) => problems.push(format!(
                "add_source_buffer({mime:?}) succeeded (source buffer {id}); the spec throws"
            )),
            Err(error @ (MseError::NotSupported(_) | MseError::Type(_))) => {
                let class = class_name(&error);
                classes.push(format!("{mime:?} -> {class}: {error:?}"));
                if class != spec_class(mime) {
                    scn.note(format!(
                        "API-FIT: add_source_buffer({mime:?}) is refused with {class}; the spec throws {}",
                        spec_class(mime)
                    ));
                }
            }
            Err(other) => problems.push(format!(
                "add_source_buffer({mime:?}) was refused with {} ({other:?}), not NotSupported or Type",
                class_name(&other)
            )),
        }
    }
    scn.metric("bad_type_refusals", classes);
    let count = rig.backend.source_buffer_count();
    scn.check(
        "add-source-buffer-refuses-bad-types",
        problems.is_empty(),
        problems.join("; "),
    );
    scn.check(
        "no-source-buffer-left-behind",
        count == 0,
        format!("{count} source buffer(s) exist after only refused add_source_buffer calls"),
    );
    shutdown(scn, "c01", rig);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_table_has_at_least_forty_unique_strings_and_all_three_expectations() {
        assert!(CASES.len() >= 40, "{} cases", CASES.len());
        let unique: BTreeSet<&str> = CASES.iter().map(|c| c.mime).collect();
        assert_eq!(
            unique.len(),
            CASES.len(),
            "duplicate type string in the table"
        );
        for expect in [Expect::Yes, Expect::No, Expect::Info] {
            assert!(CASES.iter().any(|c| c.expect == expect));
        }
        // The bogus entries must be bogus on their own, not merely unusual.
        for case in CASES.iter().filter(|c| c.expect == Expect::No) {
            assert!(
                case.mime.is_empty()
                    || case.mime.starts_with("video/foo")
                    || case.mime.contains("foo.bar")
                    || case.mime.contains("avc1.42E01E, avc1.64001F")
                    || case.mime == r#"video/mp4; codecs=""#
                    || case.mime.starts_with("application/")
                    || case.mime.starts_with("text/"),
                "{:?} is listed as bogus",
                case.mime
            );
        }
    }

    #[test]
    fn codecs_are_parsed_from_quoted_unquoted_and_missing_values() {
        assert_eq!(
            parse_codecs(r#"video/mp4; codecs="avc1.42E01E, mp4a.40.2""#),
            vec!["avc1.42E01E", "mp4a.40.2"]
        );
        assert_eq!(
            parse_codecs(r#"video/mp4; codecs="avc1.64001F,mp4a.40.2""#),
            vec!["avc1.64001F", "mp4a.40.2"]
        );
        assert_eq!(
            parse_codecs("video/mp4; codecs=avc1.42E01E; foo=bar"),
            vec!["avc1.42E01E"]
        );
        assert_eq!(parse_codecs(r#"VIDEO/MP4; CODECS="opus""#), vec!["opus"]);
        assert!(parse_codecs("video/mp4").is_empty());
        assert!(parse_codecs(r#"video/mp4; codecs="""#).is_empty());
        assert!(parse_codecs("").is_empty());
        assert_eq!(container_type(r#"Video/MP4; codecs="x""#), "video/mp4");
        assert_eq!(container_type(""), "");
    }

    #[test]
    fn every_codec_in_the_table_maps_to_caps_or_is_deliberately_unknown() {
        let mut unknown = BTreeSet::new();
        for case in CASES {
            for codec in parse_codecs(case.mime) {
                if codec_caps(&codec).is_none() {
                    unknown.insert(codec);
                }
            }
        }
        // Only the bogus codec names have no caps.
        assert_eq!(
            unknown.into_iter().collect::<Vec<_>>(),
            vec!["foo.bar".to_owned()]
        );
        assert_eq!(codec_caps("avc3.640028"), Some("video/x-h264"));
        assert_eq!(
            codec_caps("mp4a.40.5"),
            Some("audio/mpeg, mpegversion=(int)4")
        );
        assert_eq!(
            codec_caps("mp4a.6B"),
            Some("audio/mpeg, mpegversion=(int)1, layer=(int)3")
        );
        assert_eq!(codec_caps("vp09.02.10.10"), Some("video/x-vp9"));
        assert_eq!(codec_caps("nonsense"), None);
    }

    #[test]
    fn containers_have_their_demuxers_named() {
        assert_eq!(container_elements("video/mp4"), &["qtdemux"]);
        assert_eq!(container_elements("audio/webm"), &["matroskademux"]);
        assert!(container_elements("text/plain").is_empty());
        assert_eq!(spec_class(""), "Type");
        assert_eq!(spec_class("video/foo"), "NotSupported");
    }
}
