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
