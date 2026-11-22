// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `view-source:` addresses, as far as the browser's own rules need them: which page has a
//! source address, and which `http(s)` address a source address names.
//!
//! `mercury_view_source::ViewSourceUrl` is the engine-free crate's version of the same rule (this
//! crate depends only on `url`, so it cannot use it); both carry a comment naming the other and
//! the same table of cases in their tests (`tests/view_source.rs` here, `tests/address.rs` there):
//!
//! - only an `http` or `https` address with a host has a source (no `file:`, `data:`, `blob:`,
//!   `about:`, and no `view-source:` of a `view-source:` address);
//! - credentials and the fragment are dropped (a source address never carries a password, and the
//!   fragment is not part of what is fetched);
//! - an address longer than 8 KiB has none, counted on the whole `view-source:...` text as it is
//!   handed to the engine (`mercury_view_source::MAX_ADDRESS_BYTES`);
//! - the scheme is case-insensitive (`Url` lower-cases it).

use url::Url;

/// The scheme of a source address.
pub const VIEW_SOURCE_SCHEME: &str = "view-source";

/// The longest source address, in bytes (`view-source:` included). The same limit as
/// `mercury_view_source::MAX_ADDRESS_BYTES`.
const MAX_ADDRESS_BYTES: usize = 8 * 1024;

/// `view-source:` plus the address of an `http(s)` page (credentials and fragment dropped); `None`
/// for any other page, for a `view-source:` page, and for an address over 8 KiB.
///
/// `page` is the text of the tab's address; white space around it is ignored.
pub fn view_source_address(page: &str) -> Option<String> {
    let page = Url::parse(page.trim()).ok()?;
    source_address(&web_address(&page)?)
}

/// The `http(s)` address inside a `view-source:` URL, with credentials and fragment dropped;
/// `None` for any other URL, for another scheme inside (`file:`, `view-source:`, ...), and for an
/// address over 8 KiB.
pub fn view_source_target(url: &Url) -> Option<Url> {
    // Like `ViewSourceUrl::parse`, which counts the text it is given, fragment and all.
    if !is_view_source(url) || url.as_str().len() > MAX_ADDRESS_BYTES {
        return None;
    }
    // The scheme is lower-case after parsing, so the prefix is exactly this.
    let inner = url
        .as_str()
        .strip_prefix(VIEW_SOURCE_SCHEME)?
        .strip_prefix(':')?;
    let target = web_address(&Url::parse(inner).ok()?)?;
    source_address(&target)?;
    Some(target)
}

/// `url`, a `view-source:` URL as typed, as the tab should load and show it: the address inside
/// without credentials and fragment, so that the tab's address is what is fetched. `None` where
/// [`view_source_target`] is.
pub fn view_source_normalised(url: &Url) -> Option<Url> {
    let target = view_source_target(url)?;
    Url::parse(&source_address(&target)?).ok()
}

/// Whether `url` has the scheme `view-source`.
pub fn is_view_source(url: &Url) -> bool {
    url.scheme() == VIEW_SOURCE_SCHEME
}

/// `url` as the address a source page fetches: `http` or `https` with a host, credentials and
/// fragment dropped.
fn web_address(url: &Url) -> Option<Url> {
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    let mut address = url.clone();
    // These fail only for a URL that cannot carry them; an `http(s)` URL with a host can.
    let _ = address.set_username("");
    let _ = address.set_password(None);
    address.set_fragment(None);
    Some(address)
}

/// `view-source:` followed by `target`, if that is not longer than the limit.
fn source_address(target: &Url) -> Option<String> {
    let address = format!("{VIEW_SOURCE_SCHEME}:{target}");
    (address.len() <= MAX_ADDRESS_BYTES).then_some(address)
}
