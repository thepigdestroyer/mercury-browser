// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The data of a window run: the phases, the result of one test, and its JSON form (one line of
//! the journal).

use serde_json::{Value, json};

/// The three phases of `mse-run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Phase {
    /// `tests/pages/mse-*.html`.
    Pages,
    /// `tests/pages/mse-players/*.html`: dash.js, Shaka Player, hls.js.
    Players,
    /// The WPT `media-source/` tree.
    Wpt,
}

impl Phase {
    pub const ALL: [Phase; 3] = [Phase::Pages, Phase::Players, Phase::Wpt];

    pub fn name(self) -> &'static str {
        match self {
            Phase::Pages => "pages",
            Phase::Players => "players",
            Phase::Wpt => "wpt",
        }
    }

    pub fn parse(text: &str) -> Option<Phase> {
        Phase::ALL.into_iter().find(|phase| phase.name() == text)
    }
}

/// How one test ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A page finished (`(done)`) with no FAIL line and at least one PASS; a WPT file with the
    /// harness status OK and every subtest PASS.
    Pass,
    /// It finished, with a failure.
    Fail,
    /// It started and did not finish before the watchdog.
    Timeout,
    /// The browser ran but the page never reported (it did not load, or the script never ran).
    NoLoad,
    /// The browser ended before the page reported.
    Crash,
    /// The screen locked during the test. Not a failure: the test must run again (`--resume`).
    Interrupted,
    /// The browser could not be killed (state `U`). Recorded, and the run stops there.
    Stuck,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Pass => "PASS",
            Outcome::Fail => "FAIL",
            Outcome::Timeout => "TIMEOUT",
            Outcome::NoLoad => "NOLOAD",
            Outcome::Crash => "CRASH",
            Outcome::Interrupted => "INTERRUPTED",
            Outcome::Stuck => "STUCK",
        }
    }

    pub fn parse(text: &str) -> Option<Outcome> {
        [
            Outcome::Pass,
            Outcome::Fail,
            Outcome::Timeout,
            Outcome::NoLoad,
            Outcome::Crash,
            Outcome::Interrupted,
            Outcome::Stuck,
        ]
        .into_iter()
        .find(|outcome| outcome.name() == text)
    }

    /// Whether `--resume` keeps this result. A test that was interrupted or stuck has not
    /// really run, so it runs again.
    pub fn is_final(self) -> bool {
        !matches!(self, Outcome::Interrupted | Outcome::Stuck)
    }
}

/// One subtest of a WPT file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subtest {
    pub name: String,
    /// `PASS`, `FAIL`, `TIMEOUT`, `NOTRUN` or `PRECONDITION_FAILED`.
    pub status: String,
    pub message: Option<String>,
}

/// What one test left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    pub phase: Phase,
    /// The page file name (`mse-basic.html`, `dashjs.html`) or the WPT path
    /// (`media-source/mediasource-duration.html`).
    pub id: String,
    pub outcome: Outcome,
    /// Page and player tests: the PASS and FAIL lines the page showed (INFO lines too).
    pub lines: Vec<String>,
    /// Errors the page's scripts raised, and what the runner saw (`exit status 101`).
    pub notes: Vec<String>,
    /// WPT: the harness status (`OK`, `ERROR`, `TIMEOUT`, ...); the runner's own `CRASH` and
    /// `NOLOAD` too.
    pub harness: Option<String>,
    pub subtests: Vec<Subtest>,
    pub duration_ms: u64,
    pub peak_rss_kb: Option<u64>,
}

impl TestResult {
    pub fn new(phase: Phase, id: impl Into<String>, outcome: Outcome) -> TestResult {
        TestResult {
            phase,
            id: id.into(),
            outcome,
            lines: Vec::new(),
            notes: Vec::new(),
            harness: None,
            subtests: Vec::new(),
            duration_ms: 0,
            peak_rss_kb: None,
        }
    }

    pub fn key(&self) -> (Phase, &str) {
        (self.phase, &self.id)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "phase": self.phase.name(),
            "id": self.id,
            "outcome": self.outcome.name(),
            "lines": self.lines,
            "notes": self.notes,
            "harness": self.harness,
            "subtests": self.subtests.iter().map(|s| json!({
                "name": s.name, "status": s.status, "message": s.message,
            })).collect::<Vec<_>>(),
            "duration_ms": self.duration_ms,
            "peak_rss_kb": self.peak_rss_kb,
        })
    }

    pub fn from_json(value: &Value) -> Option<TestResult> {
        let strings = |name: &str| -> Vec<String> {
            value[name]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(TestResult {
            phase: Phase::parse(value["phase"].as_str()?)?,
            id: value["id"].as_str()?.to_owned(),
            outcome: Outcome::parse(value["outcome"].as_str()?)?,
            lines: strings("lines"),
            notes: strings("notes"),
            harness: value["harness"].as_str().map(str::to_owned),
            subtests: value["subtests"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            Some(Subtest {
                                name: item["name"].as_str()?.to_owned(),
                                status: item["status"].as_str()?.to_owned(),
                                message: item["message"].as_str().map(str::to_owned),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            duration_ms: value["duration_ms"].as_u64().unwrap_or(0),
            peak_rss_kb: value["peak_rss_kb"].as_u64(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_survives_its_json_form() {
        let mut result = TestResult::new(Phase::Wpt, "media-source/a.html", Outcome::Fail);
        result.lines = vec!["FAIL x: \"quoted\"\nsecond line".into()];
        result.notes = vec!["exit status 1".into()];
        result.harness = Some("OK".into());
        result.subtests = vec![
            Subtest {
                name: "name with \\ and \"".into(),
                status: "FAIL".into(),
                message: Some("assert_true".into()),
            },
            Subtest {
                name: "second".into(),
                status: "PASS".into(),
                message: None,
            },
        ];
        result.duration_ms = 1234;
        result.peak_rss_kb = Some(98_765);
        let text = result.to_json().to_string();
        assert!(!text.contains('\n'), "one journal line");
        let back = TestResult::from_json(&serde_json::from_str(&text).unwrap()).unwrap();
        assert_eq!(back, result);
    }

    #[test]
    fn unknown_phases_and_outcomes_are_refused() {
        let bad = json!({"phase": "other", "id": "x", "outcome": "PASS"});
        assert!(TestResult::from_json(&bad).is_none());
        let bad = json!({"phase": "pages", "id": "x", "outcome": "MAYBE"});
        assert!(TestResult::from_json(&bad).is_none());
        let minimal = json!({"phase": "pages", "id": "x", "outcome": "PASS"});
        assert!(TestResult::from_json(&minimal).is_some());
    }

    #[test]
    fn only_finished_tests_are_final() {
        for (outcome, is_final) in [
            (Outcome::Pass, true),
            (Outcome::Fail, true),
            (Outcome::Timeout, true),
            (Outcome::NoLoad, true),
            (Outcome::Crash, true),
            (Outcome::Interrupted, false),
            (Outcome::Stuck, false),
        ] {
            assert_eq!(outcome.is_final(), is_final, "{outcome:?}");
            assert_eq!(Outcome::parse(outcome.name()), Some(outcome));
        }
    }
}
