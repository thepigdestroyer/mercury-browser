// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `scripts/dev/fetch-mse-tools.sh --check` (no network): it refuses a file whose SHA-256 is not
//! the pinned one, and a missing tool, and says which.
#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/dev/fetch-mse-tools.sh")
}

fn run(dir: &std::path::Path, arguments: &[&str]) -> Output {
    Command::new("sh")
        .arg(script())
        .args(arguments)
        .env("MSE_TOOLS_DIR", dir)
        .output()
        .expect("run the script")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn check_refuses_a_player_with_the_wrong_hash() {
    let dir = tempfile::tempdir().unwrap();
    let players = dir.path().join("players");
    fs::create_dir_all(&players).unwrap();
    fs::write(players.join("dash.all.min.js"), "this is not dash.js").unwrap();
    let output = run(dir.path(), &["--check"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let text = stderr(&output);
    assert!(
        text.contains("dash.js 5.3.0:")
            && text.contains("has SHA-256")
            && text.contains("the pin is dfd9f5cb"),
        "{text}"
    );
    assert!(text.contains("are NOT usable"), "{text}");
    assert!(
        fs::read(players.join("dash.all.min.js")).unwrap() == b"this is not dash.js",
        "--check changes nothing"
    );
}

#[test]
fn check_names_everything_that_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["--check"]);
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    for needle in [
        "wpt is missing",
        "dash.js 5.3.0:",
        "Shaka Player 5.3.6:",
        "hls.js 1.7.4:",
        "is missing",
    ] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    assert!(
        !dir.path().join("players").exists(),
        "--check downloads nothing and creates nothing"
    );
}

#[test]
fn check_refuses_a_wpt_checkout_at_another_commit() {
    let dir = tempfile::tempdir().unwrap();
    let wpt = dir.path().join("wpt");
    fs::create_dir_all(&wpt).unwrap();
    let git = |arguments: &[&str]| {
        let status = Command::new("git")
            .arg("-C")
            .arg(&wpt)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(arguments)
            .status()
            .expect("git");
        assert!(status.success(), "git {arguments:?}");
    };
    git(&["init", "--quiet"]);
    git(&["commit", "--quiet", "--allow-empty", "-m", "other"]);
    let output = run(dir.path(), &["--check"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("the pin is 42e27408034f57afe4c726476d1a0bd96b873129"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn usage_errors_and_help() {
    let dir = tempfile::tempdir().unwrap();
    let unknown = run(dir.path(), &["--nope"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(stderr(&unknown).contains("unknown option"));
    let help = run(dir.path(), &["-h"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--check"));
}
