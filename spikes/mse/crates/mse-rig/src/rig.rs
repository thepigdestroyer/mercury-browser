// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The two pipeline shapes under test (item 1).
//!
//! * [`Shape::Bare`]: `msesrc ! decodebin3 ! (videoconvert ! appsink | audioconvert ! appsink)`. Isolates
//!   GstMSE from playbin3.
//! * [`Shape::PlayBin`]: how Servo does it (`third_party/servo-media-gstreamer/player.rs`): a
//!   `gstreamer_play::Play` over `playbin3`, `uri = "mse://..."`, the `source-setup` signal attaches the
//!   `MediaSource` to the `msesrc` playbin3 created, video and audio sinks are `appsink`s, a
//!   `PlaySignalAdapter::new_sync_emit` for messages, and no GLib main loop unless the options ask for one.
//!
//! Nothing here opens a window or an audio device; the clock is the pipeline's own.

use std::time::Duration;

use gstreamer::ClockTime;

use crate::RigError;
use crate::backend::MseBackend;
use crate::events::{ElementReadyState, EventLog, SourceBufferId};
use crate::frames::SharedStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Bare,
    PlayBin,
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
    pub to_update_start: Duration,
    pub to_update_end: Duration,
}

/// What the video sink dropped for being late, from the bus's QoS messages: the data behind
/// `getVideoPlaybackQuality()` (`totalVideoFrames`, `droppedVideoFrames`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QosStats {
    pub processed: u64,
    pub dropped: u64,
}

/// What a shutdown found.
#[derive(Debug, Clone, Default)]
pub struct ShutdownInfo {
    pub took: Duration,
    /// Warnings or errors posted on the bus during teardown.
    pub messages: Vec<String>,
}

/// One pipeline with one `MediaSource` attached.
pub struct Rig {
    pub backend: MseBackend,
    pub log: EventLog,
    pub video: SharedStats,
    pub audio: SharedStats,
    // item 1: the pipeline, the Play, the bus watch, the threads.
}

impl Rig {
    /// Builds the pipeline, attaches the `MediaSource`, and returns once `SourceOpen` is in the log
    /// (the pipeline is in PAUSED or has been asked to go there; say which in the doc comment you write).
    pub fn new(_config: RigConfig) -> Result<Rig, RigError> {
        todo!("item 1")
    }

    pub fn play(&self) -> Result<(), RigError> {
        todo!("item 1")
    }

    pub fn pause(&self) -> Result<(), RigError> {
        todo!("item 1")
    }

    /// A flushing seek to `to` (`HTMLMediaElement.currentTime = to`); returns once it is requested.
    pub fn seek(&self, _to: ClockTime) -> Result<(), RigError> {
        todo!("item 1")
    }

    pub fn position(&self) -> Option<ClockTime> {
        todo!("item 1")
    }

    pub fn element_ready_state(&self) -> ElementReadyState {
        todo!("item 1")
    }

    pub fn element_duration(&self) -> Option<ClockTime> {
        todo!("item 1")
    }

    /// Appends `data` to source buffer `id` and waits for its `UpdateEnd`.
    pub fn append_and_wait(
        &self,
        _id: SourceBufferId,
        _data: &[u8],
        _timeout: Duration,
    ) -> Result<AppendTiming, RigError> {
        todo!("item 1")
    }

    /// Polls `predicate` (every 10 ms) until it is true or `timeout` passes; returns whether it became true.
    pub fn wait_until(&self, _timeout: Duration, _predicate: impl Fn(&Rig) -> bool) -> bool {
        todo!("item 1")
    }

    /// The top-level element (`playbin3` or the bare pipeline), for pad probes and queries.
    pub fn pipeline(&self) -> gstreamer::Element {
        todo!("item 1")
    }

    /// The `msesrc` element, once it exists (in the PlayBin shape playbin3 makes it).
    pub fn msesrc(&self) -> Option<gstreamer::Element> {
        todo!("item 1")
    }

    pub fn qos(&self) -> QosStats {
        todo!("item 1")
    }

    /// The factory names of every element in the pipeline, recursively, such as `msesrc0:msesrc`,
    /// `vtdec_hw0:vtdec_hw`: how a scenario finds out which decoder was chosen.
    pub fn describe_pipeline(&self) -> Vec<String> {
        todo!("item 1")
    }

    /// Sets the pipeline to NULL and drops everything, in the order a page's teardown would.
    pub fn shutdown(self) -> Result<ShutdownInfo, RigError> {
        todo!("item 1")
    }
}
