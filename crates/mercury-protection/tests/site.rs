// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `Site`: the key per-site protections are filed under.

use mercury_protection::Site;
use url::Url;

fn of(url: &str) -> Option<String> {
    Site::of_url(&Url::parse(url).expect("a URL")).map(|site| site.as_str().to_owned())
}

#[test]
fn a_site_is_the_registrable_domain_in_lower_case() {
    let cases = [
        ("https://www.Example.co.uk/x", "example.co.uk"),
        ("https://example.com", "example.com"),
        ("https://a.b.example.com:8443/p?q=1", "example.com"),
        // A private suffix from the public suffix list: each user of github.io is a site.
        ("https://a.b.github.io/", "b.github.io"),
        ("http://EXAMPLE.org./", "example.org"),
        ("https://xn--bcher-kva.example/", "xn--bcher-kva.example"),
    ];
    for (url, site) in cases {
        assert_eq!(of(url).as_deref(), Some(site), "{url}");
    }
}

#[test]
fn an_address_or_a_single_label_host_is_its_own_site() {
    assert_eq!(of("http://127.0.0.1:8080/").as_deref(), Some("127.0.0.1"));
    assert_eq!(of("http://[::1]/").as_deref(), Some("[::1]"));
    assert_eq!(of("http://localhost/").as_deref(), Some("localhost"));
    assert_eq!(of("http://intranet/").as_deref(), Some("intranet"));
}

#[test]
fn a_url_with_no_host_has_no_site() {
    for url in ["data:text/plain,x", "about:blank", "file:///etc/hosts", "mailto:a@b.example"] {
        assert_eq!(of(url), None, "{url}");
    }
}

#[test]
fn the_engines_key_is_trimmed_and_lower_cased() {
    assert_eq!(Site::from_key("  Example.COM ").unwrap().as_str(), "example.com");
    assert_eq!(Site::from_key(""), None);
    assert_eq!(Site::from_key("   "), None);
    assert_eq!(Site::from_key(&"a".repeat(254)), None);
    assert!(Site::from_key(&"a".repeat(253)).is_some());
}

#[test]
fn a_host_written_as_text_gives_the_same_site_as_the_url() {
    assert_eq!(Site::of_host("::1").unwrap().as_str(), "[::1]");
    assert_eq!(Site::of_host("[::1]").unwrap().as_str(), "[::1]");
    assert_eq!(Site::of_host("WWW.Example.com").unwrap().as_str(), "example.com");
    assert_eq!(Site::of_host("10.0.0.1").unwrap().as_str(), "10.0.0.1");
    assert_eq!(Site::of_host("not a host"), None);
    assert_eq!(Site::of_host(""), None);
}

#[test]
fn a_key_from_the_engine_equals_the_one_computed_here() {
    let from_url = Site::of_url(&Url::parse("https://news.example.co.uk/").unwrap());
    assert_eq!(from_url, Site::from_key("example.co.uk"));
}
