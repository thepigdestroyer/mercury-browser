// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Certificate exceptions: "Continue to the site" on Mercury's certificate page.
//!
//! Servo's own page posts the certificate with a secret to `chrome:allowcert`, but in Servo 0.8
//! that secret is a random number made separately in each process
//! (`net_traits::PRIVILEGED_SECRET`): the page is built in a content process and checked in the
//! browser process, so with content processes every request is refused. Mercury's page asks
//! for `mercury-certificate-exception:allow?…` instead, carrying this run's error-page secret
//! (see `resources`). The handler runs in the browser process, where the network stack keeps
//! its list of accepted certificates, and adds the certificate there.
//!
//! Servo accepts an exception for the certificate wherever it is presented, not only for the
//! host it was accepted for; the host is recorded so the address bar can warn on that host.

use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mercury_site_security::{CERTIFICATE_EXCEPTION_SCHEME, parse_certificate_exception_request};
use rustls::pki_types::CertificateDer;
use servo::protocol_handler::{
    DoneChannel, FetchContext, HttpStatus, NetworkError, ProtocolHandler, ProtocolRegistry,
    Request, ResourceFetchTiming, Response,
};

use crate::resources::error_page_secret;

/// Hosts whose certificate the user accepted, waiting for the browser to take them.
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
        let response = match parse_certificate_exception_request(url.as_url(), error_page_secret())
        {
            Some(exception) => {
                context
                    .state
                    .override_manager
                    .add_override(&CertificateDer::from(exception.certificate));
                log::info!(
                    "accepted an invalid certificate for {} at the user's request",
                    exception.host
                );
                self.accepted.push(exception.host);
                let mut response =
                    Response::new(url, ResourceFetchTiming::new(request.timing_type()));
                response.status = HttpStatus::default();
                response
            }
            None => {
                log::warn!("refused a certificate exception that did not come from Mercury");
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
