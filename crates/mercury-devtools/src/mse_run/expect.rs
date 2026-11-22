// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `tests/wpt/media-source.expect.toml`: every known WPT failure, with a class and a reason, and
//! the comparison of a run with it. The file is the contract of flip condition F3 of the MED-1b
//! brief: after a run, anything that differs from it is reported, a new failure and an
//! unexpected pass alike.
//!
//! ```toml
//! [[test]]
//! path = "media-source/mediasource-duration.html"   # below the WPT root
//! class = "dom"                                     # mse-core, dom, player, engine-gap, test-bug, not-planned
//! reason = "why it fails, in a sentence"            # required
//! issue = "https://..."                             # optional; F3 wants one for mse-core and dom entries in the core directories
//! skip = true                                       # optional: the file is not run (not-planned)
//! harness = "TIMEOUT"                               # optional: the expected harness status (default OK)
//! fail = ["subtest name"]                           # expected FAIL
//! timeout = ["subtest name"]                        # expected TIMEOUT
//! notrun = ["subtest name"]                         # expected NOTRUN
//! precondition_failed = ["subtest name"]            # expected PRECONDITION_FAILED
//! ```
//!
//! A file with no entry is expected to finish `OK` with every subtest `PASS`.

use std::collections::BTreeMap;

use toml::{Table, Value};

use super::model::{Outcome, TestResult};

/// The failure classes of the brief (D13). `not-planned` is for what Mercury does not do (MSE in
/// workers, `ManagedMediaSource`, WebCodecs).
pub const CLASSES: [&str; 6] = [
    "mse-core",
    "dom",
    "player",
    "engine-gap",
    "test-bug",
    "not-planned",
];

/// The harness statuses an entry may expect: testharness.js's, and the runner's own.
pub const HARNESS_STATUSES: [&str; 6] = [
    "OK",
    "ERROR",
    "TIMEOUT",
    "PRECONDITION_FAILED",
    "CRASH",
    "NOLOAD",
];

const SUBTEST_STATUSES: [(&str, &str); 4] = [
    ("fail", "FAIL"),
    ("timeout", "TIMEOUT"),
    ("notrun", "NOTRUN"),
    ("precondition_failed", "PRECONDITION_FAILED"),
];

/// What one entry expects of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub class: String,
    pub reason: String,
    pub issue: Option<String>,
    pub skip: bool,
    pub harness: String,
    /// Subtests expected to be something other than PASS, with the status.
    pub subtests: BTreeMap<String, String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Expectations {
    pub entries: BTreeMap<String, Entry>,
}

/// Reads the file. Every problem found is listed (an entry without a class or a reason is
/// refused, not defaulted).
pub fn parse(text: &str) -> Result<Expectations, Vec<String>> {
    let table: Table = text
        .parse()
        .map_err(|error| vec![format!("not valid TOML: {error}")])?;
    let mut problems = Vec::new();
    let mut entries = BTreeMap::new();
    let none = Vec::new();
    let tests = match table.get("test") {
        None => &none,
        Some(Value::Array(tests)) => tests,
        Some(_) => {
            return Err(vec![
                "`test` must be an array of tables ([[test]])".to_owned(),
            ]);
        }
    };
    for (index, value) in tests.iter().enumerate() {
        let Some(entry) = value.as_table() else {
            problems.push(format!("test #{}: not a table", index + 1));
            continue;
        };
        let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
        let label = if path.is_empty() {
            format!("test #{}", index + 1)
        } else {
            path.to_owned()
        };
        if path.is_empty() {
            problems.push(format!("{label}: no path"));
            continue;
        }
        let mut local = Vec::new();
        let class = entry.get("class").and_then(Value::as_str).unwrap_or("");
        if class.is_empty() {
            local.push(format!("{label}: no class (one of {})", CLASSES.join(", ")));
        } else if !CLASSES.contains(&class) {
            local.push(format!(
                "{label}: class {class:?} is not one of {}",
                CLASSES.join(", ")
            ));
        }
        let reason = entry.get("reason").and_then(Value::as_str).unwrap_or("");
        if reason.trim().is_empty() {
            local.push(format!("{label}: no reason"));
        }
        let harness = entry
            .get("harness")
            .and_then(Value::as_str)
            .unwrap_or("OK")
            .to_owned();
        if !HARNESS_STATUSES.contains(&harness.as_str()) {
            local.push(format!(
                "{label}: harness {harness:?} is not one of {}",
                HARNESS_STATUSES.join(", ")
            ));
        }
        let mut subtests = BTreeMap::new();
        for (key, status) in SUBTEST_STATUSES {
            match entry.get(key) {
                None => {}
                Some(Value::Array(names)) => {
                    for name in names {
                        match name.as_str() {
                            Some(name) => {
                                if let Some(previous) =
                                    subtests.insert(name.to_owned(), status.to_owned())
                                {
                                    local.push(format!(
                                        "{label}: subtest {name:?} is listed as {previous} and as {status}"
                                    ));
                                }
                            }
                            None => local.push(format!("{label}: `{key}` holds a non-string")),
                        }
                    }
                }
                Some(_) => local.push(format!("{label}: `{key}` must be an array of strings")),
            }
        }
        if entries.contains_key(path) {
            local.push(format!("{label}: listed twice"));
        }
        if local.is_empty() {
            entries.insert(
                path.to_owned(),
                Entry {
                    path: path.to_owned(),
                    class: class.to_owned(),
                    reason: reason.to_owned(),
                    issue: entry
                        .get("issue")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    skip: entry.get("skip").and_then(Value::as_bool).unwrap_or(false),
                    harness,
                    subtests,
                },
            );
        }
        problems.extend(local);
    }
    if problems.is_empty() {
        Ok(Expectations { entries })
    } else {
        Err(problems)
    }
}

/// One way a result differs from what the file expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// The harness ended differently (a file expected `TIMEOUT` that now finishes `OK` is an
    /// unexpected pass of the file).
    Harness {
        path: String,
        expected: String,
        actual: String,
    },
    /// A subtest expected to PASS did not.
    UnexpectedFailure {
        path: String,
        name: String,
        actual: String,
        message: Option<String>,
    },
    /// A subtest expected to fail now PASSes.
    UnexpectedPass { path: String, name: String },
    /// A subtest expected to fail ended in another non-PASS status.
    Changed {
        path: String,
        name: String,
        expected: String,
        actual: String,
    },
    /// A subtest expected to fail did not report although the harness finished.
    Missing {
        path: String,
        name: String,
        expected: String,
    },
}

impl Finding {
    pub fn path(&self) -> &str {
        match self {
            Finding::Harness { path, .. }
            | Finding::UnexpectedFailure { path, .. }
            | Finding::UnexpectedPass { path, .. }
            | Finding::Changed { path, .. }
            | Finding::Missing { path, .. } => path,
        }
    }

    pub fn is_unexpected_pass(&self) -> bool {
        match self {
            Finding::UnexpectedPass { .. } => true,
            Finding::Harness {
                expected, actual, ..
            } => actual == "OK" && expected != "OK",
            _ => false,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Finding::Harness {
                path,
                expected,
                actual,
            } => format!("{path}: harness {actual}, expected {expected}"),
            Finding::UnexpectedFailure {
                path,
                name,
                actual,
                message,
            } => format!(
                "{path}: {name:?} is {actual}, expected PASS{}",
                message
                    .as_deref()
                    .map(|m| format!(" ({})", m.lines().next().unwrap_or("")))
                    .unwrap_or_default()
            ),
            Finding::UnexpectedPass { path, name } => {
                format!("{path}: {name:?} passes, expected a failure (unexpected pass)")
            }
            Finding::Changed {
                path,
                name,
                expected,
                actual,
            } => format!("{path}: {name:?} is {actual}, expected {expected}"),
            Finding::Missing {
                path,
                name,
                expected,
            } => format!("{path}: {name:?} did not report, expected {expected}"),
        }
    }
}

/// The harness status of a result: what the page said, or what the runner saw.
pub fn actual_harness(result: &TestResult) -> String {
    match (&result.harness, result.outcome) {
        (Some(harness), _) => harness.clone(),
        (None, Outcome::Timeout) => "TIMEOUT".to_owned(),
        (None, Outcome::Crash) => "CRASH".to_owned(),
        (None, Outcome::NoLoad) => "NOLOAD".to_owned(),
        (None, _) => "OK".to_owned(),
    }
}

/// Compares one WPT file's result with its entry (`None`: expected to be all PASS).
pub fn judge(entry: Option<&Entry>, result: &TestResult) -> Vec<Finding> {
    let path = &result.id;
    let expected_harness = entry.map_or("OK", |entry| entry.harness.as_str());
    let actual = actual_harness(result);
    let mut findings = Vec::new();
    if actual != expected_harness {
        findings.push(Finding::Harness {
            path: path.clone(),
            expected: expected_harness.to_owned(),
            actual: actual.clone(),
        });
    }
    // A duplicated name keeps its worst status: a test must not hide behind its twin.
    let mut seen: BTreeMap<&str, (&str, Option<&str>)> = BTreeMap::new();
    for subtest in &result.subtests {
        let slot = seen
            .entry(subtest.name.as_str())
            .or_insert((subtest.status.as_str(), subtest.message.as_deref()));
        if slot.0 == "PASS" && subtest.status != "PASS" {
            *slot = (subtest.status.as_str(), subtest.message.as_deref());
        }
    }
    let expected_statuses = entry.map(|entry| &entry.subtests);
    for (name, (status, message)) in &seen {
        let expected = expected_statuses
            .and_then(|map| map.get(*name))
            .map_or("PASS", String::as_str);
        if *status == expected {
            continue;
        }
        findings.push(if expected == "PASS" {
            Finding::UnexpectedFailure {
                path: path.clone(),
                name: (*name).to_owned(),
                actual: (*status).to_owned(),
                message: message.map(str::to_owned),
            }
        } else if *status == "PASS" {
            Finding::UnexpectedPass {
                path: path.clone(),
                name: (*name).to_owned(),
            }
        } else {
            Finding::Changed {
                path: path.clone(),
                name: (*name).to_owned(),
                expected: expected.to_owned(),
                actual: (*status).to_owned(),
            }
        });
    }
    if actual == "OK"
        && let Some(map) = expected_statuses
    {
        for (name, expected) in map {
            if !seen.contains_key(name.as_str()) {
                findings.push(Finding::Missing {
                    path: path.clone(),
                    name: name.clone(),
                    expected: expected.clone(),
                });
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mse_run::model::{Phase, Subtest};

    const SAMPLE: &str = r#"
[[test]]
path = "media-source/a.html"
class = "dom"
reason = "audioTracks is empty"
fail = ["tracks exist", "tracks have ids"]

[[test]]
path = "media-source/b.html"
class = "engine-gap"
reason = "needs a Worker from a Blob URL"
harness = "TIMEOUT"
timeout = ["late"]

[[test]]
path = "media-source/dedicated-worker/c.html"
class = "not-planned"
reason = "MSE in workers"
skip = true
"#;

    fn result(path: &str, harness: &str, subtests: &[(&str, &str)]) -> TestResult {
        let mut result = TestResult::new(Phase::Wpt, path, Outcome::Fail);
        result.harness = Some(harness.to_owned());
        result.subtests = subtests
            .iter()
            .map(|(name, status)| Subtest {
                name: (*name).to_owned(),
                status: (*status).to_owned(),
                message: None,
            })
            .collect();
        result
    }

    #[test]
    fn the_sample_parses() {
        let expectations = parse(SAMPLE).unwrap();
        assert_eq!(expectations.entries.len(), 3);
        let a = &expectations.entries["media-source/a.html"];
        assert_eq!(a.class, "dom");
        assert_eq!(a.harness, "OK");
        assert_eq!(a.subtests["tracks exist"], "FAIL");
        let b = &expectations.entries["media-source/b.html"];
        assert_eq!(b.harness, "TIMEOUT");
        assert_eq!(b.subtests["late"], "TIMEOUT");
        assert!(expectations.entries["media-source/dedicated-worker/c.html"].skip);
        assert!(parse("").unwrap().entries.is_empty());
    }

    #[test]
    fn an_entry_without_a_class_or_a_reason_is_refused() {
        let problems = parse(
            "[[test]]\npath = \"x.html\"\nreason = \"r\"\n\n[[test]]\npath = \"y.html\"\nclass = \"dom\"\n\n[[test]]\npath = \"z.html\"\nclass = \"dom\"\nreason = \"   \"\n",
        )
        .unwrap_err();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("x.html: no class"));
        assert!(problems[1].contains("y.html: no reason"));
        assert!(problems[2].contains("z.html: no reason"));
    }

    #[test]
    fn bad_values_are_refused_with_the_entry_named() {
        for (text, needle) in [
            ("[[test]]\nclass=\"dom\"\nreason=\"r\"\n", "no path"),
            (
                "[[test]]\npath=\"x\"\nclass=\"nope\"\nreason=\"r\"\n",
                "class \"nope\"",
            ),
            (
                "[[test]]\npath=\"x\"\nclass=\"dom\"\nreason=\"r\"\nharness=\"MAYBE\"\n",
                "harness \"MAYBE\"",
            ),
            (
                "[[test]]\npath=\"x\"\nclass=\"dom\"\nreason=\"r\"\nfail=[\"a\"]\ntimeout=[\"a\"]\n",
                "listed as FAIL and as TIMEOUT",
            ),
            (
                "[[test]]\npath=\"x\"\nclass=\"dom\"\nreason=\"r\"\nfail=\"a\"\n",
                "must be an array",
            ),
            (
                "[[test]]\npath=\"x\"\nclass=\"dom\"\nreason=\"r\"\n[[test]]\npath=\"x\"\nclass=\"dom\"\nreason=\"r\"\n",
                "listed twice",
            ),
            ("test = 1\n", "array of tables"),
            ("[[test]\n", "not valid TOML"),
        ] {
            let problems = parse(text).unwrap_err();
            assert!(
                problems.iter().any(|p| p.contains(needle)),
                "{needle}: {problems:?}"
            );
        }
    }

    #[test]
    fn a_result_that_matches_its_entry_has_no_findings() {
        let expectations = parse(SAMPLE).unwrap();
        let a = result(
            "media-source/a.html",
            "OK",
            &[
                ("tracks exist", "FAIL"),
                ("tracks have ids", "FAIL"),
                ("other", "PASS"),
            ],
        );
        assert!(judge(expectations.entries.get("media-source/a.html"), &a).is_empty());
        let b = result("media-source/b.html", "TIMEOUT", &[("late", "TIMEOUT")]);
        assert!(judge(expectations.entries.get("media-source/b.html"), &b).is_empty());
        let clean = result("media-source/new.html", "OK", &[("works", "PASS")]);
        assert!(judge(None, &clean).is_empty());
    }

    #[test]
    fn an_unexpected_failure_is_found() {
        let expectations = parse(SAMPLE).unwrap();
        let a = result(
            "media-source/a.html",
            "OK",
            &[
                ("tracks exist", "FAIL"),
                ("tracks have ids", "FAIL"),
                ("other", "FAIL"),
            ],
        );
        let findings = judge(expectations.entries.get("media-source/a.html"), &a);
        assert_eq!(findings.len(), 1);
        assert!(
            matches!(&findings[0], Finding::UnexpectedFailure { name, actual, .. } if name == "other" && actual == "FAIL")
        );
        assert!(!findings[0].is_unexpected_pass());

        let new_file = result("media-source/n.html", "OK", &[("x", "FAIL")]);
        assert_eq!(judge(None, &new_file).len(), 1);
    }

    #[test]
    fn an_unexpected_pass_is_found() {
        let expectations = parse(SAMPLE).unwrap();
        let a = result(
            "media-source/a.html",
            "OK",
            &[("tracks exist", "PASS"), ("tracks have ids", "FAIL")],
        );
        let findings = judge(expectations.entries.get("media-source/a.html"), &a);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].is_unexpected_pass());
        assert!(findings[0].describe().contains("unexpected pass"));

        let b = result("media-source/b.html", "OK", &[("late", "PASS")]);
        let findings = judge(expectations.entries.get("media-source/b.html"), &b);
        assert!(
            findings.iter().all(Finding::is_unexpected_pass),
            "{findings:?}"
        );
        assert_eq!(findings.len(), 2, "the harness and the subtest");
    }

    #[test]
    fn a_timeout_where_none_is_expected_is_found() {
        let mut timed_out = TestResult::new(Phase::Wpt, "media-source/t.html", Outcome::Timeout);
        timed_out.harness = None;
        let findings = judge(None, &timed_out);
        assert_eq!(
            findings,
            [Finding::Harness {
                path: "media-source/t.html".into(),
                expected: "OK".into(),
                actual: "TIMEOUT".into(),
            }]
        );
        assert!(!findings[0].is_unexpected_pass());
        let crashed = TestResult::new(Phase::Wpt, "media-source/t.html", Outcome::Crash);
        assert_eq!(actual_harness(&crashed), "CRASH");
        let no_load = TestResult::new(Phase::Wpt, "media-source/t.html", Outcome::NoLoad);
        assert_eq!(actual_harness(&no_load), "NOLOAD");
    }

    #[test]
    fn a_changed_status_and_a_missing_subtest_are_found() {
        let expectations = parse(SAMPLE).unwrap();
        let a = result("media-source/a.html", "OK", &[("tracks exist", "TIMEOUT")]);
        let findings = judge(expectations.entries.get("media-source/a.html"), &a);
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert!(
            findings
                .iter()
                .any(|f| matches!(f, Finding::Changed { actual, .. } if actual == "TIMEOUT"))
        );
        assert!(
            findings
                .iter()
                .any(|f| matches!(f, Finding::Missing { name, .. } if name == "tracks have ids"))
        );

        // With the harness ended early, a subtest that never reported is no finding.
        let b = result("media-source/b.html", "TIMEOUT", &[]);
        assert!(judge(expectations.entries.get("media-source/b.html"), &b).is_empty());
    }

    #[test]
    fn a_duplicate_name_keeps_its_worst_status() {
        let twin = result(
            "media-source/d.html",
            "OK",
            &[("same", "PASS"), ("same", "FAIL")],
        );
        let findings = judge(None, &twin);
        assert_eq!(findings.len(), 1);
    }
}
