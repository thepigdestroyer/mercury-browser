// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Compare two folders of PNG pictures pixel by pixel.
//!
//!     cargo run --release -p mercury-devtools --bin pixel-diff -- <before> <after> \
//!         [--tolerance N] [--write-diffs <dir>]
//!
//! Exit status: 0 when every file is identical, 1 when anything differs or is missing on one
//! side, 2 for a wrong command line or an unreadable file. See `mercury_devtools::pixel_diff`.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (status, out, err) = mercury_devtools::pixel_diff::run(&arguments);
    let _ = std::io::stdout().write_all(out.as_bytes());
    let _ = std::io::stderr().write_all(err.as_bytes());
    ExitCode::from(status as u8)
}
