// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: tests for certificate exceptions bound to (host, certificate); see certificate_exceptions.rs.

//! Certificate exceptions are bound to the pair (host name, certificate) and fail closed.
//!
//! Most tests need no network: they use the exception store and the failure record through the public
//! API (`add_override`, the `*_for_test` helpers behind the `test-util` feature, and the three methods of
//! `HttpState`). The last group uses the loopback TLS server of `test_util` (self-signed certificate) that
//! is reachable under several host names: `localhost`, `127.0.0.1` and `[::1]`.

use std::sync::Arc;

use base64::Engine as _;
use fst::Streamer;
use http::{HeaderMap, HeaderValue, Method, header};
use http_body_util::combinators::BoxBody;
use hyper::body::{Bytes, Incoming};
use hyper::{Request as HyperRequest, Response as HyperResponse};
use net::certificate_exceptions::{
    CertificateExceptionRefusal, MAX_RECORDED_FAILURES, server_name_for_host,
};
use net::connector::{
    CACertificates, CertificateErrorOverrideManager, create_http_client, create_tls_config,
};
use net::fetch::methods::FetchContext;
use net::hsts::{HstsEntry, HstsList, HstsPreloadList};
use net::test_util::Server;
use net_traits::blob_url_store::UrlWithBlobClaim;
use net_traits::request::{
    Destination, Referrer, RequestBuilder, create_request_body_with_content,
};
use net_traits::response::Response;
use net_traits::{IncludeSubdomains, NetworkError};
use rustls_pki_types::{CertificateDer, ServerName};
use servo_base::id::{TEST_PIPELINE_ID, TEST_WEBVIEW_ID};
use servo_url::ServoUrl;

use crate::{create_http_state, fetch_with_context, make_body, make_ssl_server, new_fetch_context};

/// Host as given, and the spelling the exceptions list it under (`None`: no exception can name it).
///
/// These are the rows of `HOST_FORMS` in `crates/talaria-core/tests/certificate_exceptions.rs`, the
/// table of `talaria_core::certificate_exception_host`. The two functions differ only where the
/// engine's name has no brackets: `server_name_for_host` takes `[::1]` and gives the IPv6 name `::1`,
/// and the entries list it back as `[::1]`.
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
    // IPv6 literals keep their brackets, in the canonical form.
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
    // The core normaliser turns a name with letters outside ASCII into this ASCII form before the
    // engine sees it; the engine never accepts the raw form.
    ("xn--ns-qla.local", Some("xn--ns-qla.local")),
    ("N\u{e5}s.local", None),
];

fn der(bytes: &[u8]) -> CertificateDer<'static> {
    CertificateDer::from(bytes.to_vec())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---------------------------------------------------------------------------------------------------
// No network.
// ---------------------------------------------------------------------------------------------------

#[test]
fn the_host_form_table() {
    for (host, expected) in HOST_FORMS {
        // The engine's name has no brackets around an IPv6 literal.
        let expected_name = expected.map(|text| {
            ServerName::try_from(text.trim_matches(['[', ']']))
                .expect("a name")
                .to_owned()
        });
        assert_eq!(server_name_for_host(host), expected_name, "{host:?}");
    }
}

#[test]
fn the_host_form_table_through_add_override_and_the_listing() {
    for (host, expected) in HOST_FORMS {
        let state = create_http_state(None);
        let certificate = der(b"a certificate");
        let added = state.override_manager.add_override(host, &certificate);
        match expected {
            Some(text) => {
                assert_eq!(added, Ok(()), "{host:?}");
                let entries = state.certificate_exceptions();
                assert_eq!(entries.len(), 1, "{host:?}");
                assert_eq!(entries[0].host, *text, "{host:?}");
            },
            None => {
                assert_eq!(
                    added,
                    Err(CertificateExceptionRefusal::InvalidHost),
                    "{host:?}"
                );
                assert!(state.certificate_exceptions().is_empty(), "{host:?}");
            },
        }
    }
}

#[test]
fn the_refusal_codes_are_stable() {
    assert_eq!(
        CertificateExceptionRefusal::InvalidHost.code(),
        "invalid-host"
    );
    assert_eq!(CertificateExceptionRefusal::HstsHost.code(), "hsts-host");
    assert_eq!(
        CertificateExceptionRefusal::NoFailureRecorded.code(),
        "no-failure-recorded"
    );
    assert_eq!(
        CertificateExceptionRefusal::CertificateChanged.code(),
        "certificate-changed"
    );
    assert_eq!(
        CertificateExceptionRefusal::HstsHost.to_string(),
        "certificate exception refused: hsts-host"
    );
}

#[test]
fn an_exception_is_the_pair_of_host_and_certificate() {
    let state = create_http_state(None);
    let manager = &state.override_manager;
    let accepted = der(b"the certificate the user accepted");
    let another = der(b"another certificate");

    assert_eq!(manager.add_override("nas.local", &accepted), Ok(()));

    // The same host and certificate, in every spelling of the host.
    for host in ["nas.local", "NAS.Local", "nas.local."] {
        assert!(manager.has_exception(host, &accepted), "{host}");
    }
    // The same certificate on another host name.
    for host in ["other.local", "nas.example", "127.0.0.1", "[::1]", "nas"] {
        assert!(!manager.has_exception(host, &accepted), "{host}");
    }
    // Another certificate on the same host.
    assert!(!manager.has_exception("nas.local", &another));
    // A subdomain and the parent domain of the accepted host.
    assert!(!manager.has_exception("www.nas.local", &accepted));
    assert!(!manager.has_exception("local", &accepted));
    // A host that cannot be named has no exception, whatever the certificate.
    for host in ["", "a b", "::1]", "nas.local.."] {
        assert!(!manager.has_exception(host, &accepted), "{host:?}");
    }

    // An exception for a subdomain is not one for its parent, and the other way round.
    assert_eq!(manager.add_override("a.example.test", &accepted), Ok(()));
    assert!(manager.has_exception("a.example.test", &accepted));
    assert!(!manager.has_exception("example.test", &accepted));
    assert!(!manager.has_exception("b.example.test", &accepted));
    assert!(!manager.has_exception("x.a.example.test", &accepted));

    // IP addresses and IPv6 literals are names of their own.
    assert_eq!(manager.add_override("[::1]", &accepted), Ok(()));
    assert_eq!(manager.add_override("127.0.0.1", &accepted), Ok(()));
    assert!(manager.has_exception("[0:0:0:0:0:0:0:1]", &accepted));
    assert!(manager.has_exception("127.0.0.1", &accepted));
    assert!(!manager.has_exception("127.0.0.2", &accepted));
    assert!(!manager.has_exception("localhost", &accepted));
}

#[test]
fn add_override_refuses_a_host_that_cannot_be_named() {
    let state = create_http_state(None);
    for host in ["", "a b", "::1]", "[::1", "a..b"] {
        assert_eq!(
            state.override_manager.add_override(host, &der(b"x")),
            Err(CertificateExceptionRefusal::InvalidHost),
            "{host:?}"
        );
    }
    assert!(state.certificate_exceptions().is_empty());
}

#[test]
fn accepting_needs_a_valid_host() {
    let state = create_http_state(None);
    for host in ["", "a b", "::1]", "[::1", "nas..", "a/b"] {
        assert_eq!(
            state.accept_failed_certificate(host, b"x"),
            Err(CertificateExceptionRefusal::InvalidHost),
            "{host:?}"
        );
    }
    assert!(state.certificate_exceptions().is_empty());
}

#[test]
fn accepting_needs_a_recorded_failure() {
    let state = create_http_state(None);
    // Never failed.
    assert_eq!(
        state.accept_failed_certificate("nas.local", b"x"),
        Err(CertificateExceptionRefusal::NoFailureRecorded)
    );
    // A failure that was noted but not yet reported (no error page was built) is not a record.
    let manager = &state.override_manager;
    manager.note_certificate_failure_for_test("nas.local", &der(b"x"));
    assert_eq!(
        state.accept_failed_certificate("nas.local", b"x"),
        Err(CertificateExceptionRefusal::NoFailureRecorded)
    );
    // The record of one host name does not serve another.
    assert_eq!(
        manager.report_certificate_failure_for_test("nas.local"),
        Some(der(b"x"))
    );
    for other in [
        "other.local",
        "www.nas.local",
        "local",
        "127.0.0.1",
        "[::1]",
    ] {
        assert_eq!(
            state.accept_failed_certificate(other, b"x"),
            Err(CertificateExceptionRefusal::NoFailureRecorded),
            "{other}"
        );
    }
    assert!(state.certificate_exceptions().is_empty());
    // And the host itself can be accepted, once.
    assert_eq!(state.accept_failed_certificate("nas.local", b"x"), Ok(()));
    assert_eq!(
        state.accept_failed_certificate("nas.local", b"x"),
        Err(CertificateExceptionRefusal::NoFailureRecorded)
    );
}

#[test]
fn accepting_only_takes_the_certificate_the_host_failed_with() {
    let state = create_http_state(None);
    let manager = &state.override_manager;
    let shown = der(b"the certificate the user was shown");

    manager.note_certificate_failure_for_test("bank.example", &shown);
    assert_eq!(
        manager.report_certificate_failure_for_test("bank.example"),
        Some(shown.clone())
    );

    // A page that sends another certificate cannot get it accepted, and nothing changes.
    let wrong_certificates: [&[u8]; 3] = [b"an attacker's certificate", b"", &shown.as_ref()[1..]];
    for wrong in wrong_certificates {
        assert_eq!(
            state.accept_failed_certificate("bank.example", wrong),
            Err(CertificateExceptionRefusal::CertificateChanged)
        );
    }
    assert!(state.certificate_exceptions().is_empty());
    assert!(!manager.has_exception("bank.example", &der(b"an attacker's certificate")));

    // The record survived the refusals, so the right certificate is still accepted.
    assert_eq!(
        state.accept_failed_certificate("bank.example", shown.as_ref()),
        Ok(())
    );
    assert!(manager.has_exception("bank.example", &shown));
    assert_eq!(state.certificate_exceptions().len(), 1);
}

#[test]
fn the_newest_failure_of_a_host_replaces_the_older_one() {
    let state = create_http_state(None);
    let manager = &state.override_manager;
    for certificate in [b"first", b"newer"] {
        manager.note_certificate_failure_for_test("nas.local", &der(certificate));
        assert_eq!(
            manager.report_certificate_failure_for_test("nas.local"),
            Some(der(certificate))
        );
    }
    assert_eq!(
        state.accept_failed_certificate("nas.local", b"first"),
        Err(CertificateExceptionRefusal::CertificateChanged)
    );
    assert_eq!(
        state.accept_failed_certificate("nas.local", b"newer"),
        Ok(())
    );
    assert!(manager.has_exception("nas.local", &der(b"newer")));
    assert!(!manager.has_exception("nas.local", &der(b"first")));
}

#[test]
fn accepting_clears_the_pending_failure_of_the_host() {
    let state = create_http_state(None);
    let manager = &state.override_manager;
    manager.note_certificate_failure_for_test("nas.local", &der(b"c1"));
    assert!(
        manager
            .report_certificate_failure_for_test("nas.local")
            .is_some()
    );
    // A later failure is pending again, and a refusal leaves it alone.
    manager.note_certificate_failure_for_test("nas.local", &der(b"c2"));
    assert_eq!(
        state.accept_failed_certificate("nas.local", b"c3"),
        Err(CertificateExceptionRefusal::CertificateChanged)
    );
    assert_eq!(
        manager.report_certificate_failure_for_test("nas.local"),
        Some(der(b"c2"))
    );

    // Pending again; accepting the recorded certificate (c2) clears it.
    manager.note_certificate_failure_for_test("nas.local", &der(b"c4"));
    assert_eq!(state.accept_failed_certificate("nas.local", b"c2"), Ok(()));
    // A stale pending entry would turn an unrelated later network error of this host into a
    // certificate error.
    assert_eq!(
        manager.report_certificate_failure_for_test("nas.local"),
        None
    );
}

#[test]
fn the_failure_record_is_bounded_and_drops_the_oldest_host() {
    let state = create_http_state(None);
    let manager = &state.override_manager;
    let extra = 6;
    let host = |index: usize| format!("h{index}.nas.local");
    let certificate = |index: usize| der(format!("certificate {index}").as_bytes());

    for index in 0..MAX_RECORDED_FAILURES + extra {
        manager.note_certificate_failure_for_test(&host(index), &certificate(index));
        assert!(
            manager
                .report_certificate_failure_for_test(&host(index))
                .is_some()
        );
    }
    // The oldest `extra` hosts were dropped: fail closed, the user reloads the page.
    for index in 0..extra {
        assert_eq!(
            state.accept_failed_certificate(&host(index), certificate(index).as_ref()),
            Err(CertificateExceptionRefusal::NoFailureRecorded),
            "{index}"
        );
    }
    // The newest MAX_RECORDED_FAILURES hosts are kept.
    for index in extra..MAX_RECORDED_FAILURES + extra {
        assert_eq!(
            state.accept_failed_certificate(&host(index), certificate(index).as_ref()),
            Ok(()),
            "{index}"
        );
    }
    assert_eq!(state.certificate_exceptions().len(), MAX_RECORDED_FAILURES);
}

#[test]
fn a_host_that_fails_again_counts_as_newest() {
    let state = create_http_state(None);
    let manager = &state.override_manager;
    let host = |index: usize| format!("h{index}.nas.local");
    let report = |index: usize| {
        manager.note_certificate_failure_for_test(&host(index), &der(b"c"));
        assert!(
            manager
                .report_certificate_failure_for_test(&host(index))
                .is_some()
        );
    };
    for index in 0..MAX_RECORDED_FAILURES {
        report(index);
    }
    // h0 is the oldest; failing again makes it the newest, so the next new host drops h1 instead.
    report(0);
    report(MAX_RECORDED_FAILURES);
    assert_eq!(state.accept_failed_certificate(&host(0), b"c"), Ok(()));
    assert_eq!(
        state.accept_failed_certificate(&host(1), b"c"),
        Err(CertificateExceptionRefusal::NoFailureRecorded)
    );
    assert_eq!(state.accept_failed_certificate(&host(2), b"c"), Ok(()));
}

#[test]
fn a_host_on_the_learned_hsts_list_is_refused() {
    let state = create_http_state(None);
    let manager = &state.override_manager;
    state.hsts_list.write().push(
        HstsEntry::new(
            "hsts-learned.example.org".to_owned(),
            IncludeSubdomains::Included,
            None,
        )
        .expect("a host"),
    );
    for host in ["hsts-learned.example.org", "www.hsts-learned.example.org"] {
        manager.note_certificate_failure_for_test(host, &der(b"c"));
        assert!(manager.report_certificate_failure_for_test(host).is_some());
        assert_eq!(
            state.accept_failed_certificate(host, b"c"),
            Err(CertificateExceptionRefusal::HstsHost),
            "{host}"
        );
    }
    // The refusal comes before the question whether a failure was recorded.
    assert_eq!(
        state.accept_failed_certificate("deeper.www.hsts-learned.example.org", b"c"),
        Err(CertificateExceptionRefusal::HstsHost)
    );
    assert!(state.certificate_exceptions().is_empty());

    // A host the list does not cover is not affected.
    manager.note_certificate_failure_for_test("example.org", &der(b"c"));
    assert!(
        manager
            .report_certificate_failure_for_test("example.org")
            .is_some()
    );
    assert_eq!(state.accept_failed_certificate("example.org", b"c"), Ok(()));
}

#[test]
fn a_host_on_the_preloaded_hsts_list_is_refused() {
    let host = a_preloaded_host();
    let state = create_http_state(None);
    let manager = &state.override_manager;
    // Nothing was learned for it: the refusal comes from the preload list.
    assert!(state.hsts_list.read().entries_map.is_empty());
    manager.note_certificate_failure_for_test(&host, &der(b"c"));
    assert!(manager.report_certificate_failure_for_test(&host).is_some());
    assert_eq!(
        state.accept_failed_certificate(&host, b"c"),
        Err(CertificateExceptionRefusal::HstsHost),
        "{host}"
    );
    // Upper case and the trailing dot of a fully qualified name do not get around it.
    assert_eq!(
        state.accept_failed_certificate(&format!("{}.", host.to_uppercase()), b"c"),
        Err(CertificateExceptionRefusal::HstsHost)
    );
    assert!(state.certificate_exceptions().is_empty());
}

/// The first host of the preload list that the engine can name (the list is data, not a contract).
fn a_preloaded_host() -> String {
    let preload = HstsPreloadList::from_servo_preload();
    let mut keys = preload.0.stream();
    while let Some((key, _)) = keys.next() {
        if let Ok(host) = std::str::from_utf8(key) {
            if server_name_for_host(host).is_some() && HstsList::default().is_host_secure(host) {
                return host.to_owned();
            }
        }
    }
    panic!("the HSTS preload list has no host the engine can name");
}

#[test]
fn revoking_and_listing() {
    let state = create_http_state(None);
    let manager = &state.override_manager;
    // The SHA-256 of "abc" (FIPS 180-2 test vector), to pin what the listing hashes.
    let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    assert_eq!(manager.add_override("nas.local", &der(b"abc")), Ok(()));
    assert_eq!(manager.add_override("nas.local", &der(b"second")), Ok(()));
    assert_eq!(manager.add_override("other.local", &der(b"abc")), Ok(()));
    assert_eq!(manager.add_override("[::1]", &der(b"abc")), Ok(()));
    // The same pair twice is one exception.
    assert_eq!(manager.add_override("NAS.local.", &der(b"abc")), Ok(()));

    let entries = state.certificate_exceptions();
    let listed: Vec<(&str, String)> = entries
        .iter()
        .map(|entry| (entry.host.as_str(), hex(&entry.sha256)))
        .collect();
    assert_eq!(listed.len(), 4, "{listed:?}");
    assert_eq!(listed[0], ("[::1]", abc.to_owned()));
    assert_eq!(listed[3], ("other.local", abc.to_owned()));
    // The two of nas.local, sorted by hash.
    assert_eq!(listed[1].0, "nas.local");
    assert_eq!(listed[2].0, "nas.local");
    assert!(listed[1].1 < listed[2].1, "{listed:?}");
    assert!(listed[1].1 == abc || listed[2].1 == abc, "{listed:?}");
    // The listing is sorted by host and then by hash.
    let mut sorted = entries.clone();
    sorted.sort();
    assert_eq!(sorted, entries);

    // Revoking forgets every exception of the host, in any spelling, and says how many.
    assert_eq!(state.revoke_certificate_exception("NAS.Local."), 2);
    assert_eq!(state.revoke_certificate_exception("nas.local"), 0);
    assert_eq!(state.revoke_certificate_exception("never.local"), 0);
    assert_eq!(state.revoke_certificate_exception("a b"), 0);
    assert!(!manager.has_exception("nas.local", &der(b"abc")));
    assert!(!manager.has_exception("nas.local", &der(b"second")));
    // The other hosts keep theirs.
    let hosts: Vec<_> = state
        .certificate_exceptions()
        .into_iter()
        .map(|entry| entry.host)
        .collect();
    assert_eq!(hosts, ["[::1]", "other.local"]);
    assert_eq!(state.revoke_certificate_exception("[0:0:0:0:0:0:0:1]"), 1);
    assert_eq!(state.revoke_certificate_exception("other.local"), 1);
    assert!(state.certificate_exceptions().is_empty());
}

#[test]
fn the_session_caches_of_dropped_clients_are_not_kept() {
    // `create_tls_config` is also called for every WebSocket connection.
    let state = create_http_state(None);
    let manager = state.override_manager.clone();
    let make = || create_tls_config(CACertificates::Default, false, manager.clone());
    // The state's own client, and the three below.
    let alive: Vec<_> = (0..3).map(|_| make()).collect();
    assert_eq!(manager.session_cache_handles_for_test(), 4);
    drop(alive);
    for _ in 0..50 {
        drop(make());
    }
    // The state's client, and at most the one dropped client that the next registration would drop.
    assert!(manager.session_cache_handles_for_test() <= 2);
}

#[test]
fn chrome_allowcert_answers_and_adds_nothing() {
    let mut context = new_fetch_context(None, None);
    let certificate = der(b"a certificate sent to Servo's own chrome:allowcert page");
    let hosts = ["localhost", "127.0.0.1", "bank.example", "nas.local"];
    // The old entry point took the per-process secret and the base64 of a certificate, and then
    // accepted that certificate for every host. Send exactly that.
    let body = format!(
        "{}&{}",
        *net_traits::PRIVILEGED_SECRET,
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(certificate.as_ref())
    );
    let url = ServoUrl::parse("chrome:allowcert").expect("a url");
    let mut request = RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::for_url(url.clone()).expect("not a blob url"),
        Referrer::NoReferrer,
    )
    .method(Method::POST)
    .body(Some(create_request_body_with_content(body)))
    .destination(Destination::Document)
    .origin(url.origin())
    .pipeline_id(Some(TEST_PIPELINE_ID))
    .policy_container(Default::default())
    .build();
    request.local_urls_only = false;

    let response = fetch_with_context(request, &mut context);

    assert!(
        response.get_network_error().is_none(),
        "the request is answered: {:?}",
        response.get_network_error()
    );
    assert!(context.state.certificate_exceptions().is_empty());
    for host in hosts {
        assert!(
            !context
                .state
                .override_manager
                .has_exception(host, &certificate),
            "{host}"
        );
    }
}

// ---------------------------------------------------------------------------------------------------
// The loopback TLS server (self-signed certificate), reached under several host names.
// ---------------------------------------------------------------------------------------------------

/// The test server, closing every connection after one response. (It serves one connection at a
/// time, so a kept-alive connection under one host name would block the next host name; and a closed
/// connection guarantees that the next request makes a new TLS handshake.)
fn tls_server() -> (Server, u16) {
    tls_server_with_hsts(None)
}

/// The same server, and with `Some(policy)` every answer carries `Strict-Transport-Security: <policy>`.
fn tls_server_with_hsts(hsts: Option<&'static str>) -> (Server, u16) {
    let handler =
        move |_: HyperRequest<Incoming>,
              response: &mut HyperResponse<BoxBody<Bytes, hyper::Error>>| {
            response
                .headers_mut()
                .insert(header::CONNECTION, HeaderValue::from_static("close"));
            if let Some(policy) = hsts {
                response.headers_mut().insert(
                    header::STRICT_TRANSPORT_SECURITY,
                    HeaderValue::from_static(policy),
                );
            }
            // Nothing may be answered from Servo's HTTP cache: these tests are about connections.
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            *response.body_mut() = make_body(b"Yay!".to_vec());
        };
    let (server, url) = make_ssl_server(handler);
    let port = url.port().expect("the server has a port");
    (server, port)
}

fn server_certificate(server: &Server) -> CertificateDer<'static> {
    server.certificates.as_ref().expect("a TLS server")[0].clone()
}

/// A GET of `https://<host>:<port>/`, `host` as it is written in a URL.
fn get(context: &mut FetchContext, host: &str, port: u16) -> Response {
    let url = ServoUrl::parse(&format!("https://{host}:{port}/")).expect("a url");
    let url = UrlWithBlobClaim::for_url(url).expect("not a blob url");
    let request = RequestBuilder::new(Some(TEST_WEBVIEW_ID), url.clone(), Referrer::NoReferrer)
        .method(Method::GET)
        .body(None)
        .destination(Destination::Document)
        .origin(url.origin())
        .pipeline_id(Some(TEST_PIPELINE_ID))
        .policy_container(Default::default())
        .build();
    fetch_with_context(request, context)
}

/// The certificate of a certificate error (what Mercury's certificate page shows), if the response is
/// one.
fn certificate_error(response: &Response) -> Option<Vec<u8>> {
    match response.get_network_error() {
        Some(NetworkError::SslValidation(_, certificate)) => Some(certificate.clone()),
        _ => None,
    }
}

fn loaded(response: &Response) -> bool {
    response.get_network_error().is_none() && response.status.code().is_success()
}

#[test]
fn a_failed_load_carries_the_certificate_the_server_presented() {
    let (server, port) = tls_server();
    let certificate = server_certificate(&server);
    let mut context = new_fetch_context(None, None);

    for host in ["localhost", "127.0.0.1", "[::1]"] {
        let response = get(&mut context, host, port);
        assert_eq!(
            certificate_error(&response),
            Some(certificate.to_vec()),
            "{host}: {:?}",
            response.get_network_error()
        );
    }
    server.close();
}

#[test]
fn an_exception_is_for_one_host_name_and_revoking_it_fails_closed() {
    let (server, port) = tls_server();
    let certificate = server_certificate(&server);
    let mut context = new_fetch_context(None, None);
    let state = context.state.clone();

    // Both names of the one server fail, with the same certificate.
    assert_eq!(
        certificate_error(&get(&mut context, "localhost", port)),
        Some(certificate.to_vec())
    );
    assert_eq!(
        certificate_error(&get(&mut context, "127.0.0.1", port)),
        Some(certificate.to_vec())
    );

    // The user accepts the certificate that `localhost` failed with.
    assert_eq!(
        state.accept_failed_certificate("localhost", certificate.as_ref()),
        Ok(())
    );
    assert!(loaded(&get(&mut context, "localhost", port)));
    let entries = state.certificate_exceptions();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].host, "localhost");

    // The same certificate under the other name of the same server still fails: Servo's old override
    // accepted it for every host name.
    let other = get(&mut context, "127.0.0.1", port);
    assert!(!loaded(&other));
    assert_eq!(certificate_error(&other), Some(certificate.to_vec()));
    assert!(
        !state
            .override_manager
            .has_exception("127.0.0.1", &certificate)
    );

    // Revoking forgets it; a new connection fails again. (This server closes every connection, and
    // the TLS session ticket that the accepted handshake left is forgotten by the revoke: rustls does
    // not verify a resumed session.)
    assert_eq!(state.revoke_certificate_exception("localhost"), 1);
    assert!(state.certificate_exceptions().is_empty());
    let again = get(&mut context, "localhost", port);
    assert_eq!(
        certificate_error(&again),
        Some(certificate.to_vec()),
        "status {:?}, error {:?}",
        again.status,
        again.get_network_error()
    );
    server.close();
}

#[test]
fn the_record_of_one_host_name_does_not_accept_another_name_of_the_server() {
    let (server, port) = tls_server();
    let certificate = server_certificate(&server);
    let mut context = new_fetch_context(None, None);
    let state = context.state.clone();

    assert!(certificate_error(&get(&mut context, "localhost", port)).is_some());
    // Only `localhost` failed: there is no failure to accept for the other name.
    assert_eq!(
        state.accept_failed_certificate("127.0.0.1", certificate.as_ref()),
        Err(CertificateExceptionRefusal::NoFailureRecorded)
    );
    assert!(!loaded(&get(&mut context, "127.0.0.1", port)));
    server.close();
}

#[test]
fn another_certificate_for_the_same_host_name_does_not_help() {
    let (server, port) = tls_server();
    let certificate = server_certificate(&server);
    let mut context = new_fetch_context(None, None);
    let state = context.state.clone();
    let another = der(b"not the certificate of this server");

    // An exception for another certificate on the same host name: the server's still fails.
    assert_eq!(
        state.override_manager.add_override("localhost", &another),
        Ok(())
    );
    assert_eq!(
        certificate_error(&get(&mut context, "localhost", port)),
        Some(certificate.to_vec())
    );
    // A user decision for another certificate than the one the host failed with is refused.
    assert_eq!(
        state.accept_failed_certificate("localhost", another.as_ref()),
        Err(CertificateExceptionRefusal::CertificateChanged)
    );
    assert!(!loaded(&get(&mut context, "localhost", port)));
    server.close();
}

#[test]
fn an_ipv6_literal_gets_a_certificate_error_and_can_be_accepted() {
    let (server, port) = tls_server();
    let certificate = server_certificate(&server);
    let mut context = new_fetch_context(None, None);
    let state = context.state.clone();

    // `Uri::host()` keeps the brackets, which `ServerName::try_from` refuses: before the fix no
    // certificate was reported for an IPv6 literal, so there was nothing to accept.
    assert_eq!(
        certificate_error(&get(&mut context, "[::1]", port)),
        Some(certificate.to_vec())
    );
    assert_eq!(
        state.accept_failed_certificate("[::1]", certificate.as_ref()),
        Ok(())
    );
    assert!(loaded(&get(&mut context, "[::1]", port)));
    // IPv4 loopback and `localhost` are other names of the same server.
    assert!(!loaded(&get(&mut context, "127.0.0.1", port)));
    assert!(!loaded(&get(&mut context, "localhost", port)));
    server.close();
}

// ---------------------------------------------------------------------------------------------------
// HSTS is not learned over a connection that a certificate exception made valid (RFC 6797, section 8.1).
// ---------------------------------------------------------------------------------------------------

/// What a NAS or a router sends from its self-signed HTTPS page: a year, with subdomains.
const STRICT_TRANSPORT_SECURITY: &str = "max-age=31536000; includeSubDomains";

/// An HTTP state whose client does not verify certificates at all (Servo's own switch for testing,
/// `ignore_certificate_errors`). No exception is involved in its connections, so it stands for a
/// connection that needed none: the HSTS list may learn from it. Also used by the upstream test
/// `fetch::test_load_adds_host_to_hsts_list_when_url_is_https`, which used an exception to reach its
/// self-signed server.
pub fn state_that_ignores_certificate_errors() -> net::test::HttpState {
    let mut state = create_http_state(None);
    state.override_manager = CertificateErrorOverrideManager::new();
    state.client = create_http_client(create_tls_config(
        CACertificates::Default,
        true, /* ignore_certificate_errors */
        state.override_manager.clone(),
    ));
    state
}

fn context_that_ignores_certificate_errors() -> FetchContext {
    let mut context = new_fetch_context(None, None);
    context.state = Arc::new(state_that_ignores_certificate_errors());
    context
}

#[test]
fn the_fetch_path_learns_hsts_from_a_connection_that_needed_no_exception() {
    // The control of the next test: the same server, the same header, the same fetch path, and no
    // exception. The list learns the host, so the next test's empty list is the guard's doing and not
    // a header that never reached the list.
    let (server, port) = tls_server_with_hsts(Some(STRICT_TRANSPORT_SECURITY));
    let mut context = context_that_ignores_certificate_errors();
    let state = context.state.clone();

    assert!(loaded(&get(&mut context, "localhost", port)));
    assert!(state.certificate_exceptions().is_empty());
    assert!(
        state.hsts_list.read().is_host_secure("localhost"),
        "the list learns from a connection that needed no exception"
    );
    server.close();
}

#[test]
fn hsts_is_not_learned_over_a_connection_that_an_exception_made_valid() {
    let (server, port) = tls_server_with_hsts(Some(STRICT_TRANSPORT_SECURITY));
    let certificate = server_certificate(&server);
    let mut context = new_fetch_context(None, None);
    let state = context.state.clone();

    assert_eq!(
        certificate_error(&get(&mut context, "localhost", port)),
        Some(certificate.to_vec())
    );
    assert_eq!(
        state.accept_failed_certificate("localhost", certificate.as_ref()),
        Ok(())
    );
    // The page loads over the exception, and its answer asks for HSTS.
    assert!(loaded(&get(&mut context, "localhost", port)));
    assert!(
        state.hsts_list.read().entries_map.is_empty() &&
            !state.hsts_list.read().is_host_secure("localhost"),
        "the list learned a host from a connection that only the user's exception made valid"
    );

    // So the user is not shut out later: the list is what Mercury keeps between sessions, and it still
    // does not cover the host, so a new certificate error on it can be accepted again.
    assert_eq!(state.revoke_certificate_exception("localhost"), 1);
    assert_eq!(
        certificate_error(&get(&mut context, "localhost", port)),
        Some(certificate.to_vec())
    );
    assert_eq!(
        state.accept_failed_certificate("localhost", certificate.as_ref()),
        Ok(())
    );
    server.close();
}

// ---------------------------------------------------------------------------------------------------
// The HSTS guard without a network: which hosts may teach the list.
// ---------------------------------------------------------------------------------------------------

fn strict_transport_security() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static(STRICT_TRANSPORT_SECURITY),
    );
    headers
}

fn https(host: &str) -> ServoUrl {
    ServoUrl::parse(&format!("https://{host}/")).expect("a url")
}

/// The number of hosts the learned list holds.
fn learned_hosts(state: &net::test::HttpState) -> usize {
    state
        .hsts_list
        .read()
        .entries_map
        .values()
        .map(Vec::len)
        .sum()
}

#[test]
fn a_host_without_an_exception_teaches_the_hsts_list() {
    let state = create_http_state(None);
    let headers = strict_transport_security();
    assert!(state.may_learn_hsts_from(&https("bank.example")));
    state.learn_hsts_from_response(&https("bank.example"), &headers);
    assert!(state.hsts_list.read().is_host_secure("bank.example"));
    // Only an answer over https counts, as before.
    state.learn_hsts_from_response(
        &ServoUrl::parse("http://plain.example/").expect("a url"),
        &headers,
    );
    assert!(!state.hsts_list.read().is_host_secure("plain.example"));
}

#[test]
fn a_host_with_an_exception_does_not_teach_the_hsts_list_in_any_spelling() {
    let state = create_http_state(None);
    let headers = strict_transport_security();
    assert_eq!(
        state
            .override_manager
            .add_override("nas.local", &der(b"the certificate of the NAS")),
        Ok(())
    );
    for host in ["nas.local", "NAS.LOCAL", "nas.local.", "Nas.Local."] {
        assert!(!state.may_learn_hsts_from(&https(host)), "{host}");
        state.learn_hsts_from_response(&https(host), &headers);
    }
    assert_eq!(learned_hosts(&state), 0);
    assert!(!state.hsts_list.read().is_host_secure("nas.local"));

    // Other names do: a connection to them needed no exception. (`includeSubDomains` of the NAS's own
    // answer is what the exception blocks; a neighbour's answer is the neighbour's.)
    for host in ["www.nas.local", "nas.example", "bank.example", "local"] {
        assert!(state.may_learn_hsts_from(&https(host)), "{host}");
    }
    state.learn_hsts_from_response(&https("bank.example"), &headers);
    assert!(state.hsts_list.read().is_host_secure("bank.example"));
    assert_eq!(learned_hosts(&state), 1);
}

#[test]
fn a_revoked_exception_still_blocks_hsts_because_its_connection_may_be_open() {
    // A revoke does not close a connection that was opened under the exception, and an answer on it
    // would still be one that only the exception made valid.
    let state = create_http_state(None);
    state
        .override_manager
        .add_override("nas.local", &der(b"c"))
        .expect("a host");
    assert_eq!(state.revoke_certificate_exception("nas.local"), 1);
    assert!(state.certificate_exceptions().is_empty());
    assert!(!state.may_learn_hsts_from(&https("nas.local")));
    state.learn_hsts_from_response(&https("nas.local"), &strict_transport_security());
    assert_eq!(learned_hosts(&state), 0);
}

#[test]
fn a_users_decision_blocks_hsts_for_the_host_and_a_refusal_does_not() {
    let state = create_http_state(None);
    let manager = &state.override_manager;
    // Refusals change nothing: no failure recorded, an unparsable host, another certificate.
    assert_eq!(
        state.accept_failed_certificate("device.local", b"c"),
        Err(CertificateExceptionRefusal::NoFailureRecorded)
    );
    assert_eq!(
        state.accept_failed_certificate("a b", b"c"),
        Err(CertificateExceptionRefusal::InvalidHost)
    );
    manager.note_certificate_failure_for_test("device.local", &der(b"c"));
    assert!(
        manager
            .report_certificate_failure_for_test("device.local")
            .is_some()
    );
    assert_eq!(
        state.accept_failed_certificate("device.local", b"another"),
        Err(CertificateExceptionRefusal::CertificateChanged)
    );
    assert!(state.may_learn_hsts_from(&https("device.local")));

    // The accepted decision does.
    assert_eq!(
        state.accept_failed_certificate("device.local", b"c"),
        Ok(())
    );
    assert!(!state.may_learn_hsts_from(&https("device.local")));
    state.learn_hsts_from_response(&https("device.local"), &strict_transport_security());
    assert_eq!(learned_hosts(&state), 0);
}

#[test]
fn an_ipv6_literal_with_an_exception_is_named_like_the_exception() {
    // HSTS never applies to an address, so nothing is learned either way; the answer of the guard
    // is still the one the exception's name gives.
    let state = create_http_state(None);
    state
        .override_manager
        .add_override("[::1]", &der(b"c"))
        .expect("a host");
    assert!(!state.may_learn_hsts_from(&https("[::1]")));
    assert!(!state.may_learn_hsts_from(&https("[0:0:0:0:0:0:0:1]")));
    assert!(state.may_learn_hsts_from(&https("[::2]")));
}

/// Every `.rs` file of the crate except the tests.
fn crate_sources(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("the crate directory") {
        let path = entry.expect("a directory entry").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        if path.is_dir() {
            if name != "tests" && name != "target" {
                crate_sources(&path, found);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn only_the_guard_updates_the_hsts_list_from_a_response() {
    // The two places that learn HSTS from an answer (the HTTP fetch and the WebSocket handshake) call
    // `HttpState::learn_hsts_from_response`; a call of `HstsList::update_hsts_list_from_response`
    // anywhere else, for example one that a Servo upgrade or a merge brings back, would learn from
    // a connection that only an exception made valid.
    let mut files = Vec::new();
    crate_sources(std::path::Path::new(env!("CARGO_MANIFEST_DIR")), &mut files);
    assert!(
        files.len() > 20,
        "the scan found only {} files: {files:?}",
        files.len()
    );
    let calls = |text: &str, pattern: &str| {
        text.lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| line.contains(pattern))
            .count()
    };
    let mut guard_callers = Vec::new();
    for file in &files {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        let text = std::fs::read_to_string(file).expect("a source file");
        let direct = calls(&text, ".update_hsts_list_from_response(") +
            calls(&text, "::update_hsts_list_from_response(");
        if name == "certificate_exceptions.rs" {
            assert_eq!(direct, 1, "the guard has one call of the list's update");
        } else {
            assert_eq!(direct, 0, "{}", file.display());
        }
        if calls(&text, ".learn_hsts_from_response(") > 0 {
            guard_callers.push(name);
        }
    }
    guard_callers.sort();
    assert_eq!(
        guard_callers,
        vec!["http_loader.rs", "websocket_loader.rs"],
        "the guard is called from the HTTP fetch and from the WebSocket handshake, and nowhere else"
    );
}
