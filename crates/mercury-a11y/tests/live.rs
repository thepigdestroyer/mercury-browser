// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Live regions (EE8): which nodes are live, what text was added, and the announcer's first-sight
//! and rate rules, on `TestDom`s written as HTML. Test names start with `live_`, so
//! `cargo test -p mercury-a11y --test live` runs them all.

use mercury_a11y::live::{Announcer, LiveState, Politeness, diff_added, live_state};
use mercury_a11y::testing::TestDom;

/// The live state of the element with this id in this fragment.
#[track_caller]
fn state(html: &str, id: &str) -> Option<LiveState> {
    let dom = TestDom::from_html(html);
    live_state(&dom, dom.by_id(id))
}

fn polite_atomic() -> LiveState {
    LiveState {
        politeness: Politeness::Polite,
        atomic: true,
        busy: false,
    }
}

fn polite_log() -> LiveState {
    LiveState {
        politeness: Politeness::Polite,
        atomic: false,
        busy: false,
    }
}

fn assertive_atomic() -> LiveState {
    LiveState {
        politeness: Politeness::Assertive,
        atomic: true,
        busy: false,
    }
}

// ---------------------------------------------------------------------------------------------
// live_state: implicit roles, explicit aria-live, aria-atomic, aria-busy.
// ---------------------------------------------------------------------------------------------

#[test]
fn live_alert_is_assertive_and_atomic() {
    let s = state(r#"<div id=x role=alert>Saved</div>"#, "x").unwrap();
    assert_eq!(s.politeness, Politeness::Assertive);
    assert!(s.atomic);
    assert!(!s.busy);
}

#[test]
fn live_status_is_polite_and_atomic() {
    let s = state(r#"<div id=x role=status>Ready</div>"#, "x").unwrap();
    assert_eq!(s.politeness, Politeness::Polite);
    assert!(s.atomic);
}

#[test]
fn live_log_is_polite_and_not_atomic() {
    let s = state(r#"<div id=x role=log>Line</div>"#, "x").unwrap();
    assert_eq!(s.politeness, Politeness::Polite);
    assert!(!s.atomic);
}

#[test]
fn live_timer_and_marquee_are_off() {
    let timer = state(r#"<div id=x role=timer>12:00</div>"#, "x").unwrap();
    assert_eq!(timer.politeness, Politeness::Off);
    let marquee = state(r#"<div id=x role=marquee>News</div>"#, "x").unwrap();
    assert_eq!(marquee.politeness, Politeness::Off);
}

#[test]
fn live_aria_live_overrides_the_implicit_role() {
    let s = state(r#"<div id=x role=status aria-live=off>Ready</div>"#, "x").unwrap();
    assert_eq!(s.politeness, Politeness::Off);
}

#[test]
fn live_aria_live_assertive_on_a_div() {
    let s = state(r#"<div id=x aria-live=assertive>Now</div>"#, "x").unwrap();
    assert_eq!(s.politeness, Politeness::Assertive);
    assert!(!s.atomic);
}

#[test]
fn live_aria_live_polite_on_a_div() {
    let s = state(r#"<div id=x aria-live=polite>Now</div>"#, "x").unwrap();
    assert_eq!(s.politeness, Politeness::Polite);
    assert!(!s.atomic);
}

#[test]
fn live_aria_atomic_true_makes_a_log_atomic() {
    let s = state(r#"<div id=x role=log aria-atomic=true>Line</div>"#, "x").unwrap();
    assert!(s.atomic);
}

#[test]
fn live_aria_atomic_false_makes_a_status_not_atomic() {
    let s = state(
        r#"<div id=x role=status aria-atomic=false>Ready</div>"#,
        "x",
    )
    .unwrap();
    assert!(!s.atomic);
}

#[test]
fn live_aria_busy_true_sets_busy() {
    let s = state(r#"<div id=x role=status aria-busy=true>Working</div>"#, "x").unwrap();
    assert!(s.busy);
}

#[test]
fn live_aria_busy_false_is_not_busy() {
    let s = state(r#"<div id=x role=status aria-busy=false>Done</div>"#, "x").unwrap();
    assert!(!s.busy);
}

#[test]
fn live_a_plain_div_is_not_live() {
    assert_eq!(state(r#"<div id=x>Plain</div>"#, "x"), None);
}

// ---------------------------------------------------------------------------------------------
// diff_added: the suffix after the longest common prefix.
// ---------------------------------------------------------------------------------------------

#[test]
fn live_diff_added_returns_the_suffix() {
    assert_eq!(
        diff_added("Loading", "Loading done", 100),
        Some(" done".to_string())
    );
}

#[test]
fn live_diff_added_none_when_shorter_or_equal() {
    assert_eq!(diff_added("Loading done", "Loading", 100), None);
    assert_eq!(diff_added("same", "same", 100), None);
}

#[test]
fn live_diff_added_caps_at_the_limit() {
    let added = "x".repeat(50);
    let got = diff_added("", &added, 10).unwrap();
    assert_eq!(got.chars().count(), 10);
}

// ---------------------------------------------------------------------------------------------
// Announcer: first sight, changes, busy and the per-second budget.
// ---------------------------------------------------------------------------------------------

#[test]
fn live_the_first_sight_never_speaks() {
    let mut announcer = Announcer::new(20);
    let spoken = announcer.observe(0, 1, &assertive_atomic(), "Saved", true);
    assert_eq!(spoken, None);
}

#[test]
fn live_a_change_after_first_sight_speaks() {
    let mut announcer = Announcer::new(20);
    announcer.observe(0, 1, &polite_log(), "Start", true);
    let spoken = announcer
        .observe(100, 1, &polite_log(), "Start done", false)
        .unwrap();
    assert_eq!(spoken.text, " done");
    assert!(!spoken.assertive);
}

#[test]
fn live_an_atomic_change_speaks_the_whole_text() {
    let mut announcer = Announcer::new(20);
    announcer.observe(0, 1, &assertive_atomic(), "Old", true);
    let spoken = announcer
        .observe(100, 1, &assertive_atomic(), "New text", false)
        .unwrap();
    assert_eq!(spoken.text, "New text");
    assert!(spoken.assertive);
}

#[test]
fn live_a_polite_atomic_change_speaks_polite() {
    let mut announcer = Announcer::new(20);
    announcer.observe(0, 1, &polite_atomic(), "Old", true);
    let spoken = announcer
        .observe(100, 1, &polite_atomic(), "New", false)
        .unwrap();
    assert!(!spoken.assertive);
}

#[test]
fn live_a_busy_region_is_silent_until_it_clears() {
    let mut announcer = Announcer::new(20);
    let busy = LiveState {
        busy: true,
        ..polite_atomic()
    };
    announcer.observe(0, 1, &polite_atomic(), "Start", true);
    assert_eq!(announcer.observe(100, 1, &busy, "Middle", false), None);
    assert_eq!(announcer.observe(200, 1, &busy, "Later", false), None);
    let spoken = announcer
        .observe(300, 1, &polite_atomic(), "Later done", false)
        .unwrap();
    assert_eq!(spoken.text, "Later done");
}

#[test]
fn live_at_most_max_per_second() {
    let mut announcer = Announcer::new(20);
    announcer.observe(0, 1, &polite_atomic(), "t0", true);
    let mut spoken = 0;
    for i in 1..=21u64 {
        let text = format!("t{i}");
        if announcer
            .observe(i, 1, &polite_atomic(), &text, false)
            .is_some()
        {
            spoken += 1;
        }
    }
    assert_eq!(spoken, 20);
    assert_eq!(
        announcer.observe(999, 1, &polite_atomic(), "t99", false),
        None
    );
}
