// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::time::Duration;

use talaria_core::gestures::fixture;
use talaria_core::{
    Feedback, Reach, Stage, SwipeDirection, SwipeRecogniser, Tuning, WheelPhase, WheelSample,
};

const BOTH: Reach = Reach {
    back: true,
    forward: true,
};

const NO_BACK: Reach = Reach {
    back: false,
    forward: true,
};

const NO_FORWARD: Reach = Reach {
    back: true,
    forward: false,
};

fn sample(ms: u64, dx: f32, dy: f32, phase: WheelPhase) -> WheelSample {
    WheelSample {
        at: Duration::from_millis(ms),
        delta_x: dx,
        delta_y: dy,
        phase,
        precise: true,
    }
}

fn load(name: &str) -> Vec<WheelSample> {
    let path = format!(
        "{}/tests/fixtures/wheel/{name}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    fixture::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

struct Run {
    commits: Vec<SwipeDirection>,
    frames: Vec<Feedback>,
    stages: Vec<Stage>,
    final_stage: Stage,
}

fn feed(samples: &[WheelSample], reach: Reach) -> Run {
    let mut recogniser = SwipeRecogniser::new(Tuning::DEFAULT);
    let mut run = Run {
        commits: Vec::new(),
        frames: Vec::new(),
        stages: Vec::new(),
        final_stage: Stage::Idle,
    };
    for s in samples {
        let update = recogniser.on_wheel(*s, reach);
        run.frames.push(update.feedback);
        run.stages.push(recogniser.stage());
        run.commits.extend(update.commit);
    }
    run.final_stage = recogniser.stage();
    run
}

fn run(name: &str, reach: Reach) -> Run {
    feed(&load(name), reach)
}

fn shown(f: &Feedback) -> bool {
    matches!(f, Feedback::Shown { .. })
}

fn progress(f: Feedback) -> f32 {
    match f {
        Feedback::Shown { progress, .. } => progress,
        Feedback::Hidden => panic!("expected a shown chip, got Hidden"),
    }
}

fn feed_to(
    recogniser: &mut SwipeRecogniser,
    samples: &[WheelSample],
    reach: Reach,
    stop: impl Fn(Feedback) -> bool,
) -> usize {
    for (i, s) in samples.iter().enumerate() {
        if stop(recogniser.on_wheel(*s, reach).feedback) {
            return i + 1;
        }
    }
    panic!("the stop frame never came");
}

fn rest(
    recogniser: &mut SwipeRecogniser,
    samples: &[WheelSample],
    reach: Reach,
) -> Vec<SwipeDirection> {
    samples
        .iter()
        .filter_map(|s| recogniser.on_wheel(*s, reach).commit)
        .collect()
}

#[test]
fn fixture_swipe_back_slow_commits_back_once() {
    let r = run("swipe-back-slow", BOTH);
    assert_eq!(r.commits, [SwipeDirection::Back]);
    assert!(r.frames.iter().any(|f| matches!(
        f,
        Feedback::Shown {
            direction: SwipeDirection::Back,
            armed: true,
            ..
        }
    )));
    assert_eq!(r.frames.last(), Some(&Feedback::Hidden));
}

#[test]
fn fixture_swipe_back_fast_with_momentum_commits_once() {
    let samples = load("swipe-back-fast-with-momentum");
    let r = feed(&samples, BOTH);
    assert_eq!(r.commits, [SwipeDirection::Back]);
    let first_ended = samples
        .iter()
        .position(|s| s.phase == WheelPhase::Ended)
        .expect("the fingers lift");
    assert!(!r.frames[first_ended + 1..].iter().any(shown));
}

#[test]
fn fixture_swipe_forward_commits_forward() {
    let r = run("swipe-forward", BOTH);
    assert_eq!(r.commits, [SwipeDirection::Forward]);
}

#[test]
fn fixture_short_swipe_no_commit() {
    let r = run("short-swipe-no-commit", BOTH);
    assert!(r.commits.is_empty());
    assert!(
        r.frames
            .iter()
            .any(|f| matches!(f, Feedback::Shown { armed: false, .. }))
    );
}

#[test]
fn fixture_vertical_scroll_never_shows_or_commits() {
    let r = run("vertical-scroll", BOTH);
    assert!(r.frames.iter().all(|f| *f == Feedback::Hidden));
    assert!(r.commits.is_empty());
    assert_eq!(r.final_stage, Stage::Idle);
}

#[test]
fn fixture_diagonal_scroll_never_shows_or_commits() {
    let r = run("diagonal-scroll", BOTH);
    assert!(r.frames.iter().all(|f| *f == Feedback::Hidden));
    assert!(r.commits.is_empty());
    assert_eq!(r.final_stage, Stage::Idle);
}

#[test]
fn fixture_rubber_band_reverse_does_not_commit() {
    let r = run("rubber-band-reverse", BOTH);
    assert!(r.commits.is_empty());
}

#[test]
fn fixture_second_swipe_in_momentum_is_the_tail() {
    let first_only = feed(&load("swipe-back-slow"), BOTH);
    assert_eq!(first_only.commits.len(), 1);
    let r = run("second-swipe-in-momentum", BOTH);
    assert_eq!(r.commits, [SwipeDirection::Back]);

    let tail_at = |gap_ms: u64| {
        let mut samples = load("swipe-back-slow");
        let ended = samples.last().expect("a fixture").at;
        samples.extend((0..14).map(|i| {
            let phase = if i == 0 {
                WheelPhase::Began
            } else {
                WheelPhase::Changed
            };
            WheelSample {
                at: ended + Duration::from_millis(gap_ms + 8 * i),
                delta_x: 14.0,
                delta_y: 0.0,
                phase,
                precise: true,
            }
        }));
        samples.push(WheelSample {
            at: ended + Duration::from_millis(gap_ms + 112),
            delta_x: 0.0,
            delta_y: 0.0,
            phase: WheelPhase::Ended,
            precise: true,
        });
        feed(&samples, BOTH).commits.len()
    };
    assert_eq!(tail_at(40), 1);
    assert_eq!(tail_at(300), 2);
}

#[test]
fn fixture_notched_mouse_wheel_is_ignored() {
    let samples = load("notched-mouse-wheel");
    assert!(samples.iter().all(|s| !s.precise));
    let r = feed(&samples, BOTH);
    assert!(r.frames.iter().all(|f| *f == Feedback::Hidden));
    assert!(r.commits.is_empty());
}

#[test]
fn fixture_lost_end_drops_the_gesture() {
    let r = run("lost-end", BOTH);
    assert_eq!(r.commits.len(), 1);
    let samples = load("lost-end");
    let fresh = samples
        .iter()
        .position(|s| s.at == Duration::from_millis(2192))
        .expect("the second gesture starts");
    assert!(r.frames[..fresh].iter().any(shown));
    assert_eq!(r.frames[fresh], Feedback::Hidden);
}

#[test]
fn fixture_tail_only_starts_nothing() {
    let r = run("tail-only", BOTH);
    assert!(r.frames.iter().all(|f| *f == Feedback::Hidden));
    assert!(r.commits.is_empty());
    assert_eq!(r.final_stage, Stage::Idle);
}

#[test]
fn back_without_history_never_shows_and_never_commits() {
    let r = run("swipe-back-slow", NO_BACK);
    assert!(r.frames.iter().all(|f| *f == Feedback::Hidden));
    assert!(r.commits.is_empty());
    assert!(r.stages.contains(&Stage::Ignored));
    assert_eq!(r.final_stage, Stage::Idle);
}

#[test]
fn forward_chip_needs_forward_reach() {
    let r = run("swipe-forward", NO_FORWARD);
    assert!(r.frames.iter().all(|f| *f == Feedback::Hidden));
    assert!(r.commits.is_empty());
    let with = run("swipe-forward", BOTH);
    assert!(with.frames.iter().any(|f| matches!(
        f,
        Feedback::Shown {
            direction: SwipeDirection::Forward,
            ..
        }
    )));
}

#[test]
fn page_consumed_before_the_arm_blocks_the_commit() {
    let samples = load("swipe-back-slow");
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    let next = feed_to(&mut r, &samples, BOTH, |f| {
        matches!(f, Feedback::Shown { armed: false, .. })
    });
    let update = r.page_consumed();
    assert_eq!(update.feedback, Feedback::Hidden);
    assert_eq!(update.commit, None);
    assert_eq!(r.stage(), Stage::Ignored);
    assert!(rest(&mut r, &samples[next..], BOTH).is_empty());
}

#[test]
fn page_consumed_after_the_arm_blocks_the_commit() {
    let samples = load("swipe-back-slow");
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    let next = feed_to(&mut r, &samples, BOTH, |f| {
        matches!(f, Feedback::Shown { armed: true, .. })
    });
    let update = r.page_consumed();
    assert_eq!(update.feedback, Feedback::Hidden);
    assert_eq!(update.commit, None);
    assert_eq!(r.stage(), Stage::Ignored);
    assert!(rest(&mut r, &samples[next..], BOTH).is_empty());
}

#[test]
fn page_consumed_after_ended_changes_nothing() {
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    let _ = rest(&mut r, &load("swipe-back-slow"), BOTH);
    assert_eq!(r.stage(), Stage::Idle);
    let before = r.stage();
    let update = r.page_consumed();
    assert_eq!(update.commit, None);
    assert_eq!(update.feedback, Feedback::Hidden);
    assert_eq!(r.stage(), before);
}

#[test]
fn page_consumed_in_idle_changes_nothing() {
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    let update = r.page_consumed();
    assert_eq!(update.feedback, Feedback::Hidden);
    assert_eq!(update.commit, None);
    assert_eq!(r.stage(), Stage::Idle);
}

#[test]
fn cancel_hides_and_never_commits() {
    let samples = load("swipe-back-slow");
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    let next = feed_to(&mut r, &samples, BOTH, |f| {
        matches!(f, Feedback::Shown { armed: true, .. })
    });
    let update = r.cancel();
    assert_eq!(update.feedback, Feedback::Hidden);
    assert_eq!(update.commit, None);
    assert_eq!(r.stage(), Stage::Idle);
    assert!(rest(&mut r, &samples[next..], BOTH).is_empty());
}

#[test]
fn reach_lost_mid_swipe_ignores_the_gesture() {
    let samples = load("swipe-back-slow");
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    let next = feed_to(&mut r, &samples, BOTH, |f| {
        matches!(f, Feedback::Shown { armed: true, .. })
    });
    let update = r.on_wheel(samples[next], NO_BACK);
    assert_eq!(update.feedback, Feedback::Hidden);
    assert_eq!(r.stage(), Stage::Ignored);
    assert!(rest(&mut r, &samples[next + 1..], NO_BACK).is_empty());
}

#[test]
fn progress_is_along_over_arm_distance_and_the_chip_shows_after_16_px() {
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    let u = r.on_wheel(sample(0, 24.0, 0.0, WheelPhase::Began), BOTH);
    assert_eq!(progress(u.feedback), 24.0 / 120.0);
    let u = r.on_wheel(sample(8, -14.0, 0.0, WheelPhase::Changed), BOTH);
    assert_eq!(u.feedback, Feedback::Hidden);
    let u = r.on_wheel(sample(16, 6.0, 0.0, WheelPhase::Changed), BOTH);
    assert!((progress(u.feedback) - 16.0 / 120.0).abs() < 1e-6);
    let u = r.on_wheel(sample(24, 44.0, 0.0, WheelPhase::Changed), BOTH);
    assert!((progress(u.feedback) - 0.5).abs() < 1e-6);
    let u = r.on_wheel(sample(32, 60.0, 0.0, WheelPhase::Changed), BOTH);
    assert!(matches!(
        u.feedback,
        Feedback::Shown {
            armed: true,
            progress,
            ..
        } if (progress - 1.0).abs() < 1e-6
    ));
    let u = r.on_wheel(sample(40, 380.0, 0.0, WheelPhase::Changed), BOTH);
    assert!(matches!(
        u.feedback,
        Feedback::Shown {
            armed: true,
            progress,
            ..
        } if (progress - 1.0).abs() < 1e-6
    ));
}

#[test]
fn intent_needs_24_px_and_a_2_to_1_ratio() {
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    r.on_wheel(sample(0, 23.0, 0.0, WheelPhase::Began), BOTH);
    assert_eq!(r.stage(), Stage::Tracking);

    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    r.on_wheel(sample(0, 15.7, 8.3, WheelPhase::Began), BOTH);
    assert_eq!(r.stage(), Stage::Ignored);

    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    r.on_wheel(sample(0, 16.0, 8.0, WheelPhase::Began), BOTH);
    assert_eq!(r.stage(), Stage::Swiping(SwipeDirection::Back));
}

#[test]
fn a_second_began_while_tracking_restarts_the_sums() {
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    r.on_wheel(sample(0, 20.0, 0.0, WheelPhase::Began), BOTH);
    r.on_wheel(sample(8, 10.0, 0.0, WheelPhase::Began), BOTH);
    assert_eq!(r.stage(), Stage::Tracking);
}

#[test]
fn a_vertical_turn_ends_the_swipe() {
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    r.on_wheel(sample(0, 30.0, 0.0, WheelPhase::Began), BOTH);
    assert_eq!(r.stage(), Stage::Swiping(SwipeDirection::Back));
    let u = r.on_wheel(sample(8, 0.0, 40.0, WheelPhase::Changed), BOTH);
    assert_eq!(r.stage(), Stage::Ignored);
    assert_eq!(u.feedback, Feedback::Hidden);
}

#[test]
fn non_finite_deltas_are_treated_as_zero_and_never_panic() {
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    r.on_wheel(sample(0, f32::NAN, f32::INFINITY, WheelPhase::Began), BOTH);
    r.on_wheel(
        sample(8, f32::NEG_INFINITY, f32::NAN, WheelPhase::Changed),
        BOTH,
    );
    r.on_wheel(sample(16, f32::NAN, 0.0, WheelPhase::Ended), BOTH);
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    let u = r.on_wheel(sample(0, 30.0, 0.0, WheelPhase::Began), BOTH);
    assert_eq!(r.stage(), Stage::Swiping(SwipeDirection::Back));
    assert!(matches!(u.feedback, Feedback::Shown { progress, .. } if progress.is_finite()));
}

#[test]
fn fixture_parse_round_trips_and_rejects_bad_lines() {
    let s = WheelSample {
        at: Duration::from_micros(8_123),
        delta_x: 1.5,
        delta_y: -2.25,
        phase: WheelPhase::Began,
        precise: true,
    };
    let text = fixture::format(&s, "Started");
    assert_eq!(fixture::parse(&text), Ok(vec![s]));

    let err = fixture::parse("1 2 3 Sideways 1").expect_err("unknown phase");
    assert!(err.contains("line 1"), "{err}");

    let commented = "# a comment\n\n0 1 2 Started 1\n";
    assert_eq!(fixture::parse(commented).map(|v| v.len()), Ok(1));
}

struct Lcg(u64);

impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn unit(&mut self) -> f32 {
        (self.next_u64() % 10_000) as f32 / 10_000.0
    }
}

#[test]
fn a_seeded_fuzz_never_panics_and_commits_only_after_an_armed_frame() {
    let mut rng = Lcg(0x5eed_0f5a);
    let mut r = SwipeRecogniser::new(Tuning::DEFAULT);
    let mut at = 0u64;
    let mut armed_seen = false;
    let mut last_commit: Option<u64> = None;
    let mut commits = 0usize;
    for _ in 0..20_000 {
        let gap = if rng.next_u64() % 100 < 2 {
            500
        } else {
            1 + rng.next_u64() % 40
        };
        at += gap;
        let phase = match rng.next_u64() % 8 {
            0 | 1 => WheelPhase::Began,
            2..=5 => WheelPhase::Changed,
            6 => WheelPhase::Ended,
            _ if rng.next_u64().is_multiple_of(2) => WheelPhase::Cancelled,
            _ => WheelPhase::Momentum,
        };
        let mut dx = rng.unit() * 40.0 - 8.0;
        let mut dy = rng.unit() * 20.0 - 10.0;
        match rng.next_u64() % 200 {
            0 => dx = f32::NAN,
            1 => dy = f32::INFINITY,
            _ => {}
        }
        let precise = !rng.next_u64().is_multiple_of(20);
        let reach = if rng.next_u64().is_multiple_of(10) {
            NO_BACK
        } else {
            BOTH
        };
        if phase == WheelPhase::Began {
            armed_seen = false;
        }
        let s = WheelSample {
            at: Duration::from_millis(at),
            delta_x: dx,
            delta_y: dy,
            phase,
            precise,
        };
        let u = r.on_wheel(s, reach);
        if let Feedback::Shown {
            progress, armed, ..
        } = u.feedback
        {
            assert!(progress.is_finite() && (0.0..=1.0).contains(&progress));
            armed_seen |= armed;
        }
        if phase == WheelPhase::Began {
            last_commit = None;
        }
        if u.commit.is_some() {
            commits += 1;
            assert!(armed_seen, "a commit must follow an armed frame");
            if let Some(prev) = last_commit {
                assert!(at - prev >= 80, "two commits closer than the momentum join");
            }
            last_commit = Some(at);
        }
    }
    assert!(commits > 0, "the fuzz should produce at least one commit");
}
