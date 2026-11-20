// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Experiments with audio and video in two source buffers, which GstMSE 1.28.9 stalls on: variations of
//! the minimal prototype's append sequence, to find out what makes playback start (or stall). Prints one
//! timed line per phase; ends with a teardown that gives up after 15 s when GstMSE hangs.
//!
//! `spikes/mse/run.sh run -p mse-rig --example split_av_probe -- [--shape bare|playbin] [--plugin-mode
//! system|servo] [--video-only] [--audio-only] [--muxed] [--audio-first] [--delay-ms N] [--play-first]
//! [--chunk N] [--serial-init [--nowait]]`
//!
//! `--serial-init` appends the init segment and the first media segment to one source buffer and waits
//! for `msesrc` to report the track (`n-video`/`n-audio`) before starting the next one.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use gstreamer::prelude::ObjectExt;
use mse_rig::backend::EosError;
use mse_rig::env::{InitOptions, PluginMode};
use mse_rig::rig::{Rig, RigConfig, Shape};

const STEP: Duration = Duration::from_secs(10);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            println!("probe: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let number = |name: &str| value(name).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let shape = value("--shape")
        .and_then(|s| Shape::parse(s))
        .unwrap_or(Shape::PlayBin);
    let plugin_mode = match value("--plugin-mode").map(String::as_str) {
        Some("system") => PluginMode::SystemPath,
        _ => PluginMode::servo_default(),
    };
    let scratch = std::env::temp_dir().join(format!(
        "opencode/lanes/sp-mse/probe-{}",
        std::process::id()
    ));
    mse_rig::env::init(&InitOptions {
        prefix: None,
        plugin_mode,
        scratch_dir: scratch.clone(),
        glib_main_loop: flag("--glib-main-loop"),
    })?;
    let started = Instant::now();
    let say = |text: &str| println!("[{:>6} ms] {text}", started.elapsed().as_millis());

    let mut streams = vec![];
    if flag("--muxed") {
        streams.push(mse_segments::load("h264-aac")?);
    }
    if !flag("--muxed") && !flag("--audio-only") {
        streams.push(mse_segments::load("h264-low")?);
    }
    if !flag("--muxed") && !flag("--video-only") {
        streams.push(mse_segments::load("aac")?);
    }
    if flag("--audio-first") {
        streams.reverse();
    }
    let rig = Rig::new(RigConfig {
        shape,
        sync: false,
        ..RigConfig::default()
    })?;
    say("rig up, SourceOpen seen");
    let ids: Vec<_> = streams
        .iter()
        .map(|s| rig.backend.add_source_buffer(&s.mime))
        .collect::<Result<_, _>>()?;
    say("source buffers added");
    if flag("--play-first") {
        rig.play()?;
        say("play() called before any append");
    }
    let delay = Duration::from_millis(number("--delay-ms"));
    let chunk = number("--chunk") as usize;
    let append = |id, bytes: &[u8]| -> Result<(), Box<dyn std::error::Error>> {
        if chunk == 0 {
            rig.append_and_wait(id, bytes, STEP)?;
        } else {
            for piece in bytes.chunks(chunk) {
                rig.append_and_wait(id, piece, STEP)?;
            }
        }
        Ok(())
    };
    let tracks = || -> (u32, u32) {
        let n = |name: &str| rig.msesrc().map_or(0, |e| e.property::<u32>(name));
        (n("n-video"), n("n-audio"))
    };
    for (index, (id, stream)) in ids.iter().zip(&streams).enumerate() {
        append(*id, &stream.init.bytes)?;
        say(&format!("init appended to {id}"));
        if flag("--serial-init") {
            // Give this track its first media segment, and wait until msesrc has made its stream, before the next track starts.
            append(*id, &stream.media[0].bytes)?;
            say(&format!("first media segment appended to {id}"));
            let up = rig.wait_until(
                Duration::from_secs(if flag("--nowait") { 0 } else { 5 }),
                |_| tracks().0 + tracks().1 > index as u32,
            );
            say(&format!(
                "track {index} up: {up}; (video, audio) = {:?}",
                tracks()
            ));
        }
        std::thread::sleep(delay);
    }
    say(&format!(
        "element ready {:?}, pipeline {:?}, n pads: {}",
        rig.element_ready_state(),
        rig.pipeline_state(),
        rig.describe_pipeline()
            .iter()
            .filter(|l| l.contains("src_"))
            .count()
    ));
    for segment in (if flag("--serial-init") { 1 } else { 0 })..4 {
        for (id, stream) in ids.iter().zip(&streams) {
            append(*id, &stream.media[segment].bytes)?;
        }
        say(&format!("media segment {segment} appended"));
    }
    if !flag("--play-first") {
        rig.play()?;
        say("play() called");
    }
    rig.backend.end_of_stream(EosError::None)?;
    say("end_of_stream called");
    let eos = rig.wait_for_eos(Duration::from_secs(10));
    say(&format!(
        "eos {eos}; video {} audio {}; element ready {:?}; pipeline {:?}",
        rig.video.lock().unwrap().count,
        rig.audio.lock().unwrap().count,
        rig.element_ready_state(),
        rig.pipeline_state()
    ));
    let info = rig.shutdown()?;
    say(&format!("shutdown took {:?}", info.took));
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}
