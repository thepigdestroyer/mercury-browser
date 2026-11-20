// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What GstMSE does at the edges of the API (the "API fit" questions of `backend.rs`): negative
//! `timestampOffset`, `+Infinity` durations, append windows and removals, the error classes, and when
//! `QuotaExceeded` is raised. One `Q..` line per observation, so the output can be pasted into notes.
//! Uses single source buffers (video only or muxed), which GstMSE 1.28.9 plays; see the README.
//!
//! `spikes/mse/run.sh run -p mse-rig --example api_probe -- [offset|duration|window|remove|errors|quota]...`
//! (no argument: everything but `quota`, which appends up to 1 GiB and takes a while).

use std::time::Duration;

use mse_rig::RigError;
use mse_rig::backend::{AppendMode, EosError, MseError};
use mse_rig::env::{InitOptions, PluginMode};
use mse_rig::rig::{Rig, RigConfig, Shape};
use mse_segments::SegmentedStream;

const STEP: Duration = Duration::from_secs(10);

type R<T> = Result<T, Box<dyn std::error::Error>>;

fn secs(ns: u64) -> String {
    format!("{:.3}s", ns as f64 / 1e9)
}

fn show_ranges(ranges: &Result<Vec<mse_rig::backend::TimeRange>, MseError>) -> String {
    match ranges {
        Ok(ranges) => format!(
            "[{}]",
            ranges
                .iter()
                .map(|r| format!("{}..{}", secs(r.start.nseconds()), secs(r.end.nseconds())))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Err(error) => format!("error {error}"),
    }
}

fn rig_for(stream: &SegmentedStream) -> R<(Rig, usize)> {
    let rig = Rig::new(RigConfig {
        shape: Shape::PlayBin,
        sync: false,
        ..RigConfig::default()
    })?;
    let id = rig.backend.add_source_buffer(&stream.mime)?;
    Ok((rig, id))
}

fn append_all(rig: &Rig, id: usize, stream: &SegmentedStream, segments: usize) -> R<()> {
    rig.append_and_wait(id, &stream.init.bytes, STEP)?;
    for segment in stream.media.iter().take(segments) {
        rig.append_and_wait(id, &segment.bytes, STEP)?;
    }
    Ok(())
}

fn finish(rig: Rig) {
    match rig.shutdown() {
        Ok(_) => {}
        Err(error) => println!("   (teardown: {error})"),
    }
}

fn offsets(video: &SegmentedStream) -> R<()> {
    let (rig, id) = rig_for(video)?;
    let b = &rig.backend;
    println!(
        "Q1  timestamp_offset at start: {:?}",
        b.timestamp_offset(id)
    );
    println!(
        "Q2  set_timestamp_offset(-1 s) through the signed API: {:?}",
        b.set_timestamp_offset(id, -1_000_000_000)
    );
    let wrapped = (-1_000_000_000i64) as u64;
    println!(
        "Q3  the same value as an unsigned GstClockTime ({wrapped}) given to GstMSE: {:?}; read back: {:?}",
        b.set_timestamp_offset_raw(id, Some(gstreamer::ClockTime::from_nseconds(wrapped))),
        b.timestamp_offset(id).map(|t| t.map(|t| t.nseconds()))
    );
    println!(
        "Q4  set GST_CLOCK_TIME_NONE as offset: {:?}",
        b.set_timestamp_offset_raw(id, None)
    );
    println!(
        "Q5  offset +8 s: {:?}",
        b.set_timestamp_offset(id, 8_000_000_000)
    );
    append_all(&rig, id, video, 2)?;
    println!(
        "Q6  after init + 2 segments (4 s of media) at +8 s: buffered {} at once, {} after 500 ms",
        show_ranges(&b.buffered(id)),
        {
            std::thread::sleep(Duration::from_millis(500));
            show_ranges(&b.buffered(id))
        }
    );
    println!(
        "Q7  offset change after data (not updating): {:?}",
        b.set_timestamp_offset(id, 2_000_000_000)
    );
    finish(rig);
    Ok(())
}

fn durations(video: &SegmentedStream) -> R<()> {
    let (rig, id) = rig_for(video)?;
    let b = &rig.backend;
    println!(
        "Q10 duration before any init segment: {:?}; element: {:?}",
        b.duration(),
        rig.element_duration()
    );
    rig.append_and_wait(id, &video.init.bytes, STEP)?;
    rig.append_and_wait(id, &video.media[0].bytes, STEP)?;
    std::thread::sleep(Duration::from_millis(300));
    println!(
        "Q11 duration after init + first segment: {:?}; element: {:?}",
        b.duration(),
        rig.element_duration()
    );
    for segment in video.media.iter().skip(1) {
        rig.append_and_wait(id, &segment.bytes, STEP)?;
    }
    println!(
        "Q12 after all 8 s: duration {:?}, buffered {}",
        b.duration(),
        show_ranges(&b.buffered(id))
    );
    println!(
        "Q13 set_duration(None) (= +Infinity / unknown): {:?}; then duration {:?}; element {:?}",
        b.set_duration(None),
        b.duration(),
        rig.element_duration()
    );
    println!(
        "Q14 set_duration(100 s): {:?}; then duration {:?}",
        b.set_duration(Some(gstreamer::ClockTime::from_seconds(100))),
        b.duration()
    );
    println!(
        "Q15 set_duration(1 s) below the buffered end (spec: InvalidStateError): {:?}; then duration {:?}",
        b.set_duration(Some(gstreamer::ClockTime::from_seconds(1))),
        b.duration()
    );
    println!(
        "Q16 set_duration(8 s): {:?}; then duration {:?}",
        b.set_duration(Some(gstreamer::ClockTime::from_seconds(8))),
        b.duration()
    );
    finish(rig);
    Ok(())
}

fn windows(video: &SegmentedStream) -> R<()> {
    use gstreamer::ClockTime as T;
    let (rig, id) = rig_for(video)?;
    let b = &rig.backend;
    println!(
        "Q20 append window at start: {:?}",
        b.append_window(id)
            .map(|(s, e)| (s.map(|t| t.nseconds()), e.map(|t| t.nseconds())))
    );
    println!(
        "Q21 set_append_window(2 s, +Infinity=None): {:?}; read back {:?}",
        b.set_append_window(id, T::from_seconds(2), None),
        b.append_window(id)
            .map(|(s, e)| (s.map(|t| t.nseconds()), e.map(|t| t.nseconds())))
    );
    println!(
        "Q22 set_append_window(2 s, 6 s): {:?}",
        b.set_append_window(id, T::from_seconds(2), Some(T::from_seconds(6)))
    );
    println!(
        "Q23 set_append_window(7 s, 6 s) start after end: {:?}",
        b.set_append_window(id, T::from_seconds(7), Some(T::from_seconds(6)))
    );
    println!(
        "Q24 set_append_window(0, 6 s) after the failed one: {:?}; read back {:?}",
        b.set_append_window(id, T::ZERO, Some(T::from_seconds(6))),
        b.append_window(id)
            .map(|(s, e)| (s.map(|t| t.nseconds()), e.map(|t| t.nseconds())))
    );
    append_all(&rig, id, video, 4)?;
    println!(
        "Q25 window 2..6 s, all four segments appended: buffered {}",
        show_ranges(&b.buffered(id))
    );
    println!(
        "Q26 append mode: {:?}; set Sequence: {:?}; now {:?}",
        b.append_mode(id),
        b.set_append_mode(id, AppendMode::Sequence),
        b.append_mode(id)
    );
    let show = |b: &mse_rig::backend::MseBackend| {
        b.append_window(id)
            .map(|(s, e)| (s.map(|t| t.nseconds()), e.map(|t| t.nseconds())))
    };
    println!(
        "Q27 raw order: end first. set_append_window(0, 6 s): {:?}; then (2 s, 6 s): {:?}; window {:?}",
        b.set_append_window(id, T::ZERO, Some(T::from_seconds(6))),
        b.set_append_window(id, T::from_seconds(2), Some(T::from_seconds(6))),
        show(b)
    );
    println!(
        "Q28 back to +Infinity end with start 2 s: {:?}; window {:?}",
        b.set_append_window(id, T::from_seconds(2), None),
        show(b)
    );
    println!(
        "Q29 end = u64::MAX-1 with start 2 s: {:?}; window {:?}",
        b.set_append_window(id, T::from_seconds(2), Some(T::from_nseconds(u64::MAX - 1))),
        show(b)
    );
    finish(rig);
    Ok(())
}

fn removals(video: &SegmentedStream) -> R<()> {
    use gstreamer::ClockTime as T;
    let (rig, id) = rig_for(video)?;
    let b = &rig.backend;
    append_all(&rig, id, video, 4)?;
    println!(
        "Q30 after everything: buffered {}",
        show_ranges(&b.buffered(id))
    );
    println!(
        "Q31 remove(0, 4 s): {:?}",
        b.remove(id, T::ZERO, Some(T::from_seconds(4)))
    );
    std::thread::sleep(Duration::from_millis(300));
    println!(
        "Q32 buffered 300 ms after it: {}; after 1.5 s: {}",
        show_ranges(&b.buffered(id)),
        {
            std::thread::sleep(Duration::from_millis(1200));
            show_ranges(&b.buffered(id))
        }
    );
    println!(
        "Q33 remove(6 s, +Infinity=None): {:?}",
        b.remove(id, T::from_seconds(6), None)
    );
    std::thread::sleep(Duration::from_millis(300));
    println!("Q34 buffered after it: {}", show_ranges(&b.buffered(id)));
    println!(
        "Q35 remove(5 s, 1 s) start after end: {:?}",
        b.remove(id, T::from_seconds(5), Some(T::from_seconds(1)))
    );
    let appended = b.append(id, &video.media[0].bytes);
    println!(
        "Q36 append then remove at once (updating = {}): {:?}",
        b.updating(id),
        b.remove(id, T::ZERO, None)
    );
    println!("    (the append itself returned {appended:?})");
    finish(rig);
    Ok(())
}

fn errors(video: &SegmentedStream) -> R<()> {
    let (rig, id) = rig_for(video)?;
    let b = &rig.backend;
    for mime in [
        "video/foo",
        "",
        "video/mp4",
        "video/mp4; codecs=\"hev1.1.6.L93.B0\"",
        "video/mp4; codecs=\"avc1.42C00C\", codecs=\"avc1.42C00C\"",
        "audio/mp4; codecs=\"avc1.42C00C\"",
    ] {
        println!(
            "Q40 add_source_buffer({mime:?}): {:?}",
            b.add_source_buffer(mime)
        );
    }
    println!("Q41 append to an unknown id: {:?}", b.append(99, b"x"));
    println!(
        "Q42 append garbage to a source buffer: {:?}",
        rig.append_and_wait(id, &[0x13; 2048], STEP)
            .map(|t| t.to_update_end)
    );
    println!(
        "    log errors: {} Error events, updating = {}",
        rig.log
            .count(|e| matches!(e, mse_rig::events::MseEvent::Error(_))),
        b.updating(id)
    );
    println!("Q43 empty append: {:?}", b.append(id, &[]));
    println!(
        "Q44 end_of_stream(None): {:?}; again: {:?}; ready state {:?}",
        b.end_of_stream(EosError::None),
        b.end_of_stream(EosError::None),
        b.ready_state()
    );
    println!(
        "Q45 append after end_of_stream: {:?}; ready state {:?}",
        rig.append_and_wait(id, &video.init.bytes, STEP).map(|_| ()),
        b.ready_state()
    );
    println!(
        "Q46 remove_source_buffer: {:?}; again: {:?}",
        b.remove_source_buffer(id),
        b.remove_source_buffer(id)
    );
    println!(
        "Q47 append to the removed source buffer: {:?}; updating {}; buffered {}",
        b.append(id, &video.init.bytes),
        b.updating(id),
        show_ranges(&b.buffered(id))
    );
    println!(
        "Q48 source buffer count {}; change_type to video/webm on a fresh one: {:?}",
        b.source_buffer_count(),
        b.add_source_buffer(&video.mime)
            .and_then(|n| b.change_type(n, "video/webm; codecs=\"vp9\""))
    );
    finish(rig);
    Ok(())
}

fn quota(video: &SegmentedStream) -> R<()> {
    let (rig, id) = rig_for(video)?;
    let b = &rig.backend;
    let tiled = mse_segments::tile(video, 4000)?;
    rig.append_and_wait(id, &tiled.init.bytes, STEP)?;
    let (mut total, mut count, mut last_ok) = (0u64, 0u64, 0u64);
    let started = std::time::Instant::now();
    for segment in &tiled.media {
        match rig.append_and_wait(id, &segment.bytes, STEP) {
            Ok(_) => {
                total += segment.bytes.len() as u64;
                count += 1;
                last_ok = segment.start_ms.unwrap_or(0) + segment.duration_ms.unwrap_or(0);
            }
            Err(RigError::Mse(error)) => {
                println!(
                    "Q50 append #{count} refused after {} MiB / {} s of media: {error:?}",
                    total >> 20,
                    last_ok / 1000
                );
                println!(
                    "Q51 buffered then: {}; updating {}",
                    show_ranges(&b.buffered(id)),
                    b.updating(id)
                );
                let first = b.remove(
                    id,
                    gstreamer::ClockTime::ZERO,
                    Some(gstreamer::ClockTime::from_seconds(600)),
                );
                println!(
                    "Q52 remove(0, 10 min): {first:?}; buffered {}",
                    show_ranges(&b.buffered(id))
                );
                println!(
                    "Q53 append again: {:?}",
                    rig.append_and_wait(id, &segment.bytes, STEP).map(|_| ())
                );
                finish(rig);
                return Ok(());
            }
            Err(other) => {
                println!(
                    "Q50 append #{count} failed after {} MiB / {} s of media: {other}",
                    total >> 20,
                    last_ok / 1000
                );
                finish(rig);
                return Ok(());
            }
        }
        if total >= 1 << 30 || started.elapsed() > Duration::from_secs(120) {
            break;
        }
    }
    println!(
        "Q50 no refusal after {count} appends, {} MiB, {} s of media, in {:?}; buffered {}",
        total >> 20,
        last_ok / 1000,
        started.elapsed(),
        show_ranges(&b.buffered(id))
    );
    finish(rig);
    Ok(())
}

/// How long after the last `UpdateEnd` does `buffered` reach the end of what was appended?
fn lag(video: &SegmentedStream) -> R<()> {
    let (rig, id) = rig_for(video)?;
    let b = &rig.backend;
    rig.append_and_wait(id, &video.init.bytes, STEP)?;
    let mut previous = 0;
    for (index, segment) in video.media.iter().enumerate() {
        rig.append_and_wait(id, &segment.bytes, STEP)?;
        let wanted = segment.start_ms.unwrap_or(0) + segment.duration_ms.unwrap_or(0);
        let started = std::time::Instant::now();
        let reached = rig.wait_until(Duration::from_secs(3), |rig| {
            rig.backend
                .buffered(id)
                .is_ok_and(|r| r.last().is_some_and(|r| r.end.mseconds() + 50 >= wanted))
        });
        println!(
            "Q60 segment {index} (media time ..{wanted} ms): UpdateEnd seen; buffered {} right then; reached the segment end after {:?} (within 3 s: {reached}); updating {}",
            show_ranges(&b.buffered(id)),
            started.elapsed(),
            b.updating(id)
        );
        previous = wanted;
    }
    println!(
        "Q61 last buffered end target was {previous} ms; duration {:?}; element ready {:?}",
        b.duration(),
        rig.element_ready_state()
    );
    finish(rig);
    Ok(())
}

/// Which property setter, called before the first append, makes `buffered` come back empty?
fn bisect(video: &SegmentedStream) -> R<()> {
    use gstreamer::ClockTime as T;
    type Setup = fn(&mse_rig::backend::MseBackend, usize);
    let cases: [(&str, Setup); 7] = [
        ("nothing", |_, _| {}),
        ("getter append_window only", |b, id| {
            let _ = b.append_window(id);
        }),
        ("failed set_append_window(2 s, 6 s)", |b, id| {
            let _ = b.set_append_window(id, T::from_seconds(2), Some(T::from_seconds(6)));
        }),
        ("set_append_mode(Sequence)", |b, id| {
            let _ = b.set_append_mode(id, AppendMode::Sequence);
        }),
        ("set_append_mode(Segments)", |b, id| {
            let _ = b.set_append_mode(id, AppendMode::Segments);
        }),
        ("set_timestamp_offset(0)", |b, id| {
            let _ = b.set_timestamp_offset(id, 0);
        }),
        ("set_timestamp_offset(+8 s)", |b, id| {
            let _ = b.set_timestamp_offset(id, 8_000_000_000);
        }),
    ];
    for (label, setup) in cases {
        let (rig, id) = rig_for(video)?;
        setup(&rig.backend, id);
        append_all(&rig, id, video, 4)?;
        let ranges = rig.backend.buffered(id);
        std::thread::sleep(Duration::from_millis(500));
        println!(
            "Q80 after {label}: buffered right after the 4th UpdateEnd {}, 500 ms later {}",
            show_ranges(&ranges),
            show_ranges(&rig.backend.buffered(id))
        );
        finish(rig);
    }
    Ok(())
}

fn types() -> R<()> {
    for mime in [
        "video/foo",
        "",
        "video/mp4",
        "video/mp4; codecs=\"hev1.1.6.L93.B0\"",
        "audio/mp4; codecs=\"avc1.42C00C\"",
        "video/mp4; codecs=\"avc1.42C00C\"",
        "audio/mp4; codecs=\"mp4a.40.2\"",
        "video/webm; codecs=\"vp9\"",
        "video/webm; codecs=\"av01.0.05M.08\"",
        "video/mp4; codecs=\"nonsense\"",
    ] {
        println!(
            "Q70 is_type_supported({mime:?}) = {}",
            mse_rig::backend::MseBackend::is_type_supported(mime)
        );
    }
    Ok(())
}

fn main() -> R<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wanted = |name: &str| {
        if args.is_empty() {
            name != "quota"
        } else {
            args.iter().any(|a| a == name)
        }
    };
    let scratch = std::env::temp_dir().join(format!(
        "opencode/lanes/sp-mse/api-probe-{}",
        std::process::id()
    ));
    let info = mse_rig::env::init(&InitOptions {
        prefix: None,
        plugin_mode: PluginMode::servo_default(),
        scratch_dir: scratch.clone(),
        glib_main_loop: false,
    })?;
    println!(
        "GStreamer {} {}, plugins {}",
        info.version, info.arch, info.plugin_mode
    );
    let video = mse_segments::load("h264-low")?;
    if wanted("types") {
        println!("--- types");
        types()?;
    }
    for (name, probe) in [
        ("offset", offsets as fn(&SegmentedStream) -> R<()>),
        ("duration", durations),
        ("window", windows),
        ("remove", removals),
        ("errors", errors),
        ("quota", quota),
        ("lag", lag),
        ("bisect", bisect),
    ] {
        if wanted(name) {
            println!("--- {name}");
            if let Err(error) = probe(&video) {
                println!("--- {name} stopped early: {error}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}
