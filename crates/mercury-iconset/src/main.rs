// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `mercury-iconset <icon.svg> <small-icon.svg> <AppIcon.iconset>`: writes the ten PNGs of a
//! macOS iconset. Sizes up to 32 px are drawn from the simplified small artwork.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [large, small, dir] = arguments.as_slice() else {
        eprintln!("usage: mercury-iconset <icon.svg> <small-icon.svg> <AppIcon.iconset>");
        return ExitCode::from(2);
    };
    match mercury_iconset::write_iconset(large, small, dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mercury-iconset: {error}");
            ExitCode::FAILURE
        }
    }
}
