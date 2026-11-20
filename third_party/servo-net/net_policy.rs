// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: the network policy handle and the sites of a request (EC5, EC6, EC25, EC14, EC17, EC38 core; decisions
// D2 to D4 of the net-1 lane). A new Mercury file; nothing in it is copied from Servo or another project.
// The hooks that ask it are in `http_loader.rs`, `resource_thread.rs`, `fetch/methods.rs` and
// `websocket_loader.rs`; `MERCURY.md` lists them.

//! The network policy: one cheap-to-clone handle that says what Mercury's network layer allows.
//!
//! Servo 0.8 has no policy at all: every cookie goes to every request and every `Set-Cookie` is stored,
//! whoever the page is. [`NetPolicy`] is a plain data handle (an `Arc` around atomics and two small lists)
//! that lives in `servo-net`, is a field of [`HttpState`](crate::http_loader::HttpState), and has one
//! process-wide instance ([`NetPolicy::process`]) that both the public and the private `HttpState` use and
//! that Mercury configures once at start. [`NetPolicy::new`] is Servo 0.8: every method answers as the
//! unpatched crate behaves, so the existing tests do not change.
//!
//! # The site of a request
//!
//! Cookie blocking and SameSite work on **sites**, the scheme-less registrable domain ([`SiteKey`]):
//! `https://a.example` and `http://www.a.example` are one site, `localhost`, `127.0.0.1` and a single-label
//! name are each their own, and an opaque origin has no site and is third party. [`RequestSites`] says, for
//! one request, which site the top-level page has, which site the request goes to, which site started it,
//! and whether the request is first or third party.
//!
//! The top-level site of a request is found in this order (decision D3):
//!
//! 1. the request client's `network_partition_key`, when it has one (every document and worker request
//!    that goes through `Document::determine_the_network_partition_key`);
//! 2. a **top-level navigation**: mode `Navigate`, destination `Document`, no initiator, and a client that is
//!    not in a nested browsing context. Its site is the site of its current URL, at every hop. The
//!    destination alone is not enough: a frame navigated by script or by a link is also fetched with the
//!    destination `Document` (`RequestClient::is_nested_browsing_context` is the reliable flag);
//! 3. the policy's own bounded record (256 web views, the least recently written is evicted) of the site and
//!    the origin that each web view's main frame last started loading, found by `target_webview_id`.
//!    [`NetPolicy::note_main_frame`] writes it. This covers a nested navigation (its request has no
//!    partition key) and every worker (a worker's requests carry the web view id but no key);
//! 4. otherwise the top-level site is **unknown** and the request is [`Party::Unknown`], which every
//!    security decision treats as third party (fail closed).
//!
//! The record is racy while a navigation away is in flight: a request of the page being left may be judged
//! against the page being entered for a moment. The document's own key wins whenever it exists.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use http::Method;
use net_traits::ReferrerPolicy;
use net_traits::blob_url_store::UrlWithBlobClaim;
use net_traits::pub_domains::{Site, reg_suffix};
use net_traits::request::{
    Destination, Initiator, Origin, Request, RequestBuilder, RequestClient, RequestMode,
};
use parking_lot::{Mutex, RwLock};
use servo_base::id::WebViewId;
use servo_url::{Host, ImmutableOrigin, ServoUrl};

use crate::mercury_auth::AuthLimiter;

/// A site: the scheme-less registrable domain, in lower case. An IP literal or a single-label host is its
/// own key (an IPv6 literal keeps its brackets). The empty string stands for an opaque origin, which has no
/// site and is never the same site as anything.
pub type SiteKey = String;

/// How many web views the record of top-level sites keeps (decision D3).
pub const MAX_RECORDED_WEB_VIEWS: usize = 256;

/// The site of an origin: see [`SiteKey`].
pub fn site_key_of_origin(origin: &ImmutableOrigin) -> SiteKey {
    match origin {
        ImmutableOrigin::Opaque(_) => SiteKey::new(),
        ImmutableOrigin::Tuple(_, Host::Domain(domain), _) => {
            reg_suffix(domain).to_ascii_lowercase()
        },
        ImmutableOrigin::Tuple(_, host, _) => host.to_string().to_ascii_lowercase(),
    }
}

/// The site of a URL: see [`SiteKey`].
pub fn site_key_of_url(url: &ServoUrl) -> SiteKey {
    site_key_of_origin(&url.origin())
}

/// The site key of the HTML [site](https://html.spec.whatwg.org/multipage/#site) a partition key holds. The
/// scheme is dropped (decision D4). An opaque site has no key.
pub fn site_key_of_site(site: &Site) -> SiteKey {
    match site {
        Site::Opaque(_) => SiteKey::new(),
        Site::Tuple {
            host: Host::Domain(domain),
            ..
        } => domain.to_ascii_lowercase(),
        Site::Tuple { host, .. } => host.to_string().to_ascii_lowercase(),
    }
}

/// Whether a request goes to the site of the top-level page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Party {
    /// The request's site is the top-level page's site.
    First,
    /// The request's site differs from the top-level page's site (an opaque site always differs).
    Third,
    /// The top-level site could not be found: treated as third party by every security decision.
    Unknown,
}

/// What the policy knows about the sites of one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestSites {
    /// The top-level page's site. `None` when it is unknown; `Some("")` when the top-level origin is opaque.
    pub top_level: Option<SiteKey>,
    /// The site of the request's current URL (so, of the redirect hop the request is at).
    pub request: SiteKey,
    /// The site of the origin that started the request. `None` for an opaque or absent origin (a typed URL,
    /// a sandboxed frame, a worker started from a `data:` URL).
    pub initiator: Option<SiteKey>,
    /// The request is a top-level navigation (decision D3, rule 2).
    pub top_level_navigation: bool,
    /// First, third or unknown party.
    pub party: Party,
    /// The top-level page's origin (ASCII serialisation). Filled only from the policy's web view record.
    pub top_level_origin: Option<String>,
}

/// Counters of what the policy did, for a later shield in the interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetPolicyStats {
    /// How many times the policy withheld cookies from a request or a document, or ignored a `Set-Cookie`.
    /// A request that is judged twice (its `Cookie` header and its `Set-Cookie` answer) counts twice.
    pub third_party_cookies_blocked: u64,
}

/// The pieces of a request that the policy reads. A [`Request`] and a [`RequestBuilder`] carry the same
/// fields (the WebSocket handshake only has the builder), so both are read through this one view.
pub(crate) struct RequestParts<'a> {
    pub(crate) urls: &'a [UrlWithBlobClaim],
    pub(crate) client: Option<&'a RequestClient>,
    pub(crate) origin: &'a Origin,
    pub(crate) mode: &'a RequestMode,
    pub(crate) destination: Destination,
    pub(crate) initiator: Initiator,
    pub(crate) webview: Option<WebViewId>,
    pub(crate) method: &'a Method,
}

impl<'a> RequestParts<'a> {
    pub(crate) fn of_request(request: &'a Request) -> Self {
        RequestParts {
            urls: &request.url_list[..],
            client: request.client.as_ref(),
            origin: &request.origin,
            mode: &request.mode,
            destination: request.destination,
            initiator: request.initiator,
            webview: request.target_webview_id,
            method: &request.method,
        }
    }

    pub(crate) fn of_builder(request: &'a RequestBuilder) -> Self {
        RequestParts {
            // A builder without a URL list has just its URL.
            urls: if request.url_list.is_empty() {
                std::slice::from_ref(&request.url)
            } else {
                &request.url_list[..]
            },
            client: request.client.as_ref(),
            origin: &request.origin,
            mode: &request.mode,
            destination: request.destination,
            initiator: request.initiator,
            webview: request.target_webview_id,
            method: &request.method,
        }
    }

    /// The site of the current (last) URL of the request, `""` if it has none or an opaque origin.
    pub(crate) fn current_site(&self) -> SiteKey {
        self.urls
            .last()
            .map(|url| site_key_of_origin(&url.origin()))
            .unwrap_or_default()
    }

    /// The origin that started the request: the request's own origin, or its client's when the request still
    /// says "client".
    pub(crate) fn initiator_origin(&self) -> Option<&'a ImmutableOrigin> {
        match self.origin {
            Origin::Origin(origin) => Some(origin),
            Origin::Client => match self.client.map(|client| &client.origin) {
                Some(Origin::Origin(origin)) => Some(origin),
                _ => None,
            },
        }
    }

    /// The site that started the request; `None` for an opaque or missing origin.
    pub(crate) fn initiator_site(&self) -> Option<SiteKey> {
        self.initiator_origin()
            .map(site_key_of_origin)
            .filter(|key| !key.is_empty())
    }

    /// Decision D3, rule 2: a top-level navigation is a navigation of a top-level browsing context. The
    /// destination `Document` alone does not say so (fact 8a of the lane brief).
    pub(crate) fn is_top_level_navigation(&self) -> bool {
        matches!(self.mode, RequestMode::Navigate)
            && matches!(self.destination, Destination::Document)
            && matches!(self.initiator, Initiator::None)
            && self
                .client
                .is_some_and(|client| !client.is_nested_browsing_context)
    }
}

/// The top-level site and origin that a web view's main frame last started loading.
#[derive(Clone, Debug)]
struct TopLevelRecord {
    site: SiteKey,
    origin: String,
}

/// A bounded record, least recently written first out (decision D3, rule 3).
#[derive(Default)]
struct WebViewRecords {
    entries: HashMap<WebViewId, TopLevelRecord>,
    order: VecDeque<WebViewId>,
}

impl WebViewRecords {
    fn insert(&mut self, web_view: WebViewId, record: TopLevelRecord) {
        if self.entries.insert(web_view, record).is_some() {
            // Written again: it is now the most recent.
            self.order.retain(|known| *known != web_view);
        }
        self.order.push_back(web_view);
        while self.entries.len() > MAX_RECORDED_WEB_VIEWS {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn get(&self, web_view: &WebViewId) -> Option<&TopLevelRecord> {
        self.entries.get(web_view)
    }
}

struct Inner {
    block_third_party_cookies: AtomicBool,
    same_site_enforcement: AtomicBool,
    same_site_lax_default: AtomicBool,
    global_privacy_control: AtomicBool,
    referrer_cap: AtomicBool,
    /// Top-level sites on which third-party cookies are allowed.
    cookies_allowed_on: RwLock<HashSet<SiteKey>>,
    /// Top-level sites on which mixed content is allowed.
    mixed_content_allowed_on: RwLock<HashSet<SiteKey>>,
    web_views: Mutex<WebViewRecords>,
    third_party_cookies_blocked: AtomicU64,
    auth_limiter: AuthLimiter,
}

/// The network policy handle. Cloning it is cheap and every clone sees the same state.
#[derive(Clone)]
pub struct NetPolicy(Arc<Inner>);

impl Default for NetPolicy {
    fn default() -> Self {
        NetPolicy::new()
    }
}

impl fmt::Debug for NetPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetPolicy")
            .field(
                "block_third_party_cookies",
                &self.block_third_party_cookies(),
            )
            .field("same_site_enforcement", &self.same_site_enforcement())
            .field("same_site_lax_default", &self.same_site_lax_default())
            .field("global_privacy_control", &self.global_privacy_control())
            .field("referrer_cap", &self.referrer_cap())
            .finish_non_exhaustive()
    }
}

static PROCESS_POLICY: OnceLock<NetPolicy> = OnceLock::new();

impl NetPolicy {
    /// A policy that behaves as Servo 0.8 does in every method: nothing blocked, no SameSite enforcement, no
    /// `Sec-GPC`, no referrer cap, mixed content as the specification says.
    pub fn new() -> NetPolicy {
        NetPolicy(Arc::new(Inner {
            block_third_party_cookies: AtomicBool::new(false),
            same_site_enforcement: AtomicBool::new(false),
            same_site_lax_default: AtomicBool::new(false),
            global_privacy_control: AtomicBool::new(false),
            referrer_cap: AtomicBool::new(false),
            cookies_allowed_on: RwLock::new(HashSet::new()),
            mixed_content_allowed_on: RwLock::new(HashSet::new()),
            web_views: Mutex::new(WebViewRecords::default()),
            third_party_cookies_blocked: AtomicU64::new(0),
            auth_limiter: AuthLimiter::default(),
        }))
    }

    /// The one policy of this process. Both `HttpState`s (public and private) take it, and so does Mercury
    /// when it configures the engine. It starts as [`NetPolicy::new`].
    pub fn process() -> NetPolicy {
        PROCESS_POLICY.get_or_init(NetPolicy::new).clone()
    }

    /// Whether two handles are the same policy.
    pub fn is_same_policy(&self, other: &NetPolicy) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    // The sites of a request (EC38 core).

    /// The sites of a request, at the hop it is at now.
    pub fn sites_of_request(&self, request: &Request) -> RequestSites {
        self.sites_of_parts(&RequestParts::of_request(request))
    }

    /// The sites of a request that is still a builder (the WebSocket handshake). The same rules.
    pub fn sites_of_builder(&self, request: &RequestBuilder) -> RequestSites {
        self.sites_of_parts(&RequestParts::of_builder(request))
    }

    /// The sites of a document that reads or writes `document.cookie`. The document's own partition key is
    /// the top-level site; when it has none, the web view's record is used.
    pub fn sites_of_document(
        &self,
        url: &ServoUrl,
        partition_key: &Option<Site>,
        web_view: Option<WebViewId>,
    ) -> RequestSites {
        let request = site_key_of_url(url);
        let mut top_level_origin = None;
        let top_level = if let Some(site) = partition_key {
            Some(site_key_of_site(site))
        } else if let Some(record) = web_view.and_then(|web_view| self.record_of(web_view)) {
            top_level_origin = Some(record.origin);
            Some(record.site)
        } else {
            None
        };
        let party = party_of(top_level.as_deref(), &request);
        RequestSites {
            top_level,
            request,
            initiator: None,
            top_level_navigation: false,
            party,
            top_level_origin,
        }
    }

    pub(crate) fn sites_of_parts(&self, parts: &RequestParts<'_>) -> RequestSites {
        let request = parts.current_site();
        let top_level_navigation = parts.is_top_level_navigation();
        let mut top_level_origin = None;
        let top_level = if let Some(site) = parts
            .client
            .and_then(|client| client.network_partition_key.as_ref())
        {
            // Rule 1: the document's own key.
            Some(site_key_of_site(site))
        } else if top_level_navigation {
            // Rule 2: a top-level navigation is first party by construction.
            Some(request.clone())
        } else if let Some(record) = parts.webview.and_then(|web_view| self.record_of(web_view)) {
            // Rule 3: the record of the web view's main frame.
            top_level_origin = Some(record.origin);
            Some(record.site)
        } else {
            // Rule 4: unknown.
            None
        };
        let party = party_of(top_level.as_deref(), &request);
        RequestSites {
            top_level,
            request,
            initiator: parts.initiator_site(),
            top_level_navigation,
            party,
            top_level_origin,
        }
    }

    /// Records the top-level site and origin of a web view whose main frame starts loading. Does nothing for
    /// a request that is not a top-level navigation or has no web view. Called once per hop from
    /// `http_network_or_cache_fetch`, before the cookies are judged.
    pub fn note_main_frame(&self, request: &Request) {
        let parts = RequestParts::of_request(request);
        if !parts.is_top_level_navigation() {
            return;
        }
        let (Some(web_view), Some(url)) = (parts.webview, parts.urls.last()) else {
            return;
        };
        let origin = url.origin();
        let record = TopLevelRecord {
            site: site_key_of_origin(&origin),
            origin: origin.ascii_serialization().into_owned(),
        };
        self.0.web_views.lock().insert(web_view, record);
    }

    /// The top-level site the record holds for a web view.
    pub fn recorded_top_level_site(&self, web_view: WebViewId) -> Option<SiteKey> {
        self.record_of(web_view).map(|record| record.site)
    }

    /// How many web views the record holds now (at most [`MAX_RECORDED_WEB_VIEWS`]).
    pub fn recorded_web_views(&self) -> usize {
        self.0.web_views.lock().entries.len()
    }

    fn record_of(&self, web_view: WebViewId) -> Option<TopLevelRecord> {
        self.0.web_views.lock().get(&web_view).cloned()
    }

    // EC5, EC6: cookies.

    /// Block cookies of third-party requests (and unknown ones). Off in [`NetPolicy::new`].
    pub fn set_block_third_party_cookies(&self, on: bool) {
        self.0
            .block_third_party_cookies
            .store(on, Ordering::Relaxed);
    }

    pub fn block_third_party_cookies(&self) -> bool {
        self.0.block_third_party_cookies.load(Ordering::Relaxed)
    }

    /// Allows (or stops allowing) third-party cookies while the top-level page is on `top_level`. A site is
    /// written as [`SiteKey`] (`example.com`); an empty key is ignored.
    pub fn allow_third_party_cookies_on(&self, top_level: SiteKey, allow: bool) {
        let top_level = top_level.to_ascii_lowercase();
        if top_level.is_empty() {
            return;
        }
        let mut allowed = self.0.cookies_allowed_on.write();
        if allow {
            allowed.insert(top_level);
        } else {
            allowed.remove(&top_level);
        }
    }

    /// Whether cookies may go with, and come from, a request with these sites. Counts a withheld request in
    /// [`NetPolicy::stats`].
    pub fn cookies_allowed(&self, sites: &RequestSites) -> bool {
        if !self.block_third_party_cookies() || sites.party == Party::First {
            return true;
        }
        if let Some(top_level) = sites.top_level.as_deref()
            && !top_level.is_empty()
            && self.0.cookies_allowed_on.read().contains(top_level)
        {
            return true;
        }
        self.0
            .third_party_cookies_blocked
            .fetch_add(1, Ordering::Relaxed);
        false
    }

    /// Enforce the `SameSite` attribute (see `mercury_cookies.rs`). Off in [`NetPolicy::new`].
    pub fn set_same_site_enforcement(&self, on: bool) {
        self.0.same_site_enforcement.store(on, Ordering::Relaxed);
    }

    pub fn same_site_enforcement(&self) -> bool {
        self.0.same_site_enforcement.load(Ordering::Relaxed)
    }

    /// Treat a cookie without a `SameSite` attribute as `Lax`. Off in [`NetPolicy::new`] and in Mercury.
    pub fn set_same_site_lax_default(&self, on: bool) {
        self.0.same_site_lax_default.store(on, Ordering::Relaxed);
    }

    pub fn same_site_lax_default(&self) -> bool {
        self.0.same_site_lax_default.load(Ordering::Relaxed)
    }

    pub fn stats(&self) -> NetPolicyStats {
        NetPolicyStats {
            third_party_cookies_blocked: self.0.third_party_cookies_blocked.load(Ordering::Relaxed),
        }
    }

    // EC25: Global Privacy Control.

    /// Send `Sec-GPC: 1` with every request. Off in [`NetPolicy::new`].
    pub fn set_global_privacy_control(&self, on: bool) {
        self.0.global_privacy_control.store(on, Ordering::Relaxed);
    }

    pub fn global_privacy_control(&self) -> bool {
        self.0.global_privacy_control.load(Ordering::Relaxed)
    }

    // EC14: the referrer cap. The hook is in `fetch/methods.rs`.

    /// Cap the referrer policy of every request. Off in [`NetPolicy::new`].
    pub fn set_referrer_cap(&self, on: bool) {
        self.0.referrer_cap.store(on, Ordering::Relaxed);
    }

    pub fn referrer_cap(&self) -> bool {
        self.0.referrer_cap.load(Ordering::Relaxed)
    }

    /// The referrer policy to use for a request whose policy is `policy`. With the cap on, the three policies
    /// that send more than the origin across origins (`unsafe-url`, `no-referrer-when-downgrade` and
    /// `origin-when-cross-origin`) become `strict-origin-when-cross-origin`. The cap never loosens anything:
    /// every other policy is returned as it is.
    pub fn cap_referrer_policy(&self, policy: ReferrerPolicy) -> ReferrerPolicy {
        if !self.referrer_cap() {
            return policy;
        }
        match policy {
            ReferrerPolicy::UnsafeUrl
            | ReferrerPolicy::NoReferrerWhenDowngrade
            | ReferrerPolicy::OriginWhenCrossOrigin => ReferrerPolicy::StrictOriginWhenCrossOrigin,
            other => other,
        }
    }

    // EC17: mixed content. The hook is in `fetch/methods.rs`.

    /// Allows (or stops allowing) mixed content while the top-level page is on `top_level`.
    pub fn set_mixed_content_allowed_on(&self, top_level: SiteKey, allow: bool) {
        let top_level = top_level.to_ascii_lowercase();
        if top_level.is_empty() {
            return;
        }
        let mut allowed = self.0.mixed_content_allowed_on.write();
        if allow {
            allowed.insert(top_level);
        } else {
            allowed.remove(&top_level);
        }
    }

    /// Whether the user has allowed mixed content for the top-level site of these sites. Nothing is allowed
    /// by default, which is what the specification says.
    pub fn mixed_content_allowed(&self, sites: &RequestSites) -> bool {
        sites.top_level.as_deref().is_some_and(|top_level| {
            !top_level.is_empty() && self.0.mixed_content_allowed_on.read().contains(top_level)
        })
    }

    // EC29: sign-in prompts for a 401 answered to `fetch` or XHR. The state is here, the logic in
    // `mercury_auth.rs`.

    pub fn auth_limiter(&self) -> &AuthLimiter {
        &self.0.auth_limiter
    }
}

/// First, third or unknown, from the top-level site (if any) and the request's site.
fn party_of(top_level: Option<&str>, request: &str) -> Party {
    match top_level {
        None => Party::Unknown,
        Some(top_level) if !top_level.is_empty() && !request.is_empty() && top_level == request => {
            Party::First
        },
        Some(_) => Party::Third,
    }
}
