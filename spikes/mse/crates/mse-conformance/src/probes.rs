// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Measurements shared by several scenarios: reading a picture size out of caps, seeking and watching
//! what the sinks do next, holding still to see whether playback runs on, keeping going after a finding.

use std::time::{Duration, Instant};

use mse_rig::ClockTime;
use mse_rig::frames::{ColourSample, FrameStats};
use mse_rig::rig::Rig;

use crate::support::{Abort, Res, Scn, Watch, astats, ms_between, reset_stats, seek_dones, vstats};

/// The displayed picture size `(width, height)` from a caps string such as
/// `video/x-raw, format=(string)RGB, width=(int)640, height=(int)360`: the coded width times the pixel aspect
/// ratio when the caps carry one (the rig squeezes the picture to 64 pixels wide and keeps the shape in
/// `pixel-aspect-ratio=(fraction)5/1`, so 64 x 180 with a ratio of 5/1 is a 320 x 180 picture).
pub fn caps_size(caps: &str) -> Option<(u32, u32)> {
    fn field(caps: &str, name: &str) -> Option<u32> {
        let start = caps.find(&format!("{name}=(int)"))? + name.len() + 6;
        let digits: String = caps[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    }
    let (width, height) = (field(caps, "width")?, field(caps, "height")?);
    let ratio = caps
        .find("pixel-aspect-ratio=(fraction)")
        .map(|at| &caps[at + "pixel-aspect-ratio=(fraction)".len()..])
        .and_then(|rest| {
            let (numerator, rest) = rest.split_once('/')?;
            let denominator: String = rest.chars().take_while(char::is_ascii_digit).collect();
            Some((
                numerator.trim().parse::<u32>().ok()?,
                denominator.parse::<u32>().ok()?,
            ))
        });
    match ratio {
        Some((numerator, denominator)) if numerator > 0 && denominator > 0 => {
            Some((width * numerator / denominator, height))
        }
        _ => Some((width, height)),
    }
}

/// The picture sizes of a sink's caps changes, in order.
pub fn size_sequence(stats: &FrameStats) -> Vec<Option<(u32, u32)>> {
    stats
        .caps_changes
        .iter()
        .map(|change| caps_size(&change.caps))
        .collect()
}

pub fn fmt_sizes(sizes: &[Option<(u32, u32)>]) -> String {
    sizes
        .iter()
        .map(|size| match size {
            Some((w, h)) => format!("{w}x{h}"),
            None => "?".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// A converted `Abort::Fail` becomes a recorded finding and the scenario goes on; anything else stops it.
/// Use it around one sub-run so that a finding in one does not hide the results of the next.
pub fn keep_going<T>(scn: &mut Scn, what: &str, outcome: Res<T>) -> Res<Option<T>> {
    match outcome {
        Ok(value) => Ok(Some(value)),
        Err(Abort::Fail(message)) => {
            scn.finding(format!("{what}: {message}"));
            Ok(None)
        }
        Err(other) => Err(other.context(what)),
    }
}

/// What the sinks did after a seek.
pub struct SeekObs {
    /// From the `seek()` call to the first video frame inside the window around the target (stale
    /// frames of the old position are outside it); `None` when none came.
    pub first_frame_ms: Option<i64>,
    /// Statistics since just before the seek (stale frames of the old position may be in `count`).
    pub video: FrameStats,
    pub audio: FrameStats,
    pub position: Option<ClockTime>,
    pub new_seek_dones: usize,
    lo: ClockTime,
    hi: ClockTime,
}

impl SeekObs {
    /// Colour samples inside the window around the target (the frames that belong to this seek).
    pub fn fresh_colour(&self) -> Vec<ColourSample> {
        self.video
            .colour
            .iter()
            .copied()
            .filter(|sample| sample.pts >= self.lo && sample.pts <= self.hi)
            .collect()
    }
}

/// Seeks to `target`, waits up to `first_frame_timeout` for the first video frame whose timestamp lies
/// within `[target - before, target + after]`, then lets playback run for `play_for` and returns what
/// the sinks saw. The window keeps frames of the old position (which can still be in flight when the
/// statistics are reset) from being mistaken for the result of the seek.
pub fn seek_and_observe(
    rig: &Rig,
    target: ClockTime,
    before: Duration,
    after: Duration,
    first_frame_timeout: Duration,
    play_for: Duration,
) -> Res<SeekObs> {
    let lo = target.saturating_sub(ClockTime::from_nseconds(
        u64::try_from(before.as_nanos()).unwrap_or(u64::MAX),
    ));
    let hi = target + ClockTime::from_nseconds(u64::try_from(after.as_nanos()).unwrap_or(u64::MAX));
    let condition =
        move |stats: &FrameStats| stats.last_pts.is_some_and(|pts| pts >= lo && pts <= hi);
    reset_stats(rig);
    let before_dones = seek_dones(&rig.log);
    let watch = Watch::start(
        &rig.video,
        first_frame_timeout + Duration::from_secs(1),
        condition,
    );
    let started = Instant::now();
    rig.seek(target)?;
    rig.wait_until(first_frame_timeout, |r| {
        r.video
            .lock()
            .map(|stats| condition(&stats))
            .unwrap_or(false)
    });
    let fired = watch.finish();
    std::thread::sleep(play_for);
    Ok(SeekObs {
        first_frame_ms: fired.map(|at| ms_between(started, at)),
        video: vstats(rig),
        audio: astats(rig),
        position: rig.position(),
        new_seek_dones: seek_dones(&rig.log).saturating_sub(before_dones),
        lo,
        hi,
    })
}

/// How the element behaved while it should have been holding still (no data at the position).
pub struct HoldObs {
    /// The largest position seen.
    pub max_position: Option<ClockTime>,
    /// The timestamp of the last video frame that reached the sink during the hold, if any did.
    pub last_video_pts: Option<ClockTime>,
    /// A frame at or after the `from` given to `hold_and_watch` reached the sink.
    pub frame_at_or_after: bool,
}

/// Watches `rig` for `hold`, sampling the position every 20 ms. `from` is the earliest timestamp of
/// frames that GstMSE must *not* deliver while nothing is buffered at the seek target. The caller resets
/// the statistics before the seek, so stale frames are not in `last_video_pts` unless they are late.
pub fn hold_and_watch(rig: &Rig, hold: Duration, from: ClockTime) -> HoldObs {
    let started = Instant::now();
    let mut max_position: Option<ClockTime> = None;
    while started.elapsed() < hold {
        if let Some(position) = rig.position() {
            max_position = Some(max_position.map_or(position, |m| m.max(position)));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let last_video_pts = vstats(rig).last_pts;
    HoldObs {
        max_position,
        last_video_pts,
        frame_at_or_after: last_video_pts.is_some_and(|pts| pts >= from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::t_ms;
    use mse_rig::frames::CapsChange;

    #[test]
    fn caps_size_reads_width_and_height() {
        let caps = "video/x-raw, format=(string)RGB, width=(int)640, height=(int)360, framerate=(fraction)25/1";
        assert_eq!(caps_size(caps), Some((640, 360)));
        assert_eq!(caps_size("audio/x-raw, rate=(int)48000"), None);
        assert_eq!(caps_size("video/x-raw, width=(int)320"), None);
        assert_eq!(
            caps_size("video/x-raw, height=(int)180, width=(int)320"),
            Some((320, 180))
        );
        // the rig squeezes the picture to 64 pixels wide and keeps its shape in the pixel aspect ratio
        assert_eq!(
            caps_size(
                "video/x-raw, width=(int)64, height=(int)180, pixel-aspect-ratio=(fraction)5/1, framerate=(fraction)25/1"
            ),
            Some((320, 180))
        );
        assert_eq!(
            caps_size(
                "video/x-raw, width=(int)64, height=(int)360, pixel-aspect-ratio=(fraction)10/1"
            ),
            Some((640, 360))
        );
        assert_eq!(
            caps_size(
                "video/x-raw, width=(int)64, height=(int)360, pixel-aspect-ratio=(fraction)1/1"
            ),
            Some((64, 360))
        );
    }

    #[test]
    fn size_sequence_follows_caps_changes() {
        let stats = FrameStats {
            caps_changes: vec![
                CapsChange {
                    at_pts: Some(t_ms(0)),
                    caps: "video/x-raw, width=(int)320, height=(int)180".into(),
                },
                CapsChange {
                    at_pts: Some(t_ms(4_000)),
                    caps: "video/x-raw, width=(int)640, height=(int)360".into(),
                },
                CapsChange {
                    at_pts: None,
                    caps: "video/x-raw".into(),
                },
            ],
            ..FrameStats::default()
        };
        let sizes = size_sequence(&stats);
        assert_eq!(sizes, vec![Some((320, 180)), Some((640, 360)), None]);
        assert_eq!(fmt_sizes(&sizes), "320x180 -> 640x360 -> ?");
    }

    #[test]
    fn keep_going_turns_only_failures_into_findings() {
        let mut scn = test_scn();
        assert_eq!(keep_going(&mut scn, "ok", Ok(3)).ok().flatten(), Some(3));
        assert!(
            keep_going::<u8>(&mut scn, "sub-run", Err(Abort::Fail("hung".into())))
                .is_ok_and(|v| v.is_none())
        );
        assert!(scn.has_failed());
        assert!(matches!(
            keep_going::<u8>(&mut scn, "sub-run", Err(Abort::Error("bug".into()))),
            Err(Abort::Error(m)) if m.starts_with("sub-run")
        ));
    }

    fn test_scn() -> Scn {
        Scn::for_tests()
    }
}
