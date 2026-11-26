// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use talaria_core::{MAX_PASTE_BYTES, Paste, paste_and_go};
use url::Url;

fn url(text: &str) -> Url {
    Url::parse(text).expect("the test URL should parse")
}

#[test]
fn a_single_address_goes_and_is_labelled_paste_and_go() {
    let paste = paste_and_go("https://example.com/a");

    assert_eq!(paste, Paste::Go(url("https://example.com/a")));
    assert_eq!(paste.menu_label(), Some("Paste and Go"));
}

#[test]
fn a_bare_host_is_trimmed_and_goes() {
    assert_eq!(
        paste_and_go("  example.com  "),
        Paste::Go(url("https://example.com/"))
    );
}

#[test]
fn file_addresses_go() {
    assert_eq!(
        paste_and_go("file:///tmp/x.html"),
        Paste::Go(url("file:///tmp/x.html"))
    );
}

#[test]
fn search_text_is_searched_and_labelled_paste_and_search() {
    let paste = paste_and_go("rust book");

    assert_eq!(paste, Paste::Search("rust book".to_owned()));
    assert_eq!(paste.menu_label(), Some("Paste and Search"));
}

#[test]
fn inner_white_space_of_one_line_is_kept() {
    assert_eq!(
        paste_and_go("  rust   book "),
        Paste::Search("rust   book".to_owned())
    );
}

#[test]
fn empty_or_blank_text_is_nothing_and_has_no_menu_item() {
    for text in ["", "   ", "\n\n"] {
        let paste = paste_and_go(text);

        assert_eq!(paste, Paste::Nothing, "input {text:?}");
        assert_eq!(paste.menu_label(), None, "input {text:?}");
    }
}

#[test]
fn scripts_and_data_urls_are_never_pasted() {
    for text in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "data:text/html,hi",
        "vbscript:x",
        "javascript:1",
        "blob:https://a.test/x",
    ] {
        assert_eq!(paste_and_go(text), Paste::Nothing, "input {text:?}");
    }
}

#[test]
fn several_addresses_on_separate_lines_go_in_one_paste() {
    let expected = Paste::GoMany(vec![url("https://a.test/"), url("https://b.test/")]);

    assert_eq!(paste_and_go("https://a.test\nhttps://b.test"), expected);
    assert_eq!(paste_and_go("https://a.test\r\nhttps://b.test"), expected);
    assert_eq!(paste_and_go("https://a.test\n\n\nhttps://b.test"), expected);
}

#[test]
fn one_address_with_a_trailing_line_break_goes() {
    assert_eq!(
        paste_and_go("https://a.test\n"),
        Paste::Go(url("https://a.test/"))
    );
}

#[test]
fn an_address_and_text_on_separate_lines_is_one_search() {
    assert_eq!(
        paste_and_go("https://a.test\nrust book"),
        Paste::Search("https://a.test rust book".to_owned())
    );
}

#[test]
fn lines_of_text_are_joined_into_one_search() {
    assert_eq!(
        paste_and_go("rust\n  book"),
        Paste::Search("rust book".to_owned())
    );
}

#[test]
fn many_addresses_are_capped_at_twenty() {
    let lines = |count: usize| {
        (1..=count)
            .map(|i| format!("https://a.test/{i}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    match paste_and_go(&lines(20)) {
        Paste::GoMany(urls) => assert_eq!(urls.len(), 20),
        other => panic!("twenty addresses should be GoMany, got {other:?}"),
    }
    assert_eq!(paste_and_go(&lines(21)), Paste::Nothing);
}

#[test]
fn control_characters_are_refused_but_tabs_are_text() {
    assert_eq!(paste_and_go("a\u{0}b"), Paste::Nothing);
    assert_eq!(paste_and_go("a\u{7}b"), Paste::Nothing);
    assert_eq!(paste_and_go("a\tb"), Paste::Search("a\tb".to_owned()));
}

#[test]
fn text_is_limited_to_the_byte_cap() {
    let at_cap = "x".repeat(MAX_PASTE_BYTES);
    let over_cap = "x".repeat(MAX_PASTE_BYTES + 1);

    assert_eq!(paste_and_go(&at_cap), Paste::Search(at_cap.clone()));
    assert_eq!(paste_and_go(&over_cap), Paste::Nothing);
}

#[test]
fn the_group_label_is_go() {
    let paste = paste_and_go("https://a.test\nhttps://b.test");

    assert_eq!(paste.menu_label(), Some("Paste and Go"));
}
