// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The command line of `serve-tests` (`[-h] [--port PORT] [--log LOG]`), read and answered as
//! the Python script's `argparse` parser did.

use super::python::python_int;

/// The description `--help` prints, the first line of the old script's documentation.
pub const DESCRIPTION: &str =
    "Serve the repository's tests/ directory on the loopback interface, for the media checks.";
const PROGRAM: &str = "serve-tests";
const USAGE: &str = "usage: serve-tests [-h] [--port PORT] [--log LOG]";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Print the help text on standard output and exit 0.
    Help,
    /// Serve on this port (0: any free port), and append a JSON line per request to the file
    /// `log` if given. The port is not checked against 0..=65535 here: like the Python script,
    /// an out-of-range port fails when binding, with exit status 1.
    Serve { port: i64, log: Option<String> },
}

/// A command-line error: the message for standard error and the exit status.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageError {
    pub message: String,
    pub status: i32,
}

/// The `--help` text.
pub fn help_text() -> String {
    format!(
        "{USAGE}\n\n{DESCRIPTION}\n\noptional arguments:\n  -h, --help   show this help message and exit\n  --port PORT  port to bind (default: any free port)\n  --log LOG    append one JSON line per request to this file\n"
    )
}

fn usage_error(detail: &str) -> UsageError {
    UsageError {
        message: format!("{USAGE}\n{PROGRAM}: error: {detail}\n"),
        status: 2,
    }
}

/// Whether `argument` names an option by itself or by an unambiguous prefix (argparse
/// accepts `--po 8000` for `--port 8000`).
fn long_option(argument: &str) -> Option<&'static str> {
    if argument.len() < 3 || !argument.starts_with("--") {
        return None;
    }
    ["--help", "--port", "--log"]
        .into_iter()
        .find(|option| option.starts_with(argument))
}

fn looks_like_negative_number(argument: &str) -> bool {
    argument
        .strip_prefix('-')
        .is_some_and(|rest| !rest.is_empty() && rest.parse::<f64>().is_ok())
}

/// Reads the command line (without the program name) as the Python script's `argparse`
/// parser did: `[-h] [--port PORT] [--log LOG]`, with `--port=N` and unambiguous prefixes.
pub fn parse_arguments(arguments: &[String]) -> Result<Command, UsageError> {
    let mut port = 0;
    let mut log = None;
    let mut unrecognised = Vec::new();
    let mut only_positional = false;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        index += 1;
        if only_positional
            || !argument.starts_with('-')
            || argument == "-"
            || looks_like_negative_number(argument)
        {
            unrecognised.push(argument.to_owned());
            continue;
        }
        if argument == "--" {
            only_positional = true;
            continue;
        }
        let (name, inline_value) = match argument.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (argument, None),
        };
        let option = if name == "-h" {
            Some("--help")
        } else {
            long_option(name)
        };
        match option {
            Some("--help") => {
                if inline_value.is_some() {
                    return Err(usage_error(&format!(
                        "argument -h/--help: ignored explicit argument '{}'",
                        inline_value.unwrap_or_default()
                    )));
                }
                return Ok(Command::Help);
            }
            Some(option) => {
                let value = match inline_value {
                    Some(value) => value,
                    None => match arguments.get(index) {
                        Some(next)
                            if !next.starts_with('-')
                                || next == "-"
                                || looks_like_negative_number(next) =>
                        {
                            index += 1;
                            next.as_str()
                        }
                        _ => {
                            return Err(usage_error(&format!(
                                "argument {option}: expected one argument"
                            )));
                        }
                    },
                };
                if option == "--log" {
                    log = Some(value.to_owned());
                } else {
                    port = python_int(value).ok_or_else(|| {
                        usage_error(&format!("argument --port: invalid int value: '{value}'"))
                    })?;
                }
            }
            None => unrecognised.push(argument.to_owned()),
        }
    }
    if !unrecognised.is_empty() {
        return Err(usage_error(&format!(
            "unrecognized arguments: {}",
            unrecognised.join(" ")
        )));
    }
    Ok(Command::Serve { port, log })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(list: &[&str]) -> Vec<String> {
        list.iter().map(|argument| (*argument).to_owned()).collect()
    }

    #[test]
    fn command_line_like_argparse() {
        let serve = |port| Ok(Command::Serve { port, log: None });
        assert_eq!(parse_arguments(&[]), serve(0));
        assert_eq!(
            parse_arguments(&arguments(&["--port", "8765"])),
            serve(8765)
        );
        assert_eq!(parse_arguments(&arguments(&["--port=8765"])), serve(8765));
        assert_eq!(
            parse_arguments(&arguments(&["--po", "1", "--port", "2"])),
            serve(2)
        );
        assert_eq!(parse_arguments(&arguments(&["--port", "-1"])), serve(-1));
        let logged = |port, file: &str| {
            Ok(Command::Serve {
                port,
                log: Some(file.to_owned()),
            })
        };
        assert_eq!(
            parse_arguments(&arguments(&["--log", "requests.jsonl"])),
            logged(0, "requests.jsonl")
        );
        assert_eq!(
            parse_arguments(&arguments(&["--port=1", "--lo=a b", "--log", "c"])),
            logged(1, "c")
        );
        assert_eq!(parse_arguments(&arguments(&["-h"])), Ok(Command::Help));
        assert_eq!(
            parse_arguments(&arguments(&["x", "--help"])),
            Ok(Command::Help)
        );
        for (list, detail) in [
            (&["--port"][..], "argument --port: expected one argument"),
            (&["--log"][..], "argument --log: expected one argument"),
            (
                &["--log", "--port", "1"][..],
                "argument --log: expected one argument",
            ),
            (
                &["--port", "--help"][..],
                "argument --port: expected one argument",
            ),
            (
                &["--port", "x"][..],
                "argument --port: invalid int value: 'x'",
            ),
            (
                &["--port", "80", "extra"][..],
                "unrecognized arguments: extra",
            ),
            (&["-p", "80"][..], "unrecognized arguments: -p 80"),
            (&["--", "--port"][..], "unrecognized arguments: --port"),
            (&["--verbose"][..], "unrecognized arguments: --verbose"),
        ] {
            let error = parse_arguments(&arguments(list)).expect_err(detail);
            assert_eq!(error.status, 2);
            assert_eq!(
                error.message,
                format!("{USAGE}\nserve-tests: error: {detail}\n")
            );
        }
    }

    #[test]
    fn help_names_the_option() {
        let help = help_text();
        assert!(help.starts_with("usage: serve-tests [-h] [--port PORT] [--log LOG]\n"));
        assert!(help.contains("--port PORT  port to bind (default: any free port)"));
        assert!(help.contains("--log LOG    append one JSON line per request to this file"));
    }
}
