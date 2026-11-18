// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use mercury_site_security::{CertificateException, parse_certificate_exception_request};
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
        Err(mercury_site_security::ExceptionRefusal::NotFromMercury)
    );
    let other_scheme = Url::parse(&format!(
        "https://a.test/allow?secret={SECRET}&host=a.local&certificate=QUJD"
    ))
    .expect("a URL");
    assert_eq!(
        parse_certificate_exception_request(&other_scheme, SECRET),
        Err(mercury_site_security::ExceptionRefusal::NotFromMercury)
    );
}

#[test]
fn exceptions_are_accepted_only_for_local_names() {
    use mercury_site_security::ExceptionRefusal;

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
