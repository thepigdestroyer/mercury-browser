// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! When the visible page is hidden because its window is covered or minimised (energy-1 lane).
//! The clock is the `now` argument: every test adds `Duration`s to one `Instant`, so none sleeps.

use std::time::{Duration, Instant};

use talaria_core::{
    Exemption, MINIMISED_HIDE_DELAY, OCCLUDED_HIDE_DELAY, ViewVisibility, VisibilityInputs,
    VisibilityPolicy, WindowCover,
};

fn at(base: Instant, milliseconds: u64) -> Instant {
    base + Duration::from_millis(milliseconds)
}

fn inputs(cover: WindowCover) -> VisibilityInputs {
    VisibilityInputs {
        cover,
        ..VisibilityInputs::default()
    }
}

fn with_audio(cover: WindowCover) -> VisibilityInputs {
    VisibilityInputs {
        playing_audio: true,
        ..inputs(cover)
    }
}

fn with_capture(cover: WindowCover) -> VisibilityInputs {
    VisibilityInputs {
        capturing: true,
        ..inputs(cover)
    }
}

fn low_power(cover: WindowCover) -> VisibilityInputs {
    VisibilityInputs {
        low_power_mode: true,
        ..inputs(cover)
    }
}

#[test]
fn an_uncovered_window_keeps_the_page_shown() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();

    for milliseconds in [0, 500, 60_000] {
        assert_eq!(
            policy.update(inputs(WindowCover::Uncovered), at(base, milliseconds)),
            ViewVisibility::Shown
        );
    }
    assert_eq!(policy.hide_due(inputs(WindowCover::Uncovered)), None);
}

#[test]
fn a_covered_window_hides_the_page_after_the_grace_period() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();
    let covered = inputs(WindowCover::Occluded);

    assert_eq!(policy.update(covered, base), ViewVisibility::Shown);
    assert_eq!(policy.hide_due(covered), Some(base + OCCLUDED_HIDE_DELAY));
    assert_eq!(policy.update(covered, at(base, 999)), ViewVisibility::Shown);
    assert_eq!(
        policy.update(covered, at(base, 1000)),
        ViewVisibility::Hidden
    );
    assert_eq!(policy.visibility(), ViewVisibility::Hidden);
    assert_eq!(
        policy.hide_due(covered),
        None,
        "nothing is pending any more"
    );
}

#[test]
fn a_cover_that_comes_and_goes_never_hides() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();

    policy.update(inputs(WindowCover::Occluded), base);
    policy.update(inputs(WindowCover::Occluded), at(base, 600));
    policy.update(inputs(WindowCover::Uncovered), at(base, 650));
    // Covered again at 700 ms: the first 600 ms count for nothing.
    policy.update(inputs(WindowCover::Occluded), at(base, 700));

    assert_eq!(
        policy.update(inputs(WindowCover::Occluded), at(base, 1699)),
        ViewVisibility::Shown
    );
    assert_eq!(
        policy.update(inputs(WindowCover::Occluded), at(base, 1700)),
        ViewVisibility::Hidden
    );
}

#[test]
fn a_minimised_window_hides_at_once() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();

    assert_eq!(MINIMISED_HIDE_DELAY, Duration::ZERO);
    assert_eq!(
        policy.update(inputs(WindowCover::Minimised), base),
        ViewVisibility::Hidden
    );
}

#[test]
fn low_power_mode_hides_a_covered_window_at_once() {
    for cover in [WindowCover::Occluded, WindowCover::Minimised] {
        let base = Instant::now();
        let mut policy = VisibilityPolicy::default();

        assert_eq!(
            policy.update(low_power(cover), base),
            ViewVisibility::Hidden,
            "{cover:?}"
        );
    }
}

#[test]
fn low_power_mode_does_not_hide_an_uncovered_window() {
    let mut policy = VisibilityPolicy::default();

    assert_eq!(
        policy.update(low_power(WindowCover::Uncovered), Instant::now()),
        ViewVisibility::Shown
    );
}

#[test]
fn uncovering_shows_at_once() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();
    policy.update(inputs(WindowCover::Minimised), base);
    assert_eq!(policy.visibility(), ViewVisibility::Hidden);

    assert_eq!(
        policy.update(inputs(WindowCover::Uncovered), at(base, 5)),
        ViewVisibility::Shown
    );
    assert_eq!(policy.hide_due(inputs(WindowCover::Uncovered)), None);
}

#[test]
fn playing_audio_keeps_a_covered_page_shown() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();

    for cover in [WindowCover::Occluded, WindowCover::Minimised] {
        for seconds in [0, 1, 10, 3600] {
            assert_eq!(
                policy.update(with_audio(cover), base + Duration::from_secs(seconds)),
                ViewVisibility::Shown,
                "{cover:?} after {seconds} s"
            );
        }
    }
    assert_eq!(policy.hide_due(with_audio(WindowCover::Occluded)), None);
}

#[test]
fn capture_keeps_a_covered_page_shown() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();

    for cover in [WindowCover::Occluded, WindowCover::Minimised] {
        for seconds in [0, 1, 10, 3600] {
            assert_eq!(
                policy.update(with_capture(cover), base + Duration::from_secs(seconds)),
                ViewVisibility::Shown,
                "{cover:?} after {seconds} s"
            );
        }
    }
}

#[test]
fn an_exemption_that_begins_while_hidden_shows_the_page() {
    let base = Instant::now();
    for exempt in [
        with_audio(WindowCover::Occluded),
        with_capture(WindowCover::Occluded),
    ] {
        let mut policy = VisibilityPolicy::default();
        policy.update(inputs(WindowCover::Occluded), base);
        policy.update(inputs(WindowCover::Occluded), at(base, 1000));
        assert_eq!(policy.visibility(), ViewVisibility::Hidden);

        assert_eq!(
            policy.update(exempt, at(base, 2000)),
            ViewVisibility::Shown,
            "{exempt:?}"
        );
    }
}

#[test]
fn an_exemption_that_ends_while_covered_starts_the_grace_again() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();
    policy.update(with_audio(WindowCover::Occluded), base);
    policy.update(with_audio(WindowCover::Occluded), at(base, 30_000));

    // The audio stops at 30 s: the page is not hidden for the 30 s it was covered.
    assert_eq!(
        policy.update(inputs(WindowCover::Occluded), at(base, 30_000)),
        ViewVisibility::Shown
    );
    assert_eq!(
        policy.hide_due(inputs(WindowCover::Occluded)),
        Some(at(base, 31_000))
    );
    assert_eq!(
        policy.update(inputs(WindowCover::Occluded), at(base, 30_999)),
        ViewVisibility::Shown
    );
    assert_eq!(
        policy.update(inputs(WindowCover::Occluded), at(base, 31_000)),
        ViewVisibility::Hidden
    );
}

#[test]
fn both_exemptions_together_and_one_ending() {
    let base = Instant::now();
    let both = VisibilityInputs {
        playing_audio: true,
        capturing: true,
        ..inputs(WindowCover::Occluded)
    };
    let mut policy = VisibilityPolicy::default();

    assert_eq!(policy.update(both, base), ViewVisibility::Shown);
    // Audio ends, the camera stays: still exempt, nothing starts counting.
    assert_eq!(
        policy.update(with_capture(WindowCover::Occluded), at(base, 5000)),
        ViewVisibility::Shown
    );
    assert_eq!(policy.hide_due(with_capture(WindowCover::Occluded)), None);
}

#[test]
fn the_kind_of_cover_changing_while_pending_changes_the_delay() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();
    policy.update(inputs(WindowCover::Occluded), base);

    // Minimised at 100 ms: the grace period is 0, counted from the first cover.
    assert_eq!(
        policy.update(inputs(WindowCover::Minimised), at(base, 100)),
        ViewVisibility::Hidden
    );
}

#[test]
fn low_power_mode_changing_while_pending_shortens_the_wait() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();
    policy.update(inputs(WindowCover::Occluded), base);
    assert_eq!(
        policy.hide_due(inputs(WindowCover::Occluded)),
        Some(at(base, 1000))
    );

    assert_eq!(
        policy.hide_due(low_power(WindowCover::Occluded)),
        Some(base),
        "the deadline follows the inputs it is asked about"
    );
    assert_eq!(
        policy.update(low_power(WindowCover::Occluded), at(base, 200)),
        ViewVisibility::Hidden
    );
}

#[test]
fn a_clock_that_goes_backwards_never_hides() {
    let base = Instant::now() + Duration::from_secs(10);
    let mut policy = VisibilityPolicy::default();
    policy.update(inputs(WindowCover::Occluded), base);

    assert_eq!(
        policy.update(inputs(WindowCover::Occluded), base - Duration::from_secs(5)),
        ViewVisibility::Shown
    );
}

#[test]
fn updating_again_changes_nothing() {
    let base = Instant::now();
    let mut policy = VisibilityPolicy::default();
    let covered = inputs(WindowCover::Occluded);
    policy.update(covered, base);
    policy.update(covered, at(base, 1000));
    let hidden = policy;

    for milliseconds in [1000, 1001, 5000, 100_000] {
        assert_eq!(
            policy.update(covered, at(base, milliseconds)),
            ViewVisibility::Hidden
        );
    }
    assert_eq!(policy, hidden);
}

#[test]
fn exemption_order_is_audio_then_capture() {
    let none = inputs(WindowCover::Occluded);
    let audio = with_audio(WindowCover::Occluded);
    let capture = with_capture(WindowCover::Occluded);
    let both = VisibilityInputs {
        playing_audio: true,
        capturing: true,
        ..none
    };

    assert_eq!(none.exemption(), None);
    assert_eq!(audio.exemption(), Some(Exemption::PlayingAudio));
    assert_eq!(capture.exemption(), Some(Exemption::Capture));
    assert_eq!(both.exemption(), Some(Exemption::PlayingAudio));
}

#[test]
fn the_grace_period_by_cover_and_low_power_mode() {
    let table = [
        (WindowCover::Uncovered, false, Duration::ZERO),
        (WindowCover::Occluded, false, OCCLUDED_HIDE_DELAY),
        (WindowCover::Minimised, false, MINIMISED_HIDE_DELAY),
        (WindowCover::Uncovered, true, Duration::ZERO),
        (WindowCover::Occluded, true, Duration::ZERO),
        (WindowCover::Minimised, true, Duration::ZERO),
    ];
    for (cover, low_power_mode, expected) in table {
        let inputs = VisibilityInputs {
            cover,
            low_power_mode,
            ..VisibilityInputs::default()
        };
        assert_eq!(
            inputs.hide_delay(),
            expected,
            "{cover:?}, low power {low_power_mode}"
        );
    }
    assert_eq!(OCCLUDED_HIDE_DELAY, Duration::from_secs(1));
}
