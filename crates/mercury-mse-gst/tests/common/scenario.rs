// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Measurements the scenarios share: a seek and what came after it, frame gaps, colour runs.

use std::time::{Duration, Instant};

use super::rig::Rig;
use super::sinks::{Colour, Recorded, classify};

/// What one flushing seek did, measured at the sinks.
#[derive(Debug, Clone)]
pub struct SeekOutcome {
    /// Time from the call to the first video frame after the flush.
    pub first_video: Option<Duration>,
    pub first_video_pts: Option<u64>,
    pub first_video_colour: Option<Colour>,
    pub first_audio: Option<Duration>,
    pub first_audio_pts: Option<u64>,
    /// `async-done` messages the seek caused (the pipeline's "seek done": one per seek).
    pub async_done: u32,
}

/// Seeks to `target` (ns) and waits for the first frame after the flush (and the first audio buffer when
/// the rig has an audio sink); panics when nothing comes within `timeout`.
pub fn seek_and_measure(rig: &Rig, target: u64, accurate: bool, timeout: Duration) -> SeekOutcome {
    rig.pump_bus();
    let async_before = rig.bus_log.lock().unwrap().async_done;
    let video_flushes = rig.video.as_ref().map(|s| s.stats.lock().unwrap().flushes);
    let audio_flushes = rig.audio.as_ref().map(|s| s.stats.lock().unwrap().flushes);
    let started = Instant::now();
    assert!(
        rig.seek(target, accurate),
        "the pipeline refused a seek to {target} ns"
    );

    let first_after = |sink: &Option<super::sinks::Sink>,
                       flushes: Option<u32>|
     -> Option<(Duration, u64, Option<Colour>)> {
        let stats = sink.as_ref()?.stats.lock().unwrap();
        let k = flushes? as usize;
        let mark = *stats.flush_marks.get(k)?;
        let arrival = stats.arrivals.get(mark)?;
        Some((
            arrival.at.saturating_sub(started.duration_since(rig.t0)),
            arrival.pts.unwrap_or(0),
            arrival.rgb.map(classify),
        ))
    };
    rig.wait_until(
        timeout,
        &format!("the first frame after a seek to {target} ns"),
        |_| {
            (rig.video.is_none() || first_after(&rig.video, video_flushes).is_some())
                && (rig.audio.is_none() || first_after(&rig.audio, audio_flushes).is_some())
        },
    );
    let video = first_after(&rig.video, video_flushes);
    let audio = first_after(&rig.audio, audio_flushes);
    rig.pump_bus();
    SeekOutcome {
        first_video: video.map(|v| v.0),
        first_video_pts: video.map(|v| v.1),
        first_video_colour: video.and_then(|v| v.2),
        first_audio: audio.map(|a| a.0),
        first_audio_pts: audio.map(|a| a.1),
        async_done: rig.bus_log.lock().unwrap().async_done - async_before,
    }
}

/// The largest difference between the timestamps of consecutive buffers, ns.
pub fn max_pts_gap(stats: &Recorded, from: usize) -> u64 {
    stats.arrivals[from.min(stats.arrivals.len())..]
        .windows(2)
        .filter_map(|w| Some(w[1].pts?.saturating_sub(w[0].pts?)))
        .max()
        .unwrap_or(0)
}

/// Whether the video timestamps from `from` on are strictly increasing.
pub fn is_increasing(stats: &Recorded, from: usize) -> bool {
    stats.arrivals[from.min(stats.arrivals.len())..]
        .windows(2)
        .all(|w| w[1].pts > w[0].pts)
}

/// The places where consecutive timestamps are further apart than `limit` ns: `(index, previous, next)`.
pub fn gaps_over(stats: &Recorded, limit: u64) -> Vec<(usize, u64, u64)> {
    stats
        .arrivals
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| {
            let (a, b) = (w[0].pts?, w[1].pts?);
            (b.saturating_sub(a) > limit).then_some((i + 1, a, b))
        })
        .collect()
}
