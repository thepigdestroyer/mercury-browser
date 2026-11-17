// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Print the `manifest_path` of every package with the given name in `cargo metadata` output.
//!
//!     cargo metadata --format-version 1 | package-manifest servo
//!
//! One path per line; nothing when no package has that name. Exit status 2 for a wrong command
//! line, 1 when standard input is not cargo metadata.

use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [name] = arguments.as_slice() else {
        eprintln!("usage: package-manifest <package-name> <cargo-metadata.json");
        return ExitCode::from(2);
    };
    let mut metadata = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut metadata) {
        eprintln!("package-manifest: cannot read standard input: {error}");
        return ExitCode::FAILURE;
    }
    match mercury_devtools::metadata::manifest_paths(&metadata, name) {
        Ok(paths) => {
            for path in paths {
                println!("{path}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("package-manifest: {error}");
            ExitCode::FAILURE
        }
    }
}
