// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `mse-run` against a stub browser (`tests/stub-browser.sh`, which posts canned reports) and a
//! fake `ioreg`: the plan, the journal and `--resume`, the lock handling, the watchdog, the
//! comparison with the expectations and the report. No browser is started.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mercury_devtools::mse_run::journal::Journal;
use mercury_devtools::mse_run::model::{Outcome, Phase};
use mercury_devtools::mse_run::runner::{self, Config, RunData, RunError};
use mercury_devtools::mse_run::summary;

const STUB: &str = include_str!("stub-browser.sh");

const LOCKED: &str = "\"IOConsoleUsers\" = ({\"CGSSessionScreenIsLocked\"=Yes})";
const UNLOCKED: &str = "\"IOConsoleUsers\" = ({\"kCGSSessionOnConsoleKey\"=Yes})";

struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().expect("temporary directory");
        let rig = Rig { dir };
        let tests = rig.path("tests");
        for page in ["mse-basic.html", "mse-seek.html"] {
            write(&tests.join("pages").join(page), "<!doctype html>");
        }
        write(
            &tests.join("pages/mse-players/dashjs.html"),
            "<!doctype html>",
        );
        let tools = rig.path("tools");
        for player in ["dash.all.min.js", "shaka-player.compiled.js", "hls.min.js"] {
            write(&tools.join("players").join(player), "//");
        }
        write(&tools.join("wpt/resources/testharness.js"), "//");
        for name in ["a.html", "b.html", "c.html", "d.html", "e.html"] {
            write(
                &tools.join("wpt/media-source").join(name),
                "<title>t</title>",
            );
        }
        write(
            &tools.join("wpt/media-source/f.any.js"),
            "// META: global=window\n",
        );
        write(
            &tools.join("wpt/media-source/skipped.html"),
            "<title>s</title>",
        );
        fs::create_dir_all(rig.path("out")).unwrap();
        write(&rig.path("expect.toml"), "");
        let stub = rig.path("stub-browser.sh");
        write(&stub, STUB);
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        rig
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn config(&self) -> Config {
        Config {
            tests_root: self.path("tests"),
            tools_dir: self.path("tools"),
            browser: self.path("stub-browser.sh"),
            browser_args: Vec::new(),
            phases: Phase::ALL.to_vec(),
            only: None,
            timeout: Duration::from_secs(20),
            grace: Duration::from_millis(300),
            kill_wait: Duration::from_secs(5),
            resume: false,
            out_dir: self.path("out"),
            journal: self.path("out/journal.jsonl"),
            expect: self.path("expect.toml"),
            ioreg: None,
        }
    }

    fn canned(&self, name: &str, body: &str) {
        write(&self.path("canned").join(name), body);
    }

    /// A fake ioreg that says unlocked for the first `unlocked_calls` runs and locked after.
    fn ioreg(&self, unlocked_calls: u32) -> PathBuf {
        let state = self.path("ioreg-state");
        fs::create_dir_all(&state).unwrap();
        fs::write(state.join("count"), "0").unwrap();
        let script = self.path("ioreg");
        write(
            &script,
            &format!(
                "#!/bin/sh\nn=$(cat '{0}/count')\necho $((n + 1)) > '{0}/count'\nif [ \"$n\" -lt {unlocked_calls} ]; then echo '{UNLOCKED}'; else echo '{LOCKED}'; fi\n",
                state.display()
            ),
        );
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn run(&self, config: &Config) -> Result<RunData, RunError> {
        // The stub reads these; a test that needs them set sets them before calling.
        let mut log = |_: &str| {};
        runner::run(config, &mut log)
    }

    fn started(&self) -> Vec<String> {
        fs::read_to_string(self.path("stub.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// The stub's environment is the test process's: serialise the tests that set it.
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_stub_env<T>(rig: &Rig, body: impl FnOnce() -> T) -> T {
    let _guard = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // SAFETY: the tests that touch the environment hold ENV, and nothing else here reads it
    // concurrently (the child processes inherit it when they start).
    unsafe {
        std::env::set_var("STUB_CANNED", rig.path("canned"));
        std::env::set_var("STUB_LOG", rig.path("stub.log"));
    }
    let result = body();
    unsafe {
        std::env::remove_var("STUB_CANNED");
        std::env::remove_var("STUB_LOG");
    }
    result
}

fn outcome(data: &RunData, id: &str) -> Outcome {
    data.results
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| {
            panic!(
                "no result for {id}: {:?}",
                data.results.iter().map(|r| &r.id).collect::<Vec<_>>()
            )
        })
        .outcome
}

const PAGE_FAIL: &str = r#"{"token":"@TOKEN@","kind":"hello","title":""}
{"token":"@TOKEN@","kind":"page","title":"p: 1 PASS, 1 FAIL (done)","lines":["PASS one","FAIL two: nope","INFO three"],"errors":[]}
"#;

fn wpt_report(harness: i32, tests: &[(&str, i32)]) -> String {
    let tests: Vec<String> = tests
        .iter()
        .map(|(name, status)| {
            format!("{{\"name\":\"{name}\",\"status\":{status},\"message\":null}}")
        })
        .collect();
    format!(
        "{{\"token\":\"@TOKEN@\",\"kind\":\"hello\"}}\n{{\"token\":\"@TOKEN@\",\"kind\":\"wpt\",\"title\":\"t\",\"harness\":{{\"status\":{harness},\"message\":null}},\"tests\":[{}]}}\n",
        tests.join(",")
    )
}

#[test]
fn a_dry_run_with_every_kind_of_result_prints_a_complete_report() {
    let rig = Rig::new();
    write(
        &rig.path("expect.toml"),
        r#"
[[test]]
path = "media-source/a.html"
class = "dom"
reason = "a known failure"
fail = ["known"]

[[test]]
path = "media-source/c.html"
class = "mse-core"
reason = "expected to fail, but the stub passes it"
fail = ["was failing"]

[[test]]
path = "media-source/skipped.html"
class = "not-planned"
reason = "not planned"
skip = true

[[test]]
path = "media-source/gone.html"
class = "test-bug"
reason = "names a file that does not exist"
"#,
    );
    rig.canned("pages_mse-seek.html", PAGE_FAIL);
    rig.canned(
        "media-source_a.html",
        &wpt_report(0, &[("known", 1), ("fine", 0)]),
    );
    rig.canned(
        "media-source_b.html",
        &wpt_report(0, &[("new failure", 1), ("fine", 0)]),
    );
    rig.canned("media-source_c.html", &wpt_report(0, &[("was failing", 0)]));
    rig.canned("media-source_d.html", "#hang\n");
    rig.canned("media-source_e.html", "#crash\n");
    let mut config = rig.config();
    config.timeout = Duration::from_secs(2);
    let data = with_stub_env(&rig, || rig.run(&config)).expect("the run completes");

    assert_eq!(outcome(&data, "mse-basic.html"), Outcome::Pass);
    assert_eq!(outcome(&data, "mse-seek.html"), Outcome::Fail);
    assert_eq!(outcome(&data, "dashjs.html"), Outcome::Pass);
    assert_eq!(
        outcome(&data, "media-source/a.html"),
        Outcome::Fail,
        "its failure is a known one"
    );
    assert_eq!(outcome(&data, "media-source/d.html"), Outcome::Timeout);
    assert_eq!(outcome(&data, "media-source/e.html"), Outcome::Crash);
    assert_eq!(
        outcome(&data, "media-source/f.any.html"),
        Outcome::Pass,
        "a generated page runs too"
    );
    assert!(data.not_run.is_empty());
    assert_eq!(data.skipped.len(), 1);
    assert_eq!(data.stale_entries, ["media-source/gone.html"]);

    let findings: Vec<String> = data.findings.iter().map(|f| f.describe()).collect();
    for needle in [
        "media-source/b.html: \"new failure\" is FAIL, expected PASS",
        "media-source/c.html: \"was failing\" passes, expected a failure (unexpected pass)",
        "media-source/d.html: harness TIMEOUT, expected OK",
        "media-source/e.html: harness CRASH, expected OK",
    ] {
        assert!(
            findings.iter().any(|f| f == needle),
            "{needle}\n{findings:#?}"
        );
    }
    assert!(
        !findings
            .iter()
            .any(|f| f.starts_with("media-source/a.html")),
        "{findings:#?}"
    );
    assert_eq!(summary::exit_code(&data), 1);

    let report = summary::render(&data);
    for needle in [
        "## Pages",
        "1 of 2 PASS",
        "| mse-seek.html | FAIL | 1 | 1 |",
        "- `FAIL two: nope`",
        "## Players",
        "1 of 1 PASS",
        "## WPT media-source/",
        "UNEXPECTED PASS: media-source/c.html",
        "UNEXPECTED: media-source/b.html",
        "Entries that name no test of this checkout",
        "`media-source/gone.html`",
        "not-planned: 1 file(s)",
        "**EXIT 1**",
    ] {
        assert!(report.contains(needle), "{needle}\n{report}");
    }
    // Every browser run left a journal line, and the stub was started once per test.
    let journal = Journal::new(&config.journal).load().unwrap();
    assert_eq!(journal.results.len(), 9);
    assert_eq!(rig.started().len(), 9);
}

#[test]
fn a_clean_run_exits_zero() {
    let rig = Rig::new();
    let mut config = rig.config();
    config.phases = vec![Phase::Pages, Phase::Players];
    let data = with_stub_env(&rig, || rig.run(&config)).unwrap();
    assert_eq!(data.results.len(), 3);
    assert!(data.results.iter().all(|r| r.outcome == Outcome::Pass));
    assert!(data.results.iter().all(|r| r.duration_ms > 0));
    assert_eq!(summary::exit_code(&data), 0);
    assert!(summary::render(&data).contains("**EXIT 0**"));
}

#[test]
fn it_refuses_to_start_on_a_locked_screen() {
    let rig = Rig::new();
    let mut config = rig.config();
    config.ioreg = Some(rig.ioreg(0));
    let error = with_stub_env(&rig, || rig.run(&config)).expect_err("locked");
    assert!(matches!(error, RunError::Locked(_)), "{error}");
    assert!(error.to_string().contains("locked"), "{error}");
    assert!(rig.started().is_empty(), "no browser was started");
    assert!(!config.journal.exists(), "nothing was journalled");
}

#[test]
fn a_lock_between_tests_stops_the_run_cleanly_and_resume_finishes_it() {
    let rig = Rig::new();
    let mut config = rig.config();
    config.phases = vec![Phase::Pages, Phase::Players];
    // Calls to ioreg: the preflight, then one before each test. The third test sees the lock.
    config.ioreg = Some(rig.ioreg(3));
    let first = with_stub_env(&rig, || rig.run(&config)).unwrap();
    assert_eq!(first.results.len(), 2);
    assert_eq!(first.not_run.len(), 1);
    assert!(
        first
            .interrupted
            .as_deref()
            .is_some_and(|text| text.contains("locked before")),
        "{:?}",
        first.interrupted
    );
    assert_eq!(
        summary::exit_code(&first),
        3,
        "an interrupted run is not a pass"
    );
    let report = summary::render(&first);
    assert!(report.contains("Interrupted by a screen lock"), "{report}");
    assert!(report.contains("**EXIT 3**"));
    assert_eq!(rig.started().len(), 2);

    // The screen is unlocked again: --resume runs only what is missing.
    config.ioreg = None;
    config.resume = true;
    let second = with_stub_env(&rig, || rig.run(&config)).unwrap();
    assert_eq!(second.resumed, 2);
    assert_eq!(second.results.len(), 3);
    assert!(second.interrupted.is_none() && second.not_run.is_empty());
    assert_eq!(
        rig.started().len(),
        3,
        "only the missing test started a browser"
    );
    assert_eq!(summary::exit_code(&second), 0);
}

#[test]
fn a_lock_during_a_test_is_an_interruption_and_not_a_failure() {
    let rig = Rig::new();
    rig.canned("pages_mse-basic.html", "#hang\n");
    let mut config = rig.config();
    config.phases = vec![Phase::Pages];
    config.timeout = Duration::from_secs(1);
    // preflight and the check before the first test see an unlocked screen; the check after
    // its timeout sees the lock.
    config.ioreg = Some(rig.ioreg(2));
    let data = with_stub_env(&rig, || rig.run(&config)).unwrap();
    assert!(
        data.interrupted
            .as_deref()
            .is_some_and(|t| t.contains("locked during")),
        "{:?}",
        data.interrupted
    );
    assert!(
        data.results.is_empty(),
        "the timed-out test has no verdict: {:?}",
        data.results
    );
    assert_eq!(data.not_run.len(), 2);
    assert_eq!(summary::exit_code(&data), 3);
    let journal = Journal::new(&config.journal).load().unwrap();
    assert_eq!(journal.results[0].outcome, Outcome::Interrupted);
    assert!(
        journal.done(Phase::Pages, "mse-basic.html").is_none(),
        "it runs again with --resume"
    );
    assert_eq!(rig.started().len(), 1, "the run stopped there");
}

#[test]
fn a_page_that_never_reports_is_noload_and_a_stubborn_browser_is_killed() {
    let rig = Rig::new();
    rig.canned("pages_mse-basic.html", "#stubborn\n");
    rig.canned("pages_mse-seek.html", "#silent\n");
    let mut config = rig.config();
    config.phases = vec![Phase::Pages];
    config.timeout = Duration::from_secs(1);
    let started = Instant::now();
    let data = with_stub_env(&rig, || rig.run(&config)).unwrap();
    assert_eq!(outcome(&data, "mse-basic.html"), Outcome::NoLoad);
    assert_eq!(outcome(&data, "mse-seek.html"), Outcome::NoLoad);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "SIGKILL ended the browser that ignored SIGTERM"
    );
    let notes = data.results[0].notes.join(" ");
    assert!(notes.contains("never reported"), "{notes}");
    assert_eq!(summary::exit_code(&data), 1);
}

#[test]
fn only_selects_tests_by_name() {
    let rig = Rig::new();
    let mut config = rig.config();
    config.only = Some("mse-seek".into());
    let data = with_stub_env(&rig, || rig.run(&config)).unwrap();
    assert_eq!(data.results.len(), 1);
    assert_eq!(data.results[0].id, "mse-seek.html");
    assert!(
        data.stale_entries.is_empty(),
        "no stale check on a partial run"
    );
    let listed = runner::list(&config).unwrap();
    assert_eq!(listed.len(), 1);
}

#[test]
fn the_preflight_names_everything_that_is_missing() {
    let rig = Rig::new();
    let mut config = rig.config();
    config.browser = rig.path("no-browser");
    config.tools_dir = rig.path("no-tools");
    write(
        &rig.path("expect.toml"),
        "[[test]]\npath = \"media-source/a.html\"\nreason = \"no class\"\n",
    );
    let error = rig.run(&config).expect_err("preflight");
    let RunError::Preflight(problems) = &error else {
        panic!("{error}");
    };
    let text = problems.join("\n");
    for needle in [
        "no-browser",
        "dash.all.min.js",
        "no-tools/wpt",
        "fetch-mse-tools.sh",
        "no class",
    ] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    assert!(rig.started().is_empty());
}

#[test]
fn an_unreadable_journal_line_does_not_stop_resume() {
    let rig = Rig::new();
    let mut config = rig.config();
    config.phases = vec![Phase::Pages];
    config.resume = true;
    write(
        &config.journal,
        "{\"phase\":\"pages\",\"id\":\"mse-basic.ht",
    );
    let data = with_stub_env(&rig, || rig.run(&config)).unwrap();
    assert_eq!(data.unreadable_journal_lines, 1);
    assert_eq!(data.results.len(), 2);
    assert!(summary::render(&data).contains("1 unreadable line(s)"));
}

#[test]
fn an_old_journal_is_kept_aside_when_a_run_starts_fresh() {
    let rig = Rig::new();
    let mut config = rig.config();
    config.phases = vec![Phase::Pages];
    with_stub_env(&rig, || rig.run(&config)).unwrap();
    with_stub_env(&rig, || rig.run(&config)).unwrap();
    let kept = fs::read_dir(rig.path("out"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".old"))
        .count();
    assert_eq!(kept, 1);
    assert_eq!(
        Journal::new(&config.journal).load().unwrap().results.len(),
        2
    );
}

#[test]
fn the_command_line_runs_the_whole_dry_run_and_writes_the_report() {
    let rig = Rig::new();
    let report = rig.path("report.md");
    let status = {
        let _guard = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        std::process::Command::new(env!("CARGO_BIN_EXE_mse-run"))
            .args(["all", "--no-lock-check", "--timeout", "10"])
            .arg("--browser")
            .arg(rig.path("stub-browser.sh"))
            .arg("--tools-dir")
            .arg(rig.path("tools"))
            .arg("--expect")
            .arg(rig.path("expect.toml"))
            .arg("--out")
            .arg(rig.path("out"))
            .arg("--report")
            .arg(&report)
            .env_remove("STUB_CANNED")
            .env_remove("STUB_LOG")
            .output()
            .expect("run mse-run")
    };
    let stderr = String::from_utf8_lossy(&status.stderr);
    // The real tests/pages of the repository are planned (the stub passes them all); the tools are the fake ones.
    assert_eq!(status.status.code(), Some(0), "{stderr}");
    let text = fs::read_to_string(&report).expect("the report");
    for needle in [
        "## Pages",
        "## Players",
        "## WPT media-source/",
        "**EXIT 0**",
        "mse-basic.html",
    ] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    assert!(stderr.contains("report written to"), "{stderr}");
}

#[test]
fn the_command_line_refuses_without_a_phase_and_prints_help() {
    let bare = std::process::Command::new(env!("CARGO_BIN_EXE_mse-run"))
        .output()
        .unwrap();
    assert_eq!(bare.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bare.stderr).contains("name a phase"));
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_mse-run"))
        .arg("--help")
        .output()
        .unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--resume"));
}
