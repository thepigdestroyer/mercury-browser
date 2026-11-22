// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Run the Media Source Extensions window checks: the pages, the player pages and the WPT
//! `media-source/` tree, one browser per test, with a journal and a report.
//!
//!     scripts/dev/mse-window-run.sh pages|players|wpt|all [--resume] [--timeout SECONDS] ...
//!
//! See `mse-run --help` and `docs/developer-guide/mse-testing.md`. Process groups, signals and
//! `ps` are Unix APIs, so on other systems this prints a message and exits with status 1.

use std::process::ExitCode;

#[cfg(unix)]
fn main() -> ExitCode {
    unix::main()
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    eprintln!("mse-run: needs a Unix system (process groups, signals and ps)");
    ExitCode::FAILURE
}

#[cfg(unix)]
mod unix {
    use std::process::ExitCode;

    use mercury_devtools::mse_run::cli::{self, Command, Environment};
    use mercury_devtools::mse_run::{runner, summary};
    use mercury_devtools::serve::mse_tools_dir;

    pub fn main() -> ExitCode {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let environment = Environment {
            repo: cli::default_repo(),
            tools_dir: mse_tools_dir(),
            out_dir: std::env::var_os("MSE_RUN_OUT")
                .filter(|dir| !dir.is_empty())
                .map(Into::into)
                .unwrap_or_else(|| std::env::temp_dir().join("mercury-mse-run")),
            now_stamp: summary::utc_now().replace([':', '-'], ""),
        };
        match cli::parse_arguments(&arguments, &environment) {
            Err(error) => {
                eprintln!("mse-run: {}\n{}", error.0, cli::USAGE);
                ExitCode::from(2)
            }
            Ok(Command::Help) => {
                print!("{}", cli::help_text());
                ExitCode::SUCCESS
            }
            Ok(Command::List(config)) => match runner::list(&config) {
                Ok(tests) => {
                    for test in &tests {
                        println!("{} {}", test.phase.name(), test.id);
                    }
                    eprintln!("{} tests", tests.len());
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("mse-run: {error}");
                    ExitCode::from(2)
                }
            },
            Ok(Command::Run { config, report }) => {
                let _ = std::fs::create_dir_all(&config.out_dir);
                let mut log = |line: &str| eprintln!("mse-run: {line}");
                match runner::run(&config, &mut log) {
                    Err(runner::RunError::Locked(state)) => {
                        eprintln!("mse-run: {}", runner::RunError::Locked(state));
                        ExitCode::from(3)
                    }
                    Err(error) => {
                        eprintln!("mse-run: {error}");
                        ExitCode::from(2)
                    }
                    Ok(data) => {
                        let text = summary::render(&data);
                        if let Some(parent) = report.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        match std::fs::write(&report, &text) {
                            Ok(()) => eprintln!("mse-run: report written to {}", report.display()),
                            Err(error) => {
                                eprintln!(
                                    "mse-run: cannot write {}: {error}; the report follows",
                                    report.display()
                                );
                                print!("{text}");
                            }
                        }
                        let code = summary::exit_code(&data);
                        eprintln!("mse-run: exit status {code}");
                        ExitCode::from(code as u8)
                    }
                }
            }
        }
    }
}
