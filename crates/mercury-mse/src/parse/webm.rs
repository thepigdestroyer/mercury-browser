// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! the WebM parser (EBML header, Segment, Tracks, Cluster, SimpleBlock, BlockGroup).
//!
//! Written from RFC 8794, the Matroska element specification, the WebM container guidelines and the MSE
//! "WebM byte stream format" registry; the track and Info reading start from the spike's `mse-segments`
//! (Mercury's own). Ladybird's `WebMByteStreamParser` (BSD-2-Clause, read on 20 Nov 2026 as a cross-check
//! after this one was written) was not translated, so this file is Mercury's own (PolyForm).
//!
//! * The initialization segment is the EBML header, the Segment header, `Info` and `Tracks` (any other
//!   level 1 element is skipped). The parser reports it ([`ParseEvent::Init`]) when both `Info` and
//!   `Tracks` have arrived, or at the first Cluster if `Info` never does. A Cluster before that is
//!   `MediaBeforeInit`. A second EBML header starts another initialization segment.
//! * Segments and Clusters of unknown size are the normal case (live and streamable muxers): the
//!   Segment is only a header here, and an unknown-size Cluster ends at the next element that can only
//!   be a child of the Segment. A Cluster of known size ends where it says. [`ParseEvent::SegmentEnd`]
//!   is sent at either end, which for the last Cluster of an unknown-size stream means when the next
//!   element arrives.
//! * Frames come out as soon as a block is complete. A block's duration is, in this order: the
//!   `BlockDuration` of its `BlockGroup`; the track's `DefaultDuration`; the distance to the next block
//!   of the same track (the frame waits for it, and at the end of the Cluster the last one takes the
//!   duration of the one before); with none of those a frame is held to 20 ms (audio) or 33 ms (video).
//! * Times are exact: Cluster timecode plus block offset, times `TimecodeScale`, in integer
//!   nanoseconds. A keyframe is a `SimpleBlock` with the keyframe bit or a `BlockGroup` without
//!   `ReferenceBlock`; every audio frame is a keyframe.
//! * One track per kind (decision D3); other kinds (subtitles) are ignored, with their blocks. A block of a
//!   track number the initialization segment does not list at all is `Malformed`, and a block with no data
//!   is dropped. VP8, VP9, AV1 and Opus are
//!   fed; any other codec is `Unsupported`. Lacing is `Unsupported`. `ContentEncodings` (protection) are
//!   not looked at. A VP9 track gets the short codec string `vp9` unless its `CodecPrivate` lists profile,
//!   level and bit depth (libvpx's muxer writes none).

use std::sync::Arc;

use crate::api::{
    ByteStreamParser, CodecConfig, CodedFrame, Container, InitSegment, Ns, ParseError, ParseEvent,
    TrackConfig, TrackKind,
};

use super::config::{av1_codec_string, parse_opus_head, vp9_webm_codec_string};
use super::ebml::{
    BlockHeader, ID_AUDIO, ID_BLOCK, ID_BLOCK_DURATION, ID_BLOCK_GROUP, ID_CHANNELS, ID_CLUSTER,
    ID_CODEC_ID, ID_CODEC_PRIVATE, ID_DEFAULT_DURATION, ID_DOC_TYPE, ID_DURATION, ID_EBML,
    ID_FLAG_DEFAULT, ID_FLAG_ENABLED, ID_INFO, ID_LANGUAGE, ID_PIXEL_HEIGHT, ID_PIXEL_WIDTH,
    ID_REFERENCE_BLOCK, ID_SAMPLING_FREQUENCY, ID_SEGMENT, ID_SIMPLE_BLOCK, ID_TIMECODE,
    ID_TIMECODE_SCALE, ID_TRACK_ENTRY, ID_TRACK_NUMBER, ID_TRACK_TYPE, ID_TRACKS, ID_VIDEO,
    elements, is_level1, parse_block_header, peek_header, read_float, read_string, read_uint,
};
use super::{
    ByteQueue, FEED_SLICE, MAX_BOX_BYTES, MAX_CODEC_CONFIG_BYTES, MAX_RETAINED_BYTES, Output,
    PResult, malformed, unsupported,
};

const EBML_MAGIC: [u8; 4] = [0x1a, 0x45, 0xdf, 0xa3];
const CLUSTER_MAGIC: [u8; 4] = [0x1f, 0x43, 0xb6, 0x75];

/// The duration given to a frame when nothing says (and nothing came before it to copy from).
const FALLBACK_AUDIO_DURATION: Ns = 20_000_000;
const FALLBACK_VIDEO_DURATION: Ns = 33_000_000;

#[derive(Clone, Copy, Debug)]
enum State {
    /// Waiting for the header of the next element at `pos`.
    Header,
    /// Waiting for all of an element that is read whole.
    Whole {
        id: u32,
        start: u64,
        header_len: usize,
        end: u64,
    },
    /// Passing over the body of an element up to `end`.
    Skip { end: u64 },
}

/// `Info`: what the initialization segment needs of it.
#[derive(Clone, Copy, Debug)]
struct Info {
    /// Nanoseconds per timecode unit.
    scale: u64,
    /// In timecode units.
    duration: Option<f64>,
}

impl Default for Info {
    fn default() -> Self {
        Info {
            scale: 1_000_000,
            duration: None,
        }
    }
}

/// A `TrackEntry` as written.
#[derive(Debug, Default)]
struct RawTrack {
    number: u64,
    kind: u64,
    codec_id: String,
    codec_private: Vec<u8>,
    default_duration: Option<u64>,
    width: u32,
    height: u32,
    sample_rate: Option<f64>,
    channels: Option<u64>,
    language: String,
    default: bool,
}

#[derive(Debug)]
struct Held {
    pts: Ns,
    key: bool,
    data: Arc<[u8]>,
}

/// A track of the initialization segment in use.
#[derive(Debug)]
struct Track {
    number: u64,
    cfg: TrackConfig,
    default_duration: Option<Ns>,
    last_duration: Option<Ns>,
    /// A frame waiting for the next one of the same track to learn its duration.
    held: Option<Held>,
}

impl Track {
    fn estimate(&self) -> Ns {
        self.last_duration.unwrap_or(match self.cfg.kind {
            TrackKind::Audio => FALLBACK_AUDIO_DURATION,
            _ => FALLBACK_VIDEO_DURATION,
        })
    }

    fn emit(&mut self, h: Held, duration: Ns, out: &mut Output) {
        if duration > 0 {
            self.last_duration = Some(duration);
        }
        out.frame(CodedFrame {
            kind: self.cfg.kind,
            track_id: self.cfg.track_id,
            pts: h.pts,
            dts: h.pts,
            duration,
            keyframe: h.key || self.cfg.kind == TrackKind::Audio,
            data: h.data,
        });
    }
}

/// The initialization segment in use: its timecode scale and tracks.
#[derive(Debug)]
struct Active {
    scale: u64,
    tracks: Vec<Track>,
    /// Numbers of the tracks this parser does not feed (subtitles): their blocks are dropped.
    ignored: Vec<u64>,
}

impl Active {
    /// Takes one frame of track `idx`. `explicit` is its `BlockDuration` in nanoseconds.
    fn on_frame(
        &mut self,
        idx: usize,
        pts: Ns,
        key: bool,
        data: Arc<[u8]>,
        explicit: Option<Ns>,
        out: &mut Output,
    ) {
        let Some(t) = self.tracks.get_mut(idx) else {
            return;
        };
        if let Some(h) = t.held.take() {
            let d = if pts > h.pts {
                pts - h.pts
            } else {
                t.estimate()
            };
            t.emit(h, d, out);
        }
        let held = Held { pts, key, data };
        match explicit.or(t.default_duration) {
            Some(d) => t.emit(held, d, out),
            None => t.held = Some(held),
        }
    }

    /// At the end of a Cluster: the frames still waiting for a successor take an estimated duration.
    fn flush_held(&mut self, out: &mut Output) {
        for t in &mut self.tracks {
            if let Some(h) = t.held.take() {
                let d = t.estimate();
                t.emit(h, d, out);
            }
        }
    }

    fn drop_held(&mut self) {
        for t in &mut self.tracks {
            t.held = None;
        }
    }

    /// One `SimpleBlock` or `Block`: `key` is the keyframe flag for a SimpleBlock, the absence of a
    /// ReferenceBlock for a BlockGroup.
    fn on_block(
        &mut self,
        cluster_timecode: i64,
        payload: &[u8],
        key: impl FnOnce(&BlockHeader) -> bool,
        explicit_ticks: Option<u64>,
        out: &mut Output,
    ) -> PResult<()> {
        let header = parse_block_header(payload)?;
        if header.lacing() != 0 {
            return Err(unsupported("a laced block"));
        }
        let Some(idx) = self.tracks.iter().position(|t| t.number == header.track) else {
            if self.ignored.contains(&header.track) {
                return Ok(()); // a track this parser ignores (subtitles)
            }
            return Err(malformed(
                "a block of a track that the initialization segment does not describe",
            ));
        };
        let ticks = cluster_timecode
            .checked_add(i64::from(header.timecode))
            .ok_or_else(|| malformed("a block time out of range"))?;
        let pts = ticks_ns(ticks, self.scale)?;
        let explicit = match explicit_ticks {
            Some(d) => Some(ticks_ns(
                i64::try_from(d).map_err(|_| malformed("a block duration out of range"))?,
                self.scale,
            )?),
            None => None,
        };
        let keyframe = key(&header);
        let body = payload.get(header.data_start..).unwrap_or(&[]);
        if body.is_empty() {
            return Ok(()); // a frame with no data is no use to any codec
        }
        let data: Arc<[u8]> = Arc::from(body);
        self.on_frame(idx, pts, keyframe, data, explicit, out);
        Ok(())
    }
}

/// `ticks` timecode units of `scale` nanoseconds, as nanoseconds.
fn ticks_ns(ticks: i64, scale: u64) -> PResult<Ns> {
    let n = i128::from(ticks) * i128::from(scale);
    Ns::try_from(n).map_err(|_| malformed("a timestamp does not fit in 64 bits of nanoseconds"))
}

#[derive(Debug)]
struct Cluster {
    /// Absolute offset where it ends; `None` for an unknown size.
    end: Option<u64>,
    timecode: Option<i64>,
}

/// The incremental WebM parser. Use it through [`ByteStreamParser`] (or `crate::parse::new_parser`).
#[derive(Debug)]
pub struct WebmParser {
    q: ByteQueue,
    pos: u64,
    state: State,
    failed: Option<ParseError>,
    /// An EBML header has been read: an initialization segment is being or has been read.
    saw_ebml: bool,
    info: Option<Info>,
    pending_tracks: Option<Vec<RawTrack>>,
    /// The initialization segment that began with the last EBML header has been reported.
    init_emitted: bool,
    active: Option<Active>,
    cluster: Option<Cluster>,
}

impl Default for WebmParser {
    fn default() -> Self {
        Self::new()
    }
}

impl WebmParser {
    pub fn new() -> Self {
        WebmParser {
            q: ByteQueue::new(),
            pos: 0,
            state: State::Header,
            failed: None,
            saw_ebml: false,
            info: None,
            pending_tracks: None,
            init_emitted: false,
            active: None,
            cluster: None,
        }
    }

    /// Bytes of input the parser is holding on to right now (a test hook for the memory bound).
    pub fn retained_bytes(&self) -> usize {
        self.q.len()
    }

    fn run(&mut self, data: &[u8]) -> PResult<Vec<ParseEvent>> {
        let mut out = Output::default();
        for slice in data.chunks(FEED_SLICE) {
            self.q.push(slice);
            self.pump(&mut out)?;
        }
        Ok(out.finish())
    }

    fn drop_partial(&mut self) {
        let end = self.q.end();
        self.q.clear_to(end);
        self.pos = end;
        self.state = State::Header;
        self.cluster = None;
        if let Some(a) = &mut self.active {
            a.drop_held();
        }
        self.init_emitted = self.active.is_some();
        if !self.init_emitted {
            self.saw_ebml = false;
            self.info = None;
            self.pending_tracks = None;
        }
    }

    fn pump(&mut self, out: &mut Output) -> PResult<()> {
        loop {
            let progressed = match self.state {
                State::Header => self.step_header(out)?,
                State::Whole { .. } => self.step_whole(out)?,
                State::Skip { end } => self.step_skip(end),
            };
            self.trim();
            if self.q.len() > MAX_RETAINED_BYTES {
                return Err(malformed(
                    "more input is held than the buffering limit allows",
                ));
            }
            if !progressed {
                return Ok(());
            }
        }
    }

    fn trim(&mut self) {
        let from = match self.state {
            State::Whole { start, .. } => start,
            State::Header | State::Skip { .. } => self.pos,
        };
        self.q.discard_before(from);
    }

    fn step_skip(&mut self, end: u64) -> bool {
        let target = end.min(self.q.end());
        let mut progressed = false;
        if target > self.pos {
            self.pos = target;
            progressed = true;
        }
        if self.pos == end {
            self.state = State::Header;
            progressed = true;
        }
        progressed
    }

    fn end_cluster(&mut self, out: &mut Output) {
        if let Some(a) = &mut self.active {
            a.flush_held(out);
        }
        self.cluster = None;
        out.segment_end();
    }

    /// What the first bytes of a stream that has no initialization segment yet may be.
    fn sniff(avail: &[u8]) -> PResult<()> {
        if avail[..4] == EBML_MAGIC {
            return Ok(());
        }
        if avail[..4] == CLUSTER_MAGIC {
            return Err(ParseError::MediaBeforeInit);
        }
        if matches!(
            &avail[4..8],
            b"ftyp" | b"moov" | b"moof" | b"mdat" | b"styp" | b"sidx" | b"free" | b"skip" | b"wide"
        ) {
            return Err(ParseError::WrongContainer(
                "an ISO BMFF box: this is an MP4 stream, not a WebM".to_string(),
            ));
        }
        Err(malformed("no EBML header: this is not a WebM stream"))
    }

    fn step_header(&mut self, out: &mut Output) -> PResult<bool> {
        if let Some(c) = &self.cluster
            && let Some(end) = c.end
            && self.pos >= end
        {
            self.end_cluster(out);
            return Ok(true);
        }
        let avail = self.q.peek(self.pos, 12);
        if !self.saw_ebml && self.active.is_none() {
            if avail.len() < 8 {
                return Ok(false);
            }
            Self::sniff(avail)?;
        }
        let Some(h) = peek_header(avail)? else {
            return Ok(false);
        };
        let start = self.pos;
        let header_len = h.header_len() as u64;

        // An unknown-size Cluster ends at the next element that can only be a child of the Segment.
        if let Some(c) = &self.cluster
            && c.end.is_none()
            && (is_level1(h.id) || h.id == ID_EBML || h.id == ID_SEGMENT)
        {
            self.end_cluster(out);
            return Ok(true);
        }
        let in_cluster = self.cluster.is_some();

        match h.id {
            ID_SEGMENT if !in_cluster => {
                // The Segment is only a header: its children are read as they come.
                self.pos = start + header_len;
            }
            ID_CLUSTER if !in_cluster => {
                if self.pending_tracks.is_some() && !self.init_emitted {
                    self.finish_init(out)?;
                }
                if self.active.is_none() {
                    return Err(ParseError::MediaBeforeInit);
                }
                self.pos = start + header_len;
                let end = match h.size {
                    Some(s) => Some(self.pos.checked_add(s).ok_or_else(|| {
                        malformed("a Cluster ends beyond the addressable stream")
                    })?),
                    None => None,
                };
                self.cluster = Some(Cluster {
                    end,
                    timecode: None,
                });
            }
            ID_EBML if !in_cluster => self.read_whole(h.id, start, &h)?,
            ID_INFO | ID_TRACKS if !in_cluster => self.read_whole(h.id, start, &h)?,
            ID_TIMECODE | ID_SIMPLE_BLOCK | ID_BLOCK_GROUP if in_cluster => {
                self.read_whole(h.id, start, &h)?
            }
            ID_CLUSTER => return Err(malformed("a Cluster inside a Cluster")),
            _ => {
                let size = bounded(h.size)?;
                let end = self.element_end(start, h.header_len(), size)?;
                self.pos = start + header_len;
                self.state = State::Skip { end };
            }
        }
        Ok(true)
    }

    /// Where an element of `size` payload bytes starting at `start` ends, which may not be past the end of
    /// the known-size Cluster it is in.
    fn element_end(&self, start: u64, header_len: usize, size: u64) -> PResult<u64> {
        let end = start
            .checked_add(header_len as u64)
            .and_then(|e| e.checked_add(size))
            .ok_or_else(|| malformed("an element ends beyond the addressable stream"))?;
        if let Some(c) = &self.cluster
            && let Some(ce) = c.end
            && end > ce
        {
            return Err(malformed("an element runs past the end of its Cluster"));
        }
        Ok(end)
    }

    fn read_whole(&mut self, id: u32, start: u64, h: &super::ebml::ElemHeader) -> PResult<()> {
        let size = bounded(h.size)?;
        let end = self.element_end(start, h.header_len(), size)?;
        self.state = State::Whole {
            id,
            start,
            header_len: h.header_len(),
            end,
        };
        Ok(())
    }

    fn step_whole(&mut self, out: &mut Output) -> PResult<bool> {
        let State::Whole {
            id,
            start,
            header_len,
            end,
        } = self.state
        else {
            return Ok(false);
        };
        if self.q.end() < end {
            return Ok(false);
        }
        let Some(bytes) = self.q.slice(start, end) else {
            return Err(malformed("an element that was dropped before it was read"));
        };
        let payload = bytes.get(header_len..).unwrap_or(&[]);
        match id {
            ID_EBML => {
                parse_ebml_header(payload)?;
                self.saw_ebml = true;
                self.info = None;
                self.pending_tracks = None;
                self.init_emitted = false;
            }
            ID_INFO => {
                let info = parse_info(payload)?;
                if !self.init_emitted {
                    self.info = Some(info);
                    self.state = State::Header;
                    self.pos = end;
                    self.maybe_init(out)?;
                    return Ok(true);
                }
            }
            ID_TRACKS => {
                let tracks = parse_tracks(payload)?;
                if !self.init_emitted {
                    self.pending_tracks = Some(tracks);
                    self.state = State::Header;
                    self.pos = end;
                    self.maybe_init(out)?;
                    return Ok(true);
                }
            }
            ID_TIMECODE => {
                let v = read_uint(payload)?;
                let tc =
                    i64::try_from(v).map_err(|_| malformed("a Cluster timecode out of range"))?;
                if let Some(c) = &mut self.cluster {
                    c.timecode = Some(tc);
                }
            }
            ID_SIMPLE_BLOCK => {
                let tc = self.cluster.as_ref().and_then(|c| c.timecode);
                let (Some(tc), Some(active)) = (tc, self.active.as_mut()) else {
                    return Err(malformed("a block before the Cluster timecode"));
                };
                active.on_block(tc, payload, BlockHeader::keyframe, None, out)?;
            }
            ID_BLOCK_GROUP => {
                let tc = self.cluster.as_ref().and_then(|c| c.timecode);
                let (Some(tc), Some(active)) = (tc, self.active.as_mut()) else {
                    return Err(malformed("a block before the Cluster timecode"));
                };
                let mut block = None;
                let mut duration = None;
                let mut referenced = false;
                for e in elements(payload) {
                    let e = e?;
                    match e.id {
                        ID_BLOCK => block = Some(e.payload),
                        ID_BLOCK_DURATION => duration = Some(read_uint(e.payload)?),
                        ID_REFERENCE_BLOCK => referenced = true,
                        _ => {}
                    }
                }
                if let Some(b) = block {
                    active.on_block(tc, b, |_| !referenced, duration, out)?;
                }
            }
            _ => {}
        }
        self.pos = end;
        self.state = State::Header;
        Ok(true)
    }

    /// Reports the initialization segment once `Info` and `Tracks` are both in.
    fn maybe_init(&mut self, out: &mut Output) -> PResult<()> {
        if self.info.is_some() && self.pending_tracks.is_some() && !self.init_emitted {
            self.finish_init(out)?;
        }
        Ok(())
    }

    fn finish_init(&mut self, out: &mut Output) -> PResult<()> {
        let info = self.info.unwrap_or_default();
        let raws = self
            .pending_tracks
            .take()
            .ok_or_else(|| malformed("an initialization segment without Tracks"))?;
        let mut tracks: Vec<Track> = Vec::new();
        for raw in &raws {
            if raw.kind != 1 && raw.kind != 2 {
                continue; // subtitles and the like
            }
            let (cfg, default_duration) = build_track(raw, info.scale)?;
            if tracks.iter().any(|t| t.number == raw.number) {
                return Err(malformed("two tracks with the same number"));
            }
            tracks.push(Track {
                number: raw.number,
                cfg,
                default_duration,
                last_duration: None,
                held: None,
            });
        }
        if tracks.is_empty() {
            return Err(unsupported(
                "no audio or video track in the initialization segment",
            ));
        }
        for kind in [TrackKind::Video, TrackKind::Audio] {
            if tracks.iter().filter(|t| t.cfg.kind == kind).count() > 1 {
                return Err(ParseError::TooManyTracks);
            }
        }
        let duration = info
            .duration
            .map(|d| d * info.scale as f64)
            .filter(|d| d.is_finite() && *d > 0.0 && *d < i64::MAX as f64)
            .map(|d| d.round() as Ns);
        let init = InitSegment {
            container: Container::WebM,
            tracks: tracks.iter().map(|t| t.cfg.clone()).collect(),
            duration,
            pssh: Vec::new(),
        };
        self.active = Some(Active {
            scale: info.scale,
            tracks,
            ignored: raws
                .iter()
                .filter(|r| r.kind != 1 && r.kind != 2)
                .map(|r| r.number)
                .collect(),
        });
        self.init_emitted = true;
        out.init(init);
        Ok(())
    }
}

/// The size of an element that is buffered or skipped: known and within the limit.
fn bounded(size: Option<u64>) -> PResult<u64> {
    match size {
        None => Err(malformed(
            "an element of unknown size where only a Segment or a Cluster may have one",
        )),
        Some(s) if s > MAX_BOX_BYTES => Err(malformed(format!(
            "an element of {s} bytes is larger than the {} MiB limit",
            MAX_BOX_BYTES >> 20
        ))),
        Some(s) => Ok(s),
    }
}

impl ByteStreamParser for WebmParser {
    fn container(&self) -> Container {
        Container::WebM
    }

    fn push(&mut self, data: &[u8]) -> Result<Vec<ParseEvent>, ParseError> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        match self.run(data) {
            Ok(events) => Ok(events),
            Err(e) => {
                self.failed = Some(e.clone());
                self.drop_partial();
                Err(e)
            }
        }
    }

    fn reset(&mut self) {
        self.failed = None;
        self.drop_partial();
    }
}

// ----------------------------------------------------------------------------------------------
// Initialization segment elements

fn parse_ebml_header(payload: &[u8]) -> PResult<()> {
    for e in elements(payload) {
        let e = e?;
        if e.id == ID_DOC_TYPE {
            let doc = read_string(e.payload);
            if doc != "webm" && doc != "matroska" {
                return Err(unsupported(format!("the EBML document type {doc:?}")));
            }
        }
    }
    Ok(())
}

fn parse_info(payload: &[u8]) -> PResult<Info> {
    let mut info = Info::default();
    for e in elements(payload) {
        let e = e?;
        if e.id == ID_TIMECODE_SCALE {
            let s = read_uint(e.payload)?;
            if s == 0 || s > 1_000_000_000_000 {
                return Err(malformed("a TimecodeScale out of range"));
            }
            info.scale = s;
        } else if e.id == ID_DURATION {
            info.duration = Some(read_float(e.payload)?);
        }
    }
    Ok(info)
}

fn parse_tracks(payload: &[u8]) -> PResult<Vec<RawTrack>> {
    let mut out = Vec::new();
    for e in elements(payload) {
        let e = e?;
        if e.id == ID_TRACK_ENTRY {
            out.push(parse_track_entry(e.payload)?);
        }
    }
    Ok(out)
}

fn parse_track_entry(payload: &[u8]) -> PResult<RawTrack> {
    let mut t = RawTrack {
        default: true,
        ..RawTrack::default()
    };
    let mut enabled = true;
    for e in elements(payload) {
        let e = e?;
        match e.id {
            ID_TRACK_NUMBER => t.number = read_uint(e.payload)?,
            ID_TRACK_TYPE => t.kind = read_uint(e.payload)?,
            ID_CODEC_ID => t.codec_id = read_string(e.payload),
            ID_CODEC_PRIVATE => {
                if e.payload.len() > MAX_CODEC_CONFIG_BYTES {
                    return Err(unsupported(format!(
                        "a CodecPrivate of {} bytes (the most kept is {MAX_CODEC_CONFIG_BYTES})",
                        e.payload.len()
                    )));
                }
                t.codec_private = e.payload.to_vec();
            }
            ID_DEFAULT_DURATION => t.default_duration = Some(read_uint(e.payload)?),
            ID_FLAG_DEFAULT => t.default = read_uint(e.payload)? != 0,
            ID_FLAG_ENABLED => enabled = read_uint(e.payload)? != 0,
            ID_LANGUAGE => {
                let l = read_string(e.payload);
                t.language = if l == "und" { String::new() } else { l };
            }
            ID_VIDEO => {
                for v in elements(e.payload) {
                    let v = v?;
                    match v.id {
                        ID_PIXEL_WIDTH => {
                            t.width = u32::try_from(read_uint(v.payload)?).unwrap_or(0)
                        }
                        ID_PIXEL_HEIGHT => {
                            t.height = u32::try_from(read_uint(v.payload)?).unwrap_or(0)
                        }
                        _ => {}
                    }
                }
            }
            ID_AUDIO => {
                for a in elements(e.payload) {
                    let a = a?;
                    match a.id {
                        ID_SAMPLING_FREQUENCY => t.sample_rate = Some(read_float(a.payload)?),
                        ID_CHANNELS => t.channels = Some(read_uint(a.payload)?),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    t.default &= enabled;
    if t.number == 0 {
        return Err(malformed("a track with number 0"));
    }
    Ok(t)
}

/// The configuration of one audio or video track, and its default frame duration.
fn build_track(raw: &RawTrack, scale: u64) -> PResult<(TrackConfig, Option<Ns>)> {
    let kind = if raw.kind == 1 {
        TrackKind::Video
    } else {
        TrackKind::Audio
    };
    let track_id = u32::try_from(raw.number)
        .map_err(|_| malformed("a track number that does not fit in 32 bits"))?;
    let mut width = raw.width;
    let mut height = raw.height;
    let mut sample_rate = 0u32;
    let mut channels = 0u32;
    let want = |k: TrackKind| -> PResult<()> {
        if k == kind {
            Ok(())
        } else {
            Err(malformed(format!(
                "the codec {:?} in a track of the wrong type",
                raw.codec_id
            )))
        }
    };
    let (codec, codec_string) = match raw.codec_id.as_str() {
        "V_VP8" => {
            want(TrackKind::Video)?;
            (CodecConfig::Vp8, "vp8".to_string())
        }
        "V_VP9" => {
            want(TrackKind::Video)?;
            (
                CodecConfig::Vp9 { vpcc: None },
                vp9_webm_codec_string(&raw.codec_private),
            )
        }
        "V_AV1" => {
            want(TrackKind::Video)?;
            if raw.codec_private.len() < 4 {
                return Err(unsupported("an AV1 track without CodecPrivate (av1C)"));
            }
            (
                CodecConfig::Av1 {
                    av1c: raw.codec_private.clone(),
                },
                av1_codec_string(&raw.codec_private)?,
            )
        }
        "A_OPUS" => {
            want(TrackKind::Audio)?;
            let head = parse_opus_head(&raw.codec_private)?;
            channels = u32::from(head.channels);
            sample_rate = match raw.sample_rate {
                Some(r) if r.is_finite() && r >= 1.0 && r < f64::from(u32::MAX) => r as u32,
                _ if head.input_rate != 0 => head.input_rate,
                _ => 48_000,
            };
            (
                CodecConfig::Opus {
                    head: raw.codec_private.clone(),
                },
                "opus".to_string(),
            )
        }
        other => {
            return Err(unsupported(format!("the WebM codec {other:?}")));
        }
    };
    if kind == TrackKind::Audio && channels == 0 {
        channels = raw
            .channels
            .and_then(|c| u32::try_from(c).ok())
            .unwrap_or(0);
    }
    if kind != TrackKind::Video {
        width = 0;
        height = 0;
    }
    let default_duration = match raw.default_duration {
        Some(d) if d > 0 => {
            Some(Ns::try_from(d).map_err(|_| malformed("a DefaultDuration out of range"))?)
        }
        _ => None,
    };
    Ok((
        TrackConfig {
            kind,
            track_id,
            codec,
            codec_string,
            width,
            height,
            sample_rate,
            channels,
            timescale: u32::try_from(1_000_000_000 / scale.max(1)).unwrap_or(0),
            language: raw.language.clone(),
            default: raw.default,
        },
        default_duration,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_parser_waits_and_names_what_it_was_given() {
        let mut p = WebmParser::new();
        assert_eq!(p.container(), Container::WebM);
        assert!(p.push(&[]).unwrap().is_empty());
        // Fewer than 8 bytes decide nothing, whatever they are.
        assert!(
            p.push(&[0, 0, 0, 0x20, b'f', b't', b'y'])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            p.push(b"p").unwrap_err(),
            ParseError::WrongContainer("an ISO BMFF box: this is an MP4 stream, not a WebM".into())
        );
        // The error sticks until reset.
        assert!(p.push(&[1]).is_err());
        p.reset();
        assert!(p.push(&[0x1a, 0x45]).unwrap().is_empty());
    }

    #[test]
    fn times_are_integer_nanoseconds() {
        assert_eq!(ticks_ns(1234, 1_000_000).unwrap(), 1_234_000_000);
        assert_eq!(ticks_ns(-5, 1_000_000).unwrap(), -5_000_000);
        assert_eq!(ticks_ns(3, 20_833).unwrap(), 62_499);
        assert!(ticks_ns(i64::MAX, 1_000_000).is_err());
    }

    #[test]
    fn sizes_are_bounded() {
        assert!(bounded(None).is_err());
        assert!(bounded(Some(MAX_BOX_BYTES + 1)).is_err());
        assert_eq!(bounded(Some(5)).unwrap(), 5);
    }

    #[test]
    fn track_numbers_and_codecs_are_checked() {
        let mut raw = RawTrack {
            number: 1,
            kind: 1,
            codec_id: "V_VP8".into(),
            ..RawTrack::default()
        };
        let (cfg, dd) = build_track(&raw, 1_000_000).unwrap();
        assert_eq!(
            (cfg.codec_string.as_str(), cfg.timescale, dd),
            ("vp8", 1000, None)
        );
        raw.codec_id = "V_MPEG4/ISO/AVC".into();
        assert!(matches!(
            build_track(&raw, 1_000_000),
            Err(ParseError::Unsupported(_))
        ));
        raw.codec_id = "A_OPUS".into();
        assert!(matches!(
            build_track(&raw, 1_000_000),
            Err(ParseError::Malformed(_))
        ));
        raw.codec_id = "V_AV1".into();
        assert!(matches!(
            build_track(&raw, 1_000_000),
            Err(ParseError::Unsupported(_))
        ));
        raw.number = u64::MAX;
        raw.codec_id = "V_VP9".into();
        assert!(build_track(&raw, 1_000_000).is_err());
    }
}
