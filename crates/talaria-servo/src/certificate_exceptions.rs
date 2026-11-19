// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Certificate exceptions: "Continue to the site" on Mercury's certificate page.
//!
//! Servo's own page posts the certificate with a secret to `chrome:allowcert`, but in Servo 0.8
//! that secret is a random number made separately in each process
//! (`net_traits::PRIVILEGED_SECRET`): the page is built in a content process and checked in the
//! browser process, so with content processes every request is refused (and the patched network
//! stack now refuses `chrome:allowcert` outright, see `third_party/servo-net/MERCURY.md`).
//! Mercury's page asks for `mercury-certificate-exception:allow?…` instead, carrying this run's
//! error-page secret (see `resources`). The handler runs in the browser process, where the
//! network stack keeps its certificate exceptions.
//!
//! The handler holds no policy of its own. `mercury_site_security` checks the request (the
//! secret, a well-formed host and certificate, and the local-names-only rule of
//! `talaria_core::certificate_exception_allowed`); the patched network stack then decides in
//! `HttpState::accept_failed_certificate`: an exception is bound to the pair (host,
//! certificate), only for the certificate that host actually failed with, and never for a host
//! the HSTS list covers. The page's word is not enough: a request for another certificate is
//! refused. The accepted host is recorded so the address bar warns on it.
//!
//! The answer is what the page reads: status 200 and `accepted`, or a refusal's status and
//! stable code as `text/plain` (`ExceptionRefusal::status` and `code`). Servo's fetch treats a
//! protocol handler that is fetchable as a same-origin scheme fetch (response tainting
//! "basic", `third_party/servo-net/fetch/methods.rs`, main fetch), so the page, whose origin is
//! opaque, should be able to read the answer; `Access-Control-Allow-Origin: *` is added in case
//! a later Servo applies a CORS check to it.
//! A request that does not carry the secret is not from Mercury's page and gets a network
//! error, which confirms nothing to whoever sent it. Neither the log nor the answer ever holds
//! the certificate.

use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use http::HeaderValue;
use http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_TYPE};
use mercury_site_security::{
    CERTIFICATE_EXCEPTION_SCHEME, EngineRefusal, ExceptionRefusal,
    parse_certificate_exception_request,
};
use servo::ServoUrl;
use servo::protocol_handler::{
    DoneChannel, FetchContext, HttpStatus, NetworkError, ProtocolHandler, ProtocolRegistry,
    Request, ResourceFetchTiming, Response, ResponseBody,
};
use url::Url;

use crate::resources::error_page_secret;

/// Hosts whose certificate the user accepted, waiting for the browser to take them. Each is in
/// the form of `talaria_core::certificate_exception_host`.
#[derive(Clone, Default)]
pub(crate) struct AcceptedHosts(Arc<Mutex<Vec<String>>>);

impl AcceptedHosts {
    pub(crate) fn take(&self) -> Vec<String> {
        self.0
            .lock()
            .map(|mut hosts| std::mem::take(&mut *hosts))
            .unwrap_or_default()
    }

    fn push(&self, host: String) {
        if let Ok(mut hosts) = self.0.lock() {
            hosts.push(host);
        }
    }
}

/// What the handler answers, decided apart from the network stack so that it can be tested.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// The network stack accepted the certificate for this host (in core's normal form).
    Accepted { host: String },
    /// Refused: the page reads `status` and `code`.
    Refused { status: u16, code: &'static str },
    /// The request did not carry this run's secret: not from Mercury's page.
    NotFromMercury,
}

impl Answer {
    fn refused(refusal: ExceptionRefusal) -> Self {
        Self::Refused {
            status: refusal.status(),
            code: refusal.code(),
        }
    }
}

/// A refusal the network stack gave, by its stable code (`CertificateExceptionRefusal::code`).
fn engine_refusal(code: &str) -> ExceptionRefusal {
    ExceptionRefusal::Engine(EngineRefusal::from_code(code))
}

/// Decides what to answer a request for an exception. `accept` is the network stack's
/// `HttpState::accept_failed_certificate(host, certificate)`; its refusal comes back as the
/// stable code. It is called only for a request that Mercury's page may make.
fn decide(
    url: &Url,
    secret: &str,
    accept: impl FnOnce(&str, &[u8]) -> Result<(), &'static str>,
) -> Answer {
    let exception = match parse_certificate_exception_request(url, secret) {
        Ok(exception) => exception,
        Err(ExceptionRefusal::NotFromMercury) => return Answer::NotFromMercury,
        Err(refusal) => return Answer::refused(refusal),
    };
    // The parser already gives this form; the one place that records a host asks again, so
    // that what the address bar looks up is exactly what the engine was given.
    let Some(host) = talaria_core::certificate_exception_host(&exception.host) else {
        return Answer::refused(ExceptionRefusal::Engine(EngineRefusal::InvalidHost));
    };
    match accept(&host, &exception.certificate) {
        Ok(()) => Answer::Accepted { host },
        Err(code) => Answer::refused(engine_refusal(code)),
    }
}

/// The page's answer: `status`, and `body` as plain text it may read.
fn reply(url: ServoUrl, request: &Request, status: u16, body: &str) -> Response {
    let mut response = Response::new(url, ResourceFetchTiming::new(request.timing_type()));
    let (status, reason) = match status {
        200 => (200, "OK"),
        400 => (400, "Bad Request"),
        403 => (403, "Forbidden"),
        // `HttpStatus::new_raw` panics outside 100 to 599; nothing here gives another status.
        _ => (500, "Internal Server Error"),
    };
    response.status = HttpStatus::new_raw(status, reason.as_bytes().to_vec());
    response.headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    // The certificate page's origin is opaque; see the module documentation.
    response
        .headers
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
        .headers
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    *response.body.lock() = ResponseBody::Done(body.as_bytes().to_vec());
    response
}

struct CertificateExceptionHandler {
    accepted: AcceptedHosts,
}

impl ProtocolHandler for CertificateExceptionHandler {
    fn load<'a>(
        &'a self,
        request: &'a mut Request,
        _done_chan: &mut DoneChannel,
        context: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'a>> {
        let url = request.current_url();
        let answer = decide(url.as_url(), error_page_secret(), |host, certificate| {
            context
                .state
                .accept_failed_certificate(host, certificate)
                .map_err(|refusal| refusal.code())
        });
        let response = match answer {
            Answer::Accepted { host } => {
                log::info!("accepted an invalid certificate for {host} at the user's request");
                self.accepted.push(host);
                reply(url, request, 200, "accepted")
            }
            Answer::Refused { status, code } => {
                log::warn!("refused a certificate exception: {code}");
                reply(url, request, status, code)
            }
            Answer::NotFromMercury => {
                log::warn!("refused a certificate exception: not from Mercury's page");
                Response::network_error(NetworkError::ResourceLoadError("refused".to_owned()))
            }
        };
        Box::pin(ready(response))
    }

    /// The certificate page calls it with `fetch()`, which needs this for a non-HTTP scheme.
    fn is_fetchable(&self) -> bool {
        true
    }

    /// The page is the HTTPS address's document; without this the request is mixed content.
    fn is_secure(&self) -> bool {
        true
    }
}

/// The protocols Mercury adds to Servo's.
pub(crate) fn mercury_protocols(accepted: &AcceptedHosts) -> ProtocolRegistry {
    let mut registry = ProtocolRegistry::default();
    let registered = registry.register(
        CERTIFICATE_EXCEPTION_SCHEME,
        CertificateExceptionHandler {
            accepted: accepted.clone(),
        },
    );
    if let Err(error) = registered {
        log::error!("could not register {CERTIFICATE_EXCEPTION_SCHEME}: {error:?}");
    }
    registry
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    /// A request as Mercury's page makes it: the certificate is the bytes 1, 2, 3.
    fn request(host: &str, secret: &str) -> Url {
        Url::parse(&format!(
            "{CERTIFICATE_EXCEPTION_SCHEME}:allow?secret={secret}&host={host}&certificate=AQID"
        ))
        .expect("a URL")
    }

    /// Runs `decide` with a network stack that gives `result`; returns the answer and what the
    /// network stack was asked (None if it was not asked).
    fn decided(url: &Url, result: Result<(), &'static str>) -> (Answer, Option<(String, Vec<u8>)>) {
        let asked = RefCell::new(None);
        let answer = decide(url, SECRET, |host, certificate| {
            *asked.borrow_mut() = Some((host.to_owned(), certificate.to_vec()));
            result
        });
        (answer, asked.into_inner())
    }

    #[test]
    fn the_network_stack_gets_the_host_and_the_certificate_the_page_sent() {
        let (answer, asked) = decided(&request("127.0.0.1", SECRET), Ok(()));
        assert_eq!(
            answer,
            Answer::Accepted {
                host: "127.0.0.1".to_owned()
            }
        );
        assert_eq!(asked, Some(("127.0.0.1".to_owned(), vec![1, 2, 3])));
    }

    #[test]
    fn the_accepted_host_is_in_the_form_the_address_bar_looks_up() {
        for (sent, recorded) in [
            ("Nas.Local.", "nas.local"),
            ("LOCALHOST", "localhost"),
            ("[::1]", "[::1]"),
        ] {
            let (answer, asked) = decided(&request(sent, SECRET), Ok(()));
            assert_eq!(
                answer,
                Answer::Accepted {
                    host: recorded.to_owned()
                },
                "{sent}"
            );
            assert_eq!(asked.map(|(host, _)| host).as_deref(), Some(recorded));
        }
    }

    #[test]
    fn a_request_without_this_runs_secret_is_not_from_mercury_and_asks_nothing() {
        for wrong in ["", "wrong", "0123456789abcdef0123456789abcdee"] {
            let (answer, asked) = decided(&request("127.0.0.1", wrong), Ok(()));
            assert_eq!(answer, Answer::NotFromMercury, "{wrong:?}");
            assert_eq!(asked, None, "{wrong:?}");
        }
        let (answer, asked) = decided(
            &Url::parse(&format!(
                "{CERTIFICATE_EXCEPTION_SCHEME}:allow?host=127.0.0.1"
            ))
            .unwrap(),
            Ok(()),
        );
        assert_eq!((answer, asked), (Answer::NotFromMercury, None));
    }

    #[test]
    fn a_malformed_request_is_refused_before_the_network_stack_is_asked() {
        let missing = Url::parse(&format!(
            "{CERTIFICATE_EXCEPTION_SCHEME}:allow?secret={SECRET}&host=127.0.0.1"
        ))
        .unwrap();
        let (answer, asked) = decided(&missing, Ok(()));
        assert_eq!(answer, Answer::refused(ExceptionRefusal::Malformed));
        assert!(matches!(answer, Answer::Refused { status: 400, .. }));
        assert_eq!(asked, None);
    }

    #[test]
    fn a_public_host_is_refused_by_policy_before_the_network_stack_is_asked() {
        let (answer, asked) = decided(&request("example.com", SECRET), Ok(()));
        assert!(
            matches!(answer, Answer::Refused { status: 403, .. }),
            "{answer:?}"
        );
        assert_eq!(asked, None);
    }

    #[test]
    fn every_refusal_of_the_network_stack_reaches_the_page_with_its_status_and_code() {
        // The codes `CertificateExceptionRefusal::code` gives.
        for code in [
            "invalid-host",
            "hsts-host",
            "no-failure-recorded",
            "certificate-changed",
        ] {
            let (answer, asked) = decided(&request("127.0.0.1", SECRET), Err(code));
            let expected = engine_refusal(code);
            assert!(
                !matches!(expected, ExceptionRefusal::Engine(EngineRefusal::Other)),
                "{code} is a code the page knows"
            );
            assert_eq!(answer, Answer::refused(expected), "{code}");
            assert!(
                matches!(answer, Answer::Refused { status: 403, .. }),
                "{code}"
            );
            assert!(asked.is_some(), "{code}");
        }
    }

    #[test]
    fn a_code_the_page_does_not_know_is_still_a_refusal() {
        let (answer, _) = decided(&request("127.0.0.1", SECRET), Err("something-new"));
        assert_eq!(answer, Answer::refused(engine_refusal("something-new")));
        assert!(
            matches!(answer, Answer::Refused { status: 403, .. }),
            "{answer:?}"
        );
    }
}
