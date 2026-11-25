// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Shared helpers of the `parse_*` integration tests (lane med-1, item 1): feeding parsers under a chunk
//! plan, comparing what comes out, and writers for hand-built MP4 and WebM streams (the variants the
//! committed fixtures do not have). Not a test file itself: each `tests/parse_*.rs` has `mod parse_support;`.

#![allow(dead_code)]

use mercury_mse::api::{
    ByteStreamParser, CodedFrame, Container, InitSegment, Ns, ParseError, ParseEvent, TrackKind,
};
use mercury_mse::parse::new_parser;
use mercury_mse_testkit::{ChunkPlan, Container as KitContainer, SegmentedStream, chunks};

// ----------------------------------------------------------------------------------------------
// Feeding and comparing

/// The five chunk plans of the brief: the whole thing, two fixed sizes, an awkward mix, one byte at a time.
pub fn plans() -> Vec<ChunkPlan> {
    vec![
        ChunkPlan::Whole,
        ChunkPlan::Fixed(1024),
        ChunkPlan::Fixed(65536),
        ChunkPlan::Sizes(vec![13, 4096, 1]),
        ChunkPlan::Fixed(1),
    ]
}

pub fn container_of(c: KitContainer) -> Container {
    match c {
        KitContainer::Mp4 => Container::Mp4,
        KitContainer::WebM => Container::WebM,
    }
}

/// A frame with everything comparable (the real one holds an `Arc<[u8]>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fr {
    pub kind: TrackKind,
    pub track_id: u32,
    pub pts: Ns,
    pub dts: Ns,
    pub duration: Ns,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

impl From<&CodedFrame> for Fr {
    fn from(f: &CodedFrame) -> Fr {
        Fr {
            kind: f.kind,
            track_id: f.track_id,
            pts: f.pts,
            dts: f.dts,
            duration: f.duration,
            keyframe: f.keyframe,
            data: f.data.to_vec(),
        }
    }
}

/// What a parser said, with the frame batches flattened: the sequence is the same under any chunking.
#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    Init(InitSegment),
    Frame(Fr),
    End,
}

pub fn flatten(events: Vec<ParseEvent>) -> Vec<Ev> {
    let mut out = Vec::new();
    for e in events {
        match e {
            ParseEvent::Init(i) => out.push(Ev::Init(i)),
            ParseEvent::Frames(fs) => out.extend(fs.iter().map(|f| Ev::Frame(f.into()))),
            ParseEvent::SegmentEnd => out.push(Ev::End),
        }
    }
    out
}

/// Pushes `data` cut by `plan`; stops at the first error.
pub fn push_plan(
    parser: &mut dyn ByteStreamParser,
    data: &[u8],
    plan: &ChunkPlan,
) -> Result<Vec<Ev>, ParseError> {
    let mut out = Vec::new();
    for piece in chunks(data, plan) {
        out.extend(flatten(parser.push(piece)?));
    }
    Ok(out)
}

/// Pushes every piece of `parts` (each cut by `plan`) into one fresh parser.
pub fn parse_parts(
    container: Container,
    parts: &[&[u8]],
    plan: &ChunkPlan,
) -> Result<Vec<Ev>, ParseError> {
    let mut parser = new_parser(container);
    let mut out = Vec::new();
    for part in parts {
        out.extend(push_plan(parser.as_mut(), part, plan)?);
    }
    Ok(out)
}

/// A stream as a page appends it: the init segment, then each media segment, each in its own appends.
pub struct Fed {
    pub events: Vec<Ev>,
    /// The frames that came out while segment `i` was being appended.
    pub per_segment: Vec<Vec<Fr>>,
    /// The frames that came out while the init segment was being appended (always none).
    pub during_init: Vec<Fr>,
}

pub fn feed_stream(stream: &SegmentedStream, plan: &ChunkPlan) -> Result<Fed, ParseError> {
    let mut parser = new_parser(container_of(stream.container));
    let mut events = push_plan(parser.as_mut(), &stream.init.bytes, plan)?;
    let during_init: Vec<Fr> = events
        .iter()
        .filter_map(|e| {
            if let Ev::Frame(f) = e {
                Some(f.clone())
            } else {
                None
            }
        })
        .collect();
    let mut per_segment = Vec::new();
    for seg in &stream.media {
        let ev = push_plan(parser.as_mut(), &seg.bytes, plan)?;
        per_segment.push(
            ev.iter()
                .filter_map(|e| {
                    if let Ev::Frame(f) = e {
                        Some(f.clone())
                    } else {
                        None
                    }
                })
                .collect(),
        );
        events.extend(ev);
    }
    Ok(Fed {
        events,
        per_segment,
        during_init,
    })
}

pub fn init_of(events: &[Ev]) -> Vec<&InitSegment> {
    events
        .iter()
        .filter_map(|e| if let Ev::Init(i) = e { Some(i) } else { None })
        .collect()
}

pub fn frames_of(events: &[Ev]) -> Vec<Fr> {
    events
        .iter()
        .filter_map(|e| {
            if let Ev::Frame(f) = e {
                Some(f.clone())
            } else {
                None
            }
        })
        .collect()
}

pub fn ends_of(events: &[Ev]) -> usize {
    events.iter().filter(|e| matches!(e, Ev::End)).count()
}

/// The frames of one track, in order.
pub fn track_frames(frames: &[Fr], kind: TrackKind) -> Vec<Fr> {
    frames.iter().filter(|f| f.kind == kind).cloned().collect()
}

/// Nanoseconds as milliseconds, rounded to the nearest (what the testkit's segment times are).
pub fn ns_to_ms(ns: Ns) -> i64 {
    (ns + 500_000).div_euclid(1_000_000)
}

// ----------------------------------------------------------------------------------------------
// A deterministic generator for fuzzing and payloads

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next() >> 24) as u8).collect()
    }
}

// ----------------------------------------------------------------------------------------------
// ISO BMFF writers

pub fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|p| p.iter().copied()).collect()
}

pub fn bx(typ: &str, payload: &[u8]) -> Vec<u8> {
    let mut v = ((8 + payload.len()) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(typ.as_bytes());
    v.extend_from_slice(payload);
    v
}

/// A box with the 64-bit size form.
pub fn bx64(typ: &str, payload: &[u8]) -> Vec<u8> {
    let mut v = 1u32.to_be_bytes().to_vec();
    v.extend_from_slice(typ.as_bytes());
    v.extend_from_slice(&((16 + payload.len()) as u64).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// A box whose size field is 0 ("to the end").
pub fn bx_to_end(typ: &str, payload: &[u8]) -> Vec<u8> {
    let mut v = 0u32.to_be_bytes().to_vec();
    v.extend_from_slice(typ.as_bytes());
    v.extend_from_slice(payload);
    v
}

pub fn full(typ: &str, version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![version];
    p.extend_from_slice(&flags.to_be_bytes()[1..]);
    p.extend_from_slice(payload);
    bx(typ, &p)
}

pub fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// The `avcC` record of the `avc1.42C01E` sample entry used by the hand-built streams.
pub fn avcc() -> Vec<u8> {
    vec![
        1, 0x42, 0xC0, 0x1E, 0xFF, 0xE1, 0, 4, 0x67, 0x42, 0xC0, 0x1E, 1, 0, 2, 0x68, 0xCE,
    ]
}

/// A visual sample entry with the given child boxes.
pub fn visual_entry(typ: &str, width: u16, height: u16, children: &[u8]) -> Vec<u8> {
    let mut p = vec![0; 6];
    p.extend_from_slice(&1u16.to_be_bytes()); // data reference index
    p.extend_from_slice(&[0; 16]);
    p.extend_from_slice(&width.to_be_bytes());
    p.extend_from_slice(&height.to_be_bytes());
    p.extend_from_slice(&[0, 0x48, 0, 0, 0, 0x48, 0, 0]); // resolutions
    p.extend_from_slice(&[0; 4]);
    p.extend_from_slice(&1u16.to_be_bytes()); // frame count
    p.extend_from_slice(&[0; 32]); // compressor name
    p.extend_from_slice(&0x18u16.to_be_bytes()); // depth
    p.extend_from_slice(&0xffffu16.to_be_bytes()); // pre-defined
    assert_eq!(p.len(), 78);
    p.extend_from_slice(children);
    bx(typ, &p)
}

pub fn avc1_entry() -> Vec<u8> {
    visual_entry("avc1", 320, 180, &bx("avcC", &avcc()))
}

/// An audio sample entry (version 0) with the given child boxes.
pub fn audio_entry(typ: &str, channels: u16, rate: u32, children: &[u8]) -> Vec<u8> {
    let mut p = vec![0; 6];
    p.extend_from_slice(&1u16.to_be_bytes());
    p.extend_from_slice(&[0; 8]); // version, revision, vendor
    p.extend_from_slice(&channels.to_be_bytes());
    p.extend_from_slice(&16u16.to_be_bytes());
    p.extend_from_slice(&[0; 4]);
    p.extend_from_slice(&(rate << 16).to_be_bytes());
    assert_eq!(p.len(), 28);
    p.extend_from_slice(children);
    bx(typ, &p)
}

/// The `esds` of an AAC-LC 48 kHz mono stream.
pub fn esds_aac() -> Vec<u8> {
    let asc = [0x11, 0x88];
    let mut dcd = vec![0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    dcd.extend_from_slice(&[0x05, asc.len() as u8]);
    dcd.extend_from_slice(&asc);
    let mut es = vec![0, 1, 0, 0x04, dcd.len() as u8];
    es.extend_from_slice(&dcd);
    let mut p = vec![0x03, es.len() as u8];
    p.extend_from_slice(&es);
    full("esds", 0, 0, &p)
}

pub fn mp4a_entry() -> Vec<u8> {
    audio_entry("mp4a", 1, 48_000, &esds_aac())
}

/// One track of a hand-built initialization segment.
#[derive(Clone)]
pub struct TrackSpec {
    pub id: u32,
    pub handler: &'static str,
    pub timescale: u32,
    /// The whole sample entry box.
    pub entry: Vec<u8>,
}

impl TrackSpec {
    pub fn video(id: u32, timescale: u32) -> Self {
        TrackSpec {
            id,
            handler: "vide",
            timescale,
            entry: avc1_entry(),
        }
    }

    pub fn audio(id: u32, timescale: u32) -> Self {
        TrackSpec {
            id,
            handler: "soun",
            timescale,
            entry: mp4a_entry(),
        }
    }
}

pub fn trak(t: &TrackSpec) -> Vec<u8> {
    let tkhd = full("tkhd", 0, 3, &cat(&[&[0; 8], &be32(t.id), &[0; 72]]));
    let mdhd = full(
        "mdhd",
        0,
        0,
        &cat(&[&[0; 8], &be32(t.timescale), &be32(0), &[0x55, 0xc4, 0, 0]]),
    );
    let hdlr = full(
        "hdlr",
        0,
        0,
        &cat(&[&[0; 4], t.handler.as_bytes(), &[0; 12], b"h\0"]),
    );
    let stsd = full("stsd", 0, 0, &cat(&[&be32(1), &t.entry]));
    let stbl = bx(
        "stbl",
        &cat(&[
            &stsd,
            &full("stts", 0, 0, &be32(0)),
            &full("stsc", 0, 0, &be32(0)),
            &full("stsz", 0, 0, &[0; 8]),
            &full("stco", 0, 0, &be32(0)),
        ]),
    );
    let mdia = bx("mdia", &cat(&[&mdhd, &hdlr, &bx("minf", &stbl)]));
    bx("trak", &cat(&[&tkhd, &mdia]))
}

/// `trex` defaults: (track id, duration, size, flags).
pub type TrexSpec = (u32, u32, u32, u32);

pub fn moov_of(tracks: &[TrackSpec], trex: &[TrexSpec], extra: &[Vec<u8>]) -> Vec<u8> {
    let mvhd = full(
        "mvhd",
        0,
        0,
        &cat(&[&[0; 8], &be32(1000), &be32(0), &[0; 80]]),
    );
    let mut payload = mvhd;
    for t in tracks {
        payload.extend_from_slice(&trak(t));
    }
    let mut mvex = Vec::new();
    for &(id, dur, size, flags) in trex {
        mvex.extend_from_slice(&full(
            "trex",
            0,
            0,
            &cat(&[&be32(id), &be32(1), &be32(dur), &be32(size), &be32(flags)]),
        ));
    }
    payload.extend_from_slice(&bx("mvex", &mvex));
    for e in extra {
        payload.extend_from_slice(e);
    }
    bx("moov", &payload)
}

pub fn ftyp() -> Vec<u8> {
    bx("ftyp", &cat(&[b"iso6", &be32(0), b"iso6cmfc"]))
}

/// `ftyp` + `moov` for the tracks, no `trex`.
pub fn init_of_tracks(tracks: &[TrackSpec]) -> Vec<u8> {
    cat(&[&ftyp(), &moov_of(tracks, &[], &[])])
}

/// `tfhd` with the optional fields in their places. `flags` names which are present.
#[derive(Clone, Default)]
pub struct Tfhd {
    pub track_id: u32,
    pub base_data_offset: Option<u64>,
    pub default_duration: Option<u32>,
    pub default_size: Option<u32>,
    pub default_flags: Option<u32>,
    pub default_base_is_moof: bool,
    pub duration_is_empty: bool,
}

pub fn tfhd(t: &Tfhd) -> Vec<u8> {
    let mut flags = 0u32;
    let mut p = be32(t.track_id).to_vec();
    if let Some(b) = t.base_data_offset {
        flags |= 0x01;
        p.extend_from_slice(&b.to_be_bytes());
    }
    if let Some(d) = t.default_duration {
        flags |= 0x08;
        p.extend_from_slice(&be32(d));
    }
    if let Some(s) = t.default_size {
        flags |= 0x10;
        p.extend_from_slice(&be32(s));
    }
    if let Some(f) = t.default_flags {
        flags |= 0x20;
        p.extend_from_slice(&be32(f));
    }
    if t.duration_is_empty {
        flags |= 0x1_0000;
    }
    if t.default_base_is_moof {
        flags |= 0x2_0000;
    }
    full("tfhd", 0, flags, &p)
}

pub fn tfdt(version: u8, t: u64) -> Vec<u8> {
    if version == 1 {
        full("tfdt", 1, 0, &t.to_be_bytes())
    } else {
        full("tfdt", 0, 0, &be32(t as u32))
    }
}

/// One sample of a `trun`; a field is written only when the run's flags say so.
#[derive(Clone, Copy, Default)]
pub struct RunSample {
    pub duration: u32,
    pub size: u32,
    pub flags: u32,
    pub cto: i32,
}

/// A `trun`: which fields it has is decided by the `Some`s and the flags below.
#[derive(Clone, Default)]
pub struct Run {
    pub version: u8,
    pub data_offset: Option<i32>,
    pub first_sample_flags: Option<u32>,
    pub has_duration: bool,
    pub has_size: bool,
    pub has_flags: bool,
    pub has_cto: bool,
    pub samples: Vec<RunSample>,
}

pub fn trun(r: &Run) -> Vec<u8> {
    let mut flags = 0u32;
    let mut p = be32(r.samples.len() as u32).to_vec();
    if let Some(o) = r.data_offset {
        flags |= 0x01;
        p.extend_from_slice(&o.to_be_bytes());
    }
    if let Some(f) = r.first_sample_flags {
        flags |= 0x04;
        p.extend_from_slice(&be32(f));
    }
    flags |= if r.has_duration { 0x100 } else { 0 };
    flags |= if r.has_size { 0x200 } else { 0 };
    flags |= if r.has_flags { 0x400 } else { 0 };
    flags |= if r.has_cto { 0x800 } else { 0 };
    for s in &r.samples {
        if r.has_duration {
            p.extend_from_slice(&be32(s.duration));
        }
        if r.has_size {
            p.extend_from_slice(&be32(s.size));
        }
        if r.has_flags {
            p.extend_from_slice(&be32(s.flags));
        }
        if r.has_cto {
            p.extend_from_slice(&s.cto.to_be_bytes());
        }
    }
    full("trun", r.version, flags, &p)
}

pub fn traf(parts: &[Vec<u8>]) -> Vec<u8> {
    bx("traf", &parts.concat())
}

pub fn moof(trafs: &[Vec<u8>]) -> Vec<u8> {
    bx(
        "moof",
        &cat(&[&full("mfhd", 0, 0, &be32(1)), &trafs.concat()]),
    )
}

/// A recognisable payload of `n` bytes whose first byte is `tag`.
pub fn payload(tag: u8, n: usize) -> Vec<u8> {
    (0..n).map(|i| tag.wrapping_add(i as u8)).collect()
}

/// A media segment without `styp`: a `moof` built by `make_trafs(offset)`, where `offset` is the distance
/// from the start of the `moof` to the first byte of `data` (so that `trun` data offsets can point into
/// the `mdat`), followed by an `mdat` with `data` (`mdat_box` writes the box: `bx`, `bx64`, `bx_to_end`).
pub fn fragment_with(
    make_trafs: impl Fn(u32) -> Vec<Vec<u8>>,
    mdat_box: fn(&str, &[u8]) -> Vec<u8>,
    data: &[u8],
) -> Vec<u8> {
    let mdat = mdat_box("mdat", data);
    let header = (mdat.len() - data.len()) as u32;
    let probe = moof(&make_trafs(0));
    let moof_bytes = moof(&make_trafs(probe.len() as u32 + header));
    assert_eq!(
        moof_bytes.len(),
        probe.len(),
        "the moof size must not depend on data offsets"
    );
    cat(&[&moof_bytes, &mdat])
}

/// [`fragment_with`] and a plain `mdat`.
pub fn fragment(make_trafs: impl Fn(u32) -> Vec<Vec<u8>>, data: &[u8]) -> Vec<u8> {
    fragment_with(make_trafs, bx, data)
}

// ----------------------------------------------------------------------------------------------
// EBML writers

/// An element ID as bytes (without leading zero bytes).
pub fn id_bytes(id: u32) -> Vec<u8> {
    let b = id.to_be_bytes();
    let skip = b.iter().take_while(|&&x| x == 0).count();
    b[skip..].to_vec()
}

/// A size field of `value` in the shortest form.
pub fn size_bytes(value: u64) -> Vec<u8> {
    let mut len = 1;
    while len < 8 && value >= (1u64 << (7 * len)) - 1 {
        len += 1;
    }
    (value | (1u64 << (7 * len))).to_be_bytes()[8 - len..].to_vec()
}

pub fn elem(id: u32, payload: &[u8]) -> Vec<u8> {
    cat(&[&id_bytes(id), &size_bytes(payload.len() as u64), payload])
}

/// An element with the unknown-size marker (one byte, 0xFF).
pub fn elem_unknown(id: u32, payload: &[u8]) -> Vec<u8> {
    cat(&[&id_bytes(id), &[0xff], payload])
}

pub fn uint_elem(id: u32, v: u64) -> Vec<u8> {
    let n = (8 - v.leading_zeros() as usize / 8).max(1);
    elem(id, &v.to_be_bytes()[8 - n..])
}

pub fn float_elem(id: u32, v: f64) -> Vec<u8> {
    elem(id, &v.to_be_bytes())
}

pub const EBML: u32 = 0x1A45_DFA3;
pub const SEGMENT: u32 = 0x1853_8067;
pub const INFO: u32 = 0x1549_A966;
pub const TRACKS: u32 = 0x1654_AE6B;
pub const CLUSTER: u32 = 0x1F43_B675;
pub const CUES: u32 = 0x1C53_BB6B;
pub const TRACK_ENTRY: u32 = 0xAE;
pub const SIMPLE_BLOCK: u32 = 0xA3;
pub const BLOCK_GROUP: u32 = 0xA0;
pub const BLOCK: u32 = 0xA1;

pub fn ebml_header(doc_type: &str) -> Vec<u8> {
    elem(
        EBML,
        &cat(&[
            &uint_elem(0x4286, 1),
            &uint_elem(0x42F7, 1),
            &uint_elem(0x42F2, 4),
            &uint_elem(0x42F3, 8),
            &elem(0x4282, doc_type.as_bytes()),
            &uint_elem(0x4287, 4),
            &uint_elem(0x4285, 2),
        ]),
    )
}

pub fn info_elem(scale: u64, duration: Option<f64>) -> Vec<u8> {
    let mut p = uint_elem(0x2A_D7B1, scale);
    if let Some(d) = duration {
        p.extend_from_slice(&float_elem(0x4489, d));
    }
    elem(INFO, &p)
}

/// A `TrackEntry` for a VP9 video track (no `CodecPrivate`, as libvpx's muxer writes it).
pub fn vp9_track(number: u64, default_duration_ns: Option<u64>) -> Vec<u8> {
    let mut p = cat(&[
        &uint_elem(0xD7, number),
        &uint_elem(0x73C5, number),
        &uint_elem(0x83, 1),
        &elem(0x86, b"V_VP9"),
    ]);
    if let Some(d) = default_duration_ns {
        p.extend_from_slice(&uint_elem(0x23_E383, d));
    }
    p.extend_from_slice(&elem(
        0xE0,
        &cat(&[&uint_elem(0xB0, 320), &uint_elem(0xBA, 180)]),
    ));
    elem(TRACK_ENTRY, &p)
}

pub fn vp8_track(number: u64) -> Vec<u8> {
    elem(
        TRACK_ENTRY,
        &cat(&[
            &uint_elem(0xD7, number),
            &uint_elem(0x83, 1),
            &elem(0x86, b"V_VP8"),
        ]),
    )
}

pub fn opus_head(channels: u8) -> Vec<u8> {
    let mut h = b"OpusHead".to_vec();
    h.extend_from_slice(&[1, channels]);
    h.extend_from_slice(&312u16.to_le_bytes());
    h.extend_from_slice(&48_000u32.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes());
    h.push(0);
    h
}

pub fn opus_track(number: u64, default_duration_ns: Option<u64>) -> Vec<u8> {
    let mut p = cat(&[
        &uint_elem(0xD7, number),
        &uint_elem(0x83, 2),
        &elem(0x86, b"A_OPUS"),
        &elem(0x63A2, &opus_head(2)),
    ]);
    if let Some(d) = default_duration_ns {
        p.extend_from_slice(&uint_elem(0x23_E383, d));
    }
    p.extend_from_slice(&elem(
        0xE1,
        &cat(&[&float_elem(0xB5, 48_000.0), &uint_elem(0x9F, 2)]),
    ));
    elem(TRACK_ENTRY, &p)
}

pub fn tracks_elem(entries: &[Vec<u8>]) -> Vec<u8> {
    elem(TRACKS, &entries.concat())
}

/// EBML header, an unknown-size Segment, `Info` (1 ms) and `Tracks`.
pub fn webm_init(entries: &[Vec<u8>]) -> Vec<u8> {
    cat(&[
        &ebml_header("webm"),
        &elem_unknown(SEGMENT, &[]),
        &info_elem(1_000_000, Some(8000.0)),
        &tracks_elem(entries),
    ])
}

/// A `SimpleBlock` of `track` at `rel` (relative to the Cluster timecode).
pub fn simple_block(track: u64, rel: i16, key: bool, data: &[u8]) -> Vec<u8> {
    let mut p = vec![0x80 | track as u8];
    p.extend_from_slice(&rel.to_be_bytes());
    p.push(if key { 0x80 } else { 0 });
    p.extend_from_slice(data);
    elem(SIMPLE_BLOCK, &p)
}

/// A `BlockGroup` with a `Block`, optionally a `BlockDuration` and a `ReferenceBlock`.
pub fn block_group(
    track: u64,
    rel: i16,
    data: &[u8],
    duration: Option<u64>,
    reference: Option<i64>,
) -> Vec<u8> {
    let mut p = vec![0x80 | track as u8];
    p.extend_from_slice(&rel.to_be_bytes());
    p.push(0);
    p.extend_from_slice(data);
    let mut g = elem(BLOCK, &p);
    if let Some(d) = duration {
        g.extend_from_slice(&uint_elem(0x9B, d));
    }
    if let Some(r) = reference {
        g.extend_from_slice(&elem(0xFB, &(r as i32).to_be_bytes()));
    }
    elem(BLOCK_GROUP, &g)
}

/// A Cluster of known size.
pub fn cluster(timecode: u64, blocks: &[Vec<u8>]) -> Vec<u8> {
    elem(
        CLUSTER,
        &cat(&[&uint_elem(0xE7, timecode), &blocks.concat()]),
    )
}

/// A Cluster of unknown size.
pub fn cluster_unknown(timecode: u64, blocks: &[Vec<u8>]) -> Vec<u8> {
    elem_unknown(
        CLUSTER,
        &cat(&[&uint_elem(0xE7, timecode), &blocks.concat()]),
    )
}

// ----------------------------------------------------------------------------------------------
// Comparing

/// Asserts two event sequences are the same and, if not, says where they first differ (the sequences
/// hold the whole of a fixture's bytes, so a plain `assert_eq!` would print megabytes).
pub fn assert_same(what: &str, a: &[Ev], b: &[Ev]) {
    if a == b {
        return;
    }
    let at = a.iter().zip(b.iter()).position(|(x, y)| x != y);
    match at {
        Some(i) => {
            let show = |e: &Ev| match e {
                Ev::Frame(f) => format!(
                    "Frame {{ kind {:?}, pts {}, dts {}, dur {}, key {}, {} bytes }}",
                    f.kind,
                    f.pts,
                    f.dts,
                    f.duration,
                    f.keyframe,
                    f.data.len()
                ),
                other => format!("{other:?}"),
            };
            panic!(
                "{what}: the event sequences differ at event {i} of {} and {}: {} versus {}",
                a.len(),
                b.len(),
                show(&a[i]),
                show(&b[i])
            );
        }
        None => panic!(
            "{what}: one sequence is a prefix of the other ({} and {} events)",
            a.len(),
            b.len()
        ),
    }
}

/// The sub-strings of the `codecs` parameter of a type string.
pub fn codecs_of(mime: &str) -> Vec<String> {
    let at = mime.find("codecs=\"").expect("a quoted codecs parameter") + "codecs=\"".len();
    mime[at..]
        .split('"')
        .next()
        .unwrap()
        .split(',')
        .map(|s| s.trim().to_string())
        .collect()
}

/// Parses `parts` (one parser, parts pushed in order) under every chunk plan and asserts the result,
/// events or error, is the same for all of them; returns it.
pub fn same_under_all_plans(container: Container, parts: &[&[u8]]) -> Result<Vec<Ev>, ParseError> {
    let reference = parse_parts(container, parts, &ChunkPlan::Whole);
    for plan in plans() {
        let got = parse_parts(container, parts, &plan);
        match (&reference, &got) {
            (Ok(a), Ok(b)) => assert_same(&format!("{plan:?}"), a, b),
            (Err(a), Err(b)) => assert_eq!(a, b, "the error under {plan:?}"),
            (a, b) => panic!("{plan:?}: {a:?} versus {b:?}"),
        }
    }
    reference
}

/// The `ParseError` of a result, panicking (at the caller's line) if it parsed.
#[track_caller]
pub fn error_of(r: Result<Vec<Ev>, ParseError>) -> ParseError {
    match r {
        Err(e) => e,
        Ok(ev) => panic!("expected an error, got {} events", ev.len()),
    }
}

/// A random chunking: a list of sizes from one byte to a few thousand, the last repeated.
pub fn random_plan(rng: &mut Rng) -> ChunkPlan {
    const CHOICES: [usize; 10] = [1, 1, 2, 3, 7, 13, 64, 500, 4096, 30_000];
    let n = 1 + rng.below(40);
    ChunkPlan::Sizes((0..n).map(|_| CHOICES[rng.below(CHOICES.len())]).collect())
}

/// The ten committed fixtures, loaded.
pub fn all_streams() -> Vec<(&'static str, SegmentedStream)> {
    mercury_mse_testkit::FIXTURES
        .iter()
        .map(|f| (f.name, mercury_mse_testkit::load(f.name).unwrap()))
        .collect()
}
