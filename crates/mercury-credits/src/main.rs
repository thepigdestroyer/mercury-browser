// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use mercury_credits::app;
use mercury_credits::cli::{self, Command, Parsed};
use mercury_credits::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cargo = std::env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from);
    let parsed = match cli::parse(std::env::args().skip(1), cargo) {
        Ok(parsed) => parsed,
        Err(Error::Usage(message)) => {
            eprintln!("mercury-credits: {message}");
            eprint!("{}", cli::USAGE);
            return ExitCode::from(2);
        }
        Err(error) => {
            eprintln!("mercury-credits: {error}");
            return ExitCode::FAILURE;
        }
    };
    let invocation = match parsed {
        Parsed::Help => {
            print!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Parsed::Run(invocation) => invocation,
    };

    let outcome = app::analyse(&invocation.options);
    for warning in &outcome.report.warnings {
        eprintln!("mercury-credits: warning: {warning}");
    }
    if !outcome.ok() {
        for problem in &outcome.report.problems {
            eprintln!("mercury-credits: problem: {problem}");
        }
        eprintln!(
            "mercury-credits: {} problems; nothing was written",
            outcome.report.problems.len()
        );
        return ExitCode::FAILURE;
    }
    if let (Command::Generate, Some(out), Some(page)) =
        (invocation.command, &invocation.out, &outcome.page)
    {
        match app::write_page(out, page) {
            Ok(path) => eprintln!("mercury-credits: wrote {}", path.display()),
            Err(error) => {
                eprintln!("mercury-credits: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(summary) = outcome.summary() {
        eprintln!("{summary}");
    }
    ExitCode::SUCCESS
}
