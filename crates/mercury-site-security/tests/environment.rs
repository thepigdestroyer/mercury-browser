// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use mercury_site_security::{ErrorPageEnvironment, automatic_continue_requested, is_automated_run};

const SECRET: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn automatic_continue_needs_an_automated_run_and_the_test_variable() {
    assert!(automatic_continue_requested(true, Some("1")));
    // A normal run ignores the variable, whoever set it.
    assert!(!automatic_continue_requested(false, Some("1")));
    assert!(!automatic_continue_requested(true, None));
    assert!(!automatic_continue_requested(true, Some("yes")));
}

#[test]
fn automated_runs_are_screenshot_and_background_runs() {
    let arguments = |list: &[&str]| {
        list.iter()
            .map(|argument| argument.to_string())
            .collect::<Vec<_>>()
    };
    assert!(is_automated_run(arguments(&[
        "--screenshot",
        "a.png",
        "https://a/"
    ])));
    assert!(is_automated_run(arguments(&["--background"])));
    assert!(!is_automated_run(arguments(&["https://a/"])));
}

#[test]
fn the_environment_value_carries_the_secret_and_the_switch() {
    let plain = ErrorPageEnvironment {
        secret: SECRET.to_owned(),
        continue_automatically: false,
    };
    assert_eq!(plain.encode(), SECRET);
    assert_eq!(ErrorPageEnvironment::decode(&plain.encode()), Some(plain));

    let automatic = ErrorPageEnvironment {
        secret: SECRET.to_owned(),
        continue_automatically: true,
    };
    assert_eq!(
        ErrorPageEnvironment::decode(&automatic.encode()),
        Some(automatic)
    );
    for forged in [
        "",
        "short",
        "0123456789abcdef0123456789abcdeg",
        &format!("{SECRET}:other"),
    ] {
        assert_eq!(ErrorPageEnvironment::decode(forged), None, "{forged}");
    }
}
