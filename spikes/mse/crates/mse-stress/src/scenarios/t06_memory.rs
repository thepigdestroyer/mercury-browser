// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T06: memory, eviction and quota. Tile `h264-low` + `aac` to 10 minutes and carry on appending
//! without removing, up to 1 GiB of appended data or a time budget, watching RSS against the bytes
//! appended; find the point (if any) where GstMSE evicts by itself or raises `QuotaExceeded`; read
//! `buffered` afterwards; see whether `remove` gives the memory back; read the track-queue limit
//! (`queue-size`) from the `msesrc` pads.
//!
//! Three phases, each with a rig of its own: `h264-low` + `aac` with the pipeline paused (a page
//! that buffers ahead without playing), `h264-high` + `aac` paused (more bytes per second of media,
//! so that a byte limit and a duration limit can be told apart), and `h264-low` + `aac` while the
//! pipeline plays as fast as it can (position moves, so eviction behind it is possible).

use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use mse_rig::ClockTime;
use mse_rig::backend::MseError;
use mse_rig::events::{MseEvent, SourceBufferId};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::Rig;
use mse_segments::{SegmentedStream, shift_media_time};
use serde_json::{Value, json};

use crate::common::{
    APPEND_TIMEOUT, Findings, Verdict, add_buffers, fast_config, fixture, new_rig, settled_ranges,
    wait_buffered_end,
};
use crate::proc;
use crate::stats::slope;
use mse_rig::RigError;

/// The cap of the brief.
pub const CAP_BYTES: u64 = 1024 * 1024 * 1024;

/// Wall-clock budget per phase.
const PHASE_BUDGET: Duration = Duration::from_secs(50);

/// The memory bound: RSS growth may be this many times the buffered bytes plus a fixed allowance
/// for decoders, queues and the rig's own bookkeeping.
pub const RSS_FACTOR: f64 = 2.0;
pub const RSS_ALLOWANCE_KB: f64 = 64.0 * 1024.0;

/// 10 minutes of tiled media: 75 copies of the 8 s fixtures.
pub const TEN_MINUTES_COPIES: usize = 75;

pub fn run(ctx: &Ctx) -> ScenarioResult {
    crate::common::run_isolated(
        "T06",
        "Memory, eviction and quota",
        ctx,
        "MSE_STRESS_T06_NESTED",
        std::time::Duration::from_secs(270),
        body,
    )
}

/// One phase's outcome.
#[derive(Debug, Clone, Default)]
struct PhaseOutcome {
    name: String,
    bytes_appended: u64,
    seconds_appended: f64,
    copies: usize,
    /// (bytes appended, seconds appended, error text) at the first `QuotaExceeded`.
    quota: Option<(u64, f64, String)>,
    /// (bytes appended, seconds appended) when `buffered` first started after 0 although we had never removed anything.
    eviction: Option<(u64, f64)>,
    rss_start_kb: u64,
    rss_end_kb: u64,
    /// (MB appended, RSS in MB), thinned.
    rss_curve: Vec<(f64, f64)>,
    /// `buffered` of the video buffer at the end: (start s, end s, ranges).
    buffered_end: Option<(f64, f64, usize)>,
    at_ten_minutes: Option<(f64, f64, f64)>,
    unexpected_error: Option<String>,
    budget_hit: bool,
    cap_hit: bool,
    elapsed: Duration,
    remove_report: Option<Value>,
    queue_properties: Option<Value>,
    video_frames_played: u64,
}

fn body(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let low = fixture("h264-low")?;
    let high = fixture("h264-high")?;
    let aac = fixture("aac")?;
    let mut findings = Findings::new();
    let mut outcomes = Vec::new();
    for (name, video, play) in [
        ("A1-low-paused", &low, false),
        ("A2-high-paused", &high, false),
        ("B-low-playing", &low, true),
    ] {
        let outcome = run_phase(ctx, name, video, &aac, play)?;
        record_phase(result, &outcome);
        check_phase(&outcome, &mut findings);
        outcomes.push(outcome);
    }
    let rule = classify_quota(
        outcomes[0].quota.as_ref().map(|q| (q.0 as f64, q.1)),
        outcomes[1].quota.as_ref().map(|q| (q.0 as f64, q.1)),
    );
    result.metric("quota_rule", rule);
    let evicted = outcomes.iter().any(|o| o.eviction.is_some());
    result.metric("eviction_seen", evicted);
    result.note(format!(
        "quota rule (paused phases, low and high bit rate): {rule}; eviction by GstMSE itself seen: {evicted}"
    ));
    if rule == "none" && !evicted {
        result.note(format!(
            "neither a QuotaExceeded error nor eviction up to {} MB appended (or the time budget): a page never learns that memory is full, so MED-1's wrapper or the DOM has to enforce a buffer limit itself",
            CAP_BYTES / (1024 * 1024)
        ));
    }
    Ok(findings.verdict())
}

fn record_phase(result: &mut ScenarioResult, outcome: &PhaseOutcome) {
    let name = &outcome.name;
    let growth = outcome.rss_end_kb as f64 - outcome.rss_start_kb as f64;
    let curve_slope = slope(
        &outcome.rss_curve.iter().map(|p| p.0).collect::<Vec<_>>(),
        &outcome.rss_curve.iter().map(|p| p.1).collect::<Vec<_>>(),
    );
    result.metric(
        &format!("phase_{name}"),
        json!({
            "bytes_appended_mb": outcome.bytes_appended as f64 / 1e6,
            "seconds_appended": outcome.seconds_appended,
            "copies": outcome.copies,
            "elapsed_s": (outcome.elapsed.as_secs_f64() * 10.0).round() / 10.0,
            "stopped_by": if outcome.quota.is_some() { "quota" } else if outcome.cap_hit { "1 GiB cap" } else if outcome.budget_hit { "time budget" } else { "other" },
            "quota": outcome.quota.as_ref().map(|(bytes, secs, error)| json!({ "bytes_mb": *bytes as f64 / 1e6, "seconds": secs, "error": error })),
            "eviction_first_seen": outcome.eviction.map(|(bytes, secs)| json!({ "bytes_mb": bytes as f64 / 1e6, "seconds": secs })),
            "rss_start_mb": outcome.rss_start_kb as f64 / 1024.0,
            "rss_end_mb": outcome.rss_end_kb as f64 / 1024.0,
            "rss_growth_mb": growth / 1024.0,
            "rss_mb_per_mb_appended": curve_slope.map(|s| (s * 1000.0).round() / 1000.0),
            "rss_curve_mb": outcome.rss_curve.iter().map(|(a, r)| json!([(a * 10.0).round() / 10.0, (r * 10.0).round() / 10.0])).collect::<Vec<_>>(),
            "buffered_end": outcome.buffered_end.map(|(s, e, n)| json!({ "start_s": s, "end_s": e, "ranges": n })),
            "at_ten_minutes": outcome.at_ten_minutes.map(|(secs, mb, rss)| json!({ "media_s": secs, "appended_mb": mb, "rss_mb": rss })),
            "after_remove": outcome.remove_report.clone(),
            "queue_properties": outcome.queue_properties.clone(),
            "video_frames_played": outcome.video_frames_played,
            "unexpected_error": outcome.unexpected_error.clone(),
        }),
    );
    result.note(format!(
        "{name}: appended {:.0} MB ({:.0} s of media) in {:.0} s; {}; RSS {:.0} -> {:.0} MB; buffered at the end {}",
        outcome.bytes_appended as f64 / 1e6,
        outcome.seconds_appended,
        outcome.elapsed.as_secs_f64(),
        match &outcome.quota {
            Some((bytes, secs, error)) => format!("QuotaExceeded after {:.0} MB / {secs:.0} s ({error})", *bytes as f64 / 1e6),
            None if outcome.cap_hit => "no quota error up to the 1 GiB cap".to_owned(),
            None => "no quota error before the time budget ended".to_owned(),
        },
        outcome.rss_start_kb as f64 / 1024.0,
        outcome.rss_end_kb as f64 / 1024.0,
        outcome
            .buffered_end
            .map_or("unknown".to_owned(), |(s, e, n)| format!("[{s:.1} s, {e:.1} s] in {n} range(s)")),
    ));
    if let Some((bytes, secs)) = outcome.eviction {
        result.note(format!(
            "{name}: GstMSE evicted on its own: buffered no longer started at 0 after {:.0} MB / {secs:.0} s appended",
            bytes as f64 / 1e6
        ));
    }
    if let Some(report) = &outcome.remove_report {
        result.note(format!("{name}: after remove: {report}"));
    }
}

fn check_phase(outcome: &PhaseOutcome, findings: &mut Findings) {
    if let Some(error) = &outcome.unexpected_error {
        findings.fail(format!("{}: {error}", outcome.name));
    }
    // RSS bounded by about the buffered bytes: estimate them from what is still buffered.
    let buffered_fraction = match (outcome.buffered_end, outcome.seconds_appended) {
        (Some((start, end, _)), appended) if appended > 0.0 => {
            ((end - start).max(0.0) / appended).clamp(0.0, 1.0)
        }
        _ => 1.0,
    };
    let buffered_kb = outcome.bytes_appended as f64 / 1024.0 * buffered_fraction;
    let growth_kb = outcome.rss_end_kb as f64 - outcome.rss_start_kb as f64;
    let limit = rss_limit_kb(buffered_kb);
    if growth_kb > limit {
        findings.fail(format!(
            "{}: RSS grew by {:.0} MB while about {:.0} MB were buffered (limit {RSS_FACTOR}x the buffered bytes plus {:.0} MB)",
            outcome.name,
            growth_kb / 1024.0,
            buffered_kb / 1024.0,
            RSS_ALLOWANCE_KB / 1024.0
        ));
    }
}

/// The most RSS growth (kilobytes) that `buffered_kb` of buffered data may explain.
pub fn rss_limit_kb(buffered_kb: f64) -> f64 {
    RSS_FACTOR * buffered_kb + RSS_ALLOWANCE_KB
}

/// Tells a byte limit from a duration limit from the two paused phases: `(bytes, seconds)` at the first
/// `QuotaExceeded` of the low and the high bit rate stream.
pub fn classify_quota(low: Option<(f64, f64)>, high: Option<(f64, f64)>) -> &'static str {
    match (low, high) {
        (None, None) => "none",
        (Some(_), None) | (None, Some(_)) => "undetermined (only one bit rate hit the limit)",
        (Some((low_bytes, low_secs)), Some((high_bytes, high_secs))) => {
            let close = |a: f64, b: f64| (a - b).abs() <= 0.15 * a.max(b);
            match (close(low_bytes, high_bytes), close(low_secs, high_secs)) {
                (true, false) => "bytes",
                (false, true) => "duration",
                (true, true) => {
                    "bytes and duration are indistinguishable (the two streams are too alike)"
                }
                (false, false) => {
                    "neither bytes nor duration alone (a mix, or a limit that depends on something else)"
                }
            }
        }
    }
}

fn copy_of(stream: &SegmentedStream, copy: usize) -> Result<SegmentedStream, String> {
    let mut one = stream.clone();
    if copy > 0 {
        let span = stream
            .span_ms()
            .map(|(start, end)| (end - start) as i64)
            .ok_or("the stream has no span to shift by")?;
        shift_media_time(&mut one, span * copy as i64)
            .map_err(|e| format!("shift_media_time({copy}): {e}"))?;
    }
    Ok(one)
}

fn buffered_summary(rig: &Rig, id: SourceBufferId) -> Option<(f64, f64, usize)> {
    let ranges = rig.backend.buffered(id).ok()?;
    let first = ranges.first()?;
    let last = ranges.last()?;
    Some((
        first.start.nseconds() as f64 / 1e9,
        last.end.nseconds() as f64 / 1e9,
        ranges.len(),
    ))
}

fn run_phase(
    ctx: &Ctx,
    name: &str,
    video: &SegmentedStream,
    audio: &SegmentedStream,
    play: bool,
) -> Result<PhaseOutcome, String> {
    let mut outcome = PhaseOutcome {
        name: name.to_owned(),
        ..PhaseOutcome::default()
    };
    let span_ms = video
        .span_ms()
        .map(|(start, end)| (end - start) as i64)
        .ok_or("the fixture has no span")?;
    outcome.rss_start_kb = proc::rss_kb()?;
    let started = Instant::now();
    let rig = new_rig(fast_config(ctx))?;
    let ids = add_buffers(&rig, &video.mime, &audio.mime)?;
    let vid = ids[0];
    let mut appended = 0u64;
    for (id, init) in ids.iter().zip([&video.init, &audio.init]) {
        rig.append_and_wait(*id, &init.bytes, APPEND_TIMEOUT)
            .map_err(|e| format!("init segment: {e}"))?;
        appended += init.bytes.len() as u64;
    }
    let mut seconds = 0.0f64;
    let copy_seconds = span_ms as f64 / 1000.0;
    let mut next_curve_mb = 0.0f64;
    let mut queue_read = false;
    let mut played = false;

    'copies: for copy in 0..usize::MAX {
        if started.elapsed() > PHASE_BUDGET {
            outcome.budget_hit = true;
            break;
        }
        if appended >= CAP_BYTES {
            outcome.cap_hit = true;
            break;
        }
        let video_copy = copy_of(video, copy)?;
        let audio_copy = copy_of(audio, copy)?;
        for index in 0..video_copy.media.len().max(audio_copy.media.len()) {
            for (id, stream) in ids.iter().copied().zip([&video_copy, &audio_copy]) {
                let Some(segment) = stream.media.get(index) else {
                    continue;
                };
                match rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT) {
                    Ok(_) => {
                        appended += segment.bytes.len() as u64;
                        // UpdateEnd comes before the data is in `buffered`: wait for it, so that the next
                        // append does not overrun GstMSE's real processing (that is a backlog, not a quota).
                        if id == vid
                            && let Some(end_ms) = segment.end_ms()
                        {
                            wait_buffered_end(
                                &rig,
                                vid,
                                end_ms as f64 / 1000.0,
                                Duration::from_secs(2),
                            );
                        }
                    }
                    Err(RigError::Mse(MseError::QuotaExceeded(text))) => {
                        outcome.quota = Some((appended, seconds, text));
                        break 'copies;
                    }
                    Err(error) => {
                        // An error event or a refusal of another kind: report it as what it is.
                        outcome.unexpected_error = Some(format!(
                            "append #{} ({} bytes, {:.0} MB in all) failed: {error}",
                            outcome.copies * 8 + index,
                            segment.bytes.len(),
                            appended as f64 / 1e6
                        ));
                        break 'copies;
                    }
                }
            }
        }
        seconds += copy_seconds;
        outcome.copies = copy + 1;
        if play && !played {
            rig.play().map_err(|e| format!("play: {e}"))?;
            played = true;
        }
        if !queue_read && copy == 0 {
            outcome.queue_properties = Some(queue_properties(&rig));
            queue_read = true;
        }
        let appended_mb = appended as f64 / 1e6;
        if appended_mb >= next_curve_mb {
            if let Ok(rss) = proc::rss_kb() {
                outcome.rss_curve.push((appended_mb, rss as f64 / 1024.0));
            }
            next_curve_mb = (next_curve_mb * 1.35).max(appended_mb + 2.0);
        }
        if copy + 1 == TEN_MINUTES_COPIES {
            outcome.at_ten_minutes = Some((
                seconds,
                appended_mb,
                proc::rss_kb().unwrap_or(0) as f64 / 1024.0,
            ));
        }
        if copy % 5 == 4
            && outcome.eviction.is_none()
            && let Some((start, _, _)) = buffered_summary(&rig, vid)
            && start > 0.5
        {
            outcome.eviction = Some((appended, seconds));
        }
    }
    outcome.bytes_appended = appended;
    outcome.seconds_appended = seconds;
    outcome.buffered_end = buffered_summary(&rig, vid);
    if outcome.eviction.is_none()
        && let Some((start, _, _)) = outcome.buffered_end
        && start > 0.5
    {
        outcome.eviction = Some((appended, seconds));
    }
    outcome.rss_end_kb = proc::rss_kb()?;
    outcome.video_frames_played = crate::common::video_frames(&rig);
    outcome.elapsed = started.elapsed();
    let errors = rig.log.count(|e| matches!(e, MseEvent::Error(_)));
    if errors > 0 && outcome.unexpected_error.is_none() {
        outcome.unexpected_error =
            Some(format!("{errors} source buffer Error event(s) were logged"));
    }

    // Does remove give the memory back?
    if let Some((_, end, _)) = outcome.buffered_end {
        let before = proc::rss_kb()?;
        let upto = (end - 10.0).max(1.0);
        let removed = rig.backend.remove(
            vid,
            ClockTime::ZERO,
            Some(ClockTime::from_nseconds((upto * 1e9) as u64)),
        );
        let removed_audio: Vec<String> = ids
            .iter()
            .skip(1)
            .map(|id| {
                format!(
                    "{:?}",
                    rig.backend.remove(
                        *id,
                        ClockTime::ZERO,
                        Some(ClockTime::from_nseconds((upto * 1e9) as u64)),
                    )
                )
            })
            .collect();
        let finished = rig.wait_until(Duration::from_secs(60), |rig| {
            ids.iter().all(|id| !rig.backend.updating(*id))
        });
        // Read the ranges once they have stopped changing (UpdateEnd comes first, the data later).
        let settled = settled_ranges(
            &rig,
            vid,
            Duration::from_millis(400),
            Duration::from_secs(10),
        );
        let after = proc::rss_kb()?;
        let again = settled
            .first()
            .zip(settled.last())
            .map(|(first, last)| (first.0, last.1, settled.len()));
        outcome.remove_report = Some(json!({
            "removed_up_to_s": upto,
            "video_remove_call": format!("{removed:?}"),
            "audio_remove_call": format!("{removed_audio:?}"),
            "finished": finished,
            "rss_before_mb": before as f64 / 1024.0,
            "rss_after_mb": after as f64 / 1024.0,
            "rss_given_back_mb": (before as f64 - after as f64) / 1024.0,
            "buffered_after": again.map(|(s, e, n)| json!({ "start_s": s, "end_s": e, "ranges": n })),
        }));
    }
    let _ = rig.shutdown();
    Ok(outcome)
}

/// The properties of every `msesrc` pad and of the element itself, as text: where the track queue's
/// `queue-size` shows up.
fn queue_properties(rig: &Rig) -> Value {
    let Some(element) = rig.msesrc() else {
        return json!({ "error": "no msesrc element" });
    };
    let mut pads = serde_json::Map::new();
    for pad in element.pads() {
        let mut props = serde_json::Map::new();
        for spec in pad.list_properties() {
            let name = spec.name();
            if matches!(name, "parent" | "template" | "name") {
                continue;
            }
            if let Some(text) = value_as_text(&pad.property_value(name)) {
                props.insert(name.to_owned(), json!(text));
            }
        }
        pads.insert(pad.name().to_string(), Value::Object(props));
    }
    let mut element_props = serde_json::Map::new();
    for spec in element.list_properties() {
        let name = spec.name();
        if matches!(name, "parent" | "name") {
            continue;
        }
        if let Some(text) = value_as_text(&element.property_value(name)) {
            element_props.insert(name.to_owned(), json!(text));
        }
    }
    json!({ "pads": pads, "element": element_props })
}

fn value_as_text(value: &gst::glib::Value) -> Option<String> {
    value
        .transform::<String>()
        .ok()
        .and_then(|text| text.get::<String>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_quota_in_either_phase_is_none() {
        assert_eq!(classify_quota(None, None), "none");
    }

    #[test]
    fn a_byte_limit_hits_at_the_same_bytes_and_different_seconds() {
        // 100 MB at 5,000 s of low rate media and at 2,000 s of high rate media.
        assert_eq!(
            classify_quota(Some((100e6, 5000.0)), Some((101e6, 2000.0))),
            "bytes"
        );
    }

    #[test]
    fn a_duration_limit_hits_at_the_same_seconds_and_different_bytes() {
        assert_eq!(
            classify_quota(Some((40e6, 3600.0)), Some((100e6, 3600.0))),
            "duration"
        );
    }

    #[test]
    fn one_sided_or_odd_results_are_not_guessed() {
        assert!(classify_quota(Some((1.0, 1.0)), None).starts_with("undetermined"));
        assert!(classify_quota(Some((1e6, 10.0)), Some((9e6, 90.0))).starts_with("neither"));
        assert!(classify_quota(Some((1e6, 10.0)), Some((1e6, 10.0))).contains("indistinguishable"));
    }

    #[test]
    fn the_memory_bound_is_twice_the_buffered_bytes_plus_the_allowance() {
        assert_eq!(rss_limit_kb(0.0), RSS_ALLOWANCE_KB);
        assert_eq!(rss_limit_kb(1000.0), 2000.0 + RSS_ALLOWANCE_KB);
    }

    #[test]
    fn a_phase_that_bloats_is_a_finding() {
        let mut findings = Findings::new();
        let outcome = PhaseOutcome {
            name: "x".into(),
            bytes_appended: 100 * 1024 * 1024,
            seconds_appended: 100.0,
            rss_start_kb: 100 * 1024,
            rss_end_kb: 1024 * 1024,
            buffered_end: Some((0.0, 100.0, 1)),
            ..PhaseOutcome::default()
        };
        check_phase(&outcome, &mut findings);
        assert!(!findings.is_clean(), "{:?}", findings.reasons());
        let ok = PhaseOutcome {
            rss_end_kb: 250 * 1024,
            ..outcome
        };
        let mut findings = Findings::new();
        check_phase(&ok, &mut findings);
        assert!(findings.is_clean(), "{:?}", findings.reasons());
    }

    #[test]
    fn eviction_shrinks_the_buffered_estimate() {
        // 1 GB appended but only 10 of 1000 seconds still buffered: 1 GB of RSS growth would be a leak.
        let outcome = PhaseOutcome {
            name: "y".into(),
            bytes_appended: 1_000_000_000,
            seconds_appended: 1000.0,
            rss_start_kb: 100 * 1024,
            rss_end_kb: 100 * 1024 + 900_000,
            buffered_end: Some((990.0, 1000.0, 1)),
            ..PhaseOutcome::default()
        };
        let mut findings = Findings::new();
        check_phase(&outcome, &mut findings);
        assert!(!findings.is_clean());
    }
}
