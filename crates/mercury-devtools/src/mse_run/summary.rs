// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The report of a run: a Markdown document with a table per phase, the WPT comparison with
//! `media-source.expect.toml`, and the exit status.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use super::model::{Outcome, Phase, TestResult};
use super::runner::RunData;

/// The share of passing subtests of F3 of the MED-1b brief (flip condition).
pub const F3_FLOOR_PERCENT: f64 = 85.0;

/// Words in a WPT path that mark the directories F3 cares most about (append, remove, abort,
/// duration, end of stream, buffered, seek, `changeType`, `timestampOffset`, append windows).
const CORE_WORDS: [&str; 10] = [
    "append",
    "remove",
    "abort",
    "duration",
    "endofstream",
    "buffered",
    "seek",
    "changetype",
    "timestamp",
    "appendwindow",
];

pub fn covers_core(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    CORE_WORDS.iter().any(|word| lower.contains(word))
}

/// `2026-11-22T12:34:56Z` for a time in seconds since 1970.
pub fn format_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

pub fn utc_now() -> String {
    format_utc(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs()),
    )
}

/// What the run adds up to.
#[derive(Debug, Default, PartialEq)]
pub struct Totals {
    pub pass: usize,
    pub unexpected_results: usize,
    pub unexpected_passes: usize,
    pub not_run: usize,
    pub interrupted_or_stuck: bool,
    /// WPT subtests: all, passing.
    pub subtests: usize,
    pub subtests_passing: usize,
}

impl Totals {
    pub fn subtest_percent(&self) -> f64 {
        if self.subtests == 0 {
            0.0
        } else {
            self.subtests_passing as f64 * 100.0 / self.subtests as f64
        }
    }
}

pub fn totals(data: &RunData) -> Totals {
    let mut totals = Totals {
        not_run: data.not_run.len(),
        interrupted_or_stuck: data.interrupted.is_some() || data.stuck.is_some(),
        ..Totals::default()
    };
    for result in &data.results {
        match result.phase {
            Phase::Pages | Phase::Players => {
                if result.outcome == Outcome::Pass {
                    totals.pass += 1;
                } else {
                    totals.unexpected_results += 1;
                }
            }
            Phase::Wpt => {
                totals.subtests += result.subtests.len();
                totals.subtests_passing += result
                    .subtests
                    .iter()
                    .filter(|s| s.status == "PASS")
                    .count();
            }
        }
    }
    totals.unexpected_results += data
        .findings
        .iter()
        .map(super::expect::Finding::path)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    totals.unexpected_passes = data
        .findings
        .iter()
        .filter(|f| f.is_unexpected_pass())
        .count();
    totals
}

/// 0: nothing unexpected and nothing left out. 1: unexpected results (including unexpected
/// passes). 3: the run is incomplete (a lock, a stuck browser, tests not run); it wins over 1,
/// because an incomplete run proves nothing.
pub fn exit_code(data: &RunData) -> i32 {
    let totals = totals(data);
    if totals.interrupted_or_stuck || totals.not_run > 0 {
        3
    } else if totals.unexpected_results > 0 {
        1
    } else {
        0
    }
}

fn cell(text: &str) -> String {
    let flat = text.replace(['\n', '\r'], " ").replace('|', "\\|");
    if flat.chars().count() > 200 {
        let cut: String = flat.chars().take(200).collect();
        format!("{cut}…")
    } else {
        flat
    }
}

fn rss(result: &TestResult) -> String {
    result
        .peak_rss_kb
        .map_or("-".to_owned(), |kb| format!("{} MB", kb / 1024))
}

fn seconds(result: &TestResult) -> String {
    format!("{:.1} s", result.duration_ms as f64 / 1000.0)
}

fn page_table(out: &mut String, title: &str, results: &[&TestResult]) {
    let _ = writeln!(out, "\n## {title}\n");
    if results.is_empty() {
        let _ = writeln!(out, "No test of this phase ran.");
        return;
    }
    let passed = results
        .iter()
        .filter(|r| r.outcome == Outcome::Pass)
        .count();
    let _ = writeln!(out, "{passed} of {} PASS.\n", results.len());
    let _ = writeln!(
        out,
        "| Test | Result | PASS | FAIL | Time | Peak RSS | Notes |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|");
    for result in results {
        let passes = result
            .lines
            .iter()
            .filter(|l| l.starts_with("PASS"))
            .count();
        let fails = result
            .lines
            .iter()
            .filter(|l| l.starts_with("FAIL"))
            .count();
        let notes = result.notes.join("; ");
        let _ = writeln!(
            out,
            "| {} | {} | {passes} | {fails} | {} | {} | {} |",
            cell(&result.id),
            result.outcome.name(),
            seconds(result),
            rss(result),
            cell(&notes)
        );
    }
    for result in results.iter().filter(|r| r.outcome != Outcome::Pass) {
        let _ = writeln!(out, "\n**{}** ({})\n", result.id, result.outcome.name());
        let shown: Vec<&String> = result
            .lines
            .iter()
            .filter(|l| l.starts_with("FAIL") || l.starts_with("INFO"))
            .collect();
        if shown.is_empty() {
            let _ = writeln!(out, "(no FAIL or INFO line)");
        }
        for line in shown {
            let _ = writeln!(out, "- `{}`", line.replace('`', "'"));
        }
    }
}

/// The report of a run, as Markdown.
pub fn render(data: &RunData) -> String {
    let totals = totals(data);
    let code = exit_code(data);
    let mut out = String::new();
    let _ = writeln!(out, "# MSE window run {}\n", data.started);
    let _ = writeln!(
        out,
        "- Started {}, finished {}.",
        data.started, data.finished
    );
    let _ = writeln!(out, "- Browser: `{}`.", data.browser.display());
    let _ = writeln!(
        out,
        "- Journal: `{}` ({} result(s) taken from it with --resume, {} unreadable line(s)).",
        data.journal.display(),
        data.resumed,
        data.unreadable_journal_lines
    );
    if !data.tools_versions.trim().is_empty() {
        let _ = writeln!(
            out,
            "- Tools fetched: {}.",
            data.tools_versions.trim().replace('\n', "; ")
        );
    }
    let verdict = match code {
        0 => "**EXIT 0**: every test did what was expected; nothing was left out.",
        1 => "**EXIT 1**: unexpected results (below).",
        _ => "**EXIT 3**: the run is incomplete (below); `--resume` continues it.",
    };
    let _ = writeln!(out, "\n{verdict}");

    if let Some(interrupted) = &data.interrupted {
        let _ = writeln!(
            out,
            "\n> **Interrupted by a screen lock**: {interrupted}. This is not a failure of the test it stopped."
        );
    }
    if let Some(stuck) = &data.stuck {
        let _ = writeln!(out, "\n> **Stuck**: {stuck}.");
    }
    if !data.not_run.is_empty() {
        let _ = writeln!(
            out,
            "\n> {} of {} planned tests have no result: {}.",
            data.not_run.len(),
            data.plan_total,
            data.not_run
                .iter()
                .take(8)
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let of = |phase: Phase| -> Vec<&TestResult> {
        data.results.iter().filter(|r| r.phase == phase).collect()
    };
    page_table(
        &mut out,
        "Pages (tests/pages/mse-*.html)",
        &of(Phase::Pages),
    );
    page_table(
        &mut out,
        "Players (dash.js, Shaka Player, hls.js)",
        &of(Phase::Players),
    );
    render_wpt(&mut out, data, &totals, &of(Phase::Wpt));
    let _ = writeln!(out, "\n## Exit\n\nExit status {code}.");
    out
}

fn render_wpt(out: &mut String, data: &RunData, totals: &Totals, results: &[&TestResult]) {
    let _ = writeln!(out, "\n## WPT media-source/\n");
    if results.is_empty() && data.skipped.is_empty() {
        let _ = writeln!(out, "No test of this phase ran.");
        return;
    }
    let count = |outcome: Outcome| results.iter().filter(|r| r.outcome == outcome).count();
    let not_loaded = count(Outcome::NoLoad);
    let _ = writeln!(
        out,
        "- Files run: {} ({} loaded, {} never loaded, {} crashed, {} past the watchdog); skipped by the expectations file: {}; reftest pages left out: {}.",
        results.len(),
        results.len() - not_loaded,
        not_loaded,
        count(Outcome::Crash),
        count(Outcome::Timeout),
        data.skipped.len(),
        data.reftests
    );
    let percent = totals.subtest_percent();
    let _ = writeln!(
        out,
        "- Subtests: {} run, {} PASS (**{percent:.1}%**); F3 floor {F3_FLOOR_PERCENT:.0}%: {}.",
        totals.subtests,
        totals.subtests_passing,
        if totals.subtests == 0 {
            "no subtest ran"
        } else if percent >= F3_FLOOR_PERCENT {
            "met"
        } else {
            "NOT met"
        }
    );
    let _ = writeln!(
        out,
        "- Against `media-source.expect.toml`: {} finding(s) in {} file(s), of which {} unexpected pass(es).",
        data.findings.len(),
        data.findings
            .iter()
            .map(super::expect::Finding::path)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        totals.unexpected_passes
    );
    let peak = results
        .iter()
        .filter_map(|r| r.peak_rss_kb.map(|kb| (kb, &r.id)))
        .max();
    if let Some((kb, id)) = peak {
        let _ = writeln!(out, "- Peak RSS of any file: {} MB (`{id}`).", kb / 1024);
    }

    if !data.findings.is_empty() {
        let _ = writeln!(out, "\n### Unexpected results\n");
        for finding in &data.findings {
            let label = if finding.is_unexpected_pass() {
                "UNEXPECTED PASS"
            } else {
                "UNEXPECTED"
            };
            let _ = writeln!(out, "- {label}: {}", cell(&finding.describe()));
        }
    }
    let problems: Vec<&&TestResult> = results
        .iter()
        .filter(|r| {
            matches!(
                r.outcome,
                Outcome::NoLoad | Outcome::Crash | Outcome::Timeout
            )
        })
        .collect();
    if !problems.is_empty() {
        let _ = writeln!(out, "\n### Files that did not finish normally\n");
        let _ = writeln!(out, "| File | Result | Notes |\n|---|---|---|");
        for result in problems {
            let _ = writeln!(
                out,
                "| {} | {} | {} |",
                cell(&result.id),
                result.outcome.name(),
                cell(&result.notes.join("; "))
            );
        }
    }

    if let Some(expectations) = &data.expectations {
        let mut classes: BTreeMap<&str, usize> = BTreeMap::new();
        for entry in expectations.entries.values().filter(|e| !e.skip) {
            *classes.entry(entry.class.as_str()).or_default() += 1;
        }
        let _ = writeln!(out, "\n### Known failures by class\n");
        let _ = writeln!(out, "| Class | Files with an entry |\n|---|---|");
        for (class, count) in &classes {
            let _ = writeln!(out, "| {class} | {count} |");
        }
        let core: Vec<&str> = expectations
            .entries
            .values()
            .filter(|e| {
                !e.skip
                    && (e.class == "mse-core" || e.class == "dom")
                    && e.issue.is_none()
                    && covers_core(&e.path)
            })
            .map(|e| e.path.as_str())
            .collect();
        let _ = writeln!(
            out,
            "\n### F3: `mse-core` and `dom` entries in the core areas without an issue\n"
        );
        if core.is_empty() {
            let _ = writeln!(out, "None.");
        } else {
            let _ = writeln!(
                out,
                "{} entr{} (each blocks the flip unless it gets an `issue` or is fixed):\n",
                core.len(),
                if core.len() == 1 { "y" } else { "ies" }
            );
            for path in core {
                let _ = writeln!(out, "- `{path}`");
            }
        }
    }
    if !data.skipped.is_empty() {
        let _ = writeln!(out, "\n### Skipped by the expectations file\n");
        let mut by_class: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (path, class, _) in &data.skipped {
            by_class
                .entry(class.as_str())
                .or_default()
                .push(path.as_str());
        }
        for (class, paths) in by_class {
            let _ = writeln!(
                out,
                "- {class}: {} file(s), e.g. `{}`",
                paths.len(),
                paths[0]
            );
        }
    }
    if !data.stale_entries.is_empty() {
        let _ = writeln!(out, "\n### Entries that name no test of this checkout\n");
        for path in &data.stale_entries {
            let _ = writeln!(out, "- `{path}`");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mse_run::expect::{self, Finding};
    use crate::mse_run::model::Subtest;
    use crate::mse_run::phases::Planned;

    fn page(id: &str, outcome: Outcome) -> TestResult {
        let mut result = TestResult::new(Phase::Pages, id, outcome);
        result.lines = vec!["PASS a".into(), "FAIL b: nope".into(), "INFO c".into()];
        result.duration_ms = 2500;
        result.peak_rss_kb = Some(2048 * 100);
        result
    }

    #[test]
    fn utc_is_formatted_from_seconds() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_utc(1_795_345_691), "2026-11-22T11:08:11Z");
    }

    #[test]
    fn the_core_areas_are_recognised() {
        for path in [
            "media-source/mediasource-append-buffer.html",
            "media-source/SourceBuffer-abort.html",
            "media-source/mediasource-duration.html",
            "media-source/mediasource-endofstream.html",
            "media-source/mediasource-seekable.html",
            "media-source/mediasource-changetype.html",
            "media-source/mediasource-timestamp-offset.html",
            "media-source/SourceBuffer-appendWindowEnd-rounding.html",
        ] {
            assert!(covers_core(path), "{path}");
        }
        assert!(!covers_core("media-source/mediasource-avtracks.html"));
        assert!(!covers_core("media-source/idlharness.any.html"));
    }

    #[test]
    fn exit_zero_needs_a_clean_complete_run() {
        let mut data = RunData::default();
        data.results.push(page("mse-basic.html", Outcome::Pass));
        assert_eq!(exit_code(&data), 0);
        data.results.push(page("mse-seek.html", Outcome::Fail));
        assert_eq!(exit_code(&data), 1, "a failing page");
        data.results.pop();
        data.findings.push(Finding::UnexpectedPass {
            path: "media-source/a.html".into(),
            name: "x".into(),
        });
        assert_eq!(exit_code(&data), 1, "an unexpected pass is not clean");
        data.findings.clear();
        data.interrupted = Some("locked".into());
        assert_eq!(exit_code(&data), 3, "a lock means the run is not complete");
        data.interrupted = None;
        data.not_run.push(Planned {
            phase: Phase::Pages,
            id: "mse-quota.html".into(),
            url_path: "/pages/mse-quota.html".into(),
            timeout_factor: 1,
        });
        assert_eq!(exit_code(&data), 3);
    }

    #[test]
    fn the_report_has_a_table_per_phase_and_the_wpt_comparison() {
        let mut data = RunData {
            started: "2026-11-22T10:00:00Z".into(),
            finished: "2026-11-22T10:30:00Z".into(),
            browser: "/b/mercury-browser".into(),
            ..RunData::default()
        };
        data.results.push(page("mse-basic.html", Outcome::Pass));
        data.results.push(page("mse-seek.html", Outcome::Fail));
        let mut players = TestResult::new(Phase::Players, "dashjs.html", Outcome::Timeout);
        players.notes.push("no completion within 60 s".into());
        data.results.push(players);
        let mut wpt = TestResult::new(
            Phase::Wpt,
            "media-source/mediasource-duration.html",
            Outcome::Fail,
        );
        wpt.harness = Some("OK".into());
        wpt.subtests = vec![
            Subtest {
                name: "a".into(),
                status: "PASS".into(),
                message: None,
            },
            Subtest {
                name: "b".into(),
                status: "FAIL".into(),
                message: Some("assert".into()),
            },
        ];
        data.findings = expect::judge(None, &wpt);
        data.results.push(wpt);
        data.expectations = Some(
            expect::parse("[[test]]\npath=\"media-source/mediasource-append-buffer.html\"\nclass=\"dom\"\nreason=\"r\"\n[[test]]\npath=\"media-source/x.html\"\nclass=\"not-planned\"\nreason=\"r\"\nskip=true\n").unwrap(),
        );
        data.skipped.push((
            "media-source/x.html".into(),
            "not-planned".into(),
            "r".into(),
        ));
        let text = render(&data);
        for needle in [
            "# MSE window run 2026-11-22T10:00:00Z",
            "## Pages",
            "1 of 2 PASS",
            "| mse-seek.html | FAIL | 1 | 1 | 2.5 s | 200 MB |",
            "- `FAIL b: nope`",
            "## Players",
            "| dashjs.html | TIMEOUT |",
            "## WPT media-source/",
            "Subtests: 2 run, 1 PASS (**50.0%**)",
            "NOT met",
            "UNEXPECTED: media-source/mediasource-duration.html: \"b\" is FAIL, expected PASS",
            "| dom | 1 |",
            "`media-source/mediasource-append-buffer.html`",
            "not-planned: 1 file(s)",
            "**EXIT 1**",
        ] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
    }

    #[test]
    fn an_incomplete_run_says_so() {
        let mut data = RunData {
            interrupted: Some("the screen locked during pages mse-seek.html".into()),
            plan_total: 3,
            ..RunData::default()
        };
        data.not_run.push(Planned {
            phase: Phase::Pages,
            id: "mse-seek.html".into(),
            url_path: "/pages/mse-seek.html".into(),
            timeout_factor: 1,
        });
        let text = render(&data);
        assert!(text.contains("**EXIT 3**"), "{text}");
        assert!(text.contains("Interrupted by a screen lock"), "{text}");
        assert!(
            text.contains("1 of 3 planned tests have no result: mse-seek.html"),
            "{text}"
        );
    }

    #[test]
    fn table_cells_keep_their_pipes_and_length() {
        assert_eq!(cell("a|b\nc"), "a\\|b c");
        assert_eq!(cell(&"x".repeat(300)).chars().count(), 201);
    }
}
