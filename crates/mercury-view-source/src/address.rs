// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The address `view-source:<http(s) address>`.
//!
//! `talaria_core::view_source_address` mirrors the rule for pages (an http(s) page, credentials
//! and fragment dropped, at most [`crate::MAX_ADDRESS_BYTES`] for the whole address); both carry a
//! comment naming the other and the same table of cases in their tests (`tests/address.rs` here,
//! `tests/view_source.rs` there). A change to one rule is a change to both.
//!
//! The `url` crate stores everything after `view-source:` as one opaque path, so the text after
//! the scheme is parsed again as an address of its own; `Url::path` is never used for it.

use url::Url;

use crate::{MAX_ADDRESS_BYTES, SCHEME};

/// Why a text is not a `view-source:` address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    /// The text does not start with `view-source:`.
    #[error("not a view-source address")]
    NotViewSource,
    /// What follows the scheme is not an `http` or `https` address (or is another `view-source:`).
    #[error("only http and https pages have a source")]
    NotHttp,
    /// The address is longer than [`crate::MAX_ADDRESS_BYTES`] or does not parse.
    #[error("the address is not valid")]
    Invalid,
}

/// The `http` or `https` address a `view-source:` address names.
///
/// However one is made, its target has no credentials and no fragment, and the whole address
/// (`view-source:` and the target) is at most [`MAX_ADDRESS_BYTES`] long, so
/// `ViewSourceUrl::parse(x.to_url().as_str())` always gives `x` back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewSourceUrl {
    target: Url,
    /// `view-source:` and the target, made once so that `to_url` cannot fail.
    address: Url,
}

impl ViewSourceUrl {
    /// `view-source:<http(s) URL>`; the scheme is case-insensitive and the text is trimmed.
    /// Credentials and the fragment are dropped.
    pub fn parse(text: &str) -> Result<Self, UrlError> {
        let text = text.trim_matches(is_padding);
        let prefix_len = SCHEME.len() + 1;
        if !text.get(..prefix_len).is_some_and(starts_with_scheme) {
            return Err(UrlError::NotViewSource);
        }
        if text.len() > MAX_ADDRESS_BYTES {
            return Err(UrlError::Invalid);
        }
        let rest = text
            .get(prefix_len..)
            .ok_or(UrlError::Invalid)?
            .trim_start_matches(is_padding);
        if rest.get(..prefix_len).is_some_and(starts_with_scheme) {
            return Err(UrlError::NotHttp);
        }
        let target = Url::parse(rest).map_err(|_| UrlError::Invalid)?;
        if !is_web_address(&target) {
            return Err(UrlError::NotHttp);
        }
        Self::from_target(target).ok_or(UrlError::Invalid)
    }

    /// The source address of an `http(s)` page; credentials and fragment dropped.
    pub fn for_page(page: &Url) -> Option<Self> {
        if !is_web_address(page) {
            return None;
        }
        Self::from_target(page.clone())
    }

    /// The address whose source is shown.
    pub const fn target(&self) -> &Url {
        &self.target
    }

    /// `view-source:` followed by the target.
    pub fn to_url(&self) -> Url {
        self.address.clone()
    }

    /// `href` resolved against the target, as a `view-source:` address; `None` unless the result
    /// is `http` or `https` (and the address is not too long). A reference that is only a
    /// fragment resolves to the page itself, since the fragment is dropped.
    pub fn link_to(&self, href: &str) -> Option<Self> {
        let resolved = self.target.join(href).ok()?;
        if !is_web_address(&resolved) {
            return None;
        }
        Self::from_target(resolved)
    }

    /// Strips credentials and the fragment and checks the length; `None` when the address would
    /// be longer than [`MAX_ADDRESS_BYTES`] or cannot be written as a `view-source:` URL.
    fn from_target(mut target: Url) -> Option<Self> {
        // `set_username` and `set_password` only fail for URLs that cannot have credentials,
        // and an http(s) address with a host always can.
        let _ = target.set_username("");
        let _ = target.set_password(None);
        target.set_fragment(None);
        if SCHEME.len() + 1 + target.as_str().len() > MAX_ADDRESS_BYTES {
            return None;
        }
        let address = Url::parse(&format!("{SCHEME}:{}", target.as_str())).ok()?;
        Some(Self { target, address })
    }
}

/// An `http` or `https` address with a host.
fn is_web_address(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|host| !host.is_empty())
}

/// Exactly `view-source:`, in any case.
fn starts_with_scheme(text: &str) -> bool {
    text.len() == SCHEME.len() + 1
        && text.as_bytes().last() == Some(&b':')
        && text
            .get(..SCHEME.len())
            .is_some_and(|name| name.eq_ignore_ascii_case(SCHEME))
}

/// What the address bar and the URL parser both ignore around an address: white space and the
/// C0 controls.
fn is_padding(c: char) -> bool {
    c <= ' ' || c.is_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scheme_prefix_check_handles_short_and_odd_texts() {
        assert!(starts_with_scheme("view-source:"));
        assert!(starts_with_scheme("VIEW-Source:"));
        assert!(!starts_with_scheme("view-source"));
        assert!(!starts_with_scheme("view-sourcf:"));
        assert!(!starts_with_scheme("view-source;"));
        assert!(!starts_with_scheme(""));
    }

    #[test]
    fn a_multibyte_character_across_the_prefix_is_not_a_scheme() {
        // The 12th byte is inside a character: `get(..12)` is `None`, not a panic.
        assert_eq!(
            ViewSourceUrl::parse("view-sourc\u{e9}https://a/"),
            Err(UrlError::NotViewSource)
        );
        assert_eq!(ViewSourceUrl::parse("\u{e9}"), Err(UrlError::NotViewSource));
    }

    #[test]
    fn the_stored_address_is_the_scheme_and_the_target() {
        let parsed = ViewSourceUrl::parse("view-source:https://a.test/b?c=d#e").unwrap();
        assert_eq!(parsed.target().as_str(), "https://a.test/b?c=d");
        assert_eq!(parsed.to_url().as_str(), "view-source:https://a.test/b?c=d");
    }

    #[test]
    fn the_longest_address_is_accepted_and_one_byte_more_is_not() {
        let head = "view-source:https://a.test/";
        let fits = format!("{head}{}", "x".repeat(MAX_ADDRESS_BYTES - head.len()));
        assert_eq!(fits.len(), MAX_ADDRESS_BYTES);
        assert!(ViewSourceUrl::parse(&fits).is_ok());
        let too_long = format!("{fits}x");
        assert_eq!(ViewSourceUrl::parse(&too_long), Err(UrlError::Invalid));
    }
}
