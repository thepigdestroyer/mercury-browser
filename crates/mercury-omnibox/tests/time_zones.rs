// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use mercury_omnibox::{Answer, AnswerKind, Context, answer};

/// 2026-11-22T12:00:00Z, a Sunday.
const NOW_WINTER: i64 = 1795348800;
/// 2026-07-01T12:00:00Z, a Wednesday.
const NOW_SUMMER: i64 = 1782907200;
/// 2026-03-08T00:00:00Z.
const NOW_DST_GAP: i64 = 1772928000;

fn context(now_unix_secs: i64, local_zone: &str, clock24: bool) -> Context {
    Context {
        now_unix_secs,
        local_zone: local_zone.into(),
        clock24,
    }
}

fn winter() -> Context {
    context(NOW_WINTER, "Europe/London", false)
}

fn found(input: &str, context: &Context) -> Option<Answer> {
    answer(input, context)
}

fn check(input: &str, context: &Context, text: &str, detail: &str) {
    let answer = found(input, context).unwrap_or_else(|| panic!("no answer for {input:?}"));
    assert_eq!(answer.kind, AnswerKind::TimeZone, "{input}");
    assert_eq!(answer.text, text, "{input}");
    assert_eq!(answer.copy, text, "{input}");
    assert_eq!(answer.detail.as_deref(), Some(detail), "{input}");
}

const WINTER_12H: &[(&str, &str, &str)] = &[
    ("time in tokyo", "9:00 PM", "Tokyo · Sun 22 Nov · UTC+9"),
    ("tokyo time", "9:00 PM", "Tokyo · Sun 22 Nov · UTC+9"),
    ("now in sydney", "11:00 PM", "Sydney · Sun 22 Nov · UTC+11"),
    (
        "time in new york",
        "7:00 AM",
        "New York · Sun 22 Nov · UTC-5",
    ),
    (
        "time in kolkata",
        "5:30 PM",
        "Kolkata · Sun 22 Nov · UTC+5:30",
    ),
    (
        "3pm new york to london",
        "8:00 PM",
        "London · Sun 22 Nov · UTC+0",
    ),
    (
        "3 pm nyc in london",
        "8:00 PM",
        "London · Sun 22 Nov · UTC+0",
    ),
    (
        "3:30pm est in ist",
        "2:00 AM",
        "IST · Mon 23 Nov · UTC+5:30",
    ),
    (
        "15:30 utc to tokyo",
        "12:30 AM",
        "Tokyo · Mon 23 Nov · UTC+9",
    ),
    ("noon in tokyo", "9:00 PM", "Tokyo · Sun 22 Nov · UTC+9"),
    (
        "midnight london to tokyo",
        "9:00 AM",
        "Tokyo · Sun 22 Nov · UTC+9",
    ),
    ("3pm in tokyo", "12:00 AM", "Tokyo · Mon 23 Nov · UTC+9"),
    ("time in utc+5:30", "5:30 PM", "UTC+5:30 · Sun 22 Nov"),
    (
        "time in america/new_york",
        "7:00 AM",
        "New York · Sun 22 Nov · UTC-5",
    ),
    ("time in PST", "4:00 AM", "PST · Sun 22 Nov · UTC-8"),
    (
        "time in los angeles",
        "4:00 AM",
        "Los Angeles · Sun 22 Nov · UTC-8",
    ),
];

const WINTER_24H: &[(&str, &str, &str)] = &[
    ("time in tokyo", "21:00", "Tokyo · Sun 22 Nov · UTC+9"),
    ("15:30 utc to tokyo", "00:30", "Tokyo · Mon 23 Nov · UTC+9"),
];

const SUMMER_12H: &[(&str, &str, &str)] = &[
    (
        "3pm new york to london",
        "8:00 PM",
        "London · Wed 1 Jul · UTC+1",
    ),
    (
        "time in new york",
        "8:00 AM",
        "New York · Wed 1 Jul · UTC-4",
    ),
    ("time in london", "1:00 PM", "London · Wed 1 Jul · UTC+1"),
    ("time in sydney", "10:00 PM", "Sydney · Wed 1 Jul · UTC+10"),
];

const NONE: &[&str] = &[
    "time in",
    "time in atlantis",
    "3pm",
    "25pm tokyo to london",
    "time",
    "what time",
    "tokyo",
    "now",
    "time in tokyo please",
    "3pm new york to",
    "3pm xyz to london",
    "5 to 6 london",
    "13pm tokyo to london",
    "0:61 utc to tokyo",
];

#[test]
fn winter_answers_in_12_hour_clock() {
    for (input, text, detail) in WINTER_12H {
        check(input, &winter(), text, detail);
    }
}

#[test]
fn winter_answers_in_24_hour_clock() {
    let clock24 = context(NOW_WINTER, "Europe/London", true);
    for (input, text, detail) in WINTER_24H {
        check(input, &clock24, text, detail);
    }
}

#[test]
fn summer_answers_follow_daylight_saving() {
    let summer = context(NOW_SUMMER, "Europe/London", false);
    for (input, text, detail) in SUMMER_12H {
        check(input, &summer, text, detail);
    }
}

#[test]
fn phrases_that_are_not_time_questions_give_no_answer() {
    for input in NONE {
        assert_eq!(found(input, &winter()), None, "{input}");
    }
}

#[test]
fn an_unknown_local_zone_only_breaks_the_phrases_that_need_it() {
    let unknown = context(NOW_WINTER, "Nowhere/City", false);
    assert!(found("time in tokyo", &unknown).is_some());
    assert!(found("3pm new york to london", &unknown).is_some());
    assert_eq!(found("3pm in tokyo", &unknown), None);
    assert_eq!(found("noon in tokyo", &unknown), None);
}

#[test]
fn a_clock_time_in_a_daylight_saving_gap_does_not_panic() {
    let gap = context(NOW_DST_GAP, "Europe/London", false);
    assert!(found("3:00 am america/new_york to london", &gap).is_some());
}

#[test]
fn copy_is_text_and_query_is_the_collapsed_input() {
    let answer = found("  time   in   tokyo ", &winter()).expect("an answer");
    assert_eq!(answer.query, "time in tokyo");
    assert_eq!(answer.copy, answer.text);
}
