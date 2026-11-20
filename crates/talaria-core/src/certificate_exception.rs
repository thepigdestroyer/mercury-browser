// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Certificate exceptions: which hosts may get one, and the one spelling of a host that every
//! part of Mercury uses for them.
//!
//! When the user continues past an invalid certificate, the engine accepts that certificate for
//! that host only, for the rest of the session (the pair of host and certificate; the vendored
//! `servo-net`, `certificate_exceptions.rs`). Mercury keeps its own record of the hosts, to warn
//! in the address bar (`BrowserController::has_certificate_exception`). The browser, the request
//! parser (`mercury-site-security`), the engine handler and the indicator all name the host
//! through `certificate_exception_host`, so "Nas.Local." (a typed address, a link, the host of the
//! certificate page) and "nas.local" are one host.
//!
//! The engine spells the host with `server_name_for_host` (in `servo-net`), which also drops the
//! brackets of an IPv6 literal. Both are tested against the same table of cases (`HOST_FORMS` in
//! `tests/certificate_exceptions.rs` here, and in the engine's own tests). A host Mercury names
//! that the engine does not accept is refused there (`invalid-host`), so a difference between the
//! two can make a request fail, never accept more.

use url::Host;

use crate::site_security::is_local_host;

/// The spelling of `host` that certificate exceptions use, or None when no exception can name
/// it.
///
/// It is the form `url::Url::host_str` gives for the address of the page: ASCII (names with
/// other letters in their `xn--` form), lower-case, an IPv4 or IPv6 literal in its canonical
/// serialisation (an IPv6 literal in brackets, `[::1]`), and one trailing dot dropped (a fully
/// qualified name, `nas.local.`, is the host `nas.local`). None for text that is not a host
/// (`url::Host::parse` rejects it: empty, spaces, a port or user information, an IPv6 address
/// without its brackets) and for a name with an empty label (`nas..`, `a..b`).
pub fn certificate_exception_host(host: &str) -> Option<String> {
    match Host::parse(host).ok()? {
        Host::Domain(name) => {
            let name = name.strip_suffix('.').unwrap_or(&name);
            // `Host::parse` allows empty labels; a name with one is not a host that a
            // certificate was ever presented for.
            name.split('.')
                .all(|label| !label.is_empty())
                .then(|| name.to_owned())
        }
        // The serialisation of `Host` is the one `Url::host_str` has, brackets included.
        address @ (Host::Ipv4(_) | Host::Ipv6(_)) => Some(address.to_string()),
    }
}

/// Whether `host` may get an invalid certificate accepted: it is a host an exception can name
/// (`certificate_exception_host`) and it names this computer or the local network
/// (`is_local_host`, applied to that form).
///
/// This is Mercury's policy, not a limit of the engine, which binds an exception to the host and
/// the certificate the user was shown, and refuses hosts that asked to be reached only with a
/// valid certificate (HSTS). The policy limits what a click-through can expose: a wrong decision
/// about a public site costs far more than one about a printer or a test server, and Mercury has
/// no way yet to trust a certificate authority of the user's own (a company's, for example), which
/// is the proper answer for a site whose certificate is only unknown to this computer.
///
/// What lifts the gate is a product decision (open point O1 of the certificate-exceptions
/// lane): make this function allow every host the engine does not refuse, change the certificate
/// page's `isLocalName` and the tables that pin both, and rewrite the paragraph of the user guide
/// that says which sites the page offers "Continue anyway" for.
pub fn certificate_exception_allowed(host: &str) -> bool {
    certificate_exception_host(host).is_some_and(|host| is_local_host(&host))
}
