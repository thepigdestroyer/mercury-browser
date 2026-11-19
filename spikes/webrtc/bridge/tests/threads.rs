// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Dropping a `CaptureStream` without `stop()` leaves no thread behind. One test per process, so
//! the thread counts are the test's own.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{Recorder, proc, wait_until};
use mercury_rtc_bridge::{CameraConfig, CaptureStream, MicrophoneConfig, RemoteSource};

/// Three cameras with three sinks each, a microphone and two remote sources: plenty of threads.
fn run_busy_cycle() -> usize {
    let cameras: Vec<_> = (0..3)
        .map(|_| CaptureStream::fake_camera(CameraConfig::new(320, 240, 30)).unwrap())
        .collect();
    let mic = CaptureStream::fake_microphone(MicrophoneConfig::default()).unwrap();
    let sinks: Vec<_> = (0..9).map(|_| Arc::new(Recorder::new())).collect();
    for (index, sink) in sinks.iter().enumerate() {
        cameras[index / 3].add_frame_sink(sink.clone()).unwrap();
    }
    let audio = Arc::new(Recorder::new());
    mic.add_frame_sink(audio.clone()).unwrap();
    let remote_video = RemoteSource::video(64, 48).unwrap();
    let remote_audio = RemoteSource::audio(2).unwrap();
    let ready = wait_until(Duration::from_secs(10), || {
        sinks.iter().all(|s| s.video_count() >= 3) && audio.audio_count() >= 3
    });
    if !ready {
        let mut report = String::new();
        for (index, camera) in cameras.iter().enumerate() {
            report += &format!("camera {index}:\n{}", common::dump_stream(camera));
        }
        let invalid: Vec<_> = sinks.iter().map(|s| s.invalid_count()).collect();
        panic!(
            "no frames: video counts {:?}, invalid {invalid:?}, audio {}, microphone error {:?}\n{report}",
            sinks.iter().map(|s| s.video_count()).collect::<Vec<_>>(),
            audio.audio_count(),
            mic.take_error()
        );
    }
    let running = proc::thread_count();
    // No stop(): the drop alone must release everything.
    drop(cameras);
    drop(mic);
    drop(remote_video);
    drop(remote_audio);
    running
}

#[test]
fn dropping_streams_leaves_no_thread_behind() {
    mercury_rtc_bridge::init().unwrap();
    let before_anything = proc::thread_count();
    // The first cycle starts GStreamer's lazily created helper threads (the task pool keeps a few
    // idle ones, the clock thread stays); the baseline is taken after it settles.
    let first_peak = run_busy_cycle();
    std::thread::sleep(Duration::from_millis(500));
    let baseline = proc::thread_count();

    let mut peak = 0;
    for _ in 0..4 {
        peak = peak.max(run_busy_cycle());
    }
    // GLib keeps at most two idle pool threads for a while; everything else must be gone.
    let settled = wait_until(Duration::from_secs(20), || {
        proc::thread_count() <= baseline + 2
    });
    let after = proc::thread_count();
    println!(
        "RESULT threads: {before_anything} before any stream, {first_peak} with 3 cameras x 3 sinks + microphone + 2 remote sources running, baseline after the first cycle {baseline}, peak in later cycles {peak}, after 4 more cycles {after} (settled: {settled})"
    );
    assert!(
        first_peak >= before_anything + 10,
        "running streams must own threads ({first_peak} vs {before_anything})"
    );
    assert!(
        settled,
        "threads left behind: {after} vs baseline {baseline}"
    );

    // A stream that is stopped and then dropped, and one never started, leave nothing either.
    let stopped = CaptureStream::fake_camera(CameraConfig::new(320, 240, 30)).unwrap();
    let rec = Arc::new(Recorder::new());
    stopped.add_frame_sink(rec.clone()).unwrap();
    assert!(wait_until(Duration::from_secs(10), || rec.video_count() >= 3));
    stopped.stop();
    drop(stopped);
    drop(CaptureStream::fake_camera(CameraConfig::new(320, 240, 30)).unwrap());
    assert!(wait_until(Duration::from_secs(20), || {
        proc::thread_count() <= baseline + 2
    }));
}
