// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

#[path = "common/omnibox.rs"]
mod omnibox;

use omnibox::modes;
use talaria_core::{AtCandidate, CompletionMode, Scope, TabCompletion};

fn engine(engine_id: &str) -> CompletionMode {
    CompletionMode::Engine {
        engine_id: engine_id.to_owned(),
    }
}

fn completion(text: &str, mode: CompletionMode) -> Option<TabCompletion> {
    Some(TabCompletion {
        text: text.to_owned(),
        mode,
    })
}

fn candidate(text: &str, label: &str, mode: CompletionMode) -> AtCandidate {
    AtCandidate {
        text: text.to_owned(),
        label: label.to_owned(),
        mode,
    }
}

#[test]
fn an_exact_keyword_completes_with_its_space() {
    let modes = modes();

    assert_eq!(
        modes.complete_with_tab("@wikipedia"),
        completion("@wikipedia ", engine("wikipedia"))
    );
    assert_eq!(
        modes.complete_with_tab("wikipedia"),
        completion("@wikipedia ", engine("wikipedia"))
    );
}

#[test]
fn an_unambiguous_prefix_of_a_keyword_completes() {
    assert_eq!(
        modes().complete_with_tab("@wik"),
        completion("@wikipedia ", engine("wikipedia"))
    );
}

#[test]
fn an_ambiguous_prefix_does_not_complete() {
    assert_eq!(modes().complete_with_tab("@b"), None);
}

#[test]
fn a_word_without_an_at_only_completes_when_it_is_the_keyword() {
    assert_eq!(modes().complete_with_tab("wiki"), None);
}

#[test]
fn an_exact_keyword_wins_over_longer_keywords() {
    assert_eq!(
        modes().complete_with_tab("@bing"),
        completion("@bing ", engine("bing"))
    );
}

#[test]
fn a_scope_word_completes_to_its_scope() {
    let expected = completion("@tabs ", CompletionMode::Scope(Scope::Tabs));

    assert_eq!(modes().complete_with_tab("@tabs"), expected);
    assert_eq!(modes().complete_with_tab("@ta"), expected);
}

#[test]
fn a_single_letter_keyword_completes_without_an_at() {
    assert_eq!(
        modes().complete_with_tab("d"),
        completion("d ", engine("docs"))
    );
}

#[test]
fn text_already_past_the_keyword_or_empty_does_not_complete() {
    let modes = modes();

    assert_eq!(modes.complete_with_tab("@wikipedia rust"), None);
    assert_eq!(modes.complete_with_tab(""), None);
    assert_eq!(modes.complete_with_tab("@"), None);
}

#[test]
fn a_bare_at_lists_scopes_then_engines() {
    let labels: Vec<_> = modes().at_candidates("@");

    assert_eq!(
        labels,
        vec![
            candidate("@tabs", "Tabs", CompletionMode::Scope(Scope::Tabs)),
            candidate(
                "@bookmarks",
                "Bookmarks",
                CompletionMode::Scope(Scope::Bookmarks)
            ),
            candidate("@history", "History", CompletionMode::Scope(Scope::History)),
            candidate("@actions", "Actions", CompletionMode::Scope(Scope::Actions)),
            candidate("@duckduckgo", "DuckDuckGo", engine("duckduckgo")),
            candidate("@wikipedia", "Wikipedia", engine("wikipedia")),
            candidate("@bing", "Bing", engine("bing")),
            candidate("@path", "Path", engine("path")),
        ]
    );
}

#[test]
fn at_candidates_narrow_by_prefix() {
    let texts: Vec<_> = modes()
        .at_candidates("@b")
        .into_iter()
        .map(|candidate| candidate.text)
        .collect();

    assert_eq!(texts, ["@bookmarks", "@bing"]);
}

#[test]
fn at_candidates_are_empty_for_no_match_or_no_at() {
    assert!(modes().at_candidates("@zzz").is_empty());
    assert!(modes().at_candidates("wik").is_empty());
}
