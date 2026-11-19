// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Checks or installs Mercury Browser's bundled filter lists, for packaging.
//!
//! ```text
//! mercury-filter-lists check [<directory>]
//! mercury-filter-lists install <destination>
//! ```
//!
//! Run it from the repository root: the default paths are relative to it.
//!
//! `check` verifies that the manifest in `<directory>` (default
//! `crates/talaria-servo/resources/adblock`) is valid and that each list
//! matches its SHA-256. `install` checks the repository's lists, then replaces `<destination>`
//! with a directory holding exactly the manifest, those lists, `NOTICE.md` and the licence
//! texts from `LICENSES/` that they need (the macOS bundle's
//! `Contents/Resources/filter-lists`, the Linux archive's `filter-lists/`, or
//! `target/<profile>/filter-lists` for local runs). Exit status 1 on a failed check (nothing is
//! written), 2 on a usage error.

use std::path::Path;
use std::process::ExitCode;

use mercury_filter_lists::bundle::{LICENCES_DIR, SOURCE_DIR, check_dir, install};

const USAGE: &str = "usage (from the repository root):\n  mercury-filter-lists check [<directory>]\n  mercury-filter-lists install <destination>";

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let result = match arguments.as_slice() {
        ["check"] => check(Path::new(SOURCE_DIR)),
        ["check", directory] => check(Path::new(directory)),
        ["install", destination] => install(
            Path::new(SOURCE_DIR),
            Path::new(LICENCES_DIR),
            Path::new(destination),
        )
        .map(|count| println!("filter-lists: installed {count} lists in {destination}")),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("filter-lists: error: {problem}");
            ExitCode::FAILURE
        }
    }
}

fn check(directory: &Path) -> Result<(), String> {
    let files = check_dir(directory)?;
    println!(
        "filter-lists: {} lists match {}",
        files.len(),
        directory
            .join(mercury_filter_lists::MANIFEST_FILE)
            .display()
    );
    Ok(())
}
