// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: certificate exceptions bound to the pair (host, certificate), failing closed. This is a new
// Mercury file; nothing in it is copied from Servo or another project. The hooks are in `connector.rs`,
// `lib.rs` and `fetch/methods.rs`; `MERCURY.md` lists them.

//! Certificate exceptions bound to a host name and a certificate.
//!
//! Servo 0.8 keeps one list of certificates that the user chose to trust anyway, and accepts any
//! certificate on that list for **every** host name: whoever holds the key of a certificate the user once
//! accepted for `nas.local` could present it as `bank.example` and the connection would verify. Mercury
//! replaces the list with a set of pairs `(host name, certificate)`:
//!
//! * the verifier accepts a certificate that fails normal verification only if the exact pair is in the set;
//!   the signature checks stay with the main verifier, so the server must still hold the certificate's key;
//! * the only way to add a pair from a user decision is [`HttpState::accept_failed_certificate`], which
//!   accepts only the certificate the host **actually failed with** (a bounded record of the last failure of
//!   each host), never one that the caller supplies on its own, and never for a host the HSTS list covers;
//! * everything fails closed: an unparsable host, an evicted record, a changed certificate and an HSTS host
//!   are refusals with a stable [code](CertificateExceptionRefusal::code), and no refusal changes the set.
//!
//! The key has no port and no error kind: rustls hands the verifier a name, not a port, and the error kind
//! adds nothing against someone who holds the accepted certificate's key.
//!
//! Host forms. [`server_name_for_host`] is the one place that turns a host string into the name both the
//! failure record and the exceptions use. It mirrors `talaria_core::certificate_exception_host` in the
//! Mercury application (which keeps the brackets of an IPv6 literal, as `url::Url::host_str()` does); the
//! tests of both carry the same table of cases.
//!
//! Revoking ([`HttpState::revoke_certificate_exception`]) affects new connections only. Connections that are
//! already open (HTTP/1 keep-alive, HTTP/2) stay authenticated until they close, because `hyper` offers no
//! per-host eviction of its pool. That is why Mercury offers no "remove exception" button yet.
//!
//! A new connection must also make a full handshake. rustls skips `verify_server_cert` when it resumes a
//! TLS session, so a ticket stored while the exception was in force would let the host in again after a
//! revoke. Each client's session cache is therefore created through the override manager
//! (`session_resumption`), which keeps a handle so that a revoke can empty the host's tickets.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::net::Ipv6Addr;
use std::sync::{Arc, Weak};

use aws_lc_rs::digest;
use rustls::client::{ClientSessionMemoryCache, ClientSessionStore, Resumption};
use rustls_pki_types::{CertificateDer, ServerName};

use crate::connector::CertificateErrorOverrideManager;
use crate::http_loader::HttpState;

/// How many hosts keep their last certificate failure for [`HttpState::accept_failed_certificate`]. When
/// more hosts fail, the oldest record is dropped, and accepting for that host is refused with
/// [`CertificateExceptionRefusal::NoFailureRecorded`] until the page is loaded again (fail closed).
pub const MAX_RECORDED_FAILURES: usize = 64;

/// The number of hosts whose TLS sessions one client remembers: what `Resumption::default()` uses.
const SESSION_CACHE_SIZE: usize = 256;

/// Why a certificate exception was not added. Nothing is added when one of these is returned.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CertificateExceptionRefusal {
    /// The host is not a DNS name or an IP address.
    InvalidHost,
    /// The host is covered by the HSTS list (learned or preloaded): browsers never let a user click
    /// through a certificate error there.
    HstsHost,
    /// The host has no recorded certificate failure (never failed, already accepted, or evicted).
    NoFailureRecorded,
    /// The certificate offered is not the one the host last failed with.
    CertificateChanged,
}

impl CertificateExceptionRefusal {
    /// A stable text for the refusal: `invalid-host`, `hsts-host`, `no-failure-recorded` or
    /// `certificate-changed`. Mercury's protocol handler sends it to its certificate page.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidHost => "invalid-host",
            Self::HstsHost => "hsts-host",
            Self::NoFailureRecorded => "no-failure-recorded",
            Self::CertificateChanged => "certificate-changed",
        }
    }
}

impl fmt::Display for CertificateExceptionRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "certificate exception refused: {}", self.code())
    }
}

impl std::error::Error for CertificateExceptionRefusal {}

/// One accepted exception, as [`HttpState::certificate_exceptions`] lists it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CertificateExceptionEntry {
    /// The host name: lower case, no trailing dot, an IPv6 literal in brackets (the form
    /// `url::Url::host_str()` gives, and the form `revoke_certificate_exception` takes back).
    pub host: String,
    /// The SHA-256 of the DER of the accepted certificate.
    pub sha256: [u8; 32],
}

/// The one name both the failure record and the exceptions use. Trims the brackets around an IPv6 literal,
/// lower-cases, drops one trailing dot, then `ServerName::try_from`. `None` for anything that does not
/// parse as a DNS name or an IP address; for brackets around anything that is not an IPv6 address; for
/// an IPv6 address without brackets (a host is never written that way in a URL, and `Uri::host()` keeps
/// the brackets); and for a name that still ends in a dot after the first one is dropped.
///
/// Mirrors `talaria_core::certificate_exception_host` in the Mercury application, which keeps the brackets.
pub fn server_name_for_host(host: &str) -> Option<ServerName<'static>> {
    let host = host.to_ascii_lowercase();
    let bare = match host.strip_prefix('[') {
        Some(rest) => {
            let inner = rest.strip_suffix(']')?;
            inner.parse::<Ipv6Addr>().ok()?;
            inner
        },
        None => {
            let name = host.strip_suffix('.').unwrap_or(&host);
            if name.ends_with('.') || name.contains(':') {
                return None;
            }
            name
        },
    };
    ServerName::try_from(bare.to_owned()).ok()
}

/// The text form of a name for [`CertificateExceptionEntry::host`].
fn host_text(server_name: &ServerName<'_>) -> String {
    let text = server_name.to_str().to_ascii_lowercase();
    if text.contains(':') {
        format!("[{text}]")
    } else {
        text
    }
}

/// The pairs the user accepted and the last certificate failure of each recent host. It lives inside the
/// [`CertificateErrorOverrideManager`]'s lock, next to Servo's own map of pending failures.
#[derive(Clone, Debug, Default)]
pub(crate) struct CertificateExceptionStore {
    /// The accepted pairs. The only thing the verifier consults.
    exceptions: HashSet<(ServerName<'static>, CertificateDer<'static>)>,
    /// The certificate each host last failed with, oldest first, at most [`MAX_RECORDED_FAILURES`] hosts.
    recorded_failures: VecDeque<(ServerName<'static>, CertificateDer<'static>)>,
    /// The TLS session cache of every client built with this manager that is still alive (see
    /// `session_resumption`). Weak: `create_tls_config` is also called for every WebSocket connection,
    /// and the cache must go away with its `ClientConfig`.
    session_caches: Vec<Weak<ClientSessionMemoryCache>>,
}

impl CertificateExceptionStore {
    /// Whether the exact pair was accepted. Another host name, another certificate, a subdomain or a
    /// parent domain of an accepted host do not match.
    pub(crate) fn contains(&self, name: &ServerName<'_>, certificate: &CertificateDer<'_>) -> bool {
        !self.exceptions.is_empty() &&
            self.exceptions
                .contains(&(name.to_owned(), certificate.clone().into_owned()))
    }

    /// Adds the pair without any check of a recorded failure.
    pub(crate) fn insert(
        &mut self,
        name: ServerName<'static>,
        certificate: CertificateDer<'static>,
    ) {
        self.exceptions.insert((name, certificate));
    }

    /// Remembers that `name` failed with `certificate`, replacing the older record of the same host and
    /// dropping the oldest host when there are more than [`MAX_RECORDED_FAILURES`].
    pub(crate) fn record_failure(
        &mut self,
        name: ServerName<'static>,
        certificate: CertificateDer<'static>,
    ) {
        self.recorded_failures
            .retain(|(recorded, _)| *recorded != name);
        self.recorded_failures.push_back((name, certificate));
        while self.recorded_failures.len() > MAX_RECORDED_FAILURES {
            self.recorded_failures.pop_front();
        }
    }

    /// Turns the recorded failure of `name` into an exception, if it is for `certificate`. A successful
    /// call consumes the record; a refused one leaves everything as it was.
    pub(crate) fn accept_recorded_failure(
        &mut self,
        name: &ServerName<'static>,
        certificate: &[u8],
    ) -> Result<(), CertificateExceptionRefusal> {
        let position = self
            .recorded_failures
            .iter()
            .position(|(recorded, _)| recorded == name)
            .ok_or(CertificateExceptionRefusal::NoFailureRecorded)?;
        if self.recorded_failures[position].1.as_ref() != certificate {
            return Err(CertificateExceptionRefusal::CertificateChanged);
        }
        if let Some(pair) = self.recorded_failures.remove(position) {
            self.exceptions.insert(pair);
        }
        Ok(())
    }

    /// Forgets every exception of `name`; returns how many. When there was one, the TLS sessions of the
    /// host are forgotten too, so that the next connection makes a full handshake and is verified again.
    pub(crate) fn revoke(&mut self, name: &ServerName<'static>) -> usize {
        let before = self.exceptions.len();
        self.exceptions.retain(|(accepted, _)| accepted != name);
        let revoked = before - self.exceptions.len();
        if revoked > 0 {
            for cache in self.session_caches.iter().filter_map(Weak::upgrade) {
                cache.remove_tls12_session(name);
                while cache.take_tls13_ticket(name).is_some() {}
            }
        }
        revoked
    }

    /// Creates the session cache of one client and remembers it for `revoke`. The size is rustls' own
    /// default (`Resumption::default()`), and each client keeps a cache of its own, as before.
    pub(crate) fn new_session_cache(&mut self) -> Resumption {
        let cache = Arc::new(ClientSessionMemoryCache::new(SESSION_CACHE_SIZE));
        self.session_caches.retain(|cache| cache.strong_count() > 0);
        self.session_caches.push(Arc::downgrade(&cache));
        Resumption::store(cache)
    }

    /// Every accepted pair, sorted by host and then certificate hash.
    pub(crate) fn entries(&self) -> Vec<CertificateExceptionEntry> {
        let mut entries: Vec<_> = self
            .exceptions
            .iter()
            .map(|(name, certificate)| CertificateExceptionEntry {
                host: host_text(name),
                sha256: sha256(certificate.as_ref()),
            })
            .collect();
        entries.sort();
        entries
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let hash = digest::digest(&digest::SHA256, bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(hash.as_ref());
    out
}

impl CertificateErrorOverrideManager {
    /// For tests and trusted embedders: accepts `certificate` for `host` and nothing else, with no check
    /// of a recorded failure. A user decision must go through [`HttpState::accept_failed_certificate`].
    /// The old one-argument form, which accepted the certificate for every host, no longer exists.
    ///
    /// Only [`CertificateExceptionRefusal::InvalidHost`] is returned.
    pub fn add_override(
        &self,
        host: &str,
        certificate: &CertificateDer<'static>,
    ) -> Result<(), CertificateExceptionRefusal> {
        let name = server_name_for_host(host).ok_or(CertificateExceptionRefusal::InvalidHost)?;
        self.lock().exceptions.insert(name, certificate.clone());
        Ok(())
    }

    /// The TLS resumption setting for one client's `ClientConfig`, a session cache that a revoke can
    /// empty (see the module documentation).
    pub(crate) fn session_resumption(&self) -> Resumption {
        self.lock().exceptions.new_session_cache()
    }

    /// Whether the verifier should accept `certificate` for `name` although it failed verification.
    pub(crate) fn is_accepted(
        &self,
        name: &ServerName<'_>,
        certificate: &CertificateDer<'_>,
    ) -> bool {
        self.lock().exceptions.contains(name, certificate)
    }

    /// Test helper: how many session caches the manager keeps a handle to (live clients, and at most
    /// one dead one that the next registration drops).
    #[cfg(feature = "test-util")]
    pub fn session_cache_handles_for_test(&self) -> usize {
        self.lock().exceptions.session_caches.len()
    }

    /// Test helper: whether the exact pair is accepted.
    #[cfg(feature = "test-util")]
    pub fn has_exception(&self, host: &str, certificate: &CertificateDer<'static>) -> bool {
        server_name_for_host(host).is_some_and(|name| self.is_accepted(&name, certificate))
    }

    /// Test helper: does what the verifier does when a certificate fails verification, without a
    /// network: notes `certificate` as the pending failure of `host`.
    #[cfg(feature = "test-util")]
    pub fn note_certificate_failure_for_test(
        &self,
        host: &str,
        certificate: &CertificateDer<'static>,
    ) {
        if let Some(name) = server_name_for_host(host) {
            self.lock()
                .certificates_failing_to_verify
                .insert(name, certificate.clone());
        }
    }

    /// Test helper: does what `http_loader.rs` does when it builds the error page of a failed
    /// request: reports (and so removes) the pending failure of `host`, which also moves a copy to
    /// the bounded record. Returns the certificate that was pending, if any.
    #[cfg(feature = "test-util")]
    pub fn report_certificate_failure_for_test(
        &self,
        host: &str,
    ) -> Option<CertificateDer<'static>> {
        self.remove_certificate_failing_verification(host)
    }
}

impl HttpState {
    /// The only way a user decision becomes a certificate exception. `certificate` is the DER the user was
    /// shown. The exception is added only if the host failed verification with exactly that certificate
    /// (so a caller that was tricked into sending another certificate cannot add it), and not for a host
    /// the HSTS list covers.
    ///
    /// Refusals, in this order: unparsable `host` -> [`InvalidHost`](CertificateExceptionRefusal::InvalidHost);
    /// HSTS (learned or preloaded) -> [`HstsHost`](CertificateExceptionRefusal::HstsHost); no recorded
    /// failure -> [`NoFailureRecorded`](CertificateExceptionRefusal::NoFailureRecorded); a different
    /// certificate -> [`CertificateChanged`](CertificateExceptionRefusal::CertificateChanged). On success
    /// the pair is added and the host's failure is cleared from both maps. There is no time limit: the
    /// exception is for the certificate the user saw, not for the server's current one.
    pub fn accept_failed_certificate(
        &self,
        host: &str,
        certificate: &[u8],
    ) -> Result<(), CertificateExceptionRefusal> {
        let name = server_name_for_host(host).ok_or(CertificateExceptionRefusal::InvalidHost)?;
        // HSTS never applies to an IP address (`HstsEntry::new` refuses one).
        if let ServerName::DnsName(dns_name) = &name {
            if self.hsts_list.read().is_host_secure(dns_name.as_ref()) {
                return Err(CertificateExceptionRefusal::HstsHost);
            }
        }
        let mut manager = self.override_manager.lock();
        manager
            .exceptions
            .accept_recorded_failure(&name, certificate)?;
        manager.certificates_failing_to_verify.remove(&name);
        Ok(())
    }

    /// Forgets every exception of `host`, and the TLS sessions of the host, so that a new connection is
    /// verified again; returns how many exceptions it forgot. This affects new connections only:
    /// connections that are already open (HTTP keep-alive, HTTP/2) stay authenticated until they close.
    /// An unparsable host revokes nothing.
    pub fn revoke_certificate_exception(&self, host: &str) -> usize {
        match server_name_for_host(host) {
            Some(name) => self.override_manager.lock().exceptions.revoke(&name),
            None => 0,
        }
    }

    /// Every certificate exception, sorted by host and then certificate hash.
    pub fn certificate_exceptions(&self) -> Vec<CertificateExceptionEntry> {
        self.override_manager.lock().exceptions.entries()
    }
}
