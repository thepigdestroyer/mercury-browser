// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use mercury_site_security::{
    CERTIFICATE_EXCEPTION_ACCEPTED, CertificateException, EngineRefusal, ExceptionRefusal,
    parse_certificate_exception_request,
};
use url::Url;

const SECRET: &str = "0123456789abcdef";

fn request(query: &str) -> Url {
    Url::parse(&format!("mercury-certificate-exception:allow?{query}")).expect("a URL")
}

#[test]
fn a_request_with_the_secret_names_the_host_and_certificate() {
    assert_eq!(
        parse_certificate_exception_request(
            &request(&format!(
                "secret={SECRET}&host=Printer.Local&certificate=QUJD%2B%2F8"
            )),
            SECRET
        ),
        Ok(CertificateException {
            host: "printer.local".to_owned(),
            certificate: vec![0x41, 0x42, 0x43, 0xfb, 0xff],
        })
    );
}

#[test]
fn requests_without_the_secret_or_their_parts_are_refused() {
    for query in [
        "host=a.local&certificate=QUJD".to_owned(),
        "secret=0123456789abcdee&host=a.local&certificate=QUJD".to_owned(),
        "secret=0123&host=a.local&certificate=QUJD".to_owned(),
        format!("secret={SECRET}&certificate=QUJD"),
        format!("secret={SECRET}&host=&certificate=QUJD"),
        format!("secret={SECRET}&host=a.local"),
        format!("secret={SECRET}&host=a.local&certificate="),
        format!("secret={SECRET}&host=a.local&certificate=not*base64"),
    ] {
        assert!(
            parse_certificate_exception_request(&request(&query), SECRET).is_err(),
            "{query}"
        );
    }
    assert!(
        parse_certificate_exception_request(&request("secret=&host=a&certificate=QUJD"), "")
            .is_err()
    );
    let other_path = Url::parse(&format!(
        "mercury-certificate-exception:deny?secret={SECRET}&host=a.local&certificate=QUJD"
    ))
    .expect("a URL");
    assert_eq!(
        parse_certificate_exception_request(&other_path, SECRET),
        Err(ExceptionRefusal::NotFromMercury)
    );
    let other_scheme = Url::parse(&format!(
        "https://a.test/allow?secret={SECRET}&host=a.local&certificate=QUJD"
    ))
    .expect("a URL");
    assert_eq!(
        parse_certificate_exception_request(&other_scheme, SECRET),
        Err(ExceptionRefusal::NotFromMercury)
    );
}

#[test]
fn exceptions_are_accepted_only_for_local_names() {
    for host in [
        "127.0.0.1",
        "localhost",
        "192.168.1.20",
        "printer.local",
        "nas.lan",
    ] {
        let query = format!("secret={SECRET}&host={host}&certificate=QUJD");
        assert!(
            parse_certificate_exception_request(&request(&query), SECRET).is_ok(),
            "{host}"
        );
    }
    for host in ["example.com", "8.8.8.8", "bank.example"] {
        let query = format!("secret={SECRET}&host={host}&certificate=QUJD");
        assert_eq!(
            parse_certificate_exception_request(&request(&query), SECRET),
            Err(ExceptionRefusal::PublicHost(host.to_owned())),
            "{host}"
        );
    }
    let forged = request("secret=0123456789abcdee&host=127.0.0.1&certificate=QUJD");
    assert_eq!(
        parse_certificate_exception_request(&forged, SECRET),
        Err(ExceptionRefusal::NotFromMercury)
    );
}

#[test]
fn the_host_is_the_form_the_exceptions_use() {
    // The same host in any spelling is one host: what the engine is asked for, what the
    // browser remembers and what the address bar looks up.
    for (given, expected) in [
        ("Printer.Local", "printer.local"),
        ("PRINTER.LOCAL.", "printer.local"),
        ("localhost.", "localhost"),
        ("127.0.0.1", "127.0.0.1"),
        ("192.168.1.20.", "192.168.1.20"),
        ("%5B%3A%3A1%5D", "[::1]"),
        ("%5B0%3A0%3A0%3A0%3A0%3A0%3A0%3A1%5D", "[::1]"),
        ("%5BFD00%3A%3A1%5D", "[fd00::1]"),
    ] {
        let query = format!("secret={SECRET}&host={given}&certificate=QUJD");
        let exception = parse_certificate_exception_request(&request(&query), SECRET)
            .unwrap_or_else(|refusal| panic!("{given}: {refusal:?}"));
        assert_eq!(exception.host, expected, "{given}");
        assert_eq!(
            talaria_core::certificate_exception_host(&exception.host).as_deref(),
            Some(expected),
            "{given}"
        );
    }
}

#[test]
fn text_that_is_not_a_host_is_malformed_whatever_it_ends_like() {
    for host in [
        "a%20b",
        "%3A%3A1",
        "%5B%3A%3A1",
        "nas..",
        "printer.local..",
        "host%3A8443",
        "user%40nas.local",
        "nas.local%2Fpath",
    ] {
        let query = format!("secret={SECRET}&host={host}&certificate=QUJD");
        assert_eq!(
            parse_certificate_exception_request(&request(&query), SECRET),
            Err(ExceptionRefusal::Malformed),
            "{host}"
        );
    }
}

#[test]
fn a_public_host_is_refused_in_its_exception_form() {
    for (given, refused) in [
        ("Example.COM.", "example.com"),
        ("8.8.8.8", "8.8.8.8"),
        ("%5B2001%3Adb8%3A%3A1%5D", "[2001:db8::1]"),
    ] {
        let query = format!("secret={SECRET}&host={given}&certificate=QUJD");
        assert_eq!(
            parse_certificate_exception_request(&request(&query), SECRET),
            Err(ExceptionRefusal::PublicHost(refused.to_owned())),
            "{given}"
        );
    }
}

#[test]
fn the_request_gate_is_the_policy_of_the_browser() {
    // What `certificate_exception_allowed` says is what the parser accepts.
    for host in [
        "localhost",
        "Localhost.",
        "127.0.0.1",
        "10.1.2.3",
        "172.31.255.255",
        "192.168.0.1",
        "100.100.100.100",
        "printer.local",
        "nas",
        "a.lan",
        "example.com",
        "8.8.8.8",
        "172.32.0.1",
        "intranet.example.com",
    ] {
        let query = format!("secret={SECRET}&host={host}&certificate=QUJD");
        let parsed = parse_certificate_exception_request(&request(&query), SECRET);
        assert_eq!(
            parsed.is_ok(),
            talaria_core::certificate_exception_allowed(host),
            "{host}"
        );
    }
}

#[test]
fn the_secret_is_checked_before_anything_else() {
    // A request without the run's secret learns nothing about the host or the policy: not even
    // that the host is public or malformed.
    for host in ["example.com", "a%20b", "", "127.0.0.1"] {
        for certificate in ["QUJD", "", "not*base64"] {
            let query = format!("secret=wrong&host={host}&certificate={certificate}");
            assert_eq!(
                parse_certificate_exception_request(&request(&query), SECRET),
                Err(ExceptionRefusal::NotFromMercury),
                "{host} {certificate}"
            );
        }
    }
}

/// Every answer the page can get, with its status and its code.
fn every_refusal() -> Vec<(ExceptionRefusal, u16, &'static str)> {
    vec![
        (ExceptionRefusal::NotFromMercury, 403, "not-from-mercury"),
        (ExceptionRefusal::Malformed, 400, "malformed"),
        (
            ExceptionRefusal::PublicHost("example.com".to_owned()),
            403,
            "public-host",
        ),
        (
            ExceptionRefusal::Engine(EngineRefusal::InvalidHost),
            403,
            "invalid-host",
        ),
        (
            ExceptionRefusal::Engine(EngineRefusal::Hsts),
            403,
            "hsts-host",
        ),
        (
            ExceptionRefusal::Engine(EngineRefusal::NoFailureRecorded),
            403,
            "no-failure-recorded",
        ),
        (
            ExceptionRefusal::Engine(EngineRefusal::CertificateChanged),
            403,
            "certificate-changed",
        ),
        (
            ExceptionRefusal::Engine(EngineRefusal::Other),
            403,
            "refused",
        ),
    ]
}

#[test]
fn a_refusal_has_a_status_and_a_code() {
    for (refusal, status, code) in every_refusal() {
        assert_eq!(refusal.status(), status, "{refusal:?}");
        assert_eq!(refusal.code(), code, "{refusal:?}");
        // The status says what is wrong with the request, never that it worked.
        assert!((400..500).contains(&refusal.status()), "{refusal:?}");
        // The text of a refusal is a stable word, never text taken from the request or the
        // engine.
        assert!(
            code.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
            "{code}"
        );
    }
}

#[test]
fn every_engine_variant_is_in_the_table_of_refusals() {
    // No wildcard: a new variant does not compile until the table above has it too.
    fn covered(refusal: EngineRefusal) -> bool {
        match refusal {
            EngineRefusal::InvalidHost
            | EngineRefusal::Hsts
            | EngineRefusal::NoFailureRecorded
            | EngineRefusal::CertificateChanged
            | EngineRefusal::Other => every_refusal()
                .iter()
                .any(|(entry, _, _)| *entry == ExceptionRefusal::Engine(refusal)),
        }
    }
    for refusal in [
        EngineRefusal::InvalidHost,
        EngineRefusal::Hsts,
        EngineRefusal::NoFailureRecorded,
        EngineRefusal::CertificateChanged,
        EngineRefusal::Other,
    ] {
        assert!(covered(refusal), "{refusal:?}");
    }
}

#[test]
fn the_engine_codes_become_refusals() {
    // The codes of `servo-net`'s `CertificateExceptionRefusal::code`.
    for (code, expected) in [
        ("invalid-host", EngineRefusal::InvalidHost),
        ("hsts-host", EngineRefusal::Hsts),
        ("no-failure-recorded", EngineRefusal::NoFailureRecorded),
        ("certificate-changed", EngineRefusal::CertificateChanged),
    ] {
        assert_eq!(EngineRefusal::from_code(code), expected, "{code}");
        assert_eq!(expected.code(), code, "{code}");
    }
    // A code this version does not know, or text that is not a code, is a refusal all the same.
    for code in [
        "",
        "accepted",
        "refused",
        "HSTS-HOST",
        " hsts-host",
        "hsts-host ",
        "a-new-engine-code",
        "<script>alert(1)</script>",
    ] {
        assert_eq!(
            EngineRefusal::from_code(code),
            EngineRefusal::Other,
            "{code:?}"
        );
    }
    // The code of the unknown refusal is one the page has a line for, and reads back as itself.
    assert_eq!(EngineRefusal::Other.code(), "refused");
    assert_eq!(
        EngineRefusal::from_code(EngineRefusal::Other.code()),
        EngineRefusal::Other
    );
}

#[test]
fn the_accepted_answer_is_one_word() {
    assert_eq!(CERTIFICATE_EXCEPTION_ACCEPTED, "accepted");
    // It is never the code of a refusal.
    for (refusal, _, _) in every_refusal() {
        assert_ne!(refusal.code(), CERTIFICATE_EXCEPTION_ACCEPTED);
    }
}
