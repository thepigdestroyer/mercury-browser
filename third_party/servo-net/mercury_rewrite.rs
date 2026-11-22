// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: what the request interceptor tells the embedder about a request (frame identity, EC38 and
// decision D17) and what the embedder may answer to change it (a rewrite, EC37 and decision D9). This is
// a new Mercury file; the sources are the Fetch and HTML specifications and Mercury's own design, and
// nothing in it is copied from Servo or another project. The hooks are in `request_interceptor.rs` and
// `fetch/methods.rs`; `MERCURY.md` lists them.

//! The two Mercury halves of the request interceptor.
//!
//! **What the embedder is told** ([`FrameIdentity`]). Servo 0.8 reports a request to the embedder with a
//! method, headers, URL, destination, referrer and a flag `is_for_main_frame` that is `destination ==
//! Document`. A frame that a script or a link navigates is fetched with the destination `Document` too, so
//! the flag was true for every ad frame an ad script navigated. Mercury's flag is true only for a
//! navigation of the top-level frame (a destination of `Document` and a client that is not a nested
//! browsing context; with no client the old test stays). The request also gets its frame identity: the
//! site of the top-level document, the origin that started it and whether it is third party, all from
//! [`NetPolicy::sites_of_request`], so the embedder no longer infers third party from the `Referer`.
//!
//! **What the embedder may answer** ([`apply`]). `WebResourceResponseMsg::Rewrite` changes the request's
//! URL and headers and lets it go on; it is terminal, so the embedder is asked once per hop. The network
//! thread applies a rewrite whole or not at all, and refuses it when any part breaks a rule:
//!
//! * only `http` and `https` requests, and never a WebSocket;
//! * the scheme stays (HSTS may upgrade the new URL afterwards: `fetch/methods.rs` applies it again);
//! * the origin stays, unless the request is a `GET` navigation of the top-level frame without a body: a
//!   frame's navigation was checked against the parent's `frame-src` for the old URL, and a body would go
//!   to the new origin (when the origin does change, the referrer is worked out again for the new URL);
//! * the user name and password in the URL stay;
//! * the URL may not grow past 8 KiB (a longer URL may only shrink or stay);
//! * header changes: at most 16 sets and 16 removals; names of at most 128 bytes, values of at most 4 KiB
//!   of visible ASCII; none of the names the Fetch standard forbids a page to set (`Cookie`, `Origin`,
//!   `Referer`, `Host`, `Content-Length`, `Transfer-Encoding`, `Connection`, `Upgrade`, `Accept-Encoding`,
//!   `Via` and the rest), `Authorization`, and any name that starts with `Sec-` or `Proxy-`. Removing one of
//!   them is refused too.
//!
//! A refusal is logged once at `warn` with its reason and no URL, header name or value, and the request
//! goes on as it was. Mercury's first user of the hook is the content blocker's `$removeparam` (D10).
//!
//! Drop with the hooks (the two in `request_interceptor.rs`, the three in `fetch/methods.rs`, the variant in
//! `servo-embedder-traits` and the `mod` line in `lib.rs`) if Servo grows an equivalent request-rewrite
//! point.

use std::fmt;

use embedder_traits::WebResourceRewrite;
use http::Method;
use http::header::{HeaderName, HeaderValue};
use log::warn;
use net_traits::request::{Origin, Referrer, Request, RequestMode};
use servo_url::ServoUrl;

use crate::http_loader::determine_requests_referrer;
use crate::net_policy::{NetPolicy, Party, RequestParts};

/// A rewritten URL may be this long even if the original was shorter; a longer original may only shrink.
const MAX_URL_BYTES: usize = 8 * 1024;
/// At most this many headers set and, separately, removed by one rewrite.
const MAX_HEADER_CHANGES: usize = 16;
const MAX_HEADER_NAME_BYTES: usize = 128;
const MAX_HEADER_VALUE_BYTES: usize = 4 * 1024;

/// Request header names a rewrite may neither set nor remove, in lower case: the Fetch standard's forbidden
/// request-header names (<https://fetch.spec.whatwg.org/#forbidden-request-header>) and `Authorization`.
/// The `Sec-` and `Proxy-` prefixes are checked separately.
const FORBIDDEN_HEADERS: [&str; 22] = [
    "accept-charset",
    "accept-encoding",
    "access-control-request-headers",
    "access-control-request-method",
    "authorization",
    "connection",
    "content-length",
    "cookie",
    "cookie2",
    "date",
    "dnt",
    "expect",
    "host",
    "keep-alive",
    "origin",
    "referer",
    "set-cookie",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "via",
];

/// Whether the request is a navigation of the top-level frame (D17): the destination is `Document` and the
/// client is not a nested browsing context. A request without a client keeps the old test, the destination
/// alone. A frame navigated by a script or a link has the destination `Document` as well, but its client
/// is nested.
pub(crate) fn is_top_level_document(request: &Request) -> bool {
    // One definition, shared with the network policy's "top-level navigation" (net-1 review, minor 8).
    RequestParts::of_request(request).is_top_level_document()
}

/// The ASCII serialisation of the origin that started the request: the request's origin, or the client's
/// when the request's is still "client". `None` for an opaque origin (a typed address, a sandbox, `data:`).
fn initiator_origin(request: &Request) -> Option<String> {
    let origin = match &request.origin {
        Origin::Origin(origin) => origin,
        Origin::Client => match &request.client.as_ref()?.origin {
            Origin::Origin(origin) => origin,
            Origin::Client => return None,
        },
    };
    origin
        .is_tuple()
        .then(|| origin.ascii_serialization().into_owned())
}

/// What the embedder is told about a request besides its URL, headers and destination (EC38, D17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameIdentity {
    pub is_for_main_frame: bool,
    pub top_level_site: Option<String>,
    pub initiator_origin: Option<String>,
    pub is_third_party: Option<bool>,
}

impl FrameIdentity {
    /// The identity of `request`, judged by `policy` (decision D3 in the net-1 brief: the document's own
    /// partition key, a top-level navigation, the policy's record of the web view, otherwise unknown).
    /// `is_third_party` is `None` when the top-level site is not known, so the embedder can tell "not third
    /// party" from "cannot tell".
    pub(crate) fn of(request: &Request, policy: &NetPolicy) -> FrameIdentity {
        let sites = policy.sites_of_request(request);
        let top_level_site = sites.top_level.filter(|site| !site.is_empty());
        let is_third_party = match (&top_level_site, sites.party) {
            (Some(_), Party::First) => Some(false),
            (Some(_), Party::Third) => Some(true),
            (_, _) => None,
        };
        FrameIdentity {
            is_for_main_frame: is_top_level_document(request),
            top_level_site,
            initiator_origin: initiator_origin(request),
            is_third_party,
        }
    }
}

/// What a rewrite that was applied did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    /// The request's current URL is now another URL.
    pub url_changed: bool,
    /// The new URL has another origin (only a top-level navigation may do that), so the referrer was
    /// worked out again.
    pub origin_changed: bool,
    /// Headers set.
    pub headers_set: usize,
    /// Headers that were present and are removed.
    pub headers_removed: usize,
}

/// Why a rewrite was refused. The text names the rule, never a URL, a header name or a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RewriteRefusal {
    /// The request, or the new URL, is not `http` or `https`, or the request is a WebSocket.
    NotHttp,
    SchemeChanged,
    /// The origin changed on a request that is not a `GET` navigation of the top-level frame.
    OriginChanged,
    CredentialsChanged,
    UrlTooLong,
    TooManyHeaders,
    /// A header name or value that is not valid, or is too long.
    InvalidHeader,
    /// A header the rewrite may not set or remove.
    ForbiddenHeader,
}

impl fmt::Display for RewriteRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotHttp => {
                "only an http or https request that is not a WebSocket can be rewritten"
            },
            Self::SchemeChanged => "a rewrite may not change the scheme",
            Self::OriginChanged => {
                "only a GET navigation of the top-level frame may change the origin"
            },
            Self::CredentialsChanged => "a rewrite may not change the user name or password",
            Self::UrlTooLong => "the new URL is too long",
            Self::TooManyHeaders => "too many header changes",
            Self::InvalidHeader => "a header name or value is not valid or is too long",
            Self::ForbiddenHeader => "a rewrite may not set or remove that kind of header",
        })
    }
}

/// A rewrite that passed every rule and can be committed without failing.
struct Plan {
    url: Option<ServoUrl>,
    origin_changed: bool,
    remove: Vec<HeaderName>,
    set: Vec<(HeaderName, HeaderValue)>,
}

/// Apply `rewrite` to `request`, whole or not at all (see the module documentation for the rules). A
/// refusal is logged at `warn` (the reason only) and leaves the request as it was. `policy` supplies the
/// referrer cap when the origin changed and the referrer is worked out again.
pub(crate) fn apply(
    request: &mut Request,
    rewrite: WebResourceRewrite,
    policy: &NetPolicy,
) -> Result<Applied, RewriteRefusal> {
    match plan(request, &rewrite) {
        Ok(plan) => Ok(commit(request, plan, policy)),
        Err(refusal) => {
            warn!("Mercury: refused a request rewrite: {refusal}");
            Err(refusal)
        },
    }
}

fn plan(request: &Request, rewrite: &WebResourceRewrite) -> Result<Plan, RewriteRefusal> {
    if rewrite.set_headers.len() > MAX_HEADER_CHANGES ||
        rewrite.remove_headers.len() > MAX_HEADER_CHANGES
    {
        return Err(RewriteRefusal::TooManyHeaders);
    }
    let mut remove = Vec::with_capacity(rewrite.remove_headers.len());
    for name in &rewrite.remove_headers {
        remove.push(header_name(name)?);
    }
    let mut set = Vec::with_capacity(rewrite.set_headers.len());
    for (name, value) in &rewrite.set_headers {
        set.push((header_name(name)?, header_value(value)?));
    }

    let (url, origin_changed) = match &rewrite.url {
        Some(new) => {
            let origin_changed = check_url(request, new)?;
            // Nothing to commit for the URL when it is the one the request has.
            (
                (*new != request.current_url()).then(|| new.clone()),
                origin_changed,
            )
        },
        None => (None, false),
    };
    Ok(Plan {
        url,
        origin_changed,
        remove,
        set,
    })
}

fn header_name(name: &str) -> Result<HeaderName, RewriteRefusal> {
    if name.len() > MAX_HEADER_NAME_BYTES {
        return Err(RewriteRefusal::InvalidHeader);
    }
    let name =
        HeaderName::from_bytes(name.as_bytes()).map_err(|_| RewriteRefusal::InvalidHeader)?;
    // `HeaderName` is lower case, so one comparison covers every spelling.
    let lower = name.as_str();
    if lower.starts_with("sec-") ||
        lower.starts_with("proxy-") ||
        FORBIDDEN_HEADERS.contains(&lower)
    {
        return Err(RewriteRefusal::ForbiddenHeader);
    }
    Ok(name)
}

fn header_value(value: &str) -> Result<HeaderValue, RewriteRefusal> {
    if value.len() > MAX_HEADER_VALUE_BYTES || !value.is_ascii() {
        return Err(RewriteRefusal::InvalidHeader);
    }
    // ASCII only (`from_str` alone would let UTF-8 bytes through); it also refuses a line break and
    // any control character other than a tab.
    HeaderValue::from_str(value).map_err(|_| RewriteRefusal::InvalidHeader)
}

/// The rules for the new URL; `Ok` says whether its origin differs from the request's current URL's.
fn check_url(request: &Request, new: &ServoUrl) -> Result<bool, RewriteRefusal> {
    if matches!(request.mode, RequestMode::WebSocket { .. }) {
        return Err(RewriteRefusal::NotHttp);
    }
    let old = request.current_url();
    if !matches!(old.scheme(), "http" | "https") {
        return Err(RewriteRefusal::NotHttp);
    }
    if new.scheme() != old.scheme() {
        return Err(RewriteRefusal::SchemeChanged);
    }
    if new.username() != old.username() || new.password() != old.password() {
        return Err(RewriteRefusal::CredentialsChanged);
    }
    if new.as_str().len() > MAX_URL_BYTES.max(old.as_str().len()) {
        return Err(RewriteRefusal::UrlTooLong);
    }
    let origin_changed = new.origin() != old.origin();
    if origin_changed &&
        !(request.mode == RequestMode::Navigate &&
            is_top_level_document(request) &&
            request.method == Method::GET &&
            request.body.is_none())
    {
        return Err(RewriteRefusal::OriginChanged);
    }
    Ok(origin_changed)
}

fn commit(request: &mut Request, plan: Plan, policy: &NetPolicy) -> Applied {
    let mut applied = Applied {
        origin_changed: plan.origin_changed,
        ..Applied::default()
    };
    if let Some(url) = plan.url {
        *request.current_url_mut() = url;
        applied.url_changed = true;
    }
    for name in &plan.remove {
        if request.headers.remove(name).is_some() {
            applied.headers_removed += 1;
        }
    }
    for (name, value) in plan.set {
        request.headers.insert(name, value);
        applied.headers_set += 1;
    }
    if plan.origin_changed {
        // The referrer was worked out for the old URL: a `same-origin` policy would now send it to
        // another origin. Work it out again for the new URL, from the referrer that is left (already
        // trimmed, so this can only tighten it), with the same cap as `main_fetch`.
        request.referrer = match std::mem::replace(&mut request.referrer, Referrer::NoReferrer) {
            Referrer::NoReferrer => Referrer::NoReferrer,
            Referrer::ReferrerUrl(source) | Referrer::Client(source) => {
                let referrer_policy = policy.cap_referrer_policy(request.referrer_policy);
                determine_requests_referrer(referrer_policy, source, request.current_url())
                    .map_or(Referrer::NoReferrer, Referrer::ReferrerUrl)
            },
        };
    }
    applied
}

#[cfg(test)]
mod tests {
    use net_traits::ReferrerPolicy;
    use net_traits::blob_url_store::UrlWithBlobClaim;
    use net_traits::pub_domains::Site;
    use net_traits::request::{
        Destination, InsecureRequestsPolicy, RequestBuilder, RequestClient,
    };

    use super::*;

    fn url(text: &str) -> ServoUrl {
        ServoUrl::parse(text).unwrap()
    }

    fn client(origin_url: &ServoUrl, nested: bool) -> RequestClient {
        RequestClient {
            preloaded_resources: Default::default(),
            policy_container: Default::default(),
            origin: Origin::Origin(origin_url.origin()),
            is_nested_browsing_context: nested,
            insecure_requests_policy: InsecureRequestsPolicy::DoNotUpgrade,
            has_trustworthy_ancestor_origin: false,
            network_partition_key: Some(Site::from_origin(&origin_url.origin())),
            browser_initiated_navigation: false,
        }
    }

    /// A subresource request from `page` to `target` (destination none, no-cors).
    fn subresource(page: &str, target: &str) -> Request {
        let page = url(page);
        RequestBuilder::new(
            None,
            UrlWithBlobClaim::new(url(target), None),
            Referrer::NoReferrer,
        )
        .origin(page.origin())
        .client(client(&page, false))
        .policy_container(Default::default())
        .build()
    }

    /// A navigation of a frame to `target`; `nested` says whether the frame is below the top level.
    fn navigation(target: &str, nested: bool) -> Request {
        let page = url("http://page.example/");
        RequestBuilder::new(
            None,
            UrlWithBlobClaim::new(url(target), None),
            Referrer::NoReferrer,
        )
        .destination(Destination::Document)
        .mode(RequestMode::Navigate)
        .origin(page.origin())
        .client(client(&page, nested))
        .policy_container(Default::default())
        .build()
    }

    fn rewrite_url(target: &str) -> WebResourceRewrite {
        WebResourceRewrite {
            url: Some(url(target)),
            ..WebResourceRewrite::default()
        }
    }

    fn headers(set: &[(&str, &str)], remove: &[&str]) -> WebResourceRewrite {
        WebResourceRewrite {
            url: None,
            set_headers: set
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            remove_headers: remove.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    fn unchanged(request: &Request, before_url: &str, before_headers: usize) {
        assert_eq!(request.current_url().as_str(), before_url);
        assert_eq!(request.headers.len(), before_headers);
    }

    #[test]
    fn top_level_document_needs_the_document_destination_and_a_client_that_is_not_nested() {
        assert!(is_top_level_document(&navigation(
            "http://a.example/",
            false
        )));
        // A frame navigated by a script or a link: destination Document, nested client (D17).
        assert!(!is_top_level_document(&navigation(
            "http://a.example/",
            true
        )));
        // Not a document at all.
        assert!(!is_top_level_document(&subresource(
            "http://page.example/",
            "http://a.example/x.js"
        )));
        // Without a client the destination decides, as it did before Mercury.
        let mut no_client = navigation("http://a.example/", false);
        no_client.client = None;
        assert!(is_top_level_document(&no_client));
        no_client.destination = Destination::IFrame;
        assert!(!is_top_level_document(&no_client));
    }

    /// "Top-level document" (the embedder's `is_for_main_frame`, the rewrite rules) and "top-level navigation" (the
    /// network policy's rule 2 for the site of a request) share one definition and differ in exactly three
    /// deliberate ways: a request without a client, a mode other than `Navigate` and an initiator make a request
    /// a top-level document but not a top-level navigation (net-1 review, minor 8). Every top-level navigation is a
    /// top-level document.
    #[test]
    fn a_top_level_navigation_is_a_top_level_document_and_the_difference_is_pinned() {
        let policy = NetPolicy::new();
        let is_navigation = |request: &Request| policy.sites_of_request(request).top_level_navigation;

        let top_level = navigation("http://a.example/", false);
        assert!(is_top_level_document(&top_level) && is_navigation(&top_level));
        let nested = navigation("http://a.example/", true);
        assert!(!is_top_level_document(&nested) && !is_navigation(&nested));

        let mut no_client = navigation("http://a.example/", false);
        no_client.client = None;
        assert!(is_top_level_document(&no_client) && !is_navigation(&no_client));

        let mut fetched = navigation("http://a.example/", false);
        fetched.mode = RequestMode::CorsMode;
        assert!(is_top_level_document(&fetched) && !is_navigation(&fetched));

        let mut prefetched = navigation("http://a.example/", false);
        prefetched.initiator = net_traits::request::Initiator::Prefetch;
        assert!(is_top_level_document(&prefetched) && !is_navigation(&prefetched));
    }

    #[test]
    fn initiator_origin_is_the_ascii_origin_or_none_when_opaque() {
        let request = subresource("http://page.example:8080/a", "http://other.example/");
        assert_eq!(
            initiator_origin(&request).as_deref(),
            Some("http://page.example:8080")
        );
        // The request's origin is still "client": the client's is used.
        let mut client_origin = request.clone();
        client_origin.origin = Origin::Client;
        assert_eq!(
            initiator_origin(&client_origin).as_deref(),
            Some("http://page.example:8080")
        );
        // A typed address: an opaque origin has no serialisation worth reporting.
        let mut opaque = request.clone();
        opaque.origin = Origin::Origin(servo_url::ImmutableOrigin::new_opaque());
        assert_eq!(initiator_origin(&opaque), None);
        // No origin and no client.
        let mut nothing = request;
        nothing.origin = Origin::Client;
        nothing.client = None;
        assert_eq!(initiator_origin(&nothing), None);
    }

    #[test]
    fn an_empty_rewrite_changes_nothing() {
        let policy = NetPolicy::new();
        let mut request = subresource("http://page.example/", "http://page.example/a?b=1");
        let applied = apply(&mut request, WebResourceRewrite::default(), &policy).unwrap();
        assert_eq!(applied, Applied::default());
        unchanged(&request, "http://page.example/a?b=1", 0);
    }

    #[test]
    fn a_rewrite_to_the_same_url_is_not_a_change() {
        let policy = NetPolicy::new();
        let mut request = subresource("http://page.example/", "http://page.example/a?b=1");
        let applied = apply(
            &mut request,
            rewrite_url("http://page.example/a?b=1"),
            &policy,
        )
        .unwrap();
        assert!(!applied.url_changed);
    }

    #[test]
    fn a_query_parameter_can_be_stripped_from_a_subresource() {
        let policy = NetPolicy::new();
        let mut request = subresource(
            "http://page.example/",
            "http://page.example/a?keep=1&utm_source=x#frag",
        );
        let applied = apply(
            &mut request,
            rewrite_url("http://page.example/a?keep=1#frag"),
            &policy,
        )
        .unwrap();
        assert!(applied.url_changed);
        assert!(!applied.origin_changed);
        assert_eq!(
            request.current_url().as_str(),
            "http://page.example/a?keep=1#frag"
        );
    }

    #[test]
    fn a_redirect_hop_rewrites_the_current_url_and_keeps_the_chain() {
        let policy = NetPolicy::new();
        let mut request = subresource("http://page.example/", "http://page.example/first");
        request.url_list.push(UrlWithBlobClaim::new(
            url("http://page.example/second?utm=1"),
            None,
        ));
        request.redirect_count = 1;
        apply(
            &mut request,
            rewrite_url("http://page.example/second"),
            &policy,
        )
        .unwrap();
        assert_eq!(request.url_list.len(), 2);
        assert_eq!(request.url().as_str(), "http://page.example/first");
        assert_eq!(request.current_url().as_str(), "http://page.example/second");
    }

    #[test]
    fn the_scheme_and_the_credentials_stay() {
        let policy = NetPolicy::new();
        let mut request = subresource("http://page.example/", "http://page.example/a");
        assert_eq!(
            apply(&mut request, rewrite_url("https://page.example/a"), &policy),
            Err(RewriteRefusal::SchemeChanged)
        );
        assert_eq!(
            apply(
                &mut request,
                rewrite_url("http://user:pw@page.example/a"),
                &policy
            ),
            Err(RewriteRefusal::CredentialsChanged)
        );
        unchanged(&request, "http://page.example/a", 0);
        // A navigation has no more right to change the scheme or the credentials.
        let mut navigation = navigation("http://a.example/", false);
        assert_eq!(
            apply(&mut navigation, rewrite_url("https://a.example/"), &policy),
            Err(RewriteRefusal::SchemeChanged)
        );
        assert_eq!(
            apply(
                &mut navigation,
                rewrite_url("http://user@b.example/"),
                &policy
            ),
            Err(RewriteRefusal::CredentialsChanged)
        );
    }

    #[test]
    fn only_http_and_https_requests_that_are_not_websockets_can_be_rewritten() {
        let policy = NetPolicy::new();
        let mut data = subresource("http://page.example/", "data:text/plain,hi");
        assert_eq!(
            apply(&mut data, rewrite_url("data:text/plain,ho"), &policy),
            Err(RewriteRefusal::NotHttp)
        );
        let mut socket = subresource("http://page.example/", "http://page.example/socket");
        socket.mode = RequestMode::WebSocket {
            protocols: Vec::new(),
            original_url: url("ws://page.example/socket"),
        };
        assert_eq!(
            apply(
                &mut socket,
                rewrite_url("http://page.example/other"),
                &policy
            ),
            Err(RewriteRefusal::NotHttp)
        );
        unchanged(&data, "data:text/plain,hi", 0);
        unchanged(&socket, "http://page.example/socket", 0);
    }

    #[test]
    fn the_origin_stays_for_a_subresource_and_for_a_frame_navigation() {
        let policy = NetPolicy::new();
        let mut subresource = subresource("http://page.example/", "http://page.example/a");
        for target in [
            "http://other.example/a",
            "http://page.example:81/a",
            "http://sub.page.example/a",
        ] {
            assert_eq!(
                apply(&mut subresource, rewrite_url(target), &policy),
                Err(RewriteRefusal::OriginChanged),
                "{target}"
            );
        }
        unchanged(&subresource, "http://page.example/a", 0);
        // A frame the page navigated: the parent's frame-src was checked against the old URL.
        let mut frame = navigation("http://a.example/", true);
        assert_eq!(
            apply(&mut frame, rewrite_url("http://b.example/"), &policy),
            Err(RewriteRefusal::OriginChanged)
        );
        unchanged(&frame, "http://a.example/", 0);
    }

    #[test]
    fn a_get_navigation_of_the_top_level_frame_may_change_the_origin() {
        let policy = NetPolicy::new();
        let mut request = navigation("http://a.example/click?to=http://b.example/", false);
        let applied = apply(&mut request, rewrite_url("http://b.example/"), &policy).unwrap();
        assert!(applied.url_changed);
        assert!(applied.origin_changed);
        assert_eq!(request.current_url().as_str(), "http://b.example/");
    }

    #[test]
    fn a_navigation_with_a_body_or_another_method_keeps_its_origin() {
        let policy = NetPolicy::new();
        let mut post = navigation("http://a.example/submit", false);
        post.method = Method::POST;
        assert_eq!(
            apply(&mut post, rewrite_url("http://b.example/"), &policy),
            Err(RewriteRefusal::OriginChanged)
        );
        // The same rewrite within the origin is fine for a POST.
        assert!(apply(&mut post, rewrite_url("http://a.example/submit2"), &policy).is_ok());
    }

    #[test]
    fn the_referrer_is_worked_out_again_when_the_origin_changes() {
        let policy = NetPolicy::new();
        // `same-origin`: the referrer that the first hop's URL allowed must not follow the navigation.
        let mut request = navigation("http://a.example/start", false);
        request.referrer_policy = ReferrerPolicy::SameOrigin;
        request.referrer = Referrer::ReferrerUrl(url("http://a.example/page?secret=1"));
        apply(&mut request, rewrite_url("http://b.example/"), &policy).unwrap();
        assert_eq!(request.referrer, Referrer::NoReferrer);

        // `strict-origin-when-cross-origin`: cross-origin now, so origin only.
        let mut request = navigation("http://a.example/start", false);
        request.referrer_policy = ReferrerPolicy::StrictOriginWhenCrossOrigin;
        request.referrer = Referrer::ReferrerUrl(url("http://a.example/page?secret=1"));
        apply(&mut request, rewrite_url("http://b.example/"), &policy).unwrap();
        assert_eq!(
            request.referrer,
            Referrer::ReferrerUrl(url("http://a.example/"))
        );

        // Within the origin the referrer is left alone.
        let mut request = navigation("http://a.example/start", false);
        request.referrer_policy = ReferrerPolicy::SameOrigin;
        request.referrer = Referrer::ReferrerUrl(url("http://a.example/page?secret=1"));
        apply(&mut request, rewrite_url("http://a.example/other"), &policy).unwrap();
        assert_eq!(
            request.referrer,
            Referrer::ReferrerUrl(url("http://a.example/page?secret=1"))
        );
    }

    #[test]
    fn a_longer_url_than_the_limit_may_only_shrink() {
        let policy = NetPolicy::new();
        let long = format!("http://page.example/a?{}", "x".repeat(MAX_URL_BYTES + 10));
        let mut request = subresource("http://page.example/", &long);
        // Growing a URL that is already past the limit is refused, shrinking it is fine.
        let longer = format!("{long}y");
        assert_eq!(
            apply(&mut request, rewrite_url(&longer), &policy),
            Err(RewriteRefusal::UrlTooLong)
        );
        assert!(apply(&mut request, rewrite_url("http://page.example/a"), &policy).is_ok());
        // A short URL may not grow past the limit.
        let mut request = subresource("http://page.example/", "http://page.example/a");
        let huge = format!("http://page.example/a?{}", "x".repeat(MAX_URL_BYTES));
        assert_eq!(
            apply(&mut request, rewrite_url(&huge), &policy),
            Err(RewriteRefusal::UrlTooLong)
        );
    }

    #[test]
    fn headers_are_set_and_removed_and_removal_comes_first() {
        let policy = NetPolicy::new();
        let mut request = subresource("http://page.example/", "http://page.example/a");
        request
            .headers
            .insert("x-old", HeaderValue::from_static("1"));
        request
            .headers
            .insert("x-same", HeaderValue::from_static("old"));
        let applied = apply(
            &mut request,
            headers(
                &[("X-New", "a b"), ("x-same", "new")],
                &["X-Old", "X-Absent", "x-same"],
            ),
            &policy,
        )
        .unwrap();
        assert_eq!(applied.headers_set, 2);
        // x-old and x-same were there; x-absent was not.
        assert_eq!(applied.headers_removed, 2);
        assert!(!request.headers.contains_key("x-old"));
        assert_eq!(request.headers["x-new"], "a b");
        // Removed first, then set: the set value stays.
        assert_eq!(request.headers["x-same"], "new");
    }

    #[test]
    fn the_forbidden_header_names_are_refused_in_every_spelling_and_for_removal_too() {
        let policy = NetPolicy::new();
        for name in [
            "Cookie",
            "COOKIE",
            "cookie2",
            "Set-Cookie",
            "Authorization",
            "Proxy-Authorization",
            "Proxy-Anything",
            "Host",
            "Content-Length",
            "Transfer-Encoding",
            "Connection",
            "Upgrade",
            "Sec-Fetch-Mode",
            "sec-gpc",
            "Referer",
            "Origin",
            "Accept-Encoding",
            "Via",
            "TE",
        ] {
            let mut request = subresource("http://page.example/", "http://page.example/a");
            request.headers.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_static("kept"),
            );
            // The whole rewrite is refused, so neither the harmless header nor the new URL is applied.
            let set = WebResourceRewrite {
                url: Some(url("http://page.example/b")),
                set_headers: vec![
                    ("X-Harmless".to_owned(), "1".to_owned()),
                    (name.to_owned(), "x".to_owned()),
                ],
                remove_headers: Vec::new(),
            };
            assert_eq!(
                apply(&mut request, set, &policy),
                Err(RewriteRefusal::ForbiddenHeader),
                "set {name}"
            );
            unchanged(&request, "http://page.example/a", 1);
            assert!(!request.headers.contains_key("x-harmless"));
            assert_eq!(
                apply(&mut request, headers(&[], &[name]), &policy),
                Err(RewriteRefusal::ForbiddenHeader),
                "remove {name}"
            );
            assert_eq!(request.headers.len(), 1, "{name} is still there");
        }
    }

    #[test]
    fn invalid_oversized_and_too_many_header_changes_are_refused() {
        let policy = NetPolicy::new();
        let mut request = subresource("http://page.example/", "http://page.example/a");
        for (name, value) in [
            ("bad name", "v"),
            ("", "v"),
            ("x-a", "line\r\nbreak"),
            ("x-a", "nul\0"),
            ("x-a", "caf\u{e9}"),
        ] {
            assert_eq!(
                apply(&mut request, headers(&[(name, value)], &[]), &policy),
                Err(RewriteRefusal::InvalidHeader),
                "{name:?}: {value:?}"
            );
        }
        let long_name = "x".repeat(MAX_HEADER_NAME_BYTES + 1);
        assert_eq!(
            apply(&mut request, headers(&[(&long_name, "v")], &[]), &policy),
            Err(RewriteRefusal::InvalidHeader)
        );
        let long_value = "v".repeat(MAX_HEADER_VALUE_BYTES + 1);
        assert_eq!(
            apply(&mut request, headers(&[("x-a", &long_value)], &[]), &policy),
            Err(RewriteRefusal::InvalidHeader)
        );
        let many: Vec<(String, String)> = (0..=MAX_HEADER_CHANGES)
            .map(|index| (format!("x-{index}"), "v".to_owned()))
            .collect();
        let many = WebResourceRewrite {
            url: None,
            set_headers: many,
            remove_headers: Vec::new(),
        };
        assert_eq!(
            apply(&mut request, many, &policy),
            Err(RewriteRefusal::TooManyHeaders)
        );
        // Exactly the limit is fine.
        let at_limit = WebResourceRewrite {
            url: None,
            set_headers: (0..MAX_HEADER_CHANGES)
                .map(|index| (format!("x-{index}"), "v".to_owned()))
                .collect(),
            remove_headers: Vec::new(),
        };
        assert_eq!(
            apply(&mut request, at_limit, &policy).unwrap().headers_set,
            MAX_HEADER_CHANGES
        );
    }

    #[test]
    fn a_refusal_names_the_rule_and_never_a_value() {
        for refusal in [
            RewriteRefusal::NotHttp,
            RewriteRefusal::SchemeChanged,
            RewriteRefusal::OriginChanged,
            RewriteRefusal::CredentialsChanged,
            RewriteRefusal::UrlTooLong,
            RewriteRefusal::TooManyHeaders,
            RewriteRefusal::InvalidHeader,
            RewriteRefusal::ForbiddenHeader,
        ] {
            let text = refusal.to_string();
            assert!(!text.is_empty());
            assert!(!text.contains("://"), "{text}");
        }
    }
}
