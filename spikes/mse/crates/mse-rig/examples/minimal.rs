// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The minimal prototype: an MSE-style stream played through GStreamer's GstMSE.
//!
//! The initialisation segment and the four media segments of each fixture are appended as a page's
//! `appendBuffer` would, then `play`, `endOfStream`, and a printout of what reached the counting
//! sinks, the event order and the decoders chosen. Exits 0 only if 200 (+-2) video frames arrived.
//! By default two source buffers (`h264-low`, `aac`: split audio and video, as DASH delivers them);
//! `--muxed` uses one (`h264-aac`). With GstMSE 1.28.9 the split form stalls (see README), so the
//! example gives up after `--wait` seconds (default 10) and says what it saw.
//!
//! `spikes/mse/run.sh run -p mse-rig --example minimal -- [--shape bare|playbin] [--plugin-mode
//! system|servo] [--glib-main-loop] [--sync] [--muxed] [--wait N]`

use std::process::ExitCode;
use std::time::Duration;

use gstreamer::prelude::ObjectExt;
use mse_rig::backend::EosError;
use mse_rig::env::{InitOptions, PluginMode};
use mse_rig::frames::Dominant;
use mse_rig::rig::{Rig, RigConfig, Shape};

const STEP: Duration = Duration::from_secs(10);

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("minimal: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let shape = value("--shape")
        .and_then(|s| Shape::parse(s))
        .unwrap_or(Shape::PlayBin);
    let wait: u64 = value("--wait").and_then(|v| v.parse().ok()).unwrap_or(10);
    let plugin_mode = match value("--plugin-mode").map(String::as_str) {
        Some("servo") => PluginMode::servo_default(),
        _ => PluginMode::SystemPath,
    };
    let scratch = std::env::temp_dir().join(format!(
        "opencode/lanes/sp-mse/minimal-{}",
        std::process::id()
    ));
    let info = mse_rig::env::init(&InitOptions {
        prefix: None,
        plugin_mode,
        scratch_dir: scratch.clone(),
        glib_main_loop: flag("--glib-main-loop"),
    })?;
    println!(
        "GStreamer {} {}, plugins {}, msesrc {}, failed plugins {:?}, shape {}, glib loop {}, source buffers {}",
        info.version,
        info.arch,
        info.plugin_mode,
        info.msesrc_available,
        info.failed_plugins,
        shape.label(),
        flag("--glib-main-loop"),
        if flag("--muxed") { 1 } else { 2 }
    );

    let streams = if flag("--muxed") {
        vec![mse_segments::load("h264-aac")?]
    } else {
        vec![mse_segments::load("h264-low")?, mse_segments::load("aac")?]
    };
    let rig = Rig::new(RigConfig {
        shape,
        sync: flag("--sync"),
        ..RigConfig::default()
    })?;
    let ids: Vec<_> = streams
        .iter()
        .map(|stream| rig.backend.add_source_buffer(&stream.mime))
        .collect::<Result<_, _>>()?;
    for (id, stream) in ids.iter().zip(&streams) {
        rig.append_and_wait(*id, &stream.init.bytes, STEP)?;
    }
    for segment in 0..4 {
        for (id, stream) in ids.iter().zip(&streams) {
            rig.append_and_wait(*id, &stream.media[segment].bytes, STEP)?;
        }
    }
    rig.play()?;
    rig.backend.end_of_stream(EosError::None)?;
    let eos = rig.wait_for_eos(Duration::from_secs(wait));
    let tracks = |name: &str| {
        rig.msesrc()
            .map_or(0, |element| element.property::<u32>(name))
    };
    println!(
        "end of stream seen: {eos}; msesrc tracks (video, audio) = ({}, {}); element ready {:?}, duration {:?}; pipeline {:?}",
        tracks("n-video"),
        tracks("n-audio"),
        rig.element_ready_state(),
        rig.element_duration(),
        rig.pipeline_state()
    );
    for (name, stats) in [("video", &rig.video), ("audio", &rig.audio)] {
        let s = stats.lock().unwrap();
        println!(
            "{name}: {} buffers, {} bytes, pts {:?}..{:?}, out of order {}, max gap {:?}, flushes {}, caps changes {}",
            s.count,
            s.bytes,
            s.first_pts,
            s.last_pts,
            s.out_of_order,
            s.max_gap,
            s.flushes,
            s.caps_changes.len()
        );
        let halves: String = s
            .colour
            .iter()
            .map(|c| match c.dominant() {
                Dominant::Red => 'R',
                Dominant::Blue => 'B',
                Dominant::Other => '?',
            })
            .collect();
        if !halves.is_empty() {
            println!("{name}: colour per second of media: {halves}");
        }
    }
    println!("decoders: {:?}", rig.chosen_decoders());
    let buffered: Vec<_> = ids.iter().map(|id| rig.backend.buffered(*id)).collect();
    println!("buffered per source buffer: {buffered:?}");
    let events = rig.log.snapshot();
    let first: Vec<String> = events
        .iter()
        .take(24)
        .map(|e| format!("{:?}", e.event))
        .collect();
    println!(
        "events ({} in all), the first {}: {}",
        events.len(),
        first.len(),
        first.join(", ")
    );
    // Which thread each kind of callback came on (the kind is the event's variant name).
    let mut by_kind: std::collections::BTreeMap<String, std::collections::BTreeSet<&str>> =
        Default::default();
    for event in &events {
        let text = format!("{:?}", event.event);
        let kind = text.split('(').next().unwrap_or(&text).to_owned();
        by_kind
            .entry(kind)
            .or_default()
            .insert(event.thread.as_str());
    }
    for (kind, threads) in &by_kind {
        println!("callback {kind}: on {threads:?}");
    }

    let frames = rig.video.lock().unwrap().count;
    match rig.shutdown() {
        Ok(done) => println!("shutdown: {done:?}"),
        Err(error) => println!("shutdown: {error}"),
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok((198..=202).contains(&frames))
}
