// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Images per site: the key of a site, and the set of sites whose images are off. The table of
//! keys and entries below is repeated in `crates/mercury-settings/tests/site_images.rs`, because
//! the two crates normalise a host by the same rule without sharing code.

use talaria_core::{MAX_IMAGE_BLOCKED_SITES, SiteImages, SiteImagesError, image_site_key};
use url::Url;

fn url(text: &str) -> Url {
    Url::parse(text).expect("a valid URL")
}

fn hosts(set: &SiteImages) -> Vec<&str> {
    set.hosts().collect()
}

fn set_of(entries: &[&str]) -> SiteImages {
    SiteImages::from_hosts(entries.iter().map(|entry| (*entry).to_owned()))
}

/// (page address, key). `None` means the page has no site and images cannot be switched there.
const KEYS: &[(&str, Option<&str>)] = &[
    ("https://example.com/", Some("example.com")),
    ("https://Example.COM/Path?q=1#frag", Some("example.com")),
    ("http://example.com/", Some("example.com")),
    ("http://example.com:8080/x", Some("example.com")),
    ("https://example.com:443/", Some("example.com")),
    ("https://example.com./", Some("example.com")),
    ("https://user:secret@example.com/", Some("example.com")),
    ("https://www.example.com/", Some("www.example.com")),
    (
        "https://xn--bcher-kva.example/",
        Some("xn--bcher-kva.example"),
    ),
    (
        "https://b\u{fc}cher.example/",
        Some("xn--bcher-kva.example"),
    ),
    (
        "https://B\u{dc}CHER.example/",
        Some("xn--bcher-kva.example"),
    ),
    ("https://[::1]:8080/", Some("[::1]")),
    ("https://[0:0:0:0:0:0:0:1]/", Some("[::1]")),
    ("http://[2001:DB8::1]/", Some("[2001:db8::1]")),
    ("http://127.0.0.1/", Some("127.0.0.1")),
    ("http://127.1/", Some("127.0.0.1")),
    ("http://0x7f.0.0.1:3000/", Some("127.0.0.1")),
    ("http://localhost/", Some("localhost")),
    ("http://localhost:3000/", Some("localhost")),
    ("file:///etc/hosts", None),
    ("about:blank", None),
    ("data:text/plain,hello", None),
    ("view-source:https://example.com/", None),
    ("mailto:someone@example.com", None),
    ("ftp://example.com/", None),
    ("blob:https://example.com/0000", None),
    ("javascript:void(0)", None),
];

/// (stored entry, key it becomes). `None` means the entry is dropped.
const ENTRIES: &[(&str, Option<&str>)] = &[
    ("example.com", Some("example.com")),
    ("Example.COM", Some("example.com")),
    ("example.com.", Some("example.com")),
    ("www.example.com", Some("www.example.com")),
    ("xn--bcher-kva.example", Some("xn--bcher-kva.example")),
    ("b\u{fc}cher.example", Some("xn--bcher-kva.example")),
    ("[::1]", Some("[::1]")),
    ("[0:0:0:0:0:0:0:1]", Some("[::1]")),
    ("[2001:DB8::1]", Some("[2001:db8::1]")),
    ("127.0.0.1", Some("127.0.0.1")),
    ("127.1", Some("127.0.0.1")),
    ("localhost", Some("localhost")),
    ("", None),
    (" ", None),
    (" example.com", None),
    ("example.com ", None),
    ("exa mple.com", None),
    ("exa\tmple.com", None),
    ("exa\nmple.com", None),
    ("exa\u{0}mple.com", None),
    ("https://example.com", None),
    ("https://example.com/", None),
    ("//example.com", None),
    ("example.com/", None),
    ("example.com/path", None),
    ("example.com\\path", None),
    ("example.com?x=1", None),
    ("example.com#frag", None),
    ("user@example.com", None),
    ("user:pw@example.com", None),
    ("example.com:80", None),
    ("example.com:8080", None),
    ("[::1]:8080", None),
    ("%65xample.com", None),
    ("example%2Ecom", None),
    ("[fe80::1%25eth0]", None),
    ("[", None),
    ("]", None),
    ("[]", None),
    ("[zzz]", None),
    ("[::1", None),
    ("::1", None),
    (".", None),
    ("..", None),
];

#[test]
fn the_key_of_a_page_follows_the_table() {
    for (page, expected) in KEYS {
        assert_eq!(
            image_site_key(&url(page)).as_deref(),
            *expected,
            "the key of {page}"
        );
    }
}

#[test]
fn a_stored_entry_is_normalised_like_a_page_host_or_dropped() {
    for (entry, expected) in ENTRIES {
        let set = set_of(&[entry]);
        assert_eq!(
            hosts(&set),
            expected.iter().copied().collect::<Vec<_>>(),
            "the entry {entry:?}"
        );
    }
}

#[test]
fn every_key_of_a_page_is_its_own_stored_entry() {
    for (page, expected) in KEYS {
        if let Some(key) = expected {
            let set = set_of(&[key]);
            assert_eq!(hosts(&set), [*key], "the key {key} of {page}");
            assert!(set.blocks(&url(page)), "{page} is blocked by its own key");
        }
    }
}

#[test]
fn a_host_over_253_bytes_has_no_key() {
    let label = "a".repeat(60);
    let long = format!("{label}.{label}.{label}.{label}.{label}.example.com");
    assert!(long.len() > 253);
    assert_eq!(image_site_key(&url(&format!("https://{long}/"))), None);
    assert!(set_of(&[&long]).is_empty());

    let fits = format!("{label}.{label}.{label}.{label}.com");
    assert!(fits.len() <= 253);
    assert_eq!(
        image_site_key(&url(&format!("https://{fits}/"))).as_deref(),
        Some(fits.as_str())
    );
}

#[test]
fn only_an_image_of_a_blocked_page_is_refused() {
    let set = set_of(&["example.com"]);
    let blocked = url("https://example.com/article");

    assert!(set.blocks_image_load(Some(&blocked), true));
    // Not an image.
    assert!(!set.blocks_image_load(Some(&blocked), false));
    // No page yet.
    assert!(!set.blocks_image_load(None, true));
    assert!(!set.blocks_image_load(None, false));
    // Another host.
    assert!(!set.blocks_image_load(Some(&url("https://example.org/")), true));
    // The same host on another port, and on http: one site.
    assert!(set.blocks_image_load(Some(&url("http://example.com:8080/")), true));
    assert!(set.blocks_image_load(Some(&url("http://example.com/")), true));
    // A subdomain, and the parent of a subdomain, are other sites.
    assert!(!set.blocks_image_load(Some(&url("https://www.example.com/")), true));
    assert!(!set.blocks_image_load(Some(&url("https://sub.example.com/")), true));
    let sub = set_of(&["mail.example.com"]);
    assert!(!sub.blocks_image_load(Some(&url("https://example.com/")), true));
    assert!(!sub.blocks_image_load(Some(&url("https://maps.example.com/")), true));
    assert!(sub.blocks_image_load(Some(&url("https://mail.example.com/inbox")), true));
    // A page that is not a web page never matches.
    assert!(!set.blocks_image_load(Some(&url("about:blank")), true));
    assert!(!set.blocks_image_load(Some(&url("file:///tmp/example.com")), true));
    assert!(!set.blocks_image_load(Some(&url("view-source:https://example.com/")), true));
}

#[test]
fn an_empty_set_blocks_nothing() {
    let set = SiteImages::default();
    assert!(set.is_empty());
    assert!(!set.blocks_image_load(Some(&url("https://example.com/")), true));
    assert!(!set.blocks(&url("https://example.com/")));
}

#[test]
fn blocks_looks_at_the_site_not_the_address() {
    let set = set_of(&["example.com", "[::1]"]);
    assert!(set.blocks(&url("https://EXAMPLE.com:9000/a?b#c")));
    assert!(set.blocks(&url("https://example.com./")));
    assert!(set.blocks(&url("http://[0:0:0:0:0:0:0:1]:8080/")));
    assert!(!set.blocks(&url("https://notexample.com/")));
    assert!(!set.blocks(&url("https://example.com.evil.test/")));
    assert!(!set.blocks(&url("https://evil.test/example.com")));
    assert!(!set.blocks(&url("https://evil.test/?u=example.com")));
    assert!(!set.blocks(&url("https://example.com@evil.test/")));
}

#[test]
fn set_blocked_adds_removes_and_is_idempotent() {
    let mut set = SiteImages::default();
    let page = url("https://Example.com:8443/a");

    assert_eq!(set.set_blocked(&page, true), Ok(true));
    assert_eq!(hosts(&set), ["example.com"]);
    assert!(set.blocks(&page));
    assert!(set.blocks(&url("http://example.com/")));

    // The same again, and the same site written another way.
    assert_eq!(set.set_blocked(&page, true), Ok(false));
    assert_eq!(
        set.set_blocked(&url("http://example.com./"), true),
        Ok(false)
    );
    assert_eq!(hosts(&set), ["example.com"]);

    // Another site is another entry.
    assert_eq!(
        set.set_blocked(&url("https://www.example.com/"), true),
        Ok(true)
    );
    assert_eq!(hosts(&set), ["example.com", "www.example.com"]);

    assert_eq!(set.set_blocked(&page, false), Ok(true));
    assert_eq!(hosts(&set), ["www.example.com"]);
    assert_eq!(set.set_blocked(&page, false), Ok(false));
    assert!(!set.blocks(&page));
}

#[test]
fn the_keys_are_kept_sorted() {
    let mut set = SiteImages::default();
    for host in ["c.test", "a.test", "b.test", "[::1]", "127.0.0.1"] {
        assert_eq!(
            set.set_blocked(&url(&format!("https://{host}/")), true),
            Ok(true)
        );
    }
    let listed = hosts(&set);
    let mut sorted = listed.clone();
    sorted.sort_unstable();
    assert_eq!(listed, sorted);
}

#[test]
fn a_page_that_is_not_a_web_page_is_refused_and_changes_nothing() {
    let mut set = set_of(&["example.com"]);
    let before = set.clone();
    for page in [
        "file:///etc/hosts",
        "about:blank",
        "data:text/plain,hello",
        "view-source:https://example.com/",
        "ftp://example.com/",
    ] {
        assert_eq!(
            set.set_blocked(&url(page), true),
            Err(SiteImagesError::NotAWebPage),
            "{page}"
        );
        assert_eq!(
            set.set_blocked(&url(page), false),
            Err(SiteImagesError::NotAWebPage),
            "{page}"
        );
    }
    assert_eq!(set, before);
}

#[test]
fn the_thousand_and_first_site_is_refused() {
    let mut set = SiteImages::default();
    for number in 0..MAX_IMAGE_BLOCKED_SITES {
        let page = url(&format!("https://site{number}.test/"));
        assert_eq!(set.set_blocked(&page, true), Ok(true), "site {number}");
    }
    assert_eq!(set.hosts().count(), MAX_IMAGE_BLOCKED_SITES);

    let extra = url("https://one-too-many.test/");
    assert_eq!(set.set_blocked(&extra, true), Err(SiteImagesError::TooMany));
    assert!(!set.blocks(&extra));
    assert_eq!(set.hosts().count(), MAX_IMAGE_BLOCKED_SITES);

    // A site already held is not "one more", and removing is always allowed.
    assert_eq!(
        set.set_blocked(&url("https://site7.test/"), true),
        Ok(false)
    );
    assert_eq!(
        set.set_blocked(&url("https://site7.test/"), false),
        Ok(true)
    );
    assert_eq!(set.set_blocked(&extra, true), Ok(true));
    assert_eq!(set.hosts().count(), MAX_IMAGE_BLOCKED_SITES);
}

#[test]
fn from_hosts_drops_duplicates_and_keeps_order_free_results() {
    let set = set_of(&["b.test", "A.TEST", "b.test.", "a.test", "[::1]", "[0::1]"]);
    assert_eq!(hosts(&set), ["[::1]", "a.test", "b.test"]);
    assert_eq!(set, set_of(&["a.test", "[::1]", "b.test"]));
}

#[test]
fn from_hosts_keeps_the_first_thousand_valid_entries() {
    let entries = (0..5000).map(|number| format!("site{number}.test"));
    let set = SiteImages::from_hosts(entries);
    assert_eq!(set.hosts().count(), MAX_IMAGE_BLOCKED_SITES);
    assert!(set.blocks(&url("https://site0.test/")));
    assert!(set.blocks(&url("https://site999.test/")));
    assert!(!set.blocks(&url("https://site1000.test/")));

    // Invalid entries and repeats do not use up the room.
    let mixed = (0..3000).flat_map(|number| [format!("site{number}.test"), "no good".to_owned()]);
    let mixed = SiteImages::from_hosts(mixed);
    assert_eq!(mixed.hosts().count(), MAX_IMAGE_BLOCKED_SITES);
    let twice =
        (0..600).flat_map(|number| [format!("site{number}.test"), format!("SITE{number}.test")]);
    assert_eq!(SiteImages::from_hosts(twice).hosts().count(), 600);
}

#[test]
fn from_hosts_survives_hostile_entries() {
    let giant = "a".repeat(10_000);
    let deep = format!("{}.test", "x.".repeat(3000));
    let hostile = [
        giant.clone(),
        deep,
        "\u{0}".repeat(500),
        "\u{202e}example.com".to_owned(),
        "exa\u{200b}mple.com\u{0}".to_owned(),
        "javascript:alert(1)".to_owned(),
        "[::1]/../../etc".to_owned(),
        "good.test".to_owned(),
    ];
    let set = SiteImages::from_hosts(hostile);
    assert!(set.blocks(&url("https://good.test/")));
    for host in set.hosts() {
        assert!(host.len() <= 253, "{host}");
        assert!(host.is_ascii(), "{host}");
    }
    assert!(!set.blocks(&url(&format!("https://{giant}/"))));
}

#[test]
fn the_set_compares_by_its_keys() {
    assert_eq!(SiteImages::default(), SiteImages::from_hosts(Vec::new()));
    assert_ne!(SiteImages::default(), set_of(&["example.com"]));
    assert_eq!(set_of(&["b.test", "a.test"]), set_of(&["a.test", "b.test"]));
}
