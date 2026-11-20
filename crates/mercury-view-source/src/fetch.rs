// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Fetching a page again, with the engine's cookies and proxy. Item 2 (source-fetch) fills in the
//! bodies; the rules are in `sdd/lane-sh-pages-brief.md`, "Rules the two halves keep".

use std::sync::Arc;
use std::time::Duration;

use url::Url;

use crate::address::ViewSourceUrl;
use crate::source::SourceDocument;

/// The `Cookie` header text for a URL, asked for every hop of a redirect chain; `None` sends no
/// cookie. Set-Cookie of a response is never stored.
pub type CookieLookup = Arc<dyn Fn(&Url) -> Option<String> + Send + Sync>;

/// Servo's three proxy preferences as they are (`network_http_proxy_uri`,
/// `network_https_proxy_uri`, `network_http_no_proxy`); an empty string means none. The fetch
/// goes through the proxy the page went through, or fails; it never connects directly instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProxyConfig {
    /// A proxy URI for `http` targets.
    pub http: String,
    /// A proxy URI for `https` targets.
    pub https: String,
    /// Hosts and domains that bypass the proxy (`hyper-util` matcher syntax).
    pub no_proxy: String,
}

/// What a fetch needs from the engine.
#[derive(Clone)]
pub struct FetchOptions {
    /// Sent as `User-Agent`: the engine's.
    pub user_agent: String,
    /// The cookies the engine would send; `None` sends none.
    pub cookies: Option<CookieLookup>,
    /// The proxy.
    pub proxy: ProxyConfig,
    /// How long connecting may take (default 10 s).
    pub connect_timeout: Duration,
    /// How long the whole fetch may take (default 30 s).
    pub total_timeout: Duration,
}

/// Why a fetch gave no document. The page shows [`FetchError::summary`] only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The server could not be reached (name, connection, reset).
    #[error("the server could not be reached")]
    Unreachable,
    /// A time limit passed.
    #[error("the server took too long")]
    Timeout,
    /// The TLS handshake or the certificate failed.
    #[error("the secure connection failed")]
    Tls,
    /// More than [`crate::MAX_REDIRECTS`] redirects.
    #[error("too many redirects")]
    TooManyRedirects,
    /// A redirect without a usable `Location`, or to a scheme other than `http` or `https`.
    #[error("the server sent a redirect that cannot be followed")]
    BadRedirect,
    /// The proxy refused or failed. Never retried directly.
    #[error("the proxy could not fetch the page")]
    Proxy,
    /// Anything else (a bug, a panic caught by the caller).
    #[error("the page could not be fetched")]
    Internal,
}

impl FetchError {
    /// A fixed sentence for the page. Nothing a server sent is ever shown.
    pub const fn summary(&self) -> &'static str {
        match self {
            Self::Unreachable => "The server could not be reached.",
            Self::Timeout => "The server took too long to answer.",
            Self::Tls => "The secure connection to the server failed.",
            Self::TooManyRedirects => "The server redirected too many times.",
            Self::BadRedirect => "The server sent a redirect that cannot be followed.",
            Self::Proxy => "The proxy could not fetch the page.",
            Self::Internal => "The source could not be fetched.",
        }
    }
}

/// Fetches `url` again. GET only; see the brief for the headers, redirects, bounds and proxy rules.
pub async fn fetch_source(
    _url: &ViewSourceUrl,
    _options: &FetchOptions,
) -> Result<SourceDocument, FetchError> {
    todo!("item 2: source-fetch")
}
