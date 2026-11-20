// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A small reproduction of the GstMSE deadlock found by T01 and T04 (see `sdd/sp-mse-logs/stress-findings.md`):
//! a rig with a video and an audio source buffer, both initialisation segments appended, then torn down.
//! It builds a rig, appends, plays for 0.3 s and shuts down, N times, and says which stage a hang was in.
//!
//! ```text
//! spikes/mse/run.sh run --release -p mse-stress --example deadlock_repro -- [options]
//!   --shape bare|playbin     pipeline shape (default playbin)
//!   --variant both|video|audio
//!   --iterations N           default 5
//!   --delay-ms N             sleep this long after the initialisation segments
//!   --no-wait-init           append the two initialisation segments without waiting for UpdateEnd
//!   --play-first             call play() right after Rig::new, before any append
//!   --no-play                never call play()
//!   --skip-media             append the initialisation segments only
//!   --staged                 add and feed the video buffer alone until the element has metadata, then the audio one
//!   --pre-null-ms N          sleep N ms before the shutdown (does a pause before NULL avoid the teardown crash?)
//!   --post-null-ms N         sleep N ms after the shutdown returned (does a pause after it avoid the crash?)
//!   --pre-null-ms N          sleep N ms before the shutdown (does a pause before NULL avoid the teardown crash?)
//!   --post-null-ms N         sleep N ms after the shutdown returned (does a pause after it avoid the crash?)
//!   --verbose                print the pipeline and element state after every step
//! ```
//!
//! Exit status 0: every iteration finished; 3: a stage did not finish within 10 s (printed).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mse_rig::env::{InitOptions, PluginMode, init};
use mse_rig::rig::{Rig, RigConfig, Shape};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let flag = |name: &str| args.iter().any(|a| a == name);
    let shape = value("--shape")
        .and_then(|s| Shape::parse(&s))
        .unwrap_or(Shape::PlayBin);
    let variant = value("--variant").unwrap_or_else(|| "both".to_owned());
    let iterations: usize = value("--iterations")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let delay = Duration::from_millis(
        value("--delay-ms")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    );
    let pre_null = Duration::from_millis(
        value("--pre-null-ms")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    );
    let post_null = Duration::from_millis(
        value("--post-null-ms")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    );
    let scratch = std::env::temp_dir().join("opencode/lanes/sp-mse/deadlock-repro");
    let info = match init(&InitOptions {
        prefix: None,
        plugin_mode: PluginMode::SystemPath,
        scratch_dir: scratch.clone(),
        glib_main_loop: false,
    }) {
        Ok(info) => info,
        Err(error) => {
            eprintln!("GStreamer set-up failed: {error}");
            std::process::exit(2);
        }
    };
    println!(
        "GStreamer {} on {}; shape {}; variant {variant}",
        info.version,
        info.arch,
        shape.label()
    );

    let video = mse_segments::load("h264-low").expect("fixture h264-low");
    let audio = mse_segments::load("aac").expect("fixture aac");

    // The watchdog: a stage that lasts more than 10 s is reported and ends the process.
    let stage = Arc::new(Mutex::new(("start".to_owned(), Instant::now())));
    let iteration = Arc::new(AtomicUsize::new(0));
    {
        let (stage, iteration) = (Arc::clone(&stage), Arc::clone(&iteration));
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let (name, since) = stage.lock().unwrap().clone();
                if since.elapsed() > Duration::from_secs(10) {
                    println!(
                        "HANG: iteration {} stuck in `{name}` for 10 s",
                        iteration.load(Ordering::SeqCst)
                    );
                    std::process::exit(3);
                }
            }
        });
    }
    let set = |name: &str| *stage.lock().unwrap() = (name.to_owned(), Instant::now());
    let show = |rig: &Rig, what: &str| {
        if flag("--verbose") {
            println!(
                "  after {what}: pipeline {:?}, element ready state {:?}, duration {:?}, sourcebuffers {}",
                rig.pipeline_state(),
                rig.element_ready_state(),
                rig.element_duration(),
                rig.backend.source_buffer_count()
            );
        }
    };

    for n in 0..iterations {
        iteration.store(n, Ordering::SeqCst);
        set("Rig::new");
        let rig = Rig::new(RigConfig {
            shape,
            sync: false,
            ..RigConfig::default()
        })
        .expect("Rig::new");
        if flag("--staged") {
            // One source buffer at a time: video to HaveMetadata with the pipeline playing, then audio.
            set("staged: video");
            let timeout = Duration::from_secs(5);
            let vid = rig
                .backend
                .add_source_buffer(&video.mime)
                .expect("add video");
            rig.append_and_wait(vid, &video.init.bytes, timeout)
                .expect("video init");
            rig.append_and_wait(vid, &video.media[0].bytes, timeout)
                .expect("video media");
            let _ = rig.play();
            let metadata = rig.wait_until(Duration::from_secs(3), |rig| {
                rig.element_ready_state() >= mse_rig::events::ElementReadyState::HaveMetadata
            });
            show(&rig, &format!("video alone (metadata reached: {metadata})"));
            set("staged: audio");
            let aud = rig
                .backend
                .add_source_buffer(&audio.mime)
                .expect("add audio");
            rig.append_and_wait(aud, &audio.init.bytes, timeout)
                .expect("audio init");
            rig.append_and_wait(aud, &audio.media[0].bytes, timeout)
                .expect("audio media");
            show(&rig, "audio added");
            set("wait for frames");
            rig.wait_until(Duration::from_millis(500), |rig| {
                rig.video.lock().unwrap().count >= 3
            });
            let frames = rig.video.lock().unwrap().count;
            println!(
                "iteration {n}: staged: {frames} video frames, {} audio buffers",
                rig.audio.lock().unwrap().count
            );
            set("shutdown");
            let started = Instant::now();
            let _ = rig.shutdown();
            println!(
                "iteration {n}: shutdown took {} ms",
                started.elapsed().as_millis()
            );
            continue;
        }
        let want_video = variant != "audio";
        let want_audio = variant != "video";
        let vid = want_video.then(|| {
            rig.backend
                .add_source_buffer(&video.mime)
                .expect("add video")
        });
        let aud = want_audio.then(|| {
            rig.backend
                .add_source_buffer(&audio.mime)
                .expect("add audio")
        });
        if flag("--play-first") {
            set("play (first)");
            let _ = rig.play();
        }
        set("append init segments");
        let timeout = Duration::from_secs(5);
        if flag("--no-wait-init") {
            if let Some(id) = vid {
                let _ = rig.backend.append(id, &video.init.bytes);
            }
            if let Some(id) = aud {
                let _ = rig.backend.append(id, &audio.init.bytes);
            }
            std::thread::sleep(Duration::from_millis(200));
        } else {
            if let Some(id) = vid {
                rig.append_and_wait(id, &video.init.bytes, timeout)
                    .expect("video init");
            }
            if let Some(id) = aud {
                rig.append_and_wait(id, &audio.init.bytes, timeout)
                    .expect("audio init");
            }
        }
        show(&rig, "the initialisation segments");
        std::thread::sleep(delay);
        show(&rig, "the delay");
        if !flag("--skip-media") {
            set("append first media segments");
            if let Some(id) = vid {
                rig.append_and_wait(id, &video.media[0].bytes, timeout)
                    .expect("video media");
                show(&rig, "the first video media segment");
            }
            if let Some(id) = aud {
                rig.append_and_wait(id, &audio.media[0].bytes, timeout)
                    .expect("audio media");
                show(&rig, "the first audio media segment");
            }
        }
        if !flag("--no-play") && !flag("--play-first") {
            set("play");
            let _ = rig.play();
        }
        set("wait for frames");
        rig.wait_until(Duration::from_millis(300), |rig| {
            rig.video.lock().unwrap().count >= 3
        });
        let frames = rig.video.lock().unwrap().count;
        println!(
            "iteration {n}: before shutdown: {frames} video frames, {} audio buffers, pipeline {:?}, element ready state {:?}, duration {:?}",
            rig.audio.lock().unwrap().count,
            rig.pipeline_state(),
            rig.element_ready_state(),
            rig.element_duration()
        );
        std::thread::sleep(pre_null);
        set("shutdown");
        let started = Instant::now();
        let _ = rig.shutdown();
        println!(
            "iteration {n}: {frames} video frames, shutdown took {} ms",
            started.elapsed().as_millis()
        );
        std::thread::sleep(post_null);
    }
    set("done");
    println!("all {iterations} iterations finished");
    let _ = std::fs::remove_dir_all(&scratch);
}
