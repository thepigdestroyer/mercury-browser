// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Reads the sandbox's denials from `log show` output: the lines the kernel writes for a
//! Seatbelt decision. Parsing needs no admin rights and changes no system setting; the input is
//! text the caller gives, normally from `log show --predicate 'eventMessage contains "Sandbox:"'`.

/// One sandbox decision, as the kernel logged it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxEvent {
    /// The name of the process that asked, as the sandbox logs it (`cat`, `Mercury`).
    pub process: String,
    /// The process id, which is how a run's denials are told apart from another's.
    pub pid: u32,
    /// `true` for an allow line, `false` for a deny.
    pub allowed: bool,
    /// The operation, such as `file-read-data` or `mach-lookup`.
    pub operation: String,
    /// What the operation was on: a path, a service name, or a sysctl key. It may contain spaces.
    pub target: String,
}

/// The events of one run, denials first, with the number of denials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub events: Vec<SandboxEvent>,
    pub denials: usize,
}

const MARKER: &str = "Sandbox: ";
const DUPLICATE_PREFIX: &str = "duplicate report for ";

/// Parses one `log show` line into a sandbox decision, or `None` when the line is not one.
///
/// A line the kernel folded into a duplicate report is parsed as the report it repeats. The
/// process must be `name(pid)`, the decision `allow` or `deny(n)`, and the operation a single
/// word; the rest of the line, with spaces, is the target.
pub fn parse_line(line: &str) -> Option<SandboxEvent> {
    let start = line.find(MARKER)? + MARKER.len();
    let mut body = &line[start..];
    if let Some(rest) = body.strip_prefix(DUPLICATE_PREFIX) {
        body = rest;
    }
    let (process_and_pid, rest) = body.split_once(' ')?;
    let (process, pid_text) = process_and_pid.strip_suffix(')')?.split_once('(')?;
    let pid: u32 = pid_text.parse().ok()?;
    let (decision, rest) = rest.split_once(' ')?;
    let allowed = match decision {
        "allow" => true,
        d if d == "deny" || d.starts_with("deny(") => {
            if d != "deny" {
                let count = d.strip_prefix("deny(")?.strip_suffix(')')?;
                count.parse::<u32>().ok()?;
            }
            false
        }
        _ => return None,
    };
    let (operation, target) = rest.split_once(' ')?;
    if process.is_empty() || operation.is_empty() || target.is_empty() {
        return None;
    }
    Some(SandboxEvent {
        process: process.to_owned(),
        pid,
        allowed,
        operation: operation.to_owned(),
        target: target.to_owned(),
    })
}

/// The events of the given pids (every pid when `pids` is empty), denials first, in their
/// original order within each group.
pub fn summarise(events: &[SandboxEvent], pids: &[u32]) -> Summary {
    let mut chosen: Vec<SandboxEvent> = events
        .iter()
        .filter(|event| pids.is_empty() || pids.contains(&event.pid))
        .cloned()
        .collect();
    chosen.sort_by_key(|event| event.allowed);
    let denials = chosen.iter().filter(|event| !event.allowed).count();
    Summary {
        events: chosen,
        denials,
    }
}
