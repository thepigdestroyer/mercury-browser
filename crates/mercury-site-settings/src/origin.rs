// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The origin of an `http` or `https` page, in one canonical spelling.

use std::fmt;
use std::net::Ipv4Addr;

use url::Url;

use crate::pattern::PatternError;

/// A tuple origin: scheme, host and port of an `http` or `https` page. The host is ASCII
/// (internationalised names appear in their punycode form, so a look-alike name is a different
/// origin), lower case, without a trailing dot, with brackets around an IPv6 address; the port is
/// the effective one, so `https://a.com` and `https://a.com:443` are equal.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

impl Origin {
    /// The origin of the URL written in `text`. User info, path, query and fragment are dropped.
    pub fn parse(text: &str) -> Result<Self, PatternError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(PatternError::Empty);
        }
        let url = Url::parse(text).map_err(|_| PatternError::Invalid)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(PatternError::NotHttp);
        }
        Self::from_url(&url).ok_or(PatternError::Invalid)
    }

    /// The origin of `url`, or `None` for anything but an `http` or `https` URL with a host.
    pub fn from_url(url: &Url) -> Option<Self> {
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let host = url.host_str()?.trim_end_matches('.');
        if host.is_empty() {
            return None;
        }
        Some(Self {
            scheme: url.scheme().to_owned(),
            host: host.to_owned(),
            port: url.port_or_known_default()?,
        })
    }

    /// `http` or `https`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The host as described on the type.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The effective port.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// The registrable domain of the host from the Public Suffix List (including its private
    /// section): `example.co.uk` for `www.example.co.uk`, `a.github.io` for `a.github.io`.
    /// `None` for an address, a single-label name such as `localhost`, and a public suffix.
    pub fn registrable_domain(&self) -> Option<String> {
        self.registrable_domain_str().map(str::to_owned)
    }

    pub(crate) fn registrable_domain_str(&self) -> Option<&str> {
        registrable_domain_of(&self.host)
    }

    fn default_port(&self) -> u16 {
        if self.scheme == "https" { 443 } else { 80 }
    }
}

/// Whether an ASCII host is an IP address (IPv6 keeps its brackets).
pub(crate) fn is_ip_host(host: &str) -> bool {
    host.starts_with('[') || host.parse::<Ipv4Addr>().is_ok()
}

/// The registrable domain of an ASCII host, if it has one.
pub(crate) fn registrable_domain_of(host: &str) -> Option<&str> {
    if is_ip_host(host) || !host.contains('.') {
        return None;
    }
    psl::domain_str(host)
}

impl fmt::Display for Origin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}://{}", self.scheme, self.host)?;
        if self.port != self.default_port() {
            write!(formatter, ":{}", self.port)?;
        }
        Ok(())
    }
}
