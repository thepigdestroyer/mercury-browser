// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The `sandbox-denials` parser, on recorded `log show` lines only. No test reads the live
//! system log: that needs no admin rights here, but its contents are not fixed.

use mercury_devtools::sandbox_denials::{SandboxEvent, parse_line, summarise};

const DENY: &str = "2026-11-26 10:00:01.123456+0100  localhost kernel[0:1] \
    Sandbox: cat(38123) deny(1) file-read-data /private/etc/hosts";
const ALLOW: &str = "2026-11-26 10:00:01.200000+0100  localhost kernel[0:1] \
    Sandbox: echo(38118) allow sysctl-read kern.osvariant_status";
const DUPLICATE: &str = "2026-11-26 10:00:02.000000+0100  localhost kernel[0:1] \
    1 duplicate report for Sandbox: cat(38123) deny(1) file-read-data /private/etc/hosts";
const SPACES: &str = "2026-11-26 10:00:03.000000+0100  localhost kernel[0:1] \
    Sandbox: Mercury(4242) allow file-read-data /Users/a/Library/Application Support/x";
const OTHER: &str = "2026-11-26 10:00:04.000000+0100  localhost kernel[0:1] \
    launchd[1] (com.example.agent) exited";

#[test]
fn a_denial_line_is_a_denied_event() {
    let event = parse_line(DENY).expect("the deny line parses");
    assert_eq!(
        event,
        SandboxEvent {
            process: "cat".to_owned(),
            pid: 38123,
            allowed: false,
            operation: "file-read-data".to_owned(),
            target: "/private/etc/hosts".to_owned(),
        }
    );
}

#[test]
fn an_allow_line_is_an_allowed_event() {
    let event = parse_line(ALLOW).expect("the allow line parses");
    assert_eq!(
        event,
        SandboxEvent {
            process: "echo".to_owned(),
            pid: 38118,
            allowed: true,
            operation: "sysctl-read".to_owned(),
            target: "kern.osvariant_status".to_owned(),
        }
    );
}

#[test]
fn a_duplicate_report_line_is_parsed_as_its_report() {
    let event = parse_line(DUPLICATE).expect("the duplicate-report line parses");
    assert_eq!(event.process, "cat");
    assert_eq!(event.pid, 38123);
    assert!(!event.allowed);
    assert_eq!(event.target, "/private/etc/hosts");
}

#[test]
fn a_target_with_spaces_is_kept_whole() {
    let event = parse_line(SPACES).expect("the spaced line parses");
    assert_eq!(event.target, "/Users/a/Library/Application Support/x");
}

#[test]
fn a_line_from_another_sender_is_none() {
    assert_eq!(parse_line(OTHER), None);
}

#[test]
fn garbage_is_none() {
    assert_eq!(parse_line(""), None);
    assert_eq!(parse_line("Sandbox:"), None);
    assert_eq!(parse_line("Sandbox: cat(x) deny(1) file-read-data /"), None);
    assert_eq!(parse_line("Sandbox: cat(1) maybe file-read-data /"), None);
}

#[test]
fn summary_filters_by_pid_and_lists_denials_first() {
    let events: Vec<SandboxEvent> = [ALLOW, DENY, SPACES]
        .iter()
        .filter_map(|line| parse_line(line))
        .collect();

    let all = summarise(&events, &[]);
    assert_eq!(all.events.len(), 3);
    assert!(!all.events[0].allowed, "denials come first");
    assert_eq!(all.denials, 1);

    let only_cat = summarise(&events, &[38123]);
    assert_eq!(only_cat.events.len(), 1);
    assert_eq!(only_cat.events[0].pid, 38123);
    assert_eq!(only_cat.denials, 1);
}
