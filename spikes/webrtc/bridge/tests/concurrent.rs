// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Several streams at once, each with several sinks added one after the other: every sink of every
//! stream must receive frames.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{Recorder, dump_stream, wait_until};
use mercury_rtc_bridge::{CameraConfig, CaptureStream, MicrophoneConfig};

/// One round: `cameras` cameras with `per_camera` sinks each, plus a microphone with two sinks.
/// Returns a description of everything that did not deliver (empty when all is well).
fn round(cameras: usize, per_camera: usize) -> String {
    let streams: Vec<_> = (0..cameras)
        .map(|_| CaptureStream::fake_camera(CameraConfig::new(320, 240, 30)).unwrap())
        .collect();
    let mic = CaptureStream::fake_microphone(MicrophoneConfig::default()).unwrap();
    let mut sinks = Vec::new();
    for stream in &streams {
        for _ in 0..per_camera {
            let sink = Arc::new(Recorder::new());
            stream.add_frame_sink(sink.clone()).unwrap();
            sinks.push(sink);
        }
    }
    let audio: Vec<_> = (0..2).map(|_| Arc::new(Recorder::new())).collect();
    for sink in &audio {
        mic.add_frame_sink(sink.clone()).unwrap();
    }
    let ready = wait_until(Duration::from_secs(5), || {
        sinks.iter().all(|s| s.video_count() >= 5) && audio.iter().all(|s| s.audio_count() >= 5)
    });
    if ready {
        return String::new();
    }
    let mut report = format!(
        "video counts {:?}, invalid {:?}, audio counts {:?}\n",
        sinks.iter().map(|s| s.video_count()).collect::<Vec<_>>(),
        sinks.iter().map(|s| s.invalid_count()).collect::<Vec<_>>(),
        audio.iter().map(|s| s.audio_count()).collect::<Vec<_>>()
    );
    for (index, stream) in streams.iter().enumerate() {
        let counts: Vec<_> = sinks[index * per_camera..(index + 1) * per_camera]
            .iter()
            .map(|s| s.video_count())
            .collect();
        if counts.iter().any(|c| *c < 5) {
            report += &format!("camera {index} stalled:\n{}", dump_stream(stream));
        }
    }
    report
}

#[test]
fn three_cameras_with_three_sinks_each_all_deliver() {
    let mut failures = Vec::new();
    for attempt in 0..6 {
        let report = round(3, 3);
        println!(
            "RESULT concurrent attempt {attempt}: {}",
            if report.is_empty() {
                "all sinks delivered"
            } else {
                "STALLED"
            }
        );
        if !report.is_empty() {
            failures.push(format!("attempt {attempt}:\n{report}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn two_cameras_with_one_sink_each_all_deliver() {
    for attempt in 0..6 {
        let report = round(2, 1);
        assert!(report.is_empty(), "attempt {attempt}:\n{report}");
    }
}
