// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The contract of the MSE lane (lane `med-1`, `.superpowers/sdd/lane-med-1-brief.md` section 5).
//!
//! FROZEN by the lane lead: add what you need, **rename or remove nothing**. Everything the DOM, the
//! GStreamer feed and the parts of this crate say to each other is here, so the five work items of
//! the lane compile against one text.
//!
//! * Time inside the crate is [`Ns`], integer nanoseconds. The DOM boundary is `f64` seconds; use
//!   [`secs_to_ns`] and [`ns_to_secs`].
//! * [`MediaSourceBackend`] is what the DOM calls. [`MseEvent`] is what comes back, through an
//!   [`EventSink`] that the glue turns into a `GenericCallback`.
//! * [`FrameFeed`] is what GStreamer pulls from. [`ByteStreamParser`] is what turns appended bytes into an
//!   [`InitSegment`] and [`CodedFrame`]s.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

/// The order in which a source buffer was added to its MediaSource; never reused within one.
pub type SourceBufferId = u32;

/// Time in nanoseconds. Signed: a container time before an offset or an append window can be negative.
pub type Ns = i64;

/// Nanoseconds in a second.
pub const NS_PER_SEC: Ns = 1_000_000_000;

/// Seconds (what the DOM uses) to nanoseconds, rounded to the nearest. NaN gives 0, +Infinity and
/// values beyond the range saturate.
pub fn secs_to_ns(secs: f64) -> Ns {
    (secs * 1e9).round() as Ns
}

/// Nanoseconds to seconds.
pub fn ns_to_secs(ns: Ns) -> f64 {
    ns as f64 / 1e9
}

/// The kind of a track, which is also the key of a track buffer (decision D3: one track per kind per
/// source buffer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
    Text,
}

impl TrackKind {
    /// Every kind, in a fixed order.
    pub const ALL: [TrackKind; 3] = [TrackKind::Video, TrackKind::Audio, TrackKind::Text];

    /// `"video"`, `"audio"` or `"text"`.
    pub fn name(self) -> &'static str {
        match self {
            TrackKind::Video => "video",
            TrackKind::Audio => "audio",
            TrackKind::Text => "text",
        }
    }
}

/// The byte-stream formats of this round (decision D4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Container {
    /// ISO BMFF: `ftyp` + `moov` initialization segment, then `[styp] [sidx] moof mdat` fragments.
    Mp4,
    /// WebM / Matroska: EBML header, Segment, Info, Tracks, then Clusters.
    WebM,
}

/// The codecs Mercury names. Which of them a parser feeds is decision D4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CodecId {
    H264,
    Hevc,
    Vp8,
    Vp9,
    Av1,
    Aac,
    Opus,
    Mp3,
    Flac,
    Vorbis,
    Ac3,
    Eac3,
}

impl CodecId {
    /// The kind of track this codec makes.
    pub fn kind(self) -> TrackKind {
        match self {
            CodecId::H264 | CodecId::Hevc | CodecId::Vp8 | CodecId::Vp9 | CodecId::Av1 => {
                TrackKind::Video
            }
            CodecId::Aac
            | CodecId::Opus
            | CodecId::Mp3
            | CodecId::Flac
            | CodecId::Vorbis
            | CodecId::Ac3
            | CodecId::Eac3 => TrackKind::Audio,
        }
    }

    /// `true` for the codecs the parsers feed in round 1 (H.264, VP8, VP9, AV1, AAC, Opus).
    pub fn fed_in_round_1(self) -> bool {
        matches!(
            self,
            CodecId::H264
                | CodecId::Vp8
                | CodecId::Vp9
                | CodecId::Av1
                | CodecId::Aac
                | CodecId::Opus
        )
    }
}

/// `SourceBuffer.mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppendMode {
    Segments,
    Sequence,
}

/// `MediaSource.readyState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaSourceReadyState {
    Closed,
    Open,
    Ended,
}

/// The argument of `endOfStream(error)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EosError {
    Network,
    Decode,
}

/// `HTMLMediaElement.readyState`, as computed for a MediaSource (decision D13).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ElementReadyState {
    HaveNothing,
    HaveMetadata,
    HaveCurrentData,
    HaveFutureData,
    HaveEnoughData,
}

/// A half-open interval `[start, end)` of presentation time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Ns,
    pub end: Ns,
}

impl TimeRange {
    pub fn new(start: Ns, end: Ns) -> Self {
        TimeRange { start, end }
    }

    /// `end - start`.
    pub fn len(&self) -> Ns {
        self.end - self.start
    }

    /// `true` when `end <= start`.
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// `true` when `start <= t < end`.
    pub fn contains(&self, t: Ns) -> bool {
        self.start <= t && t < self.end
    }

    pub fn start_secs(&self) -> f64 {
        ns_to_secs(self.start)
    }

    pub fn end_secs(&self) -> f64 {
        ns_to_secs(self.end)
    }
}

/// What a method of the backend can refuse with. `dom_exception_name` is the DOMException the DOM
/// throws.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum MseError {
    #[error("InvalidStateError: {0}")]
    InvalidState(String),
    /// A JavaScript `TypeError` (a bad type string, a bad window or offset).
    #[error("TypeError: {0}")]
    Type(String),
    #[error("NotSupportedError: {0}")]
    NotSupported(String),
    #[error("NotFoundError: {0}")]
    NotFound(String),
    #[error("QuotaExceededError: {0}")]
    QuotaExceeded(String),
    /// Anything else; the DOM throws `InvalidStateError`.
    #[error("{0}")]
    Other(String),
}

impl MseError {
    /// The DOMException name; `"TypeError"` means a JavaScript `TypeError`, not a DOMException.
    pub fn dom_exception_name(&self) -> &'static str {
        match self {
            MseError::InvalidState(_) | MseError::Other(_) => "InvalidStateError",
            MseError::Type(_) => "TypeError",
            MseError::NotSupported(_) => "NotSupportedError",
            MseError::NotFound(_) => "NotFoundError",
            MseError::QuotaExceeded(_) => "QuotaExceededError",
        }
    }
}

// ----------------------------------------------------------------------------------------------
// Codec configuration and initialization segments

/// The codec configuration a decoder needs, as the container carries it (what GStreamer's caps are made
/// from). `Other` is reserved for the codecs that decision D4 leaves for later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodecConfig {
    /// `avcC`.
    H264 {
        avcc: Vec<u8>,
    },
    /// `hvcC`.
    Hevc {
        hvcc: Vec<u8>,
    },
    Vp8,
    /// `vpcC` (the VPCodecConfigurationRecord), when the container has one.
    Vp9 {
        vpcc: Option<Vec<u8>>,
    },
    /// `av1C`, complete with the sequence header OBU when the container has it.
    Av1 {
        av1c: Vec<u8>,
    },
    /// The AudioSpecificConfig (from `esds`).
    Aac {
        asc: Vec<u8>,
    },
    /// `OpusHead`.
    Opus {
        head: Vec<u8>,
    },
    Other {
        codec: CodecId,
        config: Vec<u8>,
    },
}

impl CodecConfig {
    pub fn id(&self) -> CodecId {
        match self {
            CodecConfig::H264 { .. } => CodecId::H264,
            CodecConfig::Hevc { .. } => CodecId::Hevc,
            CodecConfig::Vp8 => CodecId::Vp8,
            CodecConfig::Vp9 { .. } => CodecId::Vp9,
            CodecConfig::Av1 { .. } => CodecId::Av1,
            CodecConfig::Aac { .. } => CodecId::Aac,
            CodecConfig::Opus { .. } => CodecId::Opus,
            CodecConfig::Other { codec, .. } => *codec,
        }
    }
}

/// One track of an initialization segment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackConfig {
    pub kind: TrackKind,
    /// The container's track id (`tkhd` track_ID, Matroska TrackNumber).
    pub track_id: u32,
    pub codec: CodecConfig,
    /// The RFC 6381 codec string derived from the configuration (`avc1.42C01E`, `mp4a.40.2`, `opus`).
    pub codec_string: String,
    /// Coded size for video, 0 otherwise.
    pub width: u32,
    pub height: u32,
    /// Audio sample rate and channel count, 0 otherwise.
    pub sample_rate: u32,
    pub channels: u32,
    /// Container ticks per second (`mdhd`; 1,000,000,000 / TimestampScale for WebM); 0 when not applicable.
    pub timescale: u32,
    /// The language tag, or empty.
    pub language: String,
    pub default: bool,
}

/// A parsed initialization segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitSegment {
    pub container: Container,
    pub tracks: Vec<TrackConfig>,
    /// The duration the segment declares (`mvhd`/`mehd`, WebM `Duration`), if any.
    pub duration: Option<Ns>,
    /// Every `pssh` box of the `moov`, raw and untouched (EME is a later lane; decision D4).
    pub pssh: Vec<Vec<u8>>,
}

/// What the DOM learns about one track (it builds `audioTracks`/`videoTracks` from this later).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub kind: TrackKind,
    pub id: u32,
    pub codec: String,
    pub language: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    pub sample_rate: u32,
    pub default: bool,
}

/// What the DOM learns from an initialization segment ("initialization segment received").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InitInfo {
    pub tracks: Vec<TrackInfo>,
    /// The duration the segment declares, in seconds.
    pub duration: Option<f64>,
    /// `true` for the first initialization segment of this source buffer.
    pub first: bool,
    /// `true` when the segment carries `pssh` boxes or declares a protected track.
    pub protected: bool,
}

/// One coded frame (a sample). At the parser's output the times are container times; after the
/// processor they are on the MediaSource's presentation timeline (offset and mode applied). The same
/// struct serves both.
#[derive(Clone, Debug)]
pub struct CodedFrame {
    pub kind: TrackKind,
    pub track_id: u32,
    pub pts: Ns,
    pub dts: Ns,
    pub duration: Ns,
    /// A random access point (a sync sample).
    pub keyframe: bool,
    pub data: Arc<[u8]>,
}

// ----------------------------------------------------------------------------------------------
// Events from the backend to the DOM

/// Why an append failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppendErrorKind {
    /// The bytes do not parse (the spec's append error algorithm: the stream ends with a decode error).
    Decode,
    /// A feature that decision D4 leaves out.
    Unsupported,
    /// A fault of the backend itself.
    Internal,
    /// The coded frames of the append do not fit the quota (a page appended far more than it asked
    /// room for, or a stream of tiny frames): the append stops, as for a decode error.
    QuotaExceeded,
}

/// Identifies one accepted `append` or `remove` (lane med-1b, D3). Never reused within a backend. The
/// completion events carry it, so that the DOM can drop one that was already queued when it called
/// `abort()` or removed the buffer (`OpTracker` in `op_tracker.rs` makes that decision).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OpId(pub u64);

impl std::fmt::Display for OpId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "op{}", self.0)
    }
}

/// Everything the backend tells the DOM. It crosses a `GenericCallback`, so it is plain data. The rules
/// of decisions D8 to D10: one `AppendDone` or `AppendError` per accepted append and one `RemoveDone`
/// per accepted remove, never a spurious event, the data is in `buffered` when `AppendDone` arrives,
/// nothing arrives for an aborted append, and no closure may call back into the backend on the
/// emitting thread's behalf.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MseEvent {
    /// An initialization segment was parsed; sent before the `AppendDone` of the same append.
    InitSegment { id: SourceBufferId, init: InitInfo },
    /// The append finished: `updating` is already false and the frames are in `buffered`.
    AppendDone {
        id: SourceBufferId,
        /// The operation this completes: what `append` returned.
        op: OpId,
    },
    AppendError {
        id: SourceBufferId,
        op: OpId,
        kind: AppendErrorKind,
    },
    RemoveDone {
        id: SourceBufferId,
        /// The operation this completes: what `remove` returned.
        op: OpId,
    },
    /// The MediaSource duration changed (initialization segment, end of stream, truncation).
    DurationChanged(Option<f64>),
    /// The model ended the stream itself (an append error); the DOM fires `sourceended` and, for
    /// `Some(Decode)`, the element's decode error.
    SourceEnded { error: Option<EosError> },
    /// Data left `buffered` without a DOM call (eviction).
    BufferedChanged { id: SourceBufferId },
}

/// Where events go. The glue wraps a `GenericCallback<MseEvent>`; tests collect them.
pub type EventSink = Arc<dyn Fn(MseEvent) + Send + Sync + 'static>;

// ----------------------------------------------------------------------------------------------
// Limits and decoder support

/// The quota and eviction numbers (decision D6). Tests use small ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub video_bytes: usize,
    pub audio_bytes: usize,
    pub text_bytes: usize,
    /// Eviction from the start never touches the last `keep_behind` before the playback position.
    pub keep_behind: Ns,
    /// Eviction from the end only takes data further than `keep_ahead` after the position.
    pub keep_ahead: Ns,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            video_bytes: 150 * 1024 * 1024,
            audio_bytes: 12 * 1024 * 1024,
            text_bytes: 1024 * 1024,
            keep_behind: 5 * NS_PER_SEC,
            keep_ahead: 30 * NS_PER_SEC,
        }
    }
}

/// What one stored coded frame costs beyond its bytes: its entries in the track buffer's two ordered
/// maps (key, `StoredFrame` and its `CodedFrame` fields, the presentation index), the `Arc` header and
/// allocator rounding of its data, and the group bookkeeping. About 270 bytes by count of the structures
/// (the review of lane med-1 estimated 200 to 300), so 256. The quota and the shared budget count
/// `data.len()` plus this for every frame, so that a stream of one-byte frames cannot use many times the
/// quota in heap. Real streams stay close to their old quota: a 2 s video segment of 50 frames adds
/// 12.8 KB to some megabytes, and the audio quota still holds about 8 minutes of AAC.
pub const FRAME_OVERHEAD_BYTES: usize = 256;

/// One memory budget shared by every source buffer of every MediaSource that holds a clone of it (the
/// glue makes one per content process, decision D17). A track buffer charges it for each frame it
/// stores and gives the bytes back when the frame is removed or the buffer dropped; eviction and the
/// append checks read it, so the sum over all pages cannot pass its cap whatever each page's own quota
/// says.
#[derive(Debug)]
pub struct MemoryBudget {
    cap: usize,
    used: AtomicUsize,
}

impl MemoryBudget {
    /// A budget of `cap` bytes, nothing used.
    pub fn new(cap: usize) -> Arc<MemoryBudget> {
        Arc::new(MemoryBudget {
            cap,
            used: AtomicUsize::new(0),
        })
    }

    /// The bytes all holders have stored now.
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// The most the holders may store together.
    pub fn cap(&self) -> usize {
        self.cap
    }

    pub(crate) fn charge(&self, bytes: usize) {
        self.used.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn release(&self, bytes: usize) {
        let mut current = self.used.load(Ordering::Relaxed);
        loop {
            match self.used.compare_exchange_weak(
                current,
                current.saturating_sub(bytes),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }
}

impl Limits {
    /// The most bytes one `appendBuffer` may carry: what all the track buffers of a source buffer may
    /// hold together. An append bigger than that can never fit, so it is refused before it is copied
    /// or parsed.
    pub fn max_append_bytes(&self) -> usize {
        self.video_bytes
            .saturating_add(self.audio_bytes)
            .saturating_add(self.text_bytes)
    }

    /// The byte limit of a track buffer of this kind.
    pub fn bytes_for(&self, kind: TrackKind) -> usize {
        match kind {
            TrackKind::Video => self.video_bytes,
            TrackKind::Audio => self.audio_bytes,
            TrackKind::Text => self.text_bytes,
        }
    }
}

/// Whether the platform can decode a codec. `mercury-mse-gst` answers from the GStreamer registry.
pub trait TypeSupport: Send + Sync {
    fn has_decoder(&self, kind: TrackKind, codec: CodecId) -> bool;
}

/// A [`TypeSupport`] that says yes to everything, for tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllDecoders;

impl TypeSupport for AllDecoders {
    fn has_decoder(&self, _kind: TrackKind, _codec: CodecId) -> bool {
        true
    }
}

// ----------------------------------------------------------------------------------------------
// Byte-stream parsers (item 1 implements, item 3 uses)

/// What a parser found in the bytes it was given.
#[derive(Debug)]
pub enum ParseEvent {
    Init(InitSegment),
    /// Complete frames, in decode order, as soon as their bytes are complete.
    Frames(Vec<CodedFrame>),
    /// A media segment ended (the end of a `moof`+`mdat` pair, of a Cluster).
    SegmentEnd,
}

/// Why bytes did not parse. After an error the parser ignores everything until [`ByteStreamParser::reset`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("malformed: {0}")]
    Malformed(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("a media segment came before any initialization segment")]
    MediaBeforeInit,
    #[error("more than one track of the same kind")]
    TooManyTracks,
    #[error("the bytes are not this container: {0}")]
    WrongContainer(String),
}

/// An incremental parser of one container's byte stream. Any chunking of the same bytes gives the
/// same events (frames may be split across `push` calls differently, never differently in content).
pub trait ByteStreamParser: Send {
    fn container(&self) -> Container;

    /// Feeds bytes; returns what became complete.
    fn push(&mut self, data: &[u8]) -> Result<Vec<ParseEvent>, ParseError>;

    /// "Reset parser state": drop a partial segment and any error, keep the initialization
    /// configuration.
    fn reset(&mut self);
}

// ----------------------------------------------------------------------------------------------
// The feed (item 3 implements, item 4 uses)

/// One thing the feed hands to GStreamer.
#[derive(Clone, Debug)]
pub enum FeedItem {
    /// A new codec configuration for this track: change the caps before the next frame.
    Config(TrackConfig),
    /// A frame on the presentation timeline; `discont` after a seek or a gap.
    Frame { frame: CodedFrame, discont: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedState {
    /// More is available now: pull again.
    More,
    /// The end of the buffered data; wait for the wake closure.
    WaitingForData,
    /// The stream ended and everything was served: send EOS.
    Ended,
}

/// What one pull returns.
#[derive(Clone, Debug)]
pub struct FeedBatch {
    pub items: Vec<FeedItem>,
    pub state: FeedState,
}

/// The pull interface GStreamer uses. Never blocks; callable from any thread.
pub trait FrameFeed: Send + Sync {
    /// The track configurations known so far, one per kind at most (empty before the first
    /// initialization segment).
    fn tracks(&self) -> Vec<TrackConfig>;

    /// `true` when every source buffer that exists has received its initialization segment (the
    /// element announces its streams then).
    fn all_initialized(&self) -> bool;

    /// The MediaSource duration, for the `appsrc` duration.
    fn duration(&self) -> Option<Ns>;

    /// The next pull of every kind starts at the random access point at or before `position`.
    fn seek(&self, position: Ns);

    /// Up to `max_frames` frames of `kind` (and the `Config` items in front of them).
    fn pull(&self, kind: TrackKind, max_frames: usize) -> FeedBatch;

    /// `wake` is called, from any thread, when new frames, a configuration change or the end of the
    /// stream may be available.
    fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>);
}

// ----------------------------------------------------------------------------------------------
// What the DOM calls

/// The backend of one MediaSource, as the DOM sees it. Implemented by `MseBackend`. Times are
/// seconds; `None` for a duration is NaN, `f64::INFINITY` is +Infinity. Every method returns without
/// waiting for the worker thread except where the spec needs a synchronous answer.
pub trait MediaSourceBackend: Send + Sync {
    // ---- MediaSource ----

    /// Attached to a media element: closed to open (the DOM fires `sourceopen`).
    fn open(&self);
    /// Detached: every source buffer is dropped and the state is closed.
    fn close(&self);
    fn ready_state(&self) -> MediaSourceReadyState;
    fn add_source_buffer(&self, mime: &str) -> Result<SourceBufferId, MseError>;
    fn remove_source_buffer(&self, id: SourceBufferId) -> Result<(), MseError>;
    fn source_buffer_count(&self) -> usize;
    fn duration(&self) -> Option<f64>;
    fn set_duration(&self, secs: Option<f64>) -> Result<(), MseError>;
    fn end_of_stream(&self, error: Option<EosError>) -> Result<(), MseError>;
    fn set_live_seekable_range(&self, start: f64, end: f64) -> Result<(), MseError>;
    fn clear_live_seekable_range(&self) -> Result<(), MseError>;

    // ---- SourceBuffer ----

    /// Checks now (removed buffer, `updating`, the buffer-full flag, ended to open), parses on the
    /// worker, then `AppendDone` or `AppendError`, both carrying the returned [`OpId`].
    fn append(&self, id: SourceBufferId, data: Vec<u8>) -> Result<OpId, MseError>;
    fn abort(&self, id: SourceBufferId) -> Result<(), MseError>;
    /// `end` may be `f64::INFINITY`. Then `RemoveDone`, carrying the returned [`OpId`].
    fn remove(&self, id: SourceBufferId, start: f64, end: f64) -> Result<OpId, MseError>;
    fn change_type(&self, id: SourceBufferId, mime: &str) -> Result<(), MseError>;
    fn updating(&self, id: SourceBufferId) -> bool;
    fn buffered(&self, id: SourceBufferId) -> Vec<TimeRange>;
    fn mode(&self, id: SourceBufferId) -> Result<AppendMode, MseError>;
    fn set_mode(&self, id: SourceBufferId, mode: AppendMode) -> Result<(), MseError>;
    fn timestamp_offset(&self, id: SourceBufferId) -> Result<f64, MseError>;
    fn set_timestamp_offset(&self, id: SourceBufferId, secs: f64) -> Result<(), MseError>;
    fn append_window_start(&self, id: SourceBufferId) -> Result<f64, MseError>;
    fn append_window_end(&self, id: SourceBufferId) -> Result<f64, MseError>;
    fn set_append_window_start(&self, id: SourceBufferId, secs: f64) -> Result<(), MseError>;
    fn set_append_window_end(&self, id: SourceBufferId, secs: f64) -> Result<(), MseError>;

    // ---- the media element's view ----

    /// The intersection over the active source buffers; when the stream has ended the last range
    /// extends to the highest end.
    fn element_buffered(&self) -> Vec<TimeRange>;
    fn seekable(&self) -> Vec<TimeRange>;
    /// Decision D13.
    fn element_ready_state(&self, position: f64) -> ElementReadyState;
    /// The current playback position, for eviction and readiness.
    fn set_position(&self, secs: f64);
    /// The spec's seeking algorithm; `true` when the source was ended and is open again (the DOM fires
    /// `sourceopen`).
    fn seeking(&self, secs: f64) -> bool;
    /// The feed GStreamer pulls from.
    fn frame_feed(&self) -> Arc<dyn FrameFeed>;
    /// The most bytes one `appendBuffer` may carry ([`Limits::max_append_bytes`]); the DOM checks the
    /// size of the page's buffer against it before it copies it. A backend without a limit says
    /// `usize::MAX`.
    fn max_append_bytes(&self) -> usize {
        usize::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_and_nanoseconds_round_trip() {
        for secs in [0.0, 0.001, 1.0, 8.0, 3599.999, 14400.04] {
            assert!((ns_to_secs(secs_to_ns(secs)) - secs).abs() < 1e-9, "{secs}");
        }
        assert_eq!(secs_to_ns(f64::NAN), 0);
        assert_eq!(secs_to_ns(f64::INFINITY), Ns::MAX);
        assert_eq!(secs_to_ns(0.0333333333), 33_333_333);
    }

    #[test]
    fn exception_names() {
        assert_eq!(
            MseError::Type(String::new()).dom_exception_name(),
            "TypeError"
        );
        assert_eq!(
            MseError::QuotaExceeded(String::new()).dom_exception_name(),
            "QuotaExceededError"
        );
        assert_eq!(
            MseError::Other(String::new()).dom_exception_name(),
            "InvalidStateError"
        );
    }

    #[test]
    fn codecs_know_their_kind() {
        assert_eq!(CodecId::H264.kind(), TrackKind::Video);
        assert_eq!(CodecId::Opus.kind(), TrackKind::Audio);
        assert!(CodecId::Av1.fed_in_round_1());
        assert!(!CodecId::Hevc.fed_in_round_1());
        assert_eq!(CodecConfig::Vp8.id(), CodecId::Vp8);
    }

    #[test]
    fn default_limits_are_the_documented_ones() {
        let l = Limits::default();
        assert_eq!(l.bytes_for(TrackKind::Video), 150 << 20);
        assert_eq!(l.bytes_for(TrackKind::Audio), 12 << 20);
        assert_eq!(l.keep_behind, 5 * NS_PER_SEC);
    }

    #[test]
    fn time_ranges() {
        let r = TimeRange::new(1, 4);
        assert!(r.contains(1) && r.contains(3) && !r.contains(4));
        assert_eq!(r.len(), 3);
        assert!(TimeRange::new(5, 5).is_empty());
    }

    #[test]
    fn traits_are_object_safe() {
        fn _feed(_: &dyn FrameFeed) {}
        fn _backend(_: &dyn MediaSourceBackend) {}
        fn _parser(_: &dyn ByteStreamParser) {}
        fn _types(_: &dyn TypeSupport) {}
    }
}
