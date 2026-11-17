// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Check a screenshot of `tests/pages/colors.html` (the CI smoke test).
//!
//!     cargo run --release -p mercury-devtools --bin check-colors-screenshot -- <screenshot.png>
//!
//! Prints one line per check. Exit status: 0 when every check passes, 1 when the picture is
//! wrong, 2 when the file is not a usable PNG or the command line is wrong. See
//! `mercury_devtools::colors`.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args();
    let program = arguments
        .next()
        .unwrap_or_else(|| "check-colors-screenshot".to_owned());
    let arguments: Vec<String> = arguments.collect();
    let (status, out, err) = mercury_devtools::colors::run(&program, &arguments);
    let _ = std::io::stdout().write_all(out.as_bytes());
    let _ = std::io::stderr().write_all(err.as_bytes());
    ExitCode::from(status as u8)
}
