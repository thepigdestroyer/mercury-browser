// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Counting sinks (item 1): what reached the end of the pipeline, without ever making a sound or a
//! window. A video sink is an `appsink` (RGB, small frames); an audio sink is an `appsink` or a
//! `fakesink`. Both record into a [`FrameStats`] behind an `Arc<Mutex<..>>`.

use std::sync::{Arc, Mutex};

use gstreamer::ClockTime;

/// One change of the negotiated caps seen at a sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsChange {
    /// Timestamp of the first buffer after the change.
    pub at_pts: Option<ClockTime>,
    /// The caps as a string, for example `video/x-raw, format=(string)RGB, width=(int)640, height=(int)360`.
    pub caps: String,
}

/// A sample of the picture: the mean colour of a whole decoded video frame. The fixtures are a solid red
/// background for the first half and a solid blue one for the second, with a small black ball moving over
/// it, so the mean tells which half of the clip a frame came from, whatever the codec did to the colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColourSample {
    pub pts: ClockTime,
    pub rgb: [u8; 3],
}

#[derive(Debug, Clone, Default)]
pub struct FrameStats {
    /// Buffers that reached the sink.
    pub count: u64,
    pub bytes: u64,
    pub first_pts: Option<ClockTime>,
    pub last_pts: Option<ClockTime>,
    /// Buffers whose timestamp went backwards without a flush in between.
    pub out_of_order: u64,
    /// The largest gap between two consecutive timestamps.
    pub max_gap: Option<ClockTime>,
    pub caps_changes: Vec<CapsChange>,
    /// Video only: roughly one sample per second of media time, and one after every flush.
    pub colour: Vec<ColourSample>,
    /// Flushes (flush-start/flush-stop pairs) seen at the sink.
    pub flushes: u64,
}

/// A handle every scenario can read at any time; the sink thread writes it.
pub type SharedStats = Arc<Mutex<FrameStats>>;

impl FrameStats {
    pub fn shared() -> SharedStats {
        Arc::new(Mutex::new(FrameStats::default()))
    }

    /// Forgets everything (a scenario does this after the first `play` to measure from a known point).
    pub fn reset(shared: &SharedStats) {
        *shared.lock().unwrap() = FrameStats::default();
    }
}
