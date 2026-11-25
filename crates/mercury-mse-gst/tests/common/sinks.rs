// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Counting sinks: what reached the end of the pipeline, without a sound or a window. Both are bare
//! `appsink`s; the video one sits behind `videoconvert ! RGB` when the colour is wanted (the fixtures
//! are red for 4 s, then blue, with a small black ball, so the mean colour of a frame says which half of
//! the clip it came from after any seek or switch).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

/// One buffer that reached a sink.
#[derive(Clone, Debug)]
pub struct Arrival {
    pub pts: Option<u64>,
    pub duration: Option<u64>,
    /// Time since the rig started.
    pub at: Duration,
    /// Index into [`Recorded::caps`] of the caps the buffer came with.
    pub caps: usize,
    /// Mean colour (video with colour only).
    pub rgb: Option<[u8; 3]>,
    pub bytes: usize,
}

#[derive(Default, Debug)]
pub struct Recorded {
    pub arrivals: Vec<Arrival>,
    /// The distinct caps in the order they appeared (a new entry when they change).
    pub caps: Vec<String>,
    pub flushes: u32,
    /// `arrivals.len()` at each flush-stop: the first buffer after flush k is `arrivals[flush_marks[k]]`.
    pub flush_marks: Vec<usize>,
}

pub type Stats = Arc<Mutex<Recorded>>;

/// Runs inside the sink's callback, on GStreamer's streaming thread, after each buffer (T02 calls the feed
/// from it).
pub type Hook = Arc<dyn Fn() + Send + Sync>;

pub struct Sink {
    pub element: gst::Element,
    pub stats: Stats,
    pub appsink: gst_app::AppSink,
}

impl Sink {
    /// The base sink's own counters: `(rendered, dropped)` (what `getVideoPlaybackQuality()` reads).
    pub fn rendered_and_dropped(&self) -> Option<(u64, u64)> {
        let stats = self.appsink.property::<gst::Structure>("stats");
        Some((
            stats.get::<u64>("rendered").ok()?,
            stats.get::<u64>("dropped").ok()?,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    Red,
    Blue,
    Other,
}

/// Red when the red channel beats the blue one by more than 60 levels, blue the other way round.
pub fn classify(rgb: [u8; 3]) -> Colour {
    let (red, blue) = (i32::from(rgb[0]), i32::from(rgb[2]));
    if red - blue > 60 {
        Colour::Red
    } else if blue - red > 60 {
        Colour::Blue
    } else {
        Colour::Other
    }
}

impl Recorded {
    pub fn count(&self) -> usize {
        self.arrivals.len()
    }

    pub fn last_pts(&self) -> Option<u64> {
        self.arrivals.last().and_then(|a| a.pts)
    }

    /// Sum of the buffers' durations in seconds (audio).
    pub fn total_seconds(&self) -> f64 {
        self.arrivals.iter().filter_map(|a| a.duration).sum::<u64>() as f64 / 1e9
    }

    /// The colours of the frames that arrived after `from_index`.
    pub fn colours_since(&self, from_index: usize) -> Vec<(u64, Colour)> {
        self.arrivals[from_index.min(self.arrivals.len())..]
            .iter()
            .filter_map(|a| Some((a.pts?, classify(a.rgb?))))
            .collect()
    }
}

fn mean_rgb(buffer: &gst::BufferRef, caps: &gst::CapsRef) -> Option<[u8; 3]> {
    let s = caps.structure(0)?;
    let width = s.get::<i32>("width").ok()? as usize;
    let height = s.get::<i32>("height").ok()? as usize;
    let map = buffer.map_readable().ok()?;
    let data = map.as_slice();
    // Packed RGB, rows padded to the buffer's own stride.
    let stride = data.len() / height.max(1);
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for y in (0..height).step_by(2) {
        let row = &data[y * stride..y * stride + width * 3];
        for px in row.as_chunks::<3>().0.iter().step_by(2) {
            r += u64::from(px[0]);
            g += u64::from(px[1]);
            b += u64::from(px[2]);
            n += 1;
        }
    }
    (n > 0).then(|| [(r / n) as u8, (g / n) as u8, (b / n) as u8])
}

fn record(stats: &Stats, t0: Instant, sample: &gst::Sample, colour: bool) {
    let Some(buffer) = sample.buffer() else {
        return;
    };
    let caps = sample.caps();
    let caps_text = caps.map(|c| c.to_string()).unwrap_or_default();
    let rgb = if colour {
        caps.and_then(|caps| mean_rgb(buffer, caps))
    } else {
        None
    };
    let mut stats = stats.lock().unwrap();
    if stats.caps.last() != Some(&caps_text) {
        stats.caps.push(caps_text);
    }
    let caps_index = stats.caps.len().saturating_sub(1);
    stats.arrivals.push(Arrival {
        pts: buffer.pts().map(|t| t.nseconds()),
        duration: buffer.duration().map(|t| t.nseconds()),
        at: t0.elapsed(),
        caps: caps_index,
        rgb,
        bytes: buffer.size(),
    });
}

fn appsink(
    name: &str,
    sync: bool,
    caps: &str,
    stats: &Stats,
    t0: Instant,
    colour: bool,
    hook: Option<Hook>,
) -> gst_app::AppSink {
    let sink = gst_app::AppSink::builder()
        .name(name)
        .sync(sync)
        .caps(&caps.parse::<gst::Caps>().expect("caps"))
        .build();
    if !sync {
        // Decode as fast as possible and keep every frame: nothing is late, nothing may be dropped.
        sink.set_property("qos", false);
    }
    let stats_for_samples = Arc::clone(stats);
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                record(&stats_for_samples, t0, &sample, colour);
                if let Some(hook) = &hook {
                    hook();
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    if let Some(pad) = sink.static_pad("sink") {
        let stats = Arc::clone(stats);
        pad.add_probe(gst::PadProbeType::EVENT_FLUSH, move |_, info| {
            if let Some(gst::PadProbeData::Event(event)) = &info.data
                && event.type_() == gst::EventType::FlushStop
            {
                let mut stats = stats.lock().unwrap();
                stats.flushes += 1;
                let mark = stats.arrivals.len();
                stats.flush_marks.push(mark);
            }
            gst::PadProbeReturn::Ok
        });
    }
    sink
}

/// A video sink: raw video only. With `colour`, the frames are converted to RGB first and each arrival
/// carries its mean colour.
pub fn video_sink(name: &str, t0: Instant, sync: bool, colour: bool, hook: Option<Hook>) -> Sink {
    let stats: Stats = Arc::default();
    if !colour {
        let sink = appsink(name, sync, "video/x-raw", &stats, t0, false, hook);
        return Sink {
            element: sink.clone().upcast(),
            stats,
            appsink: sink,
        };
    }
    let sink = appsink(
        &format!("{name}-sink"),
        sync,
        "video/x-raw,format=RGB",
        &stats,
        t0,
        true,
        hook,
    );
    let convert = gst::ElementFactory::make("videoconvert")
        .name(format!("{name}-convert"))
        .build()
        .expect("videoconvert");
    let bin = gst::Bin::builder().name(name).build();
    bin.add_many([&convert, sink.upcast_ref()]).unwrap();
    convert.link(&sink).unwrap();
    let pad = gst::GhostPad::with_target(&convert.static_pad("sink").unwrap()).unwrap();
    bin.add_pad(&pad).unwrap();
    Sink {
        element: bin.upcast(),
        stats,
        appsink: sink,
    }
}

/// An audio sink: raw audio only, nothing is played.
pub fn audio_sink(name: &str, t0: Instant, sync: bool, hook: Option<Hook>) -> Sink {
    let stats: Stats = Arc::default();
    let sink = appsink(name, sync, "audio/x-raw", &stats, t0, false, hook);
    Sink {
        element: sink.clone().upcast(),
        stats,
        appsink: sink,
    }
}
