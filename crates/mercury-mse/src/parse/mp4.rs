// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! the fragmented MP4 parser (moov, moof, traf, tfhd, tfdt, trun, mdat).
//!
//! Written from ISO/IEC 14496-12 (boxes, movie fragments), 14496-14/-15 (sample entries) and the MSE
//! "ISO BMFF byte stream format" registry; the sample entry reading starts from the spike's `mse-segments`
//! (Mercury's own). Ladybird's `ISOBMFFByteStreamParser` (BSD-2-Clause, read on 20 Nov 2026 as a
//! cross-check after this one was written) shares its idea of keeping a parsed `moof` and reading its
//! samples as their data arrives; nothing was translated, so this file is Mercury's own (PolyForm).
//!
//! What it does, in the terms of the MSE registry:
//!
//! * The initialization segment is `[ftyp] moov`; the parser reports it ([`ParseEvent::Init`]) as soon
//!   as the `moov` is complete. Boxes before it other than media boxes are skipped; a `moof` or `mdat`
//!   before it is `MediaBeforeInit`.
//! * A media segment is `[styp] [sidx] [emsg] moof mdat`. `styp`, `sidx`, `ssix`, `prft`, `emsg`, `free`,
//!   `skip`, `mfra` and unknown boxes are skipped without being buffered. The `moof` is buffered whole
//!   and turned into a list of samples with their byte ranges, timing and sync flags; each sample is
//!   emitted as soon as its bytes have arrived, in the order the `trun` boxes list them.
//!   [`ParseEvent::SegmentEnd`] follows the last sample once the `mdat` that carries it has ended.
//! * Timing is exact ([`super::ticks_to_ns`]): `tfdt` sets a track's decode time, a fragment without one
//!   continues from where the previous fragment of that track ended (decision D5), composition offsets
//!   are `trun` version 0 (unsigned) and 1 (signed), sample values come from the `trun`, else the `tfhd`,
//!   else the `trex` of the initialization segment.
//! * Where the sample data starts: `default-base-is-moof` and the legacy rule (the first `traf` is
//!   relative to the `moof`, the next ones to the end of the data of the previous) follow the standard.
//!   An explicit `base_data_offset` is an absolute offset in a file that does not exist here; the first
//!   one in a `moof` is taken to be the position of that `moof` and later ones relative to it (what
//!   ffmpeg's fragmenter without `default_base_moof` writes). Data before the end of its own `moof` is
//!   refused, which is how a wrong guess ends.
//! * Where this parser is more lenient than the registry (and than Ladybird): a fragment may use the legacy
//!   data offset rule, a `traf` without `tfdt` continues from the previous fragment (decision D5), the
//!   `ftyp` may be missing and unknown top-level boxes are skipped. Where it is as strict: a `moov` without
//!   `mvex`, or whose tracks already hold samples, is `Malformed` (a progressive file), and so is sample data
//!   inside the `moof` or running past its `mdat`. Samples with no data are dropped (their time passes).
//! * One track per kind (decision D3): a second video or audio track is `TooManyTracks`. Tracks that are
//!   neither (text, timecode, hint) are ignored, and so are their samples. A track whose codec decision D4
//!   leaves out (HEVC, AC-3, E-AC-3, MP3, FLAC, ...) is `Unsupported`. Edit lists are not applied.
//! * `pssh` boxes of the `moov` are copied out raw ([`InitSegment::pssh`]). Protected sample entries
//!   (`encv`/`enca`) are read through their `frma` as the codec they wrap and are not decrypted.

use crate::api::{
    ByteStreamParser, CodecConfig, CodedFrame, Container, InitSegment, Ns, ParseError, ParseEvent,
    TrackConfig, TrackKind,
};

use super::bmff::{BoxRef, Reader, boxes, find_path, full_box, peek_header, type_str};
use super::config::{
    aac_codec_string, av1_codec_string, avc_codec_string, dops_to_opus_head, language_from_mdhd,
    parse_asc, parse_esds, parse_opus_head, vp9_codec_string,
};
use super::{
    ByteQueue, FEED_SLICE, MAX_BOX_BYTES, MAX_RETAINED_BYTES, MAX_SAMPLES_PER_FRAGMENT, Output,
    PResult, frame_data, malformed, ticks_to_ns, unsupported,
};

/// `Sample::track` of a sample whose track the initialization segment does not list (it is dropped).
const NO_TRACK: usize = usize::MAX;

#[derive(Clone, Copy, Debug, Default)]
struct Trex {
    duration: u32,
    size: u32,
    flags: u32,
}

#[derive(Debug)]
struct Track {
    cfg: TrackConfig,
    trex: Trex,
    /// Where the next fragment of this track starts when it has no `tfdt`, in container ticks.
    next_dts: i64,
}

/// One sample of the fragment being delivered.
#[derive(Clone, Copy, Debug)]
struct Sample {
    /// Absolute stream offset of the first byte.
    off: u64,
    size: u32,
    track: usize,
    key: bool,
    pts: Ns,
    dts: Ns,
    dur: Ns,
}

#[derive(Debug, Default)]
struct Fragment {
    samples: Vec<Sample>,
    /// `floor[i]` is the smallest offset among `samples[i..]`: what must stay in the queue.
    floor: Vec<u64>,
    next: usize,
}

impl Fragment {
    fn new(samples: Vec<Sample>) -> Self {
        let mut floor = vec![0u64; samples.len()];
        let mut low = u64::MAX;
        for (i, s) in samples.iter().enumerate().rev() {
            low = low.min(s.off);
            floor[i] = low;
        }
        Fragment {
            samples,
            floor,
            next: 0,
        }
    }

    fn done(&self) -> bool {
        self.next >= self.samples.len()
    }

    fn floor(&self) -> Option<u64> {
        self.floor.get(self.next).copied()
    }
}

#[derive(Clone, Copy, Debug)]
enum State {
    /// Waiting for the header of the next top-level box at `pos`.
    Header,
    /// Waiting for all of a box that is read whole (`moov`, `moof`).
    Whole {
        typ: [u8; 4],
        start: u64,
        header_len: usize,
        end: u64,
    },
    /// Passing over the body of a box up to `end` (`None`: to the end of the stream).
    Skip { mdat: bool, end: Option<u64> },
}

/// The incremental fragmented MP4 parser. Use it through [`ByteStreamParser`] (or
/// `crate::parse::new_parser`).
#[derive(Debug)]
pub struct Mp4Parser {
    q: ByteQueue,
    /// The next byte the state machine has not accounted for (absolute offset).
    pos: u64,
    state: State,
    have_init: bool,
    seen_moof: bool,
    sniffed: bool,
    tracks: Vec<Track>,
    frag: Fragment,
    seg_open: bool,
    failed: Option<ParseError>,
}

impl Default for Mp4Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Mp4Parser {
    pub fn new() -> Self {
        Mp4Parser {
            q: ByteQueue::new(),
            pos: 0,
            state: State::Header,
            have_init: false,
            seen_moof: false,
            sniffed: false,
            tracks: Vec::new(),
            frag: Fragment::default(),
            seg_open: false,
            failed: None,
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

    /// Drops the partial segment: the bytes held, the fragment being delivered, the box in progress.
    fn drop_partial(&mut self) {
        let end = self.q.end();
        self.q.clear_to(end);
        self.pos = end;
        self.state = State::Header;
        self.frag = Fragment::default();
        self.seg_open = false;
    }

    fn pump(&mut self, out: &mut Output) -> PResult<()> {
        loop {
            let mut progressed = self.emit_samples(out)?;
            progressed |= self.maybe_segment_end(out);
            progressed |= self.step(out)?;
            self.trim();
            if self.q.len() > MAX_RETAINED_BYTES {
                return Err(malformed(
                    "sample data is out of order by more than the buffering limit allows",
                ));
            }
            if !progressed {
                return Ok(());
            }
        }
    }

    /// Emits the samples whose bytes are all here, in listed order. A sample is released only once the
    /// state machine has passed its last byte (`pos`), which keeps it inside the box that carries it and
    /// makes the result independent of how much input happens to be queued already.
    fn emit_samples(&mut self, out: &mut Output) -> PResult<bool> {
        let mut any = false;
        while let Some(&s) = self.frag.samples.get(self.frag.next) {
            let end = s.off + u64::from(s.size);
            if end > self.pos {
                break;
            }
            let Some(bytes) = self.q.slice(s.off, end) else {
                return Err(malformed("sample data that has already been passed over"));
            };
            let Some(track) = self.tracks.get(s.track) else {
                return Err(malformed("a sample of a track that no longer exists"));
            };
            out.frame(CodedFrame {
                kind: track.cfg.kind,
                track_id: track.cfg.track_id,
                pts: s.pts,
                dts: s.dts,
                duration: s.dur,
                keyframe: s.key,
                data: frame_data(bytes),
            });
            self.frag.next += 1;
            any = true;
        }
        Ok(any)
    }

    /// `SegmentEnd` once every sample is out and the `mdat` that carried them is over.
    fn maybe_segment_end(&mut self, out: &mut Output) -> bool {
        if !self.seg_open || !self.frag.done() {
            return false;
        }
        if let State::Skip {
            mdat: true,
            end: Some(end),
        } = self.state
            && self.pos < end
        {
            return false;
        }
        self.seg_open = false;
        self.frag = Fragment::default();
        out.segment_end();
        true
    }

    fn step(&mut self, out: &mut Output) -> PResult<bool> {
        match self.state {
            State::Header => self.step_header(),
            State::Whole { .. } => self.step_whole(out),
            State::Skip { mdat, end } => self.step_skip(mdat, end),
        }
    }

    fn step_header(&mut self) -> PResult<bool> {
        let avail = self.q.peek(self.pos, 16);
        if !self.sniffed {
            if avail.len() < 8 {
                return Ok(false);
            }
            if avail[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
                return Err(ParseError::WrongContainer(
                    "an EBML header: this is a WebM or Matroska stream, not an MP4".to_string(),
                ));
            }
            self.sniffed = true;
        }
        let Some(h) = peek_header(avail)? else {
            return Ok(false);
        };
        let start = self.pos;
        let name = type_str(&h.typ);
        match &h.typ {
            b"moov" | b"moof" => {
                let is_moof = &h.typ == b"moof";
                if is_moof && !self.have_init {
                    return Err(ParseError::MediaBeforeInit);
                }
                let size = bounded_size(&h.typ, h.size)?;
                if !self.frag.done() {
                    return Err(malformed(format!(
                        "a '{name}' box began before the sample data of the previous fragment was complete"
                    )));
                }
                self.state = State::Whole {
                    typ: h.typ,
                    start,
                    header_len: h.header_len,
                    end: start
                        .checked_add(size)
                        .ok_or_else(|| malformed("a box ends beyond the addressable stream"))?,
                };
            }
            b"mdat" => {
                if !self.have_init {
                    return Err(ParseError::MediaBeforeInit);
                }
                if !self.seen_moof {
                    return Err(malformed(
                        "an mdat box before any moof: this is not a fragmented MP4",
                    ));
                }
                let end =
                    match h.size {
                        Some(s) => Some(start.checked_add(s).ok_or_else(|| {
                            malformed("a box ends beyond the addressable stream")
                        })?),
                        None => None,
                    };
                self.pos = start + h.header_len as u64;
                self.state = State::Skip { mdat: true, end };
            }
            _ => {
                let size = bounded_size(&h.typ, h.size)?;
                self.pos = start + h.header_len as u64;
                self.state =
                    State::Skip {
                        mdat: false,
                        end: Some(start.checked_add(size).ok_or_else(|| {
                            malformed("a box ends beyond the addressable stream")
                        })?),
                    };
            }
        }
        Ok(true)
    }

    fn step_whole(&mut self, out: &mut Output) -> PResult<bool> {
        let State::Whole {
            typ,
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
            return Err(malformed("a box that was dropped before it was read"));
        };
        let payload = bytes.get(header_len..).unwrap_or(&[]);
        if &typ == b"moov" {
            let moov = parse_moov(payload)?;
            let init = InitSegment {
                container: Container::Mp4,
                tracks: moov.tracks.iter().map(|t| t.cfg.clone()).collect(),
                duration: moov.duration,
                pssh: moov.pssh,
            };
            self.tracks = moov.tracks;
            self.have_init = true;
            self.frag = Fragment::default();
            self.seg_open = false;
            out.init(init);
        } else {
            let samples = parse_moof(&mut self.tracks, payload, start, end)?;
            self.frag = Fragment::new(samples);
            self.seg_open = true;
            self.seen_moof = true;
        }
        self.pos = end;
        self.state = State::Header;
        Ok(true)
    }

    fn step_skip(&mut self, mdat: bool, end: Option<u64>) -> PResult<bool> {
        let target = match end {
            Some(e) => e.min(self.q.end()),
            None => self.q.end(),
        };
        let mut progressed = false;
        if target > self.pos {
            self.pos = target;
            progressed = true;
        }
        if let Some(e) = end
            && self.pos == e
        {
            // A sample that starts in this mdat has to end in it.
            if mdat
                && self.frag.samples[self.frag.next..]
                    .iter()
                    .any(|s| s.off < e && s.off + u64::from(s.size) > e)
            {
                return Err(malformed("a sample runs past the end of its mdat box"));
            }
            self.state = State::Header;
            progressed = true;
        }
        Ok(progressed)
    }

    /// Drops the bytes nothing needs any more.
    fn trim(&mut self) {
        let wanted = match self.state {
            State::Whole { start, .. } => start,
            State::Header | State::Skip { .. } => self.pos,
        };
        let from = self.frag.floor().map_or(wanted, |f| f.min(wanted));
        self.q.discard_before(from);
    }
}

/// The size of a box that is buffered or skipped, which may not be open-ended or larger than the limit.
fn bounded_size(typ: &[u8; 4], size: Option<u64>) -> PResult<u64> {
    let name = type_str(typ);
    let Some(size) = size else {
        return Err(malformed(format!(
            "a '{name}' box that extends to the end of the stream (only mdat may)"
        )));
    };
    if size > MAX_BOX_BYTES {
        return Err(malformed(format!(
            "a '{name}' box of {size} bytes is larger than the {} MiB limit",
            MAX_BOX_BYTES >> 20
        )));
    }
    Ok(size)
}

impl ByteStreamParser for Mp4Parser {
    fn container(&self) -> Container {
        Container::Mp4
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
// moov

struct Moov {
    tracks: Vec<Track>,
    duration: Option<Ns>,
    pssh: Vec<Vec<u8>>,
}

fn parse_moov(payload: &[u8]) -> PResult<Moov> {
    let mut movie_timescale = 0u32;
    let mut mvhd_duration: Option<u64> = None;
    let mut mehd_duration: Option<u64> = None;
    let mut tracks: Vec<Track> = Vec::new();
    let mut trexes: Vec<(u32, Trex)> = Vec::new();
    let mut pssh = Vec::new();
    let mut saw_trak = false;
    let mut saw_mvex = false;
    for b in boxes(payload) {
        let b = b?;
        match &b.typ {
            b"mvhd" => {
                let (version, _, rest) = full_box(b.payload())?;
                let mut r = Reader::new(rest);
                if version == 1 {
                    r.skip(16)?;
                    movie_timescale = r.u32()?;
                    mvhd_duration = Some(r.u64()?);
                } else {
                    r.skip(8)?;
                    movie_timescale = r.u32()?;
                    mvhd_duration = Some(u64::from(r.u32()?));
                }
            }
            b"trak" => {
                saw_trak = true;
                if let Some(t) = parse_trak(b.payload())? {
                    tracks.push(t);
                }
            }
            b"mvex" => {
                saw_mvex = true;
                for c in boxes(b.payload()) {
                    let c = c?;
                    if c.is(b"mehd") {
                        let (version, _, rest) = full_box(c.payload())?;
                        let mut r = Reader::new(rest);
                        mehd_duration = Some(if version == 1 {
                            r.u64()?
                        } else {
                            u64::from(r.u32()?)
                        });
                    } else if c.is(b"trex") {
                        let (_, _, rest) = full_box(c.payload())?;
                        let mut r = Reader::new(rest);
                        let id = r.u32()?;
                        r.skip(4)?; // default_sample_description_index
                        let trex = Trex {
                            duration: r.u32()?,
                            size: r.u32()?,
                            flags: r.u32()?,
                        };
                        trexes.push((id, trex));
                    }
                }
            }
            b"pssh" => pssh.push(b.whole.to_vec()),
            _ => {}
        }
    }
    if !saw_trak {
        return Err(malformed("a moov box without a trak"));
    }
    // The registry makes both of these append errors: a movie that is not marked as fragmented, and
    // tracks that already hold samples, are not initialization segments (a progressive MP4 file).
    if !saw_mvex {
        return Err(malformed(
            "a moov box without mvex: not the initialization segment of a fragmented MP4",
        ));
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
    if tracks.iter().any(|t| t.cfg.track_id == 0)
        || (tracks.len() == 2 && tracks[0].cfg.track_id == tracks[1].cfg.track_id)
    {
        return Err(malformed(
            "a track with id 0, or two tracks with the same id",
        ));
    }
    for t in &mut tracks {
        if let Some((_, trex)) = trexes.iter().find(|(id, _)| *id == t.cfg.track_id) {
            t.trex = *trex;
        }
    }
    // The declared duration: the fragment duration of `mehd` when there is one, else `mvhd`'s; 0 and the
    // all-ones value mean "not known" (a live or open-ended presentation).
    let duration_ticks = mehd_duration
        .or(mvhd_duration)
        .filter(|&d| d != 0 && d != u64::from(u32::MAX) && d != u64::MAX);
    let duration = match duration_ticks {
        Some(d) if movie_timescale != 0 => i64::try_from(d)
            .ok()
            .and_then(|d| ticks_to_ns(d, movie_timescale).ok()),
        _ => None,
    };
    Ok(Moov {
        tracks,
        duration,
        pssh,
    })
}

/// A `trak` as a track, or `None` for a kind of track this parser ignores (text, timecode, hint).
fn parse_trak(payload: &[u8]) -> PResult<Option<Track>> {
    let tkhd = find_path(payload, &[b"tkhd"])?.ok_or_else(|| malformed("a trak without a tkhd"))?;
    let (version, tkhd_flags, rest) = full_box(tkhd.payload())?;
    let mut r = Reader::new(rest);
    r.skip(if version == 1 { 16 } else { 8 })?;
    let track_id = r.u32()?;

    let mdia = find_path(payload, &[b"mdia"])?.ok_or_else(|| malformed("a trak without a mdia"))?;
    let mdhd =
        find_path(mdia.payload(), &[b"mdhd"])?.ok_or_else(|| malformed("a trak without a mdhd"))?;
    let (version, _, rest) = full_box(mdhd.payload())?;
    let mut r = Reader::new(rest);
    r.skip(if version == 1 { 16 } else { 8 })?;
    let timescale = r.u32()?;
    r.skip(if version == 1 { 8 } else { 4 })?;
    let language = language_from_mdhd(r.u16()?);
    if timescale == 0 {
        return Err(malformed("a track timescale of 0"));
    }

    let hdlr =
        find_path(mdia.payload(), &[b"hdlr"])?.ok_or_else(|| malformed("a trak without a hdlr"))?;
    let (_, _, rest) = full_box(hdlr.payload())?;
    let mut r = Reader::new(rest);
    r.skip(4)?;
    let handler: [u8; 4] = r
        .take(4)?
        .try_into()
        .map_err(|_| malformed("a handler type that is not 4 bytes"))?;
    let kind = match &handler {
        b"vide" => TrackKind::Video,
        b"soun" => TrackKind::Audio,
        _ => return Ok(None),
    };

    for table in [b"stts", b"stsc", b"stco", b"co64"] {
        if let Some(b) = find_path(mdia.payload(), &[b"minf", b"stbl", table])? {
            let (_, _, rest) = full_box(b.payload())?;
            if Reader::new(rest).u32()? != 0 {
                return Err(malformed(format!(
                    "track {track_id} already holds samples ('{}' has entries): not an initialization segment",
                    type_str(table)
                )));
            }
        }
    }

    let stsd = find_path(mdia.payload(), &[b"minf", b"stbl", b"stsd"])?
        .ok_or_else(|| malformed("a trak without a stsd"))?;
    let (_, _, rest) = full_box(stsd.payload())?;
    let mut r = Reader::new(rest);
    let count = r.u32()?;
    if count == 0 {
        return Err(malformed("a stsd without a sample entry"));
    }
    let entry = boxes(r.rest())
        .next()
        .ok_or_else(|| malformed("a stsd without a sample entry"))??;
    let e = parse_entry(kind, &entry)?;
    Ok(Some(Track {
        cfg: TrackConfig {
            kind,
            track_id,
            codec: e.codec,
            codec_string: e.codec_string,
            width: e.width,
            height: e.height,
            sample_rate: e.sample_rate,
            channels: e.channels,
            timescale,
            language,
            default: tkhd_flags & 1 != 0,
        },
        trex: Trex::default(),
        next_dts: 0,
    }))
}

struct Entry {
    codec: CodecConfig,
    codec_string: String,
    width: u32,
    height: u32,
    sample_rate: u32,
    channels: u32,
}

/// The first box of type `typ` among the children of a sample entry; a damaged tail is not an error.
fn entry_child<'a>(kids: &'a [u8], typ: &[u8; 4]) -> Option<BoxRef<'a>> {
    boxes(kids).map_while(Result::ok).find(|b| b.is(typ))
}

fn parse_entry(kind: TrackKind, entry: &BoxRef<'_>) -> PResult<Entry> {
    let p = entry.payload();
    let mut width = 0;
    let mut height = 0;
    let mut sample_rate = 0;
    let mut channels = 0;
    let children_at = match kind {
        TrackKind::Video => {
            let mut r = Reader::new(p);
            r.skip(24)?;
            width = u32::from(r.u16()?);
            height = u32::from(r.u16()?);
            78
        }
        _ => {
            let mut r = Reader::new(p);
            r.skip(8)?;
            let version = r.u16()?;
            r.skip(6)?;
            channels = u32::from(r.u16()?);
            r.skip(6)?;
            sample_rate = r.u32()? >> 16;
            // The QuickTime sound description versions 1 and 2 add 16 and 36 bytes.
            28 + match version {
                1 => 16,
                2 => 36,
                _ => 0,
            }
        }
    };
    let kids = p
        .get(children_at..)
        .ok_or_else(|| malformed("a sample entry shorter than its fixed fields"))?;

    // A protected entry (`encv`, `enca`) names the format it wraps in `sinf/frma`.
    let mut format = entry.typ;
    if matches!(&entry.typ, b"encv" | b"enca")
        && let Some(sinf) = entry_child(kids, b"sinf")
        && let Some(frma) = entry_child(sinf.payload(), b"frma")
        && let Some(f) = frma.payload().get(..4)
    {
        format = [f[0], f[1], f[2], f[3]];
    }
    let name = type_str(&format);
    let need = |typ: &[u8; 4]| {
        entry_child(kids, typ).ok_or_else(|| {
            malformed(format!(
                "a '{name}' sample entry without its '{}' box",
                type_str(typ)
            ))
        })
    };
    let wrong_handler = || {
        malformed(format!(
            "a '{name}' sample entry in the wrong kind of track"
        ))
    };

    let (codec, codec_string) = match &format {
        b"avc1" | b"avc3" => {
            if kind != TrackKind::Video {
                return Err(wrong_handler());
            }
            let avcc = need(b"avcC")?.payload();
            (
                CodecConfig::H264 {
                    avcc: avcc.to_vec(),
                },
                avc_codec_string(&format, avcc)?,
            )
        }
        b"vp09" => {
            if kind != TrackKind::Video {
                return Err(wrong_handler());
            }
            let (_, _, record) = full_box(need(b"vpcC")?.payload())?;
            (
                CodecConfig::Vp9 {
                    vpcc: Some(record.to_vec()),
                },
                vp9_codec_string(record)?,
            )
        }
        b"av01" => {
            if kind != TrackKind::Video {
                return Err(wrong_handler());
            }
            let av1c = need(b"av1C")?.payload();
            (
                CodecConfig::Av1 {
                    av1c: av1c.to_vec(),
                },
                av1_codec_string(av1c)?,
            )
        }
        b"mp4a" => {
            if kind != TrackKind::Audio {
                return Err(wrong_handler());
            }
            let esds = parse_esds(need(b"esds")?.payload())?;
            if esds.object_type != 0x40 {
                return Err(unsupported(format!(
                    "an mp4a track with object type {:#04x} (only MPEG-4 AAC is supported)",
                    esds.object_type
                )));
            }
            let asc = parse_asc(&esds.asc)?;
            if sample_rate == 0 {
                sample_rate = asc.sample_rate;
            }
            if channels == 0 {
                channels = asc.channels;
            }
            (CodecConfig::Aac { asc: esds.asc }, aac_codec_string(&asc))
        }
        b"Opus" => {
            if kind != TrackKind::Audio {
                return Err(wrong_handler());
            }
            let head = dops_to_opus_head(need(b"dOps")?.payload())?;
            let info = parse_opus_head(&head)?;
            channels = u32::from(info.channels);
            if sample_rate == 0 {
                sample_rate = 48_000;
            }
            (CodecConfig::Opus { head }, "opus".to_string())
        }
        b"hvc1" | b"hev1" | b"dvh1" | b"dvhe" => {
            return Err(unsupported("HEVC is not supported (decision D4)"));
        }
        b"ac-3" | b"ec-3" | b"ac-4" => {
            return Err(unsupported(format!(
                "{name} audio is not supported (decision D4; EA2 decides)"
            )));
        }
        _ => {
            return Err(unsupported(format!(
                "the sample entry '{name}' is not a codec this parser feeds"
            )));
        }
    };
    Ok(Entry {
        codec,
        codec_string,
        width,
        height,
        sample_rate,
        channels,
    })
}

// ----------------------------------------------------------------------------------------------
// moof

struct Tfhd {
    track_id: u32,
    base: Option<u64>,
    default_duration: Option<u32>,
    default_size: Option<u32>,
    default_flags: Option<u32>,
    default_base_is_moof: bool,
    duration_is_empty: bool,
}

fn parse_tfhd(payload: &[u8]) -> PResult<Tfhd> {
    let (_, flags, rest) = full_box(payload)?;
    let mut r = Reader::new(rest);
    let track_id = r.u32()?;
    let base = if flags & 0x01 != 0 {
        Some(r.u64()?)
    } else {
        None
    };
    if flags & 0x02 != 0 {
        r.skip(4)?; // sample_description_index
    }
    let default_duration = if flags & 0x08 != 0 {
        Some(r.u32()?)
    } else {
        None
    };
    let default_size = if flags & 0x10 != 0 {
        Some(r.u32()?)
    } else {
        None
    };
    let default_flags = if flags & 0x20 != 0 {
        Some(r.u32()?)
    } else {
        None
    };
    Ok(Tfhd {
        track_id,
        base,
        default_duration,
        default_size,
        default_flags,
        default_base_is_moof: flags & 0x2_0000 != 0,
        duration_is_empty: flags & 0x1_0000 != 0,
    })
}

fn parse_tfdt(payload: &[u8]) -> PResult<u64> {
    let (version, _, rest) = full_box(payload)?;
    let mut r = Reader::new(rest);
    if version == 1 {
        r.u64()
    } else {
        r.u32().map(u64::from)
    }
}

/// A sample is a sync sample unless it is marked non-sync or says it depends on other samples.
fn is_sync(flags: u32) -> bool {
    flags & 0x0001_0000 == 0 && (flags >> 24) & 0x3 != 1
}

fn offset_add(base: u64, delta: i32) -> PResult<u64> {
    let r = if delta >= 0 {
        base.checked_add(u64::from(delta as u32))
    } else {
        base.checked_sub(u64::from(delta.unsigned_abs()))
    };
    r.ok_or_else(|| malformed("a data offset that points outside the stream"))
}

/// What the `traf` boxes of one `moof` share.
struct MoofCtx {
    moof_start: u64,
    /// The first explicit `base_data_offset` seen: it stands for `moof_start`.
    first_explicit_base: Option<u64>,
    /// The end of the data of the previous `traf`: the base of the next one under the legacy rule.
    prev_end: u64,
}

fn parse_moof(
    tracks: &mut [Track],
    payload: &[u8],
    moof_start: u64,
    moof_end: u64,
) -> PResult<Vec<Sample>> {
    let mut ctx = MoofCtx {
        moof_start,
        first_explicit_base: None,
        prev_end: moof_start,
    };
    let mut samples: Vec<Sample> = Vec::new();
    for b in boxes(payload) {
        let b = b?;
        if b.is(b"traf") {
            parse_traf(tracks, b.payload(), &mut ctx, &mut samples)?;
        }
    }
    samples.retain(|s| s.track != NO_TRACK);
    if samples.iter().any(|s| s.off < moof_end) {
        return Err(malformed(
            "sample data that begins before the end of its own moof box",
        ));
    }
    Ok(samples)
}

fn parse_traf(
    tracks: &mut [Track],
    payload: &[u8],
    ctx: &mut MoofCtx,
    samples: &mut Vec<Sample>,
) -> PResult<()> {
    let mut tfhd: Option<Tfhd> = None;
    let mut tfdt: Option<u64> = None;
    let mut runs: Vec<BoxRef<'_>> = Vec::new();
    for b in boxes(payload) {
        let b = b?;
        match &b.typ {
            b"tfhd" => {
                if tfhd.replace(parse_tfhd(b.payload())?).is_some() {
                    return Err(malformed("a traf with two tfhd boxes"));
                }
            }
            b"tfdt" => tfdt = Some(parse_tfdt(b.payload())?),
            b"trun" => runs.push(b),
            _ => {}
        }
    }
    let tfhd = tfhd.ok_or_else(|| malformed("a traf without a tfhd"))?;
    let track_idx = tracks.iter().position(|t| t.cfg.track_id == tfhd.track_id);
    let (timescale, trex, track_kind) = match track_idx {
        Some(i) => (
            tracks[i].cfg.timescale,
            tracks[i].trex,
            Some(tracks[i].cfg.kind),
        ),
        None => (1, Trex::default(), None),
    };

    let base = if let Some(b) = tfhd.base {
        let first = *ctx.first_explicit_base.get_or_insert(b);
        if b >= first {
            ctx.moof_start.checked_add(b - first)
        } else {
            ctx.moof_start.checked_sub(first - b)
        }
        .ok_or_else(|| malformed("a base_data_offset that points outside the stream"))?
    } else if tfhd.default_base_is_moof {
        ctx.moof_start
    } else {
        ctx.prev_end
    };

    let mut dts_ticks: i64 = match (tfdt, track_idx) {
        (Some(t), _) => {
            i64::try_from(t).map_err(|_| malformed("a tfdt beyond the range of a timestamp"))?
        }
        (None, Some(i)) => tracks[i].next_dts,
        (None, None) => 0,
    };

    let mut cursor = base;
    if !tfhd.duration_is_empty {
        for run in runs {
            let (version, flags, rest) = full_box(run.payload())?;
            let mut r = Reader::new(rest);
            let count = r.u32()? as usize;
            let data_offset = if flags & 0x01 != 0 {
                Some(r.i32()?)
            } else {
                None
            };
            let first_flags = if flags & 0x04 != 0 {
                Some(r.u32()?)
            } else {
                None
            };
            let entry_size = 4
                * (usize::from(flags & 0x100 != 0)
                    + usize::from(flags & 0x200 != 0)
                    + usize::from(flags & 0x400 != 0)
                    + usize::from(flags & 0x800 != 0));
            if entry_size > 0 && count.saturating_mul(entry_size) > r.remaining() {
                return Err(malformed(
                    "a trun that lists more samples than its box holds",
                ));
            }
            if samples.len().saturating_add(count) > MAX_SAMPLES_PER_FRAGMENT {
                return Err(unsupported(format!(
                    "a fragment of more than {MAX_SAMPLES_PER_FRAGMENT} samples"
                )));
            }
            samples.reserve(count.min(r.remaining() / entry_size.max(1) + 1));
            let mut off = match data_offset {
                Some(d) => offset_add(base, d)?,
                None => cursor,
            };
            for i in 0..count {
                let duration = if flags & 0x100 != 0 {
                    r.u32()?
                } else {
                    tfhd.default_duration.unwrap_or(trex.duration)
                };
                let size = if flags & 0x200 != 0 {
                    r.u32()?
                } else {
                    tfhd.default_size.unwrap_or(trex.size)
                };
                let sample_flags = if flags & 0x400 != 0 {
                    Some(r.u32()?)
                } else {
                    None
                };
                let cto: i64 = if flags & 0x800 != 0 {
                    if version == 0 {
                        i64::from(r.u32()?)
                    } else {
                        i64::from(r.i32()?)
                    }
                } else {
                    0
                };
                if u64::from(size) > MAX_BOX_BYTES {
                    return Err(malformed(format!(
                        "a sample of {size} bytes is larger than the {} MiB limit",
                        MAX_BOX_BYTES >> 20
                    )));
                }
                let flags_of_sample = match (i, first_flags, sample_flags) {
                    (0, Some(f), _) => f,
                    (_, _, Some(f)) => f,
                    _ => tfhd.default_flags.unwrap_or(trex.flags),
                };
                let sample_end = off
                    .checked_add(u64::from(size))
                    .ok_or_else(|| malformed("sample data beyond the addressable stream"))?;
                if let Some(kind) = track_kind {
                    let end_ticks = dts_ticks
                        .checked_add(i64::from(duration))
                        .ok_or_else(|| malformed("a decode time out of range"))?;
                    let pts_ticks = dts_ticks
                        .checked_add(cto)
                        .ok_or_else(|| malformed("a presentation time out of range"))?;
                    let dts = ticks_to_ns(dts_ticks, timescale)?;
                    let dur = ticks_to_ns(end_ticks, timescale)? - dts;
                    // A sample with no data is no use to any codec: its time passes, nothing is emitted.
                    if size > 0 {
                        samples.push(Sample {
                            off,
                            size,
                            track: track_idx.unwrap_or(NO_TRACK),
                            key: kind == TrackKind::Audio || is_sync(flags_of_sample),
                            pts: ticks_to_ns(pts_ticks, timescale)?,
                            dts,
                            dur,
                        });
                    }
                    dts_ticks = end_ticks;
                }
                off = sample_end;
            }
            cursor = off;
        }
    }
    if let Some(i) = track_idx {
        tracks[i].next_dts = dts_ticks;
    }
    ctx.prev_end = cursor.max(base);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_flags() {
        assert!(is_sync(0));
        assert!(is_sync(0x0200_0000), "depends on nothing: an I frame");
        assert!(!is_sync(0x0101_0000), "non-sync and depends on others");
        assert!(!is_sync(0x0001_0000), "non-sync");
        assert!(!is_sync(0x0100_0000), "depends on others");
    }

    #[test]
    fn offsets_add_with_a_sign_and_refuse_to_leave_the_stream() {
        assert_eq!(offset_add(100, 8).unwrap(), 108);
        assert_eq!(offset_add(100, -8).unwrap(), 92);
        assert!(offset_add(4, -8).is_err());
        assert!(offset_add(u64::MAX, 1).is_err());
    }

    #[test]
    fn floors_are_suffix_minima() {
        let s = |off| Sample {
            off,
            size: 1,
            track: 0,
            key: true,
            pts: 0,
            dts: 0,
            dur: 0,
        };
        let f = Fragment::new(vec![s(50), s(10), s(30), s(40)]);
        assert_eq!(f.floor, vec![10, 10, 30, 40]);
        assert_eq!(f.floor(), Some(10));
        assert!(Fragment::default().done());
        assert_eq!(Fragment::default().floor(), None);
    }

    #[test]
    fn a_fresh_parser_waits_and_reports_its_container() {
        let mut p = Mp4Parser::new();
        assert_eq!(p.container(), Container::Mp4);
        assert!(p.push(&[]).unwrap().is_empty());
        assert!(p.push(&[0, 0, 0]).unwrap().is_empty());
        assert_eq!(p.retained_bytes(), 3);
    }
}
