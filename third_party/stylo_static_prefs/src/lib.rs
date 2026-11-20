/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::atomic::{AtomicBool, AtomicI32};

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

#[test]
fn test_basic_preferences() {
    assert!(!pref!("layout.css.writing-mode.enabled"));
    set_pref!("layout.css.writing-mode.enabled", true);
    assert!(pref!("layout.css.writing-mode.enabled"));

    assert_eq!(pref!("layout.threads"), -1);
    set_pref!("layout.threads", 42);
    assert_eq!(pref!("layout.threads"), 42);
}

// Mercury: tests of `set_bool` and `get_bool` (K0, ED2). The preferences are process-wide and the
// tests run in parallel, so each test has preferences of its own, and none touches the two that
// `test_basic_preferences` changes.

#[test]
fn mercury_set_bool_changes_what_pref_reads() {
    assert_eq!(get_bool("layout.css.at-scope.enabled"), Some(false));
    assert!(!pref!("layout.css.at-scope.enabled"));

    assert!(set_bool("layout.css.at-scope.enabled", true));
    assert_eq!(get_bool("layout.css.at-scope.enabled"), Some(true));
    assert!(pref!("layout.css.at-scope.enabled"));
    // The preferences next to it keep their values.
    assert!(!pref!("layout.css.anchor-positioning.enabled"));

    assert!(set_bool("layout.css.at-scope.enabled", false));
    assert_eq!(get_bool("layout.css.at-scope.enabled"), Some(false));
    assert!(!pref!("layout.css.at-scope.enabled"));
}

#[test]
fn mercury_get_bool_reads_the_defaults() {
    assert_eq!(
        get_bool("layout.css.alpha-color-function.enabled"),
        Some(true)
    );
    assert_eq!(get_bool("layout.css.has-selector.enabled"), Some(false));
}

#[test]
fn mercury_set_bool_refuses_unknown_names_and_integer_preferences() {
    assert!(!set_bool("layout.css.no-such-preference.enabled", true));
    assert_eq!(get_bool("layout.css.no-such-preference.enabled"), None);
    assert!(!set_bool("", true));
    assert_eq!(get_bool(""), None);
    // `layout.threads` is an integer preference.
    assert!(!set_bool("layout.threads", true));
    assert_eq!(get_bool("layout.threads"), None);
    // Names match exactly.
    assert_eq!(get_bool("Layout.css.at-scope.enabled"), None);
    assert_eq!(get_bool("layout.css.at-scope.enabled "), None);
}

#[test]
fn mercury_every_boolean_in_preferences_toml_has_an_accessor() {
    let mut booleans = 0;
    for line in include_str!("../preferences.toml").lines() {
        let Some((name, value)) = line.split_once(" = ") else {
            continue;
        };
        let name = name.trim().trim_matches('"');
        match value.trim() {
            "true" | "false" => {
                assert!(get_bool(name).is_some(), "{name}");
                booleans += 1;
            },
            _ => assert!(get_bool(name).is_none(), "{name} is not a boolean"),
        }
    }
    assert!(booleans > 50, "read only {booleans} boolean preferences");
}
