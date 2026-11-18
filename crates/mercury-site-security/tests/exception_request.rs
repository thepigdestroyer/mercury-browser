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
                "secret={SECRET}&host=Self.Test&certificate=QUJD%2B%2F8"
            )),
            SECRET
        ),
        Some(CertificateException {
            host: "self.test".to_owned(),
            certificate: vec![0x41, 0x42, 0x43, 0xfb, 0xff],
        })
    );
}

#[test]
fn requests_without_the_secret_or_their_parts_are_refused() {
    for query in [
        "host=a.test&certificate=QUJD".to_owned(),
        "secret=0123456789abcdee&host=a.test&certificate=QUJD".to_owned(),
        "secret=0123&host=a.test&certificate=QUJD".to_owned(),
        format!("secret={SECRET}&certificate=QUJD"),
        format!("secret={SECRET}&host=&certificate=QUJD"),
        format!("secret={SECRET}&host=a.test"),
        format!("secret={SECRET}&host=a.test&certificate="),
        format!("secret={SECRET}&host=a.test&certificate=not*base64"),
    ] {
        assert_eq!(
            parse_certificate_exception_request(&request(&query), SECRET),
            None,
            "{query}"
        );
    }
    assert_eq!(
        parse_certificate_exception_request(&request("secret=&host=a&certificate=QUJD"), ""),
        None
    );
    let other_path = Url::parse(&format!(
        "mercury-certificate-exception:deny?secret={SECRET}&host=a.test&certificate=QUJD"
    ))
    .expect("a URL");
    assert_eq!(
        parse_certificate_exception_request(&other_path, SECRET),
        None
    );
    let other_scheme = Url::parse(&format!(
        "https://a.test/allow?secret={SECRET}&host=a.test&certificate=QUJD"
    ))
    .expect("a URL");
    assert_eq!(
        parse_certificate_exception_request(&other_scheme, SECRET),
        None
    );
}
