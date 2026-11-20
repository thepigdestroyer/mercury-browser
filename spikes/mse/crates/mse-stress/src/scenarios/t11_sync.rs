// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T11: clock, A/V sync and drops. Real time (`sync=true`): 8 s of `h264-high` + `aac`, the
//! arrival-time offset at the sinks between audio and video of equal timestamp, and the QoS counts
//! (the data behind `getVideoPlaybackQuality()`) at 640x360 and at a synthetic 1080p H.264 stream.

use std::time::Duration;

use mse_rig::backend::EosError;
use mse_rig::frames::Arrival;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::RigConfig;
use serde_json::json;

use crate::common::{
    APPEND_TIMEOUT, Findings, Snapshot, Verdict, add_buffers, audio_enabled, new_rig,
    realtime_config,
};
use crate::stats::{ms, summary_json};
use crate::synth::{SynthCodec, SynthError, SynthSpec, make_stream};

/// The brief's limit on the A/V offset at the sinks.
pub const MAX_OFFSET_MS: f64 = 80.0;

pub fn run(ctx: &Ctx) -> ScenarioResult {
    crate::common::run_isolated(
        "T11",
        "Clock, A/V sync and drops",
        ctx,
        "MSE_STRESS_T11_NESTED",
        std::time::Duration::from_secs(120),
        body,
    )
}

/// One buffer's timestamp (ns) and arrival (ms on the rig clock).
pub type Point = (u64, f64);

pub fn points(arrivals: &[Arrival]) -> Vec<Point> {
    let mut points: Vec<Point> = arrivals
        .iter()
        .filter_map(|a| Some((a.pts?.nseconds(), a.at.as_secs_f64() * 1000.0)))
        .collect();
    points.sort_by_key(|p| p.0);
    points
}

/// The arrival time (ms) the `curve` gives at timestamp `pts`, by linear interpolation between the two
/// buffers around it; `None` outside the curve's range.
pub fn arrival_at(curve: &[Point], pts: u64) -> Option<f64> {
    let first = curve.first()?;
    let last = curve.last()?;
    if pts < first.0 || pts > last.0 {
        return None;
    }
    let upper = curve.partition_point(|p| p.0 < pts);
    let (high_pts, high_at) = curve[upper.min(curve.len() - 1)];
    if high_pts == pts || upper == 0 {
        return Some(high_at);
    }
    let (low_pts, low_at) = curve[upper - 1];
    if high_pts == low_pts {
        return Some(low_at);
    }
    let fraction = (pts - low_pts) as f64 / (high_pts - low_pts) as f64;
    Some(low_at + (high_at - low_at) * fraction)
}

/// For each video buffer: its arrival minus the audio's (interpolated) arrival at the same timestamp, in ms.
/// Positive means the picture arrived later than the sound of the same instant.
pub fn av_offsets(video: &[Point], audio: &[Point]) -> Vec<f64> {
    video
        .iter()
        .filter_map(|(pts, at)| Some(at - arrival_at(audio, *pts)?))
        .collect()
}

/// How late each buffer arrived against an ideal real-time clock fitted to the first one (ms; negative is early).
pub fn lateness(curve: &[Point]) -> Vec<f64> {
    let Some(first) = curve.first() else {
        return Vec::new();
    };
    curve
        .iter()
        .map(|(pts, at)| at - first.1 - (*pts as f64 - first.0 as f64) / 1e6)
        .collect()
}

fn body(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let mut findings = Findings::new();
    small_picture(ctx, result, &mut findings)?;
    full_hd(ctx, result, &mut findings)?;
    Ok(findings.verdict())
}

fn small_picture(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    findings: &mut Findings,
) -> Result<(), String> {
    // Split A/V (two source buffers) when the rig can take it; with MSE_STRESS_TRACKS=video (two source buffers
    // deadlock GstMSE 1.28.9) one source buffer that holds both tracks (the muxed fixture, 320x180) is used, so
    // that the A/V offset can still be measured.
    let split = audio_enabled();
    let (video, audio) = if split {
        (
            crate::common::fixture("h264-high")?,
            Some(crate::common::fixture("aac")?),
        )
    } else {
        (crate::common::fixture("h264-aac")?, None)
    };
    let config = RigConfig {
        audio: true,
        ..realtime_config(ctx)
    };
    let rig = new_rig(config)?;
    let ids = match &audio {
        Some(audio) => add_buffers(&rig, &video.mime, &audio.mime)?,
        None => vec![
            rig.backend
                .add_source_buffer(&video.mime)
                .map_err(|e| format!("add_source_buffer(muxed): {e:?}"))?,
        ],
    };
    let streams: Vec<&mse_segments::SegmentedStream> =
        std::iter::once(&video).chain(audio.as_ref()).collect();
    result.metric(
        "small_picture_source",
        if split {
            "split: h264-high and aac in two source buffers"
        } else {
            "muxed: h264-aac in one source buffer"
        },
    );
    for (id, stream) in ids.iter().zip(&streams) {
        rig.append_and_wait(*id, &stream.init.bytes, APPEND_TIMEOUT)
            .map_err(|e| format!("init: {e}"))?;
    }
    for index in 0..video.media.len() {
        for (id, stream) in ids.iter().copied().zip(streams.iter().copied()) {
            if let Some(segment) = stream.media.get(index) {
                rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT)
                    .map_err(|e| format!("append: {e}"))?;
            }
        }
    }
    let _ = rig.backend.end_of_stream(EosError::None);
    let started = std::time::Instant::now();
    rig.play().map_err(|e| format!("play: {e}"))?;
    let eos = rig.wait_for_eos(Duration::from_secs(25));
    let wall = started.elapsed();
    std::thread::sleep(Duration::from_millis(300));
    let video_points = points(&rig.video.lock().unwrap_or_else(|e| e.into_inner()).arrivals);
    let audio_points = points(&rig.audio.lock().unwrap_or_else(|e| e.into_inner()).arrivals);
    let offsets = av_offsets(&video_points, &audio_points);
    let absolute: Vec<f64> = offsets.iter().map(|o| o.abs()).collect();
    let late = lateness(&video_points);
    let qos = rig.qos();
    let video_snapshot = Snapshot::read(&rig.video);
    let audio_snapshot = Snapshot::read(&rig.audio);
    video_snapshot.record(result, "small_video");
    audio_snapshot.record(result, "small_audio");
    let max_abs = crate::stats::max(&absolute);
    result.metric("small_av_offset_ms", summary_json(&offsets));
    result.metric("small_av_offset_abs_ms", summary_json(&absolute));
    result.metric("small_video_lateness_ms", summary_json(&late));
    result.metric(
        "small_qos",
        json!({ "processed": qos.processed, "dropped": qos.dropped }),
    );
    result.metric("small_playback_wall_ms", ms(wall));
    result.metric("small_eos_arrived", eos);
    result.note(format!(
        "small picture + AAC in real time ({}): {} video frames and {} audio buffers; {} compared pairs; |A/V offset| max {} ms, mean {} ms; played in {:.0} ms of wall time for {:?} ms of media; QoS processed {} dropped {}",
        video
            .video_size()
            .map_or("unknown size".to_owned(), |(w, h)| format!("{w}x{h}")),
        video_snapshot.count,
        audio_snapshot.count,
        offsets.len(),
        max_abs.map_or("n/a".to_owned(), |v| format!("{v:.1}")),
        crate::stats::mean(&absolute).map_or("n/a".to_owned(), |v| format!("{v:.1}")),
        ms(wall),
        video_snapshot.span_ms(),
        qos.processed,
        qos.dropped
    ));
    findings.check(
        eos,
        "small picture: the stream never reached its end in 25 s of real time",
    );
    findings.check(
        !offsets.is_empty(),
        "no video buffer could be compared with an audio buffer (no timestamps at the sinks?)",
    );
    if let Some(max_abs) = max_abs {
        findings.check(
            max_abs < MAX_OFFSET_MS,
            format!(
                "small picture: the A/V arrival offset reached {max_abs:.1} ms (limit {MAX_OFFSET_MS} ms)"
            ),
        );
    }
    // Real time means the whole clip took about as long as the clip.
    let media_ms = video_snapshot.span_ms().unwrap_or(0.0);
    findings.check(
        media_ms == 0.0 || (ms(wall) > media_ms * 0.8 && ms(wall) < media_ms * 1.6 + 2000.0),
        format!("small picture: {:.0} ms of media took {:.0} ms of wall time: the sinks did not play in real time", media_ms, ms(wall)),
    );
    let _ = rig.shutdown();
    Ok(())
}

fn full_hd(ctx: &Ctx, result: &mut ScenarioResult, findings: &mut Findings) -> Result<(), String> {
    let spec = SynthSpec::full_hd(SynthCodec::H264, 10);
    let (stream, _) = match make_stream(&spec, &ctx.scratch) {
        Ok(made) => made,
        Err(SynthError::Missing(why) | SynthError::Failed(why)) => {
            result.metric(
                "full_hd_qos",
                json!({ "measured": false, "reason": why.clone() }),
            );
            result.note(format!("1080p H.264 in real time not measured: {why}"));
            return Ok(());
        }
    };
    let config = RigConfig {
        audio: false,
        ..realtime_config(ctx)
    };
    let rig = new_rig(config)?;
    let id = rig
        .backend
        .add_source_buffer(&stream.mime)
        .map_err(|e| format!("add_source_buffer({}): {e:?}", stream.mime))?;
    rig.append_and_wait(id, &stream.init.bytes, APPEND_TIMEOUT)
        .map_err(|e| format!("init: {e}"))?;
    for segment in &stream.media {
        rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT)
            .map_err(|e| format!("append: {e}"))?;
    }
    let _ = rig.backend.end_of_stream(EosError::None);
    let started = std::time::Instant::now();
    rig.play().map_err(|e| format!("play: {e}"))?;
    let eos = rig.wait_for_eos(Duration::from_secs(30));
    let wall = started.elapsed();
    std::thread::sleep(Duration::from_millis(300));
    let qos = rig.qos();
    let video_points = points(&rig.video.lock().unwrap_or_else(|e| e.into_inner()).arrivals);
    let late = lateness(&video_points);
    let snapshot = Snapshot::read(&rig.video);
    snapshot.record(result, "full_hd_video");
    result.metric(
        "full_hd_qos",
        json!({ "measured": true, "processed": qos.processed, "dropped": qos.dropped }),
    );
    result.metric("full_hd_lateness_ms", summary_json(&late));
    result.metric("full_hd_playback_wall_ms", ms(wall));
    result.metric("full_hd_decoders", json!(rig.chosen_decoders()));
    result.note(format!(
        "1080p H.264 in real time: {} of {} frames arrived, QoS processed {} dropped {}; lateness against an ideal clock: p95 {} ms, max {} ms; played in {:.0} ms for {:?} ms of media; decoders {:?}",
        snapshot.count,
        spec.frames(),
        qos.processed,
        qos.dropped,
        crate::stats::percentile(&late, 95.0).map_or("n/a".to_owned(), |v| format!("{v:.1}")),
        crate::stats::max(&late).map_or("n/a".to_owned(), |v| format!("{v:.1}")),
        ms(wall),
        snapshot.span_ms(),
        rig.chosen_decoders()
    ));
    findings.check(
        eos,
        "1080p: the stream never reached its end in 30 s of real time",
    );
    let _ = rig.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(items: &[(u64, f64)]) -> Vec<Point> {
        items.to_vec()
    }

    #[test]
    fn arrival_is_interpolated_between_buffers() {
        let audio = curve(&[(0, 0.0), (20_000_000, 20.0), (40_000_000, 40.0)]);
        assert_eq!(arrival_at(&audio, 0), Some(0.0));
        assert_eq!(arrival_at(&audio, 10_000_000), Some(10.0));
        assert_eq!(arrival_at(&audio, 40_000_000), Some(40.0));
        assert_eq!(arrival_at(&audio, 50_000_000), None);
        assert_eq!(arrival_at(&[], 1), None);
    }

    #[test]
    fn identical_clocks_give_zero_offset() {
        let audio = curve(&[
            (0, 5.0),
            (20_000_000, 25.0),
            (40_000_000, 45.0),
            (60_000_000, 65.0),
        ]);
        let video = curve(&[(0, 5.0), (40_000_000, 45.0)]);
        let offsets = av_offsets(&video, &audio);
        assert_eq!(offsets, vec![0.0, 0.0]);
    }

    #[test]
    fn a_late_picture_has_a_positive_offset() {
        let audio = curve(&[(0, 0.0), (20_000_000, 20.0), (40_000_000, 40.0)]);
        let video = curve(&[(20_000_000, 70.0)]);
        assert_eq!(av_offsets(&video, &audio), vec![50.0]);
    }

    #[test]
    fn video_beyond_the_audio_is_not_compared() {
        let audio = curve(&[(0, 0.0), (20_000_000, 20.0)]);
        let video = curve(&[(0, 0.0), (100_000_000, 100.0)]);
        assert_eq!(av_offsets(&video, &audio).len(), 1);
    }

    #[test]
    fn lateness_is_measured_against_the_first_buffer() {
        let curve = curve(&[
            (1_000_000_000, 100.0),
            (1_040_000_000, 140.0),
            (1_080_000_000, 190.0),
        ]);
        assert_eq!(lateness(&curve), vec![0.0, 0.0, 10.0]);
        assert!(lateness(&[]).is_empty());
    }

    #[test]
    fn the_limit_is_the_briefs() {
        assert_eq!(MAX_OFFSET_MS, 80.0);
    }
}
