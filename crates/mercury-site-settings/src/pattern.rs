// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What a rule is about: one origin, a site, a host and its subdomains, or every site.

use std::fmt;

use url::{Host, Url};

use crate::origin::{Origin, registrable_domain_of};

/// Why a pattern or an origin could not be made from text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternError {
    /// The text is empty.
    Empty,
    /// The scheme is not `http` or `https`.
    NotHttp,
    /// The text is not a pattern or an origin.
    Invalid,
    /// An origin pattern carries a path, query or fragment.
    HasPath,
    /// A wildcard or site pattern would cover a public suffix (`*.co.uk`, `*.github.io`).
    PublicSuffix,
    /// A wildcard or site pattern names an IP address; use an exact origin.
    IpAddress,
    /// A wildcard or site pattern names a single-label host (`localhost`, `com`); use an exact
    /// origin.
    SingleLabel,
    /// A site pattern names a host below its registrable domain (`site:www.example.com`).
    NotASite,
}

impl fmt::Display for PatternError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "empty",
            Self::NotHttp => "only http and https sites can have rules",
            Self::Invalid => "not a site pattern",
            Self::HasPath => "a pattern names a site, not a page",
            Self::PublicSuffix => "a pattern cannot cover a public suffix",
            Self::IpAddress => "a pattern cannot cover addresses; use one origin",
            Self::SingleLabel => "a pattern cannot name a single-label host; use one origin",
            Self::NotASite => "a site is a registrable domain",
        })
    }
}

impl std::error::Error for PatternError {}

/// How closely a pattern names an origin it matches; a more specific rule wins.
///
/// `Subdomains(n)` carries the number of labels of its host, so `*.x.shop.a.com` outranks
/// `*.shop.a.com`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Specificity {
    /// Every site: the user's default for an axis.
    AllSites,
    /// A registrable domain.
    Site,
    /// A host and its subdomains, with this many labels in the host.
    Subdomains(usize),
    /// One origin.
    Origin,
}

/// What a rule is about. Written as `*`, `site:example.com`, `*.shop.example.com` or
/// `https://example.com`; [`Display`](fmt::Display) gives the one canonical spelling and `parse`
/// reads it back. Ordered for listing: every site, sites, wildcards, then origins.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OriginPattern {
    /// Every site: the user's default for an axis.
    AllSites,
    /// A registrable domain: any subdomain, scheme and port.
    Site(String),
    /// A host that is not a registrable domain, and every host below it; any scheme and port.
    Subdomains(String),
    /// One origin.
    Origin(Origin),
}

impl OriginPattern {
    /// Reads a pattern from its text; see the type.
    pub fn parse(text: &str) -> Result<Self, PatternError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(PatternError::Empty);
        }
        if text == "*" {
            return Ok(Self::AllSites);
        }
        if let Some(domain) = text.strip_prefix("site:") {
            return Self::site(domain);
        }
        if let Some(host) = text.strip_prefix("*.") {
            return Self::subdomains(host);
        }
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            let url = Url::parse(text).map_err(|_| PatternError::Invalid)?;
            if !url.username().is_empty() || url.password().is_some() {
                return Err(PatternError::Invalid);
            }
            if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
                return Err(PatternError::HasPath);
            }
            return Origin::from_url(&url)
                .map(Self::Origin)
                .ok_or(PatternError::Invalid);
        }
        if text.contains("://") {
            return Err(PatternError::NotHttp);
        }
        Err(PatternError::Invalid)
    }

    /// The registrable domain `domain` and all that is below it. `domain` must itself be a
    /// registrable domain: `example.com`, `example.co.uk`, `user.github.io`.
    pub fn site(domain: &str) -> Result<Self, PatternError> {
        let host = pattern_host(domain)?;
        if registrable_domain_of(&host) != Some(host.as_str()) {
            return Err(PatternError::NotASite);
        }
        Ok(Self::Site(host))
    }

    /// `host` and every host below it. A host that is a registrable domain gives the same pattern
    /// as [`site`](Self::site).
    pub fn subdomains(host: &str) -> Result<Self, PatternError> {
        let host = pattern_host(host)?;
        if registrable_domain_of(&host) == Some(host.as_str()) {
            Ok(Self::Site(host))
        } else {
            Ok(Self::Subdomains(host))
        }
    }

    /// The pattern for exactly `origin`.
    pub fn for_origin(origin: &Origin) -> Self {
        Self::Origin(origin.clone())
    }

    /// The pattern for the whole site of `origin` (its registrable domain), or why it has none.
    pub fn for_site_of(origin: &Origin) -> Result<Self, PatternError> {
        match origin.registrable_domain_str() {
            Some(domain) => Ok(Self::Site(domain.to_owned())),
            None => Err(pattern_host(origin.host())
                .err()
                .unwrap_or(PatternError::PublicSuffix)),
        }
    }

    /// How closely this pattern names `origin`, or `None` when it does not cover it.
    pub fn matches(&self, origin: &Origin) -> Option<Specificity> {
        match self {
            Self::AllSites => Some(Specificity::AllSites),
            Self::Origin(own) => (own == origin).then_some(Specificity::Origin),
            Self::Site(domain) => (origin.registrable_domain_str() == Some(domain.as_str()))
                .then_some(Specificity::Site),
            Self::Subdomains(host) => {
                let covered = origin.host() == host
                    || origin
                        .host()
                        .strip_suffix(host.as_str())
                        .is_some_and(|prefix| prefix.ends_with('.'));
                covered.then(|| Specificity::Subdomains(host.split('.').count()))
            }
        }
    }
}

/// A host a wildcard or site pattern may name: a domain with at least two labels that has a
/// registrable domain (so it is not a public suffix), as lower-case ASCII.
fn pattern_host(text: &str) -> Result<String, PatternError> {
    if text.is_empty() {
        return Err(PatternError::Invalid);
    }
    let domain = match Host::parse(text).map_err(|_| PatternError::Invalid)? {
        Host::Domain(domain) => domain,
        Host::Ipv4(_) | Host::Ipv6(_) => return Err(PatternError::IpAddress),
    };
    let domain = domain.trim_end_matches('.');
    if domain.is_empty() || domain.split('.').any(str::is_empty) {
        return Err(PatternError::Invalid);
    }
    if !domain.contains('.') {
        return Err(PatternError::SingleLabel);
    }
    if psl::domain_str(domain).is_none() {
        return Err(PatternError::PublicSuffix);
    }
    Ok(domain.to_owned())
}

impl fmt::Display for OriginPattern {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllSites => formatter.write_str("*"),
            Self::Site(domain) => write!(formatter, "site:{domain}"),
            Self::Subdomains(host) => write!(formatter, "*.{host}"),
            Self::Origin(origin) => write!(formatter, "{origin}"),
        }
    }
}
