// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Behaviour of `CaptureStream`: delivery, fan-out, removal, stop, late sinks, clocks.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use common::{Recorder, Seen, anchored_map, median, micros_of, wait_until, wall_micros};
use mercury_frame_sink::{FrameSinkId, MediaFrameSink, RawVideoFrame, chroma_dim};
use mercury_rtc_bridge::{
    BridgeError, CameraConfig, CaptureStream, MicrophoneConfig, SourceFormat, StreamKind,
    TestPattern,
};

const STARTUP: Duration = Duration::from_secs(10);

fn camera(width: u32, height: u32) -> CaptureStream {
    CaptureStream::fake_camera(CameraConfig {
        width,
        height,
        fps: 30,
        pattern: TestPattern::Smpte,
    })
    .expect("a fake camera")
}

fn microphone(channels: u16) -> CaptureStream {
    CaptureStream::fake_microphone(MicrophoneConfig {
        channels,
        frequency: 440.0,
    })
    .expect("a fake microphone")
}

/// Frames that arrived within `window` of the first one.
fn within(seen: &[Seen], window: Duration) -> usize {
    let Some(first) = seen.first() else { return 0 };
    seen.iter()
        .filter(|s| s.at.duration_since(first.at) <= window)
        .count()
}

fn video_one_second(width: u32, height: u32) {
    let stream = camera(width, height);
    let rec = Arc::new(Recorder::new());
    stream.add_frame_sink(rec.clone()).expect("add sink");
    assert!(
        wait_until(STARTUP, || rec.video_count() > 0),
        "no frame arrived: {:?}",
        stream.take_error()
    );
    std::thread::sleep(Duration::from_millis(1100));
    let seen = rec.video_times();
    let in_one_second = within(&seen, Duration::from_secs(1));
    println!(
        "RESULT camera {width}x{height}: {in_one_second} frames in 1 s ({} total), invalid {}",
        seen.len(),
        rec.invalid_count()
    );
    assert!(
        in_one_second >= 25,
        "only {in_one_second} frames in 1 s at {width}x{height}"
    );
    assert_eq!(
        rec.invalid_count(),
        0,
        "{:?}",
        rec.first_error.lock().unwrap()
    );
    assert_eq!(rec.sizes.lock().unwrap().as_slice(), &[(width, height)]);
    assert_eq!(rec.ended_count(), 0);
    assert_eq!(stream.take_error(), None);
    stream.stop();
}

#[test]
fn the_fake_camera_delivers_valid_frames_at_640x360() {
    video_one_second(640, 360);
}

#[test]
fn the_fake_camera_delivers_valid_frames_at_an_odd_size() {
    video_one_second(641, 361);
}

#[test]
fn the_fake_camera_reports_its_kind_and_refuses_bad_configs() {
    let stream = camera(320, 240);
    assert_eq!(
        stream.kind(),
        StreamKind::Video {
            width: 320,
            height: 240,
            fps: 30
        }
    );
    for cfg in [
        CameraConfig::new(0, 240, 30),
        CameraConfig::new(320, 0, 30),
        CameraConfig::new(320, 240, 0),
        CameraConfig::new(9000, 240, 30),
        CameraConfig::new(320, 240, 1000),
    ] {
        assert!(
            matches!(CaptureStream::fake_camera(cfg), Err(BridgeError::Config(_))),
            "{cfg:?} should be refused"
        );
    }
    for cfg in [
        MicrophoneConfig {
            channels: 0,
            frequency: 440.0,
        },
        MicrophoneConfig {
            channels: 3,
            frequency: 440.0,
        },
        MicrophoneConfig {
            channels: 1,
            frequency: 0.0,
        },
        MicrophoneConfig {
            channels: 1,
            frequency: f64::NAN,
        },
    ] {
        assert!(
            matches!(
                CaptureStream::fake_microphone(cfg),
                Err(BridgeError::Config(_))
            ),
            "{cfg:?} should be refused"
        );
    }
}

fn audio_one_second(channels: u16) {
    let stream = microphone(channels);
    let rec = Arc::new(Recorder::new());
    stream.add_frame_sink(rec.clone()).expect("add sink");
    assert!(
        wait_until(STARTUP, || rec.audio_count() > 0),
        "no block arrived: {:?}",
        stream.take_error()
    );
    // The rate is counted over three seconds: on a shared 3-core virtual machine (the CI runner) a
    // scheduling hiccup of 50 to 100 ms moves up to ten blocks across the edge of a one-second window
    // (91 and 98 blocks were seen there), while the average over three seconds is within 3 %.
    std::thread::sleep(Duration::from_millis(3100));
    let seen = rec.audio_times();
    let first = seen[0].at;
    let in_window = |seconds: u64| {
        seen.iter()
            .filter(|s| s.at.duration_since(first) < Duration::from_secs(seconds))
            .count()
    };
    let (blocks, first_second) = (in_window(3), in_window(1));
    let peak = *rec.peak_sample.lock().unwrap();
    println!(
        "RESULT microphone {channels} ch: {blocks} blocks in 3 s ({first_second} in the first second), invalid {}, peak {peak}",
        rec.invalid_count()
    );
    assert!(
        (285..=315).contains(&blocks),
        "{blocks} blocks in 3 s with {channels} channels (95 to 105 a second)"
    );
    assert_eq!(
        rec.invalid_count(),
        0,
        "{:?}",
        rec.first_error.lock().unwrap()
    );
    assert!(
        peak > 20_000,
        "a 0.8 volume sine must peak near 26000: {peak}"
    );
    assert_eq!(stream.take_error(), None);
}

#[test]
fn the_fake_microphone_delivers_hundred_valid_blocks_a_second_mono() {
    audio_one_second(1);
}

#[test]
fn the_fake_microphone_delivers_hundred_valid_blocks_a_second_stereo() {
    audio_one_second(2);
}

#[test]
fn an_nv12_source_is_converted_to_i420_in_the_branch() {
    let stream =
        CaptureStream::fake_camera_with_format(CameraConfig::new(641, 361, 30), SourceFormat::Nv12)
            .expect("an NV12 camera");
    let rec = Arc::new(Recorder::new());
    stream.add_frame_sink(rec.clone()).unwrap();
    assert!(wait_until(STARTUP, || rec.video_count() >= 20));
    assert_eq!(rec.invalid_count(), 0);
    assert_eq!(rec.sizes.lock().unwrap().as_slice(), &[(641, 361)]);
}

#[test]
fn two_sinks_both_receive_and_removing_one_leaves_the_other_running() {
    let stream = camera(320, 240);
    let (a, b) = (Arc::new(Recorder::new()), Arc::new(Recorder::new()));
    let id_a = stream.add_frame_sink(a.clone()).unwrap();
    let id_b = stream.add_frame_sink(b.clone()).unwrap();
    assert_ne!(id_a, id_b);
    assert_eq!(stream.sink_count(), 2);
    assert!(wait_until(STARTUP, || a.video_count() >= 10 && b.video_count() >= 10));

    stream.remove_frame_sink(id_a);
    assert_eq!(stream.sink_count(), 1);
    let frozen = a.video_count();
    let b_before = b.video_count();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(a.video_count(), frozen, "a removed sink was called again");
    assert!(
        b.video_count() >= b_before + 10,
        "the other sink slowed down: {} -> {}",
        b_before,
        b.video_count()
    );
    assert_eq!(a.ended_count(), 0, "a removed sink gets no ended()");
    assert_eq!(stream.sink_stats(id_a), None);
    stream.remove_frame_sink(id_a); // unknown ids are ignored
    assert!(stream.sink_stats(id_b).is_some());

    stream.stop();
    assert_eq!(b.ended_count(), 1);
    assert_eq!(a.ended_count(), 0);
    assert_eq!(a.invalid_count() + b.invalid_count(), 0);
}

#[test]
fn stop_ends_every_sink_exactly_once_and_no_frame_follows() {
    let stream = camera(320, 240);
    let sinks: Vec<_> = (0..3).map(|_| Arc::new(Recorder::new())).collect();
    for sink in &sinks {
        stream.add_frame_sink(sink.clone()).unwrap();
    }
    assert!(wait_until(STARTUP, || sinks
        .iter()
        .all(|s| s.video_count() >= 5)));
    stream.stop();
    let counts: Vec<_> = sinks.iter().map(|s| s.video_count()).collect();
    for sink in &sinks {
        assert_eq!(sink.ended_count(), 1);
    }
    stream.stop();
    stream.stop();
    std::thread::sleep(Duration::from_millis(300));
    for (sink, count) in sinks.iter().zip(counts) {
        assert_eq!(sink.video_count(), count, "a frame arrived after stop()");
        assert_eq!(sink.ended_count(), 1, "ended() ran more than once");
        assert_eq!(sink.after_end.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn a_sink_added_after_stop_is_ended_at_once_and_gets_no_frame() {
    let stream = camera(320, 240);
    let early = Arc::new(Recorder::new());
    stream.add_frame_sink(early.clone()).unwrap();
    assert!(wait_until(STARTUP, || early.video_count() >= 3));
    stream.stop();

    let late = Arc::new(Recorder::new());
    let id = stream.add_frame_sink(late.clone()).expect("add after stop");
    assert_eq!(
        late.ended_count(),
        1,
        "ended() must come before add returns"
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(late.video_count(), 0);
    assert_eq!(stream.sink_count(), 0);
    stream.remove_frame_sink(id);
    assert_eq!(late.ended_count(), 1);

    // A stream that never had a sink behaves the same.
    let never = camera(320, 240);
    never.stop();
    let late = Arc::new(Recorder::new());
    never.add_frame_sink(late.clone()).unwrap();
    assert_eq!(late.ended_count(), 1);
    assert_eq!(early.ended_count(), 1);
}

#[test]
fn dropping_a_stream_without_stop_releases_it_and_ends_its_sinks() {
    let rec = Arc::new(Recorder::new());
    {
        let stream = camera(320, 240);
        stream.add_frame_sink(rec.clone()).unwrap();
        assert!(wait_until(STARTUP, || rec.video_count() >= 3));
    }
    assert_eq!(rec.ended_count(), 1);
    let count = rec.video_count();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(rec.video_count(), count);
    // Never started, then dropped: nothing to release, no panic.
    drop(camera(320, 240));
    drop(microphone(2));
}

#[test]
fn the_stream_survives_losing_its_last_sink() {
    let stream = camera(320, 240);
    let first = Arc::new(Recorder::new());
    let id = stream.add_frame_sink(first.clone()).unwrap();
    assert!(wait_until(STARTUP, || first.video_count() >= 3));
    stream.remove_frame_sink(id);
    assert_eq!(stream.sink_count(), 0);
    std::thread::sleep(Duration::from_millis(300));
    let second = Arc::new(Recorder::new());
    stream.add_frame_sink(second.clone()).unwrap();
    assert!(
        wait_until(STARTUP, || second.video_count() >= 5),
        "no frame after re-adding: {:?}",
        stream.take_error()
    );
    assert_eq!(stream.take_error(), None);
    assert_eq!(first.ended_count(), 0);
}

/// Removes itself on its third frame, from inside the callback.
struct SelfRemover {
    stream: Weak<CaptureStream>,
    id: Mutex<Option<FrameSinkId>>,
    calls: AtomicU64,
    ended: AtomicU64,
}

impl MediaFrameSink for SelfRemover {
    fn video_frame(&self, _: RawVideoFrame<'_>) {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 3 {
            let id = self.id.lock().unwrap().take();
            if let (Some(id), Some(stream)) = (id, self.stream.upgrade()) {
                stream.remove_frame_sink(id);
            }
        }
    }
    fn ended(&self) {
        self.ended.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_sink_may_remove_itself_from_inside_its_callback() {
    let stream = Arc::new(camera(320, 240));
    let other = Arc::new(Recorder::new());
    stream.add_frame_sink(other.clone()).unwrap();
    let me = Arc::new(SelfRemover {
        stream: Arc::downgrade(&stream),
        id: Mutex::new(None),
        calls: AtomicU64::new(0),
        ended: AtomicU64::new(0),
    });
    let id = stream.add_frame_sink(me.clone()).unwrap();
    *me.id.lock().unwrap() = Some(id);
    assert!(wait_until(STARTUP, || stream.sink_count() == 1));
    let calls = me.calls.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        me.calls.load(Ordering::SeqCst),
        calls,
        "called after removal"
    );
    assert!(calls >= 3);
    let before = other.video_count();
    std::thread::sleep(Duration::from_millis(300));
    assert!(other.video_count() > before, "the other sink stalled");
    stream.stop();
    assert_eq!(me.ended.load(Ordering::SeqCst), 0);
    assert_eq!(other.ended_count(), 1);
}

/// Stops the whole stream on its second frame, from inside the callback.
struct Stopper {
    stream: Weak<CaptureStream>,
    calls: AtomicU64,
    ended: AtomicU64,
}

impl MediaFrameSink for Stopper {
    fn video_frame(&self, _: RawVideoFrame<'_>) {
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == 2
            && let Some(stream) = self.stream.upgrade()
        {
            stream.stop();
        }
    }
    fn ended(&self) {
        self.ended.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_sink_may_stop_the_stream_from_inside_its_callback() {
    let stream = Arc::new(camera(320, 240));
    let stopper = Arc::new(Stopper {
        stream: Arc::downgrade(&stream),
        calls: AtomicU64::new(0),
        ended: AtomicU64::new(0),
    });
    let other = Arc::new(Recorder::new());
    stream.add_frame_sink(stopper.clone()).unwrap();
    stream.add_frame_sink(other.clone()).unwrap();
    assert!(
        wait_until(STARTUP, || stopper.ended.load(Ordering::SeqCst) == 1
            && other.ended_count() == 1),
        "the stop from inside the callback never completed"
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(stopper.ended.load(Ordering::SeqCst), 1);
    assert_eq!(other.ended_count(), 1);
    assert_eq!(other.after_end.load(Ordering::SeqCst), 0);
}

/// Checks the video planes of every frame against the pattern: SMPTE bars are never all one value.
#[derive(Default)]
struct PlaneChecker {
    frames: AtomicU64,
    flat: AtomicU64,
    bad_strides: AtomicU64,
}

impl MediaFrameSink for PlaneChecker {
    fn video_frame(&self, frame: RawVideoFrame<'_>) {
        self.frames.fetch_add(1, Ordering::SeqCst);
        let row = &frame.y[..frame.width as usize];
        if row.iter().all(|b| *b == row[0]) {
            self.flat.fetch_add(1, Ordering::SeqCst);
        }
        if frame.stride_u < chroma_dim(frame.width) as usize
            || frame.stride_y < frame.width as usize
        {
            self.bad_strides.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[test]
fn the_frames_carry_real_pixels_with_usable_strides() {
    let stream = camera(641, 361);
    let checker = Arc::new(PlaneChecker::default());
    stream.add_frame_sink(checker.clone()).unwrap();
    assert!(wait_until(STARTUP, || checker
        .frames
        .load(Ordering::SeqCst)
        >= 5));
    stream.stop();
    assert_eq!(
        checker.flat.load(Ordering::SeqCst),
        0,
        "SMPTE bars are not flat"
    );
    assert_eq!(checker.bad_strides.load(Ordering::SeqCst), 0);
}

fn check_clock(
    stream: &CaptureStream,
    seen: &[Seen],
    what: &str,
    max_median_lag_us: i64,
    max_ahead_us: i64,
) {
    let _ = wall_micros();
    assert!(seen.len() >= 20, "{what}: only {} frames", seen.len());
    assert!(
        seen.windows(2).all(|w| w[0].timestamp <= w[1].timestamp),
        "{what}: timestamps went backwards"
    );
    let (map, uncertainty) = anchored_map(|| stream.producer_now());
    // The map was anchored after the frames were seen, so the offset is exact: the same
    // producer clock reads against the wall clock, both monotonic.
    let lags: Vec<i64> = seen
        .iter()
        .map(|s| micros_of(s.at) - map.to_consumer(s.timestamp).0)
        .collect();
    let median_lag = median(lags.clone());
    let min_lag = *lags.iter().min().unwrap();
    let max_lag = *lags.iter().max().unwrap();
    println!(
        "RESULT clock {what}: lag capture->sink median {median_lag} us, min {min_lag} us, max {max_lag} us (anchor +-{uncertainty} us)"
    );
    assert!(
        min_lag >= -(max_ahead_us + uncertainty),
        "{what}: a frame is {min_lag} us in the future"
    );
    assert!(
        median_lag <= max_median_lag_us,
        "{what}: median lag {median_lag} us"
    );
    // The producer clock and the wall clock agree: reading both again, adjacent, after the run.
    let mut best = i64::MAX;
    for _ in 0..8 {
        let before = wall_micros();
        let producer = map.to_consumer(stream.producer_now()).0;
        let after = wall_micros();
        best = best.min((producer - (before + after) / 2).abs());
    }
    println!("RESULT clock {what}: producer_now vs wall after the run: {best} us");
    assert!(best <= 2_000, "{what}: clocks differ by {best} us");
}

#[test]
fn video_timestamps_do_not_go_backwards_and_agree_with_the_wall_clock() {
    let _ = wall_micros();
    let stream = camera(640, 360);
    let rec = Arc::new(Recorder::new());
    stream.add_frame_sink(rec.clone()).unwrap();
    assert!(wait_until(STARTUP, || rec.video_count() >= 5));
    std::thread::sleep(Duration::from_millis(1500));
    let seen = rec.video_times();
    check_clock(&stream, &seen, "video", 150_000, 2_000);
    let deltas: Vec<i64> = seen
        .windows(2)
        .map(|w| w[1].timestamp.0 - w[0].timestamp.0)
        .collect();
    let mean = deltas.iter().sum::<i64>() as f64 / deltas.len() as f64;
    println!("RESULT clock video: mean frame interval {mean:.0} us (33333 expected)");
    assert!((mean - 33_333.0).abs() < 3_500.0, "interval {mean}");
}

#[test]
fn audio_timestamps_do_not_go_backwards_and_agree_with_the_wall_clock() {
    let _ = wall_micros();
    let stream = microphone(1);
    let rec = Arc::new(Recorder::new());
    stream.add_frame_sink(rec.clone()).unwrap();
    assert!(wait_until(STARTUP, || rec.audio_count() >= 10));
    std::thread::sleep(Duration::from_millis(1500));
    let seen = rec.audio_times();
    check_clock(&stream, &seen, "audio", 150_000, 13_000);
    let span = seen.last().unwrap().timestamp.0 - seen.first().unwrap().timestamp.0;
    let per_block = span as f64 / (seen.len() - 1) as f64;
    println!("RESULT clock audio: mean block interval {per_block:.0} us (10000 expected)");
    assert!((per_block - 10_000.0).abs() < 100.0, "interval {per_block}");
}

#[test]
fn a_camera_and_a_microphone_started_apart_share_one_time_scale() {
    let _ = wall_micros();
    let video = camera(320, 240);
    let v = Arc::new(Recorder::new());
    video.add_frame_sink(v.clone()).unwrap();
    assert!(wait_until(STARTUP, || v.video_count() >= 3));
    std::thread::sleep(Duration::from_millis(700));
    let audio = microphone(1);
    let a = Arc::new(Recorder::new());
    audio.add_frame_sink(a.clone()).unwrap();
    assert!(wait_until(STARTUP, || a.audio_count() >= 20));
    std::thread::sleep(Duration::from_millis(500));
    let last_video = v.video_times().last().unwrap().timestamp.0;
    let last_audio = a.audio_times().last().unwrap().timestamp.0;
    let first_audio = a.audio_times().first().unwrap().timestamp.0;
    let first_video = v.video_times().first().unwrap().timestamp.0;
    println!(
        "RESULT clock shared scale: audio started {} us after video; last video {last_video}, last audio {last_audio}",
        first_audio - first_video
    );
    assert!(
        first_audio - first_video > 500_000,
        "the microphone started 700 ms later but its timestamps say {} us",
        first_audio - first_video
    );
    assert!((last_audio - last_video).abs() < 250_000);
}

#[test]
fn a_microphone_with_ten_millisecond_buffers_is_never_ahead_of_the_wall_clock() {
    let _ = wall_micros();
    let stream = CaptureStream::fake_microphone_with_buffer(
        MicrophoneConfig {
            channels: 2,
            frequency: 440.0,
        },
        Some(480),
    )
    .unwrap();
    let rec = Arc::new(Recorder::new());
    stream.add_frame_sink(rec.clone()).unwrap();
    assert!(wait_until(STARTUP, || rec.audio_count() >= 10));
    std::thread::sleep(Duration::from_millis(1500));
    let seen = rec.audio_times();
    check_clock(
        &stream,
        &seen,
        "audio (10 ms source buffers)",
        150_000,
        2_000,
    );
    assert_eq!(rec.invalid_count(), 0);
    assert!(matches!(
        CaptureStream::fake_microphone_with_buffer(MicrophoneConfig::default(), Some(0)),
        Err(BridgeError::Config(_))
    ));
}
