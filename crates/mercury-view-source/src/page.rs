// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The page that shows a source. Item 1 (source-page) fills in the bodies; the rules are in
//! `sdd/lane-sh-pages-brief.md`, "Rules the two halves keep".

use crate::address::ViewSourceUrl;
use crate::fetch::FetchError;
use crate::source::SourceDocument;

/// A generated page: self-contained HTML, no script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewSourcePage {
    /// The document.
    pub html: String,
}

/// The page for a fetched source. Never fails: whatever cannot be highlighted is shown plain.
pub fn render_page(_document: &SourceDocument, _request: &ViewSourceUrl) -> ViewSourcePage {
    todo!("item 1: source-page")
}

/// The page for a fetch that failed (same look, one sentence from [`FetchError::summary`]).
pub fn render_error_page(_request: &ViewSourceUrl, _error: &FetchError) -> ViewSourcePage {
    todo!("item 1: source-page")
}

/// The response headers of every page: `content-type`, `content-security-policy`,
/// `cache-control: no-store`, `x-content-type-options: nosniff`, `referrer-policy: no-referrer`.
pub fn page_headers() -> [(&'static str, &'static str); 5] {
    todo!("item 1: source-page")
}
