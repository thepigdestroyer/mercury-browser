// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the fetch gives and the page shows. Frozen by the lane lead.

use url::Url;

/// What a response holds, as far as the page is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// `text/html`, `application/xhtml+xml`: highlighted with the HTML grammar (and its script and
    /// style injections).
    Html,
    /// `text/javascript`, `application/javascript` and the other JavaScript types.
    JavaScript,
    /// `text/css`.
    Css,
    /// `application/xml`, `text/xml`, `image/svg+xml`: the HTML grammar, which tolerates them.
    Xml,
    /// Any other text (JSON, plain text): shown without highlighting.
    Plain,
    /// Images, audio, video, fonts, PDF, octet streams, or text with a NUL byte near the start:
    /// the body is not shown.
    Binary,
}

/// The body of a fetched source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceBody {
    /// Text, decoded (lossily; never fails).
    Text {
        /// The text, at most `MAX_SHOWN_BYTES` of the response.
        text: String,
        /// The bytes of the response that were read (at most `MAX_SHOWN_BYTES`).
        bytes: u64,
        /// The response had more than `MAX_SHOWN_BYTES`.
        truncated: bool,
        /// The name of the encoding used (`encoding_rs` name, for example `UTF-8`).
        charset: &'static str,
    },
    /// Not shown; only counted.
    Binary {
        /// The bytes counted (up to the cap).
        bytes: u64,
    },
}

/// A page fetched again for `view-source:`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDocument {
    /// The address after redirects.
    pub final_url: Url,
    /// The HTTP status of the final response.
    pub status: u16,
    /// The `Content-Type` header as sent.
    pub content_type: Option<String>,
    /// What `classify` made of it.
    pub kind: SourceKind,
    /// The body.
    pub body: SourceBody,
}
