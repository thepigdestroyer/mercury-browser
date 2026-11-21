// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: the cookie rules of the network policy: third-party cookies blocked (EC5) and SameSite enforced
// (EC6). A new Mercury file, written from RFC 6265bis (draft 20, sections 5.2, 5.5 and 5.8) and the net-1
// lane brief; nothing in it is copied from Servo or another project. The hooks are in `http_loader.rs`,
// `resource_thread.rs`, `cookie.rs`, `cookie_storage.rs` and `servo-script`'s `document.rs`; `MERCURY.md`
// lists them.

//! Cookie rules that need to know who is asking.
//!
//! Servo's jar answers by URL alone. Mercury's policy ([`NetPolicy`]) asks two more questions before a cookie
//! goes out or comes in:
//!
//! * **Is the request third party?** With third-party cookies blocked (EC5), a request whose site differs
//!   from the top-level page's (or whose top-level site is unknown) gets no `Cookie` header, its `Set-Cookie`
//!   is ignored, and `document.cookie` of a third-party frame reads `""` and drops writes. Decision D5.
//! * **Is the request same-site?** With SameSite enforced (EC6), an explicit `SameSite=Strict` cookie is sent
//!   only on a same-site request and an explicit `SameSite=Lax` cookie also on a top-level navigation that
//!   uses a safe method; a `Set-Cookie` with either is accepted only from a same-site request or a top-level
//!   navigation; `SameSite=None` needs `Secure` (also for `document.cookie`). A cookie without the attribute
//!   is unrestricted (Lax-by-default, [`NetPolicy::same_site_lax_default`], stays off in Mercury, decision D8).
//!
//! A request is **same-site** when the site that started it, every site in its URL list (so no redirect
//! crossed sites) and the top-level site are one site. An opaque initiator on a top-level navigation also
//! counts (a typed URL, a bookmark, a link in the interface: decision D8), which is what lets a typed URL send
//! its `Strict` cookies. A document that reads `document.cookie` is same-site when its own site is the
//! top-level site.
//!
//! The functions here are pure over [`RequestContext`] and the cookie, so the rules are tested without a
//! network (`tests/cookie_policy.rs`).

use cookie::{Cookie, SameSite};
use http::Method;
use hyper_serde::Serde;
use net_traits::CookieSource;
use net_traits::pub_domains::Site;
use net_traits::request::{Request, RequestBuilder};
use servo_base::id::WebViewId;
use servo_url::ServoUrl;

use crate::cookie::ServoCookie;
use crate::http_loader::HttpState;
use crate::net_policy::{NetPolicy, Party, RequestParts, RequestSites, site_key_of_origin};

/// What the cookie rules need to know about one request, a WebSocket handshake or a document.
#[derive(Clone, Debug)]
pub(crate) struct RequestContext {
    /// The sites of the request (top-level, request, initiator, party).
    pub(crate) sites: RequestSites,
    /// The RFC 6265bis "same-site request" test: see the module documentation.
    pub(crate) same_site: bool,
    /// The request's method is safe (`GET`, `HEAD`, `OPTIONS`, `TRACE`).
    pub(crate) safe_method: bool,
}

impl RequestContext {
    /// The context of a request, at the hop it is at now.
    pub(crate) fn of_request(policy: &NetPolicy, request: &Request) -> RequestContext {
        RequestContext::of_parts(policy, &RequestParts::of_request(request))
    }

    /// The context of a request that is still a builder (the WebSocket handshake).
    pub(crate) fn of_builder(policy: &NetPolicy, request: &RequestBuilder) -> RequestContext {
        RequestContext::of_parts(policy, &RequestParts::of_builder(request))
    }

    /// The context of a document that reads or writes `document.cookie`.
    pub(crate) fn of_document(
        policy: &NetPolicy,
        url: &ServoUrl,
        partition_key: &Option<Site>,
        web_view: Option<WebViewId>,
    ) -> RequestContext {
        let sites = policy.sites_of_document(url, partition_key, web_view);
        RequestContext {
            // A document is same-site with the top-level page when its site is the top-level site.
            same_site: sites.party == Party::First,
            safe_method: true,
            sites,
        }
    }

    fn of_parts(policy: &NetPolicy, parts: &RequestParts<'_>) -> RequestContext {
        let sites = policy.sites_of_parts(parts);
        RequestContext {
            same_site: is_same_site_request(parts, &sites),
            safe_method: is_safe_method(parts.method),
            sites,
        }
    }
}

/// RFC 9110 section 9.2.1: the safe methods.
fn is_safe_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

/// RFC 6265bis section 5.2, as decided in the lane brief (D8): the initiator's site, every site in the URL
/// list and the top-level site are one site. An opaque initiator counts only on a top-level navigation. An
/// unknown or opaque top-level site never counts (fail closed).
fn is_same_site_request(parts: &RequestParts<'_>, sites: &RequestSites) -> bool {
    let Some(top_level) = sites.top_level.as_deref().filter(|site| !site.is_empty()) else {
        return false;
    };
    // No redirect hop (and not the first URL) may be on another site.
    if parts
        .urls
        .iter()
        .any(|url| site_key_of_origin(&url.origin()) != top_level)
    {
        return false;
    }
    match sites.initiator.as_deref() {
        Some(initiator) => initiator == top_level,
        // Opaque or missing: only a navigation that the browser itself started is same-site.
        None => sites.top_level_navigation,
    }
}

/// How a cookie is restricted by its `SameSite` attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Restriction {
    /// `SameSite=None`, or no attribute (while Lax-by-default is off).
    Unrestricted,
    Lax,
    Strict,
}

fn restriction_of(policy: &NetPolicy, cookie: &Cookie<'_>) -> Restriction {
    match cookie.same_site() {
        Some(SameSite::Strict) => Restriction::Strict,
        Some(SameSite::Lax) => Restriction::Lax,
        Some(SameSite::None) => Restriction::Unrestricted,
        None if policy.same_site_lax_default() => Restriction::Lax,
        None => Restriction::Unrestricted,
    }
}

/// RFC 6265bis section 5.8.3 (the "retrieval" steps for `SameSite`): whether `cookie` may go with the request
/// or be read by the document. With SameSite not enforced every cookie may.
pub(crate) fn cookie_may_be_sent(
    policy: &NetPolicy,
    context: &RequestContext,
    cookie: &Cookie<'_>,
) -> bool {
    if !policy.same_site_enforcement() {
        return true;
    }
    match restriction_of(policy, cookie) {
        Restriction::Unrestricted => true,
        Restriction::Strict => context.same_site,
        Restriction::Lax => {
            context.same_site || (context.sites.top_level_navigation && context.safe_method)
        },
    }
}

/// RFC 6265bis section 5.5 (storage steps for `SameSite`) and the `SameSite=None` needs `Secure` rule (decision
/// D6): whether a cookie that arrived with this request or from this document is stored. With SameSite not
/// enforced every cookie is.
pub(crate) fn accept_set_cookie(
    policy: &NetPolicy,
    context: &RequestContext,
    cookie: &Cookie<'_>,
) -> bool {
    if !policy.same_site_enforcement() {
        return true;
    }
    if cookie.same_site() == Some(SameSite::None) && !cookie.secure().unwrap_or(false) {
        return false;
    }
    match restriction_of(policy, cookie) {
        Restriction::Unrestricted => true,
        // Same-site, or a navigation of the top-level page (which may set a cookie for the site it is going to).
        Restriction::Strict | Restriction::Lax => {
            context.same_site || context.sites.top_level_navigation
        },
    }
}

/// `document.cookie` read: the cookie string the document may see, or `None`. A third-party document (or one
/// whose top-level site is unknown) sees nothing while third-party cookies are blocked.
pub(crate) fn document_cookie_string(
    state: &HttpState,
    url: &ServoUrl,
    partition_key: &Option<Site>,
    web_view: Option<WebViewId>,
    source: CookieSource,
) -> Option<String> {
    let policy = &state.net_policy;
    let context = RequestContext::of_document(policy, url, partition_key, web_view);
    if !policy.cookies_allowed(&context.sites) {
        return None;
    }
    let mut cookie_jar = state.cookie_jar.write();
    cookie_jar.remove_expired_cookies_for_url(url);
    cookie_jar.cookies_for_request(url, source, &|cookie| {
        cookie_may_be_sent(policy, &context, cookie)
    })
}

/// `document.cookie` write: the cookies the document may store. A third-party document's writes are dropped
/// while third-party cookies are blocked.
pub(crate) fn store_document_cookies(
    state: &HttpState,
    url: &ServoUrl,
    partition_key: &Option<Site>,
    web_view: Option<WebViewId>,
    cookies: Vec<Serde<Cookie<'static>>>,
    source: CookieSource,
) {
    let policy = &state.net_policy;
    let context = RequestContext::of_document(policy, url, partition_key, web_view);
    if !policy.cookies_allowed(&context.sites) {
        return;
    }
    let mut cookie_jar = state.cookie_jar.write();
    for cookie in cookies {
        let cookie = cookie.into_inner();
        if !accept_set_cookie(policy, &context, &cookie) {
            continue;
        }
        if let Some(cookie) = ServoCookie::new_wrapped(cookie, url, source) {
            cookie_jar.push(cookie, url, source);
        }
    }
}

/// Whether the `cookieStore` messages (`*Async`) are answered as if no cookie existed. They carry no site
/// context, so while third-party blocking is on the resource thread cannot tell a first party from a third
/// (decisions D5 and D7). `cookieStore` is pinned off in Mercury, so this is the fail-closed half.
pub(crate) fn async_cookie_messages_refused(state: &HttpState) -> bool {
    state.net_policy.block_third_party_cookies()
}

/// For the WebSocket handshake (`websocket_loader.rs`): the `Cookie` header value to send, or `None`. The
/// caller builds the context once, with [`RequestContext::of_builder`], and passes it to
/// [`websocket_may_store_cookies`] and [`accept_set_cookie`] for the response.
pub(crate) fn websocket_cookie_header(
    state: &HttpState,
    context: &RequestContext,
    url: &ServoUrl,
) -> Option<String> {
    let policy = &state.net_policy;
    if !policy.cookies_allowed(&context.sites) {
        return None;
    }
    let mut cookie_jar = state.cookie_jar.write();
    cookie_jar.remove_expired_cookies_for_url(url);
    cookie_jar.cookies_for_request(url, CookieSource::HTTP, &|cookie| {
        cookie_may_be_sent(policy, context, cookie)
    })
}

/// For the WebSocket handshake response: whether the `Set-Cookie` headers that came with it may be looked at
/// at all (third-party blocking). Each cookie must then pass [`accept_set_cookie`].
pub(crate) fn websocket_may_store_cookies(state: &HttpState, context: &RequestContext) -> bool {
    state.net_policy.cookies_allowed(&context.sites)
}
