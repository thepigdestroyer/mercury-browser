// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The generated streams of the MSE player pages: a static DASH MPD and an HLS CMAF playlist
//! set built from the committed fixtures of `crates/mercury-mse-testkit` and its splitter, so
//! that dash.js, Shaka Player and hls.js have something real to play without a copy of any
//! segment in the repository (the MED-1b brief, D11).
//!
//! - `/mse-dash/<set>/manifest.mpd`, with `<rendition>-init.<ext>` and `<rendition>-<n>.<ext>`
//!   beside it (numbered segments, `n` from 1);
//! - `/mse-hls/<set>/master.m3u8`, `<rendition>.m3u8` for each rendition, with
//!   `<rendition>-init.mp4` and `<rendition>-<n>.m4s` (fMP4, `#EXT-X-MAP`).
//!
//! A rendition is a fixture's name (`h264-low`, `aac`, ...); its init segment and media
//! segments are the splitter's, byte for byte, so the init segment followed by the segments in
//! order is the fixture's own file.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mercury_mse_testkit::{Container, MediaKind, SegmentedStream, fixture, load};

/// One generated presentation: video renditions of one codec and one audio rendition.
pub struct SetDef {
    pub name: &'static str,
    pub video: &'static [&'static str],
    pub audio: &'static str,
}

/// The DASH sets: H.264 and AAC in fMP4, and VP9 and Opus in WebM (for Shaka).
pub const DASH_SETS: &[SetDef] = &[
    SetDef {
        name: "h264",
        video: &["h264-low", "h264-high"],
        audio: "aac",
    },
    SetDef {
        name: "vp9",
        video: &["vp9-low", "vp9-high"],
        audio: "opus",
    },
];

/// The HLS sets: CMAF is fMP4, so H.264 and AAC only.
pub const HLS_SETS: &[SetDef] = &[SetDef {
    name: "h264",
    video: &["h264-low", "h264-high"],
    audio: "aac",
}];

/// What the playlists and the MPD say about one rendition.
#[derive(Debug, Clone)]
pub struct Rendition {
    /// The fixture name, also the rendition's id.
    pub id: String,
    pub is_video: bool,
    /// `video/mp4`, `audio/webm`, ...
    pub mime_type: String,
    /// The `codecs` parameter of the init segment's type string.
    pub codecs: String,
    pub width: u32,
    pub height: u32,
    /// Bits per second, from the bytes and the duration.
    pub bandwidth: u64,
    /// `mp4` or `webm`: the init segment's extension.
    pub init_ext: &'static str,
    /// The extension of a media segment: `m4s` for fMP4, `webm` for WebM.
    pub segment_ext: &'static str,
    /// Start and duration of each media segment in milliseconds.
    pub segments: Vec<(u64, u64)>,
}

impl Rendition {
    pub fn init_name(&self) -> String {
        format!("{}-init.{}", self.id, self.init_ext)
    }

    pub fn segment_name(&self, number: usize) -> String {
        format!("{}-{number}.{}", self.id, self.segment_ext)
    }

    pub fn duration_ms(&self) -> u64 {
        self.segments.iter().map(|(_, duration)| duration).sum()
    }
}

fn split_mime(mime: &str) -> (String, String) {
    let (kind, rest) = mime.split_once(';').unwrap_or((mime, ""));
    let codecs = rest
        .split_once("codecs=\"")
        .and_then(|(_, tail)| tail.split_once('"'))
        .map(|(codecs, _)| codecs.replace(' ', ""))
        .unwrap_or_default();
    (kind.trim().to_owned(), codecs)
}

/// Describes the fixture `name` as a rendition of a presentation.
pub fn describe(name: &str, stream: &SegmentedStream) -> Option<Rendition> {
    let fixture = fixture(name)?;
    let (mime_type, codecs) = split_mime(&stream.mime);
    let segments: Vec<(u64, u64)> = stream
        .media
        .iter()
        .map(|segment| {
            (
                segment.start_ms.unwrap_or(0),
                segment.duration_ms.unwrap_or(0),
            )
        })
        .collect();
    let total: u64 = segments.iter().map(|(_, duration)| duration).sum();
    let bytes = stream.init.bytes.len() + stream.media.iter().map(|s| s.bytes.len()).sum::<usize>();
    let bandwidth = if total == 0 {
        0
    } else {
        (bytes as u64 * 8 * 1000).div_ceil(total)
    };
    let (init_ext, segment_ext) = match stream.container {
        Container::Mp4 => ("mp4", "m4s"),
        Container::WebM => ("webm", "webm"),
    };
    Some(Rendition {
        id: name.to_owned(),
        is_video: !matches!(fixture.kind, MediaKind::Audio),
        mime_type,
        codecs,
        width: fixture.width,
        height: fixture.height,
        bandwidth: bandwidth.max(1),
        init_ext,
        segment_ext,
        segments,
    })
}

fn seconds(ms: u64) -> String {
    let whole = ms / 1000;
    let fraction = ms % 1000;
    if fraction == 0 {
        format!("{whole}")
    } else {
        format!("{whole}.{fraction:03}")
            .trim_end_matches('0')
            .to_owned()
    }
}

/// An ISO 8601 duration for the MPD: `PT8S`, `PT1.5S`.
fn iso_duration(ms: u64) -> String {
    format!("PT{}S", seconds(ms))
}

/// The `<S>` elements of a rendition's segment timeline: equal durations folded into `r`.
fn timeline(segments: &[(u64, u64)]) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < segments.len() {
        let (start, duration) = segments[index];
        let mut repeats = 0;
        while index + 1 + repeats < segments.len() && segments[index + 1 + repeats].1 == duration {
            repeats += 1;
        }
        let first = index == 0;
        out.push_str("          <S ");
        if first {
            out.push_str(&format!("t=\"{start}\" "));
        }
        out.push_str(&format!("d=\"{duration}\""));
        if repeats > 0 {
            out.push_str(&format!(" r=\"{repeats}\""));
        }
        out.push_str("/>\n");
        index += 1 + repeats;
    }
    out
}

fn representation(rendition: &Rendition) -> String {
    let mut out = format!(
        "      <Representation id=\"{}\" codecs=\"{}\" bandwidth=\"{}\"",
        rendition.id, rendition.codecs, rendition.bandwidth
    );
    if rendition.is_video {
        out.push_str(&format!(
            " width=\"{}\" height=\"{}\"",
            rendition.width, rendition.height
        ));
    }
    out.push_str(">\n");
    out.push_str(&format!(
        "        <SegmentTemplate timescale=\"1000\" startNumber=\"1\" initialization=\"$RepresentationID$-init.{}\" media=\"$RepresentationID$-$Number$.{}\">\n",
        rendition.init_ext, rendition.segment_ext
    ));
    out.push_str("        <SegmentTimeline>\n");
    out.push_str(&timeline(&rendition.segments));
    out.push_str(
        "        </SegmentTimeline>\n        </SegmentTemplate>\n      </Representation>\n",
    );
    out
}

fn adaptation_set(id: u32, kind: &str, renditions: &[Rendition]) -> String {
    let first = &renditions[0];
    let mut out = format!(
        "    <AdaptationSet id=\"{id}\" contentType=\"{kind}\" mimeType=\"{}\" segmentAlignment=\"true\" startWithSAP=\"1\"",
        first.mime_type
    );
    if kind == "video" {
        out.push_str(" subsegmentAlignment=\"true\"");
    } else {
        out.push_str(" lang=\"en\"");
    }
    out.push_str(">\n");
    for rendition in renditions {
        out.push_str(&representation(rendition));
    }
    out.push_str("    </AdaptationSet>\n");
    out
}

/// The static MPD of a set: one period, a video adaptation set with the video renditions and an
/// audio adaptation set.
pub fn mpd(video: &[Rendition], audio: &Rendition) -> String {
    let duration = video
        .iter()
        .map(Rendition::duration_ms)
        .chain([audio.duration_ms()])
        .max()
        .unwrap_or(0);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" profiles=\"urn:mpeg:dash:profile:isoff-live:2011\" minBufferTime=\"PT2S\" mediaPresentationDuration=\"{}\">\n",
        iso_duration(duration)
    ));
    out.push_str(&format!(
        "  <Period id=\"1\" start=\"PT0S\" duration=\"{}\">\n",
        iso_duration(duration)
    ));
    out.push_str(&adaptation_set(1, "video", video));
    out.push_str(&adaptation_set(2, "audio", std::slice::from_ref(audio)));
    out.push_str("  </Period>\n</MPD>\n");
    out
}

/// The HLS master playlist: the audio rendition as a group and a variant for each video rendition.
pub fn hls_master(video: &[Rendition], audio: &Rendition) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    out.push_str(&format!(
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"{}.m3u8\"\n",
        audio.id
    ));
    for rendition in video {
        out.push_str(&format!(
            "#EXT-X-STREAM-INF:BANDWIDTH={},AVERAGE-BANDWIDTH={},RESOLUTION={}x{},CODECS=\"{},{}\",AUDIO=\"aud\"\n{}.m3u8\n",
            rendition.bandwidth + audio.bandwidth,
            rendition.bandwidth + audio.bandwidth,
            rendition.width,
            rendition.height,
            rendition.codecs,
            audio.codecs,
            rendition.id
        ));
    }
    out
}

/// The media playlist of one rendition: a VOD list with the init segment as `#EXT-X-MAP`.
pub fn hls_media(rendition: &Rendition) -> String {
    let target = rendition
        .segments
        .iter()
        .map(|(_, duration)| duration.div_ceil(1000))
        .max()
        .unwrap_or(1);
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:1\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-MAP:URI=\"{}\"\n",
        rendition.init_name()
    );
    for (index, (_, duration)) in rendition.segments.iter().enumerate() {
        out.push_str(&format!(
            "#EXTINF:{}.{:03},\n{}\n",
            duration / 1000,
            duration % 1000,
            rendition.segment_name(index + 1)
        ));
    }
    out.push_str("#EXT-X-ENDLIST\n");
    out
}

/// What a generated URL answers.
pub struct Body {
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

/// Which family of generated streams a URL is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Dash,
    Hls,
}

/// The fixtures, split once and kept.
#[derive(Default)]
pub struct Streams {
    cache: Mutex<HashMap<String, Arc<SegmentedStream>>>,
}

impl Streams {
    fn stream(&self, name: &str) -> Option<Arc<SegmentedStream>> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(found) = cache.get(name) {
            return Some(Arc::clone(found));
        }
        let loaded = Arc::new(load(name).ok()?);
        cache.insert(name.to_owned(), Arc::clone(&loaded));
        Some(loaded)
    }

    fn renditions(&self, set: &SetDef) -> Option<(Vec<Rendition>, Rendition)> {
        let video = set
            .video
            .iter()
            .map(|name| {
                let stream = self.stream(name)?;
                describe(name, &stream)
            })
            .collect::<Option<Vec<_>>>()?;
        let audio_stream = self.stream(set.audio)?;
        let audio = describe(set.audio, &audio_stream)?;
        Some((video, audio))
    }

    /// The answer for `/mse-dash/<rest>` or `/mse-hls/<rest>`, where `rest` is
    /// `<set>/<file>`; `None` for anything that is not a generated file.
    pub fn answer(&self, family: Family, rest: &str) -> Option<Body> {
        let (set_name, file) = rest.split_once('/')?;
        let sets = match family {
            Family::Dash => DASH_SETS,
            Family::Hls => HLS_SETS,
        };
        let set = sets.iter().find(|set| set.name == set_name)?;
        let (video, audio) = self.renditions(set)?;
        match (family, file) {
            (Family::Dash, "manifest.mpd") => Some(Body {
                content_type: "application/dash+xml",
                bytes: mpd(&video, &audio).into_bytes(),
            }),
            (Family::Hls, "master.m3u8") => Some(Body {
                content_type: "application/vnd.apple.mpegurl",
                bytes: hls_master(&video, &audio).into_bytes(),
            }),
            _ => {
                let rendition = video.iter().chain([&audio]).find(|r| {
                    file.starts_with(&format!("{}.", r.id))
                        || file.starts_with(&format!("{}-", r.id))
                })?;
                self.rendition_file(family, rendition, file)
            }
        }
    }

    fn rendition_file(&self, family: Family, rendition: &Rendition, file: &str) -> Option<Body> {
        if family == Family::Hls && file == format!("{}.m3u8", rendition.id) {
            return Some(Body {
                content_type: "application/vnd.apple.mpegurl",
                bytes: hls_media(rendition).into_bytes(),
            });
        }
        let stream = self.stream(&rendition.id)?;
        let content_type = match (rendition.is_video, rendition.init_ext) {
            (true, "mp4") => "video/mp4",
            (false, "mp4") => "audio/mp4",
            (true, _) => "video/webm",
            (false, _) => "audio/webm",
        };
        if file == rendition.init_name() {
            return Some(Body {
                content_type,
                bytes: stream.init.bytes.clone(),
            });
        }
        let number = file
            .strip_prefix(&format!("{}-", rendition.id))?
            .strip_suffix(&format!(".{}", rendition.segment_ext))?;
        let number: usize = number.parse().ok().filter(|number| *number > 0)?;
        let segment = stream.media.get(number - 1)?;
        (file == rendition.segment_name(number)).then(|| Body {
            content_type,
            bytes: segment.bytes.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_written_without_noise() {
        assert_eq!(iso_duration(8000), "PT8S");
        assert_eq!(iso_duration(1500), "PT1.5S");
        assert_eq!(iso_duration(1250), "PT1.25S");
    }

    #[test]
    fn equal_durations_fold_into_a_repeat() {
        assert_eq!(
            timeline(&[(0, 2000), (2000, 2000), (4000, 2000), (6000, 1500)]),
            "          <S t=\"0\" d=\"2000\" r=\"2\"/>\n          <S d=\"1500\"/>\n"
        );
    }

    #[test]
    fn mime_strings_split_into_type_and_codecs() {
        assert_eq!(
            split_mime("video/mp4; codecs=\"avc1.42C00C, mp4a.40.2\""),
            ("video/mp4".to_owned(), "avc1.42C00C,mp4a.40.2".to_owned())
        );
        assert_eq!(
            split_mime("audio/mp4"),
            ("audio/mp4".to_owned(), String::new())
        );
    }

    #[test]
    fn unknown_names_answer_nothing() {
        let streams = Streams::default();
        for (family, rest) in [
            (Family::Dash, "nope/manifest.mpd"),
            (Family::Dash, "h264/other.mpd"),
            (Family::Dash, "h264"),
            (Family::Dash, "h264/h264-low-0.m4s"),
            (Family::Dash, "h264/h264-low-9.m4s"),
            (Family::Dash, "h264/h264-low-01.m4s"),
            (Family::Dash, "h264/h264-low-1.webm"),
            (Family::Dash, "h264/../h264/manifest.mpd"),
            (Family::Hls, "vp9/master.m3u8"),
            (Family::Hls, "h264/manifest.mpd"),
        ] {
            assert!(streams.answer(family, rest).is_none(), "{rest}");
        }
    }
}
