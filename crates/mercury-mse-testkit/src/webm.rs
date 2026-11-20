// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! WebM: splitting into an initialisation segment and one media segment per Cluster, timing of
//! each Cluster, the type string from the Tracks element, and shifting Cluster timecodes.
//!
//! The rules (the "WebM byte stream format" of the MSE registry):
//!
//! * The initialisation segment is every byte before the first Cluster: the EBML header, the
//!   Segment element's header (its size may be "unknown", as live and streamable muxers write it),
//!   and the Segment's children before the first Cluster (`Info`, `Tracks`, `Tags`, `Void`, ...).
//! * Each Cluster is one media segment. An unknown-size Cluster ends at the next element that can only
//!   be a child of the Segment (a Cluster, `Cues`, `Tags`, ...) or at the end of the data.
//! * Level-1 elements after the first Cluster that are not Clusters (`Cues`, `Tags`, `Void`) belong to
//!   no segment and are dropped.
//! * `start_ms` is the Cluster Timecode times the TimecodeScale (default 1 ms). `duration_ms` comes
//!   from the blocks: the latest block end (its BlockDuration, else the track's DefaultDuration, else
//!   the spacing of the track's last two blocks) minus the Cluster's start.

use std::collections::HashMap;

use crate::bytes::{slice, u8_at};
use crate::ebml::{
    Elem, ID_BLOCK, ID_BLOCK_DURATION, ID_BLOCK_GROUP, ID_CLUSTER, ID_CODEC_ID, ID_CODEC_PRIVATE,
    ID_DEFAULT_DURATION, ID_DURATION, ID_EBML, ID_INFO, ID_PIXEL_HEIGHT, ID_PIXEL_WIDTH,
    ID_SEGMENT, ID_SIMPLE_BLOCK, ID_TIMECODE, ID_TIMECODE_SCALE, ID_TRACK_ENTRY, ID_TRACK_NUMBER,
    ID_TRACK_TYPE, ID_TRACKS, ID_VIDEO, children, is_level1, payload, read_elem, read_elem_header,
    read_float, read_uint, write_size, write_uint_elem,
};
use crate::{Container, Segment, SegmentKind, SegmentedStream, SplitError};

#[derive(Debug, Clone, Default)]
pub(crate) struct WebmTrack {
    pub number: u64,
    /// 1 video, 2 audio, 17 subtitle, ...
    pub kind: u64,
    pub codec_id: String,
    pub codec_private: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Nanoseconds.
    pub default_duration: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct WebmInfo {
    /// Nanoseconds per timecode unit.
    pub timecode_scale: u64,
    /// Segment duration in timecode units, when the Info element has one.
    pub duration: Option<f64>,
    pub tracks: Vec<WebmTrack>,
}

/// Parses the Info and Tracks elements of an initialisation segment.
pub(crate) fn parse_init(init: &[u8]) -> Result<WebmInfo, SplitError> {
    let (_, _, children_start, limit) = segment_bounds(init)?;
    let mut info = WebmInfo {
        timecode_scale: 1_000_000,
        duration: None,
        tracks: Vec::new(),
    };
    let mut pos = children_start;
    while pos < limit {
        let e = read_elem(init, pos, limit)?;
        let end = e.end.ok_or_else(|| {
            SplitError::Malformed(
                "a level 1 element before the first Cluster has an unknown size".into(),
            )
        })?;
        if e.id == ID_INFO {
            for c in children(init, e.data, end)? {
                if c.id == ID_TIMECODE_SCALE {
                    info.timecode_scale = read_uint(init, &c)?.max(1);
                } else if c.id == ID_DURATION {
                    info.duration = Some(read_float(init, &c)?);
                }
            }
        } else if e.id == ID_TRACKS {
            for t in children(init, e.data, end)? {
                if t.id == ID_TRACK_ENTRY {
                    info.tracks.push(parse_track(init, &t)?);
                }
            }
        }
        pos = end;
    }
    Ok(info)
}

fn parse_track(d: &[u8], entry: &Elem) -> Result<WebmTrack, SplitError> {
    let mut t = WebmTrack::default();
    let end = entry.end.ok_or(SplitError::Truncated)?;
    for c in children(d, entry.data, end)? {
        match c.id {
            ID_TRACK_NUMBER => t.number = read_uint(d, &c)?,
            ID_TRACK_TYPE => t.kind = read_uint(d, &c)?,
            ID_CODEC_ID => {
                // EBML strings may be padded with NUL bytes (GStreamer's muxer writes one).
                t.codec_id = String::from_utf8_lossy(payload(d, &c)?)
                    .trim_end_matches('\0')
                    .to_string();
            }
            ID_CODEC_PRIVATE => t.codec_private = payload(d, &c)?.to_vec(),
            ID_DEFAULT_DURATION => t.default_duration = Some(read_uint(d, &c)?),
            ID_VIDEO => {
                let vend = c.end.ok_or(SplitError::Truncated)?;
                for v in children(d, c.data, vend)? {
                    if v.id == ID_PIXEL_WIDTH {
                        t.width = u32::try_from(read_uint(d, &v)?).unwrap_or(0);
                    } else if v.id == ID_PIXEL_HEIGHT {
                        t.height = u32::try_from(read_uint(d, &v)?).unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(t)
}

/// (EBML header end, Segment element, first child offset, end of the children): validates the two
/// leading elements.
fn segment_bounds(d: &[u8]) -> Result<(usize, Elem, usize, usize), SplitError> {
    let ebml = read_elem(d, 0, d.len())?;
    if ebml.id != ID_EBML {
        return Err(SplitError::Malformed(
            "no EBML header: this is not a WebM or Matroska stream".into(),
        ));
    }
    let ebml_end = ebml
        .end
        .ok_or_else(|| SplitError::Malformed("the EBML header has an unknown size".into()))?;
    let seg = read_elem_header(d, ebml_end)?;
    if seg.id != ID_SEGMENT {
        return Err(SplitError::Malformed(
            "the EBML header is not followed by a Segment".into(),
        ));
    }
    // The Segment size is advisory: media segments are appended after the initialisation segment that
    // declared it, and may make the stream longer (shifted Clusters), so the children are read to the end
    // of the data, as an MSE byte stream parser does.
    Ok((ebml_end, seg, seg.data, d.len()))
}

/// The end of the Cluster at `e`: its known end, or the start of the next Segment child.
fn cluster_end(d: &[u8], e: &Elem, limit: usize) -> Result<usize, SplitError> {
    if let Some(end) = e.end {
        return Ok(end);
    }
    let mut pos = e.data;
    while pos < limit {
        let (id, _) = crate::ebml::read_id(d, pos)?;
        if is_level1(id) {
            return Ok(pos);
        }
        let c = read_elem(d, pos, limit)?;
        pos = c.end.ok_or_else(|| {
            SplitError::Malformed("nested unknown-size element in a Cluster".into())
        })?;
    }
    Ok(limit)
}

struct ClusterTiming {
    start_ms: u64,
    duration_ms: Option<u64>,
}

fn ns_to_ms(ns: u128) -> u64 {
    ((ns + 500_000) / 1_000_000) as u64
}

/// One block of a cluster: (track, absolute time in timecode units, explicit duration in units).
fn blocks_of(
    d: &[u8],
    cluster: &Elem,
    end: usize,
    cluster_tc: u64,
) -> Result<Vec<(u64, i128, Option<u64>)>, SplitError> {
    let mut out = Vec::new();
    let mut parse_block = |p: &[u8], duration: Option<u64>| -> Result<(), SplitError> {
        let (track, len) = crate::ebml::read_size(p, 0)?;
        let track = track.unwrap_or(0);
        let rel = i16::from_be_bytes([u8_at(p, len)?, u8_at(p, len + 1)?]);
        out.push((track, i128::from(cluster_tc) + i128::from(rel), duration));
        Ok(())
    };
    for c in children(d, cluster.data, end)? {
        if c.id == ID_SIMPLE_BLOCK {
            parse_block(payload(d, &c)?, None)?;
        } else if c.id == ID_BLOCK_GROUP {
            let gend = c.end.ok_or(SplitError::Truncated)?;
            let mut block = None;
            let mut dur = None;
            for g in children(d, c.data, gend)? {
                if g.id == ID_BLOCK {
                    block = Some(g);
                } else if g.id == ID_BLOCK_DURATION {
                    dur = Some(read_uint(d, &g)?);
                }
            }
            if let Some(b) = block {
                parse_block(payload(d, &b)?, dur)?;
            }
        }
    }
    Ok(out)
}

fn analyze_cluster(
    d: &[u8],
    cluster: &Elem,
    end: usize,
    info: &WebmInfo,
) -> Result<ClusterTiming, SplitError> {
    let mut tc = None;
    for c in children(d, cluster.data, end)? {
        if c.id == ID_TIMECODE {
            tc = Some(read_uint(d, &c)?);
            break;
        }
    }
    let tc = tc.ok_or_else(|| SplitError::Malformed("a Cluster without a Timecode".into()))?;
    let scale = u128::from(info.timecode_scale);
    let start_ms = ns_to_ms(u128::from(tc) * scale);

    // Block ends: explicit duration, else the track's DefaultDuration, else the spacing of the
    // track's last two blocks.
    let blocks = blocks_of(d, cluster, end, tc)?;
    let mut by_track: HashMap<u64, Vec<i128>> = HashMap::new();
    let mut max_end_units: Option<i128> = None;
    for (track, time, dur) in &blocks {
        by_track.entry(*track).or_default().push(*time);
        if let Some(du) = dur {
            let e = time + i128::from(*du);
            max_end_units = Some(max_end_units.map_or(e, |m| m.max(e)));
        }
    }
    let mut max_end_ns: Option<i128> = max_end_units.map(|u| u * scale as i128);
    for (track, times) in &by_track {
        let Some(&last) = times.last() else { continue };
        let default_ns = info
            .tracks
            .iter()
            .find(|t| t.number == *track)
            .and_then(|t| t.default_duration)
            .map(i128::from);
        let spacing_ns = if times.len() >= 2 {
            Some((last - times[times.len() - 2]) * scale as i128)
        } else {
            None
        };
        let end_ns = last * scale as i128 + default_ns.or(spacing_ns).unwrap_or(0);
        max_end_ns = Some(max_end_ns.map_or(end_ns, |m| m.max(end_ns)));
    }
    let duration_ms = max_end_ns.and_then(|ns| {
        let ns = u128::try_from(ns).ok()?;
        ns_to_ms(ns).checked_sub(start_ms)
    });
    Ok(ClusterTiming {
        start_ms,
        duration_ms: duration_ms.filter(|_| !blocks.is_empty()),
    })
}

// ---------------------------------------------------------------------------------------------
// Codec strings

/// Bits of the VP9 uncompressed header, most significant first.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        let byte = *self.d.get(self.pos / 8)?;
        let b = (byte >> (7 - (self.pos % 8))) & 1;
        self.pos += 1;
        Some(u32::from(b))
    }

    fn bits(&mut self, n: usize) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }
}

/// (profile, bit depth, width, height) from the uncompressed header of a VP9 key frame.
pub(crate) fn vp9_keyframe_header(frame: &[u8]) -> Option<(u32, u32, u32, u32)> {
    let mut b = Bits { d: frame, pos: 0 };
    if b.bits(2)? != 2 {
        return None;
    }
    let low = b.bit()?;
    let high = b.bit()?;
    let profile = (high << 1) + low;
    if profile == 3 {
        b.bit()?;
    }
    if b.bit()? == 1 {
        return None; // show_existing_frame
    }
    if b.bit()? != 0 {
        return None; // not a key frame
    }
    b.bit()?; // show_frame
    b.bit()?; // error_resilient_mode
    if b.bits(24)? != 0x49_8342 {
        return None;
    }
    let depth = if profile >= 2 {
        if b.bit()? == 1 { 12 } else { 10 }
    } else {
        8
    };
    let color_space = b.bits(3)?;
    if color_space != 7 {
        b.bit()?; // color_range
        if profile == 1 || profile == 3 {
            b.bits(3)?; // subsampling_x, subsampling_y, reserved_zero
        }
    } else if profile == 1 || profile == 3 {
        b.bit()?; // reserved_zero
    }
    let w = b.bits(16)? + 1;
    let h = b.bits(16)? + 1;
    Some((profile, depth, w, h))
}

/// The VP9 level a stream of this size and frame rate needs (VP9 levels table, "Level" 1 to 6.2, as
/// ten times the level): the bitstream does not carry the level.
pub(crate) fn vp9_level(w: u32, h: u32, fps: f64) -> u32 {
    // (level x 10, max luma picture size, max luma sample rate)
    const LEVELS: [(u32, u64, u64); 14] = [
        (10, 36_864, 829_440),
        (11, 73_728, 2_764_800),
        (20, 122_880, 4_608_000),
        (21, 245_760, 9_216_000),
        (30, 552_960, 20_736_000),
        (31, 983_040, 36_864_000),
        (40, 2_228_224, 83_558_400),
        (41, 2_228_224, 160_432_128),
        (50, 8_912_896, 311_951_360),
        (51, 8_912_896, 588_251_136),
        (52, 8_912_896, 1_176_502_272),
        (60, 35_651_584, 1_176_502_272),
        (61, 35_651_584, 2_353_004_544),
        (62, 35_651_584, 4_706_009_088),
    ];
    let ps = u64::from(w) * u64::from(h);
    let rate = (ps as f64 * fps.max(1.0)).ceil() as u64;
    LEVELS
        .iter()
        .find(|(_, max_ps, max_rate)| ps <= *max_ps && rate <= *max_rate)
        .map_or(62, |l| l.0)
}

/// The first frame of the first block of track `number` in the first Cluster, and the frame rate
/// estimated from that Cluster's block spacing.
fn first_video_frame(
    data: &[u8],
    cluster: &Elem,
    end: usize,
    number: u64,
    info: &WebmInfo,
) -> Result<(Option<Vec<u8>>, Option<f64>), SplitError> {
    let mut frame = None;
    let mut times: Vec<i128> = Vec::new();
    for c in children(data, cluster.data, end)? {
        let block = if c.id == ID_SIMPLE_BLOCK {
            Some(c)
        } else if c.id == ID_BLOCK_GROUP {
            let gend = c.end.ok_or(SplitError::Truncated)?;
            children(data, c.data, gend)?
                .into_iter()
                .find(|g| g.id == ID_BLOCK)
        } else {
            None
        };
        let Some(b) = block else { continue };
        let p = payload(data, &b)?;
        let (track, len) = crate::ebml::read_size(p, 0)?;
        if track != Some(number) {
            continue;
        }
        if frame.is_none() {
            frame = Some(slice(p, len + 3, p.len())?.to_vec());
        }
        let rel = i16::from_be_bytes([u8_at(p, len)?, u8_at(p, len + 1)?]);
        times.push(i128::from(rel));
    }
    let fps = if times.len() >= 2 {
        let span = (times[times.len() - 1] - times[0]) as f64 * info.timecode_scale as f64 / 1e9;
        (span > 0.0).then(|| (times.len() - 1) as f64 / span)
    } else {
        info.tracks
            .iter()
            .find(|t| t.number == number)
            .and_then(|t| t.default_duration)
            .map(|ns| 1e9 / ns as f64)
    };
    Ok((frame, fps))
}

/// The codec string of a track. VP9: from the CodecPrivate feature list when it has one (ID 1
/// profile, 2 level, 3 bit depth), else the profile and bit depth come from the first key frame's
/// header and the level is derived from the frame size and rate; when that is not possible the
/// short form `vp9` is used (it means "any VP9").
fn codec_of(
    t: &WebmTrack,
    first_frame: Option<&[u8]>,
    fps: Option<f64>,
) -> Result<String, SplitError> {
    match t.codec_id.as_str() {
        "V_VP9" => {
            let mut profile = None;
            let mut level = None;
            let mut depth = None;
            let mut i = 0;
            let p = &t.codec_private;
            while i + 2 <= p.len() {
                let (id, len) = (p[i], usize::from(p[i + 1]));
                if i + 2 + len > p.len() {
                    break;
                }
                if len == 1 {
                    match id {
                        1 => profile = Some(u32::from(p[i + 2])),
                        2 => level = Some(u32::from(p[i + 2])),
                        3 => depth = Some(u32::from(p[i + 2])),
                        _ => {}
                    }
                }
                i += 2 + len;
            }
            if let Some((fp, fd, w, h)) = first_frame.and_then(vp9_keyframe_header) {
                profile = profile.or(Some(fp));
                depth = depth.or(Some(fd));
                if level.is_none() {
                    level = Some(vp9_level(w, h, fps.unwrap_or(30.0)));
                }
            }
            match (profile, level, depth) {
                (Some(p), Some(l), Some(d)) => Ok(format!("vp09.{p:02}.{l:02}.{d:02}")),
                _ => Ok("vp9".to_string()),
            }
        }
        "V_VP8" => Ok("vp8".to_string()),
        "V_AV1" => {
            if t.codec_private.len() >= 4 {
                crate::mp4_init::av1_codec(&t.codec_private)
            } else {
                Err(SplitError::Unsupported(
                    "an AV1 track without CodecPrivate (av1C)".into(),
                ))
            }
        }
        "A_OPUS" => Ok("opus".to_string()),
        "A_VORBIS" => Ok("vorbis".to_string()),
        other => Err(SplitError::Unsupported(format!(
            "no codec string for the WebM codec {other:?}"
        ))),
    }
}

fn mime_of(
    info: &WebmInfo,
    data: &[u8],
    first_cluster: Option<(Elem, usize)>,
) -> Result<String, SplitError> {
    let mut codecs = Vec::new();
    let mut video = false;
    for t in &info.tracks {
        if t.kind != 1 && t.kind != 2 {
            continue;
        }
        video |= t.kind == 1;
        let (frame, fps) = match (t.kind, first_cluster) {
            (1, Some((c, end))) => first_video_frame(data, &c, end, t.number, info)?,
            _ => (None, None),
        };
        codecs.push(codec_of(t, frame.as_deref(), fps)?);
    }
    if codecs.is_empty() {
        return Err(SplitError::Unsupported(
            "the initialisation segment has no audio or video track".into(),
        ));
    }
    let container = if video { "video/webm" } else { "audio/webm" };
    Ok(format!("{container}; codecs=\"{}\"", codecs.join(", ")))
}

/// Splits WebM `data`; `mime_hint` is used only when the codec cannot be derived.
pub fn split_webm(data: &[u8], mime_hint: &str) -> Result<SegmentedStream, SplitError> {
    let (_, _, children_start, limit) = segment_bounds(data)?;
    let mut pos = children_start;
    let mut clusters: Vec<(Elem, usize)> = Vec::new();
    let mut init_end = None;
    while pos < limit {
        let e = read_elem(data, pos, limit)?;
        if e.id == ID_CLUSTER {
            init_end.get_or_insert(e.start);
            let end = cluster_end(data, &e, limit)?;
            clusters.push((e, end));
            pos = end;
        } else {
            let end = e.end.ok_or_else(|| {
                SplitError::Malformed(format!(
                    "level 1 element {:#x} at {pos} has an unknown size",
                    e.id
                ))
            })?;
            pos = end;
        }
    }
    let init_end = init_end.unwrap_or(limit);
    let init_bytes = &data[..init_end];
    let info = parse_init(init_bytes)?;
    if info.tracks.is_empty() {
        return Err(SplitError::Malformed("the Segment has no Tracks".into()));
    }
    let mime = match mime_of(&info, data, clusters.first().copied()) {
        Ok(m) => m,
        Err(e) if mime_hint.is_empty() => return Err(e),
        Err(_) => mime_hint.to_string(),
    };
    let mut media = Vec::with_capacity(clusters.len());
    for (e, end) in clusters {
        let t = analyze_cluster(data, &e, end, &info)?;
        media.push(Segment {
            kind: SegmentKind::Media,
            bytes: data[e.start..end].to_vec(),
            start_ms: Some(t.start_ms),
            duration_ms: t.duration_ms,
        });
    }
    Ok(SegmentedStream {
        container: Container::WebM,
        mime,
        init: Segment {
            kind: SegmentKind::Init,
            bytes: init_bytes.to_vec(),
            start_ms: None,
            duration_ms: None,
        },
        media,
    })
}

// ---------------------------------------------------------------------------------------------
// Shifting

/// Moves the Timecode of the Cluster in `seg` by `delta_ms`; returns the new bytes.
pub(crate) fn shift_cluster(
    seg: &[u8],
    info: &WebmInfo,
    delta_ms: i64,
) -> Result<Vec<u8>, SplitError> {
    let cluster = read_elem(seg, 0, seg.len())?;
    if cluster.id != ID_CLUSTER {
        return Err(SplitError::Malformed(
            "a media segment that is not a Cluster".into(),
        ));
    }
    let end = cluster_end(seg, &cluster, seg.len())?;
    let mut tc = None;
    for c in children(seg, cluster.data, end)? {
        if c.id == ID_TIMECODE {
            tc = Some(c);
            break;
        }
    }
    let tc = tc.ok_or_else(|| SplitError::Malformed("a Cluster without a Timecode".into()))?;
    let tc_end = tc.end.ok_or(SplitError::Truncated)?;
    let old = i128::from(read_uint(seg, &tc)?);
    let scale = i128::from(info.timecode_scale);
    let delta_ns = i128::from(delta_ms) * 1_000_000;
    let delta_units = if delta_ns >= 0 {
        (delta_ns + scale / 2) / scale
    } else {
        -((-delta_ns + scale / 2) / scale)
    };
    let new = old + delta_units;
    let new = u64::try_from(new)
        .map_err(|_| SplitError::Malformed("the shift moves a Cluster before time zero".into()))?;
    let old_len = tc_end - tc.data;
    let new_elem = write_uint_elem(ID_TIMECODE, new, old_len);

    let mut body = Vec::with_capacity(end - cluster.data + 8);
    body.extend_from_slice(&seg[cluster.data..tc.start]);
    body.extend_from_slice(&new_elem);
    body.extend_from_slice(&seg[tc_end..end]);

    let id_len = cluster.start..(cluster.data - cluster.size_len);
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(&seg[id_len]);
    match cluster.end {
        Some(_) => out.extend_from_slice(&write_size(body.len() as u64, cluster.size_len)),
        None => out.extend_from_slice(&seg[cluster.data - cluster.size_len..cluster.data]),
    }
    out.extend_from_slice(&body);
    out.extend_from_slice(&seg[end..]);
    Ok(out)
}

/// `delta_ms`-shifted copies of the Clusters of a WebM stream with initialisation segment `init`.
pub(crate) fn shifted_media(
    init: &[u8],
    media: &[Segment],
    delta_ms: i64,
) -> Result<Vec<Segment>, SplitError> {
    let info = parse_init(init)?;
    let mut out = Vec::with_capacity(media.len());
    for seg in media {
        let bytes = shift_cluster(&seg.bytes, &info, delta_ms)?;
        let cluster = read_elem(&bytes, 0, bytes.len())?;
        let end = cluster_end(&bytes, &cluster, bytes.len())?;
        let t = analyze_cluster(&bytes, &cluster, end, &info)?;
        out.push(Segment {
            kind: SegmentKind::Media,
            bytes,
            start_ms: Some(t.start_ms),
            duration_ms: t.duration_ms,
        });
    }
    Ok(out)
}
