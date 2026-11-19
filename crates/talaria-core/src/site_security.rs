// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Site security: what the address bar's security indicator shows, and the HTTPS upgrades.
//!
//! - A typed `http://` address is tried over HTTPS first (`typed_https_upgrade`); if Mercury's
//!   error page comes back for the HTTPS address, the tab loads the `http://` address instead.
//! - In HTTPS-only mode (off by default) every top-level `http` load is upgraded
//!   (`main_frame_load`, applied by the engine); if the HTTPS load fails, or the load cannot
//!   be upgraded, the tab shows an interstitial, from which the user may continue to the
//!   site over HTTP: that adds an exception for the site for the rest of the session.
//!
//! Local names and addresses (loopback, private networks, `localhost`, single-label names,
//! `.local`, `.internal`, `.home.arpa`, `.lan`) are never upgraded, unless
//! `HttpsPolicy::upgrade_local_names` is set (tests and diagnostics: they run on loopback).
//! Automatic upgrades of typed addresses apply only to the default port, as in Chrome; HTTPS-only
//! mode keeps an explicit port, as in Firefox.

use std::collections::BTreeSet;
use std::net::IpAddr;

use url::{Host, Url};

/// One of Mercury's own error pages, recognised by the engine (pages cannot fake them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorPageKind {
    /// The page could not be loaded (no connection, a refused or failed request).
    Network,
    /// The site's certificate was not accepted.
    Certificate,
}

/// What the address bar shows before the address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityIndicator {
    /// A new tab: nothing.
    None,
    /// HTTPS with a certificate the engine accepted.
    Secure,
    /// HTTPS whose certificate the user accepted although it was not valid.
    CertificateException,
    /// Mercury's certificate-error page.
    CertificateError,
    /// Plain HTTP to a site on the network.
    NotSecure,
    /// Plain HTTP to this computer (`localhost`, loopback addresses).
    LocalHost,
    /// A file on this computer.
    File,
    /// One of Mercury's own error pages (recognised by its secret marker).
    Internal,
}

impl SecurityIndicator {
    /// The accessible name and the popover's heading.
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Secure => "Connection is secure",
            Self::CertificateException => "Not secure: you allowed an invalid certificate",
            Self::CertificateError => "Not secure: the certificate is not valid",
            Self::NotSecure => "Not secure",
            Self::LocalHost => "Page on this computer",
            Self::File => "File on this computer",
            Self::Internal => "Mercury Browser page",
        }
    }

    /// Whether the indicator warns (drawn as a warning, not a padlock).
    pub const fn is_warning(self) -> bool {
        matches!(
            self,
            Self::CertificateException | Self::CertificateError | Self::NotSecure
        )
    }
}

/// The indicator for a tab showing `url`. `error_page` is the Mercury error page it shows, if
/// any; `certificate_exception` says whether the user accepted an invalid certificate for the
/// URL's host this session.
pub fn security_indicator(
    url: &str,
    error_page: Option<ErrorPageKind>,
    certificate_exception: bool,
) -> SecurityIndicator {
    if url.is_empty() {
        return SecurityIndicator::None;
    }
    match error_page {
        Some(ErrorPageKind::Certificate) => return SecurityIndicator::CertificateError,
        Some(ErrorPageKind::Network) => return SecurityIndicator::Internal,
        None => {}
    }
    let Ok(parsed) = Url::parse(url) else {
        return SecurityIndicator::NotSecure;
    };
    match parsed.scheme() {
        "https" if certificate_exception => SecurityIndicator::CertificateException,
        "https" => SecurityIndicator::Secure,
        "http" if parsed.host().is_some_and(|host| is_loopback(&host)) => {
            SecurityIndicator::LocalHost
        }
        "file" => SecurityIndicator::File,
        // A page can write into an `about:blank` pop-up and open `data:` pages, so neither is
        // ever labelled as Mercury's own; only recognised error pages are (above).
        "about" => SecurityIndicator::None,
        "data" => SecurityIndicator::NotSecure,
        "blob" => {
            let inner = parsed.path();
            security_indicator(inner, None, certificate_exception)
        }
        _ => SecurityIndicator::NotSecure,
    }
}

/// How top-level `http` loads are treated. Hosts in the exception list are matched exactly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpsPolicy {
    /// HTTPS-only mode (the "Load pages over HTTPS only" setting).
    pub https_only: bool,
    /// Treat local names and addresses like public ones, and upgrade typed addresses on any
    /// port. For tests and diagnostics only.
    pub upgrade_local_names: bool,
    exceptions: BTreeSet<String>,
}

impl HttpsPolicy {
    pub fn add_exception(&mut self, host: &str) -> bool {
        self.exceptions.insert(host.to_ascii_lowercase())
    }

    pub fn remove_exception(&mut self, host: &str) -> bool {
        self.exceptions.remove(&host.to_ascii_lowercase())
    }

    pub fn has_exception(&self, host: &str) -> bool {
        self.exceptions.contains(&host.to_ascii_lowercase())
    }

    /// The hosts allowed over HTTP in HTTPS-only mode, sorted.
    pub fn exceptions(&self) -> impl Iterator<Item = &str> {
        self.exceptions.iter().map(String::as_str)
    }

    fn may_upgrade(&self, host: &Host<&str>) -> bool {
        self.upgrade_local_names || !is_local_name(host)
    }
}

/// What to do with a top-level load of an `http` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainFrameLoad {
    Allow,
    /// Load this HTTPS URL instead.
    Upgrade(Url),
    /// Do not load it; the interstitial explains why.
    Block,
}

/// The decision for one top-level load of `url` under `policy`. `is_get` is false for a form
/// submission (its body cannot be resent elsewhere). The engine applies it through a tab's
/// `HttpsUpgradeChain`, which also stops loops.
pub fn main_frame_load(policy: &HttpsPolicy, url: &Url, is_get: bool) -> MainFrameLoad {
    if url.scheme() != "http" || !policy.https_only {
        return MainFrameLoad::Allow;
    }
    let Some(host) = url.host() else {
        return MainFrameLoad::Allow;
    };
    if policy.has_exception(&host.to_string()) || !policy.may_upgrade(&host) {
        return MainFrameLoad::Allow;
    }
    if !is_get {
        return MainFrameLoad::Block;
    }
    https_version(url, true).map_or(MainFrameLoad::Block, MainFrameLoad::Upgrade)
}

/// One tab's HTTPS-only upgrades since its last finished load. Every upgrade cancels a load and
/// starts a new navigation, so the engine's own redirect limit never stops a site whose HTTPS
/// pages send the browser back to HTTP; this does. An `http` address that was already upgraded
/// in the chain, or more than `MAX_UPGRADES` upgrades without a finished load, is blocked.
///
/// Known gap, accepted: the chain starts again at every finished load (`load_finished`). A site
/// whose HTTPS page finishes loading and then sends the tab to its own `http` address (by script
/// or a `<meta http-equiv="refresh">`) is upgraded again, and again, for as long as the site keeps
/// doing it. Any site can loop navigation like this without HTTPS-only. Each round shows a
/// finished page and the user can navigate away, so the browser does not hang. It is deliberately
/// not stopped here, and `the_chain_upgrades_the_same_link_again_after_a_finished_load` pins it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpsUpgradeChain {
    upgraded: Vec<Url>,
}

impl HttpsUpgradeChain {
    pub const MAX_UPGRADES: usize = 5;

    /// The decision for a load the engine is about to make, recording an upgrade.
    /// `is_main_frame` is false for frames, which are never upgraded.
    pub fn decide(
        &mut self,
        policy: &HttpsPolicy,
        url: &Url,
        is_get: bool,
        is_main_frame: bool,
    ) -> MainFrameLoad {
        if !is_main_frame {
            return MainFrameLoad::Allow;
        }
        match main_frame_load(policy, url, is_get) {
            MainFrameLoad::Upgrade(_)
                if self.upgraded.contains(url) || self.upgraded.len() >= Self::MAX_UPGRADES =>
            {
                MainFrameLoad::Block
            }
            MainFrameLoad::Upgrade(https) => {
                self.upgraded.push(url.clone());
                MainFrameLoad::Upgrade(https)
            }
            other => other,
        }
    }

    /// A load finished in the tab (the engine's "complete"): a new chain starts.
    pub fn load_finished(&mut self) {
        self.upgraded.clear();
    }

    /// What the browser is told about a decision for `url`.
    pub fn event_for(url: &Url, decision: &MainFrameLoad) -> Option<crate::BrowserEvent> {
        match decision {
            MainFrameLoad::Allow => None,
            MainFrameLoad::Upgrade(https) => Some(crate::BrowserEvent::HttpsUpgraded {
                from: url.to_string(),
                to: https.to_string(),
            }),
            MainFrameLoad::Block => Some(crate::BrowserEvent::HttpsOnlyBlocked {
                url: url.to_string(),
            }),
        }
    }
}

/// The HTTPS address to try first for a typed `http://` address, if any.
pub fn typed_https_upgrade(policy: &HttpsPolicy, url: &Url) -> Option<Url> {
    if url.scheme() != "http" {
        return None;
    }
    let host = url.host()?;
    if policy.has_exception(&host.to_string()) || !policy.may_upgrade(&host) {
        return None;
    }
    let keep_port = policy.https_only || policy.upgrade_local_names;
    if url.port().is_some() && !keep_port {
        return None;
    }
    https_version(url, keep_port)
}

fn https_version(url: &Url, keep_port: bool) -> Option<Url> {
    let mut upgraded = url.clone();
    let port = url.port();
    upgraded.set_scheme("https").ok()?;
    if !keep_port || port == Some(80) {
        upgraded.set_port(None).ok()?;
    }
    Some(upgraded)
}

fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => {
            let name = name.trim_end_matches('.');
            name.eq_ignore_ascii_case("localhost")
                || name.to_ascii_lowercase().ends_with(".localhost")
        }
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    }
}

/// Whether `host` (ASCII, IPv6 addresses in brackets) names this computer or the local network:
/// loopback and private addresses, `localhost`, single-label names, `.local`, `.internal`,
/// `.home.arpa`, `.lan`. Text that is not a host is not local. Every trailing dot is ignored here;
/// certificate exceptions use `certificate_exception_allowed`, which applies this rule to the
/// host in the one form exceptions are kept in.
pub fn is_local_host(host: &str) -> bool {
    match Host::parse(host) {
        Ok(Host::Domain(name)) => is_local_name(&Host::Domain(name.as_str())),
        Ok(Host::Ipv4(address)) => is_local_name(&Host::Ipv4(address)),
        Ok(Host::Ipv6(address)) => is_local_name(&Host::Ipv6(address)),
        Err(_) => false,
    }
}

/// Names and addresses that only make sense on a local network, where HTTPS is rare.
fn is_local_name(host: &Host<&str>) -> bool {
    if is_loopback(host) {
        return true;
    }
    match host {
        Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            !name.contains('.')
                || [".local", ".internal", ".home.arpa", ".lan"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        }
        Host::Ipv4(address) => is_local_address(IpAddr::V4(*address)),
        Host::Ipv6(address) => is_local_address(IpAddr::V6(*address)),
    }
}

fn is_local_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            address.is_private()
                || address.is_link_local()
                || address.is_loopback()
                || address.is_unspecified()
                || address.octets()[0] == 100 && (address.octets()[1] & 0xc0) == 64
        }
        IpAddr::V6(address) => {
            let first = address.segments()[0];
            address.is_loopback()
                || address.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
    }
}

/// A tab's site-security state, kept by the controller.
#[derive(Debug, Default)]
pub(crate) struct TabSiteSecurity {
    pub(crate) error_page: Option<(ErrorPageKind, String)>,
    pub(crate) pending_upgrade: Option<PendingUpgrade>,
    pub(crate) interstitial: Option<HttpsOnlyInterstitial>,
}

/// An HTTPS address the tab is trying instead of an `http` one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingUpgrade {
    pub(crate) https: String,
    pub(crate) http: String,
    /// HTTPS-only mode: a failure shows the interstitial instead of loading the `http` address.
    pub(crate) strict: bool,
}

/// The HTTPS-only interstitial of a tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsOnlyInterstitial {
    /// The `http` address the user asked for.
    pub http_url: String,
    /// The HTTPS address that failed, if one was tried.
    pub failed_https_url: Option<String>,
}

impl HttpsOnlyInterstitial {
    /// The host the interstitial is about, as the engine reports it (ASCII).
    pub fn host(&self) -> String {
        Url::parse(&self.http_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default()
    }
}

/// Whether two addresses name the same document (fragments aside).
pub(crate) fn same_address(first: &str, second: &str) -> bool {
    match (Url::parse(first), Url::parse(second)) {
        (Ok(mut first), Ok(mut second)) => {
            first.set_fragment(None);
            second.set_fragment(None);
            first == second
        }
        _ => first == second,
    }
}
