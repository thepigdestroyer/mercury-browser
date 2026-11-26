// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

#[path = "common/omnibox.rs"]
mod omnibox;

use omnibox::{engines, modes};
use talaria_core::{AddressResolutionError, Scope, ScopeNames, SearchModes, Typed};
use url::Url;

fn url(text: &str) -> Url {
    Url::parse(text).expect("the test URL should parse")
}

fn search(engine_id: &str, query: &str, by_keyword: bool) -> Typed {
    Typed::Search {
        engine_id: engine_id.to_owned(),
        query: query.to_owned(),
        by_keyword,
    }
}

fn scope(scope: Scope, query: &str) -> Typed {
    Typed::Scope {
        scope,
        query: query.to_owned(),
    }
}

#[test]
fn an_address_is_opened_and_plain_text_is_searched_with_the_default_engine() {
    let modes = modes();

    assert_eq!(
        modes.interpret("example.com/a").unwrap(),
        Typed::Address(url("https://example.com/a"))
    );
    assert_eq!(
        modes.interpret("rust book").unwrap(),
        search("duckduckgo", "rust book", false)
    );
}

#[test]
fn an_engine_keyword_chooses_the_engine_and_is_case_insensitive() {
    let modes = modes();

    assert_eq!(
        modes.interpret("@wikipedia rust book").unwrap(),
        search("wikipedia", "rust book", true)
    );
    assert_eq!(
        modes.interpret("@WIKIPEDIA  rust book ").unwrap(),
        search("wikipedia", "rust book", true)
    );
    assert_eq!(
        modes.interpret("d vec push").unwrap(),
        search("docs", "vec push", true)
    );
}

#[test]
fn a_keyword_alone_is_an_ordinary_search() {
    assert_eq!(
        modes().interpret("@wikipedia").unwrap(),
        search("duckduckgo", "@wikipedia", false)
    );
}

#[test]
fn scope_words_narrow_to_a_source() {
    let modes = modes();

    assert_eq!(modes.interpret("@tabs").unwrap(), scope(Scope::Tabs, ""));
    assert_eq!(
        modes.interpret("@tabs  foo bar").unwrap(),
        scope(Scope::Tabs, "foo bar")
    );
    assert_eq!(
        modes.interpret("@History x").unwrap(),
        scope(Scope::History, "x")
    );
    assert_eq!(
        modes.interpret("@bookmarks x").unwrap(),
        scope(Scope::Bookmarks, "x")
    );
    assert_eq!(
        modes.interpret("@actions mute").unwrap(),
        scope(Scope::Actions, "mute")
    );
}

#[test]
fn an_unknown_at_word_is_an_ordinary_search() {
    assert_eq!(
        modes().interpret("@unknown rest").unwrap(),
        search("duckduckgo", "@unknown rest", false)
    );
}

#[test]
fn empty_input_is_refused() {
    for text in ["", "   "] {
        assert!(
            matches!(
                modes().interpret(text),
                Err(AddressResolutionError::EmptyInput)
            ),
            "input {text:?}"
        );
    }
}

#[test]
fn a_script_address_is_refused() {
    assert!(matches!(
        modes().interpret("javascript:alert(1)"),
        Err(AddressResolutionError::UnsupportedScheme(scheme)) if scheme == "javascript"
    ));
}

#[test]
fn custom_scope_names_replace_the_defaults() {
    let names = ScopeNames {
        tabs: "@t".to_owned(),
        ..ScopeNames::default()
    };
    let modes = SearchModes::new(engines(), "duckduckgo", names);

    assert_eq!(modes.interpret("@t x").unwrap(), scope(Scope::Tabs, "x"));
    assert_eq!(
        modes.interpret("@tabs x").unwrap(),
        search("duckduckgo", "@tabs x", false)
    );
}

#[test]
fn a_scope_word_shadows_an_engine_keyword_spelt_the_same() {
    let modes = modes();

    assert!(modes.keyword_clashes_with_scope("@tabs"));
    assert!(modes.keyword_clashes_with_scope("@TABS"));
    assert!(!modes.keyword_clashes_with_scope("@wikipedia"));
    assert!(!modes.keyword_clashes_with_scope(""));
}

#[test]
fn an_unknown_default_engine_falls_back_to_the_first_engine() {
    let modes = SearchModes::new(engines(), "no-such-engine", ScopeNames::default());

    assert_eq!(
        modes.interpret("rust book").unwrap(),
        search("duckduckgo", "rust book", false)
    );
}

#[test]
fn a_search_with_no_engines_is_an_error() {
    let modes = SearchModes::new(Vec::new(), "duckduckgo", ScopeNames::default());

    assert!(matches!(
        modes.interpret("rust book"),
        Err(AddressResolutionError::Search(_))
    ));
}
