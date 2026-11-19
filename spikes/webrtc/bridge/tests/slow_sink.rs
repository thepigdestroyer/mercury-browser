// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A slow sink must not slow a fast one and must not make memory grow. One test per process
//! (this file), so the memory numbers are the test's own.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Recorder, proc, wait_until};
use mercury_rtc_bridge::{CameraConfig, CaptureStream, MicrophoneConfig};

#[test]
fn a_slow_sink_drops_frames_instead_of_slowing_the_others_or_growing_memory() {
    // 720p: a frame is 1.4 MB, so a queue that grew would show within seconds (20 frames a
    // second that the slow sink cannot take would be 28 MB a second).
    let stream = CaptureStream::fake_camera(CameraConfig::new(1280, 720, 30)).unwrap();
    let fast = Arc::new(Recorder::new());
    let slow = Arc::new(Recorder::slow(Duration::from_millis(100)));
    stream.add_frame_sink(fast.clone()).unwrap();
    let slow_id = stream.add_frame_sink(slow.clone()).unwrap();
    assert!(wait_until(Duration::from_secs(10), || fast.video_count()
        >= 5
        && slow.video_count() >= 1));

    // Warm up one second, then measure six.
    std::thread::sleep(Duration::from_secs(1));
    let rss_start = proc::rss_bytes();
    let peak_start = proc::peak_rss_bytes();
    let (fast_start, slow_start) = (fast.video_count(), slow.video_count());
    let started = Instant::now();
    let mut rss_max = rss_start;
    while started.elapsed() < Duration::from_secs(6) {
        std::thread::sleep(Duration::from_millis(100));
        rss_max = rss_max.max(proc::rss_bytes());
    }
    let seconds = started.elapsed().as_secs_f64();
    let fast_frames = fast.video_count() - fast_start;
    let slow_frames = slow.video_count() - slow_start;
    let stats = stream.sink_stats(slow_id).unwrap();
    let peak_end = proc::peak_rss_bytes();
    let mb = |bytes: u64| bytes as f64 / 1_048_576.0;
    println!(
        "RESULT slow sink (720p30, {seconds:.1} s): fast sink {fast_frames} frames ({:.1}/s), slow sink {slow_frames} frames ({:.1}/s), slow branch offered {} delivered {} dropped {}",
        fast_frames as f64 / seconds,
        slow_frames as f64 / seconds,
        stats.offered,
        stats.delivered,
        stats.dropped
    );
    println!(
        "RESULT slow sink memory: RSS {:.0} MB at start, {:.0} MB maximum sampled during the run (+{:.0} MB), peak RSS of the process {:.0} MB -> {:.0} MB",
        mb(rss_start),
        mb(rss_max),
        mb(rss_max.saturating_sub(rss_start)),
        mb(peak_start),
        mb(peak_end)
    );
    // The fast sink keeps its 30 fps (allow a few drops on a loaded machine).
    assert!(
        fast_frames as f64 >= 0.8 * 30.0 * seconds,
        "the fast sink got {fast_frames} frames in {seconds:.1} s"
    );
    // The slow sink can take at most 10 a second.
    assert!(
        (slow_frames as f64) <= 10.5 * seconds + 2.0,
        "the slow sink got {slow_frames} frames"
    );
    assert!(slow_frames >= 1);
    assert!(stats.dropped > 50, "dropped {}", stats.dropped);
    // Bounded memory: queues of two frames per branch, not a growing backlog.
    assert!(
        rss_max.saturating_sub(rss_start) < 48 * 1_048_576,
        "RSS grew by {:.0} MB",
        mb(rss_max.saturating_sub(rss_start))
    );
    assert_eq!(fast.invalid_count() + slow.invalid_count(), 0);
    stream.stop();
}

#[test]
fn a_slow_audio_sink_drops_blocks_and_the_fast_one_keeps_its_rate() {
    let stream = CaptureStream::fake_microphone(MicrophoneConfig::default()).unwrap();
    let fast = Arc::new(Recorder::new());
    let slow = Arc::new(Recorder::slow(Duration::from_millis(100)));
    stream.add_frame_sink(fast.clone()).unwrap();
    let slow_id = stream.add_frame_sink(slow.clone()).unwrap();
    assert!(wait_until(Duration::from_secs(10), || fast.audio_count() >= 10));
    std::thread::sleep(Duration::from_secs(1));
    let (fast_start, slow_start) = (fast.audio_count(), slow.audio_count());
    std::thread::sleep(Duration::from_secs(4));
    let fast_blocks = fast.audio_count() - fast_start;
    let slow_blocks = slow.audio_count() - slow_start;
    let stats = stream.sink_stats(slow_id).unwrap();
    println!(
        "RESULT slow audio sink (4 s): fast {fast_blocks} blocks, slow {slow_blocks} blocks, slow branch dropped {}",
        stats.dropped
    );
    assert!(
        fast_blocks >= 380,
        "the fast sink got {fast_blocks} blocks in 4 s"
    );
    assert!(slow_blocks <= 45, "the slow sink got {slow_blocks} blocks");
    assert!(stats.dropped > 100);
    stream.stop();
}
