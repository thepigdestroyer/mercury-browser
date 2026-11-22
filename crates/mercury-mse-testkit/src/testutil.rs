// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Test helpers: a small writer for synthetic fragmented MP4 streams and EBML elements, so that the
//! edge cases the committed fixtures do not have (64-bit and "to the end" boxes, `uuid` boxes,
//! version 0 `tfdt` overflow, `sidx` grouping, missing `tfdt`, unknown-size Clusters) are tested too.

use crate::bmff::{BoxRef, boxes, type_str};

pub fn bx(typ: &str, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    v.extend_from_slice(typ.as_bytes());
    v.extend_from_slice(payload);
    v
}

/// A box with the 64-bit size form (`size == 1`, then `largesize`).
pub fn bx64(typ: &str, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(typ.as_bytes());
    v.extend_from_slice(&((16 + payload.len()) as u64).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// A box whose size field is 0: it runs to the end of the data.
pub fn bx_to_end(typ: &str, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(typ.as_bytes());
    v.extend_from_slice(payload);
    v
}

pub fn bx_uuid(usertype: [u8; 16], payload: &[u8]) -> Vec<u8> {
    let mut p = usertype.to_vec();
    p.extend_from_slice(payload);
    bx("uuid", &p)
}

pub fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|p| p.iter().copied()).collect()
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    VideoAvc,
    AudioAac,
}

/// `ftyp` + `moov` with one track per entry of `tracks` (id, timescale, kind, trex default duration).
pub fn init_segment(tracks: &[(u32, u32, Kind, u32)]) -> Vec<u8> {
    let mut moov_payload = bx(
        "mvhd",
        &cat(&[&[0, 0, 0, 0], &[0; 8], &be32(1000), &be32(0), &[0; 80]]),
    );
    for &(id, timescale, kind, _) in tracks {
        let tkhd = bx("tkhd", &cat(&[&[0, 0, 0, 3], &[0; 8], &be32(id), &[0; 72]]));
        let mdhd = bx(
            "mdhd",
            &cat(&[
                &[0, 0, 0, 0],
                &[0; 8],
                &be32(timescale),
                &be32(0),
                &[0x55, 0xc4, 0, 0],
            ]),
        );
        let handler = if kind == Kind::VideoAvc {
            "vide"
        } else {
            "soun"
        };
        let hdlr = bx(
            "hdlr",
            &cat(&[&[0; 8], handler.as_bytes(), &[0; 12], b"x\0"]),
        );
        let entry = match kind {
            Kind::VideoAvc => {
                let avcc = bx("avcC", &[1, 0x64, 0x00, 0x1f, 0xff, 0xe0, 0]);
                bx(
                    "avc1",
                    &cat(&[
                        &[0; 6],
                        &be16(1),
                        &[0; 16],
                        &be16(640),
                        &be16(360),
                        &[0; 50],
                        &avcc,
                    ]),
                )
            }
            Kind::AudioAac => {
                let esds = bx(
                    "esds",
                    &[
                        0, 0, 0, 0, // version/flags
                        0x03, 0x19, 0, 1, 0, // ES_Descriptor, ES_ID 1, flags 0
                        0x04, 0x11, 0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, // DecoderConfig
                        0x05, 0x02, 0x11, 0x88, // AudioSpecificConfig: AAC-LC 48 kHz mono
                        0x06, 0x01, 0x02, // SLConfig
                    ],
                );
                bx(
                    "mp4a",
                    &cat(&[
                        &[0; 6],
                        &be16(1),
                        &[0; 8],
                        &be16(1),
                        &be16(16),
                        &[0; 4],
                        &be32(48000 << 16),
                        &esds,
                    ]),
                )
            }
        };
        let stsd = bx("stsd", &cat(&[&[0; 4], &be32(1), &entry]));
        let stbl = bx("stbl", &stsd);
        let minf = bx("minf", &stbl);
        let mdia = bx("mdia", &cat(&[&mdhd, &hdlr, &minf]));
        moov_payload.extend_from_slice(&bx("trak", &cat(&[&tkhd, &mdia])));
    }
    let mut mvex = Vec::new();
    for &(id, _, _, default_duration) in tracks {
        mvex.extend_from_slice(&bx(
            "trex",
            &cat(&[
                &[0; 4],
                &be32(id),
                &be32(1),
                &be32(default_duration),
                &be32(0),
                &be32(0),
            ]),
        ));
    }
    moov_payload.extend_from_slice(&bx("mvex", &mvex));
    cat(&[
        &bx("ftyp", b"iso6\0\0\0\0iso6cmfc"),
        &bx("moov", &moov_payload),
    ])
}

/// One `traf`: `samples` samples of `sample_size` bytes.
#[derive(Clone, Debug)]
pub struct Frag {
    pub track: u32,
    /// `Some((time, use_version_1))`, `None` for no `tfdt` at all.
    pub tfdt: Option<(u64, bool)>,
    /// Per-sample durations in the `trun`; `None` leaves the duration to `tfhd` or `trex`.
    pub durations: Option<Vec<u32>>,
    pub tfhd_default_duration: Option<u32>,
    pub samples: u32,
    pub sample_size: u32,
    /// `default-base-is-moof` (flag 0x20000) in the `tfhd`.
    pub base_is_moof: bool,
}

impl Frag {
    pub fn new(track: u32, tfdt: u64, samples: u32) -> Frag {
        Frag {
            track,
            tfdt: Some((tfdt, true)),
            durations: None,
            tfhd_default_duration: None,
            samples,
            sample_size: 16,
            base_is_moof: true,
        }
    }
}

fn traf(f: &Frag, data_offset: i32) -> Vec<u8> {
    let mut tfhd_flags = 0u32;
    if f.base_is_moof {
        tfhd_flags |= 0x02_0000;
    }
    let mut tfhd = vec![
        0,
        (tfhd_flags >> 16) as u8,
        (tfhd_flags >> 8) as u8,
        tfhd_flags as u8,
    ];
    tfhd.extend_from_slice(&be32(f.track));
    if let Some(d) = f.tfhd_default_duration {
        tfhd[3] |= 0x08;
        tfhd.extend_from_slice(&be32(d));
    }
    let mut out = bx("tfhd", &tfhd);
    if let Some((t, v1)) = f.tfdt {
        let payload = if v1 {
            cat(&[&[1, 0, 0, 0], &t.to_be_bytes()])
        } else {
            cat(&[&[0, 0, 0, 0], &(t as u32).to_be_bytes()])
        };
        out.extend_from_slice(&bx("tfdt", &payload));
    }
    let mut trun_flags = 0x01 | 0x200;
    if f.durations.is_some() {
        trun_flags |= 0x100;
    }
    let mut trun = vec![0, 0, (trun_flags >> 8) as u8, trun_flags as u8];
    trun.extend_from_slice(&be32(f.samples));
    trun.extend_from_slice(&data_offset.to_be_bytes());
    for i in 0..f.samples as usize {
        if let Some(d) = &f.durations {
            trun.extend_from_slice(&be32(d[i]));
        }
        trun.extend_from_slice(&be32(f.sample_size));
    }
    out.extend_from_slice(&bx("trun", &trun));
    bx("traf", &out)
}

/// `moof` + `mdat` for the given fragments, with correct `trun` data offsets.
pub fn moof_mdat(frags: &[Frag]) -> Vec<u8> {
    moof_mdat_opts(frags, false)
}

/// As `moof_mdat`, with the `moof` in the 64-bit size form when `wide_moof`.
pub fn moof_mdat_opts(frags: &[Frag], wide_moof: bool) -> Vec<u8> {
    let build = |offsets: &[i32]| -> Vec<u8> {
        let mut p = bx("mfhd", &cat(&[&[0; 4], &be32(1)]));
        for (f, o) in frags.iter().zip(offsets) {
            p.extend_from_slice(&traf(f, *o));
        }
        if wide_moof {
            bx64("moof", &p)
        } else {
            bx("moof", &p)
        }
    };
    let probe = build(&vec![0; frags.len()]);
    let mut offsets = Vec::new();
    let mut data = 0usize;
    for f in frags {
        offsets.push((probe.len() + 8 + data) as i32);
        data += (f.samples * f.sample_size) as usize;
    }
    let moof = build(&offsets);
    let mdat = bx("mdat", &vec![0xAB; data]);
    cat(&[&moof, &mdat])
}

/// A `sidx` (version 0) with one reference per `(size, duration)`, first_offset 0.
pub fn sidx(timescale: u32, earliest: u32, refs: &[(u32, u32)]) -> Vec<u8> {
    let mut p = cat(&[
        &[0; 4],
        &be32(1),
        &be32(timescale),
        &be32(earliest),
        &be32(0),
        &[0; 2],
    ]);
    p.extend_from_slice(&be16(refs.len() as u16));
    for &(size, dur) in refs {
        p.extend_from_slice(&be32(size));
        p.extend_from_slice(&be32(dur));
        p.extend_from_slice(&be32(0x9000_0000));
    }
    bx("sidx", &p)
}

/// The names of the top-level boxes: `ftyp moov`.
pub fn box_names(data: &[u8]) -> String {
    boxes(data, 0, data.len())
        .filter_map(Result::ok)
        .map(|b| type_str(&b.typ))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn first_box(data: &[u8], typ: &[u8; 4]) -> Option<BoxRef> {
    boxes(data, 0, data.len())
        .filter_map(Result::ok)
        .find(|b| b.is(typ))
}

/// Every box type that appears anywhere inside `data` (depth first, containers only for the
/// ones the splitter knows), as `moof/traf/tfdt` paths: lets a test assert the structure.
pub fn paths(data: &[u8]) -> Vec<String> {
    fn walk(data: &[u8], start: usize, end: usize, prefix: &str, out: &mut Vec<String>) {
        for b in boxes(data, start, end).filter_map(Result::ok) {
            let name = format!("{prefix}{}", type_str(&b.typ));
            out.push(name.clone());
            if matches!(&b.typ, b"moof" | b"traf" | b"moov" | b"mvex") {
                walk(data, b.payload, b.end, &format!("{name}/"), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(data, 0, data.len(), "", &mut out);
    out
}

// ---- EBML ----

pub fn ebml(id: u32, payload: &[u8]) -> Vec<u8> {
    let id_bytes = id.to_be_bytes();
    let skip = id_bytes.iter().take_while(|&&b| b == 0).count();
    let mut out = id_bytes[skip..].to_vec();
    out.extend_from_slice(&crate::ebml::write_size(payload.len() as u64, 1));
    out.extend_from_slice(payload);
    out
}

/// An element with the unknown-size marker (8-byte form) instead of the real size.
pub fn ebml_unknown(id: u32, payload: &[u8]) -> Vec<u8> {
    let id_bytes = id.to_be_bytes();
    let skip = id_bytes.iter().take_while(|&&b| b == 0).count();
    let mut out = id_bytes[skip..].to_vec();
    out.extend_from_slice(&[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    out.extend_from_slice(payload);
    out
}

pub fn ebml_uint(id: u32, v: u64) -> Vec<u8> {
    crate::ebml::write_uint_elem(id, v, 1)
}

/// A SimpleBlock for `track` at `rel` timecode units from the Cluster start.
pub fn simple_block(track: u8, rel: i16, frame: &[u8]) -> Vec<u8> {
    let mut p = vec![0x80 | track];
    p.extend_from_slice(&rel.to_be_bytes());
    p.push(0x80);
    p.extend_from_slice(frame);
    ebml(crate::ebml::ID_SIMPLE_BLOCK, &p)
}
