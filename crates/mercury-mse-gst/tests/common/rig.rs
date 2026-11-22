// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The two pipeline shapes under test (brief section 6, item 4):
//!
//! * [`Shape::Bare`]: `MseSrc ! decodebin3 ! appsinks`. Isolates the source element from `playbin3`.
//! * [`Shape::PlayBin`]: how Servo does it: `playbin3`, `uri = mercury-mse://<n>`, the `source-setup`
//!   signal hands the element to `attach`, the sinks are `appsink`s.
//!
//! No window, no sound, no GLib main loop: the bus is polled from the test thread.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use mercury_mse::FrameFeed;

use super::sinks::{Hook, Sink, Stats, audio_sink, video_sink};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Bare,
    PlayBin,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub shape: Shape,
    /// `false`: the sinks do not wait for the clock (as fast as possible).
    pub sync: bool,
    pub video: bool,
    pub audio: bool,
    /// Convert video to RGB and record each frame's mean colour.
    pub colour: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            shape: Shape::PlayBin,
            sync: false,
            video: true,
            audio: true,
            colour: true,
        }
    }
}

/// What the bus has said so far.
#[derive(Default, Debug)]
pub struct BusLog {
    pub eos: u32,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub async_done: u32,
    pub playing: bool,
    pub duration_changed: u32,
    pub buffering: Vec<i32>,
    pub collections: u32,
    /// Every message of interest as text, with the time since the rig started.
    pub log: Vec<(Duration, String)>,
}

pub struct Rig {
    pub pipeline: gst::Element,
    pub video: Option<Sink>,
    pub audio: Option<Sink>,
    pub t0: Instant,
    /// The `MseSrc`, once `source-setup` (or the bare shape) has made it.
    pub src: Arc<Mutex<Option<gst::Element>>>,
    pub feed: Arc<dyn FrameFeed>,
    pub options: Options,
    pub bus_log: Mutex<BusLog>,
    bus: gst::Bus,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl Rig {
    /// Builds the pipeline (in NULL) and attaches `feed`.
    pub fn new(feed: Arc<dyn FrameFeed>, options: Options) -> Rig {
        Rig::with_hook(feed, options, None)
    }

    /// As `new`, with a closure the sinks call from their streaming threads after each buffer.
    pub fn with_hook(feed: Arc<dyn FrameFeed>, options: Options, hook: Option<Hook>) -> Rig {
        super::env::init();
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let t0 = Instant::now();
        let video = options.video.then(|| {
            video_sink(
                &format!("vsink{id}"),
                t0,
                options.sync,
                options.colour,
                hook.clone(),
            )
        });
        let audio = options
            .audio
            .then(|| audio_sink(&format!("asink{id}"), t0, options.sync, hook.clone()));
        let src: Arc<Mutex<Option<gst::Element>>> = Arc::default();
        let pipeline = match options.shape {
            Shape::PlayBin => build_playbin(id, &feed, &src, video.as_ref(), audio.as_ref()),
            Shape::Bare => build_bare(id, &feed, &src, video.as_ref(), audio.as_ref()),
        };
        let bus = pipeline.bus().expect("a pipeline has a bus");
        Rig {
            pipeline,
            video,
            audio,
            t0,
            src,
            feed,
            options,
            bus_log: Mutex::default(),
            bus,
        }
    }

    /// The control of T04 (decision D14): the same `playbin3`, decoders and sinks, with the file `path`
    /// as the source instead of an `MseSrc`: no MSE in the pipeline at all.
    pub fn control(path: &std::path::Path, options: Options) -> Rig {
        super::env::init();
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let t0 = Instant::now();
        let video = options.video.then(|| {
            video_sink(
                &format!("vsink{id}"),
                t0,
                options.sync,
                options.colour,
                None,
            )
        });
        let audio = options
            .audio
            .then(|| audio_sink(&format!("asink{id}"), t0, options.sync, None));
        let uri = glib::filename_to_uri(path, None).expect("a file URI");
        let pipeline = gst::ElementFactory::make("playbin3")
            .name(format!("playbin{id}"))
            .property("uri", uri.as_str())
            .build()
            .expect("playbin3");
        configure_playbin(&pipeline, video.as_ref(), audio.as_ref());
        let bus = pipeline.bus().expect("a pipeline has a bus");
        Rig {
            pipeline,
            video,
            audio,
            t0,
            src: Arc::default(),
            feed: super::fakefeed::FakeFeed::new(),
            options,
            bus_log: Mutex::default(),
            bus,
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.t0.elapsed()
    }

    pub fn play(&self) {
        self.pipeline
            .set_state(gst::State::Playing)
            .expect("set_state(Playing)");
    }

    pub fn pause(&self) {
        self.pipeline
            .set_state(gst::State::Paused)
            .expect("set_state(Paused)");
    }

    /// Takes the pipeline to NULL. Nothing waits: this is the teardown the browser does.
    pub fn stop(&self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }

    /// Reads every message that is waiting.
    pub fn pump_bus(&self) {
        while let Some(message) = self.bus.pop() {
            let at = self.t0.elapsed();
            let mut log = self.bus_log.lock().unwrap();
            let source = message
                .src()
                .map(|s| s.name().to_string())
                .unwrap_or_default();
            match message.view() {
                gst::MessageView::Eos(_) => {
                    log.eos += 1;
                    log.log.push((at, "eos".into()));
                }
                gst::MessageView::Error(error) => {
                    let text = format!(
                        "{source}: {} ({})",
                        error.error(),
                        error.debug().unwrap_or_default()
                    );
                    log.log.push((at, format!("error {text}")));
                    log.errors.push(text);
                }
                gst::MessageView::Warning(warning) => {
                    let text = format!("{source}: {}", warning.error());
                    log.log.push((at, format!("warning {text}")));
                    log.warnings.push(text);
                }
                gst::MessageView::AsyncDone(_) => {
                    log.async_done += 1;
                    log.log.push((at, "async-done".into()));
                }
                gst::MessageView::DurationChanged(_) => {
                    log.duration_changed += 1;
                    log.log.push((at, format!("duration-changed ({source})")));
                }
                gst::MessageView::Buffering(buffering) => {
                    log.buffering.push(buffering.percent());
                    log.log
                        .push((at, format!("buffering {}%", buffering.percent())));
                }
                gst::MessageView::StreamCollection(_) => {
                    log.collections += 1;
                    log.log.push((at, format!("stream-collection ({source})")));
                }
                gst::MessageView::StateChanged(change)
                    if message.src() == Some(self.pipeline.upcast_ref::<gst::Object>()) =>
                {
                    if change.current() == gst::State::Playing {
                        log.playing = true;
                    }
                    log.log.push((
                        at,
                        format!(
                            "pipeline state {:?} -> {:?}",
                            change.old(),
                            change.current()
                        ),
                    ));
                }
                _ => {}
            }
        }
    }

    /// A message summary for a failure.
    pub fn report(&self) -> String {
        self.pump_bus();
        let log = self.bus_log.lock().unwrap();
        let mut out = String::new();
        for (at, line) in &log.log {
            out += &format!("  {:>7.3}s bus: {line}\n", at.as_secs_f64());
        }
        for (name, sink) in [("video", &self.video), ("audio", &self.audio)] {
            if let Some(sink) = sink {
                let stats = sink.stats.lock().unwrap();
                out += &format!(
                    "  {name}: {} arrivals, last pts {:?}, caps {:?}\n",
                    stats.count(),
                    stats.last_pts(),
                    stats.caps
                );
            }
        }
        out += &format!("  element tree:\n{}", self.dump_tree());
        out
    }

    /// The element tree with states and pads, for failure messages.
    pub fn dump_tree(&self) -> String {
        let mut out = String::new();
        if let Some(bin) = self.pipeline.downcast_ref::<gst::Bin>() {
            dump_bin(bin, 2, &mut out);
        }
        out
    }

    /// Polls the bus and `cond` until `cond` holds; panics on an error message or on `timeout`.
    pub fn wait_until(&self, timeout: Duration, what: &str, mut cond: impl FnMut(&Rig) -> bool) {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump_bus();
            {
                let log = self.bus_log.lock().unwrap();
                assert!(
                    log.errors.is_empty(),
                    "pipeline error while waiting for {what}: {:?}\n{}",
                    log.errors,
                    drop_lock_report(&log)
                );
            }
            if cond(self) {
                return;
            }
            if Instant::now() >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for {what}\n{}",
                    self.report()
                );
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    pub fn wait_eos(&self, timeout: Duration) {
        self.wait_until(timeout, "end of stream", |rig| {
            rig.bus_log.lock().unwrap().eos > 0
        });
    }

    pub fn video_stats(&self) -> Stats {
        Arc::clone(&self.video.as_ref().expect("a video sink").stats)
    }

    pub fn audio_stats(&self) -> Stats {
        Arc::clone(&self.audio.as_ref().expect("an audio sink").stats)
    }

    /// A flushing seek to `position` (nanoseconds). Returns whether the pipeline accepted it.
    pub fn seek(&self, position: u64, accurate: bool) -> bool {
        let flags = gst::SeekFlags::FLUSH
            | if accurate {
                gst::SeekFlags::ACCURATE
            } else {
                gst::SeekFlags::KEY_UNIT
            };
        self.pipeline
            .seek_simple(flags, gst::ClockTime::from_nseconds(position))
            .is_ok()
    }

    pub fn position(&self) -> Option<gst::ClockTime> {
        self.pipeline.query_position::<gst::ClockTime>()
    }

    pub fn duration(&self) -> Option<gst::ClockTime> {
        self.pipeline.query_duration::<gst::ClockTime>()
    }
}

fn drop_lock_report(log: &BusLog) -> String {
    log.log
        .iter()
        .map(|(at, line)| format!("  {:>7.3}s bus: {line}\n", at.as_secs_f64()))
        .collect()
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn dump_bin(bin: &gst::Bin, depth: usize, out: &mut String) {
    let pad = " ".repeat(depth * 2);
    for element in bin.iterate_elements().into_iter().flatten() {
        let (_, current, pending) = element.state(gst::ClockTime::ZERO);
        let factory = element
            .factory()
            .map(|f| f.name().to_string())
            .unwrap_or_default();
        out.push_str(&format!(
            "{pad}{} [{factory}] {current:?}/{pending:?}\n",
            element.name()
        ));
        for pad_ in element.pads() {
            let peer = pad_
                .peer()
                .and_then(|p| {
                    p.parent_element()
                        .map(|e| format!("{}:{}", e.name(), p.name()))
                })
                .unwrap_or_else(|| "-".into());
            out.push_str(&format!("{pad}    pad {} -> {peer}\n", pad_.name()));
        }
        if let Some(inner) = element.downcast_ref::<gst::Bin>() {
            dump_bin(inner, depth + 1, out);
        }
    }
}

/// Audio and video only: no text, no visualisation, no download or network buffering; our sinks.
fn configure_playbin(playbin: &gst::Element, video: Option<&Sink>, audio: Option<&Sink>) {
    playbin.set_property_from_str("flags", "video+audio");
    match video {
        Some(sink) => playbin.set_property("video-sink", &sink.element),
        None => playbin.set_property_from_str("flags", "audio"),
    }
    if let Some(sink) = audio {
        playbin.set_property("audio-sink", &sink.element);
    }
}

fn build_playbin(
    id: u32,
    feed: &Arc<dyn FrameFeed>,
    src: &Arc<Mutex<Option<gst::Element>>>,
    video: Option<&Sink>,
    audio: Option<&Sink>,
) -> gst::Element {
    let playbin = gst::ElementFactory::make("playbin3")
        .name(format!("playbin{id}"))
        .property("uri", format!("{}://{id}", mercury_mse_gst::MSE_URI_SCHEME))
        .build()
        .expect("playbin3");
    configure_playbin(&playbin, video, audio);
    let feed = Arc::clone(feed);
    let slot = Arc::clone(src);
    playbin.connect("source-setup", false, move |args| {
        let source = args[1].get::<gst::Element>().expect("the source element");
        mercury_mse_gst::attach(&source, Arc::clone(&feed)).expect("attach");
        *slot.lock().unwrap() = Some(source);
        None
    });
    playbin
}

fn build_bare(
    id: u32,
    feed: &Arc<dyn FrameFeed>,
    src: &Arc<Mutex<Option<gst::Element>>>,
    video: Option<&Sink>,
    audio: Option<&Sink>,
) -> gst::Element {
    let pipeline = gst::Pipeline::with_name(&format!("bare{id}"));
    let source = gst::ElementFactory::make(mercury_mse_gst::ELEMENT_NAME)
        .name(format!("mse{id}"))
        .build()
        .expect("mercurymsesrc");
    mercury_mse_gst::attach(&source, Arc::clone(feed)).expect("attach");
    let decoder = gst::ElementFactory::make("decodebin3")
        .name(format!("dec{id}"))
        .build()
        .expect("decodebin3");
    pipeline.add_many([&source, &decoder]).unwrap();
    let mut video_pad = None;
    let mut audio_pad = None;
    if let Some(sink) = video {
        pipeline.add(&sink.element).unwrap();
        video_pad = sink.element.static_pad("sink");
    }
    if let Some(sink) = audio {
        pipeline.add(&sink.element).unwrap();
        audio_pad = sink.element.static_pad("sink");
    }
    *src.lock().unwrap() = Some(source.clone());

    let decoder_for_src = decoder.clone();
    source.connect_pad_added(move |_, pad| {
        let sink = decoder_for_src
            .request_pad_simple("sink_%u")
            .expect("a decodebin3 sink pad");
        pad.link(&sink).expect("link the source to decodebin3");
    });
    decoder.connect_pad_added(move |_, pad| {
        let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
        let name = caps
            .structure(0)
            .map(|s| s.name().to_string())
            .unwrap_or_default();
        let target = if name.starts_with("video/") {
            video_pad.as_ref()
        } else if name.starts_with("audio/") {
            audio_pad.as_ref()
        } else {
            None
        };
        if let Some(target) = target {
            pad.link(target).expect("link decodebin3 to the sink");
        }
    });
    pipeline.upcast()
}

/// Runs `body` on a thread of its own and fails the test when it has not finished within `seconds`: a
/// hang is a failure, never a wait (the thread is left behind). A panic in `body` is the test's panic.
pub fn watchdog<R: Send + 'static>(
    name: &str,
    seconds: u64,
    body: impl FnOnce() -> R + Send + 'static,
) -> R {
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new()
        .name(format!("test-{name}"))
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
            let _ = sender.send(result);
        })
        .expect("a test thread");
    match receiver.recv_timeout(Duration::from_secs(seconds)) {
        Ok(Ok(value)) => {
            let _ = handle.join();
            value
        }
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(_) => panic!("WATCHDOG: {name} did not finish within {seconds} s (hang)"),
    }
}
