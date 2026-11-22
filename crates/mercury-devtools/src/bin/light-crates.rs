// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The workspace members the fast CI jobs build: those whose dependency closure has neither
//! `servo`, `slint` nor `gstreamer`. Run through `scripts/ci/light-crates.sh`, which feeds it
//! `cargo metadata`.
//!
//!     cargo metadata --format-version 1 | light-crates            # -p talaria-core -p …
//!     cargo metadata --format-version 1 | light-crates --explain  # one line per member
//!
//! Exit status 1 when no member is light (a fast job with nothing to check would pass for the
//! wrong reason) or standard input is not cargo metadata, 2 for a wrong command line.

use std::io::Read;
use std::process::ExitCode;

use mercury_devtools::metadata::{self, HEAVY};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let explain = match arguments.as_slice() {
        [] => false,
        [option] if option == "--explain" => true,
        _ => {
            eprintln!("usage: light-crates [--explain] <cargo-metadata.json");
            return ExitCode::from(2);
        }
    };
    let mut input = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("light-crates: cannot read standard input: {error}");
        return ExitCode::FAILURE;
    }
    let verdicts = match metadata::workspace_verdicts(&input) {
        Ok(verdicts) => verdicts,
        Err(error) => {
            eprintln!("light-crates: {error}");
            return ExitCode::FAILURE;
        }
    };
    if explain {
        for verdict in &verdicts {
            println!("{}", verdict.explain());
        }
    }
    let light: Vec<&str> = verdicts
        .iter()
        .filter(|verdict| verdict.heavy.is_empty())
        .map(|verdict| verdict.name.as_str())
        .collect();
    if light.is_empty() {
        eprintln!(
            "light-crates: no workspace member is free of {}",
            HEAVY.join(", ")
        );
        return ExitCode::FAILURE;
    }
    if !explain {
        let arguments: Vec<String> = light.iter().map(|name| format!("-p {name}")).collect();
        println!("{}", arguments.join(" "));
    }
    ExitCode::SUCCESS
}
