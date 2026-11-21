// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! sh-pages: `view-source:` in the core: which page has a source address, what a page may open,
//! what the address bar accepts and shows.
//!
//! `PAGE_CASES` and `TARGET_CASES` are the same table as the one in
//! `crates/mercury-view-source/tests/address.rs`, which tests `mercury_view_source::ViewSourceUrl`
//! (this crate depends only on `url`, so it keeps its own copy of the rule). A row changed here
//! changes there.

mod common;

use common::test_search;
use talaria_core::{
    AddressResolutionError, ErrorPageKind, NavigationTarget, NavigationVerdict, SecurityIndicator,
    VIEW_SOURCE_SCHEME, classify_address, display_origin, is_view_source, navigation_verdict,
    remembered_origin, resolve_address, security_indicator, view_source_address,
    view_source_target,
};
use url::Url;

fn url(text: &str) -> Url {
    Url::parse(text).expect("the test URL is valid")
}

/// The longest source address, `view-source:` included (8 KiB).
const LIMIT: usize = 8 * 1024;

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

#[test]
fn the_source_address_of_a_page() {
    for &(page, expected) in PAGE_CASES {
        assert_eq!(
            view_source_address(page).as_deref(),
            expected,
            "page {page:?}"
        );
    }
}

#[test]
fn the_address_a_source_address_names() {
    for &(source, expected) in TARGET_CASES {
        assert_eq!(
            view_source_target(&url(source)).as_ref().map(Url::as_str),
            expected,
            "source {source:?}"
        );
    }
}

#[test]
fn a_source_address_names_the_page_it_came_from() {
    for &(page, expected) in PAGE_CASES {
        let Some(address) = expected else { continue };
        let target = view_source_target(&url(address))
            .unwrap_or_else(|| panic!("{address:?} names no address"));
        // the page without credentials and fragment
        let mut shown = url(page.trim());
        shown.set_username("").expect("an http(s) URL takes it");
        shown.set_password(None).expect("an http(s) URL takes it");
        shown.set_fragment(None);
        assert_eq!(target, shown, "page {page:?}");
        // and asking again for the page the address names gives the same address
        assert_eq!(
            view_source_address(target.as_str()).as_deref(),
            Some(address),
            "page {page:?}"
        );
    }
}

#[test]
fn only_the_scheme_decides_whether_a_url_is_a_source_page() {
    assert_eq!(VIEW_SOURCE_SCHEME, "view-source");
    for source in [
        "view-source:https://example.com/",
        "VIEW-SOURCE:https://example.com/",
        "view-source:file:///etc/hosts",
        "view-source:",
    ] {
        assert!(is_view_source(&url(source)), "{source}");
    }
    for other in [
        "https://example.com/view-source:",
        "https://view-source/",
        "about:blank",
        "data:text/html,view-source:",
    ] {
        assert!(!is_view_source(&url(other)), "{other}");
    }
}

#[test]
fn the_limit_is_on_the_whole_source_address() {
    let fits = view_source_address(&page_for_source_address_of(LIMIT));
    assert_eq!(fits.as_ref().map(String::len), Some(LIMIT));
    assert_eq!(
        view_source_address(&page_for_source_address_of(LIMIT + 1)),
        None
    );
    // 9 KiB of path
    assert_eq!(
        view_source_address(&format!("https://example.com/{}", "a".repeat(9 * 1024))),
        None
    );
    // the limit applies to what is left after the fragment and credentials are dropped
    assert!(
        view_source_address(&format!(
            "{}#{}",
            page_for_source_address_of(LIMIT),
            "f".repeat(5000)
        ))
        .is_some()
    );
    // and a long source address names nothing
    let long = url(&format!(
        "view-source:{}",
        page_for_source_address_of(LIMIT + 1)
    ));
    assert_eq!(view_source_target(&long), None);
    let fits = url(&format!(
        "view-source:{}",
        page_for_source_address_of(LIMIT)
    ));
    assert!(view_source_target(&fits).is_some());
    // a typed address is counted as typed, fragment included, as `ViewSourceUrl::parse` counts it
    let with_fragment = url(&format!(
        "view-source:{}#{}",
        page_for_source_address_of(LIMIT),
        "f".repeat(100)
    ));
    assert_eq!(view_source_target(&with_fragment), None);
}

// ---- navigation_verdict ----

fn verdict(page: Option<&str>, target: &str) -> NavigationVerdict {
    let page = page.map(url);
    navigation_verdict(page.as_ref(), &url(target))
}

#[test]
fn a_web_page_may_not_open_a_source_page() {
    for page in [
        Some("https://example.com/"),
        Some("http://127.0.0.1:8000/"),
        Some("about:blank"),
        Some("file:///Users/me/notes.html"),
        Some("data:text/html,x"),
        // a page that has not committed yet
        None,
    ] {
        for target in [
            "view-source:https://example.com/",
            "VIEW-SOURCE:HTTP://EXAMPLE.COM/",
            "view-source:file:///etc/hosts",
            "view-source:",
        ] {
            assert_eq!(
                verdict(page, target),
                NavigationVerdict::DenyScheme,
                "{page:?} -> {target}"
            );
        }
    }
}

#[test]
fn a_source_page_may_open_the_source_of_a_web_page() {
    let page = Some("view-source:https://example.com/");
    for target in [
        "view-source:https://example.com/app.js",
        "view-source:http://other.test:8080/x?y=1",
        "view-source:HTTPS://EXAMPLE.COM/",
    ] {
        assert_eq!(verdict(page, target), NavigationVerdict::Allow, "{target}");
    }
}

#[test]
fn a_source_page_may_not_open_anything_but_the_source_of_a_web_page() {
    let page = Some("view-source:https://example.com/");
    for target in [
        "view-source:file:///etc/hosts",
        "view-source:javascript:alert(1)",
        "view-source:data:text/html,x",
        "view-source:about:blank",
        "view-source:view-source:https://example.com/",
        "view-source:",
        "view-source:/relative",
    ] {
        assert_eq!(
            verdict(page, target),
            NavigationVerdict::DenyScheme,
            "{target}"
        );
    }
}

#[test]
fn the_other_rows_do_not_change_for_a_source_page_or_without_one() {
    for page in [
        Some("view-source:https://example.com/"),
        Some("https://example.com/"),
    ] {
        for (target, expected) in [
            ("https://example.com/next", NavigationVerdict::Allow),
            ("http://example.com/", NavigationVerdict::Allow),
            ("about:blank", NavigationVerdict::Allow),
            (
                "mailto:someone@example.com",
                NavigationVerdict::OpenExternally,
            ),
            ("file:///etc/hosts", NavigationVerdict::DenyLocalFile),
            ("data:text/html,x", NavigationVerdict::DenyData),
            ("ftp://example.com/file", NavigationVerdict::DenyScheme),
            ("smb://server/share", NavigationVerdict::DenyScheme),
        ] {
            assert_eq!(verdict(page, target), expected, "{page:?} -> {target}");
        }
    }
    assert_eq!(
        verdict(None, "https://example.com/"),
        NavigationVerdict::Allow
    );
}

// ---- classify_address and resolve_address ----

#[test]
fn the_address_bar_accepts_the_source_of_a_web_page() {
    for (typed, expected) in [
        (
            "view-source:https://example.com",
            "view-source:https://example.com",
        ),
        (
            "  view-source:https://example.com/a?b=1  ",
            "view-source:https://example.com/a?b=1",
        ),
        (
            "VIEW-SOURCE:http://127.0.0.1:8080/x",
            "view-source:http://127.0.0.1:8080/x",
        ),
        (
            "view-source:http://[::1]:8080/",
            "view-source:http://[::1]:8080/",
        ),
    ] {
        let NavigationTarget::Url(address) =
            classify_address(typed).unwrap_or_else(|error| panic!("{typed:?}: {error}"))
        else {
            panic!("{typed:?} was taken for a search");
        };
        assert!(is_view_source(&address), "{typed:?}");
        assert!(view_source_target(&address).is_some(), "{typed:?}");
        assert_eq!(
            address.as_str().trim_end_matches('/'),
            expected.trim_end_matches('/'),
            "{typed:?}"
        );
        // never turned into a search, whichever way it is resolved
        let resolved = resolve_address(typed, &test_search()).expect("it resolves");
        assert_eq!(resolved, address, "{typed:?}");
    }
}

#[test]
fn the_address_bar_refuses_every_other_source_address() {
    for typed in [
        "view-source:",
        "view-source:ftp://example.com/",
        "view-source:javascript:alert(1)",
        "view-source:file:///etc/hosts",
        "view-source:data:text/html,x",
        "view-source:about:blank",
        "view-source:view-source:https://example.com/",
        "view-source:example.com",
        "view-source:/etc/hosts",
        "view-source:8080",
        "View-Source:",
    ] {
        match classify_address(typed) {
            Err(AddressResolutionError::UnsupportedScheme(scheme)) => {
                assert_eq!(scheme, "view-source", "{typed:?}");
            }
            other => panic!("{typed:?}: expected an unsupported scheme, got {other:?}"),
        }
    }
}

#[test]
fn the_address_bar_refuses_a_source_address_over_the_limit() {
    let fits = format!("view-source:{}", page_for_source_address_of(LIMIT));
    assert!(matches!(
        classify_address(&fits),
        Ok(NavigationTarget::Url(_))
    ));
    let too_long = format!("view-source:{}", page_for_source_address_of(LIMIT + 1));
    assert!(matches!(
        classify_address(&too_long),
        Err(AddressResolutionError::UnsupportedScheme(_))
    ));
}

#[test]
fn the_other_addresses_are_classified_as_before() {
    for typed in [
        "https://example.com/docs",
        "about:blank",
        "file:///tmp/page.html",
    ] {
        let Ok(NavigationTarget::Url(address)) = classify_address(typed) else {
            panic!("{typed:?} is not an address any more");
        };
        assert!(!is_view_source(&address), "{typed:?}");
    }
    assert!(matches!(
        classify_address("rust programming"),
        Ok(NavigationTarget::SearchQuery(_))
    ));
    assert!(matches!(
        classify_address("example.com"),
        Ok(NavigationTarget::Url(_))
    ));
    assert!(matches!(
        classify_address("ftp://example.com/file"),
        Err(AddressResolutionError::UnsupportedScheme(scheme)) if scheme == "ftp"
    ));
}

// ---- security_indicator, display_origin, remembered_origin ----

#[test]
fn a_source_page_has_neither_a_padlock_nor_a_warning() {
    for source in [
        "view-source:https://example.com/",
        "view-source:http://example.com/",
        "view-source:http://127.0.0.1:8000/",
        "view-source:https://expired.example/",
        "VIEW-SOURCE:https://example.com/",
    ] {
        for certificate_exception in [false, true] {
            let indicator = security_indicator(source, None, certificate_exception);
            assert_eq!(
                indicator,
                SecurityIndicator::None,
                "{source} (certificate exception: {certificate_exception})"
            );
            assert!(!indicator.is_warning(), "{source}");
            assert_eq!(indicator.label(), "", "{source}");
        }
    }
}

#[test]
fn the_security_indicator_of_the_other_pages_does_not_change() {
    for (address, expected) in [
        ("https://example.com/", SecurityIndicator::Secure),
        ("http://example.com/", SecurityIndicator::NotSecure),
        ("http://localhost:8000/", SecurityIndicator::LocalHost),
        ("file:///tmp/a.html", SecurityIndicator::File),
        ("about:blank", SecurityIndicator::None),
        ("data:text/html,x", SecurityIndicator::NotSecure),
        ("ftp://example.com/", SecurityIndicator::NotSecure),
        ("", SecurityIndicator::None),
    ] {
        assert_eq!(
            security_indicator(address, None, false),
            expected,
            "{address:?}"
        );
    }
    // an error page still wins over the scheme
    assert_eq!(
        security_indicator(
            "view-source:https://example.com/",
            Some(ErrorPageKind::Certificate),
            false
        ),
        SecurityIndicator::CertificateError
    );
    assert_eq!(
        security_indicator(
            "view-source:https://example.com/",
            Some(ErrorPageKind::Network),
            false
        ),
        SecurityIndicator::Internal
    );
}

#[test]
fn a_source_page_has_an_opaque_origin() {
    for source in [
        "view-source:https://example.com/",
        "view-source:http://127.0.0.1:8000/a",
        "VIEW-SOURCE:https://example.com:8443/",
    ] {
        assert_eq!(display_origin(source), None, "{source}");
        assert_eq!(remembered_origin(Some(&url(source))), None, "{source}");
    }
    // the page it shows keeps its own origin
    assert_eq!(
        display_origin("https://example.com/a"),
        Some("https://example.com".to_owned())
    );
}
