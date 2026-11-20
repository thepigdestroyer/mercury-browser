// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T05: throughput and latency. Time from `append` to `UpdateEnd` for 2 KB, 64 KB, 1 MB and 8 MB (cuts
//! of a tiled stream), bytes per second over 1,000 small appends, the cost of the copy into a
//! `GstBuffer`, and the decode throughput of a synthetic 1080p H.264 and VP9 stream (made at run time
//! into the scratch directory) with `sync=false`: frames per second and CPU time per decoder.

use std::time::{Duration, Instant};

use gstreamer as gst;
use mse_rig::RigError;
use mse_rig::backend::{EosError, MseError};
use mse_rig::events::SourceBufferId;
use mse_rig::harness::Ctx;
use mse_rig::report::{ScenarioResult, Status};
use mse_rig::rig::{Rig, RigConfig};
use mse_segments::{SegmentedStream, tile};
use serde_json::json;

use crate::common::{
    APPEND_TIMEOUT, Findings, Verdict, fast_config, fixture, mb_per_s, new_rig, run_scenario,
    video_frames,
};
use crate::proc;
use crate::stats::{mean, median, ms, percentile, summary_json};
use crate::sub::{SubOutcome, SubSpec, part, run_all};
use crate::synth::{SynthCodec, SynthError, SynthSpec, make_stream};

/// The brief's numbers: at least this many MB/s and an `UpdateEnd` below this for a 1 MB append.
pub const MIN_MB_PER_S: f64 = 50.0;
pub const MAX_UPDATE_END_MS_1MB: f64 = 100.0;

/// What to measure (the unit test uses a small plan).
#[derive(Debug, Clone)]
pub struct Plan {
    /// (append size in bytes, number of appends).
    pub sizes: Vec<(usize, usize)>,
    pub small_appends: usize,
    pub small_append_bytes: usize,
    /// Seconds of 1080p to synthesise per codec; `None` skips the decode part.
    pub synth_seconds: Option<u32>,
    /// Whether the 1 MB acceptance numbers are checked (needs at least 5 appends of 1 MB).
    pub check_acceptance: bool,
}

impl Plan {
    pub fn full() -> Plan {
        Plan {
            sizes: vec![
                (2 * 1024, 20),
                (64 * 1024, 20),
                (1024 * 1024, 10),
                (8 * 1024 * 1024, 2),
            ],
            small_appends: 1000,
            small_append_bytes: 1800,
            synth_seconds: Some(10),
            check_acceptance: true,
        }
    }

    #[cfg(test)]
    pub fn small() -> Plan {
        Plan {
            sizes: vec![(2 * 1024, 3), (64 * 1024, 3), (1024 * 1024, 2)],
            small_appends: 40,
            small_append_bytes: 1800,
            synth_seconds: None,
            check_acceptance: false,
        }
    }

    pub fn total_bytes(&self) -> usize {
        self.sizes.iter().map(|(size, count)| size * count).sum()
    }
}

pub const PART_VARIABLE: &str = "MSE_STRESS_T05_PART";

/// The parts of T05, each of which can run in a process of its own.
pub const PARTS: [&str; 6] = ["latency", "settle", "small", "copy", "h264", "vp9"];

/// `run` is the driver: GstMSE can crash a process some time after a rig was shut down (T04), and a
/// crash must not take the numbers of the other parts with it, so every part runs in a nested run of
/// this executable, one at a time (they measure speed), and a part that dies is run once more.
pub fn run(ctx: &Ctx) -> ScenarioResult {
    let plan = Plan::full();
    match part(PART_VARIABLE) {
        Some(name) => run_scenario("T05", "Throughput and latency", ctx, move |ctx, result| {
            run_parts(ctx, result, &plan, &[name.as_str()])
        }),
        None => run_scenario("T05", "Throughput and latency", ctx, drive_parts),
    }
}

fn drive_parts(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let mut findings = Findings::new();
    let since = std::time::SystemTime::now();
    let mut crashed_attempts = Vec::new();
    for name in PARTS {
        let mut report = None;
        for attempt in 1..=2 {
            let spec = SubSpec {
                id: "T05".to_owned(),
                tag: format!("part-{name}-{attempt}"),
                env: vec![(PART_VARIABLE.to_owned(), name.to_owned())],
                iterations: None,
                seed: None,
                deadline: Duration::from_secs(120),
            };
            let outcome = run_all(ctx, &[spec], 1, Duration::from_secs(130))
                .into_iter()
                .next();
            match outcome {
                Some(SubOutcome::Report {
                    result: part_result,
                    ..
                }) if part_result.status != Status::Error => {
                    report = Some(part_result);
                    break;
                }
                Some(SubOutcome::Report {
                    result: part_result,
                    exit,
                }) => {
                    if part_result
                        .notes
                        .iter()
                        .any(|n| n.to_lowercase().contains("panic"))
                    {
                        return Err(format!("part {name}: {}", part_result.notes.join(" | ")));
                    }
                    crashed_attempts.push(format!(
                        "part {name}, attempt {attempt}: died ({exit}): {}",
                        part_result.notes.join(" | ")
                    ));
                }
                Some(other) => crashed_attempts.push(format!(
                    "part {name}, attempt {attempt}: {}",
                    other.describe()
                )),
                None => {
                    crashed_attempts.push(format!("part {name}, attempt {attempt}: nothing ran"))
                }
            }
        }
        match report {
            Some(part_result) => {
                for (key, value) in &part_result.metrics {
                    if !matches!(
                        key.as_str(),
                        "shape" | "plugin_mode" | "glib_main_loop" | "tracks"
                    ) {
                        result.metric(key, value.clone());
                    }
                }
                for note in &part_result.notes {
                    if let Some(reason) = note.strip_prefix("FAIL: ") {
                        findings.fail(reason.to_owned());
                    } else if !note.starts_with("ran with ") {
                        result.note(note.clone());
                    }
                }
            }
            None => findings.fail(format!("the {name} part died twice, so it has no numbers")),
        }
    }
    result.metric("crashed_attempts", json!(crashed_attempts));
    let crashes = crate::sub::crash_summaries_since(since);
    result.metric("crash_reports", json!(crashes));
    for crash in &crashes {
        result.note(format!("macOS {crash}"));
    }
    for crash in &crashed_attempts {
        result.note(format!(
            "a part process died (GstMSE teardown crash, see T04): {crash}"
        ));
    }
    Ok(findings.verdict())
}

/// The whole plan in this process (what the unit test runs).
#[cfg(test)]
pub fn run_with(ctx: &Ctx, plan: &Plan) -> ScenarioResult {
    let plan = plan.clone();
    run_scenario("T05", "Throughput and latency", ctx, move |ctx, result| {
        let parts: Vec<&str> = if plan.synth_seconds.is_some() {
            PARTS.to_vec()
        } else {
            PARTS[..4].to_vec()
        };
        run_parts(ctx, result, &plan, &parts)
    })
}

/// Is `buffered` up to date at `UpdateEnd`? Appends whole segments in two sizes and reads `buffered` at the
/// `UpdateEnd` and again until it stops changing: how long after `updateend` the data becomes visible.
fn settle_part(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    video: &SegmentedStream,
    findings: &mut Findings,
) -> Result<(), String> {
    let tiled = tile(video, 40).map_err(|e| format!("tile: {e}"))?;
    let rig = new_rig(video_only(ctx))?;
    let id = rig
        .backend
        .add_source_buffer(&video.mime)
        .map_err(|e| format!("add_source_buffer: {e:?}"))?;
    rig.append_and_wait(id, &video.init.bytes, APPEND_TIMEOUT)
        .map_err(|e| format!("init append: {e}"))?;
    let end_of = |rig: &Rig| -> f64 {
        rig.backend
            .buffered(id)
            .ok()
            .and_then(|ranges| ranges.last().map(|r| r.end.nseconds() as f64 / 1e9))
            .unwrap_or(0.0)
    };
    let mut table = Vec::new();
    let mut next = 0usize;
    for target_bytes in [64 * 1024usize, 1024 * 1024, 4 * 1024 * 1024] {
        let mut bytes = Vec::new();
        while bytes.len() < target_bytes && next < tiled.media.len() {
            bytes.extend_from_slice(&tiled.media[next].bytes);
            next += 1;
        }
        let before = end_of(&rig);
        let timing = rig
            .append_and_wait(id, &bytes, APPEND_TIMEOUT)
            .map_err(|e| format!("append of {} bytes: {e}", bytes.len()))?;
        let at_update_end = end_of(&rig);
        let started = Instant::now();
        let mut last = at_update_end;
        let mut stable_since = Instant::now();
        while started.elapsed() < Duration::from_secs(3)
            && stable_since.elapsed() < Duration::from_millis(60)
        {
            std::thread::sleep(Duration::from_millis(1));
            let now = end_of(&rig);
            if (now - last).abs() > 1e-9 {
                last = now;
                stable_since = Instant::now();
            }
        }
        let settle_ms = (stable_since - started).as_secs_f64() * 1000.0;
        table.push(json!({
            "appended_bytes": bytes.len(),
            "to_update_end_ms": ms(timing.to_update_end),
            "buffered_end_before_s": before,
            "buffered_end_at_update_end_s": at_update_end,
            "buffered_end_settled_s": last,
            "settled_after_update_end_ms": (settle_ms * 10.0).round() / 10.0,
        }));
        result.note(format!(
            "settle: appended {} bytes; buffered end {before:.2} s before, {at_update_end:.2} s at UpdateEnd ({:.2} ms after the call), {last:.2} s once settled ({settle_ms:.0} ms later)",
            bytes.len(),
            ms(timing.to_update_end)
        ));
        findings.check(
            (last - at_update_end).abs() < 0.04,
            format!(
                "after an append of {} bytes `buffered` ended at {at_update_end:.2} s when UpdateEnd arrived and at {last:.2} s {settle_ms:.0} ms later: UpdateEnd comes before the data is in `buffered`, so a page that reads `buffered` in its updateend handler sees stale ranges",
                bytes.len()
            ),
        );
    }
    result.metric("buffered_at_update_end", json!(table));
    let _ = rig.shutdown();
    Ok(())
}

fn video_only(ctx: &Ctx) -> RigConfig {
    RigConfig {
        audio: false,
        ..fast_config(ctx)
    }
}

fn run_parts(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    plan: &Plan,
    parts: &[&str],
) -> Result<Verdict, String> {
    let video = fixture("h264-low")?;
    let mut findings = Findings::new();
    let seconds = plan.synth_seconds.unwrap_or(10);
    for name in parts {
        crate::common::trace(&format!("T05: part {name}"));
        match *name {
            "latency" => latency_by_size(ctx, result, &video, plan, &mut findings)?,
            "settle" => settle_part(ctx, result, &video, &mut findings)?,
            "small" => small_appends(ctx, result, &video, plan, &mut findings)?,
            "copy" => copy_cost(result),
            "h264" => decode_throughput(ctx, result, SynthCodec::H264, seconds, &mut findings)?,
            "vp9" => decode_throughput(ctx, result, SynthCodec::Vp9, seconds, &mut findings)?,
            other => return Err(format!("unknown T05 part {other:?}")),
        }
    }
    Ok(findings.verdict())
}

/// Appends `data` and waits for UpdateEnd; on `QuotaExceeded` removes everything behind `trim_to`
/// once and tries again. Returns the timing and whether a quota error happened.
fn append_trimming(
    rig: &Rig,
    id: SourceBufferId,
    data: &[u8],
    trim_to: Option<mse_rig::ClockTime>,
) -> Result<(mse_rig::rig::AppendTiming, bool), String> {
    match rig.append_and_wait(id, data, APPEND_TIMEOUT) {
        Ok(timing) => Ok((timing, false)),
        Err(RigError::Mse(MseError::QuotaExceeded(_))) => {
            if let Some(end) = trim_to {
                let _ = rig.backend.remove(id, mse_rig::ClockTime::ZERO, Some(end));
                rig.wait_until(Duration::from_secs(5), |rig| !rig.backend.updating(id));
            }
            rig.append_and_wait(id, data, APPEND_TIMEOUT)
                .map(|timing| (timing, true))
                .map_err(|e| format!("append of {} bytes after trimming: {e}", data.len()))
        }
        Err(error) => Err(format!("append of {} bytes: {error}", data.len())),
    }
}

fn label(size: usize) -> String {
    if size >= 1024 * 1024 {
        format!("{}MB", size / (1024 * 1024))
    } else {
        format!("{}KB", size / 1024)
    }
}

fn latency_by_size(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    video: &SegmentedStream,
    plan: &Plan,
    findings: &mut Findings,
) -> Result<(), String> {
    // One continuous byte stream, cut into appends of the requested sizes in turn.
    let one = video
        .media
        .iter()
        .map(|s| s.bytes.len())
        .sum::<usize>()
        .max(1);
    let copies = plan.total_bytes() / one + 2;
    let tiled = tile(video, copies).map_err(|e| format!("tile: {e}"))?;
    let bytes: Vec<u8> = tiled
        .media
        .iter()
        .flat_map(|s| s.bytes.iter().copied())
        .collect();
    result.metric("latency_stream_bytes", bytes.len());
    result.metric(
        "latency_stream_seconds",
        tiled.duration_ms().map(|d| d / 1000),
    );
    let rig = new_rig(video_only(ctx))?;
    let id = rig
        .backend
        .add_source_buffer(&video.mime)
        .map_err(|e| format!("add_source_buffer: {e:?}"))?;
    rig.append_and_wait(id, &video.init.bytes, APPEND_TIMEOUT)
        .map_err(|e| format!("init append: {e}"))?;
    rig.play().map_err(|e| format!("play: {e}"))?;

    let mut offset = 0usize;
    let mut quota_hits = 0usize;
    let mut table = serde_json::Map::new();
    let mut one_mb_rates = Vec::new();
    let mut one_mb_end_ms = Vec::new();
    for (size, count) in &plan.sizes {
        crate::common::trace(&format!("T05: {count} appends of {}", label(*size)));
        let mut to_end = Vec::new();
        let mut to_start = Vec::new();
        let mut rates = Vec::new();
        for _ in 0..*count {
            if offset + size > bytes.len() {
                break;
            }
            let slice = &bytes[offset..offset + size];
            offset += size;
            // Keep memory bounded: trim what the player has long passed (30 s behind the end).
            let trim_to = rig
                .position()
                .map(|p| p.saturating_sub(mse_rig::ClockTime::from_seconds(30)));
            let (timing, hit) = append_trimming(&rig, id, slice, trim_to)?;
            quota_hits += usize::from(hit);
            to_end.push(ms(timing.to_update_end));
            to_start.push(ms(timing.to_update_start));
            rates.push(mb_per_s(*size, timing.to_update_end));
        }
        if *size == 1024 * 1024 {
            one_mb_rates = rates.clone();
            one_mb_end_ms = to_end.clone();
        }
        table.insert(
            label(*size),
            json!({
                "appends": to_end.len(),
                "to_update_end_ms": summary_json(&to_end),
                "to_update_start_ms": summary_json(&to_start),
                "mb_per_s": summary_json(&rates),
            }),
        );
        result.note(format!(
            "{} appends of {}: UpdateEnd after median {:.2} ms, p95 {:.2} ms; median {:.1} MB/s",
            to_end.len(),
            label(*size),
            median(&to_end).unwrap_or(f64::NAN),
            percentile(&to_end, 95.0).unwrap_or(f64::NAN),
            median(&rates).unwrap_or(f64::NAN),
        ));
    }
    result.metric("latency_by_size", serde_json::Value::Object(table));
    result.metric("quota_errors_handled_by_trimming", quota_hits);
    if quota_hits > 0 {
        result.note(format!(
            "{quota_hits} append(s) got QuotaExceeded and were retried after removing the data behind the position"
        ));
    }
    if plan.check_acceptance {
        if one_mb_end_ms.len() < 5 {
            findings.fail(format!(
                "only {} appends of 1 MB were measured (need 5)",
                one_mb_end_ms.len()
            ));
        } else {
            let rate = median(&one_mb_rates).unwrap_or(0.0);
            let p95 = percentile(&one_mb_end_ms, 95.0).unwrap_or(f64::INFINITY);
            findings.check(
                rate >= MIN_MB_PER_S,
                format!("a 1 MB append runs at a median of {rate:.1} MB/s (need at least {MIN_MB_PER_S} MB/s)"),
            );
            findings.check(
                p95 < MAX_UPDATE_END_MS_1MB,
                format!("UpdateEnd of a 1 MB append came {p95:.1} ms after the call at p95 (need under {MAX_UPDATE_END_MS_1MB} ms)"),
            );
        }
    }
    result.metric("rss_kb_after_latency_part", proc::rss_kb().ok());
    let _ = rig.shutdown();
    Ok(())
}

fn small_appends(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    video: &SegmentedStream,
    plan: &Plan,
    findings: &mut Findings,
) -> Result<(), String> {
    let wanted = plan.small_appends * plan.small_append_bytes;
    let one = video
        .media
        .iter()
        .map(|s| s.bytes.len())
        .sum::<usize>()
        .max(1);
    let tiled = tile(video, wanted / one + 2).map_err(|e| format!("tile: {e}"))?;
    let bytes: Vec<u8> = tiled
        .media
        .iter()
        .flat_map(|s| s.bytes.iter().copied())
        .collect();
    let rig = new_rig(video_only(ctx))?;
    let id = rig
        .backend
        .add_source_buffer(&video.mime)
        .map_err(|e| format!("add_source_buffer: {e:?}"))?;
    rig.append_and_wait(id, &video.init.bytes, APPEND_TIMEOUT)
        .map_err(|e| format!("init append: {e}"))?;
    rig.play().map_err(|e| format!("play: {e}"))?;
    let mut latencies = Vec::new();
    let mut total = 0usize;
    let started = Instant::now();
    for index in 0..plan.small_appends {
        let from = index * plan.small_append_bytes;
        let to = (from + plan.small_append_bytes).min(bytes.len());
        if from >= to {
            break;
        }
        let (timing, _) = append_trimming(&rig, id, &bytes[from..to], None)?;
        latencies.push(ms(timing.to_update_end));
        total += to - from;
    }
    let took = started.elapsed();
    result.metric(
        "small_appends",
        json!({
            "count": latencies.len(),
            "bytes_each": plan.small_append_bytes,
            "total_bytes": total,
            "wall_ms": ms(took),
            "bytes_per_s": (total as f64 / took.as_secs_f64().max(1e-9)).round(),
            "appends_per_s": (latencies.len() as f64 / took.as_secs_f64().max(1e-9)).round(),
            "to_update_end_ms": summary_json(&latencies),
        }),
    );
    result.note(format!(
        "{} small appends of {} bytes took {:.0} ms: {:.0} appends/s, {:.2} MB/s, UpdateEnd after a mean of {:.2} ms (this part is latency-bound: it is not held to the 50 MB/s figure)",
        latencies.len(),
        plan.small_append_bytes,
        ms(took),
        latencies.len() as f64 / took.as_secs_f64().max(1e-9),
        mb_per_s(total, took),
        mean(&latencies).unwrap_or(f64::NAN)
    ));
    findings.check(
        latencies.len() == plan.small_appends,
        format!(
            "only {} of {} small appends could be made",
            latencies.len(),
            plan.small_appends
        ),
    );
    let _ = rig.shutdown();
    Ok(())
}

/// The cost of putting the page's bytes into a `GstBuffer`: one copy into a `Vec` that the buffer owns
/// (what `MseBackend::append` does), and, for comparison, an allocation plus `copy_from_slice`.
fn copy_cost(result: &mut ScenarioResult) {
    let mut table = serde_json::Map::new();
    for size in [1024 * 1024usize, 8 * 1024 * 1024] {
        let data = vec![0xA5u8; size];
        let rounds = if size >= 8 * 1024 * 1024 { 10 } else { 40 };
        let mut vec_ms = Vec::new();
        let mut map_ms = Vec::new();
        for _ in 0..rounds {
            let started = Instant::now();
            let buffer = gst::Buffer::from_slice(data.to_vec());
            vec_ms.push(ms(started.elapsed()));
            drop(buffer);
            let started = Instant::now();
            let mut buffer = gst::Buffer::with_size(size).unwrap_or_else(|_| gst::Buffer::new());
            if let Some(inner) = buffer.get_mut()
                && let Ok(mut map) = inner.map_writable()
            {
                map.copy_from_slice(&data);
            }
            map_ms.push(ms(started.elapsed()));
            drop(buffer);
        }
        table.insert(
            label(size),
            json!({
                "vec_copy_then_from_slice_ms": summary_json(&vec_ms),
                "with_size_then_copy_ms": summary_json(&map_ms),
                "vec_copy_gb_per_s": median(&vec_ms).map(|m| (size as f64 / 1e9) / (m / 1000.0)).map(|v| (v * 100.0).round() / 100.0),
            }),
        );
        result.note(format!(
            "copy of {} into a GstBuffer: {:.3} ms as a Vec copy + from_slice, {:.3} ms as with_size + copy_from_slice (medians)",
            label(size),
            median(&vec_ms).unwrap_or(f64::NAN),
            median(&map_ms).unwrap_or(f64::NAN)
        ));
    }
    result.metric("copy_cost", serde_json::Value::Object(table));
}

fn decode_throughput(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    codec: SynthCodec,
    seconds: u32,
    findings: &mut Findings,
) -> Result<(), String> {
    crate::common::trace(&format!("T05: 1080p {} decode", codec.label()));
    let key = format!("decode_1080p_{}", codec.label());
    let spec = SynthSpec::full_hd(codec, seconds);
    let (stream, encode_took) = match make_stream(&spec, &ctx.scratch) {
        Ok(made) => made,
        Err(SynthError::Missing(why)) => {
            result.metric(&key, json!({ "measured": false, "reason": why.clone() }));
            result.note(format!(
                "1080p {} decode not measured: {why}",
                codec.label()
            ));
            return Ok(());
        }
        Err(SynthError::Failed(why)) => {
            result.metric(&key, json!({ "measured": false, "reason": why.clone() }));
            result.note(format!(
                "1080p {} decode not measured: the synthetic stream could not be made: {why}",
                codec.label()
            ));
            return Ok(());
        }
    };
    let stream_bytes = stream.concat().len();
    let expected = u64::from(spec.frames());
    let rig = new_rig(video_only(ctx))?;
    let id = rig
        .backend
        .add_source_buffer(&stream.mime)
        .map_err(|e| format!("add_source_buffer({}): {e:?}", stream.mime))?;
    rig.append_and_wait(id, &stream.init.bytes, APPEND_TIMEOUT)
        .map_err(|e| format!("init append of the synthetic {} stream: {e}", codec.label()))?;
    rig.play().map_err(|e| format!("play: {e}"))?;
    let cpu_before = proc::cpu_time_ms()?;
    let started = Instant::now();
    // A page appends what the network gives it, a megabyte or so at a time (the encoder's fragments are
    // far bigger than that for snow), and removes what has been played when the buffer is full.
    let mut quota_hits = 0usize;
    let mut appended_bytes = 0usize;
    let mut append_error = None;
    'feeding: for segment in &stream.media {
        for piece in segment.bytes.chunks(1024 * 1024) {
            let trim_to = rig
                .position()
                .map(|p| p.saturating_sub(mse_rig::ClockTime::from_seconds(1)));
            match append_trimming(&rig, id, piece, trim_to) {
                Ok((_, hit)) => {
                    quota_hits += usize::from(hit);
                    appended_bytes += piece.len();
                }
                Err(error) => {
                    append_error = Some(error);
                    break 'feeding;
                }
            }
        }
    }
    if let Some(error) = append_error {
        let text = format!(
            "the synthetic 1080p {} stream could not be appended past {:.1} MB of {:.1} MB: {error}",
            codec.label(),
            appended_bytes as f64 / 1e6,
            stream_bytes as f64 / 1e6
        );
        result.metric(&key, json!({ "measured": false, "reason": text.clone() }));
        findings.fail(text);
        let _ = rig.shutdown();
        return Ok(());
    }
    result.metric(
        &format!("{key}_quota_errors_handled_by_trimming"),
        quota_hits,
    );
    rig.backend
        .end_of_stream(EosError::None)
        .map_err(|e| format!("end_of_stream: {e:?}"))?;
    let reached = rig.wait_until(Duration::from_secs(90), |rig| {
        video_frames(rig) >= expected.saturating_sub(2)
    });
    let wall = started.elapsed();
    let cpu_ms = proc::cpu_time_ms()?.saturating_sub(cpu_before);
    std::thread::sleep(Duration::from_millis(200));
    let (count, first_at, last_at) = {
        let stats = rig.video.lock().unwrap_or_else(|e| e.into_inner());
        (
            stats.count,
            stats.arrivals.first().map(|a| a.at),
            stats.arrivals.last().map(|a| a.at),
        )
    };
    let fps = match (first_at, last_at) {
        (Some(first), Some(last)) if last > first && count > 1 => {
            Some((count - 1) as f64 / (last - first).as_secs_f64())
        }
        _ => None,
    };
    let decoders = rig.chosen_decoders();
    result.metric(
        &key,
        json!({
            "measured": true,
            "frames_expected": expected,
            "frames_arrived": count,
            "stream_bytes": stream_bytes,
            "encode_s": (encode_took.as_secs_f64() * 10.0).round() / 10.0,
            "wall_ms_first_append_to_last_frame": ms(wall),
            "fps_first_to_last_frame": fps.map(|f| (f * 10.0).round() / 10.0),
            "realtime_factor": fps.map(|f| (f / f64::from(spec.fps) * 100.0).round() / 100.0),
            "cpu_ms": cpu_ms,
            "cpu_ms_per_frame": if count > 0 { Some((cpu_ms as f64 / count as f64 * 100.0).round() / 100.0) } else { None },
            "cpu_cores_busy": if wall.as_millis() > 0 { Some((cpu_ms as f64 / wall.as_millis() as f64 * 100.0).round() / 100.0) } else { None },
            "decoders": decoders.clone(),
        }),
    );
    result.note(format!(
        "1080p {} ({} frames, {:.1} MB, encoded in {:.0} s): {} frames arrived, {} fps at the sink, {} ms CPU ({:.1} ms per frame); decoders {:?}. The picture is television snow held to about 6 Mbit/s, and the rig scales every frame to 64 px for its colour sample, which is included in the time.",
        codec.label(),
        expected,
        stream_bytes as f64 / 1e6,
        encode_took.as_secs_f64(),
        count,
        fps.map_or("n/a".to_owned(), |f| format!("{f:.1}")),
        cpu_ms,
        if count > 0 { cpu_ms as f64 / count as f64 } else { f64::NAN },
        decoders,
    ));
    findings.check(
        reached,
        format!(
            "the synthetic 1080p {} stream produced {count} of {expected} frames within 90 s",
            codec.label()
        ),
    );
    let _ = rig.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    use mse_rig::env::{InitOptions, PluginMode, init};
    use mse_rig::report::Status;
    use mse_rig::rig::Shape;

    static INFO: OnceLock<Result<mse_rig::env::GstInfo, String>> = OnceLock::new();

    fn gst_info() -> Result<mse_rig::env::GstInfo, String> {
        INFO.get_or_init(|| {
            let scratch = std::env::temp_dir().join("opencode/lanes/sp-mse/stress-test-registry");
            init(&InitOptions {
                prefix: None,
                plugin_mode: PluginMode::SystemPath,
                scratch_dir: scratch,
                glib_main_loop: false,
            })
            .map_err(|e| e.to_string())
        })
        .clone()
    }

    #[test]
    fn the_full_plan_is_the_briefs() {
        let plan = Plan::full();
        assert_eq!(
            plan.sizes.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            [2048, 65536, 1048576, 8388608]
        );
        assert_eq!(plan.small_appends, 1000);
        assert_eq!(plan.synth_seconds, Some(10));
        assert!(plan.check_acceptance);
        assert_eq!(
            plan.total_bytes(),
            2048 * 20 + 65536 * 20 + 1048576 * 10 + 8388608 * 2
        );
    }

    #[test]
    fn labels_use_kb_and_mb() {
        assert_eq!(label(2048), "2KB");
        assert_eq!(label(65536), "64KB");
        assert_eq!(label(1024 * 1024), "1MB");
        assert_eq!(label(8 * 1024 * 1024), "8MB");
    }

    /// T05 at a small size, end to end, on the real GStreamer: MED-1 can reuse it as a regression test.
    #[test]
    fn t05_runs_end_to_end_at_a_small_size() {
        let info = match gst_info() {
            Ok(info) => info,
            Err(why) => panic!("GStreamer could not be set up: {why}"),
        };
        let scratch = std::env::temp_dir().join("opencode/lanes/sp-mse/stress-test-t05");
        std::fs::create_dir_all(&scratch).unwrap();
        let ctx = Ctx {
            shape: Shape::PlayBin,
            plugin_mode: PluginMode::SystemPath,
            glib_main_loop: false,
            scratch: scratch.clone(),
            iterations: 1,
            seed: 1,
            info,
        };
        let result = run_with(&ctx, &Plan::small());
        let _ = std::fs::remove_dir_all(&scratch);
        // The scenario must have run (no harness error). It currently reports one GstMSE finding, that UpdateEnd
        // comes before the data is in `buffered`; when GstMSE is fixed the status becomes Pass, and any
        // other failure is a regression.
        assert!(
            result.status == Status::Pass || result.status == Status::Fail,
            "T05 (small) is {:?}: {:?}",
            result.status,
            result.notes
        );
        let unexpected: Vec<&String> = result
            .notes
            .iter()
            .filter(|n| {
                n.starts_with("FAIL: ")
                    && !n.contains("UpdateEnd comes before the data is in `buffered`")
            })
            .collect();
        assert!(unexpected.is_empty(), "unexpected failures: {unexpected:?}");
        assert!(result.metrics.contains_key("latency_by_size"));
        assert!(result.metrics.contains_key("small_appends"));
        assert!(result.metrics.contains_key("copy_cost"));
    }
}
