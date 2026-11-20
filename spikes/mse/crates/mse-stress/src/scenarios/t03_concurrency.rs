// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T03: concurrent callers. Four threads appending to four source buffers; two threads appending to
//! one (the second must get `InvalidState`); a thread polling `buffered` and `updating` every
//! millisecond during appends and playback; `remove` during playback. No crash, no corruption (the
//! frame counts of C02), and every refusal is the documented error.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use mse_rig::ClockTime;
use mse_rig::backend::{EosError, MseError};
use mse_rig::events::{MseEvent, SourceBufferId};
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::RigConfig;
use mse_segments::{SegmentedStream, tile};
use serde_json::json;

use crate::common::{
    APPEND_TIMEOUT, FIXTURE_FRAMES, Findings, Snapshot, Verdict, fast_config, fixture, frames_ok,
    max_update_overlap, new_rig, realtime_config, video_frames,
};
use crate::common::{ManagedRig, add_buffers};
use crate::stats::ms;

pub fn run(ctx: &Ctx) -> ScenarioResult {
    crate::common::run_isolated(
        "T03",
        "Concurrent callers",
        ctx,
        "MSE_STRESS_T03_NESTED",
        std::time::Duration::from_secs(120),
        body,
    )
}

fn body(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let mut findings = Findings::new();
    four_threads(ctx, result, &mut findings)?;
    two_threads_one_buffer(ctx, result, &mut findings)?;
    poller(ctx, result, &mut findings)?;
    remove_during_playback(ctx, result, &mut findings)?;
    Ok(findings.verdict())
}

/// Takes the rig back out of its `Arc` once every thread has let go of it.
fn unwrap_rig(rig: Arc<ManagedRig>) -> Option<ManagedRig> {
    Arc::try_unwrap(rig).ok()
}

// ---------------------------------------------------------------------------------------------
// a) four threads, four source buffers

fn four_threads(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    findings: &mut Findings,
) -> Result<(), String> {
    let mut streams: Vec<(&str, SegmentedStream)> =
        vec![("h264-low", fixture("h264-low")?), ("aac", fixture("aac")?)];
    for extra in ["h264-high", "opus"] {
        match mse_segments::load(extra) {
            Ok(stream) => streams.push((extra, stream)),
            Err(error) => {
                result.note(format!(
                    "four threads: fixture {extra} is missing ({error}); fewer buffers"
                ));
            }
        }
    }
    let rig = Arc::new(new_rig(fast_config(ctx))?);
    let mut buffers: Vec<(&str, SourceBufferId, Arc<SegmentedStream>)> = Vec::new();
    for (name, stream) in streams {
        match rig.backend.add_source_buffer(&stream.mime) {
            Ok(id) => buffers.push((name, id, Arc::new(stream))),
            Err(error) => {
                result.note(format!(
                    "four threads: add_source_buffer for {name} ({}) was refused: {error:?}; continuing with {} buffer(s)",
                    stream.mime,
                    buffers.len()
                ));
            }
        }
    }
    result.metric(
        "four_threads_source_buffers",
        buffers.iter().map(|b| b.0).collect::<Vec<_>>(),
    );
    if buffers.len() < 2 {
        findings.fail(format!(
            "only {} source buffer(s) could be added for the multi-thread test",
            buffers.len()
        ));
        return Ok(());
    }
    let barrier = Arc::new(Barrier::new(buffers.len() + 1));
    let (sender, receiver) = mpsc::channel::<(String, Result<usize, String>)>();
    for (index, (name, id, stream)) in buffers.iter().enumerate() {
        let (rig, barrier, sender, stream) = (
            Arc::clone(&rig),
            Arc::clone(&barrier),
            sender.clone(),
            Arc::clone(stream),
        );
        let (name, id) = ((*name).to_owned(), *id);
        std::thread::Builder::new()
            .name(format!("t03-append-{index}"))
            .spawn(move || {
                barrier.wait();
                let outcome = (|| -> Result<usize, String> {
                    let mut appends = 0;
                    rig.append_and_wait(id, &stream.init.bytes, APPEND_TIMEOUT)
                        .map_err(|e| format!("init: {e}"))?;
                    appends += 1;
                    for segment in &stream.media {
                        rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT)
                            .map_err(|e| format!("media segment: {e}"))?;
                        appends += 1;
                    }
                    Ok(appends)
                })();
                let _ = sender.send((name, outcome));
            })
            .map_err(|e| format!("cannot start a thread: {e}"))?;
    }
    drop(sender);
    barrier.wait();
    let started = Instant::now();
    let _ = rig.play();
    let mut done = 0;
    let mut problems = Vec::new();
    while done < buffers.len() {
        match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok((name, Ok(appends))) => {
                done += 1;
                result.note(format!(
                    "four threads: the thread for {name} made {appends} appends"
                ));
            }
            Ok((name, Err(error))) => {
                done += 1;
                problems.push(format!("the thread appending {name} failed: {error}"));
            }
            Err(_) => {
                problems.push(format!(
                    "{} of {} appending threads did not finish within 60 s: deadlock or stall in append",
                    buffers.len() - done,
                    buffers.len()
                ));
                break;
            }
        }
    }
    result.metric("four_threads_append_wall_ms", ms(started.elapsed()));
    for problem in problems {
        findings.fail(format!("four threads: {problem}"));
    }
    let _ = rig.backend.end_of_stream(EosError::None);
    let reached = rig.wait_until(Duration::from_secs(30), |rig| {
        video_frames(rig) + 2 >= FIXTURE_FRAMES
    });
    std::thread::sleep(Duration::from_millis(300));
    let video = Snapshot::read(&rig.video);
    let audio = Snapshot::read(&rig.audio);
    video.record(result, "four_threads_video");
    audio.record(result, "four_threads_audio");
    let errors = rig.log.count(|e| matches!(e, MseEvent::Error(_)));
    result.metric("four_threads_error_events", errors);
    findings.check(
        frames_ok(video.count, FIXTURE_FRAMES) || (reached && video.count <= FIXTURE_FRAMES + 2),
        format!("four threads: {} video frames reached the sink, expected {FIXTURE_FRAMES} (corruption or loss)", video.count),
    );
    findings.check(
        errors == 0,
        format!("four threads: {errors} source buffer Error event(s) during concurrent appends"),
    );
    findings.check(
        video.out_of_order == 0 && audio.out_of_order == 0,
        format!(
            "four threads: timestamps went backwards (video {}, audio {})",
            video.out_of_order, audio.out_of_order
        ),
    );
    result.note(format!(
        "four threads: {} source buffers appended concurrently; {} video frames, {} audio buffers (span {:?} ms) at the sinks",
        buffers.len(),
        video.count,
        audio.count,
        audio.span_ms()
    ));
    if let Some(rig) = unwrap_rig(rig) {
        let _ = rig.shutdown();
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// b) two threads, one source buffer

#[derive(Debug, Default)]
struct Race {
    /// The second append was refused with `InvalidState` while the first was running.
    refused_invalid_state: u32,
    /// The second append was accepted although the update was still running when the call returned.
    accepted_during_update: u32,
    /// The second append got another error.
    other_errors: Vec<String>,
    /// The update was over before the second append could be tried.
    window_missed: u32,
    /// The first append itself failed.
    first_failed: Vec<String>,
}

fn two_threads_one_buffer(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    findings: &mut Findings,
) -> Result<(), String> {
    let video = fixture("h264-low")?;
    let tiled = tile(&video, 60).map_err(|e| format!("tile: {e}"))?;
    let rig = Arc::new(new_rig(fast_config(ctx))?);
    let id = rig
        .backend
        .add_source_buffer(&video.mime)
        .map_err(|e| format!("add_source_buffer: {e:?}"))?;
    rig.append_and_wait(id, &video.init.bytes, APPEND_TIMEOUT)
        .map_err(|e| format!("init: {e}"))?;
    let _ = rig.play();

    let attempts = 8usize;
    let per_attempt = (tiled.media.len() / attempts).max(1);
    let mut race = Race::default();
    for attempt in 0..attempts {
        let from = attempt * per_attempt;
        let to = ((attempt + 1) * per_attempt).min(tiled.media.len());
        if from >= to {
            break;
        }
        let big: Vec<u8> = tiled.media[from..to]
            .iter()
            .flat_map(|s| s.bytes.iter().copied())
            .collect();
        let attempt_from = rig.log.len();
        let first_started = Arc::new(AtomicBool::new(false));
        let first_result: Arc<Mutex<Option<Result<(), MseError>>>> = Arc::new(Mutex::new(None));
        let (rig_a, started_a, result_a) = (
            Arc::clone(&rig),
            Arc::clone(&first_started),
            Arc::clone(&first_result),
        );
        let thread_a = std::thread::Builder::new()
            .name("t03-first-append".into())
            .spawn(move || {
                let outcome = rig_a.backend.append(id, &big);
                started_a.store(true, Ordering::SeqCst);
                *result_a.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
            })
            .map_err(|e| format!("cannot start a thread: {e}"))?;
        // The second caller: as soon as the first has begun, try to append while the update runs.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !first_started.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::hint::spin_loop();
        }
        let before = rig.backend.updating(id);
        let second = rig.backend.append(id, &[0u8; 16]);
        let _ = thread_a.join();
        // Let the update (and anything the second append started) finish, then read the log: two updates
        // running at once means the second append was accepted while the first was still going.
        rig.wait_until(Duration::from_secs(20), |rig| !rig.backend.updating(id));
        let overlap = max_update_overlap(&rig.log.snapshot_from(attempt_from), id);
        match (&second, before) {
            (Err(MseError::InvalidState(_)), true) => race.refused_invalid_state += 1,
            (Ok(()), _) if overlap >= 2 => race.accepted_during_update += 1,
            (Ok(()), _) | (Err(MseError::InvalidState(_)), false) => race.window_missed += 1,
            (Err(other), _) => race.other_errors.push(format!("{other:?}")),
        }
        if let Some(Err(error)) = first_result
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            race.first_failed.push(format!("{error:?}"));
        }
        // Let the update (and anything the second append started) finish before the next attempt.
        rig.wait_until(Duration::from_secs(20), |rig| !rig.backend.updating(id));
        if race.accepted_during_update > 0 {
            break;
        }
    }
    result.metric(
        "two_threads_one_buffer",
        json!({
            "attempts": attempts,
            "second_append_refused_invalid_state": race.refused_invalid_state,
            "second_append_accepted_during_update": race.accepted_during_update,
            "window_missed": race.window_missed,
            "other_errors": race.other_errors,
            "first_append_failed": race.first_failed,
        }),
    );
    result.note(format!(
        "two threads, one buffer: the second append got InvalidState {} time(s), was accepted while the first update ran {} time(s), missed the window {} time(s), other errors {:?}",
        race.refused_invalid_state, race.accepted_during_update, race.window_missed, race.other_errors
    ));
    findings.check(
        race.accepted_during_update == 0,
        format!("two threads, one buffer: a second append was accepted while the first update was still running ({} time(s)): the spec says InvalidStateError and the buffer may be corrupted", race.accepted_during_update),
    );
    findings.check(
        race.other_errors.is_empty(),
        format!(
            "two threads, one buffer: a refusal other than InvalidState: {:?}",
            race.other_errors
        ),
    );
    findings.check(
        race.first_failed.is_empty(),
        format!(
            "two threads, one buffer: the first append failed: {:?}",
            race.first_failed
        ),
    );
    if race.refused_invalid_state == 0 {
        result.note(format!(
            "two threads, one buffer: the documented InvalidState refusal was never observed in {attempts} attempts (the window was missed {} time(s)): either append() finishes the update before it returns, or the update is too short to catch; this is inconclusive, not a failure",
            race.window_missed
        ));
    }
    let errors = rig.log.count(|e| matches!(e, MseEvent::Error(_)));
    findings.check(
        errors == 0,
        format!("two threads, one buffer: {errors} source buffer Error event(s) afterwards"),
    );
    if let Some(rig) = unwrap_rig(rig) {
        let _ = rig.shutdown();
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// c) a thread polling buffered and updating every millisecond

#[derive(Default)]
struct PollStats {
    polls: AtomicU64,
    buffered_errors: Mutex<Vec<String>>,
    max_call_us: AtomicU64,
    shrank: AtomicU64,
    last_end_ns: AtomicU64,
    saw_updating: AtomicU64,
}

fn poller(ctx: &Ctx, result: &mut ScenarioResult, findings: &mut Findings) -> Result<(), String> {
    let video = fixture("h264-low")?;
    let audio = fixture("aac")?;
    let video_tiled = tile(&video, 20).map_err(|e| format!("tile: {e}"))?;
    let audio_tiled = tile(&audio, 20).map_err(|e| format!("tile: {e}"))?;
    let rig = Arc::new(new_rig(fast_config(ctx))?);
    let ids = add_buffers(&rig, &video.mime, &audio.mime)?;
    let vid = ids[0];
    let ids_p = ids.clone();
    let stats = Arc::new(PollStats::default());
    let stop = Arc::new(AtomicBool::new(false));
    let (rig_p, stats_p, stop_p) = (Arc::clone(&rig), Arc::clone(&stats), Arc::clone(&stop));
    let poll_thread = std::thread::Builder::new()
        .name("t03-poller".into())
        .spawn(move || {
            while !stop_p.load(Ordering::SeqCst) {
                let started = Instant::now();
                let updating = ids_p.iter().any(|id| rig_p.backend.updating(*id));
                if updating {
                    stats_p.saw_updating.fetch_add(1, Ordering::Relaxed);
                }
                match rig_p.backend.buffered(vid) {
                    Ok(ranges) => {
                        if let Some(last) = ranges.last() {
                            let end = last.end.nseconds();
                            let previous = stats_p.last_end_ns.swap(end, Ordering::SeqCst);
                            if end < previous {
                                stats_p.shrank.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    Err(error) => {
                        let mut errors = stats_p
                            .buffered_errors
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if errors.len() < 5 {
                            errors.push(format!("{error:?}"));
                        }
                    }
                }
                let took = started.elapsed().as_micros() as u64;
                stats_p.max_call_us.fetch_max(took, Ordering::Relaxed);
                stats_p.polls.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(1));
            }
        })
        .map_err(|e| format!("cannot start the poller: {e}"))?;

    for (id, init) in ids.iter().zip([&video.init, &audio.init]) {
        rig.append_and_wait(*id, &init.bytes, APPEND_TIMEOUT)
            .map_err(|e| format!("init: {e}"))?;
    }
    let _ = rig.play();
    let started = Instant::now();
    let mut append_failure = None;
    for index in 0..video_tiled.media.len().max(audio_tiled.media.len()) {
        for (id, stream) in ids.iter().copied().zip([&video_tiled, &audio_tiled]) {
            if let Some(segment) = stream.media.get(index)
                && let Err(error) = rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT)
            {
                append_failure = Some(format!("append {index}: {error}"));
            }
        }
        if append_failure.is_some() {
            break;
        }
    }
    let _ = rig.backend.end_of_stream(EosError::None);
    let expected = (video_tiled.duration_ms().unwrap_or(0) / 40).max(1);
    rig.wait_until(Duration::from_secs(60), |rig| {
        video_frames(rig) + 2 >= expected
    });
    let wall = started.elapsed();
    stop.store(true, Ordering::SeqCst);
    // The poller must finish: a join that does not return is a lock held for good.
    let (join_sender, join_receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = poll_thread.join();
        let _ = join_sender.send(());
    });
    let poller_ended = join_receiver.recv_timeout(Duration::from_secs(5)).is_ok();
    let polls = stats.polls.load(Ordering::Relaxed);
    let max_call_ms = stats.max_call_us.load(Ordering::Relaxed) as f64 / 1000.0;
    let errors = stats
        .buffered_errors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    result.metric(
        "poller",
        json!({
            "wall_ms": ms(wall),
            "polls": polls,
            "polls_that_saw_updating_true": stats.saw_updating.load(Ordering::Relaxed),
            "max_poll_ms": max_call_ms,
            "buffered_shrank_times": stats.shrank.load(Ordering::Relaxed),
            "buffered_errors": errors,
            "video_frames": video_frames(&rig),
            "expected_frames": expected,
            "appends_failed": append_failure,
        }),
    );
    result.note(format!(
        "poller: {polls} polls of buffered()+updating() in {:.0} ms of appends and playback (about one per ms was asked); slowest poll {max_call_ms:.1} ms; {} polls saw updating=true; buffered shrank {} time(s); {} video frames of {expected}",
        ms(wall),
        stats.saw_updating.load(Ordering::Relaxed),
        stats.shrank.load(Ordering::Relaxed),
        video_frames(&rig)
    ));
    findings.check(poller_ended, "poller: the polling thread did not end within 5 s after the stop flag: a poll is stuck inside buffered() or updating()");
    findings.check(
        append_failure.is_none(),
        format!("poller: an append failed while polling: {append_failure:?}"),
    );
    findings.check(
        errors.is_empty(),
        format!("poller: buffered() returned errors: {errors:?}"),
    );
    findings.check(
        stats.shrank.load(Ordering::Relaxed) == 0,
        "poller: the end of buffered() went backwards while only appending",
    );
    findings.check(max_call_ms < 500.0, format!("poller: one poll took {max_call_ms:.0} ms (a call that blocks the caller for half a second)"));
    findings.check(
        video_frames(&rig) + 2 >= expected,
        format!(
            "poller: playback delivered {} of {expected} frames while being polled",
            video_frames(&rig)
        ),
    );
    if poller_ended && let Some(rig) = unwrap_rig(rig) {
        let _ = rig.shutdown();
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// d) remove during playback

fn remove_during_playback(
    ctx: &Ctx,
    result: &mut ScenarioResult,
    findings: &mut Findings,
) -> Result<(), String> {
    let video = fixture("h264-low")?;
    let audio = fixture("aac")?;
    for (case, remove_from, remove_to, at_position_s, expect_min, expect_max) in [
        (
            "behind",
            ClockTime::ZERO,
            Some(ClockTime::from_mseconds(1500)),
            3u64,
            FIXTURE_FRAMES - 2,
            FIXTURE_FRAMES + 2,
        ),
        ("ahead", ClockTime::from_seconds(6), None, 1, 140, 160),
    ] {
        let config = RigConfig {
            ..realtime_config(ctx)
        };
        let rig = Arc::new(new_rig(config)?);
        let ids = add_buffers(&rig, &video.mime, &audio.mime)?;
        let vid = ids[0];
        for (id, stream) in ids.iter().copied().zip([&video, &audio]) {
            rig.append_and_wait(id, &stream.init.bytes, APPEND_TIMEOUT)
                .map_err(|e| format!("init: {e}"))?;
        }
        for index in 0..video.media.len() {
            for (id, stream) in ids.iter().copied().zip([&video, &audio]) {
                if let Some(segment) = stream.media.get(index) {
                    rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT)
                        .map_err(|e| format!("append: {e}"))?;
                }
            }
        }
        let _ = rig.backend.end_of_stream(EosError::None);
        let _ = rig.play();
        let reached = rig.wait_until(Duration::from_secs(20), |rig| {
            rig.position()
                .is_some_and(|p| p >= ClockTime::from_seconds(at_position_s))
        });
        let position = rig.position();
        let removal: Vec<String> = ids
            .iter()
            .map(|id| format!("{:?}", rig.backend.remove(*id, remove_from, remove_to)))
            .collect();
        let eos = rig.wait_for_eos(Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(300));
        let snapshot = Snapshot::read(&rig.video);
        let errors = rig.log.count(|e| matches!(e, MseEvent::Error(_)));
        let after = rig.backend.buffered(vid).map(|r| format!("{r:?}"));
        result.metric(
            &format!("remove_{case}_during_playback"),
            json!({
                "position_when_removed_ms": position.map(|p| p.mseconds()),
                "reached_position": reached,
                "remove_calls": removal,
                "eos_arrived": eos,
                "video_frames": snapshot.count,
                "expected_frames": [expect_min, expect_max],
                "error_events": errors,
                "buffered_after": after.clone().ok(),
            }),
        );
        result.note(format!(
            "remove {case} the playhead during playback (position {:?} ms when removing, calls {removal:?}): EOS {}, {} video frames (expected {expect_min}..={expect_max}), {errors} error event(s), buffered afterwards {after:?}",
            position.map(|p| p.mseconds()),
            if eos { "arrived" } else { "did NOT arrive" },
            snapshot.count
        ));
        findings.check(
            reached,
            format!("remove {case}: playback never reached {at_position_s} s"),
        );
        findings.check(
            removal.iter().all(|r| r == "Ok(())"),
            format!("remove {case}: remove() during playback was refused: {removal:?}"),
        );
        findings.check(eos, format!("remove {case}: the element never reached the end of the stream after removing data during playback"));
        findings.check(
            (expect_min..=expect_max).contains(&snapshot.count),
            format!(
                "remove {case}: {} video frames reached the sink, expected {expect_min}..={expect_max}{}",
                snapshot.count,
                if case == "ahead" { " (data removed ahead of the playhead must not play)" } else { " (data behind the playhead must not matter)" }
            ),
        );
        findings.check(
            errors == 0,
            format!("remove {case}: {errors} source buffer Error event(s)"),
        );
        if let Some(rig) = unwrap_rig(rig) {
            let _ = rig.shutdown();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_race_record_is_empty() {
        let race = Race::default();
        assert_eq!(
            race.refused_invalid_state + race.accepted_during_update + race.window_missed,
            0
        );
        assert!(race.other_errors.is_empty() && race.first_failed.is_empty());
    }

    #[test]
    fn poll_stats_track_the_slowest_call() {
        let stats = PollStats::default();
        stats.max_call_us.fetch_max(300, Ordering::Relaxed);
        stats.max_call_us.fetch_max(100, Ordering::Relaxed);
        assert_eq!(stats.max_call_us.load(Ordering::Relaxed), 300);
    }
}
