// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The two pipeline shapes under test (item 1).
//!
//! * [`Shape::Bare`]: `msesrc ! decodebin3 ! (videoconvertscale ! appsink | appsink)`. Isolates
//!   GstMSE from playbin3.
//! * [`Shape::PlayBin`]: how Servo does it (`third_party/servo-media-gstreamer/player.rs`): a
//!   `gstreamer_play::Play` over `playbin3`, `uri = "mse://..."`, the `source-setup` signal attaches the
//!   `MediaSource` to the `msesrc` playbin3 created, video and audio sinks are `appsink`s, a
//!   `PlaySignalAdapter::new_sync_emit` for messages, and no GLib main loop unless the options ask for one.
//!
//! Nothing here opens a window or an audio device; the clock is the pipeline's own.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::ClockTime;
use gstreamer::prelude::*;
use gstreamer_mse as mse;
use gstreamer_play as gst_play;

use crate::RigError;
use crate::backend::{self, MseBackend};
use crate::events::{ElementReadyState, EventLog, MseEvent, PlayEvent, SourceBufferId};
use crate::frames::{CountingSink, FrameStats, SharedStats};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Bare,
    PlayBin,
}

impl Shape {
    /// `bare` or `playbin`, as on the command line.
    pub fn label(self) -> &'static str {
        match self {
            Shape::Bare => "bare",
            Shape::PlayBin => "playbin",
        }
    }

    /// Parses the command-line spelling.
    pub fn parse(text: &str) -> Option<Shape> {
        match text {
            "bare" => Some(Shape::Bare),
            "playbin" => Some(Shape::PlayBin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RigConfig {
    pub shape: Shape,
    /// `true`: sinks synchronise to the clock (real-time playback). `false`: as fast as possible.
    pub sync: bool,
    /// A decoder to prefer by raising its rank (for example `avdec_h264`, `vtdec_hw`); `None` leaves
    /// the ranks as they are.
    pub prefer_decoder: Option<String>,
    /// Make a video sink at all (`false` for audio-only scenarios).
    pub video: bool,
    pub audio: bool,
}

impl Default for RigConfig {
    fn default() -> Self {
        RigConfig {
            shape: Shape::PlayBin,
            sync: true,
            prefer_decoder: None,
            video: true,
            audio: true,
        }
    }
}

/// How long one append took, from the call to the matching `UpdateEnd`.
#[derive(Debug, Clone, Copy)]
pub struct AppendTiming {
    /// From the call to the first `UpdateStart` of this source buffer after it (equal to
    /// `to_update_end` when none was seen).
    pub to_update_start: Duration,
    pub to_update_end: Duration,
}

/// What the video sink dropped for being late: the data behind `getVideoPlaybackQuality()`
/// (`totalVideoFrames`, `droppedVideoFrames`). Taken from the video `appsink`'s `stats` property
/// (`processed` = rendered + dropped), not from bus QoS messages: GstPlay owns the bus in the PlayBin shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QosStats {
    pub processed: u64,
    pub dropped: u64,
}

/// What a shutdown found.
#[derive(Debug, Clone, Default)]
pub struct ShutdownInfo {
    pub took: Duration,
    /// Warnings or errors posted during teardown (GstPlay or bus messages, and source buffer `error`
    /// events), as text.
    pub messages: Vec<String>,
}

/// The order in which [`Rig::shutdown_with`] tears things down (T04 runs each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeardownOrder {
    /// The pipeline goes to NULL, then the backend is dropped. What [`Rig::shutdown`] does.
    PipelineNullThenBackend,
    /// The backend is dropped first, then the pipeline goes to NULL.
    BackendThenPipelineNull,
    /// The backend detaches from `msesrc` while the pipeline is still running, then NULL, then drop.
    DetachThenNull,
    /// Nothing is set to NULL: everything is dropped and GStreamer's dispose has to cope.
    DropWithoutNull,
}

/// The `uri` the PlayBin shape gives playbin3. Any text after `mse://` is accepted by `msesrc`.
pub const MSE_URI: &str = "mse://mercury-rig";

/// What `Rig::new` waits for before it gives up on `SourceOpen`.
const SOURCE_OPEN_TIMEOUT: Duration = Duration::from_secs(10);

struct PlayParts {
    play: gst_play::Play,
    /// Kept alive: its signal handlers feed the log.
    _adapter: gst_play::PlaySignalAdapter,
}

/// Everything about the pipeline that is not the public contract's fields.
struct Core {
    config: RigConfig,
    pipeline: gst::Element,
    play: Option<PlayParts>,
    /// The `msesrc` (set by `source-setup` in the PlayBin shape, at construction in the Bare shape).
    msesrc: Arc<Mutex<Option<gst::Element>>>,
    video_sink: Arc<Mutex<Option<CountingSink>>>,
    /// Bare shape only: a seek was requested and its `AsyncDone` has not come yet.
    seek_pending: Arc<AtomicBool>,
    /// The pipeline has been taken to NULL (or must not be touched again).
    torn_down: bool,
}

impl Core {
    fn set_null(&mut self) -> Result<(), RigError> {
        if let Some(parts) = &self.play {
            parts.play.stop();
        }
        let result = self.pipeline.set_state(gst::State::Null);
        self.torn_down = true;
        result
            .map(|_| ())
            .map_err(|error| RigError::Pipeline(format!("set_state(Null): {error}")))
    }
}

/// How long a drop waits for the pipeline to reach NULL before it gives up and leaks it.
const DROP_NULL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long [`Rig::shutdown`] waits for the teardown before it reports a hang.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

/// Sets `pipeline` to NULL on a helper thread and waits `limit` for it. `false` when it did not
/// finish (GstMSE can hang a state change: see the notes at the top of the crate's README), in which
/// case the helper thread is left blocked.
fn set_null_with_watchdog(pipeline: &gst::Element, limit: Duration) -> bool {
    let (sender, receiver) = mpsc::channel();
    let pipeline = pipeline.clone();
    let spawned = std::thread::Builder::new()
        .name("mse-rig-null".to_owned())
        .spawn(move || {
            let _ = pipeline.set_state(gst::State::Null);
            let _ = sender.send(());
        });
    spawned.is_ok() && receiver.recv_timeout(limit).is_ok()
}

impl Drop for Core {
    fn drop(&mut self) {
        if self.torn_down {
            return;
        }
        // A rig that is simply dropped still leaves GStreamer a pipeline in NULL. If that hangs (it
        // does when GstMSE is deadlocked), the pipeline and the player are leaked on purpose: their
        // own disposal would repeat the state change and hang this thread for good.
        if !set_null_with_watchdog(&self.pipeline, DROP_NULL_TIMEOUT) {
            std::mem::forget(self.pipeline.clone());
            std::mem::forget(self.play.take());
        }
    }
}

/// One pipeline with one `MediaSource` attached.
///
/// Dropping a `Rig` without calling [`Rig::shutdown`] drops the backend first and then takes the
/// pipeline to NULL (the field order); `shutdown` does it the other way round.
pub struct Rig {
    pub backend: MseBackend,
    pub log: EventLog,
    pub video: SharedStats,
    pub audio: SharedStats,
    core: Core,
}

fn pipeline_error(what: &str, error: impl std::fmt::Display) -> RigError {
    RigError::Pipeline(format!("{what}: {error}"))
}

/// Raises the rank of the element factory `name` above every decoder Mercury ships.
fn raise_rank(name: &str) -> Result<(), RigError> {
    let feature = gst::Registry::get()
        .lookup_feature(name)
        .ok_or_else(|| RigError::Setup(format!("prefer_decoder: no element factory {name:?}")))?;
    feature.set_rank(gst::Rank::from(1000));
    Ok(())
}

impl Rig {
    /// Builds the pipeline, attaches the `MediaSource`, and returns once `SourceOpen` is in the log.
    ///
    /// The pipeline has been asked to go to PAUSED when this returns: with no data appended GstMSE
    /// has no pads, so the change stays pending (ASYNC) until the first segments of every track have
    /// been appended, as a real page's `loadedmetadata` waits for the first initialisation segment.
    pub fn new(config: RigConfig) -> Result<Rig, RigError> {
        let log = EventLog::new();
        let backend = MseBackend::new(log.clone())?;
        if let Some(name) = &config.prefer_decoder {
            raise_rank(name)?;
        }
        let video = FrameStats::shared();
        let audio = FrameStats::shared();
        let core = match config.shape {
            Shape::PlayBin => build_playbin(&config, &backend, &log, &video, &audio)?,
            Shape::Bare => build_bare(&config, &backend, &log, &video, &audio)?,
        };
        let rig = Rig {
            backend,
            log,
            video,
            audio,
            core,
        };
        let opened = rig.log.wait_for(SOURCE_OPEN_TIMEOUT, |events| {
            events.iter().any(|e| e.event == MseEvent::SourceOpen)
        });
        if !opened {
            let errors: Vec<String> = rig
                .log
                .snapshot()
                .into_iter()
                .filter_map(|timed| match timed.event {
                    MseEvent::Play(PlayEvent::Error(text) | PlayEvent::Warning(text)) => Some(text),
                    _ => None,
                })
                .collect();
            return Err(RigError::Timeout(
                SOURCE_OPEN_TIMEOUT,
                format!("SourceOpen (pipeline messages: {errors:?})"),
            ));
        }
        Ok(rig)
    }

    pub fn config(&self) -> &RigConfig {
        &self.core.config
    }

    pub fn play(&self) -> Result<(), RigError> {
        match &self.core.play {
            Some(parts) => {
                parts.play.play();
                Ok(())
            }
            None => set_state(&self.core.pipeline, gst::State::Playing),
        }
    }

    pub fn pause(&self) -> Result<(), RigError> {
        match &self.core.play {
            Some(parts) => {
                parts.play.pause();
                Ok(())
            }
            None => set_state(&self.core.pipeline, gst::State::Paused),
        }
    }

    /// A flushing, accurate seek to `to` (`HTMLMediaElement.currentTime = to`); returns once it is
    /// requested. PlayBin: `Play::seek` (the config has `seek_accurate`); Bare: `seek_simple(FLUSH |
    /// ACCURATE)`, and the next `AsyncDone` is logged as `PlayEvent::SeekDone`.
    pub fn seek(&self, to: ClockTime) -> Result<(), RigError> {
        match &self.core.play {
            Some(parts) => {
                parts.play.seek(to);
                Ok(())
            }
            None => {
                self.core.seek_pending.store(true, Ordering::SeqCst);
                self.core
                    .pipeline
                    .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, to)
                    .map_err(|error| {
                        self.core.seek_pending.store(false, Ordering::SeqCst);
                        pipeline_error("seek_simple", error)
                    })
            }
        }
    }

    /// The pipeline's position query (`None` before the pipeline can answer).
    pub fn position(&self) -> Option<ClockTime> {
        self.core.pipeline.query_position::<ClockTime>()
    }

    /// The `msesrc`'s `ready-state` (the media element's `readyState`); `HaveNothing` before it exists.
    pub fn element_ready_state(&self) -> ElementReadyState {
        self.msesrc()
            .and_then(|element| element.downcast::<mse::MseSrc>().ok())
            .map_or(ElementReadyState::HaveNothing, |src| {
                backend::element_ready_state(src.ready_state())
            })
    }

    /// The `msesrc`'s `duration` property (`None` is unknown).
    pub fn element_duration(&self) -> Option<ClockTime> {
        self.msesrc()
            .and_then(|element| element.downcast::<mse::MseSrc>().ok())
            .and_then(|src| src.duration())
    }

    /// Appends `data` to source buffer `id` and waits for its `UpdateEnd`.
    pub fn append_and_wait(
        &self,
        id: SourceBufferId,
        data: &[u8],
        timeout: Duration,
    ) -> Result<AppendTiming, RigError> {
        let from = self.log.len();
        let called = self.log.elapsed();
        self.backend.append(id, data)?;
        let finished = self.log.wait_for(timeout, |events| {
            events
                .get(from..)
                .is_some_and(|tail| tail.iter().any(|e| e.event == MseEvent::UpdateEnd(id)))
        });
        if !finished {
            return Err(RigError::Timeout(
                timeout,
                format!(
                    "UpdateEnd of source buffer {id} after appending {} bytes",
                    data.len()
                ),
            ));
        }
        let tail = self.log.snapshot_from(from);
        let at_of = |wanted: MseEvent| tail.iter().find(|e| e.event == wanted).map(|e| e.at);
        let end = at_of(MseEvent::UpdateEnd(id)).unwrap_or(called);
        let start = at_of(MseEvent::UpdateStart(id)).unwrap_or(end);
        Ok(AppendTiming {
            to_update_start: start.saturating_sub(called),
            to_update_end: end.saturating_sub(called),
        })
    }

    /// Polls `predicate` (every 10 ms) until it is true or `timeout` passes; returns whether it became true.
    pub fn wait_until(&self, timeout: Duration, predicate: impl Fn(&Rig) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if predicate(self) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits until the log has a `PlayEvent::EndOfStream` (either shape) or `timeout` passes.
    pub fn wait_for_eos(&self, timeout: Duration) -> bool {
        self.log.wait_for(timeout, |events| {
            events
                .iter()
                .any(|e| e.event == MseEvent::Play(PlayEvent::EndOfStream))
        })
    }

    /// The top-level element (`playbin3` or the bare pipeline), for pad probes and queries.
    pub fn pipeline(&self) -> gst::Element {
        self.core.pipeline.clone()
    }

    /// The `msesrc` element, once it exists (in the PlayBin shape playbin3 makes it).
    pub fn msesrc(&self) -> Option<gst::Element> {
        self.core.msesrc.lock().unwrap().clone()
    }

    pub fn qos(&self) -> QosStats {
        let sink = self.core.video_sink.lock().unwrap();
        match sink.as_ref().and_then(CountingSink::rendered_and_dropped) {
            Some((rendered, dropped)) => QosStats {
                processed: rendered + dropped,
                dropped,
            },
            None => QosStats::default(),
        }
    }

    /// The factory names of every element in the pipeline, recursively, such as `msesrc0:msesrc`,
    /// `vtdec_hw0:vtdec_hw`: how a scenario finds out which decoder was chosen. The top-level element
    /// comes first.
    pub fn describe_pipeline(&self) -> Vec<String> {
        let mut lines = vec![describe(&self.core.pipeline)];
        if let Some(bin) = self.core.pipeline.downcast_ref::<gst::Bin>() {
            lines.extend(
                bin.iterate_recurse()
                    .into_iter()
                    .filter_map(Result::ok)
                    .map(|e| describe(&e)),
            );
        }
        lines
    }

    /// The elements of [`Rig::describe_pipeline`] whose factory is a decoder (class `Decoder`, not a bin
    /// such as `decodebin3`), as `vtdechw0:vtdec_hw` or `avdec_aac0:avdec_aac`.
    pub fn chosen_decoders(&self) -> Vec<String> {
        let mut found = Vec::new();
        if let Some(bin) = self.core.pipeline.downcast_ref::<gst::Bin>() {
            for element in bin.iterate_recurse().into_iter().filter_map(Result::ok) {
                let is_decoder = element.factory().is_some_and(|factory| {
                    let klass = factory.klass();
                    klass.contains("Decoder") && !klass.contains("Bin")
                });
                if is_decoder {
                    found.push(describe(&element));
                }
            }
        }
        found
    }

    /// The pipeline's current state (not the pending one).
    pub fn pipeline_state(&self) -> gst::State {
        self.core.pipeline.state(Some(gst::ClockTime::ZERO)).1
    }

    /// Sets the pipeline to NULL and drops everything, in the order a page's teardown would: the
    /// pipeline first, then the backend ([`TeardownOrder::PipelineNullThenBackend`]).
    pub fn shutdown(self) -> Result<ShutdownInfo, RigError> {
        self.shutdown_with(TeardownOrder::PipelineNullThenBackend)
    }

    /// Tears the rig down in the given order and reports how long it took and the warnings and errors
    /// posted meanwhile. Returns `Err` if the pipeline refused to go to NULL (everything is dropped
    /// anyway), or `Err(RigError::Timeout)` after [`SHUTDOWN_TIMEOUT`] if the teardown hangs, as it
    /// does in GstMSE 1.28.9 when the rig is in the split A/V deadlock: the teardown runs on a helper
    /// thread, which is left blocked (and the rig leaked) on a timeout.
    pub fn shutdown_with(self, order: TeardownOrder) -> Result<ShutdownInfo, RigError> {
        self.shutdown_with_timeout(order, SHUTDOWN_TIMEOUT)
    }

    /// [`Rig::shutdown_with`] with a limit of your choice.
    pub fn shutdown_with_timeout(
        self,
        order: TeardownOrder,
        limit: Duration,
    ) -> Result<ShutdownInfo, RigError> {
        let started = Instant::now();
        let log = self.log.clone();
        let from = log.len();
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("mse-rig-teardown".to_owned())
            .spawn(move || {
                let _ = sender.send(self.teardown(order));
            });
        if let Err(error) = spawned {
            return Err(RigError::Pipeline(format!(
                "cannot start the teardown thread: {error}"
            )));
        }
        let messages = || -> Vec<String> {
            log.snapshot_from(from)
                .into_iter()
                .filter_map(|timed| match timed.event {
                    MseEvent::Play(PlayEvent::Error(text)) => Some(format!("error: {text}")),
                    MseEvent::Play(PlayEvent::Warning(text)) => Some(format!("warning: {text}")),
                    MseEvent::Error(id) => Some(format!("source buffer {id} error event")),
                    _ => None,
                })
                .collect()
        };
        match receiver.recv_timeout(limit) {
            Ok(result) => result.map(|()| ShutdownInfo {
                took: started.elapsed(),
                messages: messages(),
            }),
            Err(_) => Err(RigError::Timeout(
                limit,
                format!(
                    "the teardown ({order:?}); the rig and its helper thread are leaked; messages so far: {:?}",
                    messages()
                ),
            )),
        }
    }

    /// The teardown itself, on whatever thread calls it.
    fn teardown(self, order: TeardownOrder) -> Result<(), RigError> {
        let Rig {
            backend,
            log,
            video,
            audio,
            mut core,
        } = self;
        let mut result = Ok(());
        match order {
            TeardownOrder::PipelineNullThenBackend => {
                result = core.set_null();
                drop(backend);
            }
            TeardownOrder::BackendThenPipelineNull => {
                drop(backend);
                result = core.set_null();
            }
            TeardownOrder::DetachThenNull => {
                backend.detach();
                result = core.set_null();
                drop(backend);
            }
            TeardownOrder::DropWithoutNull => {
                core.torn_down = true;
                drop(backend);
            }
        }
        drop(core);
        drop((log, video, audio));
        result
    }
}

fn describe(element: &gst::Element) -> String {
    let factory = element
        .factory()
        .map_or_else(|| "?".to_owned(), |factory| factory.name().to_string());
    format!("{}:{}", element.name(), factory)
}

fn set_state(pipeline: &gst::Element, state: gst::State) -> Result<(), RigError> {
    pipeline
        .set_state(state)
        .map(|_| ())
        .map_err(|error| pipeline_error(&format!("set_state({state:?})"), error))
}

// ----------------------------------------------------------------------------------------------
// PlayBin shape
// ----------------------------------------------------------------------------------------------

fn build_playbin(
    config: &RigConfig,
    backend: &MseBackend,
    log: &EventLog,
    video_stats: &SharedStats,
    audio_stats: &SharedStats,
) -> Result<Core, RigError> {
    for factory in ["playbin3", "decodebin3", "msesrc"] {
        if gst::ElementFactory::find(factory).is_none() {
            return Err(RigError::Setup(format!(
                "missing element factory {factory}"
            )));
        }
    }
    let play = gst_play::Play::default();
    let adapter = gst_play::PlaySignalAdapter::new_sync_emit(&play);
    let pipeline = play.pipeline();

    let video_sink = CountingSink::video("rig-video", video_stats.clone(), config.sync, log)?;
    let audio_sink = CountingSink::audio("rig-audio", audio_stats.clone(), config.sync, log)?;
    // Both sinks are always set, so that playbin3 never auto-plugs a window or an audio device.
    pipeline.set_property("video-sink", &video_sink.element);
    pipeline.set_property("audio-sink", &audio_sink.element);

    // As Servo does: a 0.5 s position update interval. Accurate seeks, so that a picture after a seek
    // is the one asked for.
    let mut play_config = play.config();
    play_config.set_position_update_interval(500);
    play_config.set_seek_accurate(true);
    play.set_config(play_config)
        .map_err(|error| pipeline_error("Play::set_config", error))?;
    play.set_video_track_enabled(config.video);
    play.set_audio_track_enabled(config.audio);

    connect_play_events(&adapter, log);

    let msesrc = Arc::new(Mutex::new(None));
    {
        let msesrc = msesrc.clone();
        let backend = backend.clone();
        let log = log.clone();
        pipeline.connect("source-setup", false, move |args| {
            if let Some(source) = args
                .get(1)
                .and_then(|value| value.get::<gst::Element>().ok())
            {
                if let Err(error) = backend.attach(&source) {
                    log.push(MseEvent::Play(PlayEvent::Error(format!(
                        "attaching the MediaSource in source-setup failed: {error:?}"
                    ))));
                }
                *msesrc.lock().unwrap() = Some(source);
            }
            None
        });
    }

    play.set_uri(Some(MSE_URI));
    play.pause();

    Ok(Core {
        config: config.clone(),
        pipeline,
        play: Some(PlayParts {
            play,
            _adapter: adapter,
        }),
        msesrc,
        video_sink: Arc::new(Mutex::new(Some(video_sink))),
        seek_pending: Arc::new(AtomicBool::new(false)),
        torn_down: false,
    })
}

fn connect_play_events(adapter: &gst_play::PlaySignalAdapter, log: &EventLog) {
    let push = |log: &EventLog, event: PlayEvent| log.push(MseEvent::Play(event));
    {
        let log = log.clone();
        adapter.connect_end_of_stream(move |_| push(&log, PlayEvent::EndOfStream));
    }
    {
        let log = log.clone();
        adapter.connect_error(move |_, error, _| push(&log, PlayEvent::Error(error.to_string())));
    }
    {
        let log = log.clone();
        adapter
            .connect_warning(move |_, error, _| push(&log, PlayEvent::Warning(error.to_string())));
    }
    {
        let log = log.clone();
        adapter.connect_state_changed(move |_, state| {
            push(&log, PlayEvent::StateChanged(format!("{state:?}")))
        });
    }
    {
        let log = log.clone();
        adapter.connect_buffering(move |_, percent| {
            push(&log, PlayEvent::Buffering(percent.clamp(0, 100) as u8))
        });
    }
    {
        let log = log.clone();
        adapter
            .connect_seek_done(move |_, position| push(&log, PlayEvent::SeekDone(Some(position))));
    }
    {
        let log = log.clone();
        adapter.connect_duration_changed(move |_, duration| {
            push(&log, PlayEvent::DurationChanged(duration))
        });
    }
    {
        let log = log.clone();
        adapter.connect_position_updated(move |_, position| {
            push(&log, PlayEvent::PositionUpdated(position))
        });
    }
    {
        let log = log.clone();
        adapter.connect_media_info_updated(move |_, _| push(&log, PlayEvent::MediaInfoUpdated));
    }
}

// ----------------------------------------------------------------------------------------------
// Bare shape
// ----------------------------------------------------------------------------------------------

/// What the `pad-added` handlers of the Bare shape need.
#[derive(Clone)]
struct BareContext {
    pipeline: gst::glib::WeakRef<gst::Pipeline>,
    log: EventLog,
    video_stats: SharedStats,
    audio_stats: SharedStats,
    video_sink: Arc<Mutex<Option<CountingSink>>>,
    audio_sink: Arc<Mutex<Option<CountingSink>>>,
    /// Every discarding `fakesink` made, so that extra or unwanted pads have somewhere to go.
    discards: Arc<Mutex<Vec<gst::Element>>>,
    sync: bool,
    want_video: bool,
    want_audio: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Video,
    Audio,
    Other,
}

fn kind_of(pad: &gst::Pad) -> Kind {
    let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
    match caps
        .structure(0)
        .map(|structure| structure.name().to_string())
    {
        Some(name) if name.starts_with("video/") => Kind::Video,
        Some(name) if name.starts_with("audio/") => Kind::Audio,
        _ => Kind::Other,
    }
}

impl BareContext {
    /// Links a raw pad from `decodebin3` to a sink made on demand: an unlinked asynchronous sink
    /// would stop the pipeline from prerolling, so sinks exist only for pads that exist.
    fn link_decoded(&self, pad: &gst::Pad) -> Result<(), String> {
        let pipeline = self.pipeline.upgrade().ok_or("the pipeline is gone")?;
        let kind = kind_of(pad);
        let element = match kind {
            Kind::Video if self.want_video => {
                let mut slot = self.video_sink.lock().unwrap();
                if slot.is_some() {
                    self.discard_sink(&pipeline)?
                } else {
                    let sink = CountingSink::video(
                        "rig-video",
                        self.video_stats.clone(),
                        self.sync,
                        &self.log,
                    )
                    .map_err(|e| e.to_string())?;
                    let element = sink.element.clone();
                    *slot = Some(sink);
                    element
                }
            }
            Kind::Audio if self.want_audio => {
                let mut slot = self.audio_sink.lock().unwrap();
                if slot.is_some() {
                    self.discard_sink(&pipeline)?
                } else {
                    let sink = CountingSink::audio(
                        "rig-audio",
                        self.audio_stats.clone(),
                        self.sync,
                        &self.log,
                    )
                    .map_err(|e| e.to_string())?;
                    let element = sink.element.clone();
                    *slot = Some(sink);
                    element
                }
            }
            _ => self.discard_sink(&pipeline)?,
        };
        // A discard sink was added to the pipeline already; the counting sinks are added here.
        if element.parent().is_none() {
            pipeline.add(&element).map_err(|e| e.to_string())?;
        }
        element
            .sync_state_with_parent()
            .map_err(|e| e.to_string())?;
        let sink_pad = element
            .static_pad("sink")
            .ok_or("the sink has no sink pad")?;
        pad.link(&sink_pad).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn discard_sink(&self, pipeline: &gst::Pipeline) -> Result<gst::Element, String> {
        let count = self.discards.lock().unwrap().len();
        let sink = CountingSink::discard(&format!("rig-discard{count}"), self.sync)
            .map_err(|e| e.to_string())?;
        pipeline.add(&sink.element).map_err(|e| e.to_string())?;
        self.discards.lock().unwrap().push(sink.element.clone());
        Ok(sink.element)
    }
}

fn build_bare(
    config: &RigConfig,
    backend: &MseBackend,
    log: &EventLog,
    video_stats: &SharedStats,
    audio_stats: &SharedStats,
) -> Result<Core, RigError> {
    let pipeline = gst::Pipeline::with_name("mse-rig-bare");
    let msesrc = gst::ElementFactory::make("msesrc")
        .name("msesrc0")
        .build()
        .map_err(|error| pipeline_error("msesrc", error))?;
    let decodebin = gst::ElementFactory::make("decodebin3")
        .name("decodebin3-0")
        .build()
        .map_err(|error| pipeline_error("decodebin3", error))?;
    pipeline
        .add_many([&msesrc, &decodebin])
        .map_err(|error| pipeline_error("add", error))?;

    let context = BareContext {
        pipeline: pipeline.downgrade(),
        log: log.clone(),
        video_stats: video_stats.clone(),
        audio_stats: audio_stats.clone(),
        video_sink: Arc::new(Mutex::new(None)),
        audio_sink: Arc::new(Mutex::new(None)),
        discards: Arc::new(Mutex::new(Vec::new())),
        sync: config.sync,
        want_video: config.video,
        want_audio: config.audio,
    };
    let video_slot = context.video_sink.clone();

    // msesrc pads (one per track, made after its initialisation segment) go to decodebin3 request pads.
    {
        let decodebin = decodebin.clone();
        let log = log.clone();
        msesrc.connect_pad_added(move |_src, pad| {
            let result = decodebin
                .request_pad_simple("sink_%u")
                .ok_or_else(|| "decodebin3 gave no request pad".to_owned())
                .and_then(|sink| pad.link(&sink).map(|_| ()).map_err(|e| e.to_string()));
            if let Err(error) = result {
                log.push(MseEvent::Play(PlayEvent::Error(format!(
                    "linking msesrc pad {} to decodebin3: {error}",
                    pad.name()
                ))));
            }
        });
    }
    {
        let context = context.clone();
        decodebin.connect_pad_added(move |_dbin, pad| {
            if pad.direction() != gst::PadDirection::Src {
                return;
            }
            if let Err(error) = context.link_decoded(pad) {
                context.log.push(MseEvent::Play(PlayEvent::Error(format!(
                    "linking decoded pad {}: {error}",
                    pad.name()
                ))));
            }
        });
    }

    let seek_pending = Arc::new(AtomicBool::new(false));
    connect_bus(&pipeline, log, &seek_pending)?;

    backend.attach(&msesrc)?;
    let slot = Arc::new(Mutex::new(Some(msesrc)));
    let pipeline_element: gst::Element = pipeline.upcast();
    set_state(&pipeline_element, gst::State::Paused)?;

    Ok(Core {
        config: config.clone(),
        pipeline: pipeline_element,
        play: None,
        msesrc: slot,
        video_sink: video_slot,
        seek_pending,
        torn_down: false,
    })
}

/// Turns the Bare pipeline's bus messages into [`PlayEvent`]s, on the thread that posts them (a sync
/// handler: no GLib main loop is needed).
fn connect_bus(
    pipeline: &gst::Pipeline,
    log: &EventLog,
    seek_pending: &Arc<AtomicBool>,
) -> Result<(), RigError> {
    let bus = pipeline
        .bus()
        .ok_or_else(|| RigError::Pipeline("the pipeline has no bus".to_owned()))?;
    let log = log.clone();
    let seek_pending = seek_pending.clone();
    let weak = pipeline.downgrade();
    bus.set_sync_handler(move |_bus, message| {
        use gst::MessageView;
        let from_pipeline = message
            .src()
            .zip(weak.upgrade())
            .is_some_and(|(src, pipeline)| src == pipeline.upcast_ref::<gst::Object>());
        let event = match message.view() {
            MessageView::Eos(_) => Some(PlayEvent::EndOfStream),
            MessageView::Error(error) => Some(PlayEvent::Error(format!(
                "{} ({:?})",
                error.error(),
                error.debug()
            ))),
            MessageView::Warning(warning) => Some(PlayEvent::Warning(format!(
                "{} ({:?})",
                warning.error(),
                warning.debug()
            ))),
            MessageView::StateChanged(changed) if from_pipeline => {
                Some(PlayEvent::StateChanged(format!("{:?}", changed.current())))
            }
            MessageView::Buffering(buffering) => {
                Some(PlayEvent::Buffering(buffering.percent().clamp(0, 100) as u8))
            }
            MessageView::DurationChanged(_) => Some(PlayEvent::DurationChanged(
                weak.upgrade()
                    .and_then(|pipeline| pipeline.query_duration::<ClockTime>()),
            )),
            MessageView::AsyncDone(done) if seek_pending.swap(false, Ordering::SeqCst) => {
                Some(PlayEvent::SeekDone(done.running_time()))
            }
            _ => None,
        };
        if let Some(event) = event {
            log.push(MseEvent::Play(event));
        }
        // The handler has seen it; nothing needs to queue up on the bus.
        gst::BusSyncReply::Drop
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_have_stable_command_line_names() {
        assert_eq!(Shape::parse("bare"), Some(Shape::Bare));
        assert_eq!(Shape::parse("playbin"), Some(Shape::PlayBin));
        assert_eq!(Shape::parse("PlayBin"), None);
        assert_eq!(Shape::Bare.label(), "bare");
        assert_eq!(Shape::PlayBin.label(), "playbin");
    }

    #[test]
    fn the_default_config_is_the_servo_shape_in_real_time() {
        let config = RigConfig::default();
        assert_eq!(config.shape, Shape::PlayBin);
        assert!(config.sync && config.video && config.audio);
        assert!(config.prefer_decoder.is_none());
    }
}
