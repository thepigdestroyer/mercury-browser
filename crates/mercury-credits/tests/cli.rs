// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The command line through the real binary: usage errors, exit statuses, and a whole run against a
//! stand-in `cargo`.
mod common;

use common::{APACHE, Fake, MIT};
use mercury_credits::cli::{self, Command, Parsed};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Output};

fn binary() -> Process {
    Process::new(env!("CARGO_BIN_EXE_mercury-credits"))
}

fn run(arguments: &[&str]) -> Output {
    binary().args(arguments).output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn bad_usage_exits_2_and_prints_the_usage() {
    let cases: &[&[&str]] = &[
        &[],
        &["frobnicate"],
        &["check"],
        &["check", "--package", "app"],
        &["check", "--target", "t"],
        &["generate", "--package", "app", "--target", "t"],
        &[
            "check",
            "--package",
            "app",
            "--target",
            "t",
            "--out",
            "/tmp/x",
        ],
        &["check", "--package", "app", "--target", "t", "--bogus"],
        &["check", "--package"],
        &[
            "check",
            "--package",
            "app",
            "--target",
            "t",
            "--runtime-only=yes",
        ],
        &["generate", "--package", "app", "--target", "t", "--out"],
    ];
    for arguments in cases {
        let output = run(arguments);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?}: {}",
            stderr(&output)
        );
        let text = stderr(&output);
        assert!(text.contains("mercury-credits: "), "{arguments:?}: {text}");
        assert!(
            text.contains("usage: mercury-credits"),
            "{arguments:?}: {text}"
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn help_prints_the_usage_on_standard_output_and_exits_0() {
    for flag in ["--help", "-h", "help"] {
        let output = run(&[flag]);
        assert_eq!(output.status.code(), Some(0), "{flag}");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("generate --out <dir>") && text.contains("--runtime-only"),
            "{text}"
        );
        assert!(text.contains("Exit status: 0 success"));
    }
}

#[test]
fn the_arguments_parse_into_the_invocation() {
    let parsed = cli::parse(
        [
            "generate",
            "--package",
            "mercury-desktop",
            "--target=aarch64-apple-darwin",
            "--out",
            "dist/credits",
            "--runtime-only",
            "--manifest-path",
            "/work/Cargo.toml",
        ],
        PathBuf::from("cargo"),
    )
    .unwrap();
    let Parsed::Run(invocation) = parsed else {
        panic!("not a run")
    };
    assert_eq!(invocation.command, Command::Generate);
    assert_eq!(invocation.options.package, "mercury-desktop");
    assert_eq!(invocation.options.target, "aarch64-apple-darwin");
    assert!(invocation.options.runtime_only);
    assert_eq!(
        invocation.options.manifest_path,
        Path::new("/work/Cargo.toml")
    );
    assert_eq!(invocation.out.as_deref(), Some(Path::new("dist/credits")));

    let Parsed::Run(check) = cli::parse(
        ["check", "-p", "app", "--target", "t"],
        PathBuf::from("cargo"),
    )
    .unwrap() else {
        panic!("not a run")
    };
    assert_eq!(check.command, Command::Check);
    assert!(!check.options.runtime_only);
    assert!(
        check.options.manifest_path.is_absolute(),
        "the default manifest is made absolute"
    );
    assert!(check.options.manifest_path.ends_with("Cargo.toml"));
    assert!(check.out.is_none());
}

#[test]
fn a_cargo_that_cannot_run_exits_1_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let output = binary()
        .env("CARGO", "/definitely/not/a/cargo")
        .args(["generate", "--package", "app", "--target", "t", "--out"])
        .arg(&out)
        .arg("--manifest-path")
        .arg(dir.path().join("Cargo.toml"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("mercury-credits: problem: cannot run"));
    assert!(!out.exists(), "nothing is written on a problem");
}

#[cfg(unix)]
mod with_a_stand_in_cargo {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A `cargo` that answers `tree` and `metadata` from files and logs its arguments.
    fn stand_in(fake: &Fake, scripts: &Path) -> PathBuf {
        let packages: Vec<serde_json::Value> = fake
            .packages
            .iter()
            .map(|package| {
                serde_json::json!({
                    "id": package.id,
                    "name": package.name,
                    "version": package.version,
                    "source": package.source,
                    "license": package.license,
                    "license_file": package.license_file,
                    "authors": package.authors,
                    "repository": package.repository,
                    "manifest_path": package.manifest_path,
                    "links": package.links,
                })
            })
            .collect();
        let metadata = serde_json::json!({ "packages": packages, "workspace_members": fake.metadata().workspace_members });
        std::fs::write(scripts.join("tree.txt"), fake.tree_text()).unwrap();
        std::fs::write(scripts.join("metadata.json"), metadata.to_string()).unwrap();
        let script = scripts.join("cargo");
        std::fs::write(
            &script,
            // Like the real cargo, it writes the `(*)` marker of `cargo tree` in colour when
            // CARGO_TERM_COLOR=always, unless it is given `--color never`. That was the first CI
            // run's failure: the colour codes made the lines of the tree unreadable to the parser.
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$(dirname "$0")/arguments.log"
colour="${CARGO_TERM_COLOR:-auto}"
case " $* " in *" --color never "*) colour=never ;; esac
case "$1" in
  tree)
    if [ "$colour" = always ]; then
      while IFS= read -r line; do
        printf '%s \033[33m\033[2m(*)\033[39m\033[22m\n' "$line"
      done < "$(dirname "$0")/tree.txt"
    else
      cat "$(dirname "$0")/tree.txt"
    fi ;;
  metadata) cat "$(dirname "$0")/metadata.json" ;;
  *) echo "unexpected: $*" >&2; exit 9 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn fake_with_crates() -> Fake {
        let mut fake = Fake::new();
        fake.add("alpha", "1.0.0", "MIT OR Apache-2.0");
        fake.crate_file("alpha", "LICENSE-MIT", MIT);
        fake.crate_file("alpha", "LICENSE-APACHE", APACHE);
        fake
    }

    #[test]
    fn generate_writes_the_page_and_ends_with_the_summary_line() {
        let fake = fake_with_crates();
        let scripts = tempfile::tempdir().unwrap();
        let cargo = stand_in(&fake, scripts.path());
        let out = tempfile::tempdir().unwrap().keep().join("credits");
        let output = binary()
            .env("CARGO", &cargo)
            .args([
                "generate",
                "--package",
                "mercury-desktop",
                "--target",
                "aarch64-apple-darwin",
                "--out",
            ])
            .arg(&out)
            .arg("--manifest-path")
            .arg(fake.root().join("Cargo.toml"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let text = stderr(&output);
        let last = text.lines().last().unwrap();
        assert!(
            last.starts_with("mercury-credits: 2 crates credited (1 registry, 0 vendored, 1 Mercury's own), 2 distinct licence texts, 0 crates on a standard text, "),
            "{last}"
        );
        assert!(last.ends_with(" bytes"));
        let page = std::fs::read_to_string(out.join("credits.html")).unwrap();
        assert!(page.contains("<meta name=\"credits-crates\" content=\"2\">"));
        assert!(last.contains(&format!(", {} bytes", page.len())));

        let log = std::fs::read_to_string(scripts.path().join("arguments.log")).unwrap();
        let manifest = fake.root().join("Cargo.toml");
        assert!(
            log.contains(&format!(
                "tree --locked --color never --manifest-path {} -p mercury-desktop -e normal,build --target aarch64-apple-darwin --prefix none --format {{p}}",
                manifest.display()
            )),
            "{log}"
        );
        assert!(
            log.contains(&format!(
                "metadata --locked --color never --manifest-path {} --format-version 1",
                manifest.display()
            )),
            "{log}"
        );
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn cargo_is_asked_for_plain_text_even_when_the_environment_asks_for_colour() {
        // CI sets CARGO_TERM_COLOR=always; cargo then colours the `(*)` of `cargo tree`.
        let fake = fake_with_crates();
        let scripts = tempfile::tempdir().unwrap();
        let cargo = stand_in(&fake, scripts.path());
        let run = |colour: &str, extra: &[&str]| {
            binary()
                .env("CARGO", &cargo)
                .env("CARGO_TERM_COLOR", colour)
                .args([
                    "check",
                    "--package",
                    "app",
                    "--target",
                    "x86_64-unknown-linux-gnu",
                ])
                .args(extra)
                .arg("--manifest-path")
                .arg(fake.root().join("Cargo.toml"))
                .output()
                .unwrap()
        };
        for colour in ["always", "auto", "never"] {
            let output = run(colour, &[]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "CARGO_TERM_COLOR={colour}: {}",
                stderr(&output)
            );
        }
        let output = run("always", &["--runtime-only"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let log = std::fs::read_to_string(scripts.path().join("arguments.log")).unwrap();
        assert!(
            log.contains(" --locked --color never --manifest-path "),
            "{log}"
        );
    }

    #[test]
    fn the_stand_in_really_colours_the_tree_without_the_flag() {
        // The test above proves nothing if the stand-in never colours: ask it as the tool used to.
        let fake = fake_with_crates();
        let scripts = tempfile::tempdir().unwrap();
        let cargo = stand_in(&fake, scripts.path());
        let coloured = std::process::Command::new(&cargo)
            .env("CARGO_TERM_COLOR", "always")
            .args(["tree", "--locked", "--prefix", "none"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&coloured.stdout).into_owned();
        assert!(
            text.contains("\u{1b}[33m\u{1b}[2m(*)\u{1b}[39m\u{1b}[22m"),
            "{text:?}"
        );
        let plain = std::process::Command::new(&cargo)
            .env("CARGO_TERM_COLOR", "always")
            .args(["tree", "--locked", "--color", "never"])
            .output()
            .unwrap();
        assert!(!String::from_utf8_lossy(&plain.stdout).contains('\u{1b}'));
    }

    #[test]
    fn runtime_only_asks_cargo_for_normal_edges_only() {
        let fake = fake_with_crates();
        let scripts = tempfile::tempdir().unwrap();
        let cargo = stand_in(&fake, scripts.path());
        let output = binary()
            .env("CARGO", &cargo)
            .args([
                "check",
                "--package",
                "app",
                "--target",
                "x86_64-unknown-linux-gnu",
                "--runtime-only",
                "--manifest-path",
            ])
            .arg(fake.root().join("Cargo.toml"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let log = std::fs::read_to_string(scripts.path().join("arguments.log")).unwrap();
        assert!(
            log.contains(" -e normal --target x86_64-unknown-linux-gnu"),
            "{log}"
        );
        assert!(!log.contains("normal,build"));
    }

    #[test]
    fn check_prints_the_summary_and_writes_nothing() {
        let fake = fake_with_crates();
        let scripts = tempfile::tempdir().unwrap();
        let cargo = stand_in(&fake, scripts.path());
        let before: Vec<_> = std::fs::read_dir(fake.root())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let output = binary()
            .env("CARGO", &cargo)
            .current_dir(fake.root())
            .args([
                "check",
                "--package",
                "app",
                "--target",
                "aarch64-apple-darwin",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert!(
            stderr(&output)
                .lines()
                .last()
                .unwrap()
                .starts_with("mercury-credits: 2 crates credited")
        );
        let after: Vec<_> = std::fs::read_dir(fake.root())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(before.len(), after.len(), "check wrote nothing");
    }

    #[test]
    fn a_problem_exits_1_prints_it_with_the_crate_and_writes_nothing() {
        let mut fake = fake_with_crates();
        fake.add("copyleft", "3.0.0", "GPL-3.0-only");
        let scripts = tempfile::tempdir().unwrap();
        let cargo = stand_in(&fake, scripts.path());
        let out = tempfile::tempdir().unwrap().keep().join("credits");
        let output = binary()
            .env("CARGO", &cargo)
            .args([
                "generate",
                "--package",
                "app",
                "--target",
                "aarch64-apple-darwin",
                "--out",
            ])
            .arg(&out)
            .arg("--manifest-path")
            .arg(fake.root().join("Cargo.toml"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let text = stderr(&output);
        assert!(
            text.contains(
                "mercury-credits: problem: copyleft 3.0.0: no licence of `GPL-3.0-only` is allowed"
            ),
            "{text}"
        );
        assert!(text.contains("1 problems; nothing was written"));
        assert!(!out.exists());
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }
}
