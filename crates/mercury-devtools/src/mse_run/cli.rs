// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The command line of `mse-run`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::model::Phase;
use super::runner::Config;

pub const USAGE: &str =
    "usage: mse-run <pages|players|wpt|all> [--resume] [--timeout SECONDS] [--only TEXT] [--list]
               [--browser PATH] [--browser-arg ARG]... [--tools-dir DIR] [--expect FILE]
               [--out DIR] [--journal FILE] [--report FILE] [--ioreg PATH | --no-lock-check]";

pub fn help_text() -> String {
    format!(
        "{USAGE}

Runs the MSE window checks of Mercury Browser (docs/developer-guide/mse-testing.md): one
browser process per test, a watchdog for each, a journal line after each, and a report.

phases:
  pages      tests/pages/mse-*.html
  players    tests/pages/mse-players/*.html (dash.js, Shaka Player, hls.js)
  wpt        the web-platform-tests media-source/ tree, compared with tests/wpt/media-source.expect.toml
  all        the three, in that order

options:
  --resume            continue the journal: finished tests are not run again; a test that was
                      interrupted by a screen lock or left a stuck browser runs again
  --timeout SECONDS   the watchdog of one test (default 60; a WPT file with the long timeout gets twice)
  --only TEXT         run only the tests whose name contains TEXT
  --list              print the tests and exit
  --browser PATH      the browser (default target/release/mercury-browser); a stub script works too
  --browser-arg ARG   an extra argument for the browser, before the URL (repeatable)
  --tools-dir DIR     where scripts/dev/fetch-mse-tools.sh put the tools (default $MSE_TOOLS_DIR or the cache directory)
  --expect FILE       the expectations (default tests/wpt/media-source.expect.toml)
  --out DIR           browser logs, journal and report (default $MSE_RUN_OUT or a temporary directory)
  --journal FILE      the journal (default <out>/journal.jsonl)
  --report FILE       the report (default <out>/window-run-<UTC>.md)
  --ioreg PATH        the program that answers the lock check (default ioreg)
  --no-lock-check     do not check for a locked screen (stub browsers only)

The run refuses to start on a locked screen. Exit status: 0 nothing unexpected and nothing left
out; 1 unexpected results; 2 usage or preflight error; 3 incomplete (screen lock, stuck browser,
tests not run).
"
    )
}

/// What the command line asks for.
#[derive(Debug)]
pub enum Command {
    Help,
    List(Config),
    Run {
        config: Config,
        report: PathBuf,
    },
    /// `mse-run verdict`: judge the journal of a finished run (`verdict.rs`).
    Verdict(super::verdict::VerdictConfig),
}

#[derive(Debug, PartialEq, Eq)]
pub struct UsageError(pub String);

/// Where the repository is and what the environment says; the defaults come from here.
pub struct Environment {
    pub repo: PathBuf,
    pub tools_dir: PathBuf,
    pub out_dir: PathBuf,
    pub now_stamp: String,
}

pub(super) fn value(
    arguments: &[String],
    index: &mut usize,
    option: &str,
    inline: Option<&str>,
) -> Result<String, UsageError> {
    if let Some(inline) = inline {
        return Ok(inline.to_owned());
    }
    *index += 1;
    arguments
        .get(*index)
        .cloned()
        .ok_or_else(|| UsageError(format!("{option} needs a value")))
}

fn seconds(text: &str, option: &str) -> Result<Duration, UsageError> {
    text.parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(Duration::from_secs_f64)
        .ok_or_else(|| {
            UsageError(format!(
                "{option} wants a positive number of seconds, not {text:?}"
            ))
        })
}

pub fn parse_arguments(
    arguments: &[String],
    environment: &Environment,
) -> Result<Command, UsageError> {
    if arguments.first().map(String::as_str) == Some("verdict") {
        if arguments[1..].iter().any(|a| a == "-h" || a == "--help") {
            return Ok(Command::Help);
        }
        return super::verdict::parse_arguments(&arguments[1..], environment).map(Command::Verdict);
    }
    let repo = &environment.repo;
    let mut phases: Option<Vec<Phase>> = None;
    let mut browser = repo.join("target/release/mercury-browser");
    let mut browser_args = Vec::new();
    let mut timeout = Duration::from_secs(60);
    let mut grace = Duration::from_secs(5);
    let mut resume = false;
    let mut only = None;
    let mut list = false;
    let mut tools_dir = environment.tools_dir.clone();
    let mut expect = repo.join("tests/wpt/media-source.expect.toml");
    let mut out_dir = environment.out_dir.clone();
    let mut journal: Option<PathBuf> = None;
    let mut report: Option<PathBuf> = None;
    let mut ioreg: Option<PathBuf> = Some(PathBuf::from("ioreg"));

    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        let (name, inline) = match argument.split_once('=') {
            Some((name, inline)) if name.starts_with("--") => (name, Some(inline)),
            _ => (argument, None),
        };
        match name {
            "-h" | "--help" => return Ok(Command::Help),
            "pages" | "players" | "wpt" | "all" if inline.is_none() => {
                if phases.is_some() {
                    return Err(UsageError("name the phase once".to_owned()));
                }
                phases = Some(if name == "all" {
                    Phase::ALL.to_vec()
                } else {
                    vec![Phase::parse(name).unwrap_or(Phase::Pages)]
                });
            }
            "--resume" if inline.is_none() => resume = true,
            "--list" if inline.is_none() => list = true,
            "--no-lock-check" if inline.is_none() => ioreg = None,
            "--timeout" => timeout = seconds(&value(arguments, &mut index, name, inline)?, name)?,
            "--grace" => grace = seconds(&value(arguments, &mut index, name, inline)?, name)?,
            "--only" => only = Some(value(arguments, &mut index, name, inline)?),
            "--browser" => browser = PathBuf::from(value(arguments, &mut index, name, inline)?),
            "--browser-arg" => browser_args.push(value(arguments, &mut index, name, inline)?),
            "--tools-dir" => tools_dir = PathBuf::from(value(arguments, &mut index, name, inline)?),
            "--expect" => expect = PathBuf::from(value(arguments, &mut index, name, inline)?),
            "--out" => out_dir = PathBuf::from(value(arguments, &mut index, name, inline)?),
            "--journal" => {
                journal = Some(PathBuf::from(value(arguments, &mut index, name, inline)?))
            }
            "--report" => report = Some(PathBuf::from(value(arguments, &mut index, name, inline)?)),
            "--ioreg" => ioreg = Some(PathBuf::from(value(arguments, &mut index, name, inline)?)),
            other => return Err(UsageError(format!("unknown argument {other:?}"))),
        }
        index += 1;
    }
    let Some(phases) = phases else {
        return Err(UsageError(
            "name a phase: pages, players, wpt or all".to_owned(),
        ));
    };
    let journal = journal.unwrap_or_else(|| out_dir.join("journal.jsonl"));
    let report =
        report.unwrap_or_else(|| out_dir.join(format!("window-run-{}.md", environment.now_stamp)));
    let config = Config {
        tests_root: repo.join("tests"),
        tools_dir,
        browser,
        browser_args,
        phases,
        only,
        timeout,
        grace,
        kill_wait: Duration::from_secs(5),
        resume,
        out_dir,
        journal,
        expect,
        ioreg,
    };
    Ok(if list {
        Command::List(config)
    } else {
        Command::Run { config, report }
    })
}

/// The repository this crate was built from.
pub fn default_repo() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::canonicalize(&manifest).unwrap_or(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment() -> Environment {
        Environment {
            repo: PathBuf::from("/repo"),
            tools_dir: PathBuf::from("/cache/mse"),
            out_dir: PathBuf::from("/out"),
            now_stamp: "20261122T100000Z".into(),
        }
    }

    fn parse(list: &[&str]) -> Result<Command, UsageError> {
        let arguments: Vec<String> = list.iter().map(|a| (*a).to_owned()).collect();
        parse_arguments(&arguments, &environment())
    }

    fn config(list: &[&str]) -> (Config, Option<PathBuf>) {
        match parse(list).expect("parses") {
            Command::Run { config, report } => (config, Some(report)),
            Command::List(config) => (config, None),
            Command::Help | Command::Verdict(_) => panic!("not a run"),
        }
    }

    #[test]
    fn the_defaults_come_from_the_repository_and_the_environment() {
        let (config, report) = config(&["all"]);
        assert_eq!(config.phases, Phase::ALL);
        assert_eq!(
            config.browser,
            Path::new("/repo/target/release/mercury-browser")
        );
        assert_eq!(config.tests_root, Path::new("/repo/tests"));
        assert_eq!(
            config.expect,
            Path::new("/repo/tests/wpt/media-source.expect.toml")
        );
        assert_eq!(config.tools_dir, Path::new("/cache/mse"));
        assert_eq!(config.timeout, Duration::from_secs(60));
        assert_eq!(config.journal, Path::new("/out/journal.jsonl"));
        assert_eq!(
            report.unwrap(),
            Path::new("/out/window-run-20261122T100000Z.md")
        );
        assert_eq!(config.ioreg, Some(PathBuf::from("ioreg")));
        assert!(!config.resume);
    }

    #[test]
    fn options_are_read_with_both_value_forms() {
        let (config, _) = config(&[
            "wpt",
            "--resume",
            "--timeout",
            "90",
            "--only=duration",
            "--browser=/b/stub.sh",
            "--browser-arg",
            "--verbose",
            "--browser-arg=--x",
            "--no-lock-check",
            "--journal",
            "/j.jsonl",
        ]);
        assert_eq!(config.phases, [Phase::Wpt]);
        assert!(config.resume);
        assert_eq!(config.timeout, Duration::from_secs(90));
        assert_eq!(config.only.as_deref(), Some("duration"));
        assert_eq!(config.browser, Path::new("/b/stub.sh"));
        assert_eq!(config.browser_args, ["--verbose", "--x"]);
        assert_eq!(config.ioreg, None);
        assert_eq!(config.journal, Path::new("/j.jsonl"));
    }

    #[test]
    fn list_and_help_and_errors() {
        assert!(matches!(parse(&["pages", "--list"]), Ok(Command::List(_))));
        assert!(matches!(parse(&["-h"]), Ok(Command::Help)));
        for (list, needle) in [
            (&[][..], "name a phase"),
            (&["pages", "wpt"][..], "once"),
            (&["pages", "--timeout"][..], "needs a value"),
            (&["pages", "--timeout", "x"][..], "positive number"),
            (&["pages", "--timeout", "0"][..], "positive number"),
            (&["pages", "--nope"][..], "unknown argument"),
            (&["everything"][..], "unknown argument"),
        ] {
            let error = parse(list).expect_err(needle);
            assert!(error.0.contains(needle), "{needle}: {}", error.0);
        }
    }

    #[test]
    fn the_help_names_every_option() {
        let help = help_text();
        for option in [
            "--resume",
            "--timeout",
            "--only",
            "--list",
            "--browser ",
            "--browser-arg",
            "--tools-dir",
            "--expect",
            "--out",
            "--journal",
            "--report",
            "--ioreg",
            "--no-lock-check",
        ] {
            assert!(help.contains(option), "{option}");
        }
    }
}
