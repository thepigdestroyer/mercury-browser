// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Match patterns: the MDN and Chrome documentation tables as facts, then `covers`, `overlaps`,
//! sets, limits and a never-panics loop.

use mercury_extensions::match_pattern::{
    HostPattern, MAX_PATTERN_LEN, MAX_PATTERNS_PER_SET, MatchPattern, MatchPatternError,
    MatchPatternSet, Scheme,
};
use url::Url;

fn p(s: &str) -> MatchPattern {
    MatchPattern::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
}

fn u(s: &str) -> Url {
    Url::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
}

fn check(pattern: &str, matching: &[&str], not_matching: &[&str]) {
    let pat = p(pattern);
    for url in matching {
        assert!(pat.matches(&u(url)), "{pattern} should match {url}");
    }
    for url in not_matching {
        assert!(!pat.matches(&u(url)), "{pattern} should not match {url}");
    }
}

// ---- the MDN table -----------------------------------------------------------------------

#[test]
fn all_urls() {
    check(
        "<all_urls>",
        &[
            "http://example.org/",
            "https://a.org/some/path/",
            "ws://sockets.somewhere.org/",
            "wss://ws.example.com/stuff/",
            "ftp://files.somewhere.org/",
            "file:///etc/hosts",
        ],
        &[
            "resource://a/b/c/",
            "ftps://files.somewhere.org/",
            "about:blank",
            "chrome://x/",
            "data:text/plain,hi",
        ],
    );
}

#[test]
fn any_web_scheme_any_host() {
    check(
        "*://*/*",
        &[
            "http://example.org/",
            "https://a.org/some/path/",
            "https://a.org/?q=1",
        ],
        // Mercury's reading: `*` is http and https only (MDN: "in some browsers also ws and wss").
        &[
            "ws://sockets.somewhere.org/",
            "wss://ws.example.com/stuff/",
            "ftp://ftp.example.org/",
            "file:///a/",
        ],
    );
}

#[test]
fn subdomain_wildcard() {
    check(
        "*://*.mozilla.org/*",
        &[
            "http://mozilla.org/",
            "https://mozilla.org/",
            "http://a.mozilla.org/",
            "http://a.b.mozilla.org/",
            "https://b.mozilla.org/path/",
        ],
        &[
            "ftp://mozilla.org/",
            "http://mozilla.com/",
            "http://firefox.org/",
            "http://notmozilla.org/",
            "http://mozilla.org.evil.com/",
        ],
    );
}

#[test]
fn exact_host_and_exact_path() {
    check(
        "*://mozilla.org/",
        &["http://mozilla.org/", "https://mozilla.org/"],
        &[
            "ftp://mozilla.org/",
            "http://a.mozilla.org/",
            "http://mozilla.org/a",
            "http://mozilla.org/?x",
        ],
    );
}

#[test]
fn ports_follow_chrome() {
    check(
        "https://mozilla.org:8080/",
        &["https://mozilla.org:8080/"],
        &[
            "http://a.mozilla.org/",
            "http://mozilla.org:8081",
            "https://mozilla.org/",
            "https://mozilla.org:8081/",
        ],
    );
    check(
        "https://mozilla.org/*",
        &[
            "https://mozilla.org/",
            "https://mozilla.org:8443/x",
            "https://mozilla.org:443/",
        ],
        &[],
    );
    check(
        "http://localhost/*",
        &["http://localhost:3000/a", "http://localhost/"],
        &["http://localhost.evil.com/"],
    );
    check(
        "https://a.com:*/x",
        &["https://a.com:1234/x", "https://a.com/x"],
        &[],
    );
    check(
        "http://127.0.0.1/*",
        &["http://127.0.0.1/", "http://127.0.0.1:8080/foo/bar.html"],
        &["http://127.0.0.2/"],
    );
}

#[test]
fn ftp_pattern() {
    check(
        "ftp://mozilla.org/",
        &["ftp://mozilla.org/", "ftp://mozilla.org"],
        &[
            "http://mozilla.org/",
            "ftp://sub.mozilla.org/",
            "ftp://mozilla.org/path",
        ],
    );
}

#[test]
fn path_only_matches_exactly() {
    check(
        "https://*/path",
        &[
            "https://mozilla.org/path",
            "https://a.mozilla.org/path",
            "https://something.com/path",
        ],
        &[
            "http://mozilla.org/path",
            "https://mozilla.org/path/",
            "https://mozilla.org/a",
            "https://mozilla.org/",
            "https://mozilla.org/path?foo=1",
        ],
    );
    check(
        "https://*/path/",
        &["https://mozilla.org/path/", "https://a.mozilla.org/path/"],
        &[
            "http://mozilla.org/path/",
            "https://mozilla.org/path",
            "https://mozilla.org/path/?foo=1",
        ],
    );
}

#[test]
fn any_path_includes_the_query() {
    check(
        "https://mozilla.org/*",
        &[
            "https://mozilla.org/",
            "https://mozilla.org/path",
            "https://mozilla.org/path/to/doc?foo=1",
        ],
        &["http://mozilla.org/path", "https://mozilla.com/path"],
    );
}

#[test]
fn the_fragment_is_ignored() {
    check(
        "https://mozilla.org/a/b/c/",
        &[
            "https://mozilla.org/a/b/c/",
            "https://mozilla.org/a/b/c/#section1",
        ],
        &["https://mozilla.org/a/b/c", "https://mozilla.org/a/b/c/d/"],
    );
}

#[test]
fn star_in_the_middle_of_the_path() {
    check(
        "https://mozilla.org/*/b/*/",
        &[
            "https://mozilla.org/a/b/c/",
            "https://mozilla.org/d/b/f/",
            "https://mozilla.org/a/b/c/d/",
            "https://mozilla.org/a/b/c/d/#section1",
            "https://mozilla.org/a/b/c/d/?foo=/",
            "https://mozilla.org/a?foo=21314&bar=/b/&extra=c/",
        ],
        &[
            "https://mozilla.org/b/*/",
            "https://mozilla.org/a/b/",
            "https://mozilla.org/a/b/c/d/?foo=bar",
        ],
    );
}

#[test]
fn file_patterns() {
    check(
        "file:///blah/*",
        &["file:///blah/", "file:///blah/bleh"],
        &["file:///bleh/", "https://blah/blah/"],
    );
    check(
        "file:///foo*",
        &["file:///foo/bar.html", "file:///foo"],
        &["file:///bar"],
    );
}

#[test]
fn chrome_documentation_examples() {
    check(
        "https://*/foo*",
        &[
            "https://example.com/foo/bar.html",
            "https://www.google.com/foo",
        ],
        &["http://example.com/foo", "https://example.com/bar"],
    );
    check(
        "https://*.google.com/foo*bar",
        &[
            "https://www.google.com/foo/baz/bar",
            "https://docs.google.com/foobar",
        ],
        &[
            "https://www.google.com/foo/baz/bax",
            "https://google.org/foobar",
        ],
    );
    check(
        "*://mail.google.com/*",
        &["http://mail.google.com/", "https://mail.google.com/inbox"],
        &["https://smail.google.com/", "ftp://mail.google.com/"],
    );
    check(
        "https://*/",
        &["https://anything.example/"],
        &["https://anything.example/x"],
    );
}

// ---- invalid patterns --------------------------------------------------------------------

#[test]
fn invalid_patterns_are_refused_with_the_right_error() {
    use MatchPatternError as E;
    let table: [(&str, MatchPatternError); 21] = [
        ("", E::Empty),
        ("resource://path/", E::UnsupportedScheme),
        ("chrome://settings/*", E::UnsupportedScheme),
        ("http*://mozilla.org/", E::UnsupportedScheme),
        ("data://x/", E::UnsupportedScheme),
        ("https://mozilla.org", E::MissingPath),
        ("*://*", E::MissingPath),
        ("file://*", E::InvalidHost),
        ("https://mozilla.*.org/", E::BadHostWildcard),
        ("https://*zilla.org/", E::BadHostWildcard),
        ("https://*.*.org/", E::BadHostWildcard),
        ("https://*./", E::MissingHost),
        ("https:///path", E::MissingHost),
        ("https://exa mple.com/", E::InvalidHost),
        ("https://user@example.com/", E::InvalidHost),
        ("https://*.1.2.3.4/", E::InvalidHost),
        ("https://example.com:/", E::InvalidPort),
        ("https://example.com:0/", E::InvalidPort),
        ("https://example.com:65536/", E::InvalidPort),
        ("https://example.com:80a/", E::InvalidPort),
        ("example.com/*", E::MissingScheme),
    ];
    for (pattern, expected) in table {
        assert_eq!(MatchPattern::parse(pattern), Err(expected), "{pattern:?}");
    }
}

#[test]
fn control_characters_are_refused() {
    for bad in [
        "https://a.com/\u{0}",
        "https://a.com/\n",
        "https://a\t.com/*",
        "https://a.com/\u{7f}",
    ] {
        assert_eq!(
            MatchPattern::parse(bad),
            Err(MatchPatternError::ControlCharacter),
            "{bad:?}"
        );
    }
}

#[test]
fn pattern_length_is_capped_at_the_edge() {
    let prefix = "https://a.com/";
    let at = format!("{prefix}{}", "x".repeat(MAX_PATTERN_LEN - prefix.len()));
    assert_eq!(at.len(), MAX_PATTERN_LEN);
    assert!(MatchPattern::parse(&at).is_ok());
    let over = format!("{at}x");
    assert_eq!(MatchPattern::parse(&over), Err(MatchPatternError::TooLong));
}

#[test]
fn a_hash_in_the_pattern_never_matches() {
    let pat = p("https://a.com/x#y");
    assert!(!pat.matches(&u("https://a.com/x")));
    assert!(!pat.matches(&u("https://a.com/x#y")));
}

#[test]
fn hosts_are_case_folded_and_idn_normalised() {
    let pat = p("https://EXAMPLE.com/*");
    assert!(pat.matches(&u("https://example.COM/")));
    let idn = p("https://*.bücher.example/*");
    assert!(idn.matches(&u("https://shop.xn--bcher-kva.example/")));
    assert!(idn.matches(&u("https://bücher.example/")));
}

#[test]
fn non_ascii_paths_line_up_with_url_encoding() {
    let pat = p("https://a.com/über/*");
    assert!(pat.matches(&u("https://a.com/über/x")));
    let spaced = p("https://a.com/a b");
    assert!(spaced.matches(&u("https://a.com/a%20b")));
}

#[test]
fn ipv6_hosts_work() {
    let pat = p("http://[::1]/*");
    assert!(pat.matches(&u("http://[::1]:8080/x")));
    assert!(!pat.matches(&u("http://[::2]/x")));
    assert!(MatchPattern::parse("http://[::1/*").is_err());
}

#[test]
fn data_and_urn_patterns() {
    let d = p("data:text/plain,*");
    assert!(d.matches(&u("data:text/plain,hello")));
    assert!(!d.matches(&u("data:text/html,hello")));
    assert!(p("urn:*").matches(&u("urn:uuid:6e8bc430-9c3a-11d9-9669-0800200c9a66")));
    assert!(!p("<all_urls>").matches(&u("data:text/plain,hello")));
}

#[test]
fn matching_never_panics_on_odd_urls() {
    let pat = p("*://*.example.com/*");
    for url in [
        "http://example.com:99999/",
        "http://[::1]/",
        "about:blank",
        "javascript:alert(1)",
        "mailto:a@b.c",
        "http://a/%zz",
    ] {
        if let Ok(url) = Url::parse(url) {
            let _ = pat.matches(&url);
        }
    }
}

// ---- accessors ---------------------------------------------------------------------------

#[test]
fn accessors_describe_the_pattern() {
    let a = p("https://*.example.com:8443/x*");
    assert_eq!(a.schemes(), vec![Scheme::Https]);
    assert_eq!(a.port(), Some(8443));
    assert_eq!(a.path(), "/x*");
    assert_eq!(a.display_host().as_deref(), Some("example.com"));
    assert!(matches!(a.host(), HostPattern::Subdomains(b) if b == "example.com"));
    assert_eq!(p("*://*/*").schemes(), vec![Scheme::Http, Scheme::Https]);
    assert_eq!(p("<all_urls>").schemes().len(), 6);
    assert!(p("<all_urls>").is_all_urls());
    assert!(!p("*://*/*").is_all_urls());
}

#[test]
fn broad_patterns_are_recognised() {
    for broad in [
        "<all_urls>",
        "*://*/*",
        "https://*/*",
        "https://*/path",
        "*://*.com/*",
    ] {
        assert!(p(broad).is_broad(), "{broad}");
    }
    for narrow in [
        "https://example.com/*",
        "https://*.example.com/*",
        "ftp://*/*",
        "ws://*/*",
        "file:///*",
    ] {
        assert!(!p(narrow).is_broad(), "{narrow}");
    }
}

#[test]
fn with_any_path_drops_the_path_only() {
    assert_eq!(
        p("https://a.com/x/y").with_any_path().to_string(),
        "https://a.com/*"
    );
    assert_eq!(
        p("https://a.com:81/x").with_any_path().to_string(),
        "https://a.com:81/*"
    );
    assert_eq!(p("file:///x").with_any_path().to_string(), "file:///x");
    assert_eq!(p("data:abc").with_any_path().to_string(), "data:abc");
}

// ---- covers and overlaps -----------------------------------------------------------------

#[test]
fn covers_table() {
    let yes: [(&str, &str); 16] = [
        ("<all_urls>", "https://a.com/x"),
        ("<all_urls>", "*://*/*"),
        ("<all_urls>", "ftp://a.com/*"),
        ("<all_urls>", "file:///x"),
        ("*://*/*", "https://a.com/*"),
        ("*://*/*", "http://*.a.com/x?y*"),
        ("https://*.a.com/*", "https://b.a.com/*"),
        ("https://*.a.com/*", "https://*.b.a.com/*"),
        ("https://*.a.com/*", "https://a.com/x"),
        ("https://a.com/*", "https://a.com:8080/*"),
        ("https://a.com/x*", "https://a.com/xy*"),
        ("https://a.com/x*", "https://a.com/x"),
        ("https://a.com/*", "https://a.com/*"),
        ("https://a.com/x", "https://a.com/x"),
        ("https://a.com:80/*", "https://a.com:80/y"),
        ("file:///*", "file:///a/b"),
    ];
    for (outer, inner) in yes {
        assert!(p(outer).covers(&p(inner)), "{outer} should cover {inner}");
    }
    let no: [(&str, &str); 15] = [
        ("https://a.com/*", "*://*/*"),
        ("https://a.com/*", "http://a.com/*"),
        ("https://a.com/*", "https://b.com/*"),
        ("https://a.com/*", "https://*.a.com/*"),
        ("https://*.a.com/*", "https://*/*"),
        ("https://*.a.com/*", "https://evila.com/*"),
        ("https://a.com:80/*", "https://a.com/*"),
        ("https://a.com/x", "https://a.com/x*"),
        ("https://a.com/x*", "https://a.com/*"),
        ("*://*/*", "ws://a.com/*"),
        ("*://*/*", "<all_urls>"),
        ("*://*/*", "file:///x"),
        ("file:///*", "https://a.com/*"),
        ("https://*/*", "file:///x"),
        ("data:a*", "data:*"),
    ];
    for (outer, inner) in no {
        assert!(
            !p(outer).covers(&p(inner)),
            "{outer} should not cover {inner}"
        );
    }
}

#[test]
fn covers_agrees_with_matches_on_a_url_grid() {
    // Soundness: whenever `a.covers(b)`, every URL that `b` matches is matched by `a`.
    let patterns: Vec<MatchPattern> = [
        "<all_urls>",
        "*://*/*",
        "https://*/*",
        "https://a.com/*",
        "https://*.a.com/*",
        "https://b.a.com/x*",
        "https://a.com:8080/*",
        "http://a.com/x",
        "https://a.com/x*y",
        "ftp://a.com/*",
        "file:///*",
        "https://*.com/*",
    ]
    .iter()
    .map(|s| p(s))
    .collect();
    let urls: Vec<Url> = [
        "https://a.com/",
        "https://a.com/x",
        "https://a.com/xy",
        "https://a.com/x?y",
        "https://b.a.com/x1",
        "https://c.b.a.com/",
        "https://a.com:8080/",
        "http://a.com/x",
        "ftp://a.com/",
        "file:///t",
        "https://z.org/",
        "https://evila.com/",
    ]
    .iter()
    .map(|s| u(s))
    .collect();
    for a in &patterns {
        for b in &patterns {
            if a.covers(b) {
                for url in &urls {
                    if b.matches(url) {
                        assert!(a.matches(url), "{a} covers {b} but misses {url}");
                    }
                }
            }
        }
    }
}

#[test]
fn overlaps_is_conservative() {
    let yes = [
        ("https://a.com/*", "https://*.a.com/*"),
        ("https://*.a.com/*", "https://*.b.a.com/*"),
        ("https://a.com/x*", "https://a.com/*y"),
        ("<all_urls>", "https://a.com/*"),
        ("https://a.com:80/*", "https://a.com/*"),
    ];
    for (a, b) in yes {
        assert!(p(a).overlaps(&p(b)), "{a} {b}");
        assert!(p(b).overlaps(&p(a)), "{b} {a}");
    }
    let no = [
        ("https://a.com/*", "http://a.com/*"),
        ("https://a.com/*", "https://b.com/*"),
        ("https://*.a.com/*", "https://*.b.com/*"),
        ("https://a.com:80/*", "https://a.com:81/*"),
        ("https://a.com/x", "https://a.com/y"),
        ("file:///*", "https://a.com/*"),
    ];
    for (a, b) in no {
        assert!(!p(a).overlaps(&p(b)), "{a} {b}");
    }
    assert!(p("https://*.a.com/*").overlaps_host(&p("ftp://b.a.com/z")));
    assert!(!p("https://*.a.com/*").overlaps_host(&p("ftp://a.org/z")));
}

// ---- sets --------------------------------------------------------------------------------

#[test]
fn sets_dedupe_and_sort() {
    let s = MatchPatternSet::parse_all(["https://b.com/*", "https://a.com/*", "https://b.com/*"])
        .unwrap();
    assert_eq!(s.len(), 2);
    let listed: Vec<String> = s.iter().map(ToString::to_string).collect();
    assert_eq!(listed, vec!["https://a.com/*", "https://b.com/*"]);
}

#[test]
fn set_operations() {
    let a = MatchPatternSet::parse_all(["https://*.a.com/*", "https://b.com/*"]).unwrap();
    let b = MatchPatternSet::parse_all(["https://x.a.com/*", "https://c.com/*"]).unwrap();
    assert!(a.covers(&p("https://x.a.com/p")));
    assert!(!a.covers(&p("https://c.com/*")));
    assert!(a.matches(&u("https://q.a.com/")));
    assert!(!a.matches(&u("https://c.com/")));
    let diff = b.difference(&a);
    assert_eq!(diff.len(), 1);
    assert!(diff.iter().any(|q| q.to_string() == "https://c.com/*"));
    let union = a.union(&b);
    assert_eq!(union.len(), 4);
    assert!(b.difference(&union).is_empty());
    assert!(
        MatchPatternSet::parse_all(["https://x.a.com/*"])
            .unwrap()
            .is_subset(&a)
    );
    assert!(!b.is_subset(&a));
    assert!(a.covers_set(&MatchPatternSet::new()));
    assert!(MatchPatternSet::new().is_subset(&MatchPatternSet::new()));
}

#[test]
fn set_insert_and_remove() {
    let mut s = MatchPatternSet::new();
    assert!(s.insert(p("https://a.com/*")));
    assert!(!s.insert(p("https://a.com/*")));
    s.insert(p("https://a.com/x"));
    s.remove_covered_by(&p("https://a.com/*"));
    assert!(s.is_empty());
    s.insert(p("https://a.com/*"));
    assert!(s.remove(&p("https://a.com/*")));
    assert!(!s.remove(&p("https://a.com/*")));
}

#[test]
fn set_flags() {
    assert!(
        MatchPatternSet::parse_all(["https://a.com/*", "<all_urls>"])
            .unwrap()
            .has_all_urls()
    );
    assert!(
        MatchPatternSet::parse_all(["https://*/*"])
            .unwrap()
            .has_broad()
    );
    assert!(
        !MatchPatternSet::parse_all(["https://a.com/*"])
            .unwrap()
            .has_broad()
    );
}

#[test]
fn set_size_is_capped_at_the_edge() {
    let at: Vec<String> = (0..MAX_PATTERNS_PER_SET)
        .map(|i| format!("https://h{i}.example/*"))
        .collect();
    assert_eq!(
        MatchPatternSet::parse_all(&at).unwrap().len(),
        MAX_PATTERNS_PER_SET
    );
    let mut over = at;
    over.push("https://one-more.example/*".to_owned());
    assert_eq!(
        MatchPatternSet::parse_all(&over),
        Err(MatchPatternError::TooManyPatterns)
    );
}

#[test]
fn a_bad_member_fails_the_whole_set() {
    assert!(MatchPatternSet::parse_all(["https://a.com/*", "bogus"]).is_err());
}

#[test]
fn serde_round_trips() {
    let s = MatchPatternSet::parse_all(["https://a.com/*", "<all_urls>"]).unwrap();
    let json = serde_json::to_string(&s).unwrap();
    let back: MatchPatternSet = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
    assert!(serde_json::from_str::<MatchPatternSet>(r#"["nope"]"#).is_err());
    assert!(serde_json::from_str::<MatchPattern>("\"https://a.com\"").is_err());
}

// ---- never panics ------------------------------------------------------------------------

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn mutated_patterns_never_panic() {
    let seeds = [
        "<all_urls>",
        "*://*.example.com/*",
        "https://example.com:8080/a*b",
        "file:///tmp/*",
        "data:text/*",
        "https://[::1]/*",
        "https://*/path?x=*",
    ];
    let alphabet: Vec<char> = "*:/.[]#?%@\\ aZ0é\u{0}\u{7f}<>_-".chars().collect();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let probe = u("https://www.example.com:8080/a-b?x=1");
    for round in 0..4000 {
        let seed = seeds[round % seeds.len()];
        let mut chars: Vec<char> = seed.chars().collect();
        for _ in 0..(1 + xorshift(&mut state) % 4) {
            let at = (xorshift(&mut state) as usize) % (chars.len() + 1);
            let pick = alphabet[(xorshift(&mut state) as usize) % alphabet.len()];
            match xorshift(&mut state) % 3 {
                0 => chars.insert(at, pick),
                1 if at < chars.len() => {
                    chars.remove(at);
                }
                _ if at < chars.len() => chars[at] = pick,
                _ => {}
            }
        }
        let text: String = chars.into_iter().collect();
        if let Ok(pat) = MatchPattern::parse(&text) {
            let _ = pat.matches(&probe);
            let _ = pat.covers(&p("https://a.com/*"));
            let _ = pat.to_string();
            let reparsed = MatchPattern::parse(&pat.to_string());
            assert_eq!(
                reparsed.as_ref(),
                Ok(&pat),
                "display of {text:?} must reparse to itself"
            );
        }
    }
}

#[test]
fn pathological_globs_stay_fast() {
    let pattern = format!("https://a.com/{}b", "a*".repeat(400));
    let pat = p(&pattern);
    let url = u(&format!("https://a.com/{}", "a".repeat(30_000)));
    assert!(!pat.matches(&url));
}
