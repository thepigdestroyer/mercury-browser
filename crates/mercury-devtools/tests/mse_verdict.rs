// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `mse-run verdict` run as a binary on a journal that a stub-browser dry run wrote: the exit
//! status and the report are what the lane lead reads after a window run.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const STUB: &str = include_str!("stub-browser.sh");

static ENV: Mutex<()> = Mutex::new(());

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
        write(&tools.join("wpt/media-source/a.html"), "<title>t</title>");
        write(&rig.path("expect.toml"), "");
        let stub = rig.path("stub-browser.sh");
        write(&stub, STUB);
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::process::Command::new(&stub).arg("--warm-up").output();
        rig
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn dry_run(&self) {
        let _guard = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_mse-run"))
            .args(["all", "--no-lock-check", "--timeout", "10"])
            .arg("--browser")
            .arg(self.path("stub-browser.sh"))
            .arg("--tools-dir")
            .arg(self.path("tools"))
            .arg("--expect")
            .arg(self.path("expect.toml"))
            .arg("--out")
            .arg(self.path("out"))
            .arg("--report")
            .arg(self.path("report.md"))
            .env_remove("STUB_CANNED")
            .env_remove("STUB_LOG")
            .output()
            .expect("run mse-run");
        assert!(status.status.code().is_some(), "dry run ran");
    }

    fn verdict(&self, extra: &[&str]) -> (Option<i32>, String, String) {
        let report_dir = self.path("out");
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_mse-run"))
            .arg("verdict")
            .arg("--out-dir")
            .arg(&report_dir)
            .arg("--journal")
            .arg(self.path("out/journal.jsonl"))
            .arg("--expect")
            .arg(self.path("expect.toml"))
            .arg("--tools-dir")
            .arg(self.path("tools"))
            .args(extra)
            .env_remove("STUB_CANNED")
            .env_remove("STUB_LOG")
            .output()
            .expect("run mse-run verdict");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn report_files(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(self.path("out"))
            .expect("out directory")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("flip-verdict-"))
            })
            .collect();
        found.sort();
        found
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn verdict_reads_a_dry_run_journal_and_writes_the_report() {
    let rig = Rig::new();
    rig.dry_run();
    let (code, stdout, stderr) = rig.verdict(&["--gates-green", "--review-ok"]);
    assert!(
        matches!(code, Some(0 | 1 | 3)),
        "exit status {code:?}\n{stdout}\n{stderr}"
    );
    assert!(stdout.contains("FLIP: "), "{stdout}");
    assert!(stdout.contains("## Conditions"), "{stdout}");
    let reports = rig.report_files();
    assert_eq!(reports.len(), 1, "{reports:?}");
    let text = fs::read_to_string(&reports[0]).expect("the report");
    assert!(text.contains("mse-basic.html"), "{text}");
    assert!(
        text.trim_end().ends_with(
            stdout
                .lines()
                .rev()
                .find(|l| l.starts_with("FLIP: "))
                .unwrap_or_default()
        )
    );
    assert!(stderr.contains("verdict written to"), "{stderr}");
}

#[test]
fn verdict_without_the_flags_is_never_go() {
    let rig = Rig::new();
    rig.dry_run();
    let (code, stdout, _) = rig.verdict(&[]);
    assert_ne!(code, Some(0), "{stdout}");
    assert!(stdout.contains("NOT PROVEN"), "{stdout}");
}

#[test]
fn verdict_with_no_journal_is_incomplete() {
    let rig = Rig::new();
    let (code, stdout, _) = rig.verdict(&["--gates-green", "--review-ok"]);
    assert_eq!(code, Some(3), "{stdout}");
    assert!(stdout.contains("FLIP: INCOMPLETE"), "{stdout}");
}

#[test]
fn verdict_refuses_an_unknown_argument() {
    let rig = Rig::new();
    let (code, _, stderr) = rig.verdict(&["--nope"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("unknown argument"), "{stderr}");
}
