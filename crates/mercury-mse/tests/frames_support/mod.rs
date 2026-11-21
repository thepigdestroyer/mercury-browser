// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Synthetic frames and small helpers for the processor tests of item 2 of lane med-1 (`frames`).
//! Included with `mod frames_support;` by every `tests/frames_*.rs`; each test crate uses a subset.
#![allow(dead_code)]

pub mod real;

use std::sync::Arc;

use mercury_mse::{
    CodecConfig, CodedFrame, Container, FeedItem, InitSegment, Limits, Ns, SourceBufferProcessor,
    TimeRange, TrackConfig, TrackKind,
};

pub const MS: Ns = 1_000_000;
pub const SEC: Ns = 1_000_000_000;

/// Seconds to nanoseconds, rounded (the DOM boundary rule).
pub fn secs(s: f64) -> Ns {
    (s * 1e9).round() as Ns
}

/// A tick count at `timescale` ticks per second to nanoseconds, rounded to the nearest, computed from
/// the absolute count so that nothing accumulates.
pub fn ticks_to_ns(ticks: i64, timescale: u32) -> Ns {
    let num = ticks as i128 * 1_000_000_000i128;
    let den = timescale as i128;
    let q = num.div_euclid(den);
    let r = num.rem_euclid(den);
    (if r * 2 >= den { q + 1 } else { q }) as Ns
}

/// Milliseconds of a range edge, for readable assertions.
pub fn ms(ns: Ns) -> i64 {
    (ns as f64 / 1e6).round() as i64
}

/// Ranges as `(start ms, end ms)` pairs.
pub fn ms_ranges(ranges: &[TimeRange]) -> Vec<(i64, i64)> {
    ranges.iter().map(|r| (ms(r.start), ms(r.end))).collect()
}

/// Ranges as `(start s, end s)` pairs.
pub fn sec_ranges(ranges: &[TimeRange]) -> Vec<(f64, f64)> {
    ranges
        .iter()
        .map(|r| (r.start as f64 / 1e9, r.end as f64 / 1e9))
        .collect()
}

// ---- configurations ----

/// A track configuration. `variant` makes two renditions of the same codec differ (the `avcC` bytes and
/// the picture size), like a low and a high quality of one stream.
pub fn config(kind: TrackKind, variant: u8) -> TrackConfig {
    let (codec, codec_string, width, height, sample_rate, channels, timescale) = match kind {
        TrackKind::Video => (
            CodecConfig::H264 {
                avcc: vec![1, 0x42, 0xC0, 0x1E, variant],
            },
            "avc1.42C01E".to_string(),
            320 + 320 * variant as u32,
            180 + 180 * variant as u32,
            0,
            0,
            90_000,
        ),
        TrackKind::Audio => (
            CodecConfig::Aac {
                asc: vec![0x11, 0x90 | variant],
            },
            "mp4a.40.2".to_string(),
            0,
            0,
            48_000,
            2,
            48_000,
        ),
        TrackKind::Text => (
            CodecConfig::Other {
                codec: mercury_mse::CodecId::Vorbis,
                config: vec![variant],
            },
            "wvtt".to_string(),
            0,
            0,
            0,
            0,
            1_000,
        ),
    };
    TrackConfig {
        kind,
        track_id: match kind {
            TrackKind::Video => 1,
            TrackKind::Audio => 2,
            TrackKind::Text => 3,
        },
        codec,
        codec_string,
        width,
        height,
        sample_rate,
        channels,
        timescale,
        language: String::new(),
        default: true,
    }
}

pub fn init(configs: Vec<TrackConfig>) -> InitSegment {
    InitSegment {
        container: Container::Mp4,
        tracks: configs,
        duration: None,
        pssh: Vec::new(),
    }
}

/// An initialization segment with the given kinds, variant 0.
pub fn init_of(kinds: &[TrackKind]) -> InitSegment {
    init(kinds.iter().map(|k| config(*k, 0)).collect())
}

/// A processor with default limits that has received an initialization segment of these kinds.
pub fn processor(kinds: &[TrackKind]) -> SourceBufferProcessor {
    processor_with(kinds, Limits::default())
}

pub fn processor_with(kinds: &[TrackKind], limits: Limits) -> SourceBufferProcessor {
    let mut p = SourceBufferProcessor::new(limits);
    p.init_segment(&init_of(kinds)).expect("init segment");
    p
}

// ---- frames ----

pub fn payload(len: usize) -> Arc<[u8]> {
    vec![0xAB; len].into()
}

/// Video frames of `frame_dur` each starting at `start`, a keyframe every `gop_frames` frames,
/// `dts == pts` (no reordering). Keyframes carry `key_bytes`, the others `delta_bytes`.
pub fn video(
    start: Ns,
    count: usize,
    frame_dur: Ns,
    gop_frames: usize,
    key_bytes: usize,
    delta_bytes: usize,
) -> Vec<CodedFrame> {
    let key = payload(key_bytes);
    let delta = payload(delta_bytes);
    (0..count)
        .map(|i| {
            let t = start + i as Ns * frame_dur;
            let keyframe = i % gop_frames == 0;
            CodedFrame {
                kind: TrackKind::Video,
                track_id: 1,
                pts: t,
                dts: t,
                duration: frame_dur,
                keyframe,
                data: if keyframe { key.clone() } else { delta.clone() },
            }
        })
        .collect()
}

/// 25 fps video of `secs` seconds from `start` seconds with a keyframe every 2 s: the shape of the
/// spike's fixtures (`h264-low`), frames of 1000 bytes (keyframes 8000).
pub fn video_25fps(start_s: i64, secs: i64) -> Vec<CodedFrame> {
    video(start_s * SEC, (secs * 25) as usize, 40 * MS, 50, 8000, 1000)
}

/// Audio frames of `frame_dur` each (all random access points).
pub fn audio(start: Ns, count: usize, frame_dur: Ns, bytes: usize) -> Vec<CodedFrame> {
    let data = payload(bytes);
    (0..count)
        .map(|i| {
            let t = start + i as Ns * frame_dur;
            CodedFrame {
                kind: TrackKind::Audio,
                track_id: 2,
                pts: t,
                dts: t,
                duration: frame_dur,
                keyframe: true,
                data: data.clone(),
            }
        })
        .collect()
}

/// Frames numbered `first..first + count` of a stream whose frame `i` starts at tick
/// `i * ticks_per_frame` of `timescale` (times rounded from the absolute count, as a parser must).
/// Video has a keyframe every `gop_frames`; audio (`gop_frames == 1`) is all keyframes.
pub fn ticked(
    kind: TrackKind,
    first: u64,
    count: usize,
    ticks_per_frame: i64,
    timescale: u32,
    gop_frames: u64,
    shared: &Arc<[u8]>,
) -> Vec<CodedFrame> {
    (first..first + count as u64)
        .map(|i| {
            let tick = i as i64 * ticks_per_frame;
            let pts = ticks_to_ns(tick, timescale);
            CodedFrame {
                kind,
                track_id: 1,
                pts,
                dts: pts,
                duration: ticks_to_ns(ticks_per_frame, timescale),
                keyframe: i % gop_frames == 0,
                data: shared.clone(),
            }
        })
        .collect()
}

/// An IBBP-style stream in decode order with reordering: closed groups of 10 frames in display order
/// (I B B P B B P B B P), decode order I P B B P B B P B B, `dts = dts_start + decode index * dur`,
/// `pts = dts_start + (display index + 2) * dur` (the composition offset an MP4 muxer writes, so
/// `pts >= dts` and the first picture is shown 2 frames after the first decode timestamp).
pub fn bframes(dts_start: Ns, groups: usize, dur: Ns, bytes: usize) -> Vec<CodedFrame> {
    const DISPLAY_OF_DECODE: [i64; 10] = [0, 3, 1, 2, 6, 4, 5, 9, 7, 8];
    let data = payload(bytes);
    let mut frames = Vec::new();
    for g in 0..groups {
        for (j, d) in DISPLAY_OF_DECODE.iter().enumerate() {
            let decode_index = (g * 10 + j) as i64;
            let display_index = (g * 10) as i64 + d;
            frames.push(CodedFrame {
                kind: TrackKind::Video,
                track_id: 1,
                pts: dts_start + (display_index + 2) * dur,
                dts: dts_start + decode_index * dur,
                duration: dur,
                keyframe: j == 0,
                data: data.clone(),
            });
        }
    }
    frames
}

// ---- the feed ----

/// Everything the feed serves for `kind` from `position`, in `max`-sized reads.
pub fn read_all(p: &SourceBufferProcessor, kind: TrackKind, position: Ns) -> Vec<FeedItem> {
    let mut cursor = p.cursor(kind, position);
    let mut out = Vec::new();
    for _ in 0..1_000_000 {
        if !p.read(kind, &mut cursor, 64, &mut out) {
            break;
        }
    }
    out
}

/// The frames among the items.
pub fn frames_of(items: &[FeedItem]) -> Vec<CodedFrame> {
    items
        .iter()
        .filter_map(|i| match i {
            FeedItem::Frame { frame, .. } => Some(frame.clone()),
            FeedItem::Config(_) => None,
        })
        .collect()
}

/// How many `Config` items are among the items.
pub fn config_count(items: &[FeedItem]) -> usize {
    items
        .iter()
        .filter(|i| matches!(i, FeedItem::Config(_)))
        .count()
}

/// The decode order and the presentation times as `(dts ms, pts ms)`.
pub fn dts_pts_ms(frames: &[CodedFrame]) -> Vec<(i64, i64)> {
    frames.iter().map(|f| (ms(f.dts), ms(f.pts))).collect()
}

/// The largest gap between the end of a frame and the start of the next in presentation order, over
/// the frames (0 when they tile).
pub fn largest_gap(frames: &[CodedFrame]) -> Ns {
    let mut sorted: Vec<&CodedFrame> = frames.iter().collect();
    sorted.sort_by_key(|f| f.pts);
    let mut gap = 0;
    for w in sorted.windows(2) {
        gap = gap.max(w[1].pts - (w[0].pts + w[0].duration));
    }
    gap
}

/// A tiny deterministic generator (SplitMix64) for the property tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    /// A number in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}
