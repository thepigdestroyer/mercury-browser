// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! C15 (P1): a dash.js-shaped loop. One thread plays like a player: init segments, then each next segment is
//! appended as soon as the previous append of that track has ended (an append is only issued when
//! `updating` is false, which is what an `updateend` handler on the script thread amounts to) while less than
//! 6 s is buffered ahead of the position; everything more than 4 s behind the position is removed in 2 s steps;
//! the video switches rendition at a segment boundary; a seek in the middle; `end_of_stream` after the last
//! append; on the fixtures (`h264-low`/`h264-high`, video only: two source buffers deadlock GstMSE 1.28.9, see C02; 8 s, real time) and on a tiled 64 s stream
//! (fast mode).
//!
//! Part `fixture` (real time, 11 s of media): 0 -> 6 s, remove behind the position, seek back to 3 s (inside
//! what is still buffered), play to the end. Pass when: the stream ends; no stall longer than 200 ms (position
//! not advancing while data is ahead; the first 700 ms after a seek are the seek's own latency, recorded
//! separately); the buffered ranges follow the player's model (after every append the end is where the
//! segments end, after every removal the start is at the removal point or at most one keyframe interval
//! later) and stay within the window; no error event; the picture is the right colour; the last frame is the
//! last of the stream.
//!
//! Part `tiled` (fast mode): 64 s tiled from the fixtures, no seek: plays through with 1600 +- 2 frames, the
//! same window and model checks, no deadlock (the loop has a deadline), no error.

use std::time::{Duration, Instant};

use mse_rig::backend::EosError;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::Rig;
use mse_segments::{Segment, SegmentedStream};

use crate::probes::keep_going;
use crate::support::{
    Res, Scn, add_source_buffer, buffered, diagnostics, errors_in, event_order_problems,
    fast_config, fixture, fmt_opt, judge_colours, ms_of, new_rig, nominal_colour, ranges_json,
    run_scenario, shutdown, t_ms, timed_config, vstats,
};

pub const TITLE: &str = "dash.js-shaped loop: append ahead, remove behind, seek, quality switch, end of stream; fixtures in real time and a tiled 64 s stream";

pub fn run(ctx: &Ctx) -> ScenarioResult {
    run_scenario(ctx, "C15", TITLE, true, body)
}

/// Buffer this far ahead of the position.
const AHEAD_MS: i64 = 6_000;
/// Keep this much behind the position.
const BEHIND_MS: i64 = 4_000;
/// Remove in steps of one segment.
const STEP_MS: i64 = 2_000;
/// A stall is the position not advancing for this long while data is ahead.
const STALL_MS: u64 = 200;
/// The seek's own latency is not a stall.
const SEEK_GRACE: Duration = Duration::from_millis(700);

fn body(ctx: &Ctx, scn: &mut Scn) -> Res<()> {
    let low = fixture("h264-low")?;
    let high = fixture("h264-high")?;
    let outcome = fixture_part(ctx, scn, &low, &high);
    keep_going(scn, "fixture", outcome)?;
    let outcome = tiled_part(ctx, scn, &low);
    keep_going(scn, "tiled", outcome)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Stall detection (pure, so that it can be tested without a clock)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stall {
    /// When the position stopped, since the start of the loop.
    pub at: Duration,
    pub len: Duration,
}

/// Watches the position over time: a stall is the position not advancing for more than `STALL_MS` while data
/// is ahead of it, outside a grace period (the start, a seek).
#[derive(Debug, Default)]
pub struct StallTracker {
    last_pos: Option<i64>,
    last_move: Duration,
    stalled_since: Option<Duration>,
    grace_until: Duration,
    pub episodes: Vec<Stall>,
}

impl StallTracker {
    /// No stall is counted before `until` (and the clock of "not advancing" restarts then); replaces any
    /// earlier grace period.
    pub fn grace(&mut self, until: Duration) {
        self.grace_until = until;
        self.stalled_since = None;
        self.last_pos = None;
    }

    pub fn sample(&mut self, now: Duration, pos_ms: i64, data_ahead: bool) {
        if now < self.grace_until {
            self.last_pos = Some(pos_ms);
            self.last_move = now;
            return;
        }
        match self.last_pos {
            Some(previous) if pos_ms > previous => {
                if let Some(since) = self.stalled_since.take() {
                    self.episodes.push(Stall {
                        at: since,
                        len: now - since,
                    });
                }
                self.last_move = now;
                self.last_pos = Some(pos_ms);
            }
            Some(previous) => {
                // Not advancing (or a position that went backwards by a seek: also start over).
                if pos_ms < previous {
                    self.last_pos = Some(pos_ms);
                    self.last_move = now;
                } else if data_ahead
                    && self.stalled_since.is_none()
                    && now.saturating_sub(self.last_move) > Duration::from_millis(STALL_MS)
                {
                    self.stalled_since = Some(self.last_move);
                }
            }
            None => {
                self.last_pos = Some(pos_ms);
                self.last_move = now;
            }
        }
    }

    /// Closes a stall that is still going on at the end of the run.
    pub fn finish(&mut self, now: Duration) {
        if let Some(since) = self.stalled_since.take() {
            self.episodes.push(Stall {
                at: since,
                len: now - since,
            });
        }
    }

    pub fn max_ms(&self) -> u64 {
        self.episodes
            .iter()
            .map(|e| u64::try_from(e.len.as_millis()).unwrap_or(u64::MAX))
            .max()
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------------------------
// The player
// ---------------------------------------------------------------------------------------------

/// One segment to append, preceded by an init segment when the rendition changes.
struct Step<'a> {
    init: Option<&'a Segment>,
    media: &'a Segment,
}

struct PlayerTrack<'a> {
    id: usize,
    name: &'static str,
    steps: Vec<Step<'a>>,
    next: usize,
    pending_media: Option<&'a Segment>,
    /// Where the player believes the buffered data ends, from the segments it appended.
    end_ms: i64,
    removed_to_ms: i64,
    appends: usize,
    removes: usize,
}

impl PlayerTrack<'_> {
    fn done(&self) -> bool {
        self.next >= self.steps.len() && self.pending_media.is_none()
    }

    /// Issues at most one operation if the track is idle: an append when less than `AHEAD_MS` is
    /// buffered, else a removal when more than `BEHIND_MS` + one step lies behind the position.
    fn tick(&mut self, rig: &Rig, position_ms: i64) -> Res<()> {
        if rig.backend.updating(self.id) {
            return Ok(());
        }
        if let Some(segment) = self.pending_media.take() {
            self.append(rig, segment)?;
            return Ok(());
        }
        if self.next < self.steps.len() && self.end_ms - position_ms < AHEAD_MS {
            let step = &self.steps[self.next];
            let (init, media) = (step.init, step.media);
            self.next += 1;
            match init {
                Some(init) => {
                    rig.backend.append(self.id, &init.bytes)?;
                    self.appends += 1;
                    self.pending_media = Some(media);
                }
                None => self.append(rig, media)?,
            }
            return Ok(());
        }
        let due = (position_ms - BEHIND_MS) / STEP_MS * STEP_MS;
        if due - self.removed_to_ms >= STEP_MS {
            rig.backend.remove(
                self.id,
                t_ms(u64::try_from(self.removed_to_ms).unwrap_or(0)),
                Some(t_ms(u64::try_from(due).unwrap_or(0))),
            )?;
            self.removed_to_ms = due;
            self.removes += 1;
        }
        Ok(())
    }

    fn append(&mut self, rig: &Rig, segment: &Segment) -> Res<()> {
        rig.backend.append(self.id, &segment.bytes)?;
        self.appends += 1;
        if let Some(end) = segment.end_ms() {
            self.end_ms = self.end_ms.max(i64::try_from(end).unwrap_or(i64::MAX));
        }
        Ok(())
    }
}

/// What the player observed about the model of `buffered`.
#[derive(Default)]
struct ModelReport {
    samples: usize,
    violations: Vec<String>,
    max_ahead_ms: i64,
    max_span_ms: i64,
}

impl ModelReport {
    /// With the track idle, `buffered` must follow what the player appended and removed.
    fn sample(&mut self, rig: &Rig, track: &PlayerTrack, position_ms: i64) {
        if rig.backend.updating(track.id) || track.pending_media.is_some() {
            return;
        }
        let Ok(ranges) = rig.backend.buffered(track.id) else {
            return;
        };
        let (Some(first), Some(last)) = (ranges.first(), ranges.last()) else {
            return;
        };
        self.samples += 1;
        let (start, end) = (ms_of(first.start), ms_of(last.end));
        self.max_ahead_ms = self.max_ahead_ms.max(end - position_ms);
        self.max_span_ms = self.max_span_ms.max(end - start);
        let tol = 100;
        if (end - track.end_ms).abs() > tol && self.violations.len() < 6 {
            self.violations.push(format!(
                "{}: buffered ends at {end} ms but the segments appended end at {} ms (position {position_ms} ms)",
                track.name, track.end_ms
            ));
        }
        // after a removal [0, removed_to): the start is there, or up to one keyframe interval later
        if track.removed_to_ms > 0
            && (start < track.removed_to_ms - 60 || start > track.removed_to_ms + STEP_MS + 100)
            && self.violations.len() < 6
        {
            self.violations.push(format!(
                "{}: buffered starts at {start} ms after removing up to {} ms (position {position_ms} ms)",
                track.name, track.removed_to_ms
            ));
        }
    }
}

struct PlayerOutcome {
    video_id: usize,
    eos: bool,
    deadline_hit: bool,
    stall_max_ms: u64,
    stalls: Vec<Stall>,
    startup_ms: Option<u128>,
    seek_resume_ms: Option<u128>,
    model: ModelReport,
    appends: usize,
    removes: usize,
    seeked: bool,
}

struct PlayerPlan {
    /// Seek to this time (ms) once the position passed `seek_when_ms` and a removal has happened.
    seek: Option<(i64, i64)>,
    deadline: Duration,
}

fn run_player(
    scn: &mut Scn,
    rig: &Rig,
    video: &mut PlayerTrack,
    init_video: &Segment,
    plan: &PlayerPlan,
) -> Res<PlayerOutcome> {
    let started = Instant::now();
    rig.append_and_wait(video.id, &init_video.bytes, Duration::from_secs(10))?;
    video.appends += 1;

    let mut stalls = StallTracker::default();
    let mut model = ModelReport::default();
    let mut seeked = false;
    let mut seek_at: Option<(Instant, i64)> = None;
    let mut seek_resume_ms = None;
    let mut startup_ms = None;
    let mut eos_called = false;
    let mut played_at: Option<Instant> = None;
    let mut last_model = Instant::now();
    let mut eos = false;
    let mut deadline_hit = false;
    stalls.grace(Duration::from_secs(3_600)); // until the position first moves

    loop {
        let now = started.elapsed();
        if now > plan.deadline {
            deadline_hit = true;
            break;
        }
        let position = rig.position().map_or(0, ms_of);
        video.tick(rig, position)?;

        // start playing when the first segment of both tracks is in
        if played_at.is_none()
            && video.next >= 1
            && !rig.backend.updating(video.id)
            && video.pending_media.is_none()
        {
            rig.play()?;
            played_at = Some(Instant::now());
        }
        if let Some(at) = played_at
            && startup_ms.is_none()
            && position >= 100
        {
            startup_ms = Some(at.elapsed().as_millis());
            stalls.grace(Duration::ZERO);
        }
        if startup_ms.is_some() {
            let ahead = video.end_ms - position;
            stalls.sample(now, position, ahead > 300);
        }

        // the seek in the middle
        if let Some((when_ms, target_ms)) = plan.seek
            && !seeked
            && position >= when_ms
            && video.removed_to_ms > 0
            && !rig.backend.updating(video.id)
        {
            rig.seek(t_ms(u64::try_from(target_ms).unwrap_or(0)))?;
            seek_at = Some((Instant::now(), target_ms));
            stalls.grace(now + SEEK_GRACE);
            seeked = true;
        }
        if let Some((at, target)) = seek_at
            && seek_resume_ms.is_none()
            && position >= target + 100
        {
            seek_resume_ms = Some(at.elapsed().as_millis());
        }

        if last_model.elapsed() >= Duration::from_millis(100) {
            model.sample(rig, video, position);
            last_model = Instant::now();
        }

        // the end of the stream, once everything is appended (and, with a seek planned, once it is done)
        let all_appended = video.done() && !rig.backend.updating(video.id);
        let seek_ok =
            plan.seek.is_none() || (seeked && position >= plan.seek.map_or(0, |(_, t)| t) + 200);
        if all_appended && seek_ok && !eos_called {
            rig.backend.end_of_stream(EosError::None)?;
            eos_called = true;
        }
        if eos_called && crate::support::eos_seen(&rig.log) {
            eos = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    stalls.finish(started.elapsed());
    scn.note_once(
        "C15 loop: polls the position every 10 ms, issues one operation per idle track per tick",
    );
    Ok(PlayerOutcome {
        video_id: video.id,
        eos,
        deadline_hit,
        stall_max_ms: stalls.max_ms(),
        stalls: stalls.episodes,
        startup_ms,
        seek_resume_ms,
        model,
        appends: video.appends,
        removes: video.removes,
        seeked,
    })
}

fn judge_player(scn: &mut Scn, rig: &Rig, label: &str, outcome: &PlayerOutcome, window_ms: i64) {
    scn.metric(&format!("{label}.appends"), outcome.appends)
        .metric(&format!("{label}.removes"), outcome.removes)
        .metric(&format!("{label}.stall_count"), outcome.stalls.len())
        .metric(&format!("{label}.stall_max_ms"), outcome.stall_max_ms)
        .metric(
            &format!("{label}.startup_ms"),
            outcome.startup_ms.map(|m| m as u64),
        )
        .metric(
            &format!("{label}.seek_resume_ms"),
            outcome.seek_resume_ms.map(|m| m as u64),
        )
        .metric(&format!("{label}.model_samples"), outcome.model.samples)
        .metric(&format!("{label}.max_ahead_ms"), outcome.model.max_ahead_ms)
        .metric(&format!("{label}.max_span_ms"), outcome.model.max_span_ms)
        .metric(
            &format!("{label}.stalls"),
            outcome
                .stalls
                .iter()
                .map(|s| format!("at +{} ms for {} ms", s.at.as_millis(), s.len.as_millis()))
                .collect::<Vec<_>>(),
        );
    scn.check(
        &format!("{label}.finishes"),
        outcome.eos && !outcome.deadline_hit,
        format!(
            "the player loop did not reach the end of the stream (deadline hit: {}, end of stream: {}); {}",
            outcome.deadline_hit,
            outcome.eos,
            diagnostics(rig)
        ),
    );
    scn.check(
        &format!("{label}.no-long-stall"),
        outcome.stall_max_ms <= STALL_MS * 2 || outcome.stalls.is_empty(),
        format!(
            "{} stall(s), the longest {} ms (limit {} ms of the position not advancing with data ahead): {}",
            outcome.stalls.len(),
            outcome.stall_max_ms,
            STALL_MS,
            outcome
                .stalls
                .iter()
                .map(|s| format!("at +{} ms for {} ms", s.at.as_millis(), s.len.as_millis()))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    );
    scn.check(
        &format!("{label}.buffered-follows-the-model"),
        outcome.model.violations.is_empty() && outcome.model.samples > 0,
        format!(
            "{} sample(s): {}",
            outcome.model.samples,
            outcome.model.violations.join("; ")
        ),
    );
    scn.check(
        &format!("{label}.buffered-within-window"),
        outcome.model.max_ahead_ms <= AHEAD_MS + STEP_MS + 500
            && outcome.model.max_span_ms <= window_ms,
        format!(
            "buffered reached {} ms ahead of the position (limit {} ms) and spanned {} ms (limit {} ms)",
            outcome.model.max_ahead_ms,
            AHEAD_MS + STEP_MS + 500,
            outcome.model.max_span_ms,
            window_ms
        ),
    );
    let events = rig.log.snapshot();
    let errors = errors_in(&events);
    scn.check(
        &format!("{label}.no-errors"),
        errors.is_empty(),
        format!("error or abort events: {}", errors.join("; ")),
    );
    let ids: Vec<usize> = vec![outcome.video_id];
    let order = event_order_problems(&events, &ids, false);
    scn.check(
        &format!("{label}.event-order"),
        order.is_empty(),
        order.join("; "),
    );
}

fn steps_of(stream: &SegmentedStream) -> Vec<Step<'_>> {
    stream
        .media
        .iter()
        .map(|media| Step { init: None, media })
        .collect()
}

fn fixture_part(
    ctx: &Ctx,
    scn: &mut Scn,
    low: &SegmentedStream,
    high: &SegmentedStream,
) -> Res<()> {
    let label = "fixture";
    let rig = new_rig(scn, timed_config(ctx))?;
    let video_id = add_source_buffer(&rig, low)?;
    // video: low for segments 0-1, then the rendition switch at the 4 s boundary (init segment, then segments 2-3)
    let mut video_steps = steps_of(low);
    video_steps.truncate(2);
    for (index, media) in high.media.iter().enumerate().skip(2) {
        video_steps.push(Step {
            init: (index == 2).then_some(&high.init),
            media,
        });
    }
    let mut video = PlayerTrack {
        id: video_id,
        name: "video",
        steps: video_steps,
        next: 0,
        pending_media: None,
        end_ms: 0,
        removed_to_ms: 0,
        appends: 0,
        removes: 0,
    };
    let plan = PlayerPlan {
        seek: Some((6_000, 3_000)),
        deadline: Duration::from_secs(40),
    };
    let outcome = run_player(scn, &rig, &mut video, &low.init, &plan)?;
    scn.check(
        &format!("{label}.seek-happened"),
        outcome.seeked && outcome.removes >= 1,
        format!(
            "the player's seek to 3 s after a removal did not happen (seeked {}, video removals {}): the position never reached 6 s",
            outcome.seeked, outcome.removes
        ),
    );
    judge_player(
        scn,
        &rig,
        label,
        &outcome,
        AHEAD_MS + BEHIND_MS + 3 * STEP_MS,
    );
    let v = vstats(&rig);
    let verdict = judge_colours(&v.colour, nominal_colour, t_ms(40));
    scn.metric(&format!("{label}.video_frames"), v.count)
        .metric(&format!("{label}.video_last_pts_ms"), v.last_pts.map(ms_of))
        .metric(
            &format!("{label}.video_caps_changes"),
            v.caps_changes
                .iter()
                .map(|c| c.caps.clone())
                .collect::<Vec<_>>(),
        );
    scn.check(
        &format!("{label}.picture"),
        v.colour.len() >= 4 && verdict.ok(),
        format!("{}: {}", verdict.summary(), verdict.wrong.join("; ")),
    );
    scn.check(
        &format!("{label}.reaches-the-end"),
        v.last_pts.is_some_and(|p| ms_of(p) >= 7_900),
        format!("the last video frame has timestamp {}", fmt_opt(v.last_pts)),
    );
    let ranges = buffered(&rig, video_id)?;
    scn.metric(
        &format!("{label}.final_video_buffered"),
        ranges_json(&ranges),
    );
    shutdown(scn, "c15-fixture", rig);
    Ok(())
}

fn tiled_part(ctx: &Ctx, scn: &mut Scn, low: &SegmentedStream) -> Res<()> {
    let label = "tiled";
    let tiled_video = mse_segments::tile(low, 8)?;
    let rig = new_rig(scn, fast_config(ctx))?;
    let video_id = add_source_buffer(&rig, &tiled_video)?;
    let mut video = PlayerTrack {
        id: video_id,
        name: "video",
        steps: steps_of(&tiled_video),
        next: 0,
        pending_media: None,
        end_ms: 0,
        removed_to_ms: 0,
        appends: 0,
        removes: 0,
    };
    let plan = PlayerPlan {
        seek: None,
        deadline: Duration::from_secs(120),
    };
    let outcome = run_player(scn, &rig, &mut video, &tiled_video.init, &plan)?;
    judge_player(
        scn,
        &rig,
        label,
        &outcome,
        AHEAD_MS + BEHIND_MS + 3 * STEP_MS,
    );
    let v = vstats(&rig);
    scn.metric(&format!("{label}.video_frames"), v.count);
    scn.check(
        &format!("{label}.frames"),
        v.count.abs_diff(1_600) <= 2,
        format!(
            "{} video frames reached the sink, expected 1600 +- 2 for 64 s at 25 fps; {}",
            v.count,
            diagnostics(&rig)
        ),
    );
    shutdown(scn, "c15-tiled", rig);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn steady_playback_has_no_stall() {
        let mut tracker = StallTracker::default();
        for tick in 0..100u64 {
            tracker.sample(ms(tick * 20), (tick * 20) as i64, true);
        }
        tracker.finish(ms(2_000));
        assert!(tracker.episodes.is_empty());
        assert_eq!(tracker.max_ms(), 0);
    }

    #[test]
    fn a_freeze_with_data_ahead_is_one_stall_of_its_length() {
        let mut tracker = StallTracker::default();
        let mut pos = 0i64;
        for tick in 0..200u64 {
            let now = tick * 20;
            // frozen between 1000 ms and 1600 ms
            if !(1_000..1_600).contains(&now) {
                pos += 20;
            }
            tracker.sample(ms(now), pos, true);
        }
        tracker.finish(ms(4_000));
        assert_eq!(tracker.episodes.len(), 1);
        let stall = tracker.episodes[0];
        assert!(stall.at >= ms(960) && stall.at <= ms(1_040), "{stall:?}");
        assert!(stall.len >= ms(560) && stall.len <= ms(700), "{stall:?}");
        assert!(tracker.max_ms() >= 560);
    }

    #[test]
    fn a_short_pause_and_a_pause_without_data_are_not_stalls() {
        let mut tracker = StallTracker::default();
        let mut pos = 0i64;
        for tick in 0..100u64 {
            let now = tick * 20;
            // 100 ms freeze: below the limit
            if !(500..600).contains(&now) {
                pos += 20;
            }
            tracker.sample(ms(now), pos, true);
        }
        assert!(tracker.episodes.is_empty());
        // 1 s freeze, but nothing is buffered ahead: the end of the data, not a stall
        let mut end = StallTracker::default();
        for tick in 0..100u64 {
            end.sample(ms(tick * 20), 500, false);
        }
        end.finish(ms(2_000));
        assert!(end.episodes.is_empty());
    }

    #[test]
    fn grace_hides_the_seek_latency() {
        let mut tracker = StallTracker::default();
        tracker.sample(ms(0), 100, true);
        tracker.sample(ms(20), 120, true);
        tracker.grace(ms(720));
        for tick in 1..40u64 {
            tracker.sample(ms(20 + tick * 20), 3_000, true); // frozen at the target inside the grace
        }
        tracker.finish(ms(820));
        assert!(tracker.episodes.is_empty(), "{:?}", tracker.episodes);
        // after the grace, a freeze counts
        tracker.sample(ms(1_000), 3_000, true);
        tracker.sample(ms(1_400), 3_000, true);
        tracker.finish(ms(1_400));
        assert_eq!(tracker.episodes.len(), 1);
    }

    #[test]
    fn a_position_that_goes_backwards_restarts_the_clock() {
        let mut tracker = StallTracker::default();
        tracker.sample(ms(0), 6_000, true);
        tracker.sample(ms(20), 6_020, true);
        tracker.sample(ms(40), 3_000, true); // a seek back
        tracker.sample(ms(60), 3_020, true);
        tracker.finish(ms(80));
        assert!(tracker.episodes.is_empty());
    }
}
