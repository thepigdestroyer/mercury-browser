// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use talaria_core::{OpenTab, TabId, TabMatch, switch_to_tab_matches};

fn tab(id: u64, title: &str, url: &str, is_active: bool) -> OpenTab {
    OpenTab {
        id: TabId::new(id),
        title: title.to_owned(),
        url: url.to_owned(),
        is_active,
    }
}

fn strip() -> Vec<OpenTab> {
    vec![
        tab(1, "Rust Book", "https://doc.rust-lang.org/book/", true),
        tab(
            2,
            "Rust by Example",
            "https://doc.rust-lang.org/rust-by-example/",
            false,
        ),
        tab(3, "Cargo Guide", "https://doc.rust-lang.org/cargo/", false),
        tab(4, "Mail", "https://mail.example.com/inbox", false),
        tab(5, "New Tab", "about:blank", false),
    ]
}

fn matches(typed: &str, limit: usize) -> Vec<(u64, bool)> {
    switch_to_tab_matches(typed, &strip(), limit)
        .into_iter()
        .map(|TabMatch { tab, exact_url }| (tab.value(), exact_url))
        .collect()
}

#[test]
fn a_word_matches_the_other_tabs_titles_and_addresses() {
    assert_eq!(matches("rust", 3), vec![(2, false), (3, false)]);
}

#[test]
fn matching_ignores_case() {
    assert_eq!(matches("RUST", 3), vec![(2, false), (3, false)]);
}

#[test]
fn a_word_can_match_an_address() {
    assert_eq!(matches("mail", 3), vec![(4, false)]);
}

#[test]
fn a_title_phrase_matches_every_word() {
    assert_eq!(matches("cargo guide", 3), vec![(3, false)]);
}

#[test]
fn one_character_or_nothing_offers_no_tabs() {
    assert!(matches("m", 3).is_empty());
    assert!(matches("", 3).is_empty());
    assert!(matches("  ", 3).is_empty());
}

#[test]
fn about_tabs_are_never_offered() {
    assert!(matches("new", 3).is_empty());
}

#[test]
fn the_exact_address_of_another_tab_is_offered_first() {
    assert_eq!(
        matches("https://doc.rust-lang.org/cargo/", 3),
        vec![(3, true)]
    );
    assert_eq!(matches("doc.rust-lang.org/cargo", 3), vec![(3, true)]);
}

#[test]
fn an_exact_address_beats_a_word_match_earlier_in_the_strip() {
    assert_eq!(
        matches("https://doc.rust-lang.org/rust-by-example", 3),
        vec![(2, true)]
    );
}

#[test]
fn the_active_tab_is_never_offered_even_for_its_own_address() {
    assert!(matches("https://doc.rust-lang.org/book/", 3).is_empty());
}

#[test]
fn the_limit_caps_the_rows() {
    assert_eq!(matches("rust", 1), vec![(2, false)]);
    assert!(matches("rust", 0).is_empty());
}
