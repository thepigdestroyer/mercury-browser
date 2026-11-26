// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Summarises the content sandbox's denials from `log show` output.
//!
//!     log show --last 5m --predicate 'eventMessage contains "Sandbox:"' | sandbox-denials [--pid <pid>]...
//!     sandbox-denials <file> [--pid <pid>]...
//!
//! Reads the text it is given and never runs `log` itself, so it needs no admin rights and
//! changes no setting. Prints one line per decision, denials first, with a count. Exit status 0
//! when there are no denials, 1 when there are some, 2 for a wrong command line or unreadable
//! input.

use std::io::Read;
use std::process::ExitCode;

use mercury_devtools::sandbox_denials::{parse_line, summarise};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut pids = Vec::new();
    let mut input_path = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--pid" => {
                let Some(value) = arguments.get(index + 1) else {
                    return usage("--pid needs a process id");
                };
                let Ok(pid) = value.parse::<u32>() else {
                    return usage("--pid needs a numeric process id");
                };
                pids.push(pid);
                index += 2;
            }
            other if input_path.is_none() && !other.starts_with("--") => {
                input_path = Some(other.to_owned());
                index += 1;
            }
            _ => return usage("unknown argument"),
        }
    }

    let text = match read_input(input_path.as_deref()) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("sandbox-denials: {error}");
            return ExitCode::from(2);
        }
    };
    let events: Vec<_> = text.lines().filter_map(parse_line).collect();
    let summary = summarise(&events, &pids);
    for event in &summary.events {
        let decision = if event.allowed { "allow" } else { "deny" };
        println!(
            "{decision} {}({}) {} {}",
            event.process, event.pid, event.operation, event.target
        );
    }
    println!(
        "{} denial(s) of {} decision(s)",
        summary.denials,
        summary.events.len()
    );
    if summary.denials == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn read_input(path: Option<&str>) -> Result<String, String> {
    match path {
        Some(path) => std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}")),
        None => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|error| format!("standard input: {error}"))?;
            Ok(text)
        }
    }
}

fn usage(problem: &str) -> ExitCode {
    eprintln!("sandbox-denials: {problem}");
    eprintln!("usage: sandbox-denials [<file>] [--pid <pid>]...");
    ExitCode::from(2)
}
