// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use mercury_site_security::{
    CERTIFICATE_EXCEPTION_SCHEME, ERROR_PAGE_PROBE_SCRIPT, ErrorPage, certificate_error_page,
    crash_page, network_error_page, parse_error_page_marker,
};

const SECRET: &str = "0123456789abcdef0123456789abcdef";

/// What the engine does with a template (Servo 0.8, `servoparser/mod.rs`).
fn fill(template: &str, reason: &str) -> String {
    template
        .replace("${reason}", reason)
        .replace("${bytes}", "QUJD")
        .replace("${secret}", "424242")
        .replace("${details}", reason)
}

#[test]
fn every_page_holds_its_reason_after_a_plaintext_element_at_the_very_end() {
    for (template, placeholder) in [
        (certificate_error_page(SECRET, false), "${reason}"),
        (network_error_page(SECRET), "${reason}"),
        (crash_page(), "${details}"),
    ] {
        let tail = format!("<plaintext hidden>{placeholder}");
        assert!(template.ends_with(&tail), "{template}");
        assert_eq!(template.matches(placeholder).count(), 1);
        // Hostile text stays text: nothing after <plaintext> is markup.
        let filled = fill(&template, "</plaintext><script>alert(1)</script>");
        let held = filled.rsplit_once("<plaintext hidden>").expect("held").1;
        assert_eq!(held, "</plaintext><script>alert(1)</script>");
    }
}

#[test]
fn pages_carry_a_marker_with_the_secret() {
    let certificate = certificate_error_page(SECRET, false);
    assert!(certificate.contains(&format!(
        "<meta name=\"mercury-error-page\" content=\"certificate {SECRET}\">"
    )));
    let network = network_error_page(SECRET);
    assert!(network.contains(&format!(
        "<meta name=\"mercury-error-page\" content=\"network {SECRET}\">"
    )));
    assert!(!crash_page().contains("mercury-error-page"));
}

#[test]
fn a_secret_cannot_break_out_of_its_attribute() {
    let page = network_error_page("ab\"><script>x</script>cd");
    assert!(page.contains("content=\"network abscriptxscriptcd\""));
}

#[test]
fn the_certificate_page_sends_the_certificate_to_the_exception_scheme() {
    let page = fill(&certificate_error_page(SECRET, false), "bad");
    assert!(page.contains("<meta name=\"mercury-certificate\" content=\"QUJD\">"));
    assert!(page.contains(&format!("fetch('{CERTIFICATE_EXCEPTION_SCHEME}:allow?'")));
    assert!(page.contains("if (false) { proceed.click(); }"));
    let automatic = certificate_error_page(SECRET, true);
    assert!(automatic.contains("if (true) { proceed.click(); }"));
    // Servo's own secret is never used.
    assert!(!certificate_error_page(SECRET, false).contains("${secret}"));
}

#[test]
fn markers_are_recognised_only_with_the_secret() {
    assert_eq!(
        parse_error_page_marker(&format!("network {SECRET}"), SECRET),
        Some(ErrorPage::Network)
    );
    assert_eq!(
        parse_error_page_marker(&format!("certificate {SECRET}"), SECRET),
        Some(ErrorPage::Certificate)
    );
    assert_eq!(parse_error_page_marker("network guess", SECRET), None);
    assert_eq!(
        parse_error_page_marker(&format!("other {SECRET}"), SECRET),
        None
    );
    assert_eq!(parse_error_page_marker("", SECRET), None);
    assert_eq!(parse_error_page_marker("network ", ""), None);
    assert!(ERROR_PAGE_PROBE_SCRIPT.contains("mercury-error-page"));
}

#[test]
fn continue_is_offered_only_in_the_top_level_page_and_for_local_names() {
    let page = certificate_error_page(SECRET, false);
    // Hidden until the script decides; a framed page (clickjacking) never shows it.
    assert!(page.contains("<div id=\"advanced\" hidden>"), "{page}");
    assert!(page.contains("window.top === window"), "{page}");
    assert!(page.contains("isLocalName(location.hostname)"), "{page}");
    // Automatic continuation (tests) obeys the same gate.
    let automatic = certificate_error_page(SECRET, true);
    let gate = automatic.find("if (!offered)").expect("the gate");
    let click = automatic.find("proceed.click()").expect("the click");
    assert!(gate < click);
}

/// The page's `isLocalName` mirrors `talaria_core::is_local_host`, which decides in the browser.
/// The table pins the edge cases; the page must give the same answers (a mismatch fails safe,
/// but then "Continue anyway" shows and does nothing).
#[test]
fn the_page_and_the_browser_agree_on_local_names() {
    let table = [
        ("localhost", true),
        ("app.localhost", true),
        ("printer.local.", true),
        ("nas", true),
        ("nas..", true),
        ("my-host.lan", true),
        ("router.home.arpa", true),
        ("home.arpa", false),
        ("intranet.example.com", false),
        ("127.8.9.10", true),
        ("0.0.0.0", true),
        ("0.1.2.3", false),
        ("0.0.0.10", false),
        ("192.168.1.1", true),
        ("192.169.1.1", false),
        ("169.255.0.1", false),
        ("255.255.255.255", false),
        ("10.0.0.1", true),
        ("172.16.0.1", true),
        ("172.32.0.1", false),
        ("100.64.0.1", true),
        ("100.128.0.1", false),
        ("169.254.1.1", true),
        ("8.8.8.8", false),
        ("[::1]", true),
        ("[::]", true),
        ("[fc00::1]", true),
        ("[fd12:3456::1]", true),
        ("[fc::1]", false),
        ("[fd00::]", true),
        ("[fe80::]", true),
        ("[fe80::1]", true),
        ("[febf::1]", true),
        ("[fe00::]", false),
        ("[::2]", false),
        ("[::ffff:a00:1]", false),
        ("[fec0::1]", false),
        ("[fe8::1]", false),
        ("[2001:db8::1]", false),
        ("example.com", false),
    ];
    for (host, local) in table {
        assert_eq!(talaria_core::is_local_host(host), local, "{host}");
    }
    let page = certificate_error_page(SECRET, false);
    // Every trailing dot is ignored, as `is_local_name` trims them.
    assert!(
        page.contains("host.toLowerCase().replace(/[.]+$/, '')"),
        "{page}"
    );
    // Of 0.0.0.0/8 only 0.0.0.0, as `Ipv4Addr::is_unspecified`.
    assert!(page.contains("host === '0.0.0.0'"), "{page}");
    assert!(!page.contains("a === 0 "), "{page}");
    // IPv6 ranges by the first group's value, not by text ("[fc::1]" is 00fc::1).
    assert!(page.contains("(first & 0xfe00) === 0xfc00"), "{page}");
    assert!(page.contains("(first & 0xffc0) === 0xfe80"), "{page}");
    assert!(!page.contains("/^f[cd]/"), "{page}");
}
