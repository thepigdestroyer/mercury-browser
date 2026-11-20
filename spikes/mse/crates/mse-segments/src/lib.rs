// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Splits fragmented MP4 and WebM files into the pieces an MSE player appends: one initialisation
//! segment, then media segments. Pure Rust, no GStreamer, so it builds and tests in seconds.
//!
//! OWNER: item 2 of lane sp-mse (see `.superpowers/sdd/lane-sp-mse-brief.md`). The names and
//! signatures below are the contract the other crates compile against: add what you need, rename
//! nothing.
//!
//! * [`split`] / [`split_mp4`] / [`split_webm`] cut a byte stream into one initialisation segment and the
//!   media segments (rules at the top of `mp4.rs` and `webm.rs`).
//! * [`SegmentedStream::mime`] is derived from the initialisation segment (`avcC`, `esds`, `vpcC`, `av1C`,
//!   WebM `Tracks`), never typed by hand.
//! * [`shift_media_time`], [`tile`] and [`with_pssh`] rewrite streams for the timestamp-offset, memory
//!   and protection scenarios.
//! * [`with_cenc_protection`] declares the first track protected (`encv`/`sinf`/`tenc`), without encrypting a
//!   sample, for the protection-event probe.
//! * [`FIXTURES`] / [`load`] give the committed media under `spikes/mse/fixtures/`.

mod av1;
mod bmff;
mod bytes;
mod cenc;
mod chunking;
mod ebml;
mod fixtures;
mod mp4;
mod mp4_init;
mod sha256;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod testutil;
mod webm;

pub use av1::{Av1Config, obu_types, webm_av1_config};
pub use cenc::{CencInfo, cenc_info, with_cenc_protection};
pub use chunking::chunks;
pub use fixtures::{FIXTURES, fixture, fixtures_dir, load, read_fixture};
pub use mp4::split_mp4;
pub use sha256::sha256_hex;
pub use webm::split_webm;

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

impl Segment {
    /// Length of the segment in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// `true` when the segment has no bytes.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// End of the segment in milliseconds (`start_ms + duration_ms`), when both are known.
    pub fn end_ms(&self) -> Option<u64> {
        Some(self.start_ms? + self.duration_ms?)
    }
}

impl SegmentedStream {
    /// The initialisation segment followed by every media segment: the original file, as far as
    /// the splitter keeps it (boxes or elements outside any segment are dropped).
    pub fn concat(&self) -> Vec<u8> {
        let mut out = self.init.bytes.clone();
        for m in &self.media {
            out.extend_from_slice(&m.bytes);
        }
        out
    }

    /// First start to last end of the media segments, in milliseconds (`None` when the stream
    /// has no media segment or a segment does not say its duration).
    pub fn span_ms(&self) -> Option<(u64, u64)> {
        let first = self.media.first()?.start_ms?;
        let mut start = first;
        let mut end = 0;
        for m in &self.media {
            start = start.min(m.start_ms?);
            end = end.max(m.end_ms()?);
        }
        Some((start, end))
    }

    /// Width and height of the first video track, from the initialisation segment.
    pub fn video_size(&self) -> Option<(u32, u32)> {
        match self.container {
            Container::Mp4 => {
                let info = mp4_init::parse_init(&self.init.bytes).ok()?;
                let t = info
                    .tracks
                    .iter()
                    .find(|t| mp4_init::InitInfo::is_video(t))?;
                Some((t.width, t.height))
            }
            Container::WebM => {
                let info = webm::parse_init(&self.init.bytes).ok()?;
                let t = info.tracks.iter().find(|t| t.kind == 1)?;
                Some((t.width, t.height))
            }
        }
    }

    /// Total media duration in milliseconds, from the first start to the last end.
    pub fn duration_ms(&self) -> Option<u64> {
        self.span_ms().map(|(s, e)| e.saturating_sub(s))
    }
}

/// Splits `data` into its initialisation segment and media segments.
///
/// `mime` is a fallback: the stream's type string is derived from the initialisation segment, and
/// the argument is used only when the codec cannot be derived (an empty string means "no
/// fallback": an underivable codec is an `Unsupported` error).
pub fn split(container: Container, data: &[u8], mime: &str) -> Result<SegmentedStream, SplitError> {
    match container {
        Container::Mp4 => mp4::split_mp4(data, mime),
        Container::WebM => webm::split_webm(data, mime),
    }
}

/// `delta_ms`-shifted copies of the media segments of `stream`.
fn shifted(stream: &SegmentedStream, delta_ms: i64) -> Result<Vec<Segment>, SplitError> {
    match stream.container {
        Container::Mp4 => mp4::shifted_media(&stream.init.bytes, &stream.media, delta_ms),
        Container::WebM => webm::shifted_media(&stream.init.bytes, &stream.media, delta_ms),
    }
}

/// Moves every media segment by `delta_ms` in time (rewrites `tfdt` baseMediaDecodeTime, or Cluster
/// timecodes), so that the same segment can be appended at another position: the way a player builds a
/// second period, tests `timestampOffset`, and fills a gap after a seek.
///
/// MP4: a version 0 `tfdt` that no longer fits 32 bits becomes version 1, which makes it, its `traf`
/// and its `moof` 4 bytes longer; the enclosing sizes, every `trun` data offset and a covering `sidx`
/// reference are fixed. WebM: the Cluster Timecode element may change length, and the Cluster size
/// with it (unless the Cluster has an unknown size). `start_ms` of each segment is read again from the
/// rewritten bytes. A shift before time zero is `Malformed`; on any error the stream is unchanged.
pub fn shift_media_time(stream: &mut SegmentedStream, delta_ms: i64) -> Result<(), SplitError> {
    stream.media = shifted(stream, delta_ms)?;
    Ok(())
}

/// Repeats the stream's media segments `times` times, each copy shifted by one stream duration
/// (`shift_media_time`), so that an 8-second fixture becomes a long stream for the memory and quota
/// scenarios without a megabyte of fixtures. The initialisation segment is not repeated.
///
/// One stream duration is measured on the actual segments (first start to last end), not on a nominal
/// length. `times == 0` gives a stream without media segments; every segment must say its duration.
pub fn tile(stream: &SegmentedStream, times: usize) -> Result<SegmentedStream, SplitError> {
    let mut out = SegmentedStream {
        container: stream.container,
        mime: stream.mime.clone(),
        init: stream.init.clone(),
        media: Vec::with_capacity(stream.media.len().saturating_mul(times).min(1 << 16)),
    };
    if times == 0 || stream.media.is_empty() {
        return Ok(out);
    }
    let (start, end) = stream.span_ms().ok_or_else(|| {
        SplitError::Unsupported("tile needs the start and duration of every segment".into())
    })?;
    let one = i64::try_from(end - start)
        .map_err(|_| SplitError::Malformed("the stream is too long to tile".into()))?;
    if one == 0 {
        return Err(SplitError::Malformed(
            "the stream has no duration to tile by".into(),
        ));
    }
    out.media.extend(stream.media.iter().cloned());
    for k in 1..times {
        let delta = one
            .checked_mul(k as i64)
            .ok_or_else(|| SplitError::Malformed("the tiled stream is too long".into()))?;
        out.media.extend(shifted(stream, delta)?);
    }
    Ok(out)
}

/// Returns `init` with a `pssh` box added to its `moov` box (MP4 only; WebM gives `Unsupported`): the
/// protection probe (T08) uses it to see whether a protection system's init data survives GstMSE's
/// append pipeline.
/// `system_id` is the 16-byte DRM system ID, `data` the system-specific payload.
///
/// The box is version 0 (system ID and data, no key IDs) and is the last child of `moov`, whose size
/// is fixed; the result still splits and has the same `mime`.
pub fn with_pssh(init: &Segment, system_id: [u8; 16], data: &[u8]) -> Result<Segment, SplitError> {
    if init.bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Err(SplitError::Unsupported(
            "pssh is an MP4 box: a WebM initialisation segment carries protection data in ContentEncodings".into(),
        ));
    }
    Ok(Segment {
        kind: SegmentKind::Init,
        bytes: mp4::add_pssh(&init.bytes, system_id, data)?,
        start_ms: None,
        duration_ms: None,
    })
}
