// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The shape of the content profile text: it balances, it denies by default, it names no
//! forbidden resource, and the parameters it reads are exactly the ones the builder produces.

use std::collections::BTreeSet;
use std::path::PathBuf;

use mercury_sandbox::{CONTENT_PROFILE, ProfileInputs, content_profile};

/// The profile without its comments (a `;` outside a string starts one). String contents stay.
fn code(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut in_string = false;
        let mut escaped = false;
        for ch in line.chars() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
            } else if ch == '"' {
                in_string = true;
            } else if ch == ';' {
                break;
            }
            out.push(ch);
        }
        out.push('\n');
    }
    out
}

/// The code with the contents of string literals removed: `"A"` becomes `""`.
fn without_strings(code: &str) -> String {
    let mut out = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in code.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
                out.push(ch);
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
        }
        out.push(ch);
    }
    out
}

fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The top-level forms of balanced code, each normalised to single spaces.
fn top_level_forms(bare: &str) -> Vec<String> {
    let mut forms = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for ch in bare.chars() {
        if depth == 0 && ch != '(' {
            continue;
        }
        current.push(ch);
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    forms.push(normalise(&current));
                    current.clear();
                }
            }
            _ => {}
        }
    }
    forms
}

/// Every name read as `(param "NAME")`, `(param-true? "NAME")` or `(param-defined? "NAME")`.
fn parameter_names(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (index, _) in source.match_indices("(param") {
        let mut rest = &source[index + "(param".len()..];
        for suffix in ["-true?", "-defined?"] {
            if let Some(after) = rest.strip_prefix(suffix) {
                rest = after;
                break;
            }
        }
        let Some(quoted) = rest.trim_start().strip_prefix('"') else {
            continue;
        };
        if let Some(end) = quoted.find('"') {
            names.insert(quoted[..end].to_owned());
        }
    }
    names
}

fn inputs() -> ProfileInputs {
    ProfileInputs::new(
        PathBuf::from("/Applications/Mercury.app"),
        PathBuf::from("/Applications/Mercury.app/Contents/MacOS/mercury-browser"),
        PathBuf::from("/Users/alice"),
    )
}

#[test]
fn balanced_and_ordered() {
    let source = code(CONTENT_PROFILE);
    let bare = without_strings(&source);
    let mut depth = 0i32;
    for ch in bare.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                assert!(depth >= 0, "a ')' closes nothing");
            }
            _ => {}
        }
    }
    assert_eq!(depth, 0, "unbalanced parentheses");
    assert_eq!(
        source.matches('"').count() % 2,
        0,
        "unbalanced string literals"
    );

    let forms = top_level_forms(&bare);
    assert_eq!(forms.first().map(String::as_str), Some("(version 1)"));
    let first_rule = forms
        .iter()
        .skip(1)
        .find(|form| !form.starts_with("(define "))
        .map(String::as_str);
    assert_eq!(first_rule, Some("(deny default)"));
}

#[test]
fn no_allow_default() {
    assert!(!code(CONTENT_PROFILE).contains("(allow default"));
}

#[test]
fn declared_parameters_match_used() {
    let mut inputs = inputs();
    inputs.run_root = Some(PathBuf::from("/private/var/folders/x/T/run"));
    let profile = content_profile(&inputs).unwrap();
    let declared: BTreeSet<String> = profile
        .parameters
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    assert_eq!(parameter_names(&code(CONTENT_PROFILE)), declared);
}

#[test]
fn forbidden_names_are_absent() {
    let source = code(CONTENT_PROFILE);
    for name in [
        "hsts",
        "filter",
        "Application Support",
        "Keychains",
        ".ssh",
        "/Users",
        "network-bind",
        "network-inbound",
        "process-exec",
        "process-fork",
        "(allow default)",
    ] {
        assert!(
            !source.contains(name),
            "forbidden name in the profile: {name}"
        );
    }

    assert_eq!(
        source.matches("network").count(),
        1,
        "one network rule only"
    );
    assert!(
        normalise(&source).contains(
            r#"(allow network-outbound (literal "/private/var/run/asl_input") (literal "/private/var/run/syslog") )"#
        ),
        "the only network rule is the two syslog sockets"
    );

    assert_eq!(source.matches("file-write*").count(), 1);
    for line in source.lines().filter(|line| line.contains("file-write*")) {
        assert!(
            line.contains("RUN_ROOT"),
            "file-write* outside RUN_ROOT: {line}"
        );
    }
}

#[test]
fn unverified_names_are_counted() {
    assert_eq!(CONTENT_PROFILE.matches("; UNVERIFIED").count(), 9);
}

#[test]
fn ascii_only_without_tabs() {
    assert!(CONTENT_PROFILE.is_ascii());
    assert!(!CONTENT_PROFILE.contains('\t'));
}
