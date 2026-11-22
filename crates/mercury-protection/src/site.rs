// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A site: the key per-site protections are filed under.

use std::net::Ipv6Addr;

use url::{Host, Url};

/// A site: the scheme-less registrable domain in lower case. An IP address or a single-label host is its own
/// site; an IPv6 address keeps its brackets. This is the key the engine's network policy uses
/// (`NetPolicyControl::site_of`), so wherever the engine hands over a key, pass it to [`Site::from_key`] instead
/// of computing one: one site must never be two strings.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Site(String);

impl Site {
    /// The longest site key kept (the longest host name, in bytes).
    pub const MAX_LEN: usize = 253;

    /// A site from the key the engine uses. Trims and lower-cases; `None` for an empty or over-long key.
    pub fn from_key(key: &str) -> Option<Site> {
        let key = key.trim().to_ascii_lowercase();
        (!key.is_empty() && key.len() <= Self::MAX_LEN).then_some(Site(key))
    }

    /// The site of a URL; `None` for a URL with no host (`data:`, `about:`, `file:`).
    pub fn of_url(url: &Url) -> Option<Site> {
        Self::of_parsed_host(&url.host()?)
    }

    /// The site of a host written as text. An IPv6 address may be written with or without brackets.
    pub fn of_host(host: &str) -> Option<Site> {
        let host = host.trim();
        if host.contains(':') && !host.starts_with('[') {
            let address: Ipv6Addr = host.parse().ok()?;
            return Self::from_key(&format!("[{address}]"));
        }
        Self::of_parsed_host(&Host::parse(host).ok()?)
    }

    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn of_parsed_host<S: AsRef<str>>(host: &Host<S>) -> Option<Site> {
        match host {
            Host::Domain(name) => {
                let name = name.as_ref().trim_end_matches('.').to_ascii_lowercase();
                if name.is_empty() {
                    return None;
                }
                let key = psl::domain_str(&name).unwrap_or(&name);
                Self::from_key(key)
            }
            Host::Ipv4(address) => Self::from_key(&address.to_string()),
            Host::Ipv6(address) => Self::from_key(&format!("[{address}]")),
        }
    }
}

impl std::fmt::Display for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
