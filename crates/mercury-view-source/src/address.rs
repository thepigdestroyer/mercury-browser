// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The address `view-source:<http(s) address>`. Item 2 (source-fetch) fills in the bodies.
//!
//! `talaria_core::view_source_address` mirrors the rule for pages (an http(s) page, credentials
//! and fragment dropped, at most [`crate::MAX_ADDRESS_BYTES`]); both carry a comment naming the
//! other and the same table of cases in their tests.

use url::Url;

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewSourceUrl {
    target: Url,
}

impl ViewSourceUrl {
    /// `view-source:<http(s) URL>`; the scheme is case-insensitive and the text is trimmed.
    /// Credentials and the fragment are dropped.
    pub fn parse(_text: &str) -> Result<Self, UrlError> {
        todo!("item 2: source-fetch")
    }

    /// The source address of an `http(s)` page; credentials and fragment dropped.
    pub fn for_page(_page: &Url) -> Option<Self> {
        todo!("item 2: source-fetch")
    }

    /// The address whose source is shown.
    pub const fn target(&self) -> &Url {
        &self.target
    }

    /// `view-source:` followed by the target.
    pub fn to_url(&self) -> Url {
        todo!("item 2: source-fetch")
    }

    /// `href` resolved against the target, as a `view-source:` address; `None` unless the result
    /// is `http` or `https`.
    pub fn link_to(&self, _href: &str) -> Option<Self> {
        todo!("item 2: source-fetch")
    }
}
