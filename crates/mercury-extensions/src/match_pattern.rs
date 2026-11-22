// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Match patterns for host permissions and content scripts.
//!
//! Behaviour per MDN "Match patterns" and Chrome's "Match patterns" page; written fresh, no code
//! copied. Why this is not the `webextension_pattern` crate: it has no way to ask whether one
//! pattern covers another or whether two overlap (the permission model needs both), it does not
//! model ports the way Chrome does, and it builds a regex per pattern. The lead can drop that
//! dependency from `Cargo.toml`.
//!
//! The grammar is `<scheme>://<host>[:<port>]<path>` or `<all_urls>`:
//!
//! * scheme: `*` (only `http` and `https`), or one of `http https ws wss ftp file`. Mercury's
//!   reading is the narrow one: Firefox also lets `*` match `ws` and `wss`; here a pattern must
//!   name them. `data` and `urn` are written without `//` (`data:*`, `urn:*`).
//! * `<all_urls>`: `http https ws wss ftp file`. It deliberately does not include `data` or
//!   `urn` (MDN lists `data`); those schemes can only be reached by a pattern that names them.
//! * host: `*`, `*.example.com` (the domain and its subdomains) or a complete host name or IP
//!   address; `*` may only start the host and must be followed by `.`. `*.` in front of an IP
//!   address is refused. `file` patterns have no host (`file:///path`).
//! * port: optional, `*` or a number (Chrome supports it, Firefox does not); no port means any.
//! * path: starts with `/`, `*` matches any run of characters, and it is matched, case
//!   sensitively, against the URL path plus `?query`; the fragment is never part of the match.
//!
//! Patterns are at most [`MAX_PATTERN_LEN`] bytes and a set holds at most [`MAX_PATTERNS_PER_SET`]
//! patterns. Matching never panics, and a URL longer than 64 KiB matches nothing (the glob match
//! is `O(pattern * url)`).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::{Host, Url};

/// Longest pattern accepted, in bytes.
pub const MAX_PATTERN_LEN: usize = 2048;
/// Most patterns in one [`MatchPatternSet`].
pub const MAX_PATTERNS_PER_SET: usize = 10_000;
/// A URL longer than this never matches (bounds the glob matching cost).
const MAX_URL_LEN: usize = 64 * 1024;

/// Why a pattern was refused. The messages carry no input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MatchPatternError {
    /// The pattern is empty.
    #[error("the match pattern is empty")]
    Empty,
    /// The pattern is longer than [`MAX_PATTERN_LEN`].
    #[error("the match pattern is too long")]
    TooLong,
    /// The pattern has a control character.
    #[error("the match pattern has a control character")]
    ControlCharacter,
    /// There is no `://` after the scheme.
    #[error("the match pattern has no scheme")]
    MissingScheme,
    /// The scheme is not one a pattern may name.
    #[error("the match pattern has an unsupported scheme")]
    UnsupportedScheme,
    /// The host is missing.
    #[error("the match pattern has no host")]
    MissingHost,
    /// The host is not a valid host name or address, or a host is not allowed here.
    #[error("the match pattern has an invalid host")]
    InvalidHost,
    /// `*` is somewhere in the host other than the start, or is not followed by `.`.
    #[error("the match pattern has a misplaced wildcard in the host")]
    BadHostWildcard,
    /// The port is not `*` or a number from 1 to 65535.
    #[error("the match pattern has an invalid port")]
    InvalidPort,
    /// There is no `/` path.
    #[error("the match pattern has no path")]
    MissingPath,
    /// A set would hold more than [`MAX_PATTERNS_PER_SET`] patterns.
    #[error("too many match patterns")]
    TooManyPatterns,
}

/// A URL scheme a pattern can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scheme {
    /// `http`
    Http,
    /// `https`
    Https,
    /// `ws`
    Ws,
    /// `wss`
    Wss,
    /// `ftp`
    Ftp,
    /// `file`
    File,
    /// `data`
    Data,
    /// `urn`
    Urn,
}

impl Scheme {
    fn from_name(name: &str) -> Option<Scheme> {
        Some(match name {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            "ws" => Scheme::Ws,
            "wss" => Scheme::Wss,
            "ftp" => Scheme::Ftp,
            "file" => Scheme::File,
            "data" => Scheme::Data,
            "urn" => Scheme::Urn,
            _ => return None,
        })
    }

    /// The scheme name.
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
            Scheme::Ws => "ws",
            Scheme::Wss => "wss",
            Scheme::Ftp => "ftp",
            Scheme::File => "file",
            Scheme::Data => "data",
            Scheme::Urn => "urn",
        }
    }

    fn bit(self) -> u8 {
        match self {
            Scheme::Http => 1,
            Scheme::Https => 2,
            Scheme::Ws => 4,
            Scheme::Wss => 8,
            Scheme::Ftp => 16,
            Scheme::File => 32,
            Scheme::Data => 64,
            Scheme::Urn => 128,
        }
    }

    fn opaque(self) -> bool {
        matches!(self, Scheme::Data | Scheme::Urn)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum SchemeSpec {
    /// `*`: http and https.
    Web,
    One(Scheme),
    AllUrls,
}

impl SchemeSpec {
    fn bits(self) -> u8 {
        match self {
            SchemeSpec::Web => Scheme::Http.bit() | Scheme::Https.bit(),
            SchemeSpec::One(s) => s.bit(),
            SchemeSpec::AllUrls => {
                Scheme::Http.bit()
                    | Scheme::Https.bit()
                    | Scheme::Ws.bit()
                    | Scheme::Wss.bit()
                    | Scheme::Ftp.bit()
                    | Scheme::File.bit()
            }
        }
    }
}

/// The host part of a pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HostPattern {
    /// `*`: any host.
    Any,
    /// `*.example.com`: the domain and its subdomains (the base is stored, lowercase).
    Subdomains(String),
    /// A complete host name or IP address.
    Exact(Host<String>),
    /// The scheme has no host (`file`, `data`, `urn`).
    None,
}

/// A parsed match pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MatchPattern {
    spec: SchemeSpec,
    host: HostPattern,
    port: Option<u16>,
    path: String,
}

impl MatchPattern {
    /// Parses a pattern. Never panics.
    pub fn parse(input: &str) -> Result<MatchPattern, MatchPatternError> {
        if input.is_empty() {
            return Err(MatchPatternError::Empty);
        }
        if input.len() > MAX_PATTERN_LEN {
            return Err(MatchPatternError::TooLong);
        }
        if input.chars().any(char::is_control) {
            return Err(MatchPatternError::ControlCharacter);
        }
        if input == "<all_urls>" {
            return Ok(MatchPattern {
                spec: SchemeSpec::AllUrls,
                host: HostPattern::Any,
                port: None,
                path: "/*".to_owned(),
            });
        }
        if let Some((scheme, rest)) = input.split_once(':') {
            let scheme = scheme.to_ascii_lowercase();
            if matches!(scheme.as_str(), "data" | "urn") && !rest.starts_with("//") {
                let Some(s) = Scheme::from_name(&scheme) else {
                    return Err(MatchPatternError::UnsupportedScheme);
                };
                if rest.is_empty() {
                    return Err(MatchPatternError::MissingPath);
                }
                return Ok(MatchPattern {
                    spec: SchemeSpec::One(s),
                    host: HostPattern::None,
                    port: None,
                    path: encode_path(rest),
                });
            }
        }
        let (scheme, rest) = input
            .split_once("://")
            .ok_or(MatchPatternError::MissingScheme)?;
        let spec = if scheme == "*" {
            SchemeSpec::Web
        } else {
            match Scheme::from_name(&scheme.to_ascii_lowercase()) {
                Some(s) if !s.opaque() => SchemeSpec::One(s),
                _ => return Err(MatchPatternError::UnsupportedScheme),
            }
        };
        if spec == SchemeSpec::One(Scheme::File) {
            if !rest.starts_with('/') {
                return Err(if rest.is_empty() {
                    MatchPatternError::MissingPath
                } else {
                    MatchPatternError::InvalidHost
                });
            }
            return Ok(MatchPattern {
                spec,
                host: HostPattern::None,
                port: None,
                path: encode_path(rest),
            });
        }
        let slash = rest.find('/').ok_or(MatchPatternError::MissingPath)?;
        let (authority, path) = rest.split_at(slash);
        if authority.is_empty() {
            return Err(MatchPatternError::MissingHost);
        }
        let (host_text, port_text) = split_authority(authority)?;
        let port = match port_text {
            None | Some("*") => None,
            Some(p) => Some(parse_port(p)?),
        };
        let host = parse_host(host_text)?;
        Ok(MatchPattern {
            spec,
            host,
            port,
            path: encode_path(path),
        })
    }

    /// The same pattern with the path replaced by `/*`: Chrome requires a path in a host
    /// permission but ignores it.
    #[must_use]
    pub fn with_any_path(&self) -> MatchPattern {
        let mut p = self.clone();
        if !p.spec_is_opaque() && p.spec != SchemeSpec::One(Scheme::File) {
            p.path = "/*".to_owned();
        }
        p
    }

    fn spec_is_opaque(&self) -> bool {
        matches!(self.spec, SchemeSpec::One(s) if s.opaque())
    }

    /// Whether this is the literal `<all_urls>`.
    pub fn is_all_urls(&self) -> bool {
        self.spec == SchemeSpec::AllUrls
    }

    /// Whether the pattern reaches every host over `http` or `https` (`<all_urls>`, `*://*/...`,
    /// `https://*/...`) or a whole top-level domain (`*://*.com/*`). The permission model treats
    /// these as broad access.
    pub fn is_broad(&self) -> bool {
        if self.spec.bits() & (Scheme::Http.bit() | Scheme::Https.bit()) == 0 {
            return false;
        }
        match &self.host {
            HostPattern::Any => true,
            HostPattern::Subdomains(base) => !base.contains('.'),
            _ => false,
        }
    }

    /// The schemes the pattern can match.
    pub fn schemes(&self) -> Vec<Scheme> {
        let bits = self.spec.bits();
        [
            Scheme::Http,
            Scheme::Https,
            Scheme::Ws,
            Scheme::Wss,
            Scheme::Ftp,
            Scheme::File,
            Scheme::Data,
            Scheme::Urn,
        ]
        .into_iter()
        .filter(|s| bits & s.bit() != 0)
        .collect()
    }

    /// The host part.
    pub fn host(&self) -> &HostPattern {
        &self.host
    }

    /// The port, `None` for any.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// The path glob (for `data` and `urn`, what follows the scheme).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// A short host label for display: the domain for `*.domain`, the host otherwise, `None` for
    /// patterns that reach every host or have no host.
    pub fn display_host(&self) -> Option<String> {
        match &self.host {
            HostPattern::Subdomains(base) => Some(base.clone()),
            HostPattern::Exact(h) => Some(h.to_string()),
            HostPattern::Any | HostPattern::None => None,
        }
    }

    /// Whether `url` matches. Never panics; a URL over 64 KiB matches nothing.
    pub fn matches(&self, url: &Url) -> bool {
        if url.as_str().len() > MAX_URL_LEN {
            return false;
        }
        let Some(scheme) = Scheme::from_name(url.scheme()) else {
            return false;
        };
        if self.spec.bits() & scheme.bit() == 0 {
            return false;
        }
        if scheme.opaque() {
            let whole = url.as_str();
            let without_fragment = whole.split('#').next().unwrap_or(whole);
            let rest = without_fragment
                .split_once(':')
                .map_or("", |(_, rest)| rest);
            return glob_match(self.path.as_bytes(), rest.as_bytes());
        }
        if scheme != Scheme::File {
            if !self.host_matches(url) {
                return false;
            }
            if let Some(want) = self.port
                && url.port_or_known_default() != Some(want)
            {
                return false;
            }
        }
        let mut target = String::with_capacity(url.path().len() + 1);
        target.push_str(url.path());
        if let Some(q) = url.query() {
            target.push('?');
            target.push_str(q);
        }
        glob_match(self.path.as_bytes(), target.as_bytes())
    }

    fn host_matches(&self, url: &Url) -> bool {
        let Some(host) = url.host() else {
            return false;
        };
        match (&self.host, host) {
            (HostPattern::Any, _) => true,
            (HostPattern::Subdomains(base), Host::Domain(d)) => is_same_or_subdomain(d, base),
            (HostPattern::Exact(Host::Domain(a)), Host::Domain(b)) => a == b,
            (HostPattern::Exact(Host::Ipv4(a)), Host::Ipv4(b)) => *a == b,
            (HostPattern::Exact(Host::Ipv6(a)), Host::Ipv6(b)) => *a == b,
            _ => false,
        }
    }

    /// Whether every URL `other` matches is also matched by `self`. Sound but not complete: when
    /// the answer is not obvious (for example two different globs) it says `false`.
    pub fn covers(&self, other: &MatchPattern) -> bool {
        if other.spec.bits() & !self.spec.bits() != 0 {
            return false;
        }
        host_covers(&self.host, &other.host)
            && (self.port.is_none() || self.port == other.port)
            && glob_covers(self.path.as_bytes(), other.path.as_bytes())
    }

    /// Whether the two patterns might match a common URL. Conservative: `true` when unsure.
    pub fn overlaps(&self, other: &MatchPattern) -> bool {
        self.spec.bits() & other.spec.bits() != 0
            && self.overlaps_host(other)
            && (self.port.is_none() || other.port.is_none() || self.port == other.port)
            && (self.path.contains('*') || other.path.contains('*') || self.path == other.path)
    }

    /// Whether the host parts might match a common host (schemes, ports and paths are ignored).
    pub fn overlaps_host(&self, other: &MatchPattern) -> bool {
        match (&self.host, &other.host) {
            (HostPattern::None, HostPattern::None) => true,
            (HostPattern::None, _) | (_, HostPattern::None) => false,
            (HostPattern::Any, _) | (_, HostPattern::Any) => true,
            (HostPattern::Subdomains(a), HostPattern::Subdomains(b)) => {
                is_same_or_subdomain(a, b) || is_same_or_subdomain(b, a)
            }
            (HostPattern::Subdomains(a), HostPattern::Exact(Host::Domain(b)))
            | (HostPattern::Exact(Host::Domain(b)), HostPattern::Subdomains(a)) => {
                is_same_or_subdomain(b, a)
            }
            (HostPattern::Subdomains(_), HostPattern::Exact(_))
            | (HostPattern::Exact(_), HostPattern::Subdomains(_)) => false,
            (HostPattern::Exact(a), HostPattern::Exact(b)) => a == b,
        }
    }
}

impl fmt::Display for MatchPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.spec {
            SchemeSpec::AllUrls => f.write_str("<all_urls>"),
            SchemeSpec::One(s) if s.opaque() => write!(f, "{}:{}", s.as_str(), self.path),
            SchemeSpec::One(Scheme::File) => write!(f, "file://{}", self.path),
            spec => {
                let scheme = match spec {
                    SchemeSpec::One(s) => s.as_str(),
                    _ => "*",
                };
                write!(f, "{scheme}://")?;
                match &self.host {
                    HostPattern::Any => f.write_str("*")?,
                    HostPattern::Subdomains(b) => write!(f, "*.{b}")?,
                    HostPattern::Exact(h) => write!(f, "{h}")?,
                    HostPattern::None => {}
                }
                if let Some(p) = self.port {
                    write!(f, ":{p}")?;
                }
                f.write_str(&self.path)
            }
        }
    }
}

impl std::str::FromStr for MatchPattern {
    type Err = MatchPatternError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        MatchPattern::parse(s)
    }
}

impl Serialize for MatchPattern {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for MatchPattern {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        MatchPattern::parse(&s).map_err(serde::de::Error::custom)
    }
}

fn split_authority(authority: &str) -> Result<(&str, Option<&str>), MatchPatternError> {
    if authority.starts_with('[') {
        let end = authority.find(']').ok_or(MatchPatternError::InvalidHost)?;
        let (host, after) = authority.split_at(end + 1);
        return match after {
            "" => Ok((host, None)),
            _ => match after.strip_prefix(':') {
                Some(port) => Ok((host, Some(port))),
                None => Err(MatchPatternError::InvalidHost),
            },
        };
    }
    Ok(match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    })
}

fn parse_port(text: &str) -> Result<u16, MatchPatternError> {
    if text.is_empty() || text.len() > 5 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(MatchPatternError::InvalidPort);
    }
    match text.parse::<u16>() {
        Ok(p) if p != 0 => Ok(p),
        _ => Err(MatchPatternError::InvalidPort),
    }
}

fn parse_host(text: &str) -> Result<HostPattern, MatchPatternError> {
    if text.is_empty() {
        return Err(MatchPatternError::MissingHost);
    }
    if text == "*" {
        return Ok(HostPattern::Any);
    }
    if let Some(base) = text.strip_prefix("*.") {
        if base.is_empty() {
            return Err(MatchPatternError::MissingHost);
        }
        if base.contains('*') {
            return Err(MatchPatternError::BadHostWildcard);
        }
        return match Host::parse(base) {
            Ok(Host::Domain(d)) if !d.is_empty() => Ok(HostPattern::Subdomains(d)),
            _ => Err(MatchPatternError::InvalidHost),
        };
    }
    if text.contains('*') {
        return Err(MatchPatternError::BadHostWildcard);
    }
    match Host::parse(text) {
        Ok(Host::Domain(d)) if d.is_empty() => Err(MatchPatternError::MissingHost),
        Ok(h) => Ok(HostPattern::Exact(h)),
        Err(_) => Err(MatchPatternError::InvalidHost),
    }
}

/// Percent-encodes what a URL path would have percent-encoded (spaces and non-ASCII bytes) so a
/// literal in the pattern lines up with `Url::path`.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b == b' ' || b >= 0x80 {
            out.push('%');
            out.push(hex_digit(b >> 4));
            out.push(hex_digit(b & 0x0f));
        } else {
            out.push(char::from(b));
        }
    }
    out
}

fn hex_digit(n: u8) -> char {
    char::from(if n < 10 { b'0' + n } else { b'A' + (n - 10) })
}

fn is_same_or_subdomain(host: &str, base: &str) -> bool {
    if host == base {
        return true;
    }
    host.len() > base.len()
        && host.ends_with(base)
        && host
            .as_bytes()
            .get(host.len() - base.len() - 1)
            .is_some_and(|b| *b == b'.')
}

fn host_covers(a: &HostPattern, b: &HostPattern) -> bool {
    match (a, b) {
        // `Any` with a scheme that has no host is only `<all_urls>` (file), and `covers` has
        // already checked the scheme bits.
        (HostPattern::Any, _) => true,
        (HostPattern::None, HostPattern::None) => true,
        (HostPattern::Subdomains(x), HostPattern::Subdomains(y)) => is_same_or_subdomain(y, x),
        (HostPattern::Subdomains(x), HostPattern::Exact(Host::Domain(y))) => {
            is_same_or_subdomain(y, x)
        }
        (HostPattern::Exact(x), HostPattern::Exact(y)) => x == y,
        _ => false,
    }
}

fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while t < text.len() {
        match (pattern.get(p), text.get(t)) {
            (Some(b'*'), _) => {
                star = Some(p);
                mark = t;
                p += 1;
            }
            (Some(a), Some(b)) if a == b => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some(s) => {
                    p = s + 1;
                    mark += 1;
                    t = mark;
                }
                None => return false,
            },
        }
    }
    pattern
        .get(p..)
        .is_some_and(|rest| rest.iter().all(|b| *b == b'*'))
}

/// Whether every string the glob `h` can match is also matched by the glob `g` (`*` in `h` only
/// counts as covered by a `*` in `g`).
fn glob_covers(g: &[u8], h: &[u8]) -> bool {
    if g == h || g == b"/*" || g == b"*" {
        return true;
    }
    if !g.contains(&b'*') {
        return false;
    }
    // next[j]: g[i+1..] covers h[j..]; computed from the back.
    let mut next = vec![false; h.len() + 1];
    if let Some(last) = next.last_mut() {
        *last = true;
    }
    let mut cur = vec![false; h.len() + 1];
    for i in (0..g.len()).rev() {
        let gi = g.get(i).copied().unwrap_or(b'*');
        // `next` holds the row for i + 1, except that for the row after the last character the
        // only true cell is the end of `h`.
        for j in (0..=h.len()).rev() {
            let value = if gi == b'*' {
                next.get(j).copied().unwrap_or(false)
                    || (j < h.len() && cur.get(j + 1).copied().unwrap_or(false))
            } else {
                match h.get(j) {
                    Some(hj) if *hj != b'*' && *hj == gi => {
                        next.get(j + 1).copied().unwrap_or(false)
                    }
                    _ => false,
                }
            };
            if let Some(cell) = cur.get_mut(j) {
                *cell = value;
            }
        }
        std::mem::swap(&mut next, &mut cur);
    }
    next.first().copied().unwrap_or(false)
}

/// A set of patterns, kept sorted without duplicates.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct MatchPatternSet {
    patterns: Vec<MatchPattern>,
}

impl MatchPatternSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// A set of the given patterns; more than [`MAX_PATTERNS_PER_SET`] distinct ones is an error.
    pub fn from_patterns(
        patterns: impl IntoIterator<Item = MatchPattern>,
    ) -> Result<Self, MatchPatternError> {
        let mut v: Vec<MatchPattern> = Vec::new();
        for p in patterns {
            v.push(p);
            if v.len() > MAX_PATTERNS_PER_SET.saturating_mul(2) {
                return Err(MatchPatternError::TooManyPatterns);
            }
        }
        v.sort();
        v.dedup();
        if v.len() > MAX_PATTERNS_PER_SET {
            return Err(MatchPatternError::TooManyPatterns);
        }
        Ok(MatchPatternSet { patterns: v })
    }

    /// Parses every string and collects them into a set.
    pub fn parse_all<I, S>(items: I) -> Result<Self, MatchPatternError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut v = Vec::new();
        for item in items {
            v.push(MatchPattern::parse(item.as_ref())?);
            if v.len() > MAX_PATTERNS_PER_SET.saturating_mul(2) {
                return Err(MatchPatternError::TooManyPatterns);
            }
        }
        Self::from_patterns(v)
    }

    /// Adds a pattern; `false` if it was already there.
    pub fn insert(&mut self, pattern: MatchPattern) -> bool {
        match self.patterns.binary_search(&pattern) {
            Ok(_) => false,
            Err(at) => {
                self.patterns.insert(at, pattern);
                true
            }
        }
    }

    /// Removes a pattern; `false` if it was not there.
    pub fn remove(&mut self, pattern: &MatchPattern) -> bool {
        match self.patterns.binary_search(pattern) {
            Ok(at) => {
                self.patterns.remove(at);
                true
            }
            Err(_) => false,
        }
    }

    /// Removes every member that `pattern` covers.
    pub fn remove_covered_by(&mut self, pattern: &MatchPattern) {
        self.patterns.retain(|p| !pattern.covers(p));
    }

    /// How many patterns.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Whether the set has no pattern.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// The patterns, in order.
    pub fn iter(&self) -> std::slice::Iter<'_, MatchPattern> {
        self.patterns.iter()
    }

    /// Whether any pattern matches `url`.
    pub fn matches(&self, url: &Url) -> bool {
        self.patterns.iter().any(|p| p.matches(url))
    }

    /// Whether one member covers `pattern`.
    pub fn covers(&self, pattern: &MatchPattern) -> bool {
        self.patterns.iter().any(|p| p.covers(pattern))
    }

    /// Whether every member of `other` is covered by this set.
    pub fn covers_set(&self, other: &MatchPatternSet) -> bool {
        other.patterns.iter().all(|p| self.covers(p))
    }

    /// Whether every member is covered by `other`.
    pub fn is_subset(&self, other: &MatchPatternSet) -> bool {
        other.covers_set(self)
    }

    /// Whether any member might match a URL that `pattern` also matches.
    pub fn overlaps(&self, pattern: &MatchPattern) -> bool {
        self.patterns.iter().any(|p| p.overlaps(pattern))
    }

    /// Whether any member is `<all_urls>`.
    pub fn has_all_urls(&self) -> bool {
        self.patterns.iter().any(MatchPattern::is_all_urls)
    }

    /// Whether any member is broad (see [`MatchPattern::is_broad`]).
    pub fn has_broad(&self) -> bool {
        self.patterns.iter().any(MatchPattern::is_broad)
    }

    /// The union. Both inputs are within the size cap, so the result is within twice the cap.
    #[must_use]
    pub fn union(&self, other: &MatchPatternSet) -> MatchPatternSet {
        let mut v = self.patterns.clone();
        v.extend(other.patterns.iter().cloned());
        v.sort();
        v.dedup();
        MatchPatternSet { patterns: v }
    }

    /// The members of this set that `other` does not cover.
    #[must_use]
    pub fn difference(&self, other: &MatchPatternSet) -> MatchPatternSet {
        MatchPatternSet {
            patterns: self
                .patterns
                .iter()
                .filter(|p| !other.covers(p))
                .cloned()
                .collect(),
        }
    }
}

impl<'a> IntoIterator for &'a MatchPatternSet {
    type Item = &'a MatchPattern;
    type IntoIter = std::slice::Iter<'a, MatchPattern>;

    fn into_iter(self) -> Self::IntoIter {
        self.patterns.iter()
    }
}

impl Serialize for MatchPatternSet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.patterns.iter())
    }
}

impl<'de> Deserialize<'de> for MatchPatternSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let items = Vec::<String>::deserialize(deserializer)?;
        MatchPatternSet::parse_all(items).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> MatchPattern {
        MatchPattern::parse(s).unwrap()
    }

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn display_round_trips() {
        for s in [
            "<all_urls>",
            "*://*/*",
            "https://*.example.com/a*b",
            "http://localhost:8080/x",
            "file:///tmp/*",
            "data:*",
            "https://[::1]/*",
        ] {
            assert_eq!(p(s).to_string(), s);
        }
    }

    #[test]
    fn glob_match_handles_stars_and_literals() {
        assert!(glob_match(b"/a*c", b"/abbbc"));
        assert!(glob_match(b"/a*c", b"/ac"));
        assert!(!glob_match(b"/a*c", b"/abbbd"));
        assert!(glob_match(b"*", b""));
        assert!(glob_match(b"/*/b/*/", b"/a/b/c/"));
        assert!(!glob_match(b"/a", b"/a/"));
    }

    #[test]
    fn glob_covers_is_sound_on_samples() {
        assert!(glob_covers(b"/*", b"/anything"));
        assert!(glob_covers(b"/a*", b"/ab*"));
        assert!(glob_covers(b"/a*c", b"/abc"));
        assert!(!glob_covers(b"/a*c", b"/a*"));
        assert!(!glob_covers(b"/ab", b"/a*"));
        assert!(glob_covers(b"/a*b*", b"/aXb"));
        assert!(!glob_covers(b"/a*b", b"/aXbY"));
    }

    #[test]
    fn long_url_matches_nothing() {
        let url = u(&format!("https://example.com/{}", "a".repeat(MAX_URL_LEN)));
        assert!(!p("<all_urls>").matches(&url));
    }
}
