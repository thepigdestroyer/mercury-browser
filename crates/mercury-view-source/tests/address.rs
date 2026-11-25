// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `ViewSourceUrl`: which texts are `view-source:` addresses and what they name.
//!
//! `PAGE_CASES` and `TARGET_CASES` are the same table as the one in
//! `crates/talaria-core/tests/view_source.rs`, which tests `talaria_core::view_source_address`
//! and `view_source_target` (that crate depends only on `url`, so it keeps its own copy of the
//! rule). A row changed here changes there.

use mercury_view_source::{MAX_ADDRESS_BYTES, UrlError, ViewSourceUrl};
use url::Url;

/// The longest source address, `view-source:` included (8 KiB).
const LIMIT: usize = MAX_ADDRESS_BYTES;

/// A page address of exactly `total` bytes once `view-source:` is put in front of it.
fn page_for_source_address_of(total: usize) -> String {
    let start = "https://example.com/";
    let padding = total - "view-source:".len() - start.len();
    format!("{start}{}", "a".repeat(padding))
}

/// (the text of a tab's address, the source address of that page, if it has one)
const PAGE_CASES: &[(&str, Option<&str>)] = &[
    // http and https pages
    (
        "https://example.com/",
        Some("view-source:https://example.com/"),
    ),
    (
        "http://example.com/",
        Some("view-source:http://example.com/"),
    ),
    (
        "https://example.com",
        Some("view-source:https://example.com/"),
    ),
    (
        "https://example.com:8443/a/b.html?x=1&y=2",
        Some("view-source:https://example.com:8443/a/b.html?x=1&y=2"),
    ),
    (
        "https://example.com:443/",
        Some("view-source:https://example.com/"),
    ),
    (
        "http://127.0.0.1:8000/page",
        Some("view-source:http://127.0.0.1:8000/page"),
    ),
    (
        "http://[::1]:8080/x",
        Some("view-source:http://[::1]:8080/x"),
    ),
    // the scheme and the host are case-insensitive, the path is not
    (
        "HTTPS://EXAMPLE.COM/Path",
        Some("view-source:https://example.com/Path"),
    ),
    // an international host name is shown as the engine sends it
    (
        "https://b\u{fc}cher.example/",
        Some("view-source:https://xn--bcher-kva.example/"),
    ),
    // white space around the text is ignored
    (
        "  https://example.com/a  ",
        Some("view-source:https://example.com/a"),
    ),
    // credentials are dropped, whichever part is given
    (
        "https://user:secret@example.com/a",
        Some("view-source:https://example.com/a"),
    ),
    (
        "https://user@example.com/a",
        Some("view-source:https://example.com/a"),
    ),
    (
        "https://:secret@example.com/a",
        Some("view-source:https://example.com/a"),
    ),
    // the fragment is dropped, the query stays
    (
        "https://example.com/a?b=1#top",
        Some("view-source:https://example.com/a?b=1"),
    ),
    (
        "https://example.com/#",
        Some("view-source:https://example.com/"),
    ),
    (
        "http://u:p@example.com:8080/a?b#c",
        Some("view-source:http://example.com:8080/a?b"),
    ),
    // pages without a source address
    ("about:blank", None),
    ("about:srcdoc", None),
    ("file:///etc/hosts", None),
    ("data:text/html,x", None),
    (
        "blob:https://example.com/0d6b2f4e-6d3c-4a8e-9a1d-1f1f1f1f1f1f",
        None,
    ),
    ("javascript:alert(1)", None),
    ("ftp://example.com/", None),
    ("mailto:someone@example.com", None),
    // a source page has no source of its own
    ("view-source:https://example.com/", None),
    ("VIEW-SOURCE:http://example.com/", None),
    // not an address
    ("", None),
    ("   ", None),
    ("example.com", None),
    ("not a url", None),
    ("https://", None),
    ("http://", None),
];

/// (a URL, the `http(s)` address inside it, if it is a source address of one)
const TARGET_CASES: &[(&str, Option<&str>)] = &[
    (
        "view-source:https://example.com/",
        Some("https://example.com/"),
    ),
    (
        "view-source:http://example.com/a?b",
        Some("http://example.com/a?b"),
    ),
    // upper case, and what the engine keeps of it
    (
        "VIEW-SOURCE:HTTPS://EXAMPLE.COM/Path",
        Some("https://example.com/Path"),
    ),
    // the fragment and the credentials are dropped
    (
        "view-source:https://example.com/b?c#d",
        Some("https://example.com/b?c"),
    ),
    (
        "view-source:https://user:secret@example.com/",
        Some("https://example.com/"),
    ),
    // a space is escaped as the engine escapes it
    (
        "view-source:https://example.com/a b",
        Some("https://example.com/a%20b"),
    ),
    ("view-source:http://[::1]:8080/", Some("http://[::1]:8080/")),
    // nothing but an http(s) address names a source
    ("view-source:file:///etc/hosts", None),
    ("view-source:javascript:alert(1)", None),
    ("view-source:data:text/html,x", None),
    ("view-source:about:blank", None),
    ("view-source:ftp://example.com/", None),
    (
        "view-source:blob:https://example.com/0d6b2f4e-6d3c-4a8e-9a1d-1f1f1f1f1f1f",
        None,
    ),
    ("view-source:view-source:https://example.com/", None),
    ("view-source:", None),
    ("view-source:/relative", None),
    ("view-source:example.com", None),
    ("view-source:https://", None),
    // not a source address at all
    ("https://example.com/", None),
    ("about:blank", None),
    ("file:///etc/hosts", None),
];

fn url(text: &str) -> Url {
    Url::parse(text).expect("the test URL is valid")
}

fn parse(text: &str) -> Result<ViewSourceUrl, UrlError> {
    ViewSourceUrl::parse(text)
}

/// The source address of the page a tab shows, if it has one: the same question as
/// `talaria_core::view_source_address`, which is given the text.
fn address_of_page(text: &str) -> Option<String> {
    let page = Url::parse(text.trim()).ok()?;
    ViewSourceUrl::for_page(&page).map(|source| source.to_url().to_string())
}

#[test]
fn the_source_address_of_a_page() {
    for &(page, expected) in PAGE_CASES {
        assert_eq!(address_of_page(page).as_deref(), expected, "page {page:?}");
    }
}

#[test]
fn the_address_a_source_address_names() {
    for &(source, expected) in TARGET_CASES {
        assert_eq!(
            parse(source)
                .ok()
                .map(|parsed| parsed.target().to_string())
                .as_deref(),
            expected,
            "source {source:?}"
        );
    }
}

#[test]
fn the_limit_is_on_the_whole_source_address() {
    let fits = address_of_page(&page_for_source_address_of(LIMIT));
    assert_eq!(fits.as_ref().map(String::len), Some(LIMIT));
    assert_eq!(
        address_of_page(&page_for_source_address_of(LIMIT + 1)),
        None
    );
    assert_eq!(
        address_of_page(&format!("https://example.com/{}", "a".repeat(9 * 1024))),
        None
    );
    // The limit applies to what is left after the fragment and credentials are dropped.
    assert!(
        address_of_page(&format!(
            "{}#{}",
            page_for_source_address_of(LIMIT),
            "f".repeat(5000)
        ))
        .is_some()
    );
    // A typed address is counted as typed, fragment included.
    let exact = format!("view-source:{}", page_for_source_address_of(LIMIT));
    assert_eq!(exact.len(), LIMIT);
    assert!(parse(&exact).is_ok());
    assert_eq!(
        parse(&format!(
            "view-source:{}",
            page_for_source_address_of(LIMIT + 1)
        ))
        .unwrap_err(),
        UrlError::Invalid
    );
    assert_eq!(
        parse(&format!("{exact}#{}", "f".repeat(100))).unwrap_err(),
        UrlError::Invalid
    );
    // 9 KiB, as typed.
    assert_eq!(
        parse(&format!(
            "view-source:https://example.com/{}",
            "a".repeat(9 * 1024)
        ))
        .unwrap_err(),
        UrlError::Invalid
    );
}

#[test]
fn an_address_that_grows_when_it_is_normalised_is_held_to_the_limit_too() {
    // Each space becomes `%20`: the text fits, the address the engine would be given does not.
    let head = "view-source:https://example.com/";
    let spaces = (LIMIT - head.len()) / 2;
    let text = format!("{head}{}", " ".repeat(spaces));
    assert!(text.len() <= LIMIT);
    // Trailing spaces are trimmed, so put something after them.
    let text = format!("{text}x");
    assert_eq!(parse(&text).unwrap_err(), UrlError::Invalid);
    let spaced_page = format!("https://example.com/{}x", " ".repeat(spaces));
    assert_eq!(address_of_page(&spaced_page), None);
}

#[test]
fn texts_that_are_not_source_addresses_say_why() {
    for text in [
        "",
        "   ",
        "https://example.com/",
        "view-source",
        "view-sourc:https://example.com/",
        "view source:https://example.com/",
        "xview-source:https://example.com/",
        "view-source;https://example.com/",
        "view-sourc\u{e9}https://example.com/",
        "\u{e9}",
    ] {
        assert_eq!(
            parse(text).unwrap_err(),
            UrlError::NotViewSource,
            "{text:?}"
        );
    }
    for text in [
        "view-source:file:///etc/hosts",
        "view-source:javascript:alert(1)",
        "view-source:data:text/html,hi",
        "view-source:about:blank",
        "view-source:ftp://example.com/",
        "view-source:ws://example.com/",
        "view-source:mailto:a@example.com",
        "view-source:localhost:8080/x",
        "view-source:view-source:https://example.com/",
        "VIEW-SOURCE:View-Source:https://example.com/",
        "view-source: view-source:https://example.com/",
    ] {
        assert_eq!(parse(text).unwrap_err(), UrlError::NotHttp, "{text:?}");
    }
    for text in [
        "view-source:",
        "view-source:   ",
        "view-source:/relative",
        "view-source:example.com",
        "view-source:example.com/path",
        "view-source:https://",
        "view-source:http://[::1",
        "view-source:https://exa mple.com/",
        "view-source:http://example.com:99999/",
        format!("view-source:https://example.com/{}", "a".repeat(LIMIT)).as_str(),
    ] {
        assert_eq!(parse(text).unwrap_err(), UrlError::Invalid, "{text:?}");
    }
}

#[test]
fn the_scheme_is_case_insensitive_and_the_text_is_trimmed() {
    let expected = "view-source:https://a.test/b";
    for text in [
        "view-source:https://a.test/b",
        "VIEW-SOURCE:https://a.test/b",
        "View-Source:HTTPS://A.TEST/b",
        "  view-source:https://a.test/b  ",
        "\t\r\nview-source:https://a.test/b\r\n\t",
        "\u{a0}view-source:https://a.test/b\u{2003}",
        "view-source: https://a.test/b",
        "view-source:https://a.test/b#",
        "view-source:https://a.test/b#frag",
        "view-source:https://u:p@a.test/b",
        "view-source:https://a.test:443/b",
        "view-source:https://a.test/\tb\n",
    ] {
        assert_eq!(
            parse(text).expect(text).to_url().as_str(),
            expected,
            "{text:?}"
        );
    }
}

#[test]
fn spaces_and_international_names_are_written_as_the_engine_writes_them() {
    for (text, expected) in [
        ("view-source:https://a.test/b c", "https://a.test/b%20c"),
        (
            "view-source:https://a.test/?q=a b",
            "https://a.test/?q=a%20b",
        ),
        (
            "view-source:https://b\u{fc}cher.example/",
            "https://xn--bcher-kva.example/",
        ),
        (
            "view-source:https://\u{65e5}\u{672c}\u{8a9e}.example/p\u{e4}th",
            "https://xn--wgv71a119e.example/p%C3%A4th",
        ),
        ("view-source:http://[2001:DB8::1]/", "http://[2001:db8::1]/"),
        ("view-source:http://[::1]:8080/x", "http://[::1]:8080/x"),
        (
            "view-source:http://127.0.0.1:8000",
            "http://127.0.0.1:8000/",
        ),
        ("view-source:http://0x7f.1/", "http://127.0.0.1/"),
        (
            "view-source:http://EXAMPLE.com:80/A",
            "http://example.com/A",
        ),
    ] {
        assert_eq!(
            parse(text).expect(text).target().as_str(),
            expected,
            "{text:?}"
        );
    }
}

#[test]
fn to_url_and_parse_are_inverses() {
    let mut texts: Vec<&str> = TARGET_CASES
        .iter()
        .filter(|(_, target)| target.is_some())
        .map(|(source, _)| *source)
        .collect();
    texts.extend([
        "view-source:https://a.test/b?c=d&e=f",
        "view-source:https://a.test/%7Euser/%E2%82%AC",
        "view-source:http://[::1]:1/",
    ]);
    for text in texts {
        let parsed = parse(text).expect(text);
        let address = parsed.to_url();
        assert_eq!(address.scheme(), "view-source", "{text:?}");
        assert!(address.as_str().starts_with("view-source:http"), "{text:?}");
        assert!(address.as_str().len() <= LIMIT, "{text:?}");
        // The address as a URL and as text both come back to the same value.
        assert_eq!(parse(address.as_str()).as_ref(), Ok(&parsed), "{text:?}");
        // And so does the page it names.
        assert_eq!(
            ViewSourceUrl::for_page(parsed.target()).as_ref(),
            Some(&parsed),
            "{text:?}"
        );
        assert!(parsed.target().username().is_empty(), "{text:?}");
        assert_eq!(parsed.target().password(), None, "{text:?}");
        assert_eq!(parsed.target().fragment(), None, "{text:?}");
    }
}

#[test]
fn a_page_without_a_web_address_has_no_source_address() {
    for page in [
        "file:///etc/hosts",
        "about:blank",
        "data:text/html,x",
        "blob:https://example.com/0d6b2f4e-6d3c-4a8e-9a1d-1f1f1f1f1f1f",
        "ftp://example.com/",
        "ws://example.com/",
        "wss://example.com/",
        "javascript:alert(1)",
        "mailto:a@example.com",
        "view-source:https://example.com/",
    ] {
        assert_eq!(ViewSourceUrl::for_page(&url(page)), None, "{page}");
    }
}

#[test]
fn a_link_is_resolved_against_the_page() {
    let page = parse("view-source:https://example.com/dir/sub/page.html?a=1#top").unwrap();
    assert_eq!(
        page.target().as_str(),
        "https://example.com/dir/sub/page.html?a=1"
    );
    for (href, expected) in [
        ("other.html", "https://example.com/dir/sub/other.html"),
        ("./other.html", "https://example.com/dir/sub/other.html"),
        ("sub2/x.js", "https://example.com/dir/sub/sub2/x.js"),
        ("/abs/path", "https://example.com/abs/path"),
        ("../up.css", "https://example.com/dir/up.css"),
        ("../../up.css", "https://example.com/up.css"),
        ("../../../../way/up", "https://example.com/way/up"),
        ("?q=2", "https://example.com/dir/sub/page.html?q=2"),
        ("//cdn.test/lib.js", "https://cdn.test/lib.js"),
        (
            "//cdn.test:8443/lib.js?v=1",
            "https://cdn.test:8443/lib.js?v=1",
        ),
        ("http://other.test/a", "http://other.test/a"),
        ("HTTPS://OTHER.test/A", "https://other.test/A"),
        ("https://u:p@other.test/a", "https://other.test/a"),
        ("x.html#frag", "https://example.com/dir/sub/x.html"),
        ("a b.html", "https://example.com/dir/sub/a%20b.html"),
        ("  padded.html  ", "https://example.com/dir/sub/padded.html"),
        ("tab\tbed.html", "https://example.com/dir/sub/tabbed.html"),
        (
            "caf\u{e9}.html",
            "https://example.com/dir/sub/caf%C3%A9.html",
        ),
        // Only a fragment, or nothing: the page itself, since the fragment is not kept.
        ("#section", "https://example.com/dir/sub/page.html?a=1"),
        ("#", "https://example.com/dir/sub/page.html?a=1"),
        ("", "https://example.com/dir/sub/page.html?a=1"),
    ] {
        let link = page
            .link_to(href)
            .unwrap_or_else(|| panic!("{href:?} has no link"));
        assert_eq!(link.target().as_str(), expected, "{href:?}");
        assert_eq!(
            link.to_url().as_str(),
            format!("view-source:{expected}"),
            "{href:?}"
        );
    }
}

#[test]
fn a_link_to_anything_but_a_web_page_is_none() {
    let page = parse("view-source:http://example.com/a/b").unwrap();
    for href in [
        "javascript:alert(1)",
        "JavaScript:void(0)",
        "data:text/html,<p>x",
        "mailto:someone@example.com",
        "tel:+15555550100",
        "ftp://example.com/",
        "file:///etc/hosts",
        "blob:https://example.com/x",
        "about:blank",
        "view-source:https://example.com/",
        "ws://example.com/",
        "https://",
        "https://[::1",
        "http://exa mple.com/",
        "http://example.com:99999/",
    ] {
        assert_eq!(page.link_to(href), None, "{href:?}");
    }
    // From an http page an https link is still a link, and the other way round.
    assert!(page.link_to("https://example.com/").is_some());
    let secure = parse("view-source:https://example.com/").unwrap();
    assert!(secure.link_to("http://example.com/").is_some());
}

#[test]
fn a_link_that_would_make_the_address_too_long_is_none() {
    let page = parse("view-source:https://example.com/").unwrap();
    let longest = page_for_source_address_of(LIMIT);
    let fits = longest.trim_start_matches("https://example.com");
    assert!(page.link_to(fits).is_some());
    assert!(page.link_to(&format!("{fits}a")).is_none());
    assert!(page.link_to(&format!("/{}", "a".repeat(100_000))).is_none());
    // A long fragment does not count: it is dropped.
    assert!(page.link_to(&format!("#{}", "f".repeat(100_000))).is_some());
}

#[test]
fn a_page_address_that_is_hostile_does_not_panic() {
    for text in [
        "view-source:\u{0}",
        "view-source:https://\u{0}/",
        "view-source:https://a.test/\u{202e}\u{2028}",
        "view-source:%00",
        "view-source:https://%zz/",
        "view-source:https://a.test:-1/",
        "view-source:////////",
        "view-source:https:////a.test//b",
        "view-source:https://a.test/..%2f..%2f",
    ] {
        // Whatever the answer, it is an answer.
        let _ = parse(text);
    }
    let long = format!("view-source:{}", "view-source:".repeat(100));
    assert_eq!(parse(&long).unwrap_err(), UrlError::NotHttp);
    let mut deep = String::from("view-source:https://a.test/");
    deep.push_str(&"../".repeat(2000));
    let _ = parse(&deep);
}
