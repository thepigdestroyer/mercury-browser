// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The initialisation segment of a fragmented MP4: tracks, timescales, and the codec strings a page
//! would give `addSourceBuffer`, derived from the sample entries and never typed by hand.
//!
//! Codec string rules (RFC 6381 and the codec registrations): `avc1`/`avc3` + `avcC` give
//! `avc1.PPCCLL` (profile, constraint flags, level, as upper-case hex); `hvc1`/`hev1` + `hvcC` give
//! the dotted form of ISO/IEC 14496-15 Annex E; `mp4a` + `esds` give `mp4a.40.<audioObjectType>`
//! (or `mp4a.69` / `mp4a.6B` for MPEG-1/2 audio); `vp09` + `vpcC` give `vp09.PP.LL.DD`; `av01` + `av1C`
//! give `av01.P.LLT.DD`; `Opus`, `fLaC`, `ac-3`, `ec-3` give their registered names.

use crate::SplitError;
use crate::bmff::{BoxRef, boxes, find_child, find_path, parse_box, type_str};
use crate::bytes::{u8_at, u16_at, u32_at};

#[derive(Debug, Clone)]
pub(crate) struct TrackInfo {
    pub id: u32,
    pub timescale: u32,
    /// `vide`, `soun`, `text`, ...
    pub handler: [u8; 4],
    /// Four-character code of the first sample entry (`avc1`, `mp4a`, ...).
    pub entry: [u8; 4],
    pub codec: Option<String>,
    pub width: u32,
    pub height: u32,
    /// `trex` default sample duration, in the track's timescale (0 when there is no `trex`).
    pub default_duration: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct InitInfo {
    pub tracks: Vec<TrackInfo>,
    pub has_mvex: bool,
}

impl InitInfo {
    pub fn track(&self, id: u32) -> Option<&TrackInfo> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn is_video(t: &TrackInfo) -> bool {
        &t.handler == b"vide"
    }

    pub fn is_audio(t: &TrackInfo) -> bool {
        &t.handler == b"soun"
    }
}

/// Parses the `moov` box of an initialisation segment.
pub(crate) fn parse_init(init: &[u8]) -> Result<InitInfo, SplitError> {
    let mut moov = None;
    for b in boxes(init, 0, init.len()) {
        let b = b?;
        if b.is(b"moov") {
            moov = Some(b);
            break;
        }
    }
    let moov = moov.ok_or_else(|| {
        SplitError::Malformed("the initialisation segment has no moov box".into())
    })?;
    let mut tracks = Vec::new();
    let mut trex: Vec<(u32, u32)> = Vec::new();
    let mut has_mvex = false;
    for child in boxes(init, moov.payload, moov.end) {
        let child = child?;
        if child.is(b"trak") {
            tracks.push(parse_trak(init, &child)?);
        } else if child.is(b"mvex") {
            has_mvex = true;
            for t in boxes(init, child.payload, child.end) {
                let t = t?;
                if t.is(b"trex") {
                    let p = t.payload_of(init);
                    trex.push((u32_at(p, 4)?, u32_at(p, 12)?));
                }
            }
        }
    }
    for t in &mut tracks {
        if let Some((_, d)) = trex.iter().find(|(id, _)| *id == t.id) {
            t.default_duration = *d;
        }
    }
    if tracks.is_empty() {
        return Err(SplitError::Malformed("the moov box has no trak".into()));
    }
    Ok(InitInfo { tracks, has_mvex })
}

fn parse_trak(data: &[u8], trak: &BoxRef) -> Result<TrackInfo, SplitError> {
    let tkhd = find_child(data, trak, b"tkhd")?
        .ok_or_else(|| SplitError::Malformed("trak without tkhd".into()))?;
    let p = tkhd.payload_of(data);
    let id = if u8_at(p, 0)? == 1 {
        u32_at(p, 20)?
    } else {
        u32_at(p, 12)?
    };
    let mdhd = find_path(data, trak, &["mdia", "mdhd"])?
        .ok_or_else(|| SplitError::Malformed("trak without mdhd".into()))?;
    let p = mdhd.payload_of(data);
    let timescale = if u8_at(p, 0)? == 1 {
        u32_at(p, 20)?
    } else {
        u32_at(p, 12)?
    };
    if timescale == 0 {
        return Err(SplitError::Malformed("mdhd timescale is 0".into()));
    }
    let hdlr = find_path(data, trak, &["mdia", "hdlr"])?
        .ok_or_else(|| SplitError::Malformed("trak without hdlr".into()))?;
    let p = hdlr.payload_of(data);
    let handler = {
        let b = crate::bytes::slice(p, 8, 12)?;
        [b[0], b[1], b[2], b[3]]
    };
    let stsd = find_path(data, trak, &["mdia", "minf", "stbl", "stsd"])?
        .ok_or_else(|| SplitError::Malformed("trak without stsd".into()))?;
    // stsd: version/flags(4), entry_count(4), then the sample entries as boxes.
    let first = parse_box(data, stsd.payload + 8, stsd.end)?;
    let (width, height, codec) = sample_entry(data, &first, &handler)?;
    Ok(TrackInfo {
        id,
        timescale,
        handler,
        entry: first.typ,
        codec,
        width,
        height,
        default_duration: 0,
    })
}

/// Width, height (video only) and codec string of one sample entry.
fn sample_entry(
    data: &[u8],
    entry: &BoxRef,
    handler: &[u8; 4],
) -> Result<(u32, u32, Option<String>), SplitError> {
    let p = entry.payload_of(data);
    let (width, height, children_at) = if handler == b"vide" {
        // 6 reserved + 2 data_reference_index + 16 + width(2) height(2) + 50 more = 78 bytes.
        (u32::from(u16_at(p, 24)?), u32::from(u16_at(p, 26)?), 78)
    } else if handler == b"soun" {
        // 28 bytes; the QuickTime sound description versions 1 and 2 add 16 and 36.
        let extra = match u16_at(p, 8)? {
            1 => 16,
            2 => 36,
            _ => 0,
        };
        (0, 0, 28 + extra)
    } else {
        (0, 0, 8)
    };
    let child = |typ: &[u8; 4]| -> Result<Option<BoxRef>, SplitError> {
        // The walker works on the entry's whole range so that offsets stay relative to `data`.
        let from = entry.payload + children_at;
        if from >= entry.end {
            return Ok(None);
        }
        for b in boxes(data, from, entry.end) {
            let b = b?;
            if b.is(typ) {
                return Ok(Some(b));
            }
        }
        Ok(None)
    };
    let codec = match entry.typ.as_slice() {
        b"avc1" | b"avc3" => child(b"avcC")?
            .map(|b| avc_codec(&entry.typ, b.payload_of(data)))
            .transpose()?,
        b"hvc1" | b"hev1" => child(b"hvcC")?
            .map(|b| hevc_codec(&entry.typ, b.payload_of(data)))
            .transpose()?,
        b"vp09" => child(b"vpcC")?
            .map(|b| vp9_codec(b.payload_of(data)))
            .transpose()?,
        b"av01" => child(b"av1C")?
            .map(|b| av1_codec(b.payload_of(data)))
            .transpose()?,
        b"mp4a" => match child(b"esds")? {
            Some(b) => esds_codec(b.payload_of(data))?,
            None => None,
        },
        b"Opus" => Some("opus".to_string()),
        b"fLaC" => Some("flac".to_string()),
        b"ac-3" => Some("ac-3".to_string()),
        b"ec-3" => Some("ec-3".to_string()),
        _ => None,
    };
    Ok((width, height, codec))
}

fn avc_codec(entry: &[u8; 4], p: &[u8]) -> Result<String, SplitError> {
    // configurationVersion(1) AVCProfileIndication(1) profile_compatibility(1) AVCLevelIndication(1)
    Ok(format!(
        "{}.{:02X}{:02X}{:02X}",
        type_str(entry),
        u8_at(p, 1)?,
        u8_at(p, 2)?,
        u8_at(p, 3)?
    ))
}

pub(crate) fn hevc_codec(entry: &[u8; 4], p: &[u8]) -> Result<String, SplitError> {
    // configurationVersion(1); space(2) tier(1) profile_idc(5); compatibility flags(4);
    // constraint indicator flags(6); level_idc(1).
    let b1 = u8_at(p, 1)?;
    let space = match b1 >> 6 {
        1 => "A",
        2 => "B",
        3 => "C",
        _ => "",
    };
    let profile = b1 & 0x1f;
    let tier = if b1 & 0x20 != 0 { 'H' } else { 'L' };
    let compat = u32_at(p, 2)?.reverse_bits();
    let level = u8_at(p, 12)?;
    let mut constraints = Vec::new();
    for i in 0..6 {
        constraints.push(u8_at(p, 6 + i)?);
    }
    while constraints.last() == Some(&0) {
        constraints.pop();
    }
    let mut s = format!(
        "{}.{space}{profile}.{compat:X}.{tier}{level}",
        type_str(entry)
    );
    for c in constraints {
        s.push_str(&format!(".{c:X}"));
    }
    Ok(s)
}

pub(crate) fn vp9_codec(p: &[u8]) -> Result<String, SplitError> {
    // FullBox(4), profile(1), level(1), bitDepth(4) chromaSubsampling(3) videoFullRangeFlag(1), ...
    let profile = u8_at(p, 4)?;
    let level = u8_at(p, 5)?;
    let depth = u8_at(p, 6)? >> 4;
    Ok(format!("vp09.{profile:02}.{level:02}.{depth:02}"))
}

pub(crate) fn av1_codec(p: &[u8]) -> Result<String, SplitError> {
    // marker/version(1), seq_profile(3) seq_level_idx_0(5), seq_tier_0(1) high_bitdepth(1)
    // twelve_bit(1) monochrome(1) chroma_subsampling_x(1) y(1) chroma_sample_position(2)
    let b1 = u8_at(p, 1)?;
    let b2 = u8_at(p, 2)?;
    let profile = b1 >> 5;
    let level = b1 & 0x1f;
    let tier = if b2 & 0x80 != 0 { 'H' } else { 'M' };
    let depth = if b2 & 0x40 != 0 {
        if profile == 2 && b2 & 0x20 != 0 {
            12
        } else {
            10
        }
    } else {
        8
    };
    Ok(format!("av01.{profile}.{level:02}{tier}.{depth:02}"))
}

/// Reads an MPEG-4 descriptor header at `pos`: (tag, payload start, payload end).
fn descriptor(p: &[u8], pos: usize) -> Result<(u8, usize, usize), SplitError> {
    let tag = u8_at(p, pos)?;
    let mut len = 0usize;
    let mut at = pos + 1;
    for _ in 0..4 {
        let b = u8_at(p, at)?;
        at += 1;
        len = (len << 7) | usize::from(b & 0x7f);
        if b & 0x80 == 0 {
            break;
        }
    }
    let end = at.checked_add(len).ok_or(SplitError::Truncated)?;
    if end > p.len() {
        return Err(SplitError::Truncated);
    }
    Ok((tag, at, end))
}

fn esds_codec(p: &[u8]) -> Result<Option<String>, SplitError> {
    // FullBox(4), then ES_Descriptor (0x03): ES_ID(2) flags(1) [..], DecoderConfigDescriptor (0x04):
    // objectTypeIndication(1) streamType(1) bufferSize(3) maxBitrate(4) avgBitrate(4),
    // DecoderSpecificInfo (0x05) = AudioSpecificConfig.
    let (tag, es_start, es_end) = descriptor(p, 4)?;
    if tag != 0x03 {
        return Ok(None);
    }
    let flags = u8_at(p, es_start + 2)?;
    let mut pos = es_start + 3;
    if flags & 0x80 != 0 {
        pos += 2;
    }
    if flags & 0x40 != 0 {
        pos += 1 + usize::from(u8_at(p, pos)?);
    }
    if flags & 0x20 != 0 {
        pos += 2;
    }
    let (tag, dc_start, dc_end) = descriptor(p, pos)?;
    if tag != 0x04 || dc_end > es_end {
        return Ok(None);
    }
    let oti = u8_at(p, dc_start)?;
    match oti {
        0x40 | 0x66 | 0x67 | 0x68 => {
            let (tag, asc_start, _) = descriptor(p, dc_start + 13)?;
            if tag != 0x05 {
                return Ok(None);
            }
            let b0 = u8_at(p, asc_start)?;
            let mut aot = u32::from(b0 >> 3);
            if aot == 31 {
                let b1 = u8_at(p, asc_start + 1)?;
                aot = 32 + u32::from(((b0 & 0x07) << 3) | (b1 >> 5));
            }
            Ok(Some(format!("mp4a.40.{aot}")))
        }
        other => Ok(Some(format!("mp4a.{other:02X}"))),
    }
}

/// The type string for `addSourceBuffer`: container by the tracks present, codecs in track order,
/// comma-separated for muxed files.
pub(crate) fn mime_of(info: &InitInfo) -> Result<String, SplitError> {
    let mut codecs = Vec::new();
    let mut video = false;
    for t in &info.tracks {
        let is_v = InitInfo::is_video(t);
        let is_a = InitInfo::is_audio(t);
        if !is_v && !is_a {
            continue;
        }
        video |= is_v;
        match &t.codec {
            Some(c) => codecs.push(c.clone()),
            None => {
                return Err(SplitError::Unsupported(format!(
                    "no codec string for the sample entry '{}' of track {}",
                    type_str(&t.entry),
                    t.id
                )));
            }
        }
    }
    if codecs.is_empty() {
        return Err(SplitError::Unsupported(
            "the initialisation segment has no audio or video track".into(),
        ));
    }
    let container = if video { "video/mp4" } else { "audio/mp4" };
    Ok(format!("{container}; codecs=\"{}\"", codecs.join(", ")))
}
