// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The comparing half of `scripts/ci/github-env.sh`, which runs the command, evaluates its
//! output and takes the two snapshots.
//!
//!     github-env <before> <after> [--print]
//!
//! `before` and `after` hold `env -0` output. Appends the variables and `PATH` directories the
//! command added to `$GITHUB_ENV` and `$GITHUB_PATH`, or with `--print` writes them to standard
//! output. Refuses, writing nothing, with exit status 1 when the change is not one
//! `mercury_devtools::github_env` allows; exit status 2 for a wrong command line.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use mercury_devtools::github_env::{self, Environment};

fn read_environment(path: &str) -> Result<Environment, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    github_env::parse_environment(&bytes).map_err(|error| format!("{path}: {error}"))
}

/// 128 random bits as hex, from the system's generator.
fn random_hex() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn append(variable: &str, text: &str) -> Result<(), String> {
    let path = std::env::var_os(variable)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| format!("{variable} is not set (use --print outside Actions)"))?;
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(Path::new(&path))
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| format!("cannot write {}: {error}", Path::new(&path).display()))
}

fn run(arguments: &[String]) -> Result<(), String> {
    let (before, after, print) = match arguments {
        [before, after] => (before, after, false),
        [before, after, option] if option == "--print" => (before, after, true),
        _ => return Err("usage".into()),
    };
    let before = read_environment(before)?;
    let after = read_environment(after)?;
    // `export` stops at the first failure to make a delimiter and returns its message, which
    // is told apart from a refusal here so that it does not read "refused: ...".
    let mut random_failed = false;
    let export = github_env::export(&before, &after, || {
        random_hex()
            .map(|hex| format!("MERCURY_ENV_{hex}"))
            .map_err(|error| {
                random_failed = true;
                format!("cannot get random bytes: {error}")
            })
    })
    .map_err(|reason| {
        if random_failed {
            reason
        } else {
            format!("refused: {reason}")
        }
    })?;
    for name in &export.names {
        eprintln!("github-env: exporting {name}");
    }
    eprintln!(
        "github-env: adding to PATH: {}",
        export.path_prefix.join(":")
    );
    if print {
        print!(
            "# for $GITHUB_ENV\n{}# for $GITHUB_PATH\n{}",
            export.env_text, export.path_text
        );
        return Ok(());
    }
    append("GITHUB_ENV", &export.env_text)?;
    append("GITHUB_PATH", &export.path_text)
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error == "usage" => {
            eprintln!("usage: github-env <before> <after> [--print]");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("github-env: {error}");
            ExitCode::FAILURE
        }
    }
}
