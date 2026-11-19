// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The GStreamer side of the libwebrtc bridge (sp-rtc lane, item 2).
//!
//! Two directions, both built on the contract of `mercury-frame-sink`:
//!
//! * [`CaptureStream`] is what `GStreamerMediaStream` becomes in the product: one capture source
//!   (here `videotestsrc` or `audiotestsrc`, never a real device), a `tee`, and one appsink branch
//!   per [`MediaFrameSink`](mercury_frame_sink::MediaFrameSink). A WebRTC sender registers a sink
//!   and receives I420 frames or 10 ms S16 blocks on a streaming thread.
//! * [`RemoteSource`] and [`RemoteFeeder`] take frames from a WebRTC receiver and turn them into
//!   timestamped GStreamer buffers in an `appsrc`.
//!
//! The crate is its own cargo workspace and is not part of the default build. It has no `unsafe`
//! and never names a device element.
//!
//! # The pipelines
//!
//! Video, `fake_camera` (the capsfilter fixes size, rate and source format):
//!
//! ```text
//! videotestsrc is-live=true ! capsfilter(video/x-raw,format=I420,width=W,height=H,framerate=F/1)
//!   ! videoconvert ! tee allow-not-linked=true name=t
//!   t. ! queue(max-size-buffers=2,max-size-bytes=0,max-size-time=0,leaky=downstream)
//!      ! videoconvert ! capsfilter(video/x-raw,format=I420)
//!      ! appsink(sync=false,async=false,drop=true,max-buffers=2,emit-signals=false)  # one per sink
//! ```
//!
//! Audio, `fake_microphone`:
//!
//! ```text
//! audiotestsrc is-live=true wave=sine freq=F volume=0.8 ! audioconvert ! audioresample
//!   ! capsfilter(audio/x-raw,format=S16LE,rate=48000,channels=N,layout=interleaved)
//!   ! audiobuffersplit(output-buffer-duration=1/100) ! tee allow-not-linked=true name=t
//!   t. ! queue(max-size-buffers=0,max-size-bytes=0,max-size-time=500ms,leaky=downstream)
//!      ! appsink(sync=false,async=false,drop=true,max-buffers=50,emit-signals=false)  # one per sink
//! ```
//!
//! Remote, `RemoteSource` (the chains `create_video_from` and `create_audio_from` build, plus an
//! appsink):
//!
//! ```text
//! appsrc(is-live,format=time,leaky-type=downstream) ! videoconvert ! queue ! appsink
//! appsrc(is-live,format=time,leaky-type=downstream) ! queue ! audioconvert ! audioresample ! queue ! appsink
//! ```
//!
//! # Threads, ordering and lifetime
//!
//! * A sink is called on the streaming thread of its own branch queue, so a slow sink delays
//!   only itself; the leaky queue drops the oldest buffers. It must not block for long.
//! * `add_frame_sink`, `remove_frame_sink` and `stop` may be called from any thread, including
//!   from inside a sink callback (the work then runs on a short-lived helper thread).
//! * `stop` (and dropping the stream) sets the pipeline to NULL, which joins every streaming
//!   thread, and only then calls `ended()` on each remaining sink, once.
//!
//! # Timestamps
//!
//! All bridge pipelines use one clock (GStreamer's monotonic `SystemClock`, the one
//! `servo-media-gstreamer` pins its pipelines to). A frame's timestamp is the sink element's base
//! time (the clock time at which its pipeline went PLAYING) plus the running time of the buffer
//! PTS, in microseconds, i.e. the clock time of the capture. So `CaptureStream::producer_now()`
//! and frame timestamps are directly comparable, and a camera and a microphone started at
//! different moments are still on the same scale. A consumer anchors a
//! [`ClockMap`](mercury_frame_sink::ClockMap) with `(producer_now(), its own clock now)`.
//!
//! # How the product feeds `GStreamerMediaStream` (read, not edited, in `third_party`)
//!
//! Incoming (remote frames into `<video srcObject>`): build a [`RemoteFeeder`], pass
//! `feeder.appsrc().clone().upcast::<gstreamer::Element>()` to
//! `GStreamerMediaStream::create_video_from(source)` (or `create_audio_from`), and call
//! `feeder.push_video` / `push_audio` from the libwebrtc receive callbacks. No engine change is
//! needed for this direction: `create_*_from` accepts any source element, links `source !
//! videoconvert ! queue` (video) or `source ! queue ! audioconvert ! audioresample ! queue`
//! (audio), and the consumer (`MediaSink::add_stream`, `encoded()`, `AudioStreamReader`) attaches
//! as for a camera. `RemoteSource` proves in the tests that those chains keep I420 and S16 frames
//! byte-identical.
//!
//! Outgoing (camera or microphone into a WebRTC sender) needs the changes listed in the sp-rtc
//! lane's part file `R18-parts/2-bridge.md`: a `tee` as the stream's last element in place of
//! the single `queue`, the removal of `assert!(self.pipeline.is_none())` in `attach_to_pipeline`,
//! a `SinkSet` and `add_frame_sink`/`remove_frame_sink` on `MediaStream`, and
//! `SinkSet::end()` after the pipeline reaches NULL in `GStreamerMediaStream::stop`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod capture;
mod error;
mod remote;
mod util;

pub use capture::{
    CameraConfig, CaptureStream, MicrophoneConfig, SinkStats, SourceFormat, StreamKind, TestPattern,
};
pub use error::BridgeError;
pub use remote::{RemoteFeeder, RemoteSource};
pub use util::{clock_now, init};
