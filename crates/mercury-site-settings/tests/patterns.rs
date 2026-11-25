// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Origins and origin patterns: canonical forms, refusals, matching and precedence.

use mercury_site_settings::{Origin, OriginPattern, PatternError, Specificity};

fn origin(text: &str) -> Origin {
    Origin::parse(text).unwrap_or_else(|error| panic!("{text}: {error:?}"))
}

fn pattern(text: &str) -> OriginPattern {
    OriginPattern::parse(text).unwrap_or_else(|error| panic!("{text}: {error:?}"))
}

#[test]
fn an_origin_has_one_canonical_spelling() {
    let table = [
        ("https://Example.COM", "https://example.com"),
        ("https://example.com:443", "https://example.com"),
        ("https://example.com/path?q#f", "https://example.com"),
        ("https://user:secret@example.com/", "https://example.com"),
        ("http://example.com:80", "http://example.com"),
        ("http://example.com:8080", "http://example.com:8080"),
        ("http://localhost:3000", "http://localhost:3000"),
        ("https://[::1]:8443", "https://[::1]:8443"),
        ("https://[::1]", "https://[::1]"),
        ("http://127.0.0.1:80", "http://127.0.0.1"),
        ("https://bücher.example", "https://xn--bcher-kva.example"),
    ];
    for (input, expected) in table {
        assert_eq!(origin(input).to_string(), expected, "{input}");
    }
    assert_eq!(origin("https://a.com"), origin("https://A.com:443/x"));
}

#[test]
fn an_origin_exposes_its_parts() {
    let origin = origin("https://www.example.co.uk:8443/x");
    assert_eq!(origin.scheme(), "https");
    assert_eq!(origin.host(), "www.example.co.uk");
    assert_eq!(origin.port(), 8443);
    assert_eq!(
        origin.registrable_domain().as_deref(),
        Some("example.co.uk")
    );
    assert_eq!(self::origin("https://127.0.0.1").registrable_domain(), None);
    assert_eq!(self::origin("http://localhost").registrable_domain(), None);
    assert_eq!(self::origin("https://github.io").registrable_domain(), None);
    assert_eq!(
        self::origin("https://a.github.io")
            .registrable_domain()
            .as_deref(),
        Some("a.github.io")
    );
}

#[test]
fn origins_that_are_not_http_are_refused() {
    let table = [
        ("", PatternError::Empty),
        ("   ", PatternError::Empty),
        ("ftp://a.com", PatternError::NotHttp),
        ("about:blank", PatternError::NotHttp),
        ("file:///x", PatternError::NotHttp),
        ("data:text/plain,hi", PatternError::NotHttp),
        ("https://", PatternError::Invalid),
        ("not a url", PatternError::Invalid),
    ];
    for (input, expected) in table {
        assert_eq!(Origin::parse(input), Err(expected), "{input:?}");
    }
}

#[test]
fn patterns_that_could_cover_too_much_are_refused() {
    let table = [
        ("*.com", PatternError::SingleLabel),
        ("*.co.uk", PatternError::PublicSuffix),
        ("*.github.io", PatternError::PublicSuffix),
        ("site:co.uk", PatternError::PublicSuffix),
        ("site:com", PatternError::SingleLabel),
        ("*.127.0.0.1", PatternError::IpAddress),
        ("site:10.0.0.1", PatternError::IpAddress),
        ("*.[::1]", PatternError::IpAddress),
        ("site:localhost", PatternError::SingleLabel),
        ("*.localhost", PatternError::SingleLabel),
        ("https://a.com/x", PatternError::HasPath),
        ("https://a.com/?q=1", PatternError::HasPath),
        ("*.a.com/x", PatternError::Invalid),
        ("a.com", PatternError::Invalid),
        ("*.", PatternError::Invalid),
        ("site:", PatternError::Invalid),
        ("site:www.a.com", PatternError::NotASite),
        ("site:x.a.co.uk", PatternError::NotASite),
        ("https://u@a.com", PatternError::Invalid),
        ("", PatternError::Empty),
        ("ftp://a.com", PatternError::NotHttp),
    ];
    for (input, expected) in table {
        assert_eq!(OriginPattern::parse(input), Err(expected), "{input:?}");
    }
}

#[test]
fn one_set_of_sites_has_one_spelling() {
    let table = [
        ("*", "*"),
        ("*.example.com", "site:example.com"),
        ("site:Example.COM", "site:example.com"),
        ("*.shop.example.com", "*.shop.example.com"),
        ("*.Shop.Example.com", "*.shop.example.com"),
        ("*.a.co.uk", "site:a.co.uk"),
        ("*.x.a.co.uk", "*.x.a.co.uk"),
        ("https://Example.com:443/", "https://example.com"),
        ("http://localhost:3000", "http://localhost:3000"),
        ("*.bücher.example", "site:xn--bcher-kva.example"),
    ];
    for (input, expected) in table {
        let parsed = pattern(input);
        assert_eq!(parsed.to_string(), expected, "{input}");
        assert_eq!(pattern(expected), parsed, "re-parsing {expected}");
    }
    assert_eq!(
        OriginPattern::subdomains("example.com").unwrap(),
        OriginPattern::site("example.com").unwrap()
    );
    assert_eq!(
        OriginPattern::for_origin(&origin("https://a.com:443")).to_string(),
        "https://a.com"
    );
    assert_eq!(
        OriginPattern::for_site_of(&origin("https://www.a.co.uk:8443"))
            .unwrap()
            .to_string(),
        "site:a.co.uk"
    );
    assert_eq!(
        OriginPattern::for_site_of(&origin("https://127.0.0.1")),
        Err(PatternError::IpAddress)
    );
    assert_eq!(
        OriginPattern::for_site_of(&origin("http://localhost:3000")),
        Err(PatternError::SingleLabel)
    );
}

#[test]
fn patterns_match_the_origins_they_say() {
    use Specificity::{AllSites, Origin as Exact, Site, Subdomains};
    let table: &[(&str, &str, Option<Specificity>)] = &[
        ("https://a.com", "https://a.com", Some(Exact)),
        ("https://a.com", "https://a.com:443", Some(Exact)),
        ("https://a.com", "http://a.com", None),
        ("https://a.com", "https://a.com:8443", None),
        ("https://a.com", "https://www.a.com", None),
        ("site:a.com", "https://a.com", Some(Site)),
        ("site:a.com", "http://www.a.com:81", Some(Site)),
        ("site:a.com", "https://x.y.a.com", Some(Site)),
        ("site:a.com", "https://nota.com", None),
        ("site:a.com", "https://a.com.evil.org", None),
        ("site:a.com", "https://a.co", None),
        ("site:a.com", "https://127.0.0.1", None),
        ("site:a.co.uk", "https://www.a.co.uk", Some(Site)),
        ("site:a.co.uk", "https://b.co.uk", None),
        ("site:user.github.io", "https://user.github.io", Some(Site)),
        ("site:user.github.io", "https://other.github.io", None),
        ("*.shop.a.com", "https://shop.a.com", Some(Subdomains(3))),
        (
            "*.shop.a.com",
            "https://x.shop.a.com:99",
            Some(Subdomains(3)),
        ),
        ("*.shop.a.com", "https://a.com", None),
        ("*.shop.a.com", "https://eshop.a.com", None),
        (
            "*.x.shop.a.com",
            "https://y.x.shop.a.com",
            Some(Subdomains(4)),
        ),
        ("*", "https://a.com", Some(AllSites)),
        ("*", "http://127.0.0.1:9", Some(AllSites)),
        ("*", "https://[::1]", Some(AllSites)),
        ("http://127.0.0.1:9", "http://127.0.0.1:9", Some(Exact)),
        ("http://127.0.0.1:9", "http://127.0.0.1:10", None),
    ];
    for (rule, subject, expected) in table {
        assert_eq!(
            pattern(rule).matches(&origin(subject)),
            *expected,
            "{rule} against {subject}"
        );
    }
}

#[test]
fn specificity_orders_exact_over_deeper_wildcards_over_sites_over_everything() {
    use Specificity::{AllSites, Origin, Site, Subdomains};
    let ascending = [AllSites, Site, Subdomains(3), Subdomains(4), Origin];
    for pair in ascending.windows(2) {
        assert!(pair[0] < pair[1], "{:?} < {:?}", pair[0], pair[1]);
    }
}

#[test]
fn patterns_sort_for_listing_all_sites_first_then_sites_then_wildcards_then_origins() {
    let mut patterns = [
        pattern("https://b.com"),
        pattern("*.shop.a.com"),
        pattern("site:z.com"),
        pattern("*"),
        pattern("https://a.com"),
        pattern("site:a.com"),
    ];
    patterns.sort();
    let texts: Vec<String> = patterns.iter().map(ToString::to_string).collect();
    assert_eq!(
        texts,
        [
            "*",
            "site:a.com",
            "site:z.com",
            "*.shop.a.com",
            "https://a.com",
            "https://b.com"
        ]
    );
}
