// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Behaviour of `RemoteSource` and `RemoteFeeder`: byte-identical round trips and timestamps.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_video as gst_video;
use gstreamer_video::VideoFrameExt;
use mercury_frame_sink::{
    AUDIO_SAMPLES_PER_BLOCK, MediaFrameSink, Micros, RawAudioFrame, RawVideoFrame, chroma_dim,
};
use mercury_rtc_bridge::{BridgeError, CameraConfig, CaptureStream, RemoteFeeder, RemoteSource};

const PULL: Duration = Duration::from_secs(5);

/// Deterministic pseudo-random bytes (no `rand` dependency).
fn pattern(seed: u32, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(12345);
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

struct Planes {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    stride_y: usize,
    stride_u: usize,
}

/// A frame whose rows have `pad` extra bytes (the stride is bigger than the row).
fn planes(width: u32, height: u32, pad: usize, seed: u32) -> Planes {
    let (cw, ch) = (chroma_dim(width) as usize, chroma_dim(height) as usize);
    let stride_y = width as usize + pad;
    let stride_u = cw + pad;
    Planes {
        y: pattern(seed, stride_y * (height as usize - 1) + width as usize),
        u: pattern(seed + 1, stride_u * (ch - 1) + cw),
        v: pattern(seed + 2, stride_u * (ch - 1) + cw),
        stride_y,
        stride_u,
    }
}

impl Planes {
    fn frame(&self, width: u32, height: u32, timestamp: Micros) -> RawVideoFrame<'_> {
        RawVideoFrame {
            width,
            height,
            y: &self.y,
            u: &self.u,
            v: &self.v,
            stride_y: self.stride_y,
            stride_u: self.stride_u,
            stride_v: self.stride_u,
            timestamp,
        }
    }
}

fn rows_of(plane: &[u8], stride: usize, row_len: usize, rows: usize) -> Vec<&[u8]> {
    (0..rows)
        .map(|r| &plane[r * stride..r * stride + row_len])
        .collect()
}

fn assert_same_video(sample: &gst::Sample, sent: &Planes, width: u32, height: u32) {
    let caps = sample.caps().expect("caps");
    let info = gst_video::VideoInfo::from_caps(caps).expect("video info");
    assert_eq!(
        (info.format(), info.width(), info.height()),
        (gst_video::VideoFormat::I420, width, height)
    );
    let buffer = sample.buffer().expect("buffer");
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info).expect("map");
    let (cw, ch) = (chroma_dim(width) as usize, chroma_dim(height) as usize);
    let got = [
        rows_of(
            frame.plane_data(0).unwrap(),
            frame.plane_stride()[0] as usize,
            width as usize,
            height as usize,
        ),
        rows_of(
            frame.plane_data(1).unwrap(),
            frame.plane_stride()[1] as usize,
            cw,
            ch,
        ),
        rows_of(
            frame.plane_data(2).unwrap(),
            frame.plane_stride()[2] as usize,
            cw,
            ch,
        ),
    ];
    let want = [
        rows_of(&sent.y, sent.stride_y, width as usize, height as usize),
        rows_of(&sent.u, sent.stride_u, cw, ch),
        rows_of(&sent.v, sent.stride_u, cw, ch),
    ];
    for (plane, (got, want)) in got.iter().zip(&want).enumerate() {
        assert_eq!(got, want, "plane {plane} differs at {width}x{height}");
    }
}

fn micros_of_sample(source: &RemoteSource, sample: &gst::Sample) -> i64 {
    let pts = sample.buffer().unwrap().pts().expect("pts");
    let base = source.appsink().base_time().expect("base time");
    ((pts + base).nseconds() / 1000) as i64
}

#[test]
fn i420_frames_come_out_byte_identical_tight_and_padded_odd_and_even() {
    for (width, height, pad) in [
        (64u32, 48u32, 0usize),
        (64, 48, 5),
        (641, 361, 0),
        (641, 361, 7),
        (2, 2, 0),
        (1, 1, 0),
        (17, 9, 3),
    ] {
        let source = RemoteSource::video(width, height).unwrap();
        for k in 0..4u32 {
            let sent = planes(width, height, pad, 100 * k + width + height);
            let ts = Micros(source.feeder().now().0 + i64::from(k) * 33_333);
            source.push_video(sent.frame(width, height, ts)).unwrap();
            let sample = source
                .pull_sample(PULL)
                .unwrap_or_else(|| panic!("no frame back at {width}x{height} pad {pad}"));
            assert_same_video(&sample, &sent, width, height);
            assert_eq!(
                micros_of_sample(&source, &sample),
                ts.0,
                "timestamp changed"
            );
        }
    }
    println!("RESULT remote I420 round trip: byte-identical for 7 size/stride cases x 4 frames");
}

#[test]
fn a_frame_of_another_size_changes_the_caps_and_still_matches() {
    let source = RemoteSource::video(64, 48).unwrap();
    for (width, height) in [(64u32, 48u32), (80, 60), (33, 21), (64, 48)] {
        let sent = planes(width, height, 2, width * height);
        source
            .push_video(sent.frame(width, height, source.feeder().now()))
            .unwrap();
        let sample = source.pull_sample(PULL).expect("a frame");
        assert_same_video(&sample, &sent, width, height);
    }
}

fn s16_round_trip(channels: u16) {
    let source = RemoteSource::audio(channels).unwrap();
    let per_block = AUDIO_SAMPLES_PER_BLOCK * usize::from(channels);
    for k in 0..30i64 {
        let bytes = pattern(k as u32 + 7, per_block * 2);
        let samples: Vec<i16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| i16::from_le_bytes(*p))
            .collect();
        let ts = Micros(source.feeder().now().0 + k * 10_000);
        source
            .push_audio(RawAudioFrame {
                samples: &samples,
                channels,
                sample_rate: 48_000,
                timestamp: ts,
            })
            .unwrap();
        let sample = source.pull_sample(PULL).expect("a block back");
        let buffer = sample.buffer().unwrap();
        let map = buffer.map_readable().unwrap();
        assert_eq!(map.as_slice(), bytes.as_slice(), "block {k} differs");
        assert_eq!(
            buffer.duration(),
            Some(gst::ClockTime::from_mseconds(10)),
            "the duration of a block is 10 ms"
        );
        assert_eq!(micros_of_sample(&source, &sample), ts.0);
    }
    println!("RESULT remote S16 round trip: byte-identical, {channels} ch, 30 blocks");
}

#[test]
fn s16_blocks_come_out_byte_identical_mono() {
    s16_round_trip(1);
}

#[test]
fn s16_blocks_come_out_byte_identical_stereo() {
    s16_round_trip(2);
}

#[test]
fn wrong_kinds_sizes_and_frames_are_refused() {
    let video = RemoteSource::video(16, 16).unwrap();
    let audio = RemoteSource::audio(2).unwrap();
    let silence = vec![0i16; AUDIO_SAMPLES_PER_BLOCK * 2];
    let block = |channels| RawAudioFrame {
        samples: &silence,
        channels,
        sample_rate: 48_000,
        timestamp: Micros(0),
    };
    assert!(matches!(
        video.push_audio(block(2)),
        Err(BridgeError::Mismatch(_))
    ));
    assert!(matches!(
        audio.push_audio(block(1)),
        Err(BridgeError::Frame(_)) | Err(BridgeError::Mismatch(_))
    ));
    let sent = planes(16, 16, 0, 1);
    assert!(matches!(
        audio.push_video(sent.frame(16, 16, Micros(0))),
        Err(BridgeError::Mismatch(_))
    ));
    let mut short = sent.frame(16, 16, Micros(0));
    short.y = &sent.y[..10];
    assert!(matches!(
        video.push_video(short),
        Err(BridgeError::Frame(_))
    ));
    assert!(matches!(
        RemoteSource::video(0, 10),
        Err(BridgeError::Config(_))
    ));
    assert!(matches!(
        RemoteSource::audio(3),
        Err(BridgeError::Config(_))
    ));
}

#[test]
fn a_feeder_works_inside_the_product_chain_with_a_fixed_base_time() {
    // The product's pipeline: `GStreamerMediaStream::pipeline_or_new` pins the clock and a base
    // time chosen by the backend and sets the start time to NONE, so the base time never moves.
    // `create_video_from(source)` links `source ! videoconvert ! queue`, then a sink.
    mercury_rtc_bridge::init().unwrap();
    let feeder = RemoteFeeder::video(32, 24).unwrap();
    let pipeline = gst::Pipeline::with_name("product-like");
    let clock = gst::SystemClock::obtain();
    pipeline.use_clock(Some(&clock));
    pipeline.set_start_time(gst::ClockTime::NONE);
    let base = clock.time() - gst::ClockTime::from_seconds(5);
    pipeline.set_base_time(base);
    let appsrc: gst::Element = feeder.appsrc().clone().upcast();
    let convert = gst::ElementFactory::make("videoconvert").build().unwrap();
    let queue = gst::ElementFactory::make("queue").build().unwrap();
    let sink = gst::ElementFactory::make("appsink")
        .property("sync", false)
        .build()
        .unwrap();
    let chain = [&appsrc, &convert, &queue, &sink];
    pipeline.add_many(chain).unwrap();
    gst::Element::link_many(chain).unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();

    let sent = planes(32, 24, 0, 9);
    let ts = feeder.now();
    feeder.push_video(sent.frame(32, 24, ts)).unwrap();
    let sink = sink.downcast::<gstreamer_app::AppSink>().unwrap();
    let sample = sink
        .try_pull_sample(gst::ClockTime::from_seconds(5))
        .expect("a frame");
    let pts = sample.buffer().unwrap().pts().unwrap();
    // PTS is the running time of the pipeline: clock time minus its fixed base time, about 5 s.
    let expected = ts.0 * 1000 - base.nseconds() as i64;
    assert_eq!(pts.nseconds() as i64, expected);
    assert!(pts > gst::ClockTime::from_seconds(4));
    assert_same_video(&sample, &sent, 32, 24);
    pipeline.set_state(gst::State::Null).unwrap();
}

/// A sink adapter that pushes everything it gets into a `RemoteSource`: what a loopback test of a
/// WebRTC route does, here without the network.
struct Loopback {
    video: Option<RemoteSource>,
    audio: Option<RemoteSource>,
    failures: AtomicU64,
}

impl MediaFrameSink for Loopback {
    fn video_frame(&self, frame: RawVideoFrame<'_>) {
        if let Some(remote) = &self.video
            && remote.push_video(frame).is_err()
        {
            self.failures.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn audio_frame(&self, frame: RawAudioFrame<'_>) {
        if let Some(remote) = &self.audio
            && remote.push_audio(frame).is_err()
        {
            self.failures.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[test]
fn a_capture_stream_loops_back_through_a_remote_source() {
    let camera = CaptureStream::fake_camera(CameraConfig::new(641, 361, 30)).unwrap();
    let loopback = Arc::new(Loopback {
        video: Some(RemoteSource::video(641, 361).unwrap()),
        audio: None,
        failures: AtomicU64::new(0),
    });
    camera.add_frame_sink(loopback.clone()).unwrap();
    let remote = loopback.video.as_ref().unwrap();
    let mut got = Vec::new();
    while got.len() < 30 {
        let sample = remote.pull_sample(PULL).expect("a looped frame");
        got.push(micros_of_sample(remote, &sample));
        let info = gst_video::VideoInfo::from_caps(sample.caps().unwrap()).unwrap();
        assert_eq!((info.width(), info.height()), (641, 361));
    }
    assert!(
        got.windows(2).all(|w| w[0] <= w[1]),
        "timestamps went backwards"
    );
    assert_eq!(loopback.failures.load(Ordering::SeqCst), 0);
    camera.stop();

    let mic = CaptureStream::fake_microphone(mercury_rtc_bridge::MicrophoneConfig {
        channels: 2,
        frequency: 1000.0,
    })
    .unwrap();
    let loopback = Arc::new(Loopback {
        video: None,
        audio: Some(RemoteSource::audio(2).unwrap()),
        failures: AtomicU64::new(0),
    });
    mic.add_frame_sink(loopback.clone()).unwrap();
    let remote = loopback.audio.as_ref().unwrap();
    let mut blocks = 0;
    while blocks < 100 {
        let sample = remote.pull_sample(PULL).expect("a looped block");
        assert_eq!(
            sample.buffer().unwrap().size(),
            AUDIO_SAMPLES_PER_BLOCK * 2 * 2
        );
        blocks += 1;
    }
    assert_eq!(loopback.failures.load(Ordering::SeqCst), 0);
}
