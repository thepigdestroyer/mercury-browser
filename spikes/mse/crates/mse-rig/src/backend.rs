// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The sketch of MED-1's backend: everything the DOM's `MediaSource` and `SourceBuffer` need from
//! GstMSE, behind plain Rust types. No `gstreamer_mse` type appears in the public API, so the DOM
//! side (and servo-media's traits) never depend on the binding crate.
//!
//! Written over `gstreamer-mse` 0.25 from the GstMSE headers and GIR (not copied from servo/media#458,
//! which is MPL-2.0 and which MED-1 carries in a file of its own).
//!
//! Every callback GstMSE makes is turned into an [`MseEvent`] pushed to the [`EventLog`] on whatever
//! thread GstMSE used (which is itself a result: record it, do not hide it by marshalling). The
//! backend never holds one of its own locks while it calls into GstMSE, so a callback (or an
//! [`EventLog`] hook) may call straight back into the backend: whether GstMSE survives that is what
//! the re-entrancy scenario (T02) measures.
//!
//! What the spike found out through this API (the "API fit" questions; the measured answers are in
//! the doc comments of the methods they concern and in R17):
//! * `timestampOffset` is a signed double in the spec; GstMSE takes an unsigned `GstClockTime`.
//! * `duration` may be NaN or +Infinity (live); GstMSE has `GST_CLOCK_TIME_NONE`.
//! * `buffered` per source buffer exists; the element's `buffered`/`seekable` are the DOM's job.
//! * Track lists (`audioTracks`, `videoTracks`) and codec details are not in GstMSE's API.
//! * `appendBuffer` takes a `GstBuffer`: the page's bytes are copied once into it (see [`MseBackend::append`]).
//! * `appendWindowEnd = +Infinity`, `remove(start, +Infinity)`.
//! * `QuotaExceededError`: when does GstMSE raise it, and can the page tell how much is buffered?

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use glib::prelude::*;
use glib::translate::ToGlibPtr;
use gstreamer::ClockTime;
use gstreamer::prelude::*;
use gstreamer_mse as mse;

use crate::events::{ElementReadyState, EventLog, MseEvent, SourceBufferId};

/// `GstMediaSourceError`, plus the failures that are not MSE's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MseError {
    InvalidState(String),
    Type(String),
    NotSupported(String),
    NotFound(String),
    QuotaExceeded(String),
    /// A GError of another domain, or a refusal the rig itself makes (for example a negative offset).
    Other(String),
}

impl std::fmt::Display for MseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.dom_exception() {
            Some(name) => write!(f, "{name}: {}", self.message()),
            None => write!(f, "{}", self.message()),
        }
    }
}

impl std::error::Error for MseError {}

impl MseError {
    /// The message GstMSE (or the rig) gave.
    pub fn message(&self) -> &str {
        match self {
            MseError::InvalidState(m)
            | MseError::Type(m)
            | MseError::NotSupported(m)
            | MseError::NotFound(m)
            | MseError::QuotaExceeded(m)
            | MseError::Other(m) => m,
        }
    }

    /// The DOMException name a page would see for this error (`InvalidStateError`, `TypeError`,
    /// `NotSupportedError`, `NotFoundError`, `QuotaExceededError`; `Other` has none).
    pub fn dom_exception(&self) -> Option<&'static str> {
        match self {
            MseError::InvalidState(_) => Some("InvalidStateError"),
            MseError::Type(_) => Some("TypeError"),
            MseError::NotSupported(_) => Some("NotSupportedError"),
            MseError::NotFound(_) => Some("NotFoundError"),
            MseError::QuotaExceeded(_) => Some("QuotaExceededError"),
            MseError::Other(_) => None,
        }
    }
}

/// `MediaSource.readyState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSourceReadyState {
    Closed,
    Open,
    Ended,
}

/// The argument of `MediaSource.endOfStream()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EosError {
    None,
    Network,
    Decode,
}

/// `SourceBuffer.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendMode {
    Segments,
    Sequence,
}

/// One range of `SourceBuffer.buffered`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    pub start: ClockTime,
    pub end: ClockTime,
}

/// Maps a `GError` from GstMSE to [`MseError`]: domain `gst_media_source_error_quark` and its five
/// codes to the five classes (the message is kept); any other domain, or an unknown code, to
/// [`MseError::Other`] with domain and code in the text.
pub fn map_error(error: &glib::Error) -> MseError {
    let message = error.message().to_owned();
    match error.kind::<mse::MediaSourceError>() {
        Some(mse::MediaSourceError::InvalidState) => MseError::InvalidState(message),
        Some(mse::MediaSourceError::Type) => MseError::Type(message),
        Some(mse::MediaSourceError::NotSupported) => MseError::NotSupported(message),
        Some(mse::MediaSourceError::NotFound) => MseError::NotFound(message),
        Some(mse::MediaSourceError::QuotaExceeded) => MseError::QuotaExceeded(message),
        Some(other) => MseError::Other(format!("GstMediaSourceError {other:?}: {message}")),
        None => MseError::Other(format!(
            "GError domain {}: {message}",
            error.domain().as_str()
        )),
    }
}

fn not_a_source_buffer(id: SourceBufferId) -> MseError {
    MseError::NotFound(format!("the rig has no source buffer {id}"))
}

/// `GstClockTime` nanoseconds to a `ClockTime` (never panics: `u64::MAX` is `GST_CLOCK_TIME_NONE`
/// and is clamped one nanosecond below).
fn clock_time(nanoseconds: u64) -> ClockTime {
    ClockTime::from_nseconds(nanoseconds.min(u64::MAX - 1))
}

pub(crate) fn element_ready_state(state: mse::MseSrcReadyState) -> ElementReadyState {
    match state {
        mse::MseSrcReadyState::Nothing => ElementReadyState::HaveNothing,
        mse::MseSrcReadyState::Metadata => ElementReadyState::HaveMetadata,
        mse::MseSrcReadyState::CurrentData => ElementReadyState::HaveCurrentData,
        mse::MseSrcReadyState::FutureData => ElementReadyState::HaveFutureData,
        mse::MseSrcReadyState::EnoughData => ElementReadyState::HaveEnoughData,
        _ => ElementReadyState::HaveNothing,
    }
}

/// The source buffers the backend knows, by id. Ids are the order of first sight and are never reused.
struct Entry {
    id: SourceBufferId,
    buffer: mse::SourceBuffer,
    /// `SourceBufferAdded` has been logged for it (by the list callback, on GstMSE's thread).
    added_logged: bool,
    /// `SourceBufferRemoved` has been logged for it.
    removed_logged: bool,
}

#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
}

impl Registry {
    fn id_of(&self, buffer: &mse::SourceBuffer) -> Option<SourceBufferId> {
        self.entries
            .iter()
            .find(|entry| entry.buffer == *buffer)
            .map(|entry| entry.id)
    }

    fn buffer(&self, id: SourceBufferId) -> Option<mse::SourceBuffer> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.buffer.clone())
    }

    /// Adds `buffer` if it is new. Returns its id and whether it was new (then the caller connects its signals).
    fn register(
        &mut self,
        buffer: &mse::SourceBuffer,
        added_logged: bool,
    ) -> (SourceBufferId, bool) {
        if let Some(id) = self.id_of(buffer) {
            return (id, false);
        }
        let id = self.entries.len();
        self.entries.push(Entry {
            id,
            buffer: buffer.clone(),
            added_logged,
            removed_logged: false,
        });
        (id, true)
    }
}

/// Connects the five `SourceBuffer` signals to the log. Each handler logs on the thread GstMSE calls it on.
fn connect_source_buffer(log: &EventLog, id: SourceBufferId, buffer: &mse::SourceBuffer) {
    type MakeEvent = fn(SourceBufferId) -> MseEvent;
    let events: [(&str, MakeEvent); 5] = [
        ("start", MseEvent::UpdateStart),
        ("update", MseEvent::Update),
        ("end", MseEvent::UpdateEnd),
        ("abort", MseEvent::Abort),
        ("error", MseEvent::Error),
    ];
    for (which, make) in events {
        let log = log.clone();
        let handler = move |_: &mse::SourceBuffer| log.push(make(id));
        match which {
            "start" => buffer.connect_on_update_start(handler),
            "update" => buffer.connect_on_update(handler),
            "end" => buffer.connect_on_update_end(handler),
            "abort" => buffer.connect_on_abort(handler),
            _ => buffer.connect_on_error(handler),
        };
    }
}

/// Reconciles the registry with `list` (the `SourceBufferList` GstMSE just changed) and logs what
/// is new: `SourceBufferAdded` for a buffer seen in the list for the first time, `SourceBufferRemoved`
/// for one that was added and is no longer there. Runs on the thread GstMSE emitted the signal on.
fn on_list_changed(log: &EventLog, registry: &Weak<Mutex<Registry>>, list: &mse::SourceBufferList) {
    let Some(registry) = registry.upgrade() else {
        return;
    };
    // Query the list before taking our lock: nothing of ours is held while GstMSE is called.
    let present: Vec<mse::SourceBuffer> =
        (0..list.length()).filter_map(|i| list.index(i)).collect();
    let mut created = Vec::new();
    let mut events = Vec::new();
    {
        let mut registry = registry.lock().unwrap();
        for buffer in &present {
            let (id, is_new) = registry.register(buffer, true);
            if is_new {
                created.push((id, buffer.clone()));
                events.push(MseEvent::SourceBufferAdded(id));
            }
        }
        for entry in registry.entries.iter_mut() {
            let is_present = present.contains(&entry.buffer);
            if is_present && !entry.added_logged {
                entry.added_logged = true;
                events.push(MseEvent::SourceBufferAdded(entry.id));
            }
            if !is_present && entry.added_logged && !entry.removed_logged {
                entry.removed_logged = true;
                events.push(MseEvent::SourceBufferRemoved(entry.id));
            }
        }
    }
    for (id, buffer) in created {
        connect_source_buffer(log, id, &buffer);
    }
    for event in events {
        log.push(event);
    }
}

/// The `msesrc` the backend is attached to and the handlers connected on it.
struct Attached {
    src: mse::MseSrc,
    handlers: Vec<glib::SignalHandlerId>,
}

/// One `MediaSource` attached to one `msesrc`. All methods take `&self` and may be called from any
/// thread, which is what the DOM needs (script thread calls in, GStreamer threads call out); whether
/// GstMSE is safe under that is one of the things the spike measures. Cloning gives another handle to
/// the same `MediaSource` (the `source-setup` handler of the PlayBin shape needs one).
#[derive(Clone)]
pub struct MseBackend {
    s: Arc<Shared>,
}

struct Shared {
    log: EventLog,
    source: mse::MediaSource,
    /// Kept alive because its `added`/`removed` signals are connected.
    list: mse::SourceBufferList,
    registry: Arc<Mutex<Registry>>,
    attached: Mutex<Option<Attached>>,
    log_position: AtomicBool,
}

impl MseBackend {
    /// Creates a `GstMediaSource` in the `Closed` state and connects its signals to `log`.
    ///
    /// GStreamer must be initialised first (`env::init`): without it this returns [`MseError::Other`].
    pub fn new(log: EventLog) -> Result<Self, MseError> {
        if !gstreamer::INITIALIZED.load(Ordering::SeqCst) {
            return Err(MseError::Other(
                "GStreamer is not initialised: call env::init first".to_owned(),
            ));
        }
        let source = mse::MediaSource::new();
        {
            let log = log.clone();
            source.connect_on_source_open(move |_| log.push(MseEvent::SourceOpen));
        }
        {
            let log = log.clone();
            source.connect_on_source_ended(move |_| log.push(MseEvent::SourceEnded));
        }
        {
            let log = log.clone();
            source.connect_on_source_close(move |_| log.push(MseEvent::SourceClose));
        }
        let registry = Arc::new(Mutex::new(Registry::default()));
        let list = source.source_buffers();
        for added in [true, false] {
            let log = log.clone();
            let registry = Arc::downgrade(&registry);
            if added {
                list.connect_on_sourcebuffer_added(move |list| {
                    on_list_changed(&log, &registry, list)
                });
            } else {
                list.connect_on_sourcebuffer_removed(move |list| {
                    on_list_changed(&log, &registry, list)
                });
            }
        }
        Ok(MseBackend {
            s: Arc::new(Shared {
                log,
                source,
                list,
                registry,
                attached: Mutex::new(None),
                log_position: AtomicBool::new(false),
            }),
        })
    }

    /// `MediaSource.isTypeSupported`, for one type string such as `video/mp4; codecs="avc1.42C01E"`.
    /// `false` when GStreamer is not initialised.
    ///
    /// GstMSE 1.28.9 is generous (Q70 of `examples/api_probe.rs`): `video/foo` is supported, and so
    /// are `hev1`, `av01` and `vp9` codec strings and an `audio/mp4` type with an `avc1` codec,
    /// whatever decoders exist; only the empty string and a codec string GStreamer cannot parse
    /// (`codecs="nonsense"`) are refused. `add_source_buffer` accepts the same set (Q40: `video/foo`
    /// gives a source buffer, `""` gives `Type("supplied content type is empty")`). A page's
    /// `isTypeSupported` must therefore be answered by the DOM from its own codec table (EA2), with
    /// this as a floor, not as the answer.
    pub fn is_type_supported(mime: &str) -> bool {
        gstreamer::INITIALIZED.load(Ordering::SeqCst) && mse::MediaSource::is_type_supported(mime)
    }

    /// Also log `notify::position` of the attached `msesrc` as [`MseEvent::ElementPosition`]
    /// (off by default: it can fire for every buffer). Call before [`MseBackend::attach`].
    pub fn set_log_position(&self, enabled: bool) {
        self.s.log_position.store(enabled, Ordering::SeqCst);
    }

    /// Attaches to the `msesrc` element that playbin3 made (`source-setup`) or that the rig put in
    /// the pipeline. Also connects `notify::ready-state` and `notify::duration` of the element to the
    /// log (and `notify::position` after [`MseBackend::set_log_position`]).
    pub fn attach(&self, msesrc: &gstreamer::Element) -> Result<(), MseError> {
        let src = msesrc
            .clone()
            .downcast::<mse::MseSrc>()
            .map_err(|element| {
                MseError::Type(format!("element {} is not an msesrc", element.name()))
            })?;
        let mut handlers = Vec::new();
        {
            let log = self.s.log.clone();
            handlers.push(src.connect_ready_state_notify(move |src| {
                log.push(MseEvent::ElementReadyState(element_ready_state(
                    src.ready_state(),
                )))
            }));
        }
        {
            let log = self.s.log.clone();
            handlers.push(src.connect_duration_notify(move |src| {
                log.push(MseEvent::ElementDuration(src.duration()))
            }));
        }
        if self.s.log_position.load(Ordering::SeqCst) {
            let log = self.s.log.clone();
            handlers.push(src.connect_position_notify(move |src| {
                log.push(MseEvent::ElementPosition(src.position()))
            }));
        }
        self.s.source.attach(&src);
        *self.s.attached.lock().unwrap() = Some(Attached { src, handlers });
        Ok(())
    }

    /// Detaches from the `msesrc` (and stops logging its notifications). Idempotent.
    pub fn detach(&self) {
        let attached = self.s.attached.lock().unwrap().take();
        if let Some(attached) = attached {
            for handler in attached.handlers {
                attached.src.disconnect(handler);
            }
        }
        self.s.source.detach();
    }

    pub fn ready_state(&self) -> MediaSourceReadyState {
        match self.s.source.ready_state() {
            mse::MediaSourceReadyState::Open => MediaSourceReadyState::Open,
            mse::MediaSourceReadyState::Ended => MediaSourceReadyState::Ended,
            _ => MediaSourceReadyState::Closed,
        }
    }

    fn buffer(&self, id: SourceBufferId) -> Result<mse::SourceBuffer, MseError> {
        self.s
            .registry
            .lock()
            .unwrap()
            .buffer(id)
            .ok_or_else(|| not_a_source_buffer(id))
    }

    pub fn add_source_buffer(&self, mime: &str) -> Result<SourceBufferId, MseError> {
        let buffer = self
            .s
            .source
            .add_source_buffer(mime)
            .map_err(|e| map_error(&e))?;
        // The list callback may already have registered it (it runs on GstMSE's thread).
        let (id, is_new) = self.s.registry.lock().unwrap().register(&buffer, false);
        if is_new {
            connect_source_buffer(&self.s.log, id, &buffer);
        }
        Ok(id)
    }

    pub fn remove_source_buffer(&self, id: SourceBufferId) -> Result<(), MseError> {
        let buffer = self.buffer(id)?;
        self.s
            .source
            .remove_source_buffer(&buffer)
            .map_err(|e| map_error(&e))
    }

    /// `appendBuffer`: starts the update; `UpdateStart`, `Update`, `UpdateEnd` (or `Error`) follow in the log.
    ///
    /// The bytes are copied once into a new `GstBuffer` (`Buffer::from_slice` of a `Vec` copy), which
    /// GstMSE then owns: the caller's slice may be freed or reused the moment this returns.
    ///
    /// What GstMSE 1.28.9 does (measured with `examples/api_probe.rs`, Intel Mac, `Qnn` are its lines):
    ///
    /// * **`QuotaExceeded` is raised by `append` itself** (the `GError`, domain
    ///   `gst_media_source_error_quark`, message `buffer is full`), when a source buffer already holds
    ///   about 100 MiB of appended data: with the tiled `h264-low` fixture (7.4 KB per second of
    ///   media) the append number 7,508 was refused, after 104 MiB and 15,016 s of media (Q50). The
    ///   limit is on bytes held, not on duration. Nothing tells a page how full the buffer is. After
    ///   `remove(0, 10 min)` the next append was refused again (Q52, Q53), so a page that reacts to the
    ///   error the way dash.js does (remove behind the playhead, retry) would loop; whether the removal
    ///   frees the storage is for conformance scenario C10.
    /// * Bytes that are not a media segment (2 KB of `0x13`) are accepted: `UpdateStart`, `Update`,
    ///   `UpdateEnd`, no `Error` event, `updating` back to false at once (Q42). An empty slice is
    ///   accepted too (Q43).
    /// * `UpdateEnd` does not mean the data is in `buffered`: right after the fourth `UpdateEnd` of
    ///   an init segment plus four media segments sent back to back `buffered` was still empty, and it
    ///   reached `[0, 8 s]` within 500 ms (Q80). When segments come one at a time it follows `UpdateEnd`
    ///   by 12 to 15 ms (Q60). Poll `buffered` (or wait for the element's `ready-state`) before
    ///   deciding what to append next.
    /// * After `end_of_stream` an append is accepted and moves the media source back to `Open` (as
    ///   the spec says), but its `UpdateEnd` did not come within 10 s (Q45), and the next call, a
    ///   `remove_source_buffer`, never returned in a 4-minute run. Treat an append after
    ///   `endOfStream` as unsupported until C11 says otherwise.
    pub fn append(&self, id: SourceBufferId, data: &[u8]) -> Result<(), MseError> {
        let buffer = self.buffer(id)?;
        buffer
            .append_buffer(gstreamer::Buffer::from_slice(data.to_vec()))
            .map_err(|e| map_error(&e))
    }

    /// `SourceBuffer.updating`; `false` for an id the rig does not know.
    pub fn updating(&self, id: SourceBufferId) -> bool {
        self.buffer(id).is_ok_and(|buffer| buffer.is_updating())
    }

    pub fn abort(&self, id: SourceBufferId) -> Result<(), MseError> {
        self.buffer(id)?.abort().map_err(|e| map_error(&e))
    }

    /// `SourceBuffer.remove(start, end)`; `end` of `None` is +Infinity (passed to GstMSE as
    /// `GST_CLOCK_TIME_NONE`).
    ///
    /// GstMSE 1.28.9 (Q30 to Q36 of `examples/api_probe.rs`): `end = None` is accepted (Q33), as is a
    /// start after the end (Q35, the spec says `TypeError`) and a call while `updating` is true (Q36,
    /// the spec says `InvalidStateError`). On the one-track `h264-low` fixture `buffered` stayed
    /// `[0, 8 s]` for 1.5 s after `remove(0, 4 s)` and after `remove(6 s, None)` (Q32, Q34): the
    /// call returns `Ok` and no `UpdateStart`/`UpdateEnd` events were looked for here; scenario C10
    /// measures whether data is really removed.
    pub fn remove(
        &self,
        id: SourceBufferId,
        start: ClockTime,
        end: Option<ClockTime>,
    ) -> Result<(), MseError> {
        self.buffer(id)?
            .remove(Some(start), end)
            .map_err(|e| map_error(&e))
    }

    pub fn change_type(&self, id: SourceBufferId, mime: &str) -> Result<(), MseError> {
        self.buffer(id)?
            .change_content_type(mime)
            .map_err(|e| map_error(&e))
    }

    /// Signed nanoseconds, as the spec's double in seconds would be. A negative value is refused here
    /// with [`MseError::NotSupported`] before GstMSE is called, because GstMSE's offset is an unsigned
    /// `GstClockTime`; [`MseBackend::set_timestamp_offset_raw`] passes the unsigned value through.
    ///
    /// What GstMSE 1.28.9 does with negative offsets (Q1 to Q7 of `examples/api_probe.rs`): it has no
    /// negative offsets. `-1 s` as the unsigned value `18446744072709551616` ns is accepted and read
    /// back unchanged (Q3), and `GST_CLOCK_TIME_NONE` is accepted as an offset too (Q4), so a wrapper
    /// that does not refuse them stores nonsense silently; a page's `timestampOffset = -1` has to
    /// be handled by the DOM (shift the media itself, or throw). A positive offset is accepted and
    /// read back (Q5), but with `+8 s` set before the first init segment, four seconds of media
    /// appended afterwards showed as buffered `[0, 4 s]`, not `[8 s, 12 s]` (Q6): the offset was not
    /// applied in that order. C12 repeats this with the offset set after the init segment.
    pub fn set_timestamp_offset(&self, id: SourceBufferId, offset_ns: i64) -> Result<(), MseError> {
        let Ok(offset) = u64::try_from(offset_ns) else {
            return Err(MseError::NotSupported(format!(
                "negative timestampOffset ({offset_ns} ns): GstMSE's offset is an unsigned GstClockTime"
            )));
        };
        self.set_timestamp_offset_raw(id, Some(clock_time(offset)))
    }

    /// The unsigned call exactly as GstMSE has it (`None` is `GST_CLOCK_TIME_NONE`): what the spike
    /// uses to find out what GstMSE does with values the signed API refuses.
    pub fn set_timestamp_offset_raw(
        &self,
        id: SourceBufferId,
        offset: Option<ClockTime>,
    ) -> Result<(), MseError> {
        self.buffer(id)?
            .set_timestamp_offset(offset)
            .map_err(|e| map_error(&e))
    }

    /// `SourceBuffer.timestampOffset` as GstMSE holds it (`None` is `GST_CLOCK_TIME_NONE`).
    pub fn timestamp_offset(&self, id: SourceBufferId) -> Result<Option<ClockTime>, MseError> {
        Ok(self.buffer(id)?.timestamp_offset())
    }

    /// `appendWindowStart`/`appendWindowEnd`; an `end` of `None` is +Infinity
    /// (`GST_CLOCK_TIME_NONE`). The two are set one after the other (GstMSE has two setters, each
    /// checking against the other's current value): the end first unless the start fits under the
    /// current end; if the second fails the first stays set.
    ///
    /// What GstMSE 1.28.9 does (Q20 to Q29 of `examples/api_probe.rs`): the default window is
    /// `(0, None)`; **the start setter's check is inverted**. It answers `Type("append window start
    /// must be between zero and append window end")` for every start that is at or below the end
    /// (`0` with end `6 s`, `2 s` with end `6 s`, any start with end `None`, Q21 to Q24, Q27 to Q29)
    /// and accepts a start *after* the end (`7 s` with end `6 s`: `Ok`, read back as `(7 s, 6 s)`,
    /// Q23). So with this GstMSE a page cannot narrow the window from its default, `+Infinity` is
    /// reachable only as the default, and a window with start after end is possible: a bug to report
    /// upstream, and a reason for MED-1's wrapper to keep the window itself.
    pub fn set_append_window(
        &self,
        id: SourceBufferId,
        start: ClockTime,
        end: Option<ClockTime>,
    ) -> Result<(), MseError> {
        let buffer = self.buffer(id)?;
        let current_end = buffer.append_window_end();
        // GstMSE's start setter refuses every value while the end is +Infinity (`GST_CLOCK_TIME_NONE`):
        // the end has to be a real time first (measured, see `set_append_window`).
        let start_first = current_end.is_some_and(|current_end| start <= current_end);
        if start_first {
            buffer
                .set_append_window_start(Some(start))
                .map_err(|e| map_error(&e))?;
            buffer.set_append_window_end(end).map_err(|e| map_error(&e))
        } else {
            buffer
                .set_append_window_end(end)
                .map_err(|e| map_error(&e))?;
            buffer
                .set_append_window_start(Some(start))
                .map_err(|e| map_error(&e))
        }
    }

    /// The window as GstMSE holds it: `(start, end)`, `end` of `None` being +Infinity.
    pub fn append_window(
        &self,
        id: SourceBufferId,
    ) -> Result<(Option<ClockTime>, Option<ClockTime>), MseError> {
        let buffer = self.buffer(id)?;
        Ok((buffer.append_window_start(), buffer.append_window_end()))
    }

    pub fn set_append_mode(&self, id: SourceBufferId, mode: AppendMode) -> Result<(), MseError> {
        let mode = match mode {
            AppendMode::Segments => mse::SourceBufferAppendMode::Segments,
            AppendMode::Sequence => mse::SourceBufferAppendMode::Sequence,
        };
        self.buffer(id)?
            .set_append_mode(mode)
            .map_err(|e| map_error(&e))
    }

    pub fn append_mode(&self, id: SourceBufferId) -> Result<AppendMode, MseError> {
        Ok(match self.buffer(id)?.append_mode() {
            mse::SourceBufferAppendMode::Sequence => AppendMode::Sequence,
            _ => AppendMode::Segments,
        })
    }

    /// `SourceBuffer.buffered`. The binding crate does not wrap `gst_source_buffer_get_buffered`
    /// (it returns a `GArray`), so this calls the C function and copies the intervals out.
    pub fn buffered(&self, id: SourceBufferId) -> Result<Vec<TimeRange>, MseError> {
        use gstreamer_mse::ffi;
        let buffer = self.buffer(id)?;
        let mut error = std::ptr::null_mut();
        // SAFETY: `buffer` is a live GstSourceBuffer for the whole call; `error` is a valid out
        // pointer. The function returns a new GArray (transfer full) of `GstSourceBufferInterval`
        // (two `GstClockTime`), or NULL with `error` set. The array is read within its `len`
        // elements, then released with `g_array_unref`, once.
        let ranges = unsafe {
            let array = ffi::gst_source_buffer_get_buffered(buffer.to_glib_none().0, &mut error);
            if array.is_null() {
                if error.is_null() {
                    return Err(MseError::Other(
                        "gst_source_buffer_get_buffered returned NULL without an error".to_owned(),
                    ));
                }
                let error: glib::Error = glib::translate::from_glib_full(error);
                return Err(map_error(&error));
            }
            let len = (*array).len as usize;
            let data = (*array).data as *const ffi::GstSourceBufferInterval;
            let mut ranges = Vec::with_capacity(len);
            for index in 0..len {
                let interval = &*data.add(index);
                ranges.push(TimeRange {
                    start: clock_time(interval.start),
                    end: clock_time(interval.end),
                });
            }
            glib::ffi::g_array_unref(array);
            ranges
        };
        Ok(ranges)
    }

    /// `MediaSource.sourceBuffers.length`, as GstMSE counts them.
    pub fn source_buffer_count(&self) -> usize {
        self.s.list.length() as usize
    }

    /// `None` is "unknown" (NaN) or, for a live stream, +Infinity: GstMSE has one value for both,
    /// `GST_CLOCK_TIME_NONE`, which this returns as `None`.
    ///
    /// Measured (Q10 to Q16 of `examples/api_probe.rs`): `None` before the first init segment; after
    /// the init segment and the first media segment, the duration the init segment states (2 s for
    /// the fixtures, whose `mvhd` holds the first fragment's duration, not 8 s) and the same value
    /// in the `msesrc` `duration` property; it does not grow as more segments are appended.
    pub fn duration(&self) -> Option<ClockTime> {
        self.s.source.duration()
    }

    /// `None` sets `GST_CLOCK_TIME_NONE`, which GstMSE 1.28.9 accepts (Q13): `duration()` and the
    /// element's `duration` then both read `None`, so NaN and `+Infinity` are one state and the DOM
    /// has to remember which one the page set. A duration below the end of the buffered data is
    /// accepted as well (`1 s` with 8 s buffered, Q15; the spec says `InvalidStateError` unless the
    /// data is removed first), and a larger one too (`100 s`, Q14).
    pub fn set_duration(&self, duration: Option<ClockTime>) -> Result<(), MseError> {
        self.s
            .source
            .set_duration(duration)
            .map_err(|e| map_error(&e))
    }

    pub fn end_of_stream(&self, error: EosError) -> Result<(), MseError> {
        let error = match error {
            EosError::None => mse::MediaSourceEOSError::None,
            EosError::Network => mse::MediaSourceEOSError::Network,
            EosError::Decode => mse::MediaSourceEOSError::Decode,
        };
        self.s
            .source
            .end_of_stream(error)
            .map_err(|e| map_error(&e))
    }

    pub fn set_live_seekable_range(
        &self,
        start: ClockTime,
        end: ClockTime,
    ) -> Result<(), MseError> {
        self.s
            .source
            .set_live_seekable_range(Some(start), Some(end))
            .map_err(|e| map_error(&e))
    }

    pub fn clear_live_seekable_range(&self) -> Result<(), MseError> {
        self.s
            .source
            .clear_live_seekable_range()
            .map_err(|e| map_error(&e))
    }

    /// The live seekable range as GstMSE holds it.
    pub fn live_seekable_range(&self) -> TimeRange {
        let range = self.s.source.live_seekable_range();
        TimeRange {
            start: range.start(),
            end: range.end(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mse_errors_have_their_dom_exception_names() {
        assert_eq!(
            MseError::InvalidState("x".into()).dom_exception(),
            Some("InvalidStateError")
        );
        assert_eq!(
            MseError::QuotaExceeded("x".into()).dom_exception(),
            Some("QuotaExceededError")
        );
        assert_eq!(MseError::Other("x".into()).dom_exception(), None);
        assert_eq!(MseError::NotFound("why".into()).message(), "why");
    }

    #[test]
    fn each_gstmse_error_code_maps_to_its_class_with_the_message_kept() {
        let cases = [
            (mse::MediaSourceError::InvalidState, "invalid"),
            (mse::MediaSourceError::Type, "type"),
            (mse::MediaSourceError::NotSupported, "unsupported"),
            (mse::MediaSourceError::NotFound, "missing"),
            (mse::MediaSourceError::QuotaExceeded, "buffer is full"),
        ];
        let mapped: Vec<MseError> = cases
            .iter()
            .map(|(code, message)| map_error(&glib::Error::new(*code, message)))
            .collect();
        assert_eq!(
            mapped,
            vec![
                MseError::InvalidState("invalid".into()),
                MseError::Type("type".into()),
                MseError::NotSupported("unsupported".into()),
                MseError::NotFound("missing".into()),
                MseError::QuotaExceeded("buffer is full".into()),
            ]
        );
    }

    #[test]
    fn an_error_of_another_domain_or_an_unknown_code_is_other_with_the_details() {
        let other = map_error(&glib::Error::new(glib::FileError::Exist, "already there"));
        assert!(
            matches!(&other, MseError::Other(text) if text.contains("already there")),
            "{other:?}"
        );
        assert_eq!(other.dom_exception(), None);
        let unknown = map_error(&glib::Error::new(
            mse::MediaSourceError::__Unknown(99),
            "from the future",
        ));
        assert!(
            matches!(&unknown, MseError::Other(text) if text.contains("from the future")),
            "{unknown:?}"
        );
        assert_eq!(
            MseError::QuotaExceeded("buffer is full".into()).to_string(),
            "QuotaExceededError: buffer is full"
        );
    }

    #[test]
    fn clock_time_never_panics_on_the_none_value() {
        assert_eq!(clock_time(5).nseconds(), 5);
        assert_eq!(clock_time(u64::MAX).nseconds(), u64::MAX - 1);
    }

    #[test]
    fn a_backend_cannot_be_made_before_gstreamer_is_initialised() {
        // This test binary never calls `env::init`, so GStreamer is down: the error is a value, not a panic.
        assert!(!gstreamer::INITIALIZED.load(Ordering::SeqCst));
        assert!(matches!(
            MseBackend::new(EventLog::new()),
            Err(MseError::Other(_))
        ));
        assert!(!MseBackend::is_type_supported("video/mp4"));
    }
}
