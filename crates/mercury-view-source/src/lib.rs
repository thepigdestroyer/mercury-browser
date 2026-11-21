// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! View source for Mercury Browser, without the engine and without the interface toolkit.
//!
//! An address `view-source:<http(s) address>` names a page's source. This crate knows everything
//! about it except how the engine asks: the address rules ([`ViewSourceUrl`]), fetching the page
//! again with the engine's cookies and proxy ([`fetch_source`]), turning bytes into text
//! ([`decode`], [`classify`]), highlighting HTML, JavaScript and CSS with tree-sitter, and the page
//! that shows the result ([`render_page`], [`render_error_page`], [`page_headers`]).
//!
//! `talaria-servo` serves the page through a protocol handler for the scheme; this crate never
//! sees a Servo type.
//!
//! The page and the highlighter live in `highlight`, `page` and `style`; the address, the fetch
//! and the text handling in `address`, `fetch` and `text`.

#![deny(missing_docs)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod address;
mod fetch;
mod highlight;
mod page;
mod source;
mod style;
mod text;

pub use address::{UrlError, ViewSourceUrl};
pub use fetch::{CookieLookup, FetchError, FetchOptions, ProxyConfig, fetch_source};
pub use page::{ViewSourcePage, page_headers, render_error_page, render_page};
pub use source::{SourceBody, SourceDocument, SourceKind};
pub use text::{classify, decode};

/// The URL scheme: `view-source:https://example.com/`.
pub const SCHEME: &str = "view-source";
/// The most bytes fetched and shown. One byte more in the response means "truncated".
pub const MAX_SHOWN_BYTES: usize = 2 * 1024 * 1024;
/// Above this many bytes the text is shown without highlighting.
pub const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;
/// The most lines shown; the rest is cut with a notice.
pub const MAX_SHOWN_LINES: usize = 50_000;
/// The most `href` and `src` values turned into links.
pub const MAX_LINKS: usize = 5_000;
/// The most redirects followed.
pub const MAX_REDIRECTS: usize = 10;
/// The longest address accepted, in bytes.
pub const MAX_ADDRESS_BYTES: usize = 8 * 1024;
