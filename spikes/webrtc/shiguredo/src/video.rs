// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A video source that frames are pushed into, and a meter for remote video.
//!
//! `AdaptedVideoTrackSource` is the wrapper's only source type: the caller asks it how to adapt a
//! frame (`adapt_frame`, which applies the frame-rate and resolution limits the encoder side asked
//! for), crops and scales when told to, and calls `on_frame`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use mercury_frame_sink::{MediaFrameSink, Micros, RawVideoFrame, chroma_dim};
use shiguredo_webrtc::{
    AdaptedVideoTrackSource, I420Buffer, MediaStreamTrack, VideoFrame, VideoFrameRef, VideoSink,
    VideoSinkHandler, VideoSinkWants, VideoTrack,
};

use crate::Result;
use crate::engine::Engine;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What happened to a pushed frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// Given to libwebrtc.
    Pushed,
    /// libwebrtc's adapter asked for the frame to be dropped (frame-rate limit or no consumer).
    Dropped,
    /// The frame failed `validate()`.
    Invalid,
}

/// The pushing half of a local video source: cheap to clone, usable from any thread.
#[derive(Clone)]
pub struct VideoPusher {
    source: AdaptedVideoTrackSource,
}

fn copy_plane(
    dst: &mut [u8],
    dst_stride: usize,
    src: &[u8],
    src_stride: usize,
    width: usize,
    rows: usize,
) {
    for row in 0..rows {
        dst[row * dst_stride..row * dst_stride + width]
            .copy_from_slice(&src[row * src_stride..row * src_stride + width]);
    }
}

impl VideoPusher {
    /// Copies `frame` into an I420 buffer, adapts it as libwebrtc asks and hands it over.
    /// `timestamp_us` is on libwebrtc's clock.
    pub fn push(&self, frame: &RawVideoFrame<'_>, timestamp_us: i64) -> Push {
        if frame.validate().is_err() {
            return Push::Invalid;
        }
        let (width, height) = (frame.width as i32, frame.height as i32);
        let adapted = self.source.adapt_frame(width, height, timestamp_us);
        if !adapted.applied {
            return Push::Dropped;
        }
        let mut buffer = I420Buffer::new(width, height);
        let strides = (
            buffer.stride_y() as usize,
            buffer.stride_u() as usize,
            buffer.stride_v() as usize,
        );
        let (chroma_width, chroma_height) = (
            chroma_dim(frame.width) as usize,
            chroma_dim(frame.height) as usize,
        );
        let (y, u, v) = buffer.planes_mut();
        copy_plane(
            y,
            strides.0,
            frame.y,
            frame.stride_y,
            frame.width as usize,
            frame.height as usize,
        );
        copy_plane(
            u,
            strides.1,
            frame.u,
            frame.stride_u,
            chroma_width,
            chroma_height,
        );
        copy_plane(
            v,
            strides.2,
            frame.v,
            frame.stride_v,
            chroma_width,
            chroma_height,
        );
        let mut buffer = buffer.cast_to_video_frame_buffer();
        let size = adapted.size;
        if (
            size.adapted_width,
            size.adapted_height,
            size.crop_width,
            size.crop_height,
        ) != (width, height, width, height)
        {
            let Some(scaled) = buffer.crop_and_scale(
                size.crop_x,
                size.crop_y,
                size.crop_width,
                size.crop_height,
                size.adapted_width,
                size.adapted_height,
            ) else {
                return Push::Dropped;
            };
            buffer = scaled;
        }
        let frame = VideoFrame::builder(&buffer)
            .set_timestamp_us(timestamp_us)
            .build();
        self.source.on_frame(&frame);
        Push::Pushed
    }
}

/// A local video track fed by [`VideoPusher`].
pub struct LocalVideo {
    source: AdaptedVideoTrackSource,
    track: VideoTrack,
}

impl LocalVideo {
    /// A new source and a track on it, made by `engine`'s factory.
    pub fn new(engine: &Engine, track_id: &str) -> Result<LocalVideo> {
        let source = AdaptedVideoTrackSource::new();
        let track = engine
            .factory()
            .create_video_track(&source.cast_to_video_track_source(), track_id)?;
        Ok(LocalVideo { source, track })
    }

    /// The video track (for `add_track` and `add_transceiver_with_track`).
    pub fn track(&self) -> &VideoTrack {
        &self.track
    }

    /// The track as a generic media track.
    pub fn media_track(&self) -> MediaStreamTrack {
        self.track.cast_to_media_stream_track()
    }

    /// The handle frames are pushed through.
    pub fn pusher(&self) -> VideoPusher {
        VideoPusher {
            source: self.source.clone(),
        }
    }
}

/// What a remote video track delivered.
#[derive(Debug, Clone, Default)]
pub struct VideoMeter {
    /// Frames seen.
    pub frames: u64,
    /// When each frame arrived (capped at 20 000).
    pub arrivals: Vec<Instant>,
    /// Frames per (width, height).
    pub sizes: BTreeMap<(i32, i32), u64>,
    /// The size of the last frame.
    pub last_size: (i32, i32),
    /// Times a frame's timestamp was smaller than the one before.
    pub timestamp_regressions: u64,
    last_timestamp: Option<i64>,
}

impl VideoMeter {
    /// Frames that arrived at or after `since`.
    pub fn frames_since(&self, since: Instant) -> usize {
        self.arrivals.iter().filter(|at| **at >= since).count()
    }

    /// Frames of exactly `size`, at or after `since`, given a list of arrival sizes is not kept:
    /// use [`VideoMeter::sizes`] for the totals.
    pub fn size_summary(&self) -> String {
        self.sizes
            .iter()
            .map(|((w, h), n)| format!("{w}x{h}:{n}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

struct MeterHandler {
    state: Arc<Mutex<VideoMeter>>,
    forward: Option<Arc<dyn MediaFrameSink>>,
}

impl VideoSinkHandler for MeterHandler {
    fn on_frame(&mut self, frame: VideoFrameRef<'_>) {
        let (width, height, timestamp) = (frame.width(), frame.height(), frame.timestamp_us());
        {
            let mut meter = lock(&self.state);
            meter.frames += 1;
            if meter.arrivals.len() < 20_000 {
                meter.arrivals.push(Instant::now());
            }
            *meter.sizes.entry((width, height)).or_default() += 1;
            meter.last_size = (width, height);
            if meter.last_timestamp.is_some_and(|last| timestamp < last) {
                meter.timestamp_regressions += 1;
            }
            meter.last_timestamp = Some(timestamp);
        }
        let Some(sink) = &self.forward else { return };
        let mut buffer = frame.buffer();
        let Some(i420) = buffer.to_i420() else { return };
        sink.video_frame(RawVideoFrame {
            width: width as u32,
            height: height as u32,
            y: i420.y_data(),
            u: i420.u_data(),
            v: i420.v_data(),
            stride_y: i420.stride_y() as usize,
            stride_u: i420.stride_u() as usize,
            stride_v: i420.stride_v() as usize,
            timestamp: Micros(timestamp),
        });
    }
}

/// A sink on a remote video track that measures what arrives and optionally forwards each frame
/// to a [`MediaFrameSink`] (the reverse direction of the bridge: remote frames into an `appsrc`).
pub struct RemoteVideo {
    track: VideoTrack,
    sink: VideoSink,
    state: Arc<Mutex<VideoMeter>>,
}

impl RemoteVideo {
    /// Attaches the meter to `track`.
    pub fn attach(track: &VideoTrack, forward: Option<Arc<dyn MediaFrameSink>>) -> RemoteVideo {
        let state = Arc::new(Mutex::new(VideoMeter::default()));
        let sink = VideoSink::new_with_handler(Box::new(MeterHandler {
            state: Arc::clone(&state),
            forward,
        }));
        track.add_or_update_sink(&sink, &VideoSinkWants::new());
        RemoteVideo {
            track: track.clone(),
            sink,
            state,
        }
    }

    /// The numbers so far.
    pub fn meter(&self) -> VideoMeter {
        lock(&self.state).clone()
    }
}

impl Drop for RemoteVideo {
    fn drop(&mut self) {
        // libwebrtc keeps a pointer to the sink until it is removed.
        self.track.remove_sink(&self.sink);
    }
}
