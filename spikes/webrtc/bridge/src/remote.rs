// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `RemoteSource`: remote frames (from libwebrtc) pushed into an `appsrc`, the reverse direction of
//! `CaptureStream`.
//!
//! [`RemoteFeeder`] is the part the product keeps: an `appsrc` plus the code that turns a
//! [`RawVideoFrame`] or [`RawAudioFrame`] into a timestamped GStreamer buffer. [`RemoteSource`]
//! wraps a feeder in a small pipeline that ends in an `appsink`, which is what the spike tests
//! read; in the product the feeder's `appsrc` goes into `GStreamerMediaStream::create_video_from`
//! or `create_audio_from` instead (see the crate docs).

use std::sync::Mutex;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use mercury_frame_sink::{
    AUDIO_BLOCK_MS, AUDIO_SAMPLE_RATE, Micros, RawAudioFrame, RawVideoFrame, chroma_dim,
};

use crate::BridgeError;
use crate::util::{clock_now, init, make, new_pipeline};

#[derive(Clone, Copy, PartialEq, Eq)]
enum FeederKind {
    Video,
    Audio { channels: u16 },
}

/// An `appsrc` that remote frames are pushed into.
///
/// Timestamps: `frame.timestamp` is a time on the pipeline clock (the scale of
/// [`RemoteFeeder::now`] and of the timestamps `CaptureStream` delivers), so a consumer maps the
/// remote capture time to it with a `ClockMap` first. The buffer PTS is that time minus the
/// base time of the pipeline the appsrc is in, i.e. the running time there, so a sink that syncs to
/// the clock presents the frame at the right moment. (Before the pipeline is PLAYING the base time
/// is unknown and the PTS is the clock time itself.)
pub struct RemoteFeeder {
    kind: FeederKind,
    appsrc: gst_app::AppSrc,
    size: Mutex<(u32, u32)>,
}

impl RemoteFeeder {
    /// A video feeder. `width` and `height` set the initial caps; a frame of another size changes
    /// the caps (remote video changes resolution as the sender adapts).
    pub fn video(width: u32, height: u32) -> Result<RemoteFeeder, BridgeError> {
        init()?;
        check_size(width, height)?;
        let appsrc = make_appsrc(
            &video_caps(width, height),
            4 * RawVideoFrame::packed_len(width, height),
        )?;
        Ok(RemoteFeeder {
            kind: FeederKind::Video,
            appsrc,
            size: Mutex::new((width, height)),
        })
    }

    /// An audio feeder for interleaved S16, 48 kHz, 10 ms blocks of `channels` channels.
    pub fn audio(channels: u16) -> Result<RemoteFeeder, BridgeError> {
        init()?;
        if !(1..=2).contains(&channels) {
            return Err(BridgeError::Config(format!(
                "audio channels {channels} is not 1 or 2"
            )));
        }
        let block_bytes = 2 * mercury_frame_sink::AUDIO_SAMPLES_PER_BLOCK * usize::from(channels);
        let appsrc = make_appsrc(&audio_caps(channels), 100 * block_bytes)?;
        Ok(RemoteFeeder {
            kind: FeederKind::Audio { channels },
            appsrc,
            size: Mutex::new((0, 0)),
        })
    }

    /// The element to hand to `GStreamerMediaStream::create_video_from` or `create_audio_from`
    /// (`appsrc.upcast::<gstreamer::Element>()`), or to put into any pipeline.
    pub fn appsrc(&self) -> &gst_app::AppSrc {
        &self.appsrc
    }

    /// The pipeline clock now, on the scale [`RemoteFeeder::push_video`] timestamps use.
    pub fn now(&self) -> Micros {
        clock_now()
    }

    /// Pushes one I420 frame (copied into a tightly packed GStreamer buffer: one copy per plane
    /// row, or per plane when the strides match). Fails when the feeder is audio, when the frame
    /// is invalid, or when the pipeline behind the appsrc is not running.
    pub fn push_video(&self, frame: RawVideoFrame<'_>) -> Result<(), BridgeError> {
        if self.kind != FeederKind::Video {
            return Err(BridgeError::Mismatch("this source takes audio".into()));
        }
        frame.validate()?;
        check_size(frame.width, frame.height)?;
        {
            let mut size = self.size.lock().unwrap_or_else(|e| e.into_inner());
            if *size != (frame.width, frame.height) {
                self.appsrc
                    .set_caps(Some(&video_caps(frame.width, frame.height)));
                *size = (frame.width, frame.height);
            }
        }
        let mut buffer = pack_i420(&frame)?;
        buffer
            .get_mut()
            .ok_or_else(|| BridgeError::Pipeline("the new buffer is shared".into()))?
            .set_pts(self.pts(frame.timestamp));
        self.push(buffer)
    }

    /// Pushes one 10 ms S16 block.
    pub fn push_audio(&self, frame: RawAudioFrame<'_>) -> Result<(), BridgeError> {
        let FeederKind::Audio { channels } = self.kind else {
            return Err(BridgeError::Mismatch("this source takes video".into()));
        };
        frame.validate()?;
        if frame.channels != channels {
            return Err(BridgeError::Mismatch(format!(
                "the source takes {channels} channels, the block has {}",
                frame.channels
            )));
        }
        let mut bytes = Vec::with_capacity(frame.samples.len() * 2);
        for sample in frame.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let mut buffer = gst::Buffer::from_mut_slice(bytes);
        {
            let buffer = buffer
                .get_mut()
                .ok_or_else(|| BridgeError::Pipeline("the new buffer is shared".into()))?;
            buffer.set_pts(self.pts(frame.timestamp));
            buffer.set_duration(gst::ClockTime::from_mseconds(u64::from(AUDIO_BLOCK_MS)));
        }
        self.push(buffer)
    }

    /// Ends the stream (the remote track ended).
    pub fn end_of_stream(&self) {
        let _ = self.appsrc.end_of_stream();
    }

    /// The buffer PTS for a frame time on the pipeline clock: the running time of the pipeline the
    /// appsrc is in (clock time minus the element's base time; never negative).
    fn pts(&self, timestamp: Micros) -> gst::ClockTime {
        let nanos = u64::try_from(timestamp.0).unwrap_or(0).saturating_mul(1000);
        let base = self.appsrc.base_time().unwrap_or(gst::ClockTime::ZERO);
        gst::ClockTime::from_nseconds(nanos.saturating_sub(base.nseconds()))
    }

    fn push(&self, buffer: gst::Buffer) -> Result<(), BridgeError> {
        self.appsrc
            .push_buffer(buffer)
            .map(|_| ())
            .map_err(|error| BridgeError::Push(error.to_string()))
    }
}

/// A feeder in a pipeline of its own that ends in an `appsink`:
/// video `appsrc ! videoconvert ! queue ! appsink`, audio
/// `appsrc ! queue ! audioconvert ! audioresample ! queue ! appsink`. These are the element
/// chains `GStreamerMediaStream::create_video_from` and `create_audio_from` build, so a frame that
/// comes out unchanged here comes out unchanged from the product's stream.
pub struct RemoteSource {
    feeder: RemoteFeeder,
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
}

impl RemoteSource {
    /// A video source, PLAYING and waiting for frames. `width` and `height` are the initial caps.
    pub fn video(width: u32, height: u32) -> Result<RemoteSource, BridgeError> {
        let feeder = RemoteFeeder::video(width, height)?;
        Self::assemble(
            feeder,
            "mercury-rtc-bridge remote video",
            &["videoconvert", "queue"],
        )
    }

    /// An audio source for `channels` (1 or 2), PLAYING and waiting for blocks.
    pub fn audio(channels: u16) -> Result<RemoteSource, BridgeError> {
        let feeder = RemoteFeeder::audio(channels)?;
        Self::assemble(
            feeder,
            "mercury-rtc-bridge remote audio",
            &["queue", "audioconvert", "audioresample", "queue"],
        )
    }

    fn assemble(
        feeder: RemoteFeeder,
        name: &str,
        chain: &[&'static str],
    ) -> Result<RemoteSource, BridgeError> {
        let pipeline = new_pipeline(name);
        let appsink = gst::ElementFactory::make("appsink")
            .property("sync", false)
            .property("drop", true)
            .property("max-buffers", 64u32)
            .property("emit-signals", false)
            .build()
            .map_err(|_| BridgeError::MissingElement("appsink"))?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| BridgeError::Pipeline("appsink is not an AppSink".into()))?;
        let mut elements: Vec<gst::Element> = vec![feeder.appsrc.clone().upcast()];
        for factory in chain {
            elements.push(make(factory)?);
        }
        elements.push(appsink.clone().upcast());
        pipeline.add_many(elements.iter())?;
        gst::Element::link_many(elements.iter())?;
        pipeline.set_state(gst::State::Playing)?;
        Ok(RemoteSource {
            feeder,
            pipeline,
            appsink,
        })
    }

    /// See [`RemoteFeeder::push_video`].
    pub fn push_video(&self, frame: RawVideoFrame<'_>) -> Result<(), BridgeError> {
        self.feeder.push_video(frame)
    }

    /// See [`RemoteFeeder::push_audio`].
    pub fn push_audio(&self, frame: RawAudioFrame<'_>) -> Result<(), BridgeError> {
        self.feeder.push_audio(frame)
    }

    /// The pipeline that ends in the appsink (a clone of the handle).
    pub fn take_sink_pipeline(&self) -> gst::Pipeline {
        self.pipeline.clone()
    }

    /// The appsink the tests read from.
    pub fn appsink(&self) -> &gst_app::AppSink {
        &self.appsink
    }

    /// The feeder (to reach its `appsrc`).
    pub fn feeder(&self) -> &RemoteFeeder {
        &self.feeder
    }

    /// Waits up to `timeout` for the next sample at the end of the pipeline.
    pub fn pull_sample(&self, timeout: Duration) -> Option<gst::Sample> {
        let nanos = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
        self.appsink
            .try_pull_sample(gst::ClockTime::from_nseconds(nanos))
    }

    /// Ends the stream.
    pub fn end_of_stream(&self) {
        self.feeder.end_of_stream();
    }
}

impl Drop for RemoteSource {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn check_size(width: u32, height: u32) -> Result<(), BridgeError> {
    if (1..=8192).contains(&width) && (1..=8192).contains(&height) {
        Ok(())
    } else {
        Err(BridgeError::Config(format!(
            "size {width}x{height} is outside 1..=8192"
        )))
    }
}

fn video_caps(width: u32, height: u32) -> gst::Caps {
    gst::Caps::builder("video/x-raw")
        .field("format", "I420")
        .field("width", width as i32)
        .field("height", height as i32)
        .field("framerate", gst::Fraction::new(0, 1))
        .build()
}

fn audio_caps(channels: u16) -> gst::Caps {
    gst::Caps::builder("audio/x-raw")
        .field("format", "S16LE")
        .field("rate", AUDIO_SAMPLE_RATE as i32)
        .field("channels", i32::from(channels))
        .field("layout", "interleaved")
        .build()
}

/// The appsrc: live, time format, a bounded internal queue that drops the oldest buffer instead of
/// growing when the consumer is slower than the network.
fn make_appsrc(caps: &gst::Caps, max_bytes: usize) -> Result<gst_app::AppSrc, BridgeError> {
    let appsrc = gst::ElementFactory::make("appsrc")
        .property("is-live", true)
        .property("caps", caps)
        .property_from_str("format", "time")
        .property_from_str("stream-type", "stream")
        .property("block", false)
        .property("do-timestamp", false)
        .property("max-bytes", max_bytes as u64)
        .build()
        .map_err(|_| BridgeError::MissingElement("appsrc"))?;
    if appsrc.find_property("leaky-type").is_some() {
        appsrc.set_property_from_str("leaky-type", "downstream");
    }
    appsrc
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| BridgeError::Pipeline("appsrc is not an AppSrc".into()))
}

/// Copies the three planes into one buffer in GStreamer's default I420 layout for the size.
fn pack_i420(frame: &RawVideoFrame<'_>) -> Result<gst::Buffer, BridgeError> {
    let info =
        gst_video::VideoInfo::builder(gst_video::VideoFormat::I420, frame.width, frame.height)
            .build()
            .map_err(|error| BridgeError::Pipeline(error.to_string()))?;
    let mut buffer = gst::Buffer::with_size(info.size())
        .map_err(|error| BridgeError::Pipeline(error.to_string()))?;
    {
        let buffer = buffer
            .get_mut()
            .ok_or_else(|| BridgeError::Pipeline("the new buffer is shared".into()))?;
        let mut map = buffer.map_writable()?;
        let dst = map.as_mut_slice();
        let (cw, ch) = (
            chroma_dim(frame.width) as usize,
            chroma_dim(frame.height) as usize,
        );
        let planes = [
            (
                frame.y,
                frame.stride_y,
                frame.width as usize,
                frame.height as usize,
            ),
            (frame.u, frame.stride_u, cw, ch),
            (frame.v, frame.stride_v, cw, ch),
        ];
        for (plane, (src, src_stride, row_len, rows)) in planes.into_iter().enumerate() {
            let dst_stride = info.stride()[plane] as usize;
            let dst_offset = info.offset()[plane];
            if src_stride == dst_stride {
                let len = dst_stride * (rows - 1) + row_len;
                dst[dst_offset..dst_offset + len].copy_from_slice(&src[..len]);
            } else {
                for row in 0..rows {
                    let from = row * src_stride;
                    let to = dst_offset + row * dst_stride;
                    dst[to..to + row_len].copy_from_slice(&src[from..from + row_len]);
                }
            }
        }
    }
    Ok(buffer)
}
