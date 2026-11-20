// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The HSTS preload list tool: `build`, `check` and `diff` (see `mercury_devtools::hsts`).
//!
//!     hsts-preload build --input <file|-> [--base64] --output <file.fstmap> [options]
//!     hsts-preload check <file.fstmap> [options]
//!     hsts-preload diff <old.fstmap> <new.fstmap> [--limit N | --all]
//!     hsts-preload --help
//!
//! Exit status 0 when done, 1 for a data problem (a refused build, a failed check, a file that
//! cannot be read), 2 for a wrong command line. `scripts/dev/update-hsts-preload.sh` (`make
//! hsts-preload-refresh`) is the maintainer's way to run it on a fresh download.

use std::process::ExitCode;

use mercury_devtools::hsts::cli;
use mercury_devtools::hsts::manifest::Date;

fn main() -> ExitCode {
    let mut arguments = Vec::new();
    for argument in std::env::args_os().skip(1) {
        match argument.into_string() {
            Ok(argument) => arguments.push(argument),
            Err(argument) => {
                eprintln!("hsts-preload: an argument is not valid Unicode: {argument:?}");
                return ExitCode::from(2);
            }
        }
    }
    let status = cli::run(
        &arguments,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
        Date::today_utc(),
    );
    ExitCode::from(status)
}
