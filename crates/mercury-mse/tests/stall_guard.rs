// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The `StallGuard` of lane med-1b (decision D4): table tests of the rule and its thresholds, the 250 ms
//! hysteresis, and a property test over random sequences of states (item 1, `core`).

mod model_support;

use mercury_mse::api::ElementReadyState::{self, *};
use mercury_mse::stall_guard::HYSTERESIS_MS;
use mercury_mse::{StallAction, StallGuard, StallInput};
use model_support::Rng;

const ALL_READY: [ElementReadyState; 5] = [
    HaveNothing,
    HaveMetadata,
    HaveCurrentData,
    HaveFutureData,
    HaveEnoughData,
];

fn input(ready: ElementReadyState, now_ms: u64) -> StallInput {
    StallInput {
        position: 1.0,
        ready,
        playing: true,
        seeking: false,
        ended: false,
        now_ms,
    }
}

fn stalled_guard(now_ms: u64) -> StallGuard {
    let mut g = StallGuard::new();
    assert_eq!(
        g.update(input(HaveCurrentData, now_ms)),
        StallAction::EnterStall
    );
    assert!(g.is_stalled());
    g
}

#[test]
fn stall_enters_exactly_when_playing_with_at_most_current_data() {
    for ready in ALL_READY {
        let mut g = StallGuard::new();
        let got = g.update(input(ready, 1_000));
        let want = if ready <= HaveCurrentData {
            StallAction::EnterStall
        } else {
            StallAction::None
        };
        assert_eq!(got, want, "ready {ready:?}");
        assert_eq!(g.is_stalled(), want == StallAction::EnterStall, "{ready:?}");
    }
}

#[test]
fn stall_leaves_exactly_at_future_data() {
    for ready in ALL_READY {
        let mut g = stalled_guard(1_000);
        let got = g.update(input(ready, 1_000 + HYSTERESIS_MS));
        let want = if ready >= HaveFutureData {
            StallAction::LeaveStall
        } else {
            StallAction::None
        };
        assert_eq!(got, want, "ready {ready:?}");
        assert_eq!(g.is_stalled(), want != StallAction::LeaveStall, "{ready:?}");
    }
}

#[test]
fn stall_the_same_state_again_does_nothing() {
    let mut g = StallGuard::new();
    assert_eq!(g.update(input(HaveFutureData, 0)), StallAction::None);
    assert_eq!(g.update(input(HaveFutureData, 100)), StallAction::None);
    assert_eq!(
        g.update(input(HaveCurrentData, 200)),
        StallAction::EnterStall
    );
    for t in 3..40 {
        assert_eq!(
            g.update(input(HaveCurrentData, t * 100)),
            StallAction::None,
            "t {t}"
        );
        assert!(g.is_stalled());
    }
    assert_eq!(
        g.update(input(HaveEnoughData, 5_000)),
        StallAction::LeaveStall
    );
    for t in 51..90 {
        assert_eq!(
            g.update(input(HaveEnoughData, t * 100)),
            StallAction::None,
            "t {t}"
        );
        assert!(!g.is_stalled());
    }
}

#[test]
fn stall_leave_waits_for_the_hysteresis() {
    let mut g = stalled_guard(1_000);
    for dt in [0, 1, 100, HYSTERESIS_MS - 1] {
        assert_eq!(
            g.update(input(HaveFutureData, 1_000 + dt)),
            StallAction::None,
            "dt {dt}"
        );
        assert!(g.is_stalled(), "still stalled at dt {dt}");
    }
    assert_eq!(
        g.update(input(HaveFutureData, 1_000 + HYSTERESIS_MS)),
        StallAction::LeaveStall
    );
}

#[test]
fn stall_enter_again_waits_for_the_hysteresis() {
    let mut g = stalled_guard(1_000);
    assert_eq!(
        g.update(input(HaveEnoughData, 1_000 + HYSTERESIS_MS)),
        StallAction::LeaveStall
    );
    let left = 1_000 + HYSTERESIS_MS;
    for dt in [0, 1, 100, HYSTERESIS_MS - 1] {
        assert_eq!(
            g.update(input(HaveNothing, left + dt)),
            StallAction::None,
            "dt {dt}"
        );
        assert!(!g.is_stalled());
    }
    assert_eq!(
        g.update(input(HaveNothing, left + HYSTERESIS_MS)),
        StallAction::EnterStall
    );
}

#[test]
fn stall_a_late_leave_still_happens_once_the_data_is_back_and_stays() {
    let mut g = stalled_guard(0);
    // Data flickers back at 100 ms (too early), goes again, returns for good at 400 ms.
    assert_eq!(g.update(input(HaveFutureData, 100)), StallAction::None);
    assert_eq!(g.update(input(HaveCurrentData, 200)), StallAction::None);
    assert_eq!(
        g.update(input(HaveFutureData, 300)),
        StallAction::LeaveStall
    );
}

#[test]
fn stall_a_clock_that_steps_back_counts_as_no_time_passed() {
    let mut g = stalled_guard(10_000);
    assert_eq!(g.update(input(HaveEnoughData, 9_000)), StallAction::None);
    assert!(g.is_stalled());
    assert_eq!(
        g.update(input(HaveEnoughData, 10_000 + HYSTERESIS_MS)),
        StallAction::LeaveStall
    );
}

#[test]
fn stall_seeking_never_stalls_and_clears_a_stall_without_an_action() {
    for ready in ALL_READY {
        let mut g = StallGuard::new();
        let seeking = StallInput {
            seeking: true,
            ..input(ready, 1_000)
        };
        assert_eq!(g.update(seeking), StallAction::None, "{ready:?}");
        assert!(!g.is_stalled());
    }
    let mut g = stalled_guard(1_000);
    let seeking = StallInput {
        seeking: true,
        ..input(HaveMetadata, 1_050)
    };
    assert_eq!(g.update(seeking), StallAction::None);
    assert!(!g.is_stalled(), "the seek cleared the stall");
    // After the seek the new situation may stall at once: the hysteresis does not span a seek.
    assert_eq!(
        g.update(input(HaveMetadata, 1_060)),
        StallAction::EnterStall
    );
}

#[test]
fn stall_ended_never_stalls() {
    for ready in ALL_READY {
        let mut g = StallGuard::new();
        let ended = StallInput {
            ended: true,
            ..input(ready, 1_000)
        };
        assert_eq!(g.update(ended), StallAction::None, "{ready:?}");
        assert!(!g.is_stalled());
    }
}

#[test]
fn stall_ended_leaves_a_stall_even_without_data() {
    let mut g = stalled_guard(1_000);
    let ended = StallInput {
        ended: true,
        ..input(HaveCurrentData, 1_000 + HYSTERESIS_MS)
    };
    assert_eq!(g.update(ended), StallAction::LeaveStall);
    assert!(!g.is_stalled());
}

#[test]
fn stall_paused_never_stalls() {
    for ready in ALL_READY {
        let mut g = StallGuard::new();
        let paused = StallInput {
            playing: false,
            ..input(ready, 1_000)
        };
        assert_eq!(g.update(paused), StallAction::None, "{ready:?}");
        assert!(!g.is_stalled());
    }
}

#[test]
fn stall_a_pause_during_a_stall_lets_go_without_resuming() {
    let mut g = stalled_guard(1_000);
    let paused = StallInput {
        playing: false,
        ..input(HaveFutureData, 5_000)
    };
    assert_eq!(g.update(paused), StallAction::None);
    assert!(!g.is_stalled(), "the guard no longer owns the pause");
    // Play again with no data: a fresh stall.
    assert_eq!(
        g.update(input(HaveCurrentData, 5_100)),
        StallAction::EnterStall
    );
}

#[test]
fn stall_position_does_not_change_the_answer() {
    let mut a = StallGuard::new();
    let mut b = StallGuard::new();
    for (i, ready) in ALL_READY.iter().cycle().take(40).enumerate() {
        let now = i as u64 * 100;
        let x = StallInput {
            position: 0.0,
            ..input(*ready, now)
        };
        let y = StallInput {
            position: 1e9,
            ..input(*ready, now)
        };
        assert_eq!(a.update(x), b.update(y), "step {i}");
    }
}

/// Replay with `MSE_STALL_SEED=n`.
fn seeds() -> Vec<u64> {
    match std::env::var("MSE_STALL_SEED") {
        Ok(v) => vec![v.parse().expect("MSE_STALL_SEED is a number")],
        Err(_) => (1..=300).collect(),
    }
}

#[test]
fn stall_property_over_random_sequences() {
    for seed in seeds() {
        let mut rng = Rng::new(seed);
        let mut g = StallGuard::new();
        let mut now = rng.below(1_000) as u64;
        // What the element would do with the answers: the pipeline is paused by MSE exactly while
        // `mse_paused`.
        let mut mse_paused = false;
        let mut last_action: Option<(StallAction, u64)> = None;
        for step in 0..400 {
            now += [0, 10, 50, 100, 100, 100, 250, 400, 1000][rng.below(9)] as u64;
            let inp = StallInput {
                position: step as f64 * 0.1,
                ready: ALL_READY[rng.below(5)],
                playing: rng.chance(80),
                seeking: rng.chance(8),
                ended: rng.chance(8),
                now_ms: now,
            };
            let was_stalled = g.is_stalled();
            let action = g.update(inp);
            let ctx = format!("seed {seed} step {step} {inp:?} -> {action:?}");

            match action {
                StallAction::EnterStall => {
                    assert!(!was_stalled, "{ctx}: Enter while stalled");
                    assert!(
                        inp.playing && !inp.seeking && !inp.ended,
                        "{ctx}: stall began while paused, seeking or ended"
                    );
                    assert!(inp.ready <= HaveCurrentData, "{ctx}");
                    assert!(g.is_stalled(), "{ctx}");
                    mse_paused = true;
                }
                StallAction::LeaveStall => {
                    assert!(was_stalled, "{ctx}: Leave while not stalled");
                    assert!(!inp.seeking && inp.playing, "{ctx}");
                    assert!(inp.ended || inp.ready >= HaveFutureData, "{ctx}");
                    assert!(!g.is_stalled(), "{ctx}");
                    mse_paused = false;
                }
                StallAction::None => {
                    if was_stalled && !g.is_stalled() {
                        // A silent let-go: only for a seek or a page pause.
                        assert!(inp.seeking || !inp.playing, "{ctx}: silent let-go");
                        mse_paused = false;
                    } else {
                        assert_eq!(was_stalled, g.is_stalled(), "{ctx}: silent entry");
                    }
                }
            }
            assert_eq!(
                mse_paused,
                g.is_stalled(),
                "{ctx}: element and guard disagree"
            );

            if action != StallAction::None {
                if let Some((prev, at)) = last_action {
                    // Alternation and spacing hold across every pair of actions with no seek or pause
                    // letting go in between; a let-go resets both.
                    if prev == action {
                        panic!("{ctx}: two {action:?} in a row (last at {at})");
                    }
                    assert!(
                        now - at >= HYSTERESIS_MS,
                        "{ctx}: transitions {at} and {now} are closer than {HYSTERESIS_MS} ms"
                    );
                }
                last_action = Some((action, now));
            }
            if inp.seeking || (!inp.playing && was_stalled) {
                // A seek or a page pause ends the guard's story: the next stall is a new one.
                last_action = None;
            }
        }
    }
}
