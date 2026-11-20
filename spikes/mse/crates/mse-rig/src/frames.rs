// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Counting sinks (item 1): what reached the end of the pipeline, without ever making a sound or a
//! window. Both the video sink and the audio sink are a bare `appsink` (the video one restricted to raw
//! video, nothing else: no conversion and no scaling in the pipeline, so the decoder's own format and
//! size reach it). Both record into a [`FrameStats`] behind an `Arc<Mutex<..>>`.
//!
//! What is counted where:
//! * `count`, `first_pts`/`last_pts`, `out_of_order`, `max_gap`, `arrivals`, `caps_changes` and (audio)
//!   `bytes` are taken in the `appsink` callback, that is at the end of the pipeline, after the clock
//!   wait when the sink is synchronised: an arrival time is the time the frame would be shown.
//! * `flushes`, the video `caps_changes` source (the caps event) and the video `bytes` (the size of
//!   each decoded frame) come from a pad probe on the sink's input pad.
//! * `colour` is the mean of the whole picture: about once per second of media time, on the first frame
//!   after every flush and when the timestamps jump backwards, the frame is converted to RGB with
//!   `gst_video_converter` inside the callback and every pixel is averaged.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::ClockTime;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video::VideoFrameExt;

use crate::RigError;
use crate::events::EventLog;

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

/// When one buffer reached the sink: its timestamp and the time on the rig's [`EventLog`] clock
/// (`EventLog::elapsed`), so that frames and events share a timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrival {
    pub pts: Option<ClockTime>,
    pub at: Duration,
}

/// The most arrivals kept per sink (a 10-minute 25 fps stream is 15,000).
pub const MAX_ARRIVALS: usize = 200_000;

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
    /// Flushes (flush-stop events) seen at the sink.
    pub flushes: u64,
    /// Every buffer's arrival (at most [`MAX_ARRIVALS`]).
    pub arrivals: Vec<Arrival>,
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

/// The classification of a mean colour against the fixtures' two halves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dominant {
    Red,
    Blue,
    /// Neither: a black frame, grey, or a mix.
    Other,
}

impl ColourSample {
    /// Red when the red channel clearly beats the blue one, blue when the blue clearly beats the red one
    /// (by more than 60 levels of 255, so codec colour shifts and the black ball do not matter).
    pub fn dominant(&self) -> Dominant {
        let [red, _, blue] = self.rgb;
        if i32::from(red) - i32::from(blue) > 60 {
            Dominant::Red
        } else if i32::from(blue) - i32::from(red) > 60 {
            Dominant::Blue
        } else {
            Dominant::Other
        }
    }
}

/// What the probe and the callback share about one sink.
#[derive(Default)]
struct SinkState {
    /// The caps of the data now flowing (from the caps event for video, from the sample for audio).
    current_caps: Option<gst::Caps>,
    /// The previous buffer's timestamp since the last flush, for `out_of_order` and `max_gap`.
    previous_pts: Option<ClockTime>,
    /// A flush-stop passed: the next frame gets a colour sample.
    flush_pending: bool,
    /// The timestamp of the last colour sample.
    last_colour_pts: Option<ClockTime>,
}

/// A sink that counts. `element` goes into the pipeline (`video-sink`/`audio-sink` of playbin3, or
/// linked after `decodebin3`); `stats` is what it measured.
pub struct CountingSink {
    pub element: gst::Element,
    pub stats: SharedStats,
    appsink: Option<gst_app::AppSink>,
}

impl CountingSink {
    /// A video sink: a bare `appsink` that takes any raw video (no conversion and no scaling in the
    /// pipeline, so the decoder's own output format and size reach it; a frame about once per second
    /// is converted to RGB inside the callback for the colour sample). `sync` is the `appsink`'s
    /// `sync` property: `false` decodes as fast as possible.
    pub fn video(
        name: &str,
        stats: SharedStats,
        sync: bool,
        log: &EventLog,
    ) -> Result<Self, RigError> {
        let appsink = make_appsink(name, sync)?;
        appsink.set_caps(Some(&gst::Caps::builder("video/x-raw").build()));
        let state = Arc::new(Mutex::new(SinkState::default()));
        install_callbacks(&appsink, stats.clone(), state.clone(), log.clone(), true);
        let pad = appsink
            .static_pad("sink")
            .ok_or_else(|| RigError::Pipeline("appsink has no sink pad".to_owned()))?;
        install_probe(&pad, stats.clone(), state);
        Ok(CountingSink {
            element: appsink.clone().upcast(),
            stats,
            appsink: Some(appsink),
        })
    }

    /// An audio sink: a bare `appsink` (no caps restriction, no conversion).
    pub fn audio(
        name: &str,
        stats: SharedStats,
        sync: bool,
        log: &EventLog,
    ) -> Result<Self, RigError> {
        let appsink = make_appsink(name, sync)?;
        let state = Arc::new(Mutex::new(SinkState::default()));
        install_callbacks(&appsink, stats.clone(), state, log.clone(), false);
        Ok(CountingSink {
            element: appsink.clone().upcast(),
            stats,
            appsink: Some(appsink),
        })
    }

    /// A sink that discards everything (`fakesink`), for a kind of stream the rig does not want
    /// (it never makes a sound or a window either). `async=false` so that it cannot hold up preroll.
    pub fn discard(name: &str, sync: bool) -> Result<Self, RigError> {
        let element = gst::ElementFactory::make("fakesink")
            .name(name)
            .property("sync", sync)
            .property("async", false)
            .build()
            .map_err(|error| RigError::Pipeline(format!("fakesink: {error}")))?;
        Ok(CountingSink {
            element,
            stats: FrameStats::shared(),
            appsink: None,
        })
    }

    /// The sink's `stats` property (what the base sink rendered and dropped as late): `(rendered, dropped)`.
    /// `None` for a discarding sink or when the property is not there.
    pub fn rendered_and_dropped(&self) -> Option<(u64, u64)> {
        let appsink = self.appsink.as_ref()?;
        let stats = appsink.property::<gst::Structure>("stats");
        Some((
            stats.get::<u64>("rendered").ok()?,
            stats.get::<u64>("dropped").ok()?,
        ))
    }
}

fn make(factory: &str, name: &str) -> Result<gst::Element, RigError> {
    gst::ElementFactory::make(factory)
        .name(name)
        .build()
        .map_err(|error| RigError::Pipeline(format!("{factory}: {error}")))
}

fn make_appsink(name: &str, sync: bool) -> Result<gst_app::AppSink, RigError> {
    let element = make("appsink", name)?;
    element.set_property("sync", sync);
    element.set_property("emit-signals", false);
    if !sync {
        // Decode as fast as possible and keep every frame: nothing is late, so nothing may be dropped.
        element.set_property("qos", false);
    }
    element
        .downcast::<gst_app::AppSink>()
        .map_err(|_| RigError::Pipeline("appsink is not an AppSink".to_owned()))
}

/// The pad probe at the video bin's input: caps, flushes and the decoded frame size.
fn install_probe(pad: &gst::Pad, stats: SharedStats, state: Arc<Mutex<SinkState>>) {
    pad.add_probe(
        gst::PadProbeType::BUFFER
            | gst::PadProbeType::EVENT_DOWNSTREAM
            | gst::PadProbeType::EVENT_FLUSH,
        move |_pad, info| {
            match &info.data {
                Some(gst::PadProbeData::Buffer(buffer)) => {
                    stats.lock().unwrap().bytes += buffer.size() as u64;
                }
                Some(gst::PadProbeData::Event(event)) => match event.view() {
                    gst::EventView::Caps(caps) => {
                        state.lock().unwrap().current_caps = Some(caps.caps_owned());
                    }
                    gst::EventView::FlushStop(_) => {
                        {
                            let mut state = state.lock().unwrap();
                            state.previous_pts = None;
                            state.flush_pending = true;
                        }
                        stats.lock().unwrap().flushes += 1;
                    }
                    _ => {}
                },
                _ => {}
            }
            gst::PadProbeReturn::Ok
        },
    );
}

fn install_callbacks(
    appsink: &gst_app::AppSink,
    stats: SharedStats,
    state: Arc<Mutex<SinkState>>,
    log: EventLog,
    video: bool,
) {
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            // The preroll buffer is rendered again as a sample once the pipeline plays; counting only samples.
            .new_preroll(|_| Ok(gst::FlowSuccess::Ok))
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                record_sample(&stats, &state, &log, &sample, video);
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
}

fn record_sample(
    stats: &SharedStats,
    state: &Mutex<SinkState>,
    log: &EventLog,
    sample: &gst::Sample,
    video: bool,
) {
    let Some(buffer) = sample.buffer() else {
        return;
    };
    let pts = buffer.pts();
    let at = log.elapsed();
    let mut state = state.lock().unwrap();
    if !video {
        // Audio has no reduction step: its caps are the sample's.
        if let Some(caps) = sample.caps().map(|caps| caps.to_owned())
            && state.current_caps.as_ref() != Some(&caps)
        {
            state.current_caps = Some(caps);
        }
    }

    let take_colour = video
        && match (pts, state.last_colour_pts) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(pts), Some(last)) => {
                state.flush_pending || pts < last || pts >= last + ClockTime::SECOND
            }
        };
    let colour = if take_colour {
        pts.zip(mean_rgb(sample))
            .map(|(pts, rgb)| ColourSample { pts, rgb })
    } else {
        None
    };
    if colour.is_some() || (video && state.flush_pending) {
        state.last_colour_pts = pts.or(state.last_colour_pts);
        state.flush_pending = false;
    }

    let mut stats = stats.lock().unwrap();
    stats.count += 1;
    if !video {
        stats.bytes += buffer.size() as u64;
    }
    if let Some(pts) = pts {
        if stats.first_pts.is_none() {
            stats.first_pts = Some(pts);
        }
        if let Some(previous) = state.previous_pts {
            if pts < previous {
                stats.out_of_order += 1;
            } else {
                let gap = pts - previous;
                if stats.max_gap.is_none_or(|max| gap > max) {
                    stats.max_gap = Some(gap);
                }
            }
        }
        state.previous_pts = Some(pts);
        stats.last_pts = Some(pts);
    }
    if let Some(caps) = &state.current_caps {
        let text = caps.to_string();
        if stats
            .caps_changes
            .last()
            .is_none_or(|last| last.caps != text)
        {
            stats.caps_changes.push(CapsChange {
                at_pts: pts,
                caps: text,
            });
        }
    }
    if let Some(colour) = colour {
        stats.colour.push(colour);
    }
    if stats.arrivals.len() < MAX_ARRIVALS {
        stats.arrivals.push(Arrival { pts, at });
    }
}

/// The mean colour of a video sample: the frame is converted to RGB (all of it; this runs about once
/// per second of media time) and every pixel counts, rows honouring the stride. `None` if the sample
/// cannot be read or converted.
fn mean_rgb(sample: &gst::Sample) -> Option<[u8; 3]> {
    let info = gstreamer_video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let source =
        gstreamer_video::VideoFrameRef::from_buffer_ref_readable(sample.buffer()?, &info).ok()?;
    if info.format() == gstreamer_video::VideoFormat::Rgb {
        let stride = usize::try_from(*source.plane_stride().first()?).ok()?;
        return mean_of_rows(
            source.plane_data(0).ok()?,
            stride,
            source.width() as usize,
            source.height() as usize,
        );
    }
    let target = gstreamer_video::VideoInfo::builder(
        gstreamer_video::VideoFormat::Rgb,
        info.width(),
        info.height(),
    )
    .fps(info.fps())
    .interlace_mode(info.interlace_mode())
    .build()
    .ok()?;
    let converter = gstreamer_video::VideoConverter::new(&info, &target, None).ok()?;
    let mut buffer = gst::Buffer::with_size(target.size()).ok()?;
    let mut out =
        gstreamer_video::VideoFrameRef::from_buffer_ref_writable(buffer.get_mut()?, &target)
            .ok()?;
    converter.frame_ref(&source, &mut out);
    let stride = usize::try_from(*out.plane_stride().first()?).ok()?;
    mean_of_rows(
        out.plane_data(0).ok()?,
        stride,
        out.width() as usize,
        out.height() as usize,
    )
}

/// The mean of `height` rows of `width` RGB pixels laid out `stride` bytes apart. `None` when the data is
/// too short or empty. Pure, so it is unit tested.
pub fn mean_of_rows(data: &[u8], stride: usize, width: usize, height: usize) -> Option<[u8; 3]> {
    if width == 0 || height == 0 || stride < width * 3 {
        return None;
    }
    let mut sums = [0u64; 3];
    for row in 0..height {
        let start = row * stride;
        let line = data.get(start..start + width * 3)?;
        for pixel in line.as_chunks::<3>().0 {
            sums[0] += u64::from(pixel[0]);
            sums[1] += u64::from(pixel[1]);
            sums[2] += u64::from(pixel[2]);
        }
    }
    let pixels = (width * height) as u64;
    Some([
        (sums[0] / pixels) as u8,
        (sums[1] / pixels) as u8,
        (sums[2] / pixels) as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mean_of_a_solid_frame_is_its_colour_and_the_stride_is_honoured() {
        // 2x2 red pixels, stride 8 (2 bytes of padding per row that must not count).
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend_from_slice(&[200, 10, 20, 200, 10, 20]);
            data.extend_from_slice(&[0, 255]);
        }
        assert_eq!(mean_of_rows(&data, 8, 2, 2), Some([200, 10, 20]));
    }

    #[test]
    fn the_mean_averages_all_pixels() {
        let data = [255, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 255];
        assert_eq!(mean_of_rows(&data, 6, 2, 2), Some([127, 0, 127]));
    }

    #[test]
    fn short_or_empty_data_gives_no_mean() {
        assert_eq!(mean_of_rows(&[1, 2, 3], 6, 2, 1), None);
        assert_eq!(mean_of_rows(&[], 0, 0, 0), None);
        assert_eq!(mean_of_rows(&[0; 6], 2, 2, 1), None);
    }

    #[test]
    fn colour_samples_are_classified_against_the_fixtures() {
        let at = ClockTime::ZERO;
        let sample = |rgb| ColourSample { pts: at, rgb }.dominant();
        assert_eq!(sample([220, 10, 20]), Dominant::Red);
        assert_eq!(sample([20, 10, 230]), Dominant::Blue);
        assert_eq!(sample([0, 0, 0]), Dominant::Other);
        assert_eq!(sample([120, 120, 120]), Dominant::Other);
        // The black ball lowers the mean but does not flip it.
        assert_eq!(sample([190, 20, 30]), Dominant::Red);
    }

    #[test]
    fn reset_forgets_everything() {
        let shared = FrameStats::shared();
        {
            let mut stats = shared.lock().unwrap();
            stats.count = 5;
            stats.flushes = 2;
            stats.arrivals.push(Arrival {
                pts: None,
                at: Duration::ZERO,
            });
        }
        FrameStats::reset(&shared);
        let stats = shared.lock().unwrap();
        assert_eq!(
            (stats.count, stats.flushes, stats.arrivals.len()),
            (0, 0, 0)
        );
    }
}
