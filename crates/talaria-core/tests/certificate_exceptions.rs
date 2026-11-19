// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Certificate exceptions: the one form of a host that the browser, the request parser and the
//! indicator share (`certificate_exception_host`), the policy that decides which hosts may get an
//! exception (`certificate_exception_allowed`), and the controller's lookups that use them.
//!
//! The engine keeps exceptions per (host, certificate) under its own spelling of the host
//! (`server_name_for_host` in the vendored `servo-net`, which also drops the brackets of an IPv6
//! literal). The `HOST_FORMS` table below is the table that crate's tests carry too.

use talaria_core::{
    BrowserController, BrowserEvent, SecurityIndicator, TabId, certificate_exception_allowed,
    certificate_exception_host, is_local_host,
};
use url::Url;

/// Host as given, host as the exceptions spell it (`None`: no exception can name it).
const HOST_FORMS: &[(&str, Option<&str>)] = &[
    // Case and one trailing dot (the form of a fully qualified name).
    ("nas.local", Some("nas.local")),
    ("Nas.Local", Some("nas.local")),
    ("NAS.LOCAL.", Some("nas.local")),
    ("nas", Some("nas")),
    ("localhost.", Some("localhost")),
    // More than one trailing dot is an empty label: no name.
    ("nas..", None),
    ("nas.local..", None),
    ("a..b", None),
    (".", None),
    ("..", None),
    // IPv4.
    ("127.0.0.1", Some("127.0.0.1")),
    ("192.168.1.20.", Some("192.168.1.20")),
    // IPv6 literals keep their brackets, as `Url::host_str` gives them, in its canonical form.
    ("[::1]", Some("[::1]")),
    ("[0:0:0:0:0:0:0:1]", Some("[::1]")),
    ("[FD00:0:0:0:0:0:0:ABC]", Some("[fd00::abc]")),
    ("[fe80::1]", Some("[fe80::1]")),
    // Anything that is not a host, or is half of one.
    ("", None),
    (" ", None),
    ("a b", None),
    ("a/b", None),
    ("a?b", None),
    ("a#b", None),
    ("user@host", None),
    ("host:8443", None),
    ("nas.local:8443", None),
    ("::1", None),
    ("::1]", None),
    ("[::1", None),
    ("[::1].", None),
    ("[::1]:8443", None),
    ("[not-an-address]", None),
    ("a%2fb", None),
];

#[test]
fn the_host_form_table() {
    for (host, expected) in HOST_FORMS {
        assert_eq!(
            certificate_exception_host(host).as_deref(),
            *expected,
            "{host:?}"
        );
    }
}

#[test]
fn the_host_form_is_what_a_url_gives() {
    // The controller compares the form of the host the user accepted with the form of the host
    // of the page, which is `Url::host_str`.
    for (address, expected) in [
        ("https://Nas.Local./", "nas.local"),
        ("https://nas.local:8443/", "nas.local"),
        ("https://[::1]:8443/", "[::1]"),
        ("https://[0:0:0:0:0:0:0:1]/", "[::1]"),
        ("https://127.0.0.1/", "127.0.0.1"),
        ("https://LOCALHOST/x?y#z", "localhost"),
    ] {
        let url = Url::parse(address).expect("an address");
        let host = url.host_str().expect("a host");
        assert_eq!(
            certificate_exception_host(host).as_deref(),
            Some(expected),
            "{address}"
        );
    }
    // A name with letters outside ASCII is the ASCII form the engine reports.
    let url = Url::parse("https://Nås.local/").expect("an address");
    let ascii = url.host_str().expect("a host");
    assert!(ascii.is_ascii() && ascii.starts_with("xn--"), "{ascii}");
    assert_eq!(
        certificate_exception_host("Nås.local").as_deref(),
        Some(ascii)
    );
    assert_eq!(certificate_exception_host(ascii).as_deref(), Some(ascii));
}

#[test]
fn the_host_form_is_stable_when_applied_again() {
    for (host, _) in HOST_FORMS {
        if let Some(form) = certificate_exception_host(host) {
            assert_eq!(
                certificate_exception_host(&form).as_deref(),
                Some(form.as_str()),
                "{host:?}"
            );
        }
    }
}

/// Which hosts may get an exception today (decision O1: names on this computer or the local
/// network only).
const POLICY: &[(&str, bool)] = &[
    // This computer.
    ("localhost", true),
    ("Localhost.", true),
    ("app.localhost", true),
    ("127.0.0.1", true),
    ("127.8.9.10", true),
    ("[::1]", true),
    ("[0:0:0:0:0:0:0:1]", true),
    ("0.0.0.0", true),
    // The local network.
    ("10.0.0.5", true),
    ("172.16.0.1", true),
    ("192.168.1.20", true),
    ("169.254.1.1", true),
    ("100.64.0.1", true),
    ("[fd00::1]", true),
    ("[fe80::1]", true),
    ("printer.local", true),
    ("Nas.Local.", true),
    ("nas", true),
    ("nas.lan", true),
    ("router.home.arpa", true),
    ("svc.internal", true),
    // The public internet.
    ("example.com", false),
    ("Example.COM.", false),
    ("bank.example", false),
    ("intranet.example.com", false),
    ("home.arpa", false),
    ("8.8.8.8", false),
    ("172.32.0.1", false),
    ("100.128.0.1", false),
    ("192.169.1.1", false),
    ("[2001:db8::1]", false),
    ("[::ffff:a00:1]", false),
    // Not a host no exception can name, even when it ends like a local one.
    ("", false),
    ("a b", false),
    ("::1", false),
    ("nas..", false),
    ("printer.local..", false),
    ("[::1]:8443", false),
    ("nas.local:8443", false),
];

#[test]
fn the_policy_table() {
    for (host, allowed) in POLICY {
        assert_eq!(certificate_exception_allowed(host), *allowed, "{host:?}");
    }
}

#[test]
fn the_policy_is_the_local_names_rule_on_the_host_form() {
    // Whatever is allowed is a local name in the form exceptions use, and the rule for local
    // names itself is unchanged: it still ignores every trailing dot, the policy does not.
    for (host, allowed) in POLICY {
        let Some(form) = certificate_exception_host(host) else {
            assert!(!certificate_exception_allowed(host), "{host:?}");
            continue;
        };
        assert_eq!(*allowed, is_local_host(&form), "{host:?}");
    }
    assert!(is_local_host("nas.."));
    assert!(!certificate_exception_allowed("nas.."));
}

// The controller

fn ready_controller() -> (BrowserController, TabId) {
    let controller = BrowserController::default();
    let tab_id = controller.active_tab_id();
    (controller, tab_id)
}

fn engine_shows(controller: &mut BrowserController, tab_id: TabId, url: &str) {
    controller.handle_event(tab_id, BrowserEvent::PageLoadStarted);
    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: url.to_owned(),
        },
    );
}

#[test]
fn the_controller_matches_exceptions_by_the_host_form() {
    let mut controller = BrowserController::default();
    controller.add_certificate_exception("Nas.Local.");
    for host in ["nas.local", "NAS.LOCAL", "nas.local.", "Nas.Local."] {
        assert!(controller.has_certificate_exception(host), "{host}");
    }
    for host in [
        "nas",
        "other.local",
        "local",
        "sub.nas.local",
        "nas.local..",
        "",
    ] {
        assert!(!controller.has_certificate_exception(host), "{host:?}");
    }

    controller.add_certificate_exception("[::1]");
    assert!(controller.has_certificate_exception("[::1]"));
    assert!(controller.has_certificate_exception("[0:0:0:0:0:0:0:1]"));
    assert!(!controller.has_certificate_exception("[::2]"));
    assert!(!controller.has_certificate_exception("::1"));
}

#[test]
fn a_host_that_is_not_a_host_is_never_an_exception() {
    let mut controller = BrowserController::default();
    for host in ["", "a b", "::1", "nas..", "host:8443", "[::1"] {
        controller.add_certificate_exception(host);
        assert!(!controller.has_certificate_exception(host), "{host:?}");
    }
    // Nothing was stored for the hosts that failed: no empty or half name matches by accident.
    for host in ["", "nas", "localhost", "[::1]"] {
        assert!(!controller.has_certificate_exception(host), "{host:?}");
    }
}

#[test]
fn the_indicator_warns_on_the_host_of_the_exception_in_any_spelling() {
    let (mut controller, tab_id) = ready_controller();
    controller.add_certificate_exception("Nas.Local.");
    controller.add_certificate_exception("[::1]");

    for address in [
        "https://nas.local/",
        "https://NAS.LOCAL./index.html",
        "https://nas.local:8443/",
        "https://[::1]:8443/",
        "https://[0:0:0:0:0:0:0:1]/",
    ] {
        engine_shows(&mut controller, tab_id, address);
        assert_eq!(
            controller.security_indicator(tab_id),
            SecurityIndicator::CertificateException,
            "{address}"
        );
    }
    for address in [
        "https://nas/",
        "https://other.local/",
        "https://sub.nas.local/",
        "https://[::2]/",
        "https://127.0.0.1/",
    ] {
        engine_shows(&mut controller, tab_id, address);
        assert_eq!(
            controller.security_indicator(tab_id),
            SecurityIndicator::Secure,
            "{address}"
        );
    }
}

#[test]
fn an_exception_does_not_make_a_plain_http_page_look_different() {
    let (mut controller, tab_id) = ready_controller();
    controller.add_certificate_exception("nas.local");
    engine_shows(&mut controller, tab_id, "http://nas.local/");
    assert_eq!(
        controller.security_indicator(tab_id),
        SecurityIndicator::NotSecure
    );
}
