// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `view-source:` addresses, as far as the browser's own rules need them. Item 3 (engine-glue)
//! fills in the bodies.
//!
//! `mercury_view_source::ViewSourceUrl` is the engine-free crate's version of the same rule (this
//! crate depends only on `url`); both carry a comment naming the other and the same table of cases
//! in their tests.

use url::Url;

/// The scheme of a source address.
pub const VIEW_SOURCE_SCHEME: &str = "view-source";

/// `view-source:` plus the address of an `http(s)` page (credentials and fragment dropped); `None`
/// for any other page, for a `view-source:` page, and for an address over 8 KiB.
pub fn view_source_address(_page: &str) -> Option<String> {
    todo!("item 3: engine-glue")
}

/// The `http(s)` address inside a `view-source:` URL.
pub fn view_source_target(_url: &Url) -> Option<Url> {
    todo!("item 3: engine-glue")
}

/// Whether `url` has the scheme `view-source`.
pub fn is_view_source(url: &Url) -> bool {
    url.scheme() == VIEW_SOURCE_SCHEME
}
