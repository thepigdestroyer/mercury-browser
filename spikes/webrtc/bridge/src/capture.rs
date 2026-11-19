// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `CaptureStream`: one fake capture source, a `tee`, and one appsink branch per frame sink.
//!
//! This is the shape `GStreamerMediaStream` takes in the product (see the crate docs).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_audio as gst_audio;
use gstreamer_video as gst_video;
use gstreamer_video::VideoFrameExt;
use mercury_frame_sink::{
    AUDIO_SAMPLE_RATE, FrameSinkId, MediaFrameSink, Micros, RawAudioFrame, RawVideoFrame, SinkSet,
};

use crate::BridgeError;
use crate::util::{
    SinkCallGuard, clock_now, in_sink_call, init, make, new_pipeline, run_detached,
    sample_timestamp,
};

/// The test patterns of `videotestsrc` the bridge exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TestPattern {
    /// SMPTE colour bars (the default).
    #[default]
    Smpte,
    /// A moving ball (what the product's "Mercury fake camera" shows).
    Ball,
    /// Random noise; the worst case for an encoder.
    Snow,
    /// Solid black.
    Black,
    /// Solid white.
    White,
    /// A checkerboard with 1-pixel squares.
    Checkers1,
}

impl TestPattern {
    fn nick(self) -> &'static str {
        match self {
            TestPattern::Smpte => "smpte",
            TestPattern::Ball => "ball",
            TestPattern::Snow => "snow",
            TestPattern::Black => "black",
            TestPattern::White => "white",
            TestPattern::Checkers1 => "checkers-1",
        }
    }
}

/// What the fake camera produces. All four fields are the contract; build with
/// `CameraConfig { width: 640, height: 360, fps: 30, pattern: TestPattern::Smpte }` or
/// `CameraConfig::new(640, 360, 30)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraConfig {
    /// Luma width in pixels (1 to 8192; odd sizes are supported).
    pub width: u32,
    /// Luma height in pixels (1 to 8192; odd sizes are supported).
    pub height: u32,
    /// Frames per second (1 to 240).
    pub fps: u32,
    /// The `videotestsrc` pattern.
    pub pattern: TestPattern,
}

impl CameraConfig {
    /// A camera of the given size and rate with the default pattern.
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        Self {
            width,
            height,
            fps,
            pattern: TestPattern::default(),
        }
    }
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self::new(640, 360, 30)
    }
}

/// The pixel format the fake camera source produces before the branches convert to I420. A real
/// camera produces NV12 or YUY2, so `Nv12` measures the conversion cost a real
/// camera adds in every branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceFormat {
    /// Planar 4:2:0, what the sinks want: the branch conversion is a pass-through.
    #[default]
    I420,
    /// Semi-planar 4:2:0: each branch converts to I420.
    Nv12,
}

impl SourceFormat {
    fn name(self) -> &'static str {
        match self {
            SourceFormat::I420 => "I420",
            SourceFormat::Nv12 => "NV12",
        }
    }
}

/// What the fake microphone produces. Both fields are the contract.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicrophoneConfig {
    /// 1 or 2.
    pub channels: u16,
    /// Sine frequency in Hz (above 0, up to 24000).
    pub frequency: f64,
}

impl Default for MicrophoneConfig {
    fn default() -> Self {
        Self {
            channels: 1,
            frequency: 440.0,
        }
    }
}

/// Counters of one sink's branch, for tests and for the numbers in the spike report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SinkStats {
    /// Buffers that reached the branch queue.
    pub offered: u64,
    /// Frames handed to the sink.
    pub delivered: u64,
    /// Buffers the leaky branch queue dropped because the sink was slower than the source.
    pub dropped: u64,
}

/// What a `CaptureStream` produces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StreamKind {
    /// I420 video at the configured size.
    Video {
        /// Luma width.
        width: u32,
        /// Luma height.
        height: u32,
        /// Frames per second.
        fps: u32,
    },
    /// 48 kHz S16 audio in 10 ms blocks.
    Audio {
        /// Channel count.
        channels: u16,
    },
}

#[derive(Default)]
struct BranchShared {
    removed: AtomicBool,
    offered: AtomicU64,
    delivered: AtomicU64,
}

struct Branch {
    elements: Vec<gst::Element>,
    queue: gst::Element,
    tee_pad: gst::Pad,
    shared: Arc<BranchShared>,
}

#[derive(Default)]
struct Control {
    stopped: bool,
    playing: bool,
    branches: HashMap<FrameSinkId, Branch>,
}

struct Inner {
    kind: StreamKind,
    pipeline: gst::Pipeline,
    tee: gst::Element,
    sinks: SinkSet,
    control: Mutex<Control>,
}

/// One capture source (fake camera or fake microphone) with a `tee` and a branch per sink; the
/// analogue of the product's `GStreamerMediaStream` once it has `add_frame_sink`.
///
/// Dropping the stream stops it (see [`CaptureStream::stop`]), so the device is released on every
/// path.
pub struct CaptureStream {
    inner: Arc<Inner>,
}

impl CaptureStream {
    /// A fake camera: `videotestsrc is-live=true` in I420, one sink branch per sink.
    ///
    /// Nothing runs until the first sink is added.
    pub fn fake_camera(cfg: CameraConfig) -> Result<CaptureStream, BridgeError> {
        Self::fake_camera_with_format(cfg, SourceFormat::I420)
    }

    /// Like [`CaptureStream::fake_camera`], with the source format chosen (to measure the
    /// NV12 to I420 conversion that a real camera adds).
    pub fn fake_camera_with_format(
        cfg: CameraConfig,
        format: SourceFormat,
    ) -> Result<CaptureStream, BridgeError> {
        init()?;
        if !(1..=8192).contains(&cfg.width) || !(1..=8192).contains(&cfg.height) {
            return Err(BridgeError::Config(format!(
                "camera size {}x{} is outside 1..=8192",
                cfg.width, cfg.height
            )));
        }
        if !(1..=240).contains(&cfg.fps) {
            return Err(BridgeError::Config(format!(
                "camera frame rate {} is outside 1..=240",
                cfg.fps
            )));
        }
        let source = gst::ElementFactory::make("videotestsrc")
            .property("is-live", true)
            .property_from_str("pattern", cfg.pattern.nick())
            .build()
            .map_err(|_| BridgeError::MissingElement("videotestsrc"))?;
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", format.name())
            .field("width", cfg.width as i32)
            .field("height", cfg.height as i32)
            .field("framerate", gst::Fraction::new(cfg.fps as i32, 1))
            .build();
        let capsfilter = make("capsfilter")?;
        capsfilter.set_property("caps", &caps);
        let convert = make("videoconvert")?;
        Self::assemble(
            StreamKind::Video {
                width: cfg.width,
                height: cfg.height,
                fps: cfg.fps,
            },
            "mercury-rtc-bridge fake camera",
            &[source, capsfilter, convert],
        )
    }

    /// A fake microphone: `audiotestsrc is-live=true`, 48 kHz S16 interleaved, 10 ms blocks.
    ///
    /// The source makes buffers of 1024 samples (21.3 ms, `audiotestsrc`'s default), which
    /// `audiobuffersplit` cuts into 10 ms blocks, the way a real microphone's irregular buffers
    /// would be. A live `audiotestsrc` pushes each buffer when its first sample is due, so the
    /// blocks cut from one buffer arrive together and the later ones carry timestamps up to
    /// about 11 ms ahead of their delivery (a real source stamps capture time, in the past).
    ///
    /// Nothing runs until the first sink is added.
    pub fn fake_microphone(cfg: MicrophoneConfig) -> Result<CaptureStream, BridgeError> {
        Self::fake_microphone_with_buffer(cfg, None)
    }

    /// Like [`CaptureStream::fake_microphone`], with the number of samples per source buffer
    /// chosen. `Some(480)` makes the source produce exactly 10 ms buffers: one block per push, no
    /// bursts, and timestamps that are never ahead of delivery.
    pub fn fake_microphone_with_buffer(
        cfg: MicrophoneConfig,
        samples_per_buffer: Option<u32>,
    ) -> Result<CaptureStream, BridgeError> {
        init()?;
        if !(1..=2).contains(&cfg.channels) {
            return Err(BridgeError::Config(format!(
                "microphone channels {} is not 1 or 2",
                cfg.channels
            )));
        }
        if !(cfg.frequency > 0.0 && cfg.frequency <= 24_000.0) {
            return Err(BridgeError::Config(format!(
                "microphone frequency {} is outside (0, 24000]",
                cfg.frequency
            )));
        }
        let source = gst::ElementFactory::make("audiotestsrc")
            .property("is-live", true)
            .property_from_str("wave", "sine")
            .property("freq", cfg.frequency)
            .property("volume", 0.8f64)
            .build()
            .map_err(|_| BridgeError::MissingElement("audiotestsrc"))?;
        if let Some(samples) = samples_per_buffer {
            if !(1..=48_000).contains(&samples) {
                return Err(BridgeError::Config(format!(
                    "{samples} samples per buffer is outside 1..=48000"
                )));
            }
            source.set_property("samplesperbuffer", samples as i32);
        }
        let convert = make("audioconvert")?;
        let resample = make("audioresample")?;
        let caps = gst::Caps::builder("audio/x-raw")
            .field("format", "S16LE")
            .field("rate", AUDIO_SAMPLE_RATE as i32)
            .field("channels", i32::from(cfg.channels))
            .field("layout", "interleaved")
            .build();
        let capsfilter = make("capsfilter")?;
        capsfilter.set_property("caps", &caps);
        let split = gst::ElementFactory::make("audiobuffersplit")
            .property("output-buffer-duration", gst::Fraction::new(1, 100))
            .build()
            .map_err(|_| BridgeError::MissingElement("audiobuffersplit"))?;
        Self::assemble(
            StreamKind::Audio {
                channels: cfg.channels,
            },
            "mercury-rtc-bridge fake microphone",
            &[source, convert, resample, capsfilter, split],
        )
    }

    /// Builds `chain ! tee` inside a pipeline on the process clock.
    fn assemble(
        kind: StreamKind,
        name: &str,
        chain: &[gst::Element],
    ) -> Result<CaptureStream, BridgeError> {
        let pipeline = new_pipeline(name);
        // Without `allow-not-linked` the tee answers NOT_LINKED when its last branch is removed,
        // and a live source treats that as a fatal error and stops the stream.
        let tee = gst::ElementFactory::make("tee")
            .property("allow-not-linked", true)
            .build()
            .map_err(|_| BridgeError::MissingElement("tee"))?;
        let mut all: Vec<&gst::Element> = chain.iter().collect();
        all.push(&tee);
        pipeline.add_many(all.iter().copied())?;
        gst::Element::link_many(all.iter().copied())?;
        Ok(CaptureStream {
            inner: Arc::new(Inner {
                kind,
                pipeline,
                tee,
                sinks: SinkSet::new(),
                control: Mutex::new(Control::default()),
            }),
        })
    }

    /// Registers a sink and starts delivering frames to it on a streaming thread. The first sink
    /// sets the pipeline PLAYING (this is what opens the device in the product). The sink may be
    /// called before this function returns.
    ///
    /// On a stream that has been stopped the sink gets `ended()` at once and no branch is made.
    pub fn add_frame_sink(
        &self,
        sink: Arc<dyn MediaFrameSink>,
    ) -> Result<FrameSinkId, BridgeError> {
        let inner = &self.inner;
        let mut control = inner.lock();
        if control.stopped {
            drop(control);
            return Ok(inner.sinks.add(sink));
        }
        let shared = Arc::new(BranchShared::default());
        let (elements, queue) = inner.build_branch(&sink, &shared)?;
        let tee_pad = inner.attach_branch(&elements, &queue, &mut control)?;
        let id = inner.sinks.add(sink);
        control.branches.insert(
            id,
            Branch {
                elements,
                queue,
                tee_pad,
                shared,
            },
        );
        Ok(id)
    }

    /// Removes one sink: no further call reaches it once this returns (unless it is called from
    /// inside a sink callback, where the branch is torn down on a helper thread and the sink is
    /// only guaranteed not to be called again). It gets no `ended()`: its owner asked to stop.
    /// Unknown ids are ignored. The other sinks and the pipeline are not disturbed.
    pub fn remove_frame_sink(&self, id: FrameSinkId) {
        let inner = &self.inner;
        let branch = inner.lock().branches.remove(&id);
        inner.sinks.remove(id);
        let Some(branch) = branch else { return };
        branch.shared.removed.store(true, Ordering::Release);
        if in_sink_call() {
            let inner = Arc::clone(inner);
            run_detached("mercury-rtc-bridge-remove", move || {
                inner.teardown_branch(&branch);
            });
        } else {
            inner.teardown_branch(&branch);
        }
    }

    /// Releases the device: the pipeline goes to NULL (every streaming thread has ended when this
    /// returns), then every sink gets `ended()` exactly once. Idempotent. Called from inside a
    /// sink callback it runs on a helper thread (a state change would wait for the calling thread).
    pub fn stop(&self) {
        self.inner.stop();
    }

    /// The pipeline clock now, on the same scale as frame timestamps. Anchor a `ClockMap` on this
    /// and the consumer's clock read at the same moment.
    pub fn producer_now(&self) -> Micros {
        clock_now()
    }

    /// What this stream produces.
    pub fn kind(&self) -> StreamKind {
        self.inner.kind
    }

    /// Sinks currently registered.
    pub fn sink_count(&self) -> usize {
        self.inner.sinks.len()
    }

    /// Counters of one sink's branch; `None` for an unknown or removed sink.
    pub fn sink_stats(&self, id: FrameSinkId) -> Option<SinkStats> {
        let control = self.inner.lock();
        let branch = control.branches.get(&id)?;
        let offered = branch.shared.offered.load(Ordering::Relaxed);
        let delivered = branch.shared.delivered.load(Ordering::Relaxed);
        let queued = u64::from(branch.queue.property::<u32>("current-level-buffers"));
        Some(SinkStats {
            offered,
            delivered,
            dropped: offered.saturating_sub(delivered + queued),
        })
    }

    /// The first error message the pipeline posted on its bus, if any (asynchronous failures such
    /// as a negotiation error do not come back from `add_frame_sink`).
    pub fn take_error(&self) -> Option<String> {
        let bus = self.inner.pipeline.bus()?;
        let message = bus.pop_filtered(&[gst::MessageType::Error])?;
        match message.view() {
            gst::MessageView::Error(error) => {
                Some(format!("{} ({:?})", error.error(), error.debug()))
            }
            _ => None,
        }
    }

    /// The pipeline, for inspection (element names, pad counts). Do not change its state.
    pub fn pipeline(&self) -> &gst::Pipeline {
        &self.inner.pipeline
    }
}

impl Drop for CaptureStream {
    fn drop(&mut self) {
        self.inner.stop();
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Control> {
        self.control.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn stop(self: &Arc<Self>) {
        if in_sink_call() {
            let me = Arc::clone(self);
            run_detached("mercury-rtc-bridge-stop", move || me.stop_now());
        } else {
            self.stop_now();
        }
    }

    fn stop_now(&self) {
        {
            let mut control = self.lock();
            if control.stopped {
                return;
            }
            control.stopped = true;
            for branch in control.branches.values() {
                branch.shared.removed.store(true, Ordering::Release);
            }
            control.branches.clear();
        }
        // NULL joins every streaming thread, so no sink call is running or starts after this.
        if let Err(error) = self.pipeline.set_state(gst::State::Null) {
            eprintln!("mercury-rtc-bridge: stopping the pipeline failed: {error}");
        }
        self.sinks.end();
    }

    /// Makes the elements of one branch (not yet in the pipeline). Returns them in link order and
    /// the queue (for its fill level).
    fn build_branch(
        &self,
        sink: &Arc<dyn MediaFrameSink>,
        shared: &Arc<BranchShared>,
    ) -> Result<(Vec<gst::Element>, gst::Element), BridgeError> {
        let queue = gst::ElementFactory::make("queue")
            .property("max-size-bytes", 0u32)
            .property_from_str("leaky", "downstream")
            .build()
            .map_err(|_| BridgeError::MissingElement("queue"))?;
        let appsink = gst::ElementFactory::make("appsink")
            .property("sync", false)
            // Not async: a sink that must preroll turns every state change of the live pipeline into an
            // asynchronous one, and a sink added to a running pipeline can leave it waiting in PAUSED.
            .property("async", false)
            .property("drop", true)
            .property("emit-signals", false)
            .build()
            .map_err(|_| BridgeError::MissingElement("appsink"))?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| BridgeError::Pipeline("appsink is not an AppSink".into()))?;
        let mut elements = vec![queue.clone()];
        match self.kind {
            StreamKind::Video { .. } => {
                // A leaky queue of two frames: a slow sink drops frames instead of stalling the
                // tee (and with it every other sink) or growing memory.
                queue.set_property("max-size-buffers", 2u32);
                queue.set_property("max-size-time", 0u64);
                appsink.set_property("max-buffers", 2u32);
                let convert = make("videoconvert")?;
                let caps = gst::Caps::builder("video/x-raw")
                    .field("format", "I420")
                    .build();
                let capsfilter = make("capsfilter")?;
                capsfilter.set_property("caps", &caps);
                appsink.set_property("caps", &caps);
                elements.push(convert);
                elements.push(capsfilter);
                appsink.set_callbacks(video_callbacks(Arc::clone(sink), Arc::clone(shared)));
            }
            StreamKind::Audio { .. } => {
                // Half a second of 10 ms blocks.
                queue.set_property("max-size-buffers", 0u32);
                queue.set_property("max-size-time", 500_000_000u64);
                appsink.set_property("max-buffers", 50u32);
                appsink.set_callbacks(audio_callbacks(Arc::clone(sink), Arc::clone(shared)));
            }
        }
        // Counts what reaches the queue so `sink_stats` can tell dropped from delivered.
        if let Some(pad) = queue.static_pad("sink") {
            let counter = Arc::clone(shared);
            pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                counter.offered.fetch_add(1, Ordering::Relaxed);
                gst::PadProbeReturn::Ok
            });
        }
        elements.push(appsink.upcast());
        Ok((elements, queue))
    }

    /// Adds a branch to the pipeline, links it to a fresh tee pad, brings it to the pipeline's
    /// state, and starts the pipeline when it is the first. On failure nothing is left behind.
    fn attach_branch(
        &self,
        elements: &[gst::Element],
        queue: &gst::Element,
        control: &mut Control,
    ) -> Result<gst::Pad, BridgeError> {
        self.pipeline.add_many(elements.iter())?;
        let linked = gst::Element::link_many(elements.iter());
        let tee_pad = match linked {
            Ok(()) => self.tee.request_pad_simple("src_%u"),
            Err(error) => {
                self.discard(elements, None);
                return Err(error.into());
            }
        };
        let Some(tee_pad) = tee_pad else {
            self.discard(elements, None);
            return Err(BridgeError::Pipeline("the tee gave no source pad".into()));
        };
        let Some(queue_sink) = queue.static_pad("sink") else {
            self.discard(elements, Some(&tee_pad));
            return Err(BridgeError::Pipeline("the queue has no sink pad".into()));
        };
        // On a running pipeline the branch is brought up first and linked to the tee afterwards,
        // so the tee never pushes into an inactive pad.
        let started = (|| -> Result<(), BridgeError> {
            if control.playing {
                for element in elements {
                    element.sync_state_with_parent()?;
                }
            }
            tee_pad.link(&queue_sink).map_err(|error| {
                BridgeError::Pipeline(format!("linking the tee to the branch failed: {error:?}"))
            })?;
            if !control.playing {
                self.pipeline.set_state(gst::State::Playing)?;
                // The appsinks are not async (see `build_branch`), so this returns at once; the
                // wait only turns a failed start into an error here instead of a silent stall.
                let (result, _, _) = self.pipeline.state(gst::ClockTime::from_seconds(5));
                result?;
                control.playing = true;
            }
            Ok(())
        })();
        match started {
            Ok(()) => Ok(tee_pad),
            Err(error) => {
                self.discard(elements, Some(&tee_pad));
                Err(error)
            }
        }
    }

    /// Takes a branch out: unlinks and releases its tee pad, sets its elements to NULL, and
    /// removes them from the pipeline.
    fn discard(&self, elements: &[gst::Element], tee_pad: Option<&gst::Pad>) {
        if let Some(pad) = tee_pad {
            if let Some(peer) = pad.peer() {
                let _ = pad.unlink(&peer);
            }
            self.tee.release_request_pad(pad);
        }
        for element in elements {
            let _ = element.set_state(gst::State::Null);
        }
        let _ = self.pipeline.remove_many(elements.iter());
    }

    /// Takes a running branch out of the pipeline. The tee pad is unlinked from an idle probe so
    /// that no buffer is in flight on it, then the branch elements go to NULL (which waits for
    /// the branch's streaming thread, so no sink call is running afterwards).
    fn teardown_branch(&self, branch: &Branch) {
        let (done, wait) = mpsc::channel::<()>();
        let probe = branch
            .tee_pad
            .add_probe(gst::PadProbeType::IDLE, move |pad, _| {
                if let Some(peer) = pad.peer() {
                    let _ = pad.unlink(&peer);
                }
                let _ = done.send(());
                gst::PadProbeReturn::Remove
            });
        if probe.is_some() {
            // The probe runs here when the pad is idle, else on the tee's streaming thread.
            let _ = wait.recv_timeout(Duration::from_secs(2));
        }
        self.discard(&branch.elements, Some(&branch.tee_pad));
    }
}

/// Appsink callbacks for a video branch: maps the I420 sample and calls the sink.
fn video_callbacks(
    sink: Arc<dyn MediaFrameSink>,
    shared: Arc<BranchShared>,
) -> gst_app::AppSinkCallbacks {
    let mut cache: Option<(gst::Caps, gst_video::VideoInfo)> = None;
    gst_app::AppSinkCallbacks::builder()
        .new_sample(move |appsink| {
            let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
            if shared.removed.load(Ordering::Acquire) {
                return Ok(gst::FlowSuccess::Ok);
            }
            let (Some(buffer), Some(caps)) = (sample.buffer(), sample.caps()) else {
                return Ok(gst::FlowSuccess::Ok);
            };
            if cache.as_ref().is_none_or(|(have, _)| have.as_ref() != caps) {
                cache = gst_video::VideoInfo::from_caps(caps)
                    .ok()
                    .map(|info| (caps.to_owned(), info));
            }
            let Some((_, info)) = cache.as_ref() else {
                return Ok(gst::FlowSuccess::Ok);
            };
            if info.format() != gst_video::VideoFormat::I420 {
                return Ok(gst::FlowSuccess::Ok);
            }
            let Ok(frame) = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, info) else {
                return Ok(gst::FlowSuccess::Ok);
            };
            let (Ok(y), Ok(u), Ok(v)) = (
                frame.plane_data(0),
                frame.plane_data(1),
                frame.plane_data(2),
            ) else {
                return Ok(gst::FlowSuccess::Ok);
            };
            let strides = frame.plane_stride();
            // The planes are borrowed from the GStreamer buffer for the call: no copy, whatever
            // the stride (the frame type carries the strides).
            let raw = RawVideoFrame {
                width: info.width(),
                height: info.height(),
                y,
                u,
                v,
                stride_y: strides[0] as usize,
                stride_u: strides[1] as usize,
                stride_v: strides[2] as usize,
                timestamp: sample_timestamp(appsink.upcast_ref(), &sample, buffer),
            };
            let _guard = SinkCallGuard::enter();
            sink.video_frame(raw);
            shared.delivered.fetch_add(1, Ordering::Relaxed);
            Ok(gst::FlowSuccess::Ok)
        })
        .build()
}

/// Appsink callbacks for an audio branch: S16LE bytes to `i16` samples in a reused scratch buffer
/// (one copy of 960 bytes per stereo block; reading them in place would need `unsafe`).
fn audio_callbacks(
    sink: Arc<dyn MediaFrameSink>,
    shared: Arc<BranchShared>,
) -> gst_app::AppSinkCallbacks {
    let mut scratch: Vec<i16> = Vec::new();
    let mut cache: Option<(gst::Caps, gst_audio::AudioInfo)> = None;
    gst_app::AppSinkCallbacks::builder()
        .new_sample(move |appsink| {
            let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
            if shared.removed.load(Ordering::Acquire) {
                return Ok(gst::FlowSuccess::Ok);
            }
            let (Some(buffer), Some(caps)) = (sample.buffer(), sample.caps()) else {
                return Ok(gst::FlowSuccess::Ok);
            };
            if cache.as_ref().is_none_or(|(have, _)| have.as_ref() != caps) {
                cache = gst_audio::AudioInfo::from_caps(caps)
                    .ok()
                    .map(|info| (caps.to_owned(), info));
            }
            let Some((_, info)) = cache.as_ref() else {
                return Ok(gst::FlowSuccess::Ok);
            };
            // The capsfilter in the source chain guarantees this; a block in any other format
            // would not be 10 ms of 48 kHz S16 and must not reach a sink.
            if info.format() != gst_audio::AudioFormat::S16le || info.rate() != AUDIO_SAMPLE_RATE {
                return Ok(gst::FlowSuccess::Ok);
            }
            let Ok(map) = buffer.map_readable() else {
                return Ok(gst::FlowSuccess::Ok);
            };
            scratch.clear();
            scratch.extend(
                map.as_slice()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| i16::from_le_bytes(*pair)),
            );
            let raw = RawAudioFrame {
                samples: &scratch,
                channels: info.channels() as u16,
                sample_rate: AUDIO_SAMPLE_RATE,
                timestamp: sample_timestamp(appsink.upcast_ref(), &sample, buffer),
            };
            let _guard = SinkCallGuard::enter();
            sink.audio_frame(raw);
            shared.delivered.fetch_add(1, Ordering::Relaxed);
            Ok(gst::FlowSuccess::Ok)
        })
        .build()
}
