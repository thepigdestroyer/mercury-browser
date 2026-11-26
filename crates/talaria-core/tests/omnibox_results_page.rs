// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

#[path = "common/omnibox.rs"]
mod omnibox;

use omnibox::{engines, modes};
use talaria_core::{ScopeNames, SearchModes};
use url::Url;

fn url(text: &str) -> Url {
    Url::parse(text).expect("the test URL should parse")
}

fn terms(modes: &SearchModes, text: &str) -> Option<String> {
    modes.search_terms_for_url(&url(text))
}

#[test]
fn a_duckduckgo_results_page_shows_its_terms() {
    let modes = modes();

    assert_eq!(
        terms(&modes, "https://duckduckgo.com/?q=rust+book").as_deref(),
        Some("rust book")
    );
    assert_eq!(
        terms(&modes, "https://duckduckgo.com/?q=rust%20book&t=h_&ia=web").as_deref(),
        Some("rust book")
    );
    assert_eq!(
        terms(&modes, "https://duckduckgo.com/?t=h_&q=a%26b").as_deref(),
        Some("a&b")
    );
}

#[test]
fn a_home_page_or_an_empty_query_shows_the_url() {
    let modes = modes();

    assert_eq!(terms(&modes, "https://duckduckgo.com/"), None);
    assert_eq!(terms(&modes, "https://duckduckgo.com/?q="), None);
}

#[test]
fn a_look_alike_or_wrong_address_shows_the_url() {
    let modes = modes();

    for text in [
        "https://evil.test/?q=rust",
        "http://duckduckgo.com/?q=x",
        "https://user@duckduckgo.com/?q=x",
        "https://duckduckgo.com:8443/?q=x",
        "https://duckduckgo.com/html/?q=x",
    ] {
        assert_eq!(terms(&modes, text), None, "{text}");
    }
}

#[test]
fn control_bidi_and_invalid_terms_show_the_url() {
    let modes = modes();

    assert_eq!(terms(&modes, "https://duckduckgo.com/?q=a%0Ab"), None);
    assert_eq!(
        terms(&modes, "https://duckduckgo.com/?q=%E2%80%AEtxt"),
        None
    );
    assert_eq!(terms(&modes, "https://duckduckgo.com/?q=%FF"), None);
}

#[test]
fn terms_are_limited_to_256_characters() {
    let modes = modes();
    let at_limit = "a".repeat(256);
    let over_limit = "a".repeat(257);

    assert_eq!(
        terms(&modes, &format!("https://duckduckgo.com/?q={at_limit}")),
        Some(at_limit)
    );
    assert_eq!(
        terms(&modes, &format!("https://duckduckgo.com/?q={over_limit}")),
        None
    );
}

#[test]
fn another_engines_results_page_is_not_shown_as_terms() {
    assert_eq!(
        terms(
            &modes(),
            "https://en.wikipedia.org/w/index.php?title=Special:Search&search=rust"
        ),
        None
    );
}

#[test]
fn the_default_engine_decides_which_results_pages_match() {
    let modes = SearchModes::new(engines(), "wikipedia", ScopeNames::default());

    assert_eq!(
        terms(
            &modes,
            "https://en.wikipedia.org/w/index.php?title=Special:Search&search=rust"
        )
        .as_deref(),
        Some("rust")
    );
    assert_eq!(
        terms(&modes, "https://en.wikipedia.org/w/index.php?search=rust"),
        None,
        "title=Special:Search is required"
    );
}

#[test]
fn a_path_template_shows_the_decoded_last_segment() {
    let modes = SearchModes::new(engines(), "path", ScopeNames::default());

    assert_eq!(
        terms(&modes, "https://example.test/search/rust%20book").as_deref(),
        Some("rust book")
    );
    assert_eq!(terms(&modes, "https://example.test/search/"), None);
    assert_eq!(terms(&modes, "https://example.test/search/a/b"), None);
}
