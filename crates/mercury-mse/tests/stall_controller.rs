// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The `StallController` (med-1b review fix, findings 5 and 11): the element's raw facts in, the guard's
//! input built here, the pipeline change and the one pending re-check out. The DOM keeps only the player
//! calls and the timer, so what these tests drive is the decision the shipped element takes.

mod model_support;

use mercury_mse::StallAction;
use mercury_mse::api::ElementReadyState::{self, *};
use mercury_mse::api::{NS_PER_SEC, TimeRange};
use mercury_mse::stall_controller::{
    ElementSnapshot, PipelineChange, StallController, StallStep, highest_buffered_end,
};
use mercury_mse::stall_guard::HYSTERESIS_MS;
use model_support::Rng;

const ALL_READY: [ElementReadyState; 5] = [
    HaveNothing,
    HaveMetadata,
    HaveCurrentData,
    HaveFutureData,
    HaveEnoughData,
];

/// A healthy element that is playing at 3 s, with 8 s of data (open source).
fn playing(backend_ready: ElementReadyState, now_ms: u64) -> ElementSnapshot {
    ElementSnapshot {
        position: 3.0,
        backend_ready,
        element_ready: HaveEnoughData,
        paused: false,
        has_error: false,
        network_empty: false,
        ended_playback: false,
        seeking: false,
        source_ended: false,
        highest_buffered_end: Some(8.0),
        now_ms,
    }
}

fn enter(c: &mut StallController, now_ms: u64) {
    let step = c.step(playing(HaveCurrentData, now_ms));
    assert_eq!(step.action, StallAction::EnterStall);
    assert!(c.is_stalled());
}

const KEEP: StallStep = StallStep {
    action: StallAction::None,
    pipeline: PipelineChange::Keep,
    recheck_in_ms: None,
};

// ---- the stall, as the element sees it ----

#[test]
fn stall_a_playing_element_that_runs_out_of_data_pauses_the_pipeline() {
    let mut c = StallController::new();
    let step = c.step(playing(HaveCurrentData, 1_000));
    assert_eq!(step.action, StallAction::EnterStall);
    assert_eq!(step.pipeline, PipelineChange::Pause);
    assert_eq!(step.recheck_in_ms, None);
    assert!(c.is_stalled());
}

#[test]
fn stall_a_healthy_playing_element_is_left_alone() {
    let mut c = StallController::new();
    for (i, ready) in [HaveFutureData, HaveEnoughData].into_iter().enumerate() {
        assert_eq!(c.step(playing(ready, 1_000 + i as u64 * 100)), KEEP);
    }
    assert!(!c.is_stalled());
}

#[test]
fn stall_playing_is_derived_from_the_raw_facts() {
    // Each fact that makes the element "not playing" alone prevents a stall (the DOM's old expression:
    // not paused, no error, network state not EMPTY, readyState not HAVE_NOTHING, not ended playback).
    type Edit = fn(&mut ElementSnapshot);
    let not_playing: [(&str, Edit); 5] = [
        ("paused", |s| s.paused = true),
        ("error", |s| s.has_error = true),
        ("network EMPTY", |s| s.network_empty = true),
        ("readyState HAVE_NOTHING", |s| s.element_ready = HaveNothing),
        ("ended playback", |s| s.ended_playback = true),
    ];
    for (what, edit) in not_playing {
        let mut snapshot = playing(HaveCurrentData, 1_000);
        edit(&mut snapshot);
        let mut c = StallController::new();
        assert_eq!(c.step(snapshot), KEEP, "{what}");
        assert!(!c.is_stalled(), "{what}");
    }
    // And with none of them, the same snapshot stalls.
    let mut c = StallController::new();
    assert_eq!(
        c.step(playing(HaveCurrentData, 1_000)).action,
        StallAction::EnterStall
    );
}

#[test]
fn stall_the_readiness_that_decides_is_the_backends_not_the_elements() {
    // The element's own readyState lags; the guard reads what the backend computed for the position.
    let mut snapshot = playing(HaveFutureData, 1_000);
    snapshot.element_ready = HaveCurrentData;
    assert_eq!(StallController::new().step(snapshot), KEEP);
}

// ---- paused or seeking during a stall ----

#[test]
fn stall_the_page_pausing_during_a_stall_releases_it_without_a_stall_action() {
    let mut c = StallController::new();
    enter(&mut c, 0);
    let mut paused = playing(HaveCurrentData, 100);
    paused.paused = true;
    let step = c.step(paused);
    assert_eq!(step.action, StallAction::None);
    // The hold is released so the element re-evaluates (`update_media_state` plays only a potentially
    // playing element, and a paused one is not).
    assert_eq!(step.pipeline, PipelineChange::Resume);
    assert_eq!(step.recheck_in_ms, None);
    assert!(!c.is_stalled());
    // Still paused, data still missing: nothing to do and nothing scheduled.
    let mut later = playing(HaveCurrentData, 400);
    later.paused = true;
    assert_eq!(c.step(later), KEEP);
}

#[test]
fn stall_playing_again_after_the_pause_stalls_again_if_the_data_is_still_missing() {
    let mut c = StallController::new();
    enter(&mut c, 0);
    let mut paused = playing(HaveCurrentData, 100);
    paused.paused = true;
    c.step(paused);
    // The page calls play() a second later; the data has not come.
    let step = c.step(playing(HaveCurrentData, 1_100));
    assert_eq!(step.action, StallAction::EnterStall);
    assert_eq!(step.pipeline, PipelineChange::Pause);
}

#[test]
fn stall_a_seek_during_a_stall_releases_it_and_forgets_the_hysteresis() {
    let mut c = StallController::new();
    enter(&mut c, 0);
    let mut seeking = playing(HaveMetadata, 10);
    seeking.seeking = true;
    let step = c.step(seeking);
    assert_eq!(step.action, StallAction::None);
    assert_eq!(step.pipeline, PipelineChange::Resume);
    assert_eq!(step.recheck_in_ms, None);
    assert!(!c.is_stalled());
    // The seek lands on data: playing, nothing to do. (Had the hysteresis survived, a missing target would
    // not stall for 250 ms.)
    assert_eq!(c.step(playing(HaveEnoughData, 20)), KEEP);
    let step = c.step(playing(HaveCurrentData, 30));
    assert_eq!(step.action, StallAction::EnterStall);
}

#[test]
fn stall_a_seek_never_stalls() {
    let mut c = StallController::new();
    let mut seeking = playing(HaveMetadata, 1_000);
    seeking.seeking = true;
    assert_eq!(c.step(seeking), KEEP);
}

// ---- data arriving inside the hysteresis ----

#[test]
fn stall_data_arriving_inside_the_hysteresis_schedules_one_recheck_and_the_recheck_resumes() {
    let mut c = StallController::new();
    enter(&mut c, 0);

    // Data arrives 100 ms later: the guard wants to leave and must wait out the rest of its hysteresis.
    let step = c.step(playing(HaveFutureData, 100));
    assert_eq!(step.action, StallAction::None);
    assert_eq!(step.pipeline, PipelineChange::Keep);
    let delay = step.recheck_in_ms.expect("a recheck must be scheduled");
    assert!(
        (HYSTERESIS_MS - 100..=HYSTERESIS_MS - 100 + 50).contains(&delay),
        "the recheck waits for the rest of the hysteresis, not {delay} ms"
    );
    assert!(c.is_stalled());

    // More facts arrive while that recheck is pending (a second append, a position tick): none schedules
    // another one.
    for at in [120, 150, 199] {
        let step = c.step(playing(HaveEnoughData, at));
        assert_eq!(step, KEEP, "at {at} ms");
    }

    // The timer fires: the stall ends, the pipeline resumes, nothing is left pending.
    c.recheck_fired();
    let step = c.step(playing(HaveEnoughData, 100 + delay));
    assert_eq!(step.action, StallAction::LeaveStall);
    assert_eq!(step.pipeline, PipelineChange::Resume);
    assert_eq!(step.recheck_in_ms, None);
    assert!(!c.is_stalled());
}

#[test]
fn stall_a_recheck_that_finds_the_data_gone_again_stays_stalled_and_asks_for_nothing_more() {
    let mut c = StallController::new();
    enter(&mut c, 0);
    let delay = c
        .step(playing(HaveFutureData, 100))
        .recheck_in_ms
        .expect("recheck");
    // By the time the timer fires, the data is gone again (a remove, an eviction).
    c.recheck_fired();
    assert_eq!(c.step(playing(HaveCurrentData, 100 + delay)), KEEP);
    assert!(c.is_stalled());
    // The hysteresis has passed by now, so data that comes back is acted on at once.
    let step = c.step(playing(HaveFutureData, 100 + delay + 10));
    assert_eq!(step.action, StallAction::LeaveStall);
}

#[test]
fn stall_a_data_arrival_after_the_hysteresis_resumes_at_once_without_a_recheck() {
    let mut c = StallController::new();
    enter(&mut c, 0);
    let step = c.step(playing(HaveFutureData, HYSTERESIS_MS));
    assert_eq!(step.action, StallAction::LeaveStall);
    assert_eq!(step.pipeline, PipelineChange::Resume);
    assert_eq!(step.recheck_in_ms, None);
}

#[test]
fn stall_data_that_runs_out_again_inside_the_hysteresis_of_a_resume_schedules_one_recheck() {
    let mut c = StallController::new();
    enter(&mut c, 0);
    assert_eq!(
        c.step(playing(HaveFutureData, HYSTERESIS_MS)).action,
        StallAction::LeaveStall
    );
    // 50 ms after resuming, the position is at the end of the data again.
    let step = c.step(playing(HaveCurrentData, HYSTERESIS_MS + 50));
    assert_eq!(step.action, StallAction::None);
    assert!(step.recheck_in_ms.is_some());
    assert_eq!(
        c.step(playing(HaveCurrentData, HYSTERESIS_MS + 60))
            .recheck_in_ms,
        None
    );
    c.recheck_fired();
    assert_eq!(
        c.step(playing(HaveCurrentData, HYSTERESIS_MS * 2 + 20))
            .action,
        StallAction::EnterStall
    );
}

// ---- finding 11: `ended` means "ended and the data reaches its end" ----

/// An ended source whose data runs to 8 s, at `position`.
fn ended_at(position: f64, backend_ready: ElementReadyState, now_ms: u64) -> ElementSnapshot {
    ElementSnapshot {
        position,
        source_ended: true,
        highest_buffered_end: Some(8.0),
        ..playing(backend_ready, now_ms)
    }
}

#[test]
fn stall_an_ended_source_with_a_hole_in_the_middle_still_stalls_at_the_hole() {
    // endOfStream() was called with data [0, 2) and [5, 8): the position is at 3 s in the hole. The old
    // input (`ended` = the source is ended) never entered a stall here and the pipeline ran on.
    let mut c = StallController::new();
    let step = c.step(ended_at(3.0, HaveMetadata, 1_000));
    assert_eq!(step.action, StallAction::EnterStall);
    assert_eq!(step.pipeline, PipelineChange::Pause);
}

#[test]
fn stall_the_end_of_the_stream_does_not_end_a_stall_in_a_hole() {
    // Stalled in the hole before endOfStream(); the call must not let the pipeline run on over nothing.
    let mut c = StallController::new();
    enter(&mut c, 0);
    for at in [300, 600, 5_000] {
        let step = c.step(ended_at(3.0, HaveMetadata, at));
        assert_eq!(step, KEEP, "at {at} ms");
    }
    assert!(c.is_stalled());
}

#[test]
fn stall_an_ended_stream_at_its_end_is_not_a_stall() {
    // Past the last frame, or at the very end: playback ends, it does not stall.
    for position in [8.0, 8.2] {
        let mut c = StallController::new();
        assert_eq!(
            c.step(ended_at(position, HaveMetadata, 1_000)),
            KEEP,
            "position {position}"
        );
        assert!(!c.is_stalled());
    }
}

#[test]
fn stall_a_stall_waiting_for_the_last_segment_ends_when_the_stream_ends_at_the_position() {
    // Data ran out at 7.9 s waiting for the last segment; the page calls endOfStream() instead (the
    // data it has is all there is): the stall ends and the element goes on to the end.
    let mut c = StallController::new();
    let mut waiting = playing(HaveCurrentData, 0);
    waiting.position = 7.9;
    waiting.highest_buffered_end = Some(7.9);
    assert_eq!(c.step(waiting).action, StallAction::EnterStall);

    let mut ended = waiting;
    ended.source_ended = true;
    ended.now_ms = HYSTERESIS_MS;
    let step = c.step(ended);
    assert_eq!(step.action, StallAction::LeaveStall);
    assert_eq!(step.pipeline, PipelineChange::Resume);
}

#[test]
fn stall_an_open_source_is_never_ended_whatever_the_position() {
    let mut c = StallController::new();
    let mut open = ended_at(9.0, HaveMetadata, 1_000);
    open.source_ended = false;
    assert_eq!(c.step(open).action, StallAction::EnterStall);
}

#[test]
fn stall_an_ended_source_with_nothing_buffered_has_nothing_to_wait_for() {
    let mut c = StallController::new();
    let mut empty = ended_at(0.0, HaveMetadata, 1_000);
    empty.highest_buffered_end = None;
    assert_eq!(c.step(empty), KEEP);
}

#[test]
fn stall_the_highest_buffered_end_is_the_end_of_the_last_range() {
    assert_eq!(highest_buffered_end(&[]), None);
    let ranges = [
        TimeRange::new(0, 2 * NS_PER_SEC),
        TimeRange::new(5 * NS_PER_SEC, 8 * NS_PER_SEC),
    ];
    assert_eq!(highest_buffered_end(&ranges), Some(8.0));
    assert_eq!(highest_buffered_end(&ranges[..1]), Some(2.0));
}

// ---- nothing is left waiting (a property over random facts) ----

/// After any history, facts that stay the same settle: the controller asks for its rechecks, the timer
/// fires, and the final state is exactly what the rule says for those facts. This is the guarantee the
/// DOM's timer exists for (the pipeline is paused during a stall, so no position update will come).
#[test]
fn stall_constant_facts_always_settle_after_the_scheduled_rechecks() {
    for seed in 0..300u64 {
        let mut rng = Rng::new(seed);
        let mut c = StallController::new();
        let mut now = 0u64;
        let mut outstanding: Option<u64> = None;

        let random_facts = |rng: &mut Rng, now: u64| ElementSnapshot {
            position: rng.below(10) as f64,
            backend_ready: ALL_READY[rng.below(5)],
            element_ready: ALL_READY[rng.below(5)],
            paused: rng.chance(20),
            has_error: rng.chance(5),
            network_empty: rng.chance(5),
            ended_playback: rng.chance(10),
            seeking: rng.chance(15),
            source_ended: rng.chance(30),
            highest_buffered_end: if rng.chance(90) {
                Some(rng.below(10) as f64)
            } else {
                None
            },
            now_ms: now,
        };

        let mut facts = random_facts(&mut rng, now);
        for _ in 0..40 {
            now += rng.below(400) as u64;
            facts = random_facts(&mut rng, now);
            let step = c.step(facts);
            check_step(&step, &mut outstanding, now);
            // The timer sometimes fires between two facts.
            if let Some(due) = outstanding
                && now >= due
                && rng.chance(70)
            {
                outstanding = None;
                c.recheck_fired();
            }
        }

        // Now the facts stop changing; the timers fire when due and the element steps again.
        for _ in 0..4 {
            if let Some(due) = outstanding.take() {
                now = now.max(due);
                c.recheck_fired();
            } else {
                now += 1_000;
            }
            facts.now_ms = now;
            let step = c.step(facts);
            check_step(&step, &mut outstanding, now);
        }
        now += 1_000;
        facts.now_ms = now;
        let step = c.step(facts);
        assert_eq!(step.recheck_in_ms, None, "seed {seed}: still waiting");

        let playing = !facts.paused
            && !facts.has_error
            && !facts.network_empty
            && facts.element_ready != HaveNothing
            && !facts.ended_playback;
        let ended = facts.source_ended
            && facts
                .highest_buffered_end
                .is_none_or(|end| facts.position >= end);
        let want = playing && !facts.seeking && !ended && facts.backend_ready <= HaveCurrentData;
        assert_eq!(
            c.is_stalled(),
            want,
            "seed {seed}: settled facts {facts:?} (stalled {})",
            c.is_stalled()
        );
    }
}

/// Invariants of one step: a pause comes only with `EnterStall`, a resume only when a stall ended, and at
/// most one recheck is outstanding.
fn check_step(step: &StallStep, outstanding: &mut Option<u64>, now: u64) {
    match step.action {
        StallAction::EnterStall => assert_eq!(step.pipeline, PipelineChange::Pause),
        StallAction::LeaveStall => assert_eq!(step.pipeline, PipelineChange::Resume),
        StallAction::None => assert_ne!(step.pipeline, PipelineChange::Pause),
    }
    if let Some(delay) = step.recheck_in_ms {
        assert!(
            outstanding.is_none(),
            "a second recheck while one is pending"
        );
        assert!(delay > 0 && delay <= HYSTERESIS_MS + 50, "delay {delay}");
        *outstanding = Some(now + delay);
    }
}
