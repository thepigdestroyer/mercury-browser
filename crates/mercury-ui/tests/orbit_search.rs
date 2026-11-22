// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Orbit's search predicate (brief D6): case, diacritics, several words, the host, the empty query,
//! the order of `top_result` and the speed with 64 groups and 200 tabs.

use std::time::Instant;

use mercury_ui::orbit_view::fixtures::{orbit_groups, orbit_input, orbit_tabs};
use mercury_ui::orbit_view::search::{matches, top_result};
use mercury_ui::orbit_view::{LOOSE_GROUP_ID, OrbitInput};
use mercury_ui::{GroupRow, TabRow};

#[test]
fn case_does_not_matter_in_either_direction() {
    assert!(matches("SERVO", "Servo layout notes", "docs.example.org"));
    assert!(matches("servo", "SERVO LAYOUT NOTES", "docs.example.org"));
    assert!(matches("DOCS", "x", "Docs.Example.ORG"));
}

#[test]
fn diacritics_are_left_alone_and_only_case_folds() {
    // Documented behaviour: no accent folding ("cafe" does not find "Café"), but non-ASCII
    // letters still fold case ("CAFÉ" finds "café").
    assert!(!matches("cafe", "Café du monde", "example.fr"));
    assert!(matches("café", "Café du monde", ""));
    assert!(matches("CAFÉ", "café du monde", ""));
    assert!(!matches("café", "Cafe du monde", ""));
}

#[test]
fn a_word_may_sit_anywhere_in_the_title() {
    assert!(matches("ayout", "Servo layout notes", ""));
    assert!(matches("notes", "Servo layout notes", ""));
}

#[test]
fn every_word_must_occur_in_any_order() {
    assert!(matches(
        "servo notes",
        "Servo layout notes",
        "docs.example.org"
    ));
    assert!(matches(
        "notes servo",
        "Servo layout notes",
        "docs.example.org"
    ));
    assert!(!matches(
        "servo zzz",
        "Servo layout notes",
        "docs.example.org"
    ));
    assert!(matches("  servo \t notes  ", "Servo layout notes", ""));
}

#[test]
fn the_words_may_be_split_between_the_title_and_the_host() {
    assert!(matches(
        "servo docs",
        "Servo layout notes",
        "docs.example.org"
    ));
    assert!(matches("a11y", "AccessKit guide", "a11y.example.net"));
    assert!(!matches("a11y", "AccessKit guide", "other.example.net"));
}

#[test]
fn an_empty_or_blank_query_matches_everything() {
    assert!(matches("", "anything", "host"));
    assert!(matches("   ", "anything", "host"));
    assert!(matches("", "", ""));
}

#[test]
fn a_word_never_matches_across_the_title_host_boundary() {
    // "notesdocs" would only exist if the two strings were glued together.
    assert!(!matches(
        "notesdocs",
        "Servo layout notes",
        "docs.example.org"
    ));
}

#[test]
fn top_result_is_the_first_match_in_group_then_strip_order() {
    let input = orbit_input(false);
    assert_eq!(top_result("servo", &input), Some(101));
    assert_eq!(top_result("interview", &input), Some(127));
    assert_eq!(top_result("example.pt", &input), Some(113));
}

#[test]
fn top_result_follows_the_group_order_of_the_input_not_the_ids() {
    let mut input = orbit_input(false);
    assert_eq!(top_result("example", &input), Some(101));
    input.groups.reverse();
    assert_eq!(
        top_result("example", &input),
        Some(135),
        "Work is first now"
    );
}

#[test]
fn top_result_follows_the_strip_order_inside_a_group_not_the_row_order() {
    let mut input = orbit_input(false);
    // "servo" matches 101 (title) and 102 (host); swap their places in the strip.
    for tab in &mut input.tabs {
        match tab.id {
            101 => tab.index_in_group = 1,
            102 => tab.index_in_group = 0,
            _ => {}
        }
    }
    assert_eq!(top_result("servo", &input), Some(102));
}

#[test]
fn top_result_looks_at_the_loose_planet_last() {
    let mut tabs = orbit_tabs();
    tabs.push(TabRow {
        id: 900,
        group: -1,
        title: "Loose servo page".into(),
        host: "loose.example.org".into(),
        ..tabs[0].clone()
    });
    let input = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
    assert_eq!(input.groups.last().map(|g| g.id), Some(LOOSE_GROUP_ID));
    assert_eq!(top_result("loose", &input), Some(900));
    assert_eq!(top_result("servo", &input), Some(101));
}

#[test]
fn top_result_is_none_for_an_empty_query_a_blank_one_and_no_match() {
    let input = orbit_input(false);
    assert_eq!(top_result("", &input), None);
    assert_eq!(top_result("   ", &input), None);
    assert_eq!(top_result("no such tab anywhere", &input), None);
    assert_eq!(top_result("servo", &OrbitInput::default()), None);
}

fn crowded(groups: usize, tabs: usize) -> OrbitInput {
    let group_rows: Vec<GroupRow> = (0..groups)
        .map(|g| GroupRow {
            id: g as i32 + 1,
            name: format!("Group {g}").into(),
            ..Default::default()
        })
        .collect();
    let mut seen = vec![0; groups];
    let tab_rows: Vec<TabRow> = (0..tabs)
        .map(|t| {
            let g = t % groups;
            seen[g] += 1;
            TabRow {
                id: 1000 + t as i32,
                group: g as i32 + 1,
                index_in_group: seen[g] - 1,
                title: format!("Tab number {t} of the crowded window").into(),
                host: format!("host{t}.example.org").into(),
                ..Default::default()
            }
        })
        .collect();
    OrbitInput::new(&group_rows, &tab_rows, "P", "W", 1000, false)
}

#[test]
fn sixty_four_groups_and_two_hundred_tabs_search_in_under_five_milliseconds() {
    let input = crowded(64, 200);
    assert_eq!(input.groups.len(), 64);
    assert_eq!(input.tabs.len(), 200);
    // The worst case: nothing matches, so every tab is looked at; and a late match.
    let started = Instant::now();
    let none = top_result("zzz qqq", &input);
    let none_time = started.elapsed();
    let started = Instant::now();
    let late = top_result("number 199", &input);
    let late_time = started.elapsed();
    println!(
        "orbit search, 64 groups / 200 tabs: no match {none_time:?}, late match {late_time:?}"
    );
    assert_eq!(none, None);
    assert_eq!(late, Some(1199));
    assert!(none_time.as_millis() < 5, "no match took {none_time:?}");
    assert!(late_time.as_millis() < 5, "late match took {late_time:?}");
}
