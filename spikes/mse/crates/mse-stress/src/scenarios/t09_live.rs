// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T09: a live-style stream. 60 s of media: a 2 s segment appended about every 2 s of media time (as
//! fast as possible, not in real time), the live seekable range sliding, old data removed, the
//! duration unknown; then a `Sequence`-mode source buffer fed the same segments three times.
//! Playback must continue, memory stay flat and the ranges be as set.

use std::time::Duration;

use mse_rig::ClockTime;
use mse_rig::backend::{AppendMode, EosError};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_segments::tile;
use serde_json::json;

use crate::common::{
    APPEND_TIMEOUT, Findings, Verdict, add_buffers, fast_config, fixture, new_rig, settled_ranges,
    video_frames, wait_buffered_end,
};
use crate::proc;
use crate::stats::slope;

/// Seconds of media in the live part (8 copies of the 8 s fixtures).
pub const LIVE_COPIES: usize = 8;

/// How much data the live window keeps behind the newest segment.
pub const KEEP_SECONDS: f64 = 10.0;

/// The live seekable range spans this many seconds behind the newest segment.
pub const SEEKABLE_SECONDS: f64 = 30.0;

/// RSS growth over the second half of the run that still counts as flat (kilobytes).
pub const FLAT_RSS_KB: f64 = 20.0 * 1024.0;

pub fn run(ctx: &Ctx) -> ScenarioResult {
    crate::common::run_isolated(
        "T09",
        "Live-style stream",
        ctx,
        "MSE_STRESS_T09_NESTED",
        std::time::Duration::from_secs(90),
        body,
    )
}

/// Whether `start` (seconds) is where a removal up to `end - keep` can leave the buffered range:
/// removal runs on to the next key frame, which is at most one GOP (2 s) later.
pub fn start_is_plausible(start: f64, end: f64, keep: f64) -> bool {
    let wanted = end - keep;
    start >= wanted - 0.2 && start <= wanted + 2.2
}

fn body(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let mut findings = Findings::new();
    live_window(ctx, result, &mut findings)?;
    sequence_mode(ctx, result, &mut findings)?;
    Ok(findings.verdict())
}

fn live_window(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    findings: &mut Findings,
) -> Result<(), String> {
    let video = fixture("h264-low")?;
    let audio = fixture("aac")?;
    let video_tiled = tile(&video, LIVE_COPIES).map_err(|e| format!("tile: {e}"))?;
    let audio_tiled = tile(&audio, LIVE_COPIES).map_err(|e| format!("tile: {e}"))?;
    let rig = new_rig(fast_config(ctx))?;
    let ids = add_buffers(&rig, &video.mime, &audio.mime)?;
    let vid = ids[0];
    let duration_call = rig.backend.set_duration(None);
    result.metric("live_set_duration_none", format!("{duration_call:?}"));
    for (id, init) in ids.iter().zip([&video.init, &audio.init]) {
        rig.append_and_wait(*id, &init.bytes, APPEND_TIMEOUT)
            .map_err(|e| format!("init: {e}"))?;
    }
    result.metric(
        "live_element_duration_ms_after_init",
        rig.element_duration().map(|d| d.mseconds()),
    );

    let mut seekable_errors = Vec::new();
    let mut remove_errors = Vec::new();
    let mut rss_points: Vec<(f64, f64)> = Vec::new();
    let segments = video_tiled.media.len().max(audio_tiled.media.len());
    let mut end_s = 0.0;
    let mut last_range = None;
    for index in 0..segments {
        for (id, stream) in ids.iter().copied().zip([&video_tiled, &audio_tiled]) {
            if let Some(segment) = stream.media.get(index) {
                rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT)
                    .map_err(|e| format!("append {index}: {e}"))?;
                if let Some(segment_end) = segment.end_ms() {
                    end_s = f64::max(end_s, segment_end as f64 / 1000.0);
                }
            }
        }
        wait_buffered_end(&rig, vid, end_s, Duration::from_secs(2));
        if index == 1 {
            let _ = rig.play();
        }
        let start = (end_s - SEEKABLE_SECONDS).max(0.0);
        let to_time = |s: f64| ClockTime::from_nseconds((s * 1e9) as u64);
        match rig
            .backend
            .set_live_seekable_range(to_time(start), to_time(end_s))
        {
            Ok(()) => last_range = Some((start, end_s)),
            Err(error) => {
                if seekable_errors.len() < 3 {
                    seekable_errors.push(format!("{error:?}"));
                }
            }
        }
        if end_s > KEEP_SECONDS {
            for id in ids.iter().copied() {
                if let Err(error) =
                    rig.backend
                        .remove(id, ClockTime::ZERO, Some(to_time(end_s - KEEP_SECONDS)))
                {
                    if remove_errors.len() < 3 {
                        remove_errors.push(format!("{error:?}"));
                    }
                } else {
                    rig.wait_until(Duration::from_secs(10), |rig| !rig.backend.updating(id));
                }
            }
        }
        if index % 4 == 3
            && let Ok(rss) = proc::rss_kb()
        {
            rss_points.push((end_s, rss as f64));
        }
    }
    let _ = rig.backend.end_of_stream(EosError::None);
    let expected_frames = (end_s * 25.0) as u64;
    let reached = rig.wait_until(Duration::from_secs(60), |rig| {
        video_frames(rig) + 25 >= expected_frames
    });
    std::thread::sleep(Duration::from_millis(300));
    let frames = video_frames(&rig);
    let buffered = rig
        .backend
        .buffered(vid)
        .map_err(|e| format!("buffered: {e:?}"))?;
    let first = buffered.first().map(|r| r.start.nseconds() as f64 / 1e9);
    let last = buffered.last().map(|r| r.end.nseconds() as f64 / 1e9);
    let seekable = rig.backend.live_seekable_range();
    let second_half: Vec<(f64, f64)> = rss_points
        .iter()
        .copied()
        .filter(|p| p.0 >= end_s / 2.0)
        .collect();
    let growth = match (second_half.first(), second_half.last()) {
        (Some(a), Some(b)) => b.1 - a.1,
        _ => 0.0,
    };
    let curve_slope = slope(
        &rss_points.iter().map(|p| p.0).collect::<Vec<_>>(),
        &rss_points.iter().map(|p| p.1 / 1024.0).collect::<Vec<_>>(),
    );
    result.metric(
        "live_window",
        json!({
            "media_seconds": end_s,
            "video_frames": frames,
            "expected_frames": expected_frames,
            "buffered_video": buffered.iter().map(|r| json!([r.start.nseconds() as f64 / 1e9, r.end.nseconds() as f64 / 1e9])).collect::<Vec<_>>(),
            "live_seekable_range_set_last": last_range,
            "live_seekable_range_read_back": [seekable.start.nseconds() as f64 / 1e9, seekable.end.nseconds() as f64 / 1e9],
            "set_live_seekable_range_errors": seekable_errors,
            "remove_errors": remove_errors,
            "rss_points_mb": rss_points.iter().map(|p| json!([p.0, (p.1 / 1024.0 * 10.0).round() / 10.0])).collect::<Vec<_>>(),
            "rss_growth_second_half_mb": growth / 1024.0,
            "rss_mb_per_media_second": curve_slope,
            "element_duration_ms_at_end": rig.element_duration().map(|d| d.mseconds()),
        }),
    );
    result.note(format!(
        "live window: {end_s:.0} s of media appended segment by segment; {frames} of about {expected_frames} video frames played; buffered {:?}; live seekable range set {last_range:?}, read back [{:.1}, {:.1}] s; RSS growth over the second half {:.1} MB; set_duration(None) gave {duration_call:?}",
        buffered.iter().map(|r| (r.start.nseconds() as f64 / 1e9, r.end.nseconds() as f64 / 1e9)).collect::<Vec<_>>(),
        seekable.start.nseconds() as f64 / 1e9,
        seekable.end.nseconds() as f64 / 1e9,
        growth / 1024.0
    ));
    findings.check(
        reached,
        format!(
            "live window: only {frames} of {expected_frames} video frames played: playback stopped"
        ),
    );
    findings.check(
        seekable_errors.is_empty(),
        format!("live window: set_live_seekable_range failed: {seekable_errors:?}"),
    );
    findings.check(
        remove_errors.is_empty(),
        format!("live window: remove failed: {remove_errors:?}"),
    );
    findings.check(growth < FLAT_RSS_KB, format!("live window: RSS grew by {:.1} MB over the second half although old data was removed (limit {:.0} MB)", growth / 1024.0, FLAT_RSS_KB / 1024.0));
    match (first, last) {
        (Some(first), Some(last)) => {
            findings.check(
                start_is_plausible(first, last, KEEP_SECONDS),
                format!("live window: buffered is [{first:.1}, {last:.1}] s, expected a start about {KEEP_SECONDS:.0} s behind the end (up to one GOP later)"),
            );
        }
        _ => findings.fail("live window: buffered() is empty at the end"),
    }
    if let Some((start, end)) = last_range {
        let read_start = seekable.start.nseconds() as f64 / 1e9;
        let read_end = seekable.end.nseconds() as f64 / 1e9;
        findings.check(
            (read_start - start).abs() < 0.01 && (read_end - end).abs() < 0.01,
            format!("live window: the live seekable range was set to [{start:.1}, {end:.1}] but reads back as [{read_start:.1}, {read_end:.1}]"),
        );
    }
    let _ = rig.shutdown();
    Ok(())
}

fn sequence_mode(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    findings: &mut Findings,
) -> Result<(), String> {
    let video = fixture("h264-low")?;
    let rig = new_rig(fast_config(ctx))?;
    let vid = rig
        .backend
        .add_source_buffer(&video.mime)
        .map_err(|e| format!("add_source_buffer: {e:?}"))?;
    let mode = rig.backend.set_append_mode(vid, AppendMode::Sequence);
    result.metric("sequence_set_append_mode", format!("{mode:?}"));
    if mode.is_err() {
        findings.fail(format!(
            "sequence mode: set_append_mode(Sequence) was refused: {mode:?}"
        ));
        let _ = rig.shutdown();
        return Ok(());
    }
    let mut problems = Vec::new();
    for round in 0..3 {
        // Every round starts with an initialisation segment, which begins a new coded frame group.
        if let Err(error) = rig.append_and_wait(vid, &video.init.bytes, APPEND_TIMEOUT) {
            problems.push(format!("round {round}: init segment: {error}"));
            break;
        }
        for segment in &video.media {
            if let Err(error) = rig.append_and_wait(vid, &segment.bytes, APPEND_TIMEOUT) {
                problems.push(format!("round {round}: media segment: {error}"));
                break;
            }
        }
    }
    // UpdateEnd comes before the data is in `buffered`: read the ranges once they have stopped changing.
    let as_seconds = settled_ranges(
        &rig,
        vid,
        Duration::from_millis(150),
        Duration::from_secs(4),
    );
    let offset = rig.backend.timestamp_offset(vid);
    result.metric(
        "sequence_mode",
        json!({
            "appended": "init + 4 segments, three times, the same timestamps each time",
            "buffered_s": as_seconds,
            "timestamp_offset": format!("{offset:?}"),
            "problems": problems,
        }),
    );
    result.note(format!(
        "sequence mode: init + 4 segments appended three times with identical timestamps; buffered {as_seconds:?} (the spec's sequence mode chains them to [0, 24] s); timestamp offset {offset:?}"
    ));
    findings.check(problems.is_empty(), format!("sequence mode: {problems:?}"));
    let chained =
        as_seconds.len() == 1 && as_seconds[0].0 <= 0.2 && (as_seconds[0].1 - 24.0).abs() <= 0.3;
    findings.check(
        chained,
        format!("sequence mode: buffered is {as_seconds:?}, expected one range [0, 24] s (three copies chained end to end)"),
    );
    let _ = rig.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_of_the_window_may_run_on_to_the_next_key_frame() {
        assert!(start_is_plausible(50.0, 60.0, 10.0));
        assert!(start_is_plausible(52.0, 60.0, 10.0));
        assert!(!start_is_plausible(0.0, 60.0, 10.0));
        assert!(!start_is_plausible(53.0, 60.0, 10.0));
    }

    #[test]
    fn the_live_window_is_sixty_four_seconds() {
        assert_eq!(LIVE_COPIES * 8, 64);
    }
}
