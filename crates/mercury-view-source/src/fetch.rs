// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Fetching a page again, with the engine's cookies and proxy; the rules are in
//! `sdd/lane-sh-pages-brief.md`, "Rules the two halves keep".
//!
//! One GET per hop, redirects followed by hand (so every hop is checked and its cookies asked
//! for), the body read in chunks up to a cap, and nothing a server sent is ever carried in an
//! error. No log line holds an address, a header or a cookie.

use std::error::Error as _;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, CONTENT_ENCODING, CONTENT_TYPE, COOKIE, HeaderValue, LOCATION,
    USER_AGENT,
};
use reqwest::redirect::Policy;
use reqwest::{Client, NoProxy, Proxy, Response, StatusCode};
use url::Url;

use crate::address::ViewSourceUrl;
use crate::source::{SourceBody, SourceDocument, SourceKind};
use crate::text::{SNIFF_BYTES, classify, decode};
use crate::{MAX_REDIRECTS, MAX_SHOWN_BYTES};

/// The `Accept` header of a document request, as the engine sends it.
///
/// `DOCUMENT_ACCEPT_HEADER_VALUE` in `third_party/servo-net-traits/lib.rs` (lines 72-74), used
/// for `Destination::Document` by `set_default_accept` in `third_party/servo-net/http_loader.rs`
/// (lines 186-217; called from `fetch/methods.rs` lines 250-252) and set on navigations by
/// `third_party/servo-script/navigation.rs` (line 272).
const DOCUMENT_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

/// The `Accept-Language` header when [`FetchOptions::accept_language`] is empty.
///
/// The engine sends one language, the system's (`sys_locale`) or the `intl_locale_override`
/// preference, through `set_default_accept_language` in `third_party/servo-net-traits/lib.rs`
/// (lines 1441-1449; the locale is read by `get_current_locale`, lines 1425-1437). This crate
/// cannot read either; the engine glue passes the value in `accept_language`. This is what the
/// engine itself falls back to when it cannot read a locale (same lines): `en-US`.
const DEFAULT_ACCEPT_LANGUAGE: &str = "en-US";

/// The `Cookie` header text for a URL, asked for every hop of a redirect chain; `None` sends no
/// cookie. Set-Cookie of a response is never stored.
///
/// The engine glue (`talaria-servo/src/view_source.rs`, `engine_cookies`) is the one place that
/// says what the engine would send for a top-level GET of the address.
pub type CookieLookup = Arc<dyn Fn(&Url) -> Option<String> + Send + Sync>;

/// What the browser's own rules say about an address before it is requested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HopVerdict {
    /// Request this address, which may be the one asked about, upgraded (`http` to `https`).
    Go(Url),
    /// Do not request it: [`FetchError::Blocked`].
    Refuse,
}

/// The rules the engine applies to a top-level load that the re-fetch must apply too: HTTPS-only
/// mode, the HSTS upgrade and the block on bad ports. Asked for the first address and for every
/// redirect hop, before the cookies are looked up and before anything is sent.
pub type HopPolicy = Arc<dyn Fn(&Url) -> HopVerdict + Send + Sync>;

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
    /// Sent as `Accept-Language`: the engine's. Empty sends `en-US`.
    pub accept_language: String,
    /// The browser's rules for an address (HTTPS-only, HSTS, bad ports); `None` applies none.
    pub policy: Option<HopPolicy>,
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
    /// The browser's rules for an address (HTTPS-only mode, the block on bad ports) refuse it,
    /// as they would for a page.
    #[error("the address is refused by the browser's rules")]
    Blocked,
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
            Self::Blocked => {
                "This address is refused by the same rules that apply to pages (HTTPS-only mode, blocked ports)."
            }
            Self::Internal => "The source could not be fetched.",
        }
    }
}

/// Fetches `url` again. GET only; see the brief for the headers, redirects, bounds and proxy rules.
///
/// Dropping the future abandons the request and closes its connection.
pub async fn fetch_source(
    url: &ViewSourceUrl,
    options: &FetchOptions,
) -> Result<SourceDocument, FetchError> {
    let result = tokio::time::timeout(options.total_timeout, fetch_chain(url, options)).await;
    let result = result.unwrap_or(Err(FetchError::Timeout));
    if let Err(error) = &result {
        log::debug!("view-source: the fetch failed ({error:?})");
    }
    result
}

/// The request, its redirects and the body.
async fn fetch_chain(
    url: &ViewSourceUrl,
    options: &FetchOptions,
) -> Result<SourceDocument, FetchError> {
    let proxies = Proxies::new(&options.proxy)?;
    let client = build_client(options, &proxies)?;
    let headers = RequestHeaders::new(options)?;
    let mut current = apply_policy(options, url.target().clone())?;
    let mut redirects = 0;
    let mut seen_https = false;
    loop {
        let mut request = client
            .get(current.clone())
            .header(ACCEPT, headers.accept.clone())
            .header(ACCEPT_LANGUAGE, headers.accept_language.clone());
        if let Some(user_agent) = &headers.user_agent {
            request = request.header(USER_AGENT, user_agent.clone());
        }
        if let Some(cookie) = cookie_for_hop(options.cookies.as_ref(), seen_https, &current) {
            request = request.header(COOKIE, cookie);
        }
        let response = request
            .send()
            .await
            .map_err(|error| map_error(&error, proxies.applies_to(&current)))?;
        seen_https |= current.scheme() == "https";
        if !is_redirect(response.status()) {
            return read_document(response, current).await;
        }
        let next = redirect_target(&current, response.headers().get(LOCATION))?;
        if redirects >= MAX_REDIRECTS {
            return Err(FetchError::TooManyRedirects);
        }
        redirects += 1;
        // The body of a redirect is never read: dropping the response closes the connection.
        drop(response);
        current = apply_policy(options, next)?;
    }
}

/// The address to request for `url`: what the browser's rules make of it, or
/// [`FetchError::Blocked`]. Credentials and the fragment never survive, whatever the rules do.
fn apply_policy(options: &FetchOptions, url: Url) -> Result<Url, FetchError> {
    let Some(policy) = &options.policy else {
        return Ok(url);
    };
    match policy(&url) {
        HopVerdict::Refuse => Err(FetchError::Blocked),
        HopVerdict::Go(mut go) => {
            if !matches!(go.scheme(), "http" | "https") || go.host_str().is_none() {
                return Err(FetchError::Blocked);
            }
            let _ = go.set_username("");
            let _ = go.set_password(None);
            go.set_fragment(None);
            Ok(go)
        }
    }
}

/// The headers every hop carries, checked once.
struct RequestHeaders {
    accept: HeaderValue,
    accept_language: HeaderValue,
    user_agent: Option<HeaderValue>,
}

impl RequestHeaders {
    fn new(options: &FetchOptions) -> Result<Self, FetchError> {
        let user_agent = match options.user_agent.as_str() {
            "" => None,
            text => Some(HeaderValue::from_str(text).map_err(|_| FetchError::Internal)?),
        };
        let accept_language = match options.accept_language.trim() {
            "" => HeaderValue::from_static(DEFAULT_ACCEPT_LANGUAGE),
            text => HeaderValue::from_str(text).map_err(|_| FetchError::Internal)?,
        };
        Ok(Self {
            accept: HeaderValue::from_static(DOCUMENT_ACCEPT),
            accept_language,
            user_agent,
        })
    }
}

/// The `Cookie` header for one hop, or `None`. The lookup is asked with the hop's own address, so
/// a redirect to another host gets the cookies of that host and none of the first. Nothing is
/// sent from `https` down to `http`, where `Secure` cookies would leak; the lookup is not even
/// asked then.
fn cookie_for_hop(
    lookup: Option<&CookieLookup>,
    seen_https: bool,
    hop: &Url,
) -> Option<HeaderValue> {
    if seen_https && hop.scheme() == "http" {
        return None;
    }
    let text = lookup?(hop)?;
    if text.is_empty() {
        return None;
    }
    let mut value = HeaderValue::from_str(&text).ok()?;
    value.set_sensitive(true);
    Some(value)
}

/// The statuses a browser follows to a `Location`.
fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

/// Where a redirect from `current` leads. The address is resolved against `current`, loses its
/// credentials and fragment, and must be `http` or `https` with a host.
fn redirect_target(current: &Url, location: Option<&HeaderValue>) -> Result<Url, FetchError> {
    let location = location.ok_or(FetchError::BadRedirect)?;
    let text = String::from_utf8_lossy(location.as_bytes());
    let mut next = current.join(&text).map_err(|_| FetchError::BadRedirect)?;
    if !matches!(next.scheme(), "http" | "https") || next.host_str().is_none() {
        return Err(FetchError::BadRedirect);
    }
    // Credentials in an address are never sent: reqwest would turn them into `Authorization`.
    let _ = next.set_username("");
    let _ = next.set_password(None);
    next.set_fragment(None);
    Ok(next)
}

/// Reads the final response: status, type and the body up to the cap.
async fn read_document(
    mut response: Response,
    final_url: Url,
) -> Result<SourceDocument, FetchError> {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned());
    // No `Accept-Encoding` is sent, so a server should not compress; one that does anyway sends
    // bytes that cannot be shown as text.
    let encoded = response
        .headers()
        .get(CONTENT_ENCODING)
        .is_some_and(|value| {
            !value
                .as_bytes()
                .trim_ascii()
                .eq_ignore_ascii_case(b"identity")
        });
    let mut binary = encoded || classify(content_type.as_deref(), &[]) == SourceKind::Binary;
    let mut decided = binary;
    let mut counted = 0_usize;
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| map_error(&error, false))?;
        let Some(chunk) = chunk else { break };
        // One byte past the cap tells that there is more; whatever `Content-Length` promised.
        let take = chunk.len().min(MAX_SHOWN_BYTES + 1 - counted);
        counted += take;
        if !binary {
            buffer.extend_from_slice(&chunk[..take]);
            if !decided && buffer.len() >= SNIFF_BYTES {
                decided = true;
                binary = classify(content_type.as_deref(), &buffer) == SourceKind::Binary;
            }
        }
        if binary {
            buffer = Vec::new();
        }
        if counted > MAX_SHOWN_BYTES {
            break;
        }
    }
    let truncated = counted > MAX_SHOWN_BYTES;
    let bytes = counted.min(MAX_SHOWN_BYTES);
    if !decided {
        binary = classify(content_type.as_deref(), &buffer) == SourceKind::Binary;
    }
    if binary {
        return Ok(SourceDocument {
            final_url,
            status,
            content_type,
            kind: SourceKind::Binary,
            body: SourceBody::Binary {
                bytes: bytes as u64,
            },
        });
    }
    buffer.truncate(MAX_SHOWN_BYTES);
    let kind = classify(content_type.as_deref(), &buffer);
    let (text, charset) = decode(&buffer, content_type.as_deref());
    Ok(SourceDocument {
        final_url,
        status,
        content_type,
        kind,
        body: SourceBody::Text {
            text,
            bytes: bytes as u64,
            truncated,
            charset,
        },
    })
}

/// The client of one fetch.
///
/// Redirects are off (they are followed by hand), nothing is stored (no cookie store is built
/// in), and no `Accept-Encoding` is sent, so bodies arrive as the server wrote them.
///
/// `no_proxy()` comes first: it clears every proxy and switches off reqwest's own reading of the
/// `HTTP_PROXY` family of environment variables and of the system settings, which would
/// otherwise be added on top of what is given. Then the engine's proxies are added, and they
/// are the only ones. A target without a proxy for its scheme is connected to directly, exactly
/// as the engine does; a target with one never is.
fn build_client(options: &FetchOptions, proxies: &Proxies) -> Result<Client, FetchError> {
    let no_proxy = NoProxy::from_string(&options.proxy.no_proxy);
    let mut builder = Client::builder()
        .redirect(Policy::none())
        .referer(false)
        .connect_timeout(options.connect_timeout)
        .no_proxy();
    if let Some(uri) = &proxies.http {
        let proxy = Proxy::http(uri.clone()).map_err(|_| FetchError::Proxy)?;
        builder = builder.proxy(proxy.no_proxy(no_proxy.clone()));
    }
    if let Some(uri) = &proxies.https {
        let proxy = Proxy::https(uri.clone()).map_err(|_| FetchError::Proxy)?;
        builder = builder.proxy(proxy.no_proxy(no_proxy));
    }
    builder.build().map_err(|_| FetchError::Internal)
}

/// The two proxy addresses, checked, and the bypass list, for deciding whether a failure is the
/// proxy's. Where traffic goes is decided by reqwest (the same matcher family as the engine's);
/// this only names the failure.
struct Proxies {
    http: Option<Url>,
    https: Option<Url>,
    bypass: Bypass,
}

impl Proxies {
    /// A proxy that is set but cannot be used (not an `http` or `https` address) is an error, not
    /// a reason to connect directly.
    fn new(config: &ProxyConfig) -> Result<Self, FetchError> {
        Ok(Self {
            http: proxy_address(&config.http)?,
            https: proxy_address(&config.https)?,
            bypass: Bypass::new(&config.no_proxy),
        })
    }

    /// A proxy is configured for the scheme of `url` and the host is not on the bypass list.
    fn applies_to(&self, url: &Url) -> bool {
        let configured = match url.scheme() {
            "http" => self.http.is_some(),
            "https" => self.https.is_some(),
            _ => false,
        };
        configured
            && !url
                .host_str()
                .is_some_and(|host| self.bypass.contains(host))
    }
}

/// A proxy preference as an address: empty is none, a missing scheme means `http` (as in the
/// engine's matcher), anything but `http` and `https` is refused.
fn proxy_address(value: &str) -> Result<Option<Url>, FetchError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let text = if value.contains("://") {
        value.to_owned()
    } else {
        format!("http://{value}")
    };
    let url = Url::parse(&text).map_err(|_| FetchError::Proxy)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(FetchError::Proxy);
    }
    Ok(Some(url))
}

/// The `no_proxy` list with the rules of the engine's matcher (`hyper-util`): entries separated
/// by commas; an IP address or an `address/length` range; `*`; anything else a domain, which
/// matches itself and its subdomains (a leading dot is optional). A host that is an IP address is
/// only compared with the address entries.
struct Bypass {
    everything: bool,
    domains: Vec<String>,
    addresses: Vec<(IpAddr, Option<u8>)>,
}

impl Bypass {
    fn new(list: &str) -> Self {
        let mut bypass = Self {
            everything: false,
            domains: Vec::new(),
            addresses: Vec::new(),
        };
        for entry in list
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            if let Some(range) = parse_range(entry) {
                bypass.addresses.push(range);
            } else if entry == "*" {
                bypass.everything = true;
            } else {
                let domain = entry.trim_start_matches('.').to_ascii_lowercase();
                if !domain.is_empty() {
                    bypass.domains.push(domain);
                }
            }
        }
        bypass
    }

    fn contains(&self, host: &str) -> bool {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(address) = host.parse::<IpAddr>() {
            return self
                .addresses
                .iter()
                .any(|(network, length)| in_range(address, *network, *length));
        }
        if self.everything {
            return true;
        }
        let host = host.to_ascii_lowercase();
        self.domains.iter().any(|domain| {
            host == *domain
                || host
                    .strip_suffix(domain.as_str())
                    .is_some_and(|before| before.ends_with('.'))
        })
    }
}

/// `10.0.0.1`, `10.0.0.0/8`, `::1` or `fe80::/10`; `None` for anything else (a domain).
fn parse_range(entry: &str) -> Option<(IpAddr, Option<u8>)> {
    let (address, length) = match entry.split_once('/') {
        Some((address, length)) => (address, Some(length.parse::<u8>().ok()?)),
        None => (entry, None),
    };
    let address: IpAddr = address.parse().ok()?;
    let longest = if address.is_ipv4() { 32 } else { 128 };
    if length.is_some_and(|length| length > longest) {
        return None;
    }
    Some((address, length))
}

fn in_range(address: IpAddr, network: IpAddr, length: Option<u8>) -> bool {
    match (address, network) {
        (IpAddr::V4(address), IpAddr::V4(network)) => {
            let length = u32::from(length.unwrap_or(32));
            let mask = u32::MAX.checked_shl(32 - length).unwrap_or(0);
            u32::from(address) & mask == u32::from(network) & mask
        }
        (IpAddr::V6(address), IpAddr::V6(network)) => {
            let length = u32::from(length.unwrap_or(128));
            let mask = u128::MAX.checked_shl(128 - length).unwrap_or(0);
            u128::from(address) & mask == u128::from(network) & mask
        }
        _ => false,
    }
}

/// What a failed request or body read means for the page. `proxied` says that a proxy was used
/// for this address, so a failure to connect is the proxy's (or the tunnel's), not the server's.
///
/// Only the sources of the error are read, never its own text, which carries the address; the
/// words found are matched, never shown.
fn map_error(error: &reqwest::Error, proxied: bool) -> FetchError {
    if error.is_timeout() {
        return FetchError::Timeout;
    }
    if error.is_builder() {
        return FetchError::Internal;
    }
    let mut causes = String::new();
    let mut source = error.source();
    while let Some(cause) = source {
        causes.push_str(&cause.to_string().to_ascii_lowercase());
        causes.push(' ');
        source = cause.source();
    }
    let secure = [
        "tls",
        "certificate",
        "handshake",
        "rustls",
        "unknownissuer",
        "corrupt message",
    ]
    .iter()
    .any(|word| causes.contains(word));
    let tunnel = causes.contains("tunnel") || causes.contains("proxy");
    if proxied && (tunnel || !secure) {
        return FetchError::Proxy;
    }
    if secure {
        return FetchError::Tls;
    }
    FetchError::Unreachable
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    fn lookup(counter: &Arc<AtomicUsize>) -> CookieLookup {
        let counter = Arc::clone(counter);
        Arc::new(move |hop: &Url| {
            counter.fetch_add(1, Ordering::SeqCst);
            Some(format!("host={}", hop.host_str().unwrap_or_default()))
        })
    }

    #[test]
    fn cookies_are_asked_for_each_hop_and_never_sent_from_https_to_http() {
        let asked = Arc::new(AtomicUsize::new(0));
        let lookup = lookup(&asked);
        let cookie = |seen_https: bool, hop: &str| {
            cookie_for_hop(Some(&lookup), seen_https, &url(hop))
                .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
        };
        assert_eq!(
            cookie(false, "http://a.test/").as_deref(),
            Some("host=a.test")
        );
        assert_eq!(
            cookie(false, "https://b.test/").as_deref(),
            Some("host=b.test")
        );
        assert_eq!(
            cookie(true, "https://c.test/").as_deref(),
            Some("host=c.test")
        );
        assert_eq!(asked.load(Ordering::SeqCst), 3);
        // After any https hop, an http hop gets nothing and the lookup is not even asked.
        assert_eq!(cookie(true, "http://a.test/"), None);
        assert_eq!(asked.load(Ordering::SeqCst), 3);
        assert_eq!(cookie_for_hop(None, false, &url("http://a.test/")), None);
    }

    #[test]
    fn an_empty_or_unusable_cookie_is_not_sent() {
        let empty: CookieLookup = Arc::new(|_: &Url| Some(String::new()));
        let bad: CookieLookup = Arc::new(|_: &Url| Some("a=b\r\nInjected: 1".to_owned()));
        let none: CookieLookup = Arc::new(|_: &Url| None);
        let hop = url("https://a.test/");
        assert!(cookie_for_hop(Some(&empty), false, &hop).is_none());
        assert!(cookie_for_hop(Some(&bad), false, &hop).is_none());
        assert!(cookie_for_hop(Some(&none), false, &hop).is_none());
    }

    #[test]
    fn a_cookie_value_is_marked_sensitive() {
        let lookup: CookieLookup = Arc::new(|_: &Url| Some("a=b".to_owned()));
        let value = cookie_for_hop(Some(&lookup), false, &url("https://a.test/")).unwrap();
        assert!(value.is_sensitive());
    }

    fn header(text: &str) -> HeaderValue {
        HeaderValue::from_str(text).unwrap()
    }

    #[test]
    fn redirect_targets_are_resolved_cleaned_and_checked() {
        let current = url("https://a.test/dir/page?q=1");
        let target = |location: &str| redirect_target(&current, Some(&header(location)));
        assert_eq!(target("/x").unwrap().as_str(), "https://a.test/x");
        assert_eq!(
            target("y?z=1").unwrap().as_str(),
            "https://a.test/dir/y?z=1"
        );
        assert_eq!(target("../up").unwrap().as_str(), "https://a.test/up");
        assert_eq!(target("//b.test/p").unwrap().as_str(), "https://b.test/p");
        assert_eq!(
            target("http://b.test:8080/p#frag").unwrap().as_str(),
            "http://b.test:8080/p"
        );
        assert_eq!(
            target("https://user:secret@b.test/p").unwrap().as_str(),
            "https://b.test/p"
        );
        // An address with a space is encoded, not refused.
        assert_eq!(target("/a b").unwrap().as_str(), "https://a.test/a%20b");
    }

    #[test]
    fn redirects_to_other_schemes_or_nowhere_are_bad() {
        let current = url("https://a.test/");
        for location in [
            "file:///etc/hosts",
            "javascript:alert(1)",
            "data:text/html,hi",
            "ftp://b.test/",
            "about:blank",
            "view-source:https://b.test/",
            "http://",
            "https://[::1",
        ] {
            assert_eq!(
                redirect_target(&current, Some(&header(location))),
                Err(FetchError::BadRedirect),
                "{location}"
            );
        }
        assert_eq!(
            redirect_target(&current, None),
            Err(FetchError::BadRedirect)
        );
    }

    #[test]
    fn a_location_that_is_not_utf8_is_read_as_lossy_text() {
        let current = url("https://a.test/");
        let odd = HeaderValue::from_bytes(b"/caf\xE9").unwrap();
        let target = redirect_target(&current, Some(&odd)).unwrap();
        assert_eq!(target.host_str(), Some("a.test"));
        assert!(target.path().starts_with("/caf"));
    }

    #[test]
    fn only_the_browsers_redirect_statuses_are_followed() {
        for code in [301, 302, 303, 307, 308] {
            assert!(is_redirect(StatusCode::from_u16(code).unwrap()), "{code}");
        }
        for code in [200, 204, 300, 304, 305, 306, 404, 500] {
            assert!(!is_redirect(StatusCode::from_u16(code).unwrap()), "{code}");
        }
    }

    #[test]
    fn proxy_preferences_are_empty_or_an_http_address() {
        assert_eq!(proxy_address(""), Ok(None));
        assert_eq!(proxy_address("   "), Ok(None));
        assert_eq!(
            proxy_address("proxy.test:3128").unwrap().unwrap().as_str(),
            "http://proxy.test:3128/"
        );
        assert_eq!(
            proxy_address(" https://u:p@proxy.test:8443 ")
                .unwrap()
                .unwrap()
                .as_str(),
            "https://u:p@proxy.test:8443/"
        );
        for refused in [
            "socks5://proxy.test:1080",
            "ftp://proxy.test",
            "http://",
            "://x",
        ] {
            assert_eq!(proxy_address(refused), Err(FetchError::Proxy), "{refused}");
        }
    }

    #[test]
    fn the_bypass_list_follows_the_engines_rules() {
        let bypass =
            Bypass::new("localhost, .Example.com ,10.0.0.0/8, ::1 ,fe80::/10, 192.168.1.5");
        for host in [
            "localhost",
            "LOCALHOST",
            "example.com",
            "www.example.com",
            "a.b.example.com",
            "10.1.2.3",
            "[::1]",
            "fe80::1",
            "192.168.1.5",
        ] {
            assert!(bypass.contains(host), "{host}");
        }
        for host in [
            "notexample.com",
            "example.com.evil.test",
            "11.0.0.1",
            "192.168.1.6",
            "[::2]",
            "other.test",
            "localhost.test",
        ] {
            assert!(!bypass.contains(host), "{host}");
        }
        // An address range is not a domain, and a port is not part of a host.
        assert!(!Bypass::new("example.com:8080").contains("example.com"));
        assert!(!Bypass::new("").contains("example.com"));
    }

    #[test]
    fn a_star_bypasses_names_but_not_address_literals() {
        // The engine's matcher keeps `*` with the domains, so an address literal is not covered.
        let bypass = Bypass::new("*");
        assert!(bypass.contains("example.com"));
        assert!(!bypass.contains("203.0.113.9"));
        assert!(!bypass.contains("[2001:db8::1]"));
    }

    #[test]
    fn a_range_with_a_length_that_is_too_long_is_a_domain() {
        assert!(parse_range("10.0.0.0/33").is_none());
        assert!(parse_range("::1/129").is_none());
        assert!(parse_range("10.0.0.0/xyz").is_none());
        assert!(parse_range("example.com").is_none());
        assert_eq!(
            parse_range("10.0.0.0/0").map(|(_, length)| length),
            Some(Some(0))
        );
        assert!(in_range(
            "203.0.113.9".parse().unwrap(),
            "10.0.0.0".parse().unwrap(),
            Some(0)
        ));
    }

    #[test]
    fn a_proxy_applies_to_its_scheme_unless_the_host_is_bypassed() {
        let proxies = Proxies::new(&ProxyConfig {
            http: "http://proxy.test:3128".to_owned(),
            https: String::new(),
            no_proxy: "internal.test".to_owned(),
        })
        .unwrap();
        assert!(proxies.applies_to(&url("http://a.test/")));
        assert!(!proxies.applies_to(&url("https://a.test/")));
        assert!(!proxies.applies_to(&url("http://www.internal.test/")));
        assert!(
            !Proxies::new(&ProxyConfig::default())
                .unwrap()
                .applies_to(&url("http://a.test/"))
        );
    }
}
