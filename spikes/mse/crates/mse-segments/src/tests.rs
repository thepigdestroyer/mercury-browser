// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Tests over the committed fixtures and over synthetic streams (the edge cases the fixtures do not
//! have). Everything here is pure Rust and finishes in seconds.

use std::collections::HashMap;

use crate::bmff::{BoxRef, boxes, children, find_child, type_str};
use crate::testutil::{
    Frag, Kind, box_names, bx, bx_to_end, bx_uuid, bx64, cat, ebml, ebml_uint, ebml_unknown,
    first_box, init_segment, moof_mdat, moof_mdat_opts, paths, sidx, simple_block,
};
use crate::{
    ChunkPlan, Container, FIXTURES, Segment, SegmentKind, SegmentedStream, SplitError, chunks,
    fixtures_dir, load, read_fixture, sha256_hex, shift_media_time, split, split_mp4, split_webm,
    tile, with_pssh,
};

// ---------------------------------------------------------------------------------------------
// The committed fixtures

fn all() -> Vec<(&'static str, SegmentedStream)> {
    FIXTURES
        .iter()
        .map(|f| (f.name, load(f.name).expect(f.name)))
        .collect()
}

#[test]
fn fixtures_match_sha256sums() {
    let sums = std::fs::read_to_string(fixtures_dir().join("SHA256SUMS")).expect("SHA256SUMS");
    let mut listed: HashMap<String, String> = HashMap::new();
    for line in sums.lines().filter(|l| !l.trim().is_empty()) {
        let (hash, path) = line.split_once("  ").expect("`hash  path` lines");
        listed.insert(path.to_string(), hash.to_string());
    }
    let mut total = 0;
    for f in FIXTURES {
        let data = read_fixture(f.name).unwrap();
        total += data.len();
        assert_eq!(
            listed.get(f.path).map(String::as_str),
            Some(sha256_hex(&data).as_str()),
            "{} differs from SHA256SUMS",
            f.path
        );
    }
    assert_eq!(
        listed.len(),
        FIXTURES.len(),
        "SHA256SUMS lists a file that is not in FIXTURES"
    );
    assert!(
        total <= 1_500_000,
        "the fixtures are {total} bytes (budget 1.5 MB)"
    );
}

#[test]
fn every_fixture_name_is_unique_and_loads() {
    let mut names: Vec<_> = FIXTURES.iter().map(|f| f.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), FIXTURES.len());
    assert!(matches!(load("nope"), Err(SplitError::UnknownFixture(_))));
    assert_eq!(all().len(), 9);
}

#[test]
fn the_table_describes_the_files() {
    for f in FIXTURES {
        let s = load(f.name).unwrap();
        assert_eq!(s.container, f.container, "{}", f.name);
        assert_eq!(
            s.mime, f.mime,
            "{}: the derived type string differs from FIXTURES",
            f.name
        );
        assert_eq!(s.duration_ms(), Some(f.duration_ms), "{}", f.name);
        if f.width > 0 {
            assert_eq!(s.video_size(), Some((f.width, f.height)), "{}", f.name);
        }
        assert!(f.path.starts_with(if f.container == Container::Mp4 {
            "fmp4/"
        } else {
            "webm/"
        }));
    }
}

/// (start, duration) of each media segment.
fn timing(s: &SegmentedStream) -> Vec<(u64, u64)> {
    s.media
        .iter()
        .map(|m| (m.start_ms.unwrap(), m.duration_ms.unwrap()))
        .collect()
}

#[test]
fn video_fixtures_have_four_two_second_segments() {
    for name in [
        "h264-low",
        "h264-high",
        "h264-bframes",
        "vp9-low",
        "vp9-high",
        "av1-low",
    ] {
        let s = load(name).unwrap();
        assert_eq!(
            timing(&s),
            [(0, 2000), (2000, 2000), (4000, 2000), (6000, 2000)],
            "{name}"
        );
    }
}

#[test]
fn renditions_have_identical_segment_boundaries() {
    let (l, h) = (load("h264-low").unwrap(), load("h264-high").unwrap());
    assert_eq!(timing(&l), timing(&h));
    let (l, h) = (load("vp9-low").unwrap(), load("vp9-high").unwrap());
    assert_eq!(timing(&l), timing(&h));
    // The B-frame file shares the boundaries of the 640x360 baseline one too.
    assert_eq!(
        timing(&load("h264-bframes").unwrap()),
        timing(&load("h264-high").unwrap())
    );
}

#[test]
fn audio_fixtures_have_the_documented_segments() {
    // AAC frames are 1024 samples: 94 of them make a 2.005 s fragment.
    assert_eq!(
        timing(&load("aac").unwrap()),
        [(0, 2005), (2005, 2006), (4011, 2005), (6016, 1984)]
    );
    // webmmux starts a Cluster every 500 ms in an audio-only file.
    let opus = load("opus").unwrap();
    let want: Vec<(u64, u64)> = (0..16).map(|i| (i * 500, 500)).collect();
    assert_eq!(timing(&opus), want);
}

#[test]
fn the_muxed_fixture_has_four_fragments_with_both_tracks() {
    let s = load("h264-aac").unwrap();
    assert_eq!(s.media.len(), 4);
    assert_eq!(s.mime, "video/mp4; codecs=\"avc1.42C00C, mp4a.40.2\"");
    let (start, end) = s.span_ms().unwrap();
    assert_eq!((start, end), (0, 8000));
    for (i, m) in s.media.iter().enumerate() {
        // Each moof carries a traf for the video and one for the audio.
        let moof = first_box(&m.bytes, b"moof").unwrap();
        let trafs = children(&m.bytes, &moof)
            .unwrap()
            .into_iter()
            .filter(|b| b.is(b"traf"))
            .count();
        assert_eq!(trafs, 2, "fragment {i}");
        let near = |a: u64, b: u64| a.abs_diff(b) <= 40;
        assert!(
            near(m.start_ms.unwrap(), i as u64 * 2000),
            "fragment {i} starts at {:?}",
            m.start_ms
        );
    }
}

fn top_names(data: &[u8]) -> Vec<String> {
    boxes(data, 0, data.len())
        .map(|b| type_str(&b.unwrap().typ))
        .collect()
}

#[test]
fn mp4_init_segments_hold_ftyp_and_moov_only() {
    for name in ["h264-low", "h264-high", "aac", "h264-aac", "h264-bframes"] {
        let s = load(name).unwrap();
        assert_eq!(top_names(&s.init.bytes), ["ftyp", "moov"], "{name}");
        let moov = first_box(&s.init.bytes, b"moov").unwrap();
        let kids: Vec<String> = children(&s.init.bytes, &moov)
            .unwrap()
            .iter()
            .map(|b| type_str(&b.typ))
            .collect();
        let tracks = if name == "h264-aac" { 2 } else { 1 };
        assert_eq!(
            kids.iter().filter(|k| *k == "trak").count(),
            tracks,
            "{name}: {kids:?}"
        );
        assert!(kids.contains(&"mvex".to_string()), "{name}: {kids:?}");
        assert!(s.init.bytes.len() < 1300, "{name}");
        assert_eq!(
            (s.init.kind, s.init.start_ms, s.init.duration_ms),
            (SegmentKind::Init, None, None)
        );
        for m in &s.media {
            let names = top_names(&m.bytes);
            assert_eq!(
                &names[names.len() - 2..],
                ["moof", "mdat"],
                "{name}: {names:?}"
            );
            assert_eq!(m.kind, SegmentKind::Media);
        }
    }
}

#[test]
fn webm_init_segments_stop_before_the_first_cluster() {
    for name in ["vp9-low", "vp9-high", "opus", "av1-low"] {
        let s = load(name).unwrap();
        assert!(
            s.init.bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
            "{name}"
        );
        // The Segment has the unknown-size marker (streamable muxing).
        let seg = s
            .init
            .bytes
            .windows(9)
            .position(|w| w[..4] == [0x18, 0x53, 0x80, 0x67]);
        let seg = seg.unwrap_or_else(|| panic!("{name}: no Segment element"));
        assert_eq!(
            &s.init.bytes[seg + 4..seg + 12],
            &[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
            "{name}"
        );
        // The Info (0x1549A966) and Tracks (0x1654AE6B) elements are in it, no Cluster is.
        let has = |id: [u8; 4]| s.init.bytes.windows(4).any(|w| w == id);
        assert!(has([0x15, 0x49, 0xa9, 0x66]), "{name}");
        assert!(has([0x16, 0x54, 0xae, 0x6b]), "{name}");
        for m in &s.media {
            assert!(
                m.bytes.starts_with(&[0x1f, 0x43, 0xb6, 0x75]),
                "{name}: a Cluster per segment"
            );
        }
    }
}

#[test]
fn init_plus_media_is_the_file() {
    for f in FIXTURES {
        let data = read_fixture(f.name).unwrap();
        assert_eq!(load(f.name).unwrap().concat(), data, "{}", f.name);
    }
}

#[test]
fn the_bframe_fixture_really_has_composition_offsets() {
    let s = load("h264-bframes").unwrap();
    let mut with_offsets = 0;
    let mut reordered = 0;
    for m in &s.media {
        let moof = first_box(&m.bytes, b"moof").unwrap();
        for traf in children(&m.bytes, &moof)
            .unwrap()
            .iter()
            .filter(|b| b.is(b"traf"))
        {
            for trun in children(&m.bytes, traf)
                .unwrap()
                .iter()
                .filter(|b| b.is(b"trun"))
            {
                let p = trun.payload_of(&m.bytes);
                let flags = u32::from_be_bytes([p[0], p[1], p[2], p[3]]) & 0xff_ffff;
                if flags & 0x800 != 0 {
                    with_offsets += 1;
                    // Walk the samples: a non-zero offset means presentation order differs from decode order.
                    let count = u32::from_be_bytes([p[4], p[5], p[6], p[7]]) as usize;
                    let mut pos =
                        8 + if flags & 1 != 0 { 4 } else { 0 } + if flags & 4 != 0 { 4 } else { 0 };
                    let fields = [0x100, 0x200, 0x400, 0x800]
                        .iter()
                        .filter(|b| flags & **b != 0)
                        .count();
                    let cts_at = (fields - 1) * 4;
                    for _ in 0..count {
                        let off = i32::from_be_bytes([
                            p[pos + cts_at],
                            p[pos + cts_at + 1],
                            p[pos + cts_at + 2],
                            p[pos + cts_at + 3],
                        ]);
                        if off != 0 {
                            reordered += 1;
                        }
                        pos += fields * 4;
                    }
                }
            }
        }
    }
    assert!(
        with_offsets > 0,
        "no trun carries composition time offsets: the file has no B-frames"
    );
    assert!(reordered > 0, "all composition offsets are zero");
}

#[test]
fn every_fixture_segment_survives_every_chunk_plan() {
    for (name, s) in all() {
        for seg in std::iter::once(&s.init).chain(&s.media) {
            for plan in [
                ChunkPlan::Whole,
                ChunkPlan::Fixed(1024),
                ChunkPlan::Fixed(65_536),
                ChunkPlan::Sizes(vec![13, 4096, 1]),
            ] {
                let parts = chunks(&seg.bytes, &plan);
                let joined: Vec<u8> = parts.iter().flat_map(|p| p.iter().copied()).collect();
                assert_eq!(joined, seg.bytes, "{name} {plan:?}");
            }
        }
    }
    // The cut that C04 uses on the initialisation segment: 7 bytes in.
    let s = load("h264-low").unwrap();
    let parts = chunks(&s.init.bytes, &ChunkPlan::Sizes(vec![7, 65_536]));
    assert_eq!(parts[0].len(), 7);
    assert_eq!(parts.len(), 2);
}

// ---------------------------------------------------------------------------------------------
// shift_media_time

fn assert_same_segments(a: &SegmentedStream, b: &SegmentedStream) {
    assert_eq!(a.mime, b.mime);
    assert_eq!(a.media.len(), b.media.len());
    for (x, y) in a.media.iter().zip(&b.media) {
        assert_eq!((x.start_ms, x.duration_ms), (y.start_ms, y.duration_ms));
    }
}

#[test]
fn shifting_and_splitting_again_gives_the_same_starts() {
    for (name, original) in all() {
        for delta in [1, 8000, 1_234_567, 400_000_000] {
            let mut s = original.clone();
            shift_media_time(&mut s, delta).unwrap_or_else(|e| panic!("{name} +{delta}: {e}"));
            // The stored start_ms is what a fresh split of the rewritten bytes says.
            let again = split(s.container, &s.concat(), "").unwrap();
            assert_same_segments(&s, &again);
            for (new, old) in s.media.iter().zip(&original.media) {
                let moved = new.start_ms.unwrap() as i64 - old.start_ms.unwrap() as i64;
                assert!(
                    (moved - delta).abs() <= 1,
                    "{name} +{delta}: moved by {moved}"
                );
                assert_eq!(
                    new.duration_ms, old.duration_ms,
                    "{name}: durations are unchanged"
                );
            }
            // And back again.
            shift_media_time(&mut s, -delta).unwrap();
            let back = split(s.container, &s.concat(), "").unwrap();
            assert_same_segments(&back, &original);
        }
    }
}

#[test]
fn a_huge_shift_upgrades_version_0_tfdt_and_keeps_the_boxes_consistent() {
    // 100 days: 8.64e9 ms is 7.8e11 ticks at 90 kHz, far beyond 32 bits.
    let delta = 100 * 86_400 * 1000;
    for name in ["h264-low", "aac", "h264-aac", "h264-bframes"] {
        let original = load(name).unwrap();
        let mut s = original.clone();
        shift_media_time(&mut s, delta).unwrap();
        for (new, old) in s.media.iter().zip(&original.media) {
            assert_eq!(
                new.bytes.len(),
                old.bytes.len() + 4 * trafs(&old.bytes),
                "{name}: 4 bytes per tfdt"
            );
            check_moved(&old.bytes, &new.bytes);
            assert_eq!(first_tfdt_version(&new.bytes), 1, "{name}");
            assert_eq!(
                first_tfdt_version(&old.bytes),
                0,
                "{name}: the fixture has version 0 tfdt"
            );
        }
        let again = split(Container::Mp4, &s.concat(), "").unwrap();
        assert_same_segments(&s, &again);
        assert!(s.media[0].start_ms.unwrap() >= delta as u64 - 1);
    }
}

fn trafs(seg: &[u8]) -> usize {
    paths(seg)
        .iter()
        .filter(|p| p.as_str() == "moof/traf")
        .count()
}

fn first_tfdt_version(seg: &[u8]) -> u8 {
    let moof = first_box(seg, b"moof").unwrap();
    let traf = find_child(seg, &moof, b"traf").unwrap().unwrap();
    let tfdt = find_child(seg, &traf, b"tfdt").unwrap().unwrap();
    tfdt.payload_of(seg)[0]
}

/// Every box size is the sum of its children, the boxes are contiguous, and every `trun` data offset (they
/// are relative to the `moof`, with `default-base-is-moof`) points inside the `mdat` payload.
fn check_structure(seg: &[u8]) {
    let tops: Vec<BoxRef> = boxes(seg, 0, seg.len()).map(Result::unwrap).collect();
    let mut covered = 0;
    for b in &tops {
        assert_eq!(b.start, covered, "boxes are contiguous");
        covered = b.end;
    }
    assert_eq!(covered, seg.len(), "the boxes cover the segment");
    let moof = tops.iter().find(|b| b.is(b"moof")).unwrap();
    let mdat = tops.iter().find(|b| b.is(b"mdat")).unwrap();
    let kids_total: usize = children(seg, moof).unwrap().iter().map(|k| k.len()).sum();
    assert_eq!(
        moof.header_len() + kids_total,
        moof.len(),
        "moof size = header + children"
    );
    for traf in children(seg, moof)
        .unwrap()
        .iter()
        .filter(|b| b.is(b"traf"))
    {
        let kids = children(seg, traf).unwrap();
        let size: usize = kids.iter().map(|k| k.len()).sum();
        assert_eq!(
            traf.header_len() + size,
            traf.len(),
            "traf size = header + children"
        );
    }
    for (_, off) in trun_offsets(seg) {
        let at = moof.start as i64 + i64::from(off);
        assert!(
            at >= mdat.payload as i64 && at <= mdat.end as i64,
            "data_offset {off} is outside the mdat"
        );
    }
}

/// (start of the moof, data_offset) of every `trun` of the segment, in order.
fn trun_offsets(seg: &[u8]) -> Vec<(usize, i32)> {
    let moof = first_box(seg, b"moof").unwrap();
    let mut out = Vec::new();
    for traf in children(seg, &moof)
        .unwrap()
        .iter()
        .filter(|b| b.is(b"traf"))
    {
        for trun in children(seg, traf)
            .unwrap()
            .iter()
            .filter(|b| b.is(b"trun"))
        {
            let p = trun.payload_of(seg);
            let flags = u32::from_be_bytes([p[0], p[1], p[2], p[3]]) & 0xff_ffff;
            if flags & 1 != 0 {
                out.push((moof.start, i32::from_be_bytes([p[8], p[9], p[10], p[11]])));
            }
        }
    }
    out
}

/// A shifted segment: every data offset moved by exactly the growth of the moof.
fn check_moved(old: &[u8], new: &[u8]) {
    check_structure(new);
    let grown = first_box(new, b"moof").unwrap().len() as i64
        - first_box(old, b"moof").unwrap().len() as i64;
    let (o, n) = (trun_offsets(old), trun_offsets(new));
    assert_eq!(o.len(), n.len());
    assert!(!o.is_empty());
    for ((_, a), (_, b)) in o.iter().zip(&n) {
        assert_eq!(
            i64::from(*b) - i64::from(*a),
            grown,
            "data_offset moves with the moof"
        );
    }
}

/// For the synthetic streams (one run of samples per traf, in order): the offsets point exactly at the
/// samples of their traf.
fn check_consistent(seg: &[u8]) {
    let tops: Vec<BoxRef> = boxes(seg, 0, seg.len()).map(Result::unwrap).collect();
    let mut covered = 0;
    for b in &tops {
        assert_eq!(b.start, covered, "boxes are contiguous");
        covered = b.end;
    }
    assert_eq!(covered, seg.len(), "the boxes cover the segment");
    let moof = tops.iter().find(|b| b.is(b"moof")).unwrap();
    let mdat = tops.iter().find(|b| b.is(b"mdat")).unwrap();
    let mut data_pos = mdat.payload as i64;
    for traf in children(seg, moof)
        .unwrap()
        .iter()
        .filter(|b| b.is(b"traf"))
    {
        let kids = children(seg, traf).unwrap();
        let mut size = 0;
        for k in &kids {
            size += k.len();
        }
        assert_eq!(
            traf.header_len() + size,
            traf.len(),
            "traf size = header + children"
        );
        for trun in kids.iter().filter(|b| b.is(b"trun")) {
            let p = trun.payload_of(seg);
            let flags = u32::from_be_bytes([p[0], p[1], p[2], p[3]]) & 0xff_ffff;
            let count = u32::from_be_bytes([p[4], p[5], p[6], p[7]]) as usize;
            assert!(flags & 1 != 0, "the fixtures write a data offset");
            let off = i32::from_be_bytes([p[8], p[9], p[10], p[11]]);
            assert_eq!(
                moof.start as i64 + i64::from(off),
                data_pos,
                "trun data_offset points at its samples"
            );
            let fields = [0x100, 0x200, 0x400, 0x800]
                .iter()
                .filter(|b| flags & **b != 0)
                .count();
            let mut at = 8 + 4 + if flags & 4 != 0 { 4 } else { 0 };
            if flags & 0x200 != 0 {
                let size_at = if flags & 0x100 != 0 { 4 } else { 0 };
                for _ in 0..count {
                    let sz = u32::from_be_bytes([
                        p[at + size_at],
                        p[at + size_at + 1],
                        p[at + size_at + 2],
                        p[at + size_at + 3],
                    ]);
                    data_pos += i64::from(sz);
                    at += fields * 4;
                }
            }
        }
    }
    assert_eq!(data_pos, mdat.end as i64, "the samples fill the mdat");
    let kids_total: usize = children(seg, moof).unwrap().iter().map(|k| k.len()).sum();
    assert_eq!(
        moof.header_len() + kids_total,
        moof.len(),
        "moof size = header + children"
    );
}

#[test]
fn shifting_before_zero_is_an_error_and_changes_nothing() {
    for name in ["h264-low", "vp9-low"] {
        let original = load(name).unwrap();
        let mut s = original.clone();
        let err = shift_media_time(&mut s, -2001).unwrap_err();
        assert!(matches!(err, SplitError::Malformed(_)), "{name}: {err}");
        assert_eq!(
            s.concat(),
            original.concat(),
            "{name}: unchanged after the error"
        );
        // Exactly to zero is fine for the second segment (2000 ms) but not the first one: all or nothing.
        assert!(shift_media_time(&mut s, -2000).is_err());
    }
}

#[test]
fn webm_timecodes_grow_in_the_fixtures() {
    let original = load("vp9-low").unwrap();
    let mut s = original.clone();
    shift_media_time(&mut s, 5_000_000).unwrap(); // 5,000 s: the timecode needs 3 bytes
    for (new, old) in s.media.iter().zip(&original.media) {
        assert!(
            new.bytes.len() > old.bytes.len(),
            "the Timecode element is longer"
        );
        assert!(new.bytes.len() <= old.bytes.len() + 3);
    }
    let again = split_webm(&s.concat(), "").unwrap();
    assert_same_segments(&s, &again);
}

// ---------------------------------------------------------------------------------------------
// tile

#[test]
fn tile_repeats_the_segments_with_shifted_times() {
    for (name, original) in all() {
        let span = original.duration_ms().unwrap();
        let t = tile(&original, 5).unwrap();
        assert_eq!(t.media.len(), original.media.len() * 5, "{name}");
        assert_eq!(
            t.init.bytes, original.init.bytes,
            "{name}: the init segment is not repeated"
        );
        assert_eq!(t.mime, original.mime);
        assert_eq!(t.duration_ms(), Some(span * 5), "{name}");
        // Starts increase, copies are `span` apart and the duration of each is unchanged.
        let per = original.media.len();
        for (i, m) in t.media.iter().enumerate() {
            let (k, j) = (i / per, i % per);
            let expect = original.media[j].start_ms.unwrap() + k as u64 * span;
            assert!(
                m.start_ms.unwrap().abs_diff(expect) <= 1,
                "{name}: segment {i}"
            );
            assert_eq!(m.duration_ms, original.media[j].duration_ms, "{name}");
        }
        // It splits again from bytes.
        let again = split(t.container, &t.concat(), "").unwrap();
        assert_same_segments(&t, &again);
    }
}

#[test]
fn tile_edge_cases() {
    let s = load("h264-low").unwrap();
    assert!(tile(&s, 0).unwrap().media.is_empty());
    assert_same_segments(&tile(&s, 1).unwrap(), &s);
    let ten_minutes = tile(&s, 75).unwrap();
    assert_eq!(ten_minutes.media.len(), 300);
    assert_eq!(ten_minutes.duration_ms(), Some(600_000));
}

// ---------------------------------------------------------------------------------------------
// with_pssh

const WIDEVINE: [u8; 16] = [
    0xed, 0xef, 0x8b, 0xa9, 0x79, 0xd6, 0x4a, 0xce, 0xa3, 0xc8, 0x27, 0xdc, 0xd5, 0x1d, 0x21, 0xed,
];

#[test]
fn with_pssh_adds_a_box_at_the_end_of_moov() {
    for name in ["h264-low", "aac", "h264-aac", "h264-bframes"] {
        let s = load(name).unwrap();
        let data = [1u8, 2, 3, 4, 5];
        let init = with_pssh(&s.init, WIDEVINE, &data).unwrap();
        assert_eq!(
            init.bytes.len(),
            s.init.bytes.len() + 8 + 4 + 16 + 4 + data.len(),
            "{name}"
        );
        assert_eq!(top_names(&init.bytes), ["ftyp", "moov"], "{name}");
        let moov = first_box(&init.bytes, b"moov").unwrap();
        let old_moov = first_box(&s.init.bytes, b"moov").unwrap();
        assert_eq!(
            moov.len(),
            old_moov.len() + 37,
            "{name}: moov size is fixed up"
        );
        assert_eq!(
            moov.end,
            init.bytes.len(),
            "{name}: moov still ends the segment"
        );
        let kids = children(&init.bytes, &moov).unwrap();
        let last = kids.last().unwrap();
        assert!(last.is(b"pssh"), "{name}: pssh is the last child of moov");
        let p = last.payload_of(&init.bytes);
        assert_eq!(&p[..4], &[0, 0, 0, 0], "version 0, no flags");
        assert_eq!(&p[4..20], &WIDEVINE);
        assert_eq!(&p[20..24], &5u32.to_be_bytes());
        assert_eq!(&p[24..], &data);
        // The stream with that init segment still splits, with the same type and segments.
        let mut stream = s.clone();
        stream.init = init;
        let again = split(Container::Mp4, &stream.concat(), "").unwrap();
        assert_same_segments(&again, &s);
        // And twice: a second system ID.
        let twice = with_pssh(&stream.init, [7; 16], &[]).unwrap();
        stream.init = twice;
        assert_same_segments(&split(Container::Mp4, &stream.concat(), "").unwrap(), &s);
    }
}

#[test]
fn with_pssh_refuses_webm_and_garbage() {
    let w = load("vp9-low").unwrap();
    assert!(matches!(
        with_pssh(&w.init, WIDEVINE, &[1]),
        Err(SplitError::Unsupported(_))
    ));
    let junk = Segment {
        kind: SegmentKind::Init,
        bytes: vec![1, 2, 3],
        start_ms: None,
        duration_ms: None,
    };
    assert!(with_pssh(&junk, WIDEVINE, &[1]).is_err());
}

// ---------------------------------------------------------------------------------------------
// Damaged input

/// A deterministic generator (xorshift64*), so that "random" input is the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn truncated_files_give_errors_never_panics() {
    for f in FIXTURES {
        let data = read_fixture(f.name).unwrap();
        let mut cuts: Vec<usize> = (0..data.len()).step_by(data.len() / 53 + 1).collect();
        cuts.extend([1, 3, 4, 7, 8, 11, 12, 15, 16, 31, 32, data.len() - 1]);
        for cut in cuts {
            let r = split(f.container, &data[..cut], "");
            // A cut that lands exactly between two segments is a shorter, valid stream.
            if let Ok(s) = &r {
                assert!(
                    s.media.len() < 16 && s.concat().len() <= cut,
                    "{} cut at {cut}",
                    f.name
                );
            }
        }
        // A cut inside the last segment is an error.
        let mid_last = data.len() - 10;
        assert!(
            split(f.container, &data[..mid_last], "").is_err(),
            "{}",
            f.name
        );
        assert!(split(f.container, &[], "").is_err());
    }
}

#[test]
fn garbage_gives_errors_never_panics_or_loops() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for len in [0usize, 1, 2, 3, 7, 8, 9, 15, 16, 17, 100, 1000, 5000] {
        for _ in 0..20 {
            let junk: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            let _ = split_mp4(&junk, "");
            let _ = split_webm(&junk, "");
        }
    }
    // Input that looks like the start of each container but then is garbage.
    let mut mp4 = bx("ftyp", b"iso6");
    mp4.extend((0..300).map(|_| rng.next() as u8));
    assert!(split_mp4(&mp4, "").is_err());
    let mut webm = vec![0x1a, 0x45, 0xdf, 0xa3, 0x80];
    webm.extend((0..300).map(|_| rng.next() as u8));
    assert!(split_webm(&webm, "").is_err());
    assert!(split_mp4(&[0u8; 64], "").is_err());
    assert!(split_webm(&[0xffu8; 64], "").is_err());
}

#[test]
fn damaged_fixtures_never_panic() {
    let mut rng = Rng(0x0dd_ba11);
    for f in FIXTURES {
        let data = read_fixture(f.name).unwrap();
        // Damage only the first part (init segment and first fragment) so that the walk is long.
        let limit = data.len().min(6000);
        for _ in 0..150 {
            let mut d = data.clone();
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(limit);
                d[at] = rng.next() as u8;
            }
            if let Ok(s) = split(f.container, &d, "") {
                // Whatever it makes of it must be usable by the rewriting functions without a panic.
                let mut s = s;
                let _ = shift_media_time(&mut s, 12345);
                let _ = tile(&s, 2);
            }
            let _ = with_pssh(
                &Segment {
                    kind: SegmentKind::Init,
                    bytes: d[..limit.min(d.len())].to_vec(),
                    start_ms: None,
                    duration_ms: None,
                },
                WIDEVINE,
                &[9],
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Synthetic MP4 streams: what the committed files do not have

fn video_init(default_duration: u32) -> Vec<u8> {
    init_segment(&[(1, 1000, Kind::VideoAvc, default_duration)])
}

fn media(s: &SegmentedStream) -> Vec<(u64, Option<u64>, usize)> {
    s.media
        .iter()
        .map(|m| (m.start_ms.unwrap(), m.duration_ms, m.bytes.len()))
        .collect()
}

#[test]
fn each_moof_mdat_pair_is_its_own_segment_without_a_sidx() {
    let mut f = Frag::new(1, 0, 4);
    f.durations = Some(vec![40; 4]);
    let a = moof_mdat(std::slice::from_ref(&f));
    f.tfdt = Some((160, true));
    let b = moof_mdat(std::slice::from_ref(&f));
    f.tfdt = Some((320, true));
    let c = moof_mdat(&[f]);
    let data = cat(&[&video_init(40), &a, &b, &c]);
    let s = split_mp4(&data, "").unwrap();
    assert_eq!(
        media(&s),
        [
            (0, Some(160), a.len()),
            (160, Some(160), b.len()),
            (320, Some(160), c.len())
        ]
    );
    assert_eq!(s.mime, "video/mp4; codecs=\"avc1.64001F\"");
}

#[test]
fn a_styp_always_starts_a_segment_and_belongs_to_it() {
    let f = Frag::new(1, 0, 2);
    let a = cat(&[
        &bx("styp", b"msdh\0\0\0\0msdh"),
        &moof_mdat(std::slice::from_ref(&f)),
    ]);
    let mut g = f;
    g.tfdt = Some((80, true));
    let b = cat(&[&bx("styp", b"msdh\0\0\0\0msdh"), &moof_mdat(&[g])]);
    let s = split_mp4(&cat(&[&video_init(40), &a, &b]), "").unwrap();
    assert_eq!(s.media.len(), 2);
    assert_eq!(s.media[0].bytes, a);
    assert_eq!(s.media[1].bytes, b);
    assert_eq!(s.media[1].start_ms, Some(80));
}

#[test]
fn a_sidx_groups_the_fragments_it_indexes_into_one_segment() {
    let mut f = Frag::new(1, 0, 2);
    f.durations = Some(vec![50, 50]);
    let m1 = moof_mdat(std::slice::from_ref(&f));
    f.tfdt = Some((100, true));
    let m2 = moof_mdat(std::slice::from_ref(&f));
    f.tfdt = Some((200, true));
    let m3 = moof_mdat(&[f]);
    let ix = sidx(1000, 0, &[(m1.len() as u32, 100), (m2.len() as u32, 100)]);
    let grouped = cat(&[&bx("styp", b"msdh\0\0\0\0msdh"), &ix, &m1, &m2]);
    let data = cat(&[&video_init(50), &grouped, &m3]);
    let s = split_mp4(&data, "").unwrap();
    assert_eq!(
        s.media.len(),
        2,
        "two indexed fragments are one segment, the next moof another"
    );
    assert_eq!(s.media[0].bytes, grouped);
    assert_eq!(s.media[0].start_ms, Some(0));
    assert_eq!(s.media[0].duration_ms, Some(200));
    assert_eq!(s.media[1].start_ms, Some(200));
}

#[test]
fn sidx_durations_are_the_fallback_when_the_trun_has_none() {
    // No durations anywhere except the sidx.
    let f = Frag::new(1, 0, 2);
    let m = moof_mdat(&[f]);
    let ix = sidx(1000, 0, &[(m.len() as u32, 250)]);
    let s = split_mp4(&cat(&[&video_init(0), &ix, &m]), "").unwrap();
    assert_eq!(s.media[0].duration_ms, Some(250));
}

#[test]
fn durations_come_from_trun_then_tfhd_then_trex() {
    let init = video_init(33); // trex default 33 ticks (ms: timescale 1000)
    let mut per_sample = Frag::new(1, 0, 3);
    per_sample.durations = Some(vec![10, 20, 30]);
    let mut with_tfhd = Frag::new(1, 60, 3);
    with_tfhd.tfhd_default_duration = Some(40);
    let from_trex = Frag::new(1, 180, 3);
    let s = split_mp4(
        &cat(&[
            &init,
            &moof_mdat(&[per_sample]),
            &moof_mdat(&[with_tfhd]),
            &moof_mdat(&[from_trex]),
        ]),
        "",
    )
    .unwrap();
    assert_eq!(
        media(&s).iter().map(|m| (m.0, m.1)).collect::<Vec<_>>(),
        [(0, Some(60)), (60, Some(120)), (180, Some(99))]
    );
}

#[test]
fn a_missing_tfdt_continues_where_the_track_left_off() {
    let mut a = Frag::new(1, 0, 2);
    a.durations = Some(vec![500, 500]);
    let mut b = Frag::new(1, 0, 2);
    b.tfdt = None;
    b.durations = Some(vec![500, 500]);
    let s = split_mp4(
        &cat(&[&video_init(0), &moof_mdat(&[a]), &moof_mdat(&[b])]),
        "",
    )
    .unwrap();
    assert_eq!(
        media(&s).iter().map(|m| (m.0, m.1)).collect::<Vec<_>>(),
        [(0, Some(1000)), (1000, Some(1000))]
    );
    // A traf without tfdt cannot be shifted.
    let mut s = s;
    assert!(matches!(
        shift_media_time(&mut s, 10),
        Err(SplitError::Unsupported(_))
    ));
    assert!(shift_media_time(&mut s, 0).is_ok());
}

#[test]
fn box_size_forms_are_all_read() {
    let ftyp = bx64("ftyp", b"iso6\0\0\0\0iso6");
    let plain_init = video_init(40);
    let old_moov = first_box(&plain_init, b"moov").unwrap();
    let moov64 = bx64("moov", old_moov.payload_of(&plain_init));
    let free = bx("free", &[0; 12]);
    let uuid = bx_uuid([9; 16], b"hello");
    let init = cat(&[&ftyp, &free, &uuid, &moov64]);
    // A 64-bit moof whose tfdt is version 0 and about to overflow, then an mdat with size 0 (to the end).
    let mut f = Frag::new(1, 0, 2);
    f.tfdt = Some((4_294_967_000, false));
    let frag = moof_mdat_opts(&[f], true);
    let moof = first_box(&frag, b"moof").unwrap();
    assert_eq!(moof.header_len(), 16);
    let mut tail = frag[..moof.end].to_vec();
    tail.extend_from_slice(&bx_to_end("mdat", &[0xAB; 32]));
    let s = split_mp4(&cat(&[&init, &tail]), "").unwrap();
    assert_eq!(
        s.init.bytes, init,
        "ftyp, free, uuid and a 64-bit moov are all init"
    );
    assert_eq!(s.media.len(), 1);
    assert_eq!(s.media[0].bytes, tail);
    assert_eq!(s.mime, "video/mp4; codecs=\"avc1.64001F\"");
    // Shifting grows the 64-bit moof (version 0 tfdt becomes version 1) and keeps its 64-bit header.
    let mut t = s.clone();
    shift_media_time(&mut t, 10_000).unwrap();
    let seg = &t.media[0].bytes;
    assert_eq!(&seg[..4], &[0, 0, 0, 1], "still the 64-bit form");
    assert_eq!(seg.len(), s.media[0].bytes.len() + 4);
    let moof2 = first_box(seg, b"moof").unwrap();
    let kids = children(seg, &moof2).unwrap();
    assert_eq!(
        moof2.header_len() + kids.iter().map(|k| k.len()).sum::<usize>(),
        moof2.len()
    );
    assert_eq!(t.media[0].start_ms, Some(4_294_967_000 + 10_000));
    let again = split_mp4(&t.concat(), "").unwrap();
    assert_same_segments(&t, &again);
}

#[test]
fn leading_free_and_trailing_mfra_boxes_are_not_segments() {
    let f = Frag::new(1, 0, 2);
    let data = cat(&[&video_init(40), &moof_mdat(&[f]), &bx("mfra", &[0; 20])]);
    let s = split_mp4(&data, "").unwrap();
    assert_eq!(s.media.len(), 1);
    assert_eq!(box_names(&s.media[0].bytes), "moof mdat");
}

#[test]
fn progressive_and_incomplete_mp4_are_rejected_with_the_right_error() {
    // mdat before any moof: a progressive file.
    let init = video_init(40);
    let progressive = cat(&[&init, &bx("mdat", &[0; 64])]);
    assert!(matches!(
        split_mp4(&progressive, ""),
        Err(SplitError::NotFragmented(_))
    ));
    // No media at all but a moov with mvex: an init-only stream is fine.
    let s = split_mp4(&init, "").unwrap();
    assert!(s.media.is_empty());
    // A moof without a moov before it.
    let f = Frag::new(1, 0, 2);
    assert!(matches!(
        split_mp4(&moof_mdat(std::slice::from_ref(&f)), ""),
        Err(SplitError::Malformed(_))
    ));
    // A moof with no mdat.
    let moof_only = &moof_mdat(std::slice::from_ref(&f))[..];
    let moof_len =
        u32::from_be_bytes([moof_only[0], moof_only[1], moof_only[2], moof_only[3]]) as usize;
    let data = cat(&[&init, &moof_only[..moof_len]]);
    assert!(split_mp4(&data, "").is_err());
    // A second moov among the media.
    let data = cat(&[
        &init,
        &moof_mdat(std::slice::from_ref(&f)),
        &bx("moov", &[0; 8]),
        &moof_mdat(&[f]),
    ]);
    assert!(matches!(
        split_mp4(&data, ""),
        Err(SplitError::Unsupported(_))
    ));
}

#[test]
fn box_headers_that_lie_are_errors() {
    // Size smaller than the header, size beyond the data, a 64-bit size beyond the data.
    assert!(matches!(
        split_mp4(&[0, 0, 0, 4, b'f', b't', b'y', b'p'], ""),
        Err(SplitError::Malformed(_))
    ));
    assert!(matches!(
        split_mp4(&[0, 0, 1, 0, b'f', b't', b'y', b'p', 0, 0], ""),
        Err(SplitError::Truncated)
    ));
    let mut huge = vec![0, 0, 0, 1, b'm', b'o', b'o', b'v'];
    huge.extend_from_slice(&u64::MAX.to_be_bytes());
    assert!(split_mp4(&huge, "").is_err());
    // A uuid box too short for its user type.
    let mut u = bx("uuid", &[1, 2, 3]);
    u.truncate(12);
    u[3] = 12;
    assert!(split_mp4(&u, "").is_err());
}

#[test]
fn trun_with_an_absurd_sample_count_is_an_error_not_a_hang() {
    let f = Frag::new(1, 0, 2);
    let mut m = moof_mdat(&[f]);
    // Find the trun and set sample_count to 0xFFFF_FFF0 with a per-sample duration flag.
    let moof = first_box(&m, b"moof").unwrap();
    let traf = find_child(&m, &moof, b"traf").unwrap().unwrap();
    let trun = find_child(&m, &traf, b"trun").unwrap().unwrap();
    m[trun.payload + 4..trun.payload + 8].copy_from_slice(&0xffff_fff0u32.to_be_bytes());
    m[trun.payload + 2] |= 0x01; // flags 0x100 (durations present)
    let r = split_mp4(&cat(&[&video_init(40), &m]), "");
    assert!(matches!(r, Err(SplitError::Truncated)), "{r:?}");
}

#[test]
fn shifting_synthetic_streams_fixes_every_size_and_offset() {
    // Version 0 tfdt close to the 32-bit limit, two trafs (muxed), per-sample sizes.
    let init = init_segment(&[
        (1, 90_000, Kind::VideoAvc, 3000),
        (2, 48_000, Kind::AudioAac, 1024),
    ]);
    let mut v = Frag::new(1, 0, 5);
    v.tfdt = Some((4_000_000_000, false));
    v.sample_size = 100;
    let mut a = Frag::new(2, 0, 7);
    a.tfdt = Some((4_000_000_000, false));
    a.sample_size = 30;
    a.base_is_moof = true;
    let seg = moof_mdat(&[v, a]);
    let s = split_mp4(&cat(&[&init, &seg]), "").unwrap();
    assert_eq!(s.mime, "video/mp4; codecs=\"avc1.64001F, mp4a.40.2\"");
    let mut t = s.clone();
    // +5,000 s: +450e6 ticks at 90 kHz overflow the video tfdt (4.0e9 + 0.45e9 > 2^32); +240e6 ticks at 48 kHz do
    // not overflow the audio one. Only the video traf grows, but the data offsets of both move.
    shift_media_time(&mut t, 5_000_000).unwrap();
    check_consistent(&t.media[0].bytes);
    check_moved(&s.media[0].bytes, &t.media[0].bytes);
    assert_eq!(
        t.media[0].bytes.len(),
        s.media[0].bytes.len() + 4,
        "only the video tfdt grew, by 4 bytes"
    );
    assert_eq!(first_tfdt_version(&t.media[0].bytes), 1);
}

#[test]
fn a_covering_sidx_follows_the_growth_of_its_moof() {
    let mut f = Frag::new(1, 0, 2);
    f.tfdt = Some((4_294_967_000, false)); // version 0, near the limit at 1 kHz
    f.durations = Some(vec![100, 100]);
    let m = moof_mdat(&[f]);
    let ix = sidx(1000, 0, &[(m.len() as u32, 200)]);
    let data = cat(&[&video_init(100), &ix, &m]);
    let mut s = split_mp4(&data, "").unwrap();
    shift_media_time(&mut s, 10_000).unwrap();
    let seg = &s.media[0].bytes;
    check_consistent(&seg[ix.len()..]);
    // The sidx reference now says the fragment is 4 bytes longer, and it still covers exactly the rest.
    let again = split_mp4(&cat(&[&s.init.bytes, seg]), "").unwrap();
    assert_eq!(
        again.media.len(),
        1,
        "the sidx still groups the whole fragment"
    );
    let size_field = 8 + 24; // box header + payload offset of the first reference
    let size = u32::from_be_bytes([
        seg[size_field],
        seg[size_field + 1],
        seg[size_field + 2],
        seg[size_field + 3],
    ]);
    assert_eq!(size as usize, m.len() + 4);
    assert_eq!(size as usize, seg.len() - ix.len());
}

#[test]
fn codec_strings_follow_the_registrations() {
    use crate::mp4_init::{av1_codec, hevc_codec, vp9_codec};
    // avcC bytes: 1, profile, compat, level.
    let avc = |p: u8, c: u8, l: u8| format!("avc1.{p:02X}{c:02X}{l:02X}");
    assert_eq!(avc(0x42, 0xe0, 0x1e), "avc1.42E01E");
    // hvcC: version 1; space 0 tier 0 profile 1; compat 0x60000000 -> reversed 0x00000006; level 93;
    // constraints B0 00 00 00 00 00.
    let mut hvcc = vec![1, 0x01, 0x60, 0, 0, 0, 0xb0, 0, 0, 0, 0, 0, 93];
    assert_eq!(hevc_codec(b"hvc1", &hvcc).unwrap(), "hvc1.1.6.L93.B0");
    hvcc[1] = 0x20 | 2; // high tier, Main 10
    hvcc[2..6].copy_from_slice(&0x2000_0000u32.to_be_bytes());
    assert_eq!(hevc_codec(b"hev1", &hvcc).unwrap(), "hev1.2.4.H93.B0");
    // vpcC: FullBox(4), profile 2, level 31, bit depth 10 in the high nibble.
    assert_eq!(
        vp9_codec(&[1, 0, 0, 0, 2, 31, 0xa0, 1, 1, 1]).unwrap(),
        "vp09.02.31.10"
    );
    // av1C: marker/version, profile 0 + level 8, tier 0 + high_bitdepth 1.
    assert_eq!(av1_codec(&[0x81, 0x08, 0x4c, 0]).unwrap(), "av01.0.08M.10");
    assert_eq!(av1_codec(&[0x81, 0x2d, 0x8c, 0]).unwrap(), "av01.1.13H.08");
    assert!(av1_codec(&[0x81]).is_err());
}

#[test]
fn the_derived_type_for_a_stream_with_an_unknown_codec_needs_a_hint() {
    // Replace the avcC with something and the entry with an unknown fourcc.
    let mut init = video_init(40);
    let at = init.windows(4).position(|w| w == b"avc1").unwrap();
    init[at..at + 4].copy_from_slice(b"zzz1");
    let f = Frag::new(1, 0, 2);
    let data = cat(&[&init, &moof_mdat(&[f])]);
    assert!(matches!(
        split_mp4(&data, ""),
        Err(SplitError::Unsupported(_))
    ));
    let s = split_mp4(&data, "video/mp4; codecs=\"zzz\"").unwrap();
    assert_eq!(
        s.mime, "video/mp4; codecs=\"zzz\"",
        "the hint is used only when the codec cannot be derived"
    );
    // When it can be derived the hint is ignored.
    let good = cat(&[&video_init(40), &moof_mdat(&[Frag::new(1, 0, 2)])]);
    assert_eq!(
        split_mp4(&good, "video/webm").unwrap().mime,
        "video/mp4; codecs=\"avc1.64001F\""
    );
}

// ---------------------------------------------------------------------------------------------
// Synthetic WebM streams

const ID_EBML: u32 = 0x1A45_DFA3;
const ID_SEGMENT: u32 = 0x1853_8067;
const ID_INFO: u32 = 0x1549_A966;
const ID_TRACKS: u32 = 0x1654_AE6B;
const ID_CLUSTER: u32 = 0x1F43_B675;
const ID_TIMECODE: u32 = 0xE7;
const ID_CUES: u32 = 0x1C53_BB6B;

fn webm_header() -> Vec<u8> {
    ebml(ID_EBML, &ebml(0x4282, b"webm"))
}

fn opus_tracks() -> Vec<u8> {
    let entry = ebml(
        0xAE,
        &cat(&[
            &ebml_uint(0xD7, 1),
            &ebml_uint(0x83, 2),
            &ebml(0x86, b"A_OPUS\0"),
            &ebml_uint(0x23E383, 20_000_000),
        ]),
    );
    ebml(ID_TRACKS, &entry)
}

fn cluster(tc: u64, frames: &[(i16, u8)]) -> Vec<u8> {
    let mut p = ebml_uint(ID_TIMECODE, tc);
    for (rel, b) in frames {
        p.extend_from_slice(&simple_block(1, *rel, &[*b; 10]));
    }
    ebml(ID_CLUSTER, &p)
}

fn synthetic_webm(segment_unknown: bool, clusters: &[Vec<u8>]) -> Vec<u8> {
    let info = ebml(ID_INFO, &ebml_uint(0x2AD7B1, 1_000_000));
    let body = cat(&[
        &info,
        &opus_tracks(),
        &cat(&clusters.iter().map(Vec::as_slice).collect::<Vec<_>>()),
    ]);
    let seg = if segment_unknown {
        ebml_unknown(ID_SEGMENT, &body)
    } else {
        ebml(ID_SEGMENT, &body)
    };
    cat(&[&webm_header(), &seg])
}

#[test]
fn webm_segments_are_clusters_with_block_based_durations() {
    let c1 = cluster(0, &[(0, 1), (20, 2), (40, 3)]);
    let c2 = cluster(60, &[(0, 4), (20, 5)]);
    for unknown in [false, true] {
        let data = synthetic_webm(unknown, &[c1.clone(), c2.clone()]);
        let s = split_webm(&data, "").unwrap();
        assert_eq!(s.mime, "audio/webm; codecs=\"opus\"");
        assert_eq!(s.media.len(), 2);
        assert_eq!(s.media[0].bytes, c1);
        // The last block of a cluster lasts as long as the track's DefaultDuration (20 ms).
        assert_eq!(
            (s.media[0].start_ms, s.media[0].duration_ms),
            (Some(0), Some(60))
        );
        assert_eq!(
            (s.media[1].start_ms, s.media[1].duration_ms),
            (Some(60), Some(40))
        );
        assert_eq!(s.concat(), data);
    }
}

#[test]
fn unknown_size_clusters_end_at_the_next_level_one_element() {
    let body1 = cat(&[
        &ebml_uint(ID_TIMECODE, 0),
        &simple_block(1, 0, &[1; 8]),
        &simple_block(1, 20, &[2; 8]),
    ]);
    let body2 = cat(&[&ebml_uint(ID_TIMECODE, 40), &simple_block(1, 0, &[3; 8])]);
    let c1 = ebml_unknown(ID_CLUSTER, &body1);
    let c2 = ebml_unknown(ID_CLUSTER, &body2);
    let cues = ebml(ID_CUES, &ebml(0xBB, &[0; 4]));
    let data = synthetic_webm(true, &[c1.clone(), c2.clone(), cues]);
    let s = split_webm(&data, "").unwrap();
    assert_eq!(
        s.media.len(),
        2,
        "the Cues element after the last Cluster is not a segment"
    );
    assert_eq!(s.media[0].bytes, c1);
    assert_eq!(
        s.media[1].bytes, c2,
        "the unknown-size Cluster stops at the Cues element"
    );
    assert_eq!(s.media[1].start_ms, Some(40));
    // An unknown-size Cluster stays unknown-size when shifted, and its timecode grows.
    let mut t = s.clone();
    shift_media_time(&mut t, 70_000).unwrap();
    assert_eq!(&t.media[0].bytes[..4], &[0x1f, 0x43, 0xb6, 0x75]);
    assert_eq!(
        &t.media[0].bytes[4..12],
        &[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
    );
    assert_eq!(t.media[0].start_ms, Some(70_000));
    assert_eq!(t.media[1].start_ms, Some(70_040));
    let again = split_webm(&t.concat(), "").unwrap();
    assert_same_segments(&t, &again);
}

#[test]
fn a_known_size_cluster_grows_with_its_timecode() {
    let c = cluster(0, &[(0, 1), (20, 2)]);
    let data = synthetic_webm(false, std::slice::from_ref(&c));
    let mut s = split_webm(&data, "").unwrap();
    shift_media_time(&mut s, 70_000).unwrap(); // timecode 0 (1 byte) becomes 70000 (3 bytes)
    assert_eq!(s.media[0].bytes.len(), c.len() + 2);
    let again = split_webm(&s.concat(), "").unwrap();
    assert_eq!(again.media[0].start_ms, Some(70_000));
    // Back: the element keeps its longer form, the value is right.
    shift_media_time(&mut s, -70_000).unwrap();
    assert_eq!(s.media[0].start_ms, Some(0));
}

#[test]
fn webm_errors() {
    assert!(matches!(split_webm(&[], ""), Err(SplitError::Truncated)));
    assert!(split_webm(b"not webm at all", "").is_err());
    // EBML header without a Segment after it.
    assert!(split_webm(&webm_header(), "").is_err());
    // A Cluster cut in the middle.
    let data = synthetic_webm(true, &[cluster(0, &[(0, 1), (20, 2)])]);
    assert!(matches!(
        split_webm(&data[..data.len() - 5], ""),
        Err(SplitError::Truncated)
    ));
    // A Cluster without a Timecode.
    let bad = ebml(ID_CLUSTER, &simple_block(1, 0, &[1; 4]));
    assert!(matches!(
        split_webm(&synthetic_webm(true, &[bad]), ""),
        Err(SplitError::Malformed(_))
    ));
}

#[test]
fn vp9_header_and_level_derivation() {
    use crate::webm::{vp9_keyframe_header, vp9_level};
    // frame_marker 2, profile 0, show_existing 0, key frame (0), show_frame 1, error_res 0, sync code,
    // color_space 2 (bt709) + range 0, then 16+16 bits of size - 1 = 319 x 179.
    let mut bits: Vec<u8> = vec![1, 0, 0, 0, 0, 0, 1, 0];
    for b in [0x49u8, 0x83, 0x42] {
        for i in (0..8).rev() {
            bits.push((b >> i) & 1);
        }
    }
    bits.extend([0, 1, 0, 0]); // color_space = 2, color_range = 0
    for v in [319u32, 179] {
        for i in (0..16).rev() {
            bits.push(((v >> i) & 1) as u8);
        }
    }
    while !bits.len().is_multiple_of(8) {
        bits.push(0);
    }
    let bytes: Vec<u8> = bits
        .chunks(8)
        .map(|c| c.iter().fold(0u8, |a, b| (a << 1) | b))
        .collect();
    assert_eq!(vp9_keyframe_header(&bytes), Some((0, 8, 320, 180)));
    assert_eq!(vp9_keyframe_header(&[0x00, 0x00]), None);
    // 320x180 at 25 fps is level 1.1, 640x360 level 2.1, 1080p30 level 4, 4K60 level 5.1 or 5.2.
    assert_eq!(vp9_level(320, 180, 25.0), 11);
    assert_eq!(vp9_level(640, 360, 25.0), 21);
    assert_eq!(vp9_level(1920, 1080, 30.0), 40);
    assert_eq!(vp9_level(3840, 2160, 60.0), 51);
}
