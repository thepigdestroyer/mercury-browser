// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use mercury_site_security::{display_host, display_url};

/// The ASCII (punycode) form a URL parser gives `unicode`, which is what the engine reports.
fn ascii(unicode: &str) -> String {
    url::Url::parse(&format!("https://{unicode}/"))
        .unwrap_or_else(|error| panic!("{unicode} does not parse: {error}"))
        .host_str()
        .expect("a host")
        .to_owned()
}

fn shown_as_unicode(unicode: &str) {
    let host = ascii(unicode);
    assert!(host.contains("xn--"), "{unicode} should need punycode");
    assert_eq!(display_host(&host), unicode, "{unicode} ({host})");
}

fn shown_as_punycode(unicode: &str) {
    let host = ascii(unicode);
    assert_eq!(display_host(&host), host, "{unicode} ({host})");
}

#[test]
fn ascii_hosts_and_addresses_are_shown_unchanged() {
    for host in [
        "example.com",
        "127.0.0.1",
        "[::1]",
        "localhost",
        "a-b.example",
        "xn--",
    ] {
        assert_eq!(display_host(host), host);
    }
}

#[test]
fn single_script_names_are_shown_in_their_script() {
    shown_as_unicode("münchen.de");
    shown_as_unicode("bücher.example");
    shown_as_unicode("ελληνικά.gr");
    shown_as_unicode("яндекс.рф");
    shown_as_unicode("сайт.com");
    shown_as_unicode("ไทย.com");
    shown_as_unicode("例え.jp");
}

#[test]
fn the_east_asian_combinations_with_latin_are_allowed() {
    // Japanese: Han, Hiragana, Katakana and Latin.
    shown_as_unicode("日本のカメラabc.jp");
    // Korean: Han, Hangul and Latin.
    shown_as_unicode("한국abc.kr");
    // Chinese: Han, Bopomofo and Latin.
    shown_as_unicode("中文ㄅabc.tw");
}

#[test]
fn latin_with_one_other_script_is_allowed_unless_cyrillic_or_greek() {
    shown_as_unicode("abcไทย.com");
    shown_as_punycode("pаypal.com"); // Cyrillic а
    shown_as_punycode("pαypal.com"); // Greek α
}

#[test]
fn other_mixtures_are_shown_as_punycode() {
    shown_as_punycode("한カ.com"); // Hangul with Katakana
    shown_as_punycode("ไทยабв.com"); // Thai with Cyrillic
    shown_as_punycode("abcไทยລາວ.com"); // Latin with two other scripts
}

#[test]
fn whole_script_look_alikes_of_latin_are_shown_as_punycode() {
    // "apple" in Cyrillic letters.
    shown_as_punycode("аррӏе.com");
    shown_as_punycode("ехосо.com");
    // Greek letters that read as "ovo".
    shown_as_punycode("ονο.com");
    // Under a top-level domain in the same script the name is that script's own.
    shown_as_unicode("сосо.рф");
}

#[test]
fn combining_marks_on_latin_letters_are_shown_as_punycode() {
    // q with a combining dot above has no precomposed form, so it survives normalisation.
    shown_as_punycode("q\u{0307}uick.com");
}

#[test]
fn scripts_that_are_not_recommended_for_identifiers_are_shown_as_punycode() {
    shown_as_punycode("ᏣᎳᎩ.com"); // Cherokee
}

#[test]
fn symbols_and_punctuation_in_a_name_are_shown_as_punycode() {
    shown_as_punycode("a\u{00b7}b.com"); // middle dot
    shown_as_punycode("ab\u{2665}.com"); // heart
}

#[test]
fn one_unsafe_label_shows_the_whole_host_as_punycode() {
    shown_as_punycode("münchen.аррӏе.com");
}

#[test]
fn undecodable_punycode_is_shown_as_it_is() {
    assert_eq!(display_host("xn--zz.com"), "xn--zz.com");
}

#[test]
fn display_url_replaces_only_the_host() {
    let url = format!("https://{}:8443/a%20b?q=1#f", ascii("münchen.de"));
    assert_eq!(display_url(&url), "https://münchen.de:8443/a%20b?q=1#f");
    let spoof = format!("https://{}/login", ascii("аррӏе.com"));
    assert_eq!(display_url(&spoof), spoof);
    assert_eq!(display_url("about:blank"), "about:blank");
    assert_eq!(display_url("not a url"), "not a url");
    assert_eq!(display_url(""), "");
}
