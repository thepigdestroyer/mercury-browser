// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `urlFilter` and `regexFilter`: the two ways a rule matches a request URL.
//!
//! Behaviour per <https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest>
//! (`RuleCondition.urlFilter`, `RuleCondition.regexFilter`) and Chromium's
//! `indexed_rule.cc` (BSD-3-Clause; read for the rules of what is valid, nothing is copied).
//!
//! # `urlFilter`
//!
//! A filter is an optional left anchor, a pattern, and an optional right anchor:
//!
//! * `||` at the start is the domain anchor: the match must begin at the start of the URL's host or just
//!   after a `.` inside it, so `||example.com` matches `https://example.com/` and
//!   `https://a.example.com/` but not `https://notexample.com/`. Nothing stops the match at the end of a
//!   label: `||a.example.com` also matches `https://a.example.company`, as Chrome documents.
//! * `|` at the start anchors to the start of the URL, `|` at the end to its end.
//! * `*` matches any run of characters, `^` matches one separator (anything except a letter, a digit,
//!   `_`, `-`, `.` or `%`) or the end of the URL. A `|` anywhere else is a literal character.
//! * Matching is case-insensitive unless the rule says otherwise (ASCII case folding).
//! * The target is the URL as the network stack serialises it (scheme, host in punycode, path, query),
//!   **without the fragment**.
//!
//! Invalid, as in Chrome: an empty filter, non-ASCII text, and a pattern that starts with `||*`.
//! Mercury also refuses a domain anchor with an empty pattern (`||`) and ASCII control characters.
//!
//! # `regexFilter` and how it differs from RE2
//!
//! Chrome compiles `regexFilter` with RE2; Mercury uses the [`regex`] crate (decision D2), which has the
//! same linear-time guarantee and the same unsupported features (no backreferences, no look-around: a
//! rule that uses them is invalid), but the syntax is not identical:
//!
//! * `\C` and `\Q...\E` are RE2 syntax that the regex crate rejects; such a rule is invalid here.
//! * Unicode is on: `\d`, `\w`, `\s` and `\b` are Unicode-aware, RE2's are ASCII. Request URLs are ASCII
//!   (non-ASCII characters are percent-encoded and hosts are punycode), so a match against a URL cannot
//!   tell the difference.
//! * Flags: `(?i)`, `(?s)`, `(?m)`, `(?x)`, `(?U)` exist in both; RE2's `(?P<name>...)` and the regex
//!   crate's `(?<name>...)` both work.
//! * Size: Chrome refuses a pattern that compiles to more than 2 KiB of program; Mercury refuses more than
//!   [`Limits::regex_size_limit`] (default 2 MiB) of compiled size and [`Limits::regex_dfa_size_limit`] of
//!   lazy-DFA cache, and a pattern of more than [`Limits::max_filter_bytes`] bytes. This is looser than
//!   Chrome in size; no behaviour that works in Chrome is refused by it.
//! * The pattern must be ASCII, as in Chrome. It is searched anywhere in the URL (RE2's
//!   `PartialMatch`), not anchored, and is case-insensitive unless the rule says otherwise.

use std::ops::Range;

use regex::{Regex, RegexBuilder};
use thiserror::Error;
use url::{Position, Url};

use crate::limits::Limits;

/// Why a filter was refused. No variant carries the filter text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FilterError {
    /// The filter is the empty string.
    #[error("the filter is empty")]
    Empty,
    /// The filter is longer than the limit.
    #[error("the filter is {found} bytes (limit {limit})")]
    TooLong {
        /// The limit in bytes.
        limit: usize,
        /// The length found.
        found: usize,
    },
    /// The filter has a non-ASCII character.
    #[error("the filter must be ASCII")]
    NonAscii,
    /// The filter has an ASCII control character.
    #[error("the filter has a control character")]
    ControlCharacter,
    /// A `urlFilter` of `||` with nothing after it.
    #[error("a domain anchor needs a pattern after it")]
    DomainAnchorEmpty,
    /// A `urlFilter` that starts with `||*`.
    #[error("a pattern after the domain anchor cannot start with a wildcard")]
    DomainAnchorWildcard,
    /// The regex compiles to more than the size limit.
    #[error("the regular expression is too large")]
    RegexTooLarge,
    /// The regex does not compile (syntax error, or a feature RE2 and this engine lack).
    #[error("the regular expression is not supported")]
    RegexUnsupported,
}

/// The URL a filter is matched against: the serialised URL without its fragment, and where its host is.
#[derive(Debug, Clone, Copy)]
pub struct UrlTarget<'a> {
    text: &'a str,
    host: (usize, usize),
}

impl<'a> UrlTarget<'a> {
    /// The target of a parsed URL.
    pub fn new(url: &'a Url) -> Self {
        let full = url.as_str();
        let text = match full.find('#') {
            Some(i) => full.get(..i).unwrap_or(full),
            None => full,
        };
        let start = url[..Position::BeforeHost].len();
        let end = url[..Position::AfterHost].len().min(text.len());
        UrlTarget {
            text,
            host: (start.min(end), end),
        }
    }

    /// A target from plain text with no known host: `||` filters never match it.
    pub fn from_text(text: &'a str) -> Self {
        UrlTarget { text, host: (0, 0) }
    }

    /// A target from text and the byte range of its host (for tests and the index).
    pub fn from_parts(text: &'a str, host: Range<usize>) -> Self {
        UrlTarget {
            text,
            host: (host.start, host.end),
        }
    }

    /// The text that filters see.
    pub fn text(&self) -> &'a str {
        self.text
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    None,
    Start,
    Domain,
}

/// One element of a compiled pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Piece {
    /// Literal bytes (lowercased when the filter is case-insensitive).
    Literal(Vec<u8>),
    /// `^`.
    Separator,
    /// `*`.
    Wildcard,
}

/// A compiled `urlFilter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlFilter {
    source: String,
    case_sensitive: bool,
    anchor: Anchor,
    end_anchor: bool,
    pieces: Vec<Piece>,
}

/// Whether `b` is a separator for `^`: not a letter, digit, `_`, `-`, `.` or `%`.
pub(crate) fn is_separator(b: u8) -> bool {
    !(b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'%'))
}

impl UrlFilter {
    /// Compiles a `urlFilter`. `max_len` is [`Limits::max_filter_bytes`].
    pub fn parse(pattern: &str, case_sensitive: bool, max_len: usize) -> Result<Self, FilterError> {
        if pattern.is_empty() {
            return Err(FilterError::Empty);
        }
        if pattern.len() > max_len {
            return Err(FilterError::TooLong {
                limit: max_len,
                found: pattern.len(),
            });
        }
        if !pattern.is_ascii() {
            return Err(FilterError::NonAscii);
        }
        if pattern.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return Err(FilterError::ControlCharacter);
        }
        let bytes = pattern.as_bytes();
        let (anchor, rest) = if let Some(rest) = bytes.strip_prefix(b"||") {
            (Anchor::Domain, rest)
        } else if let Some(rest) = bytes.strip_prefix(b"|") {
            (Anchor::Start, rest)
        } else {
            (Anchor::None, bytes)
        };
        let (body, end_anchor) = match rest.split_last() {
            Some((b'|', body)) => (body, true),
            _ => (rest, false),
        };
        if anchor == Anchor::Domain {
            if body.is_empty() {
                return Err(FilterError::DomainAnchorEmpty);
            }
            if body.first() == Some(&b'*') {
                return Err(FilterError::DomainAnchorWildcard);
            }
        }
        let mut pieces: Vec<Piece> = Vec::new();
        let mut literal: Vec<u8> = Vec::new();
        for &b in body {
            match b {
                b'*' | b'^' => {
                    if !literal.is_empty() {
                        pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                    }
                    if b == b'*' {
                        if pieces.last() != Some(&Piece::Wildcard) {
                            pieces.push(Piece::Wildcard);
                        }
                    } else {
                        pieces.push(Piece::Separator);
                    }
                }
                _ => literal.push(if case_sensitive {
                    b
                } else {
                    b.to_ascii_lowercase()
                }),
            }
        }
        if !literal.is_empty() {
            pieces.push(Piece::Literal(literal));
        }
        Ok(UrlFilter {
            source: pattern.to_owned(),
            case_sensitive,
            anchor,
            end_anchor,
            pieces,
        })
    }

    /// The filter as written in the rule.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether letter case matters.
    pub fn is_case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    /// Whether the filter starts with `||`.
    pub fn is_domain_anchored(&self) -> bool {
        self.anchor == Anchor::Domain
    }

    /// Whether the filter starts with `|` or `||`.
    pub(crate) fn has_left_anchor(&self) -> bool {
        self.anchor != Anchor::None
    }

    /// Whether the filter ends with `|`.
    pub(crate) fn has_end_anchor(&self) -> bool {
        self.end_anchor
    }

    /// The compiled pieces, for the index.
    pub(crate) fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// Whether the filter matches a parsed URL (the fragment is not looked at).
    pub fn matches_url(&self, url: &Url) -> bool {
        self.matches(&UrlTarget::new(url))
    }

    /// Whether the filter matches the target.
    pub fn matches(&self, target: &UrlTarget<'_>) -> bool {
        let s = target.text.as_bytes();
        match self.anchor {
            Anchor::Start => self.run(s, 0, false),
            Anchor::None => self.run(s, 0, true),
            Anchor::Domain => {
                let (host_start, host_end) = target.host;
                if host_end <= host_start {
                    return false;
                }
                (host_start..host_end).any(|p| {
                    let at_boundary = p == host_start
                        || p.checked_sub(1)
                            .and_then(|i| s.get(i))
                            .is_some_and(|b| *b == b'.');
                    at_boundary && self.run(s, p, false)
                })
            }
        }
    }

    /// Matches the pieces from `start`. With `leading_star` the match may begin anywhere at or after
    /// `start`. Greedy matching with backtracking to the last wildcard: O(len * pieces).
    fn run(&self, s: &[u8], start: usize, leading_star: bool) -> bool {
        let pieces = &self.pieces;
        let end_anchored = self.end_anchor && !matches!(pieces.last(), Some(Piece::Wildcard));
        let mut idx = 0usize;
        let mut pos = start;
        let mut star: Option<(usize, usize)> = if leading_star { Some((0, start)) } else { None };
        loop {
            match pieces.get(idx) {
                None => {
                    if !end_anchored || pos == s.len() {
                        return true;
                    }
                }
                Some(Piece::Wildcard) => {
                    idx += 1;
                    star = Some((idx, pos));
                    continue;
                }
                Some(Piece::Literal(lit)) => {
                    if self.literal_at(s, pos, lit) {
                        pos += lit.len();
                        idx += 1;
                        continue;
                    }
                }
                Some(Piece::Separator) => match s.get(pos) {
                    Some(&c) if is_separator(c) => {
                        pos += 1;
                        idx += 1;
                        continue;
                    }
                    None => {
                        idx += 1;
                        continue;
                    }
                    Some(_) => {}
                },
            }
            match star {
                Some((next_idx, star_pos)) if star_pos < s.len() => {
                    let resume = star_pos + 1;
                    star = Some((next_idx, resume));
                    idx = next_idx;
                    pos = resume;
                }
                _ => return false,
            }
        }
    }

    fn literal_at(&self, s: &[u8], pos: usize, lit: &[u8]) -> bool {
        let Some(end) = pos.checked_add(lit.len()) else {
            return false;
        };
        match s.get(pos..end) {
            Some(window) if self.case_sensitive => window == lit,
            Some(window) => window.eq_ignore_ascii_case(lit),
            None => false,
        }
    }
}

/// A compiled `regexFilter`.
#[derive(Debug, Clone)]
pub struct RegexFilter {
    regex: Regex,
    case_sensitive: bool,
}

impl PartialEq for RegexFilter {
    fn eq(&self, other: &Self) -> bool {
        self.case_sensitive == other.case_sensitive && self.regex.as_str() == other.regex.as_str()
    }
}

impl Eq for RegexFilter {}

impl RegexFilter {
    /// Compiles a `regexFilter` with the size limits of `limits`.
    pub fn compile(
        pattern: &str,
        case_sensitive: bool,
        limits: &Limits,
    ) -> Result<Self, FilterError> {
        if pattern.is_empty() {
            return Err(FilterError::Empty);
        }
        if pattern.len() > limits.max_filter_bytes {
            return Err(FilterError::TooLong {
                limit: limits.max_filter_bytes,
                found: pattern.len(),
            });
        }
        if !pattern.is_ascii() {
            return Err(FilterError::NonAscii);
        }
        let built = RegexBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .size_limit(limits.regex_size_limit)
            .dfa_size_limit(limits.regex_dfa_size_limit)
            .build();
        match built {
            Ok(regex) => Ok(RegexFilter {
                regex,
                case_sensitive,
            }),
            Err(regex::Error::CompiledTooBig(_)) => Err(FilterError::RegexTooLarge),
            Err(_) => Err(FilterError::RegexUnsupported),
        }
    }

    /// The pattern as written in the rule.
    pub fn source(&self) -> &str {
        self.regex.as_str()
    }

    /// Whether letter case matters.
    pub fn is_case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    /// The compiled regex (for `regexSubstitution`).
    pub fn regex(&self) -> &Regex {
        &self.regex
    }

    /// Whether the pattern is found anywhere in the target.
    pub fn is_match(&self, target: &UrlTarget<'_>) -> bool {
        self.regex.is_match(target.text)
    }
}

/// The URL condition of a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlPattern {
    /// A `urlFilter`.
    Filter(UrlFilter),
    /// A `regexFilter`.
    Regex(RegexFilter),
}

impl UrlPattern {
    /// Whether the pattern matches the target.
    pub fn matches(&self, target: &UrlTarget<'_>) -> bool {
        match self {
            UrlPattern::Filter(f) => f.matches(target),
            UrlPattern::Regex(r) => r.is_match(target),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(pattern: &str) -> UrlFilter {
        UrlFilter::parse(pattern, false, 2048).expect("valid filter")
    }

    fn m(pattern: &str, url: &str) -> bool {
        filter(pattern).matches_url(&Url::parse(url).expect("url"))
    }

    fn m_cs(pattern: &str, url: &str) -> bool {
        UrlFilter::parse(pattern, true, 2048)
            .expect("valid filter")
            .matches_url(&Url::parse(url).expect("url"))
    }

    // --- Chrome's urlFilter documentation table ---------------------------------------------------

    #[test]
    fn doc_table_abc_substring() {
        assert!(m("abc", "https://abcd.com"));
        assert!(m("abc", "https://example.com/abcd"));
        assert!(!m("abc", "https://ab.com"));
    }

    #[test]
    fn doc_table_abc_star_d() {
        assert!(m("abc*d", "https://abcd.com"));
        assert!(m("abc*d", "https://example.com/abcxyzd"));
        assert!(!m("abc*d", "https://abc.com"));
    }

    #[test]
    fn doc_table_domain_anchor() {
        assert!(m("||a.example.com", "https://a.example.com/"));
        assert!(m("||a.example.com", "https://b.a.example.com/xyz"));
        assert!(m("||a.example.com", "https://a.example.company"));
        assert!(!m("||a.example.com", "https://example.com/"));
    }

    #[test]
    fn doc_table_start_anchor_with_wildcard() {
        assert!(m("|https*", "https://example.com"));
        assert!(!m("|https*", "http://example.com/"));
        assert!(!m("|https*", "http://https.com"));
    }

    #[test]
    fn doc_table_separator_and_end_anchor() {
        assert!(m("example*^123|", "https://example.com/123"));
        assert!(m("example*^123|", "http://abc.com/example?123"));
        assert!(!m("example*^123|", "https://example.com/1234"));
        assert!(!m("example*^123|", "https://abc.com/example0123"));
    }

    // --- the "write good URL conditions" cautions -------------------------------------------------

    #[test]
    fn bare_domain_text_matches_in_query_strings_as_documented() {
        assert!(m("google.com", "https://example.com/?param=google.com"));
    }

    #[test]
    fn domain_anchor_without_trailing_slash_matches_longer_hosts_as_documented() {
        assert!(m("||google.com", "https://google.company"));
        assert!(!m("||google.com/", "https://google.company/"));
    }

    #[test]
    fn domain_anchor_with_slash_matches_all_subdomains_and_paths() {
        assert!(m("||google.com/", "https://google.com/"));
        assert!(m("||google.com/", "https://www.google.com/search?q=1"));
        assert!(!m(
            "||google.com/",
            "https://example.com/?param=google.com/"
        ));
    }

    #[test]
    fn start_anchor_with_scheme_matches_only_that_prefix() {
        assert!(m("|https://www.google.com/", "https://www.google.com/x"));
        assert!(!m(
            "|https://www.google.com/",
            "https://sub.www.google.com/"
        ));
        assert!(!m(
            "|https://www.google.com/",
            "https://example.com/?param=https://www.google.com/"
        ));
    }

    // --- anchors -----------------------------------------------------------------------------------

    #[test]
    fn domain_anchor_needs_a_label_boundary() {
        assert!(!m("||example.com", "https://notexample.com/"));
        assert!(m("||example.com", "https://sub.example.com/"));
        assert!(!m("||example.com", "https://evil.test/?u=example.com"));
        assert!(!m("||example.com", "https://evil.test/example.com"));
    }

    #[test]
    fn domain_anchor_does_not_look_into_userinfo() {
        assert!(!m("||example.com", "https://example.com@evil.test/"));
        assert!(m("||evil.test", "https://example.com@evil.test/"));
    }

    #[test]
    fn domain_anchor_never_matches_a_hostless_url() {
        assert!(!m("||example.com", "data:text/plain,example.com"));
        assert!(!m("||a", "mailto:a@b.c"));
    }

    #[test]
    fn domain_anchor_with_end_anchor() {
        assert!(m("||example.com/|", "https://example.com/"));
        assert!(!m("||example.com/|", "https://example.com/x"));
    }

    #[test]
    fn domain_anchor_matches_ip_hosts_at_the_start() {
        assert!(m("||127.0.0.1", "http://127.0.0.1:8080/x"));
    }

    #[test]
    fn end_anchor_requires_the_end() {
        assert!(m("|https://a.test/x|", "https://a.test/x"));
        assert!(!m("|https://a.test/x|", "https://a.test/xy"));
        assert!(m(".js|", "https://a.test/app.js"));
        assert!(!m(".js|", "https://a.test/app.js?v=1"));
    }

    #[test]
    fn start_anchor_alone_matches_everything() {
        assert!(m("|", "https://a.test/"));
    }

    #[test]
    fn interior_pipe_is_a_literal() {
        let f = filter("a|b");
        assert!(f.matches(&UrlTarget::from_text("https://x.test/a|b")));
        assert!(!f.matches(&UrlTarget::from_text("https://x.test/ab")));
    }

    // --- wildcard and separator --------------------------------------------------------------------

    #[test]
    fn wildcard_alone_matches_everything() {
        assert!(m("*", "https://a.test/"));
    }

    #[test]
    fn ads_path_segment_pattern() {
        assert!(m("*/ads/*", "https://a.test/ads/x.js"));
        assert!(m("*/ads/*", "https://a.test/static/ads/"));
        assert!(!m("*/ads/*", "https://a.test/adsx"));
        assert!(!m("*/ads/*", "https://a.test/ads"));
    }

    #[test]
    fn separator_matches_the_end_of_the_url() {
        assert!(m("example^", "https://a.test/example"));
        assert!(m("example^", "https://a.test/example/"));
        assert!(m("example^", "https://a.test/example?x=1"));
        assert!(m("example^", "https://a.test/example:8080"));
    }

    #[test]
    fn separator_does_not_match_dot_dash_underscore_percent_or_alphanumerics() {
        assert!(!m("example^", "https://a.test/example.js"));
        assert!(!m("example^", "https://a.test/example-1"));
        assert!(!m("example^", "https://a.test/example_1"));
        assert!(!m("example^", "https://a.test/example%20"));
        assert!(!m("example^", "https://a.test/examples"));
        assert!(!m("example^", "https://a.test/example1"));
    }

    #[test]
    fn separator_between_literals_needs_a_real_separator() {
        assert!(m("a^b", "https://x.test/a/b"));
        assert!(m("a^b", "https://x.test/a?b"));
        assert!(!m("a^b", "https://x.test/ab"));
        assert!(!m("a^b", "https://x.test/a.b"));
    }

    #[test]
    fn consecutive_wildcards_collapse() {
        assert_eq!(filter("a**b").pieces().len(), 3);
        assert!(m("a**b", "https://x.test/a123b"));
    }

    #[test]
    fn wildcard_backtracking_finds_later_matches() {
        assert!(m("a*b*c", "https://x.test/abxbxc"));
        assert!(m("*b*", "https://x.test/aab"));
        assert!(!m("a*b*c", "https://x.test/acb"));
    }

    // --- case --------------------------------------------------------------------------------------

    #[test]
    fn matching_is_case_insensitive_by_default() {
        assert!(m("ABC", "https://x.test/abc"));
        assert!(m("abc", "https://x.test/ABC"));
        assert!(m("||EXAMPLE.com", "https://example.com/"));
    }

    #[test]
    fn case_sensitive_filters_respect_case() {
        assert!(!m_cs("ABC", "https://x.test/abc"));
        assert!(m_cs("ABC", "https://x.test/ABC"));
        assert!(!m_cs("abc", "https://x.test/ABC"));
    }

    #[test]
    fn host_is_lowercased_by_the_url_so_case_sensitive_host_filters_see_lowercase() {
        assert!(m_cs("||example.com", "https://EXAMPLE.com/"));
        assert!(!m_cs("||EXAMPLE.com", "https://example.com/"));
    }

    // --- target ------------------------------------------------------------------------------------

    #[test]
    fn the_fragment_is_not_matched() {
        assert!(!m("frag", "https://a.test/#frag"));
        assert!(m("frag", "https://a.test/?frag#x"));
        assert!(m("a.test/|", "https://a.test/#frag"));
    }

    #[test]
    fn the_target_is_the_canonical_url() {
        assert!(m("||xn--p1ai", "http://abc.рф/?q=ф"));
        assert!(m("q=%D1%84", "http://abc.рф/?q=ф"));
    }

    #[test]
    fn default_ports_are_not_in_the_target() {
        assert!(!m(":443", "https://a.test:443/"));
        assert!(m(":8443", "https://a.test:8443/"));
    }

    // --- parsing -----------------------------------------------------------------------------------

    #[test]
    fn empty_filter_is_refused() {
        assert_eq!(UrlFilter::parse("", false, 2048), Err(FilterError::Empty));
    }

    #[test]
    fn domain_anchor_alone_is_refused() {
        assert_eq!(
            UrlFilter::parse("||", false, 2048),
            Err(FilterError::DomainAnchorEmpty)
        );
        assert_eq!(
            UrlFilter::parse("|||", false, 2048),
            Err(FilterError::DomainAnchorEmpty)
        );
    }

    #[test]
    fn domain_anchor_followed_by_wildcard_is_refused() {
        assert_eq!(
            UrlFilter::parse("||*abc", false, 2048),
            Err(FilterError::DomainAnchorWildcard)
        );
        assert!(UrlFilter::parse("|*abc", false, 2048).is_ok());
    }

    #[test]
    fn non_ascii_filter_is_refused() {
        assert_eq!(
            UrlFilter::parse("пример", false, 2048),
            Err(FilterError::NonAscii)
        );
    }

    #[test]
    fn control_characters_are_refused() {
        assert_eq!(
            UrlFilter::parse("a\0b", false, 2048),
            Err(FilterError::ControlCharacter)
        );
        assert_eq!(
            UrlFilter::parse("a\nb", false, 2048),
            Err(FilterError::ControlCharacter)
        );
    }

    #[test]
    fn filter_length_edge() {
        let at = "a".repeat(2048);
        assert!(UrlFilter::parse(&at, false, 2048).is_ok());
        let over = "a".repeat(2049);
        assert_eq!(
            UrlFilter::parse(&over, false, 2048),
            Err(FilterError::TooLong {
                limit: 2048,
                found: 2049
            })
        );
    }

    #[test]
    fn long_wildcard_filters_do_not_blow_up() {
        let pattern = "a*".repeat(1000);
        let f = UrlFilter::parse(&pattern, false, 4096).expect("valid");
        let url = format!("https://x.test/{}", "b".repeat(5000));
        assert!(!f.matches(&UrlTarget::from_text(&url)));
    }

    #[test]
    fn source_and_flags_are_kept() {
        let f = UrlFilter::parse("||Ex.com^", true, 2048).expect("valid");
        assert_eq!(f.source(), "||Ex.com^");
        assert!(f.is_case_sensitive());
        assert!(f.is_domain_anchored());
    }

    // --- regexFilter -------------------------------------------------------------------------------

    fn rx(pattern: &str) -> Result<RegexFilter, FilterError> {
        RegexFilter::compile(pattern, false, &Limits::default())
    }

    #[test]
    fn regex_is_unanchored_and_case_insensitive_by_default() {
        let f = rx(r"/ADS/").expect("valid");
        assert!(f.is_match(&UrlTarget::from_text("https://a.test/x/ads/y")));
        assert!(!f.is_match(&UrlTarget::from_text("https://a.test/x/adz/y")));
    }

    #[test]
    fn regex_case_sensitive_flag_is_honoured() {
        let f = RegexFilter::compile(r"/ADS/", true, &Limits::default()).expect("valid");
        assert!(!f.is_match(&UrlTarget::from_text("https://a.test/x/ads/y")));
        assert!(f.is_match(&UrlTarget::from_text("https://a.test/x/ADS/y")));
    }

    #[test]
    fn regex_anchors_work() {
        let f = rx(r"^https://www\.(abc|def)\.xyz\.com/").expect("valid");
        assert!(f.is_match(&UrlTarget::from_text("https://www.abc.xyz.com/path")));
        assert!(!f.is_match(&UrlTarget::from_text("http://www.abc.xyz.com/path")));
    }

    #[test]
    fn regex_look_around_is_refused() {
        assert_eq!(rx(r"(?=a)b"), Err(FilterError::RegexUnsupported));
        assert_eq!(rx(r"(?<=a)b"), Err(FilterError::RegexUnsupported));
        assert_eq!(rx(r"(?!a)b"), Err(FilterError::RegexUnsupported));
    }

    #[test]
    fn regex_backreferences_are_refused() {
        assert_eq!(rx(r"(a)\1"), Err(FilterError::RegexUnsupported));
    }

    #[test]
    fn regex_re2_only_syntax_is_refused() {
        assert_eq!(rx(r"\Qa.b\E"), Err(FilterError::RegexUnsupported));
        assert_eq!(rx(r"\C"), Err(FilterError::RegexUnsupported));
    }

    #[test]
    fn regex_syntax_errors_are_refused_without_panic() {
        for bad in ["(", ")", "[", "a{2,1}", "*a", "\\", "(?P<n", "[z-a]"] {
            assert_eq!(rx(bad), Err(FilterError::RegexUnsupported), "{bad}");
        }
    }

    #[test]
    fn regex_too_large_is_refused() {
        assert_eq!(rx(r"((a{100}){100}){100}"), Err(FilterError::RegexTooLarge));
    }

    #[test]
    fn regex_size_limit_is_configurable() {
        let limits = Limits {
            regex_size_limit: 100_000,
            ..Limits::default()
        };
        assert_eq!(
            RegexFilter::compile(r"\pL{20}", false, &limits),
            Err(FilterError::RegexTooLarge)
        );
        assert!(RegexFilter::compile(r"a{20}", false, &limits).is_ok());
    }

    #[test]
    fn regex_empty_and_non_ascii_are_refused() {
        assert_eq!(rx(""), Err(FilterError::Empty));
        assert_eq!(rx("é"), Err(FilterError::NonAscii));
    }

    #[test]
    fn regex_length_edge() {
        let at = "a".repeat(2048);
        assert!(rx(&at).is_ok());
        let over = "a".repeat(2049);
        assert_eq!(
            rx(&over),
            Err(FilterError::TooLong {
                limit: 2048,
                found: 2049
            })
        );
    }

    #[test]
    fn regex_matching_is_linear_on_pathological_input() {
        let f = rx(r"(a+)+$").expect("valid");
        let url = format!("https://a.test/{}b", "a".repeat(20_000));
        assert!(!f.is_match(&UrlTarget::from_text(&url)));
    }

    #[test]
    fn url_pattern_dispatches() {
        let f = UrlPattern::Filter(filter("||a.test/"));
        let url = Url::parse("https://a.test/x").expect("url");
        assert!(f.matches(&UrlTarget::new(&url)));
        let r = UrlPattern::Regex(rx(r"a\.test").expect("valid"));
        assert!(r.matches(&UrlTarget::new(&url)));
    }
}
