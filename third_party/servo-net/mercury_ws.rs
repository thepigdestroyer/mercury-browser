// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: WebSockets through the proxy (EC23a) and the network policy's say in the handshake (EC5, EC25).
// This is a new Mercury file; nothing in it is copied from Servo or another project. The hooks are in
// `websocket_loader.rs` and `connector.rs` (`ProxyConnector::new` is `pub(crate)`); `MERCURY.md` lists them.

//! WebSockets through the proxy, and the policy's decisions in the opening handshake.
//!
//! # The proxy (EC23a)
//!
//! Servo 0.8 opens the TCP connection of a WebSocket with a bare `TcpStream::connect`, so a page that
//! opens `ws://` or `wss://` goes **around** the proxy that every HTTP request of the same browser uses
//! (and around the `no_proxy` list). [`connect_tcp`] asks the same [`ProxyConnector`] that HTTP uses:
//!
//! * the proxy for the scheme (`network_http_proxy_uri` for `ws`, `network_https_proxy_uri` for `wss`),
//!   with a `CONNECT host:port` tunnel and the proxy's basic authentication, exactly as HTTP does;
//! * a direct connection when the `no_proxy` list matches or there is no proxy; the connector then applies
//!   the host-file mapping (`hosts::replace_host`) as it does for HTTP, and only on this path: a proxy is
//!   asked for the name the page used, never for the test mapping.
//!
//! The TLS handshake of `wss` is done by the caller over the returned stream, after the tunnel is open, so
//! the proxy sees the target's name and port and nothing of the traffic.
//!
//! The proxy preferences are read when the function is **called** (not when the future is first polled),
//! so a test can set them, call, and restore them.
//!
//! # The handshake (EC5, EC25)
//!
//! The opening handshake is a request, and it follows the same policy as every other one:
//!
//! * cookies: [`handshake_cookie_header`] builds the `Cookie` header with the rules of `mercury_cookies.rs` (a
//!   third-party socket sends none while third-party blocking is on; `SameSite` cookies only where the rules let
//!   a request of this kind carry them), and [`ResponseCookies`] decides which `Set-Cookie` of the answer is stored;
//! * [`add_global_privacy_control`] adds `Sec-GPC: 1` when the policy says so. `http_network_or_cache_fetch`
//!   adds the same header to the same request; both use `insert`, so the header is there once.
//!
//! A blocked decision is counted by the policy's statistics, as it is for HTTP.

use std::future::Future;
use std::io;

use http::header::{HeaderMap, HeaderValue};
use http::uri::Uri as Destination;
use net_traits::request::{Request, RequestBuilder};
use tokio::net::TcpStream;
use tower::ServiceExt;

use crate::connector::ProxyConnector;
use crate::cookie::ServoCookie;
use crate::http_loader::HttpState;
use crate::mercury_cookies::{self, RequestContext};
// Mercury: re-exported so that the integration tests (which see only the public API) build the handshake
// request the way `resource_thread.rs` does.
pub use crate::websocket_loader::create_handshake_request;

/// The destination `connect_tcp` hands to the connector: `http://host:port` for `ws` (`tls` false) and
/// `https://host:port` for `wss` (`tls` true). The scheme only chooses which proxy preference applies;
/// no request is made with it. A bare IPv6 host gets its brackets.
pub fn destination_for(host: &str, port: u16, tls: bool) -> io::Result<Destination> {
    let scheme = if tls { "https" } else { "http" };
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    format!("{scheme}://{host}:{port}")
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

/// Open the TCP connection of a WebSocket to `host:port`: through the proxy for this scheme when there is
/// one (a `CONNECT` tunnel, with the proxy's credentials), directly when the `no_proxy` list matches or
/// there is no proxy. `tls` is true for `wss`. See the module documentation.
///
/// The returned future owns everything it needs (it does not borrow `host`).
pub fn connect_tcp(
    host: &str,
    port: u16,
    tls: bool,
) -> impl Future<Output = io::Result<TcpStream>> + Send + use<> {
    // The proxy preferences are read here, now, not when the future is polled.
    let connector = ProxyConnector::new();
    let destination = destination_for(host, port, tls);
    async move {
        let stream = connector
            .oneshot(destination?)
            .await
            .map_err(io::Error::other)?;
        Ok(stream.into_inner())
    }
}

/// The `Cookie` header value of the handshake of `request`, or `None`: nothing when the policy blocks cookies
/// for a socket of this party (third party with third-party blocking on, or a top-level site the policy
/// does not know), and only the cookies that the `SameSite` rules let a request of this kind carry.
pub(crate) fn handshake_cookie_header(
    state: &HttpState,
    request: &RequestBuilder,
) -> Option<String> {
    let context = RequestContext::of_builder(&state.net_policy, request);
    mercury_cookies::websocket_cookie_header(state, &context, &request.url)
}

/// What the policy lets the answer to a handshake store: decided once from the request the fetch carries
/// (so after redirects, which a WebSocket does not follow, and with the web view's record).
pub(crate) struct ResponseCookies {
    context: RequestContext,
    may_store: bool,
}

impl ResponseCookies {
    pub(crate) fn of_request(state: &HttpState, request: &Request) -> ResponseCookies {
        let context = RequestContext::of_request(&state.net_policy, request);
        let may_store = mercury_cookies::websocket_may_store_cookies(state, &context);
        ResponseCookies { context, may_store }
    }

    /// Whether this `Set-Cookie` of the answer is stored: third-party blocking first, then the `SameSite`
    /// storage rules.
    pub(crate) fn accepts(&self, state: &HttpState, cookie: &ServoCookie) -> bool {
        self.may_store
            && mercury_cookies::accept_set_cookie(&state.net_policy, &self.context, &cookie.cookie)
    }
}

/// Add `Sec-GPC: 1` to the handshake's headers when the policy asks for it.
pub(crate) fn add_global_privacy_control(state: &HttpState, headers: &mut HeaderMap) {
    if state.net_policy.global_privacy_control() {
        headers.insert("Sec-GPC", HeaderValue::from_static("1"));
    }
}
