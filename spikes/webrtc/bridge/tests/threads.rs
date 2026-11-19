// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Dropping a `CaptureStream` without `stop()` leaves no thread behind. One test per process, so
//! the thread counts are the test's own.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{Recorder, proc, wait_until};
use mercury_rtc_bridge::{CameraConfig, CaptureStream, MicrophoneConfig, RemoteSource};

fn run_one_cycle() {
    let camera = CaptureStream::fake_camera(CameraConfig::new(320, 240, 30)).unwrap();
    let mic = CaptureStream::fake_microphone(MicrophoneConfig::default()).unwrap();
    let sinks: Vec<_> = (0..3).map(|_| Arc::new(Recorder::new())).collect();
    for sink in &sinks {
        camera.add_frame_sink(sink.clone()).unwrap();
    }
    mic.add_frame_sink(sinks[0].clone()).unwrap();
    let remote = RemoteSource::video(64, 48).unwrap();
    drop(remote);
    assert!(wait_until(Duration::from_secs(10), || {
        sinks.iter().all(|s| s.video_count() >= 3) && sinks[0].audio_count() >= 3
    }));
    // No stop(): the drop alone must release everything.
    drop(camera);
    drop(mic);
}

#[test]
fn dropping_streams_leaves_no_thread_behind() {
    mercury_rtc_bridge::init().unwrap();
    // The first cycle starts GStreamer's lazily created helper threads (the clock, the task pool);
    // they stay for the life of the process, so the baseline is taken after it.
    run_one_cycle();
    std::thread::sleep(Duration::from_millis(500));
    let baseline = proc::thread_count();

    let mut peak_during = baseline;
    for _ in 0..5 {
        let camera = CaptureStream::fake_camera(CameraConfig::new(320, 240, 30)).unwrap();
        let rec = Arc::new(Recorder::new());
        camera.add_frame_sink(rec.clone()).unwrap();
        assert!(wait_until(Duration::from_secs(10), || rec.video_count() >= 3));
        peak_during = peak_during.max(proc::thread_count());
        drop(camera);
    }
    // GLib keeps at most two idle pool threads for a while; everything else must be gone.
    let settled = wait_until(Duration::from_secs(20), || {
        proc::thread_count() <= baseline + 2
    });
    let after = proc::thread_count();
    println!(
        "RESULT threads: baseline {baseline}, peak while running {peak_during}, after 5 create/run/drop cycles {after} (settled: {settled})"
    );
    assert!(
        peak_during > baseline,
        "a running stream must own threads ({peak_during} vs {baseline})"
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
