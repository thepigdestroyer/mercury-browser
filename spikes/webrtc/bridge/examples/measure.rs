// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The measurements of the sp-rtc bridge part file.
//!
//! ```text
//! measure perf  [seconds]   CPU and per-frame latency at 1280x720@30, I420 and NV12 sources, 1 and 3 sinks
//! measure drift [seconds]   pipeline clock against std::time::Instant, and frame timestamps against it
//! ```
//!
//! Run from the crate directory after `eval "$(scripts/macos/gstreamer.sh env)"`:
//! `cargo run --release --example measure -- perf`.

#[path = "../tests/common/proc.rs"]
mod proc;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mercury_frame_sink::{ClockMap, MediaFrameSink, Micros, RawAudioFrame, RawVideoFrame};
use mercury_rtc_bridge::{
    CameraConfig, CaptureStream, MicrophoneConfig, SourceFormat, TestPattern, clock_now,
};

/// Records the delay from the frame's capture timestamp to the sink call.
#[derive(Default)]
struct LatencySink {
    frames: Mutex<Vec<i64>>,
}

impl MediaFrameSink for LatencySink {
    fn video_frame(&self, frame: RawVideoFrame<'_>) {
        let latency = clock_now().0 - frame.timestamp.0;
        self.frames.lock().unwrap().push(latency);
    }
}

fn percentile(sorted: &[i64], p: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn perf_run(format: SourceFormat, sinks: usize, seconds: u64) {
    let cfg = CameraConfig {
        width: 1280,
        height: 720,
        fps: 30,
        pattern: TestPattern::Smpte,
    };
    let stream = CaptureStream::fake_camera_with_format(cfg, format).expect("camera");
    let recorders: Vec<_> = (0..sinks)
        .map(|_| Arc::new(LatencySink::default()))
        .collect();
    for recorder in &recorders {
        stream.add_frame_sink(recorder.clone()).expect("sink");
    }
    // Warm up: the first frames include pipeline start and allocation.
    std::thread::sleep(Duration::from_secs(3));
    for recorder in &recorders {
        recorder.frames.lock().unwrap().clear();
    }
    let cpu0 = proc::cpu_seconds();
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_secs(seconds));
    let wall = t0.elapsed().as_secs_f64();
    let cpu = proc::cpu_seconds() - cpu0;
    let rss = proc::rss_bytes() as f64 / 1_048_576.0;
    stream.stop();
    let mut all = Vec::new();
    let mut per_sink_frames = Vec::new();
    for recorder in &recorders {
        let frames = recorder.frames.lock().unwrap();
        per_sink_frames.push(frames.len());
        all.extend(frames.iter().copied());
    }
    all.sort_unstable();
    let total_frames: usize = per_sink_frames.iter().sum();
    println!(
        "PERF source={:?} sinks={sinks} cpu={:.1}% of one core ({:.2} cpu-s in {wall:.1} s) frames/sink/s={:.1} latency_ms p50={:.2} p95={:.2} max={:.2} rss={rss:.0} MB cpu_per_delivered_frame_ms={:.2}",
        format,
        cpu / wall * 100.0,
        cpu,
        per_sink_frames[0] as f64 / wall,
        percentile(&all, 0.5) as f64 / 1000.0,
        percentile(&all, 0.95) as f64 / 1000.0,
        all.last().copied().unwrap_or(0) as f64 / 1000.0,
        cpu * 1000.0 / total_frames.max(1) as f64
    );
}

#[derive(Clone, Copy)]
struct Stamp {
    timestamp: Micros,
    at: Instant,
}

#[derive(Default)]
struct StampSink {
    video: Mutex<Vec<Stamp>>,
    audio: Mutex<Vec<Stamp>>,
}

impl MediaFrameSink for StampSink {
    fn video_frame(&self, frame: RawVideoFrame<'_>) {
        self.video.lock().unwrap().push(Stamp {
            timestamp: frame.timestamp,
            at: Instant::now(),
        });
    }
    fn audio_frame(&self, frame: RawAudioFrame<'_>) {
        self.audio.lock().unwrap().push(Stamp {
            timestamp: frame.timestamp,
            at: Instant::now(),
        });
    }
}

/// Least-squares slope of y against x.
fn slope(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    let (sx, sy) = points
        .iter()
        .fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let (mx, my) = (sx / n, sy / n);
    let num: f64 = points.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
    let den: f64 = points.iter().map(|(x, _)| (x - mx) * (x - mx)).sum();
    num / den
}

fn drift_run(seconds: u64) {
    let camera = CaptureStream::fake_camera(CameraConfig::new(640, 360, 30)).expect("camera");
    let mic = CaptureStream::fake_microphone(MicrophoneConfig::default()).expect("microphone");
    let (video, audio) = (
        Arc::new(StampSink::default()),
        Arc::new(StampSink::default()),
    );
    camera.add_frame_sink(video.clone()).unwrap();
    mic.add_frame_sink(audio.clone()).unwrap();

    // Anchor the pipeline clock on Instant with the tightest of several adjacent reads.
    let origin = Instant::now();
    let wall = |at: Instant| at.duration_since(origin).as_micros() as i64;
    let adjacent = || {
        let mut best = (i64::MAX, Micros(0), 0i64);
        for _ in 0..16 {
            let before = wall(Instant::now());
            let producer = camera.producer_now();
            let after = wall(Instant::now());
            if after - before < best.0 {
                best = (after - before, producer, (before + after) / 2);
            }
        }
        best
    };
    let (gap0, p0, w0) = adjacent();
    let map = ClockMap::new(p0, Micros(w0));
    println!(
        "DRIFT anchor: producer {} us = wall {w0} us (+-{} us)",
        p0.0,
        gap0 / 2
    );

    let started = Instant::now();
    let mut next_report = 10;
    while started.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_millis(250));
        let elapsed = started.elapsed().as_secs();
        if elapsed >= next_report {
            next_report += 10;
            let (gap, p, w) = adjacent();
            let skew = map.to_consumer(p).0 - w;
            let recent = |stamps: &Mutex<Vec<Stamp>>| {
                let stamps = stamps.lock().unwrap();
                let tail = &stamps[stamps.len().saturating_sub(100)..];
                let lags: Vec<i64> = tail
                    .iter()
                    .map(|s| wall(s.at) - map.to_consumer(s.timestamp).0)
                    .collect();
                lags.iter().sum::<i64>() / lags.len().max(1) as i64
            };
            println!(
                "DRIFT t={elapsed:>3}s clock-vs-Instant skew {skew:>5} us (+-{} us), mean capture-to-sink lag: video {} us, audio {} us",
                gap / 2,
                recent(&video.video),
                recent(&audio.audio),
            );
        }
    }
    let (gap, p, w) = adjacent();
    let skew = map.to_consumer(p).0 - w;
    camera.stop();
    mic.stop();
    println!(
        "DRIFT final: pipeline clock minus Instant over {seconds} s: {skew} us (+-{} us read gap), i.e. {:.3} ppm",
        gap / 2,
        skew as f64 / (seconds as f64 * 1e6) * 1e6
    );
    for (name, stamps) in [("video", &video.video), ("audio", &audio.audio)] {
        let stamps = stamps.lock().unwrap();
        let points: Vec<(f64, f64)> = stamps
            .iter()
            .map(|s| {
                (
                    wall(s.at) as f64 / 1e6,
                    (wall(s.at) - map.to_consumer(s.timestamp).0) as f64,
                )
            })
            .collect();
        let lags: Vec<f64> = points.iter().map(|p| p.1).collect();
        let first = lags.iter().take(100).sum::<f64>() / 100.0;
        let last = lags.iter().rev().take(100).sum::<f64>() / 100.0;
        let mut monotonic = true;
        for pair in stamps.windows(2) {
            monotonic &= pair[0].timestamp <= pair[1].timestamp;
        }
        println!(
            "DRIFT {name}: {} frames, lag of the first 100 {first:.0} us, of the last 100 {last:.0} us, slope {:.3} us/s ({:.3} ppm), timestamps monotonic: {monotonic}",
            stamps.len(),
            slope(&points),
            slope(&points)
        );
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seconds = |args: &mut dyn Iterator<Item = String>, default| {
        args.next().and_then(|s| s.parse().ok()).unwrap_or(default)
    };
    match args.next().as_deref() {
        Some("perf") => {
            let seconds = seconds(&mut args, 10);
            for (format, sinks) in [
                (SourceFormat::I420, 1),
                (SourceFormat::I420, 3),
                (SourceFormat::Nv12, 1),
                (SourceFormat::Nv12, 3),
            ] {
                perf_run(format, sinks, seconds);
            }
        }
        Some("drift") => drift_run(seconds(&mut args, 180)),
        _ => eprintln!("usage: measure perf [seconds] | measure drift [seconds]"),
    }
}
