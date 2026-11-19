// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Splits fragmented MP4 and WebM files into the pieces an MSE player appends: one initialisation
//! segment, then media segments. Pure Rust, no GStreamer, so it builds and tests in seconds.
//!
//! OWNER: item 2 of lane sp-mse (see `.superpowers/sdd/lane-sp-mse-brief.md`). The names and
//! signatures below are the contract the other crates compile against: add what you need, rename
//! nothing. Every function is a stub (`todo!`) until item 2 fills it in.

use std::path::PathBuf;

/// The two byte-stream formats of the spike. (MPEG-TS and Ogg are out of scope.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// ISO BMFF: `ftyp` + `moov` initialisation segment, then `[styp] [sidx] moof mdat` fragments.
    Mp4,
    /// WebM/Matroska: EBML header + Segment start + Info + Tracks, then Clusters.
    WebM,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Init,
    Media,
}

/// One piece of a stream, as a page would pass it to `appendBuffer`.
#[derive(Debug, Clone)]
pub struct Segment {
    pub kind: SegmentKind,
    pub bytes: Vec<u8>,
    /// Start of the segment's first sample in milliseconds (`tfdt` / Cluster timecode); `None` for `Init`.
    pub start_ms: Option<u64>,
    /// Duration in milliseconds; `None` for `Init`, or when the container does not say.
    pub duration_ms: Option<u64>,
}

/// An initialisation segment and the media segments that follow it.
#[derive(Debug, Clone)]
pub struct SegmentedStream {
    pub container: Container,
    /// The type string a page would give `addSourceBuffer`, e.g. `video/mp4; codecs="avc1.42C01E"`.
    pub mime: String,
    pub init: Segment,
    pub media: Vec<Segment>,
}

#[derive(Debug, thiserror::Error)]
pub enum SplitError {
    #[error("the data ends inside a box or element")]
    Truncated,
    #[error("not fragmented: {0}")]
    NotFragmented(String),
    #[error("malformed: {0}")]
    Malformed(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("fixture {0:?} is not in FIXTURES")]
    UnknownFixture(String),
    #[error("reading {path}: {message}")]
    Io { path: String, message: String },
}

/// How an append is cut up: GstMSE has to cope with any cut, because pages append whatever the network gave them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkPlan {
    /// The whole segment in one append.
    Whole,
    /// Pieces of this many bytes (the last one shorter).
    Fixed(usize),
    /// The listed sizes in turn, then the last size again until the data ends.
    Sizes(Vec<usize>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
    /// Audio and video in one file.
    Muxed,
}

/// One committed fixture (item 2 fills `FIXTURES`; see `fixtures/README.md`).
#[derive(Debug, Clone, Copy)]
pub struct Fixture {
    /// Short name used in scenarios, e.g. `h264-low`, `h264-high`, `aac`, `h264-aac`, `vp9-low`, `vp9-high`, `opus`.
    pub name: &'static str,
    /// Path below `spikes/mse/fixtures/`.
    pub path: &'static str,
    pub container: Container,
    pub kind: MediaKind,
    /// The type string for `addSourceBuffer`.
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
}

/// Every committed fixture (item 2).
pub const FIXTURES: &[Fixture] = &[];

/// `spikes/mse/fixtures/`, found from `CARGO_MANIFEST_DIR` at compile time.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Splits `data` into its initialisation segment and media segments.
pub fn split(
    _container: Container,
    _data: &[u8],
    _mime: &str,
) -> Result<SegmentedStream, SplitError> {
    todo!("item 2")
}

/// Reads and splits the committed fixture called `name`.
pub fn load(_name: &str) -> Result<SegmentedStream, SplitError> {
    todo!("item 2")
}

/// Cuts `data` into the pieces `plan` describes, without copying.
pub fn chunks<'a>(_data: &'a [u8], _plan: &ChunkPlan) -> Vec<&'a [u8]> {
    todo!("item 2")
}

/// Moves every media segment by `delta_ms` in time (rewrites `tfdt` baseMediaDecodeTime, or Cluster
/// timecodes), so that the same segment can be appended at another position: the way a player builds a
/// second period, tests `timestampOffset`, and fills a gap after a seek.
pub fn shift_media_time(_stream: &mut SegmentedStream, _delta_ms: i64) -> Result<(), SplitError> {
    todo!("item 2")
}

/// Repeats the stream's media segments `times` times, each copy shifted by one stream duration
/// (`shift_media_time`), so that an 8-second fixture becomes a long stream for the memory and quota
/// scenarios without a megabyte of fixtures. The initialisation segment is not repeated.
pub fn tile(_stream: &SegmentedStream, _times: usize) -> Result<SegmentedStream, SplitError> {
    todo!("item 2")
}

/// Returns `init` with a `pssh` box added to its `moov` box (MP4 only; WebM gives `Unsupported`): the
/// protection probe (T08) uses it to see whether a protection system's init data survives GstMSE's
/// append pipeline.
/// `system_id` is the 16-byte DRM system ID, `data` the system-specific payload.
pub fn with_pssh(
    _init: &Segment,
    _system_id: [u8; 16],
    _data: &[u8],
) -> Result<Segment, SplitError> {
    todo!("item 2")
}
