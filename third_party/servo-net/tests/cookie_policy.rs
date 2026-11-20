// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: tests for third-party cookies blocked, SameSite enforced and Sec-GPC sent; see net_policy.rs and
// mercury_cookies.rs.

//! Third-party cookie blocking, SameSite and Global Privacy Control, end to end through the fetch path.
//!
//! Each test starts a local server that records what it was sent (`Cookie`, `Sec-GPC`) and answers with the
//! `Set-Cookie` lines and redirects the test asks for in the URL. A request is built by hand as script
//! would build it: a client whose partition key is the top-level site, an origin (the document that made it),
//! a mode and a destination. The server is reached as `localhost` and as `127.0.0.1`, two different sites,
//! so no host table is needed; the page the request is "made from" is only an origin and is never contacted.
//!
//! The groups, in order: the third-party rules for `Cookie` and `Set-Cookie`; the web view record that a
//! top-level navigation writes; SameSite for sending and for storing; `Sec-GPC`; the messages of the
//! resource thread (`document.cookie` and `cookieStore`); and the policy that does nothing. The last test
//! scans `http_loader.rs` and fails if a cookie is attached or stored outside the policy check.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use http::header::{self, HeaderValue};
use http::{Method, StatusCode};
use http_body_util::combinators::BoxBody;
use hyper::body::{Bytes, Incoming};
use hyper::{Request as HyperRequest, Response as HyperResponse};
use hyper_serde::Serde;
use net::connector::CACertificates;
use net::fetch::methods::FetchContext;
use net::net_policy::{NetPolicy, Party};
use net::protocols::ProtocolRegistry;
use net::resource_thread::new_core_resource_thread;
use net::test_util::Server;
use net_traits::request::{CredentialsMode, Referrer, RequestBuilder, RequestMode};
use net_traits::response::Response;
use net_traits::{
    CookieAsyncResponse, CookieData, CookieSource, CoreResourceMsg, CoreResourceThread,
};
use parking_lot::Mutex;
use profile_traits::mem::ProfilerChan as MemProfilerChan;
use profile_traits::time::ProfilerChan;
use servo_base::generic_channel::{self, GenericCallback, GenericReceiver};
use servo_base::id::{
    CookieStoreId, Index, PipelineNamespaceId, TEST_PIPELINE_ID, TEST_WEBVIEW_ID, WebViewId,
};

use crate::net_policy::{
    PROCESS_POLICY_LOCK, claim, client, navigation, nested_navigation, origin_of, parse, site_of,
    subresource, top_level_navigation, web_view,
};
use crate::{
    create_generic_embedder_proxy, fetch_with_context, make_body, make_server, make_ssl_server,
    new_fetch_context,
};

// ---------------------------------------------------------------------------------------------------
// The server and the helpers.
// ---------------------------------------------------------------------------------------------------

/// The page a request is made from. Only its origin and its site matter; nothing listens there.
const PAGE_LOCALHOST: &str = "http://localhost:1/";
const PAGE_LOOPBACK: &str = "http://127.0.0.1:1/";

/// One request as the server saw it.
#[derive(Clone, Debug)]
struct Seen {
    method: Method,
    /// The path and the query.
    path: String,
    cookie: Option<String>,
    sec_gpc: Option<String>,
}

impl Seen {
    fn cookie_names(&self) -> Vec<String> {
        cookie_names(self.cookie.as_deref())
    }
}

/// The names in a `Cookie` header (or in a cookie string), sorted.
fn cookie_names(header: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = header
        .unwrap_or("")
        .split("; ")
        .filter(|pair| !pair.is_empty())
        .map(|pair| pair.split('=').next().unwrap_or("").to_owned())
        .collect();
    names.sort();
    names
}

/// A sorted list of names, to compare with `cookie_names`.
fn names(list: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = list.iter().map(|name| (*name).to_owned()).collect();
    names.sort();
    names
}

/// Answers every request with 200, or with the `Set-Cookie` lines (`set=`) and the redirect (`to=`) that the
/// query asks for, and records it. Every answer allows any origin to read it (the CORS test needs that) and
/// forbids caching, so that no test is served from the HTTP cache.
fn answer_probe(
    seen: Arc<Mutex<Vec<Seen>>>,
    tls: bool,
) -> impl Fn(HyperRequest<Incoming>, &mut HyperResponse<BoxBody<Bytes, hyper::Error>>)
+ Send
+ Sync
+ 'static {
    move |request, response| {
        let header_of = |name: &str| {
            request
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        seen.lock().push(Seen {
            method: request.method().clone(),
            path: request
                .uri()
                .path_and_query()
                .map(|path| path.as_str().to_owned())
                .unwrap_or_default(),
            cookie: header_of("cookie"),
            sec_gpc: header_of("sec-gpc"),
        });

        let mut location = None;
        for (key, value) in
            url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
        {
            match &*key {
                "set" => {
                    response
                        .headers_mut()
                        .append(header::SET_COOKIE, HeaderValue::from_str(&value).unwrap());
                },
                "to" => location = Some(value.into_owned()),
                _ => {},
            }
        }
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        if request.method() == Method::OPTIONS {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, PUT, POST"),
            );
        }
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if tls {
            // The TLS test server serves one connection at a time.
            headers.insert(header::CONNECTION, HeaderValue::from_static("close"));
        }
        match location {
            Some(location) => {
                *response.status_mut() = StatusCode::FOUND;
                response
                    .headers_mut()
                    .insert(header::LOCATION, HeaderValue::from_str(&location).unwrap());
            },
            None => *response.body_mut() = make_body(b"ok".to_vec()),
        }
    }
}

/// The recording server. Closed when dropped.
struct Probe {
    server: Option<Server>,
    scheme: &'static str,
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
    counter: AtomicUsize,
}

impl Probe {
    fn new() -> Probe {
        Probe::start(false)
    }

    fn tls() -> Probe {
        Probe::start(true)
    }

    fn start(tls: bool) -> Probe {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = answer_probe(seen.clone(), tls);
        let (server, url) = if tls {
            make_ssl_server(handler)
        } else {
            make_server(handler)
        };
        Probe {
            server: Some(server),
            scheme: if tls { "https" } else { "http" },
            port: url.port().expect("the server has a port"),
            seen,
            counter: AtomicUsize::new(0),
        }
    }

    /// A URL of the server under a host name, made unique so that nothing is answered from a cache.
    fn url(&self, host: &str, path: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let separator = if path.contains('?') { '&' } else { '?' };
        format!(
            "{}://{host}:{}{path}{separator}k={n}",
            self.scheme, self.port
        )
    }

    fn localhost(&self, path: &str) -> String {
        self.url("localhost", path)
    }

    fn loopback(&self, path: &str) -> String {
        self.url("127.0.0.1", path)
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().clone()
    }

    /// The requests whose path starts with `prefix`.
    fn requests(&self, prefix: &str) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|seen| seen.path.starts_with(prefix))
            .collect()
    }

    /// The only request whose path starts with `prefix`.
    fn only(&self, prefix: &str) -> Seen {
        let mut requests = self.requests(prefix);
        assert_eq!(
            requests.len(),
            1,
            "one request for {prefix}, saw {:?}",
            self.seen()
        );
        requests.remove(0)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            let _ = server.close_channel.send(());
        }
    }
}

fn query(pairs: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

/// A path that answers with these `Set-Cookie` lines.
fn sets(path: &str, cookies: &[&str]) -> String {
    let pairs: Vec<(&str, &str)> = cookies.iter().map(|cookie| ("set", *cookie)).collect();
    format!("{path}?{}", query(&pairs))
}

/// A path that redirects to `to` (and sets these cookies on the redirect response itself).
fn redirect(path: &str, to: &str, cookies: &[&str]) -> String {
    let mut pairs = vec![("to", to)];
    pairs.extend(cookies.iter().map(|cookie| ("set", *cookie)));
    format!("{path}?{}", query(&pairs))
}

/// A fetch context whose policy has been set up by `setup`. The tests' contexts never use the process policy.
fn context_with(setup: impl FnOnce(&NetPolicy)) -> FetchContext {
    let context = new_fetch_context(None, None);
    setup(&context.state.net_policy);
    context
}

/// Third-party cookies blocked.
fn blocking() -> FetchContext {
    context_with(|policy| policy.set_block_third_party_cookies(true))
}

/// SameSite enforced, third-party cookies not blocked: what is under test is the SameSite rule alone.
fn same_site() -> FetchContext {
    context_with(|policy| policy.set_same_site_enforcement(true))
}

/// Puts cookies in the jar as an earlier visit would have.
fn seed(context: &FetchContext, url: &str, cookies: &[&str]) {
    let url = parse(url);
    for cookie in cookies {
        let cookie = net::cookie::ServoCookie::from_cookie_string(cookie, &url, CookieSource::HTTP)
            .unwrap_or_else(|| panic!("{cookie} is a cookie for {url}"));
        context
            .state
            .cookie_jar
            .write()
            .push(cookie, &url, CookieSource::HTTP);
    }
}

/// The names of the cookies the jar holds for a URL (read without the policy).
fn jar(context: &FetchContext, url: &str) -> Vec<String> {
    let cookies = context
        .state
        .cookie_jar
        .write()
        .cookies_for_url(&parse(url), CookieSource::HTTP);
    cookie_names(cookies.as_deref())
}

fn with_credentials(builder: RequestBuilder) -> RequestBuilder {
    builder
        .credentials_mode(CredentialsMode::Include)
        .pipeline_id(Some(TEST_PIPELINE_ID))
        .policy_container(Default::default())
}

/// A subresource of a document at `page`, whose page is also the top level.
fn in_page(url: &str, page: &str) -> RequestBuilder {
    with_credentials(subresource(url, page, page))
}

/// A URL typed in the address bar of a web view: a top-level navigation with no initiator.
fn typed(url: &str) -> RequestBuilder {
    with_credentials(top_level_navigation(url, TEST_WEBVIEW_ID))
}

/// A top-level navigation that a page started (a link, `location.href`).
fn from_page(url: &str, page: &str) -> RequestBuilder {
    with_credentials(navigation(url, Some(page), false, TEST_WEBVIEW_ID))
}

/// A frame that navigates, inside the page the policy has recorded for the web view.
fn in_frame(url: &str, parent: &str) -> RequestBuilder {
    with_credentials(nested_navigation(url, parent, TEST_WEBVIEW_ID))
}

fn run(context: &mut FetchContext, builder: RequestBuilder) -> Response {
    let url = builder.url.to_string();
    let response = fetch_with_context(builder.build(), context);
    assert!(
        !response.is_network_error(),
        "{url} must load: {:?}",
        response.get_network_error()
    );
    response
}

/// Records, as a load of the web view's main frame would, the page the web view is on.
fn the_web_view_is_on(context: &FetchContext, page: &str) {
    context
        .state
        .net_policy
        .note_main_frame(&typed(page).build());
}

// ---------------------------------------------------------------------------------------------------
// Third-party cookies blocked (EC5).
// ---------------------------------------------------------------------------------------------------

#[test]
fn a_third_party_set_cookie_is_not_stored_and_a_first_party_one_is() {
    let probe = Probe::new();
    let mut context = blocking();

    // A page on localhost loads something from 127.0.0.1, which answers with a cookie.
    let third = probe.loopback(&sets("/third", &["tracker=1"]));
    run(&mut context, in_page(&third, PAGE_LOCALHOST));
    assert_eq!(jar(&context, &probe.loopback("/")), names(&[]));

    // The control: the same answer from the page's own site is stored.
    let first = probe.localhost(&sets("/first", &["session=1"]));
    run(&mut context, in_page(&first, PAGE_LOCALHOST));
    assert_eq!(jar(&context, &probe.localhost("/")), names(&["session"]));

    // The third-party request was refused twice: for the `Cookie` header it would have had, and for the
    // `Set-Cookie` of its answer. The first-party one was never refused.
    assert_eq!(
        context.state.net_policy.stats().third_party_cookies_blocked,
        2
    );
}

#[test]
fn a_stored_cookie_is_not_sent_to_a_third_party_request_but_is_to_a_first_party_one() {
    let probe = Probe::new();
    let mut context = blocking();
    seed(&context, &probe.loopback("/"), &["tracker=1"]);
    seed(&context, &probe.localhost("/"), &["session=1"]);

    run(
        &mut context,
        in_page(&probe.loopback("/third"), PAGE_LOCALHOST),
    );
    assert_eq!(probe.only("/third").cookie, None);

    run(
        &mut context,
        in_page(&probe.localhost("/first"), PAGE_LOCALHOST),
    );
    assert_eq!(probe.only("/first").cookie.as_deref(), Some("session=1"));

    // The same cookie goes to 127.0.0.1 when the page is on 127.0.0.1.
    run(
        &mut context,
        in_page(&probe.loopback("/own"), PAGE_LOOPBACK),
    );
    assert_eq!(probe.only("/own").cookie.as_deref(), Some("tracker=1"));
}

#[test]
fn a_cookie_the_caller_put_in_the_header_map_is_not_sent_to_a_third_party_request() {
    let probe = Probe::new();
    let mut context = blocking();
    let mut headers = http::HeaderMap::new();
    headers.insert(header::COOKIE, HeaderValue::from_static("smuggled=1"));

    run(
        &mut context,
        in_page(&probe.loopback("/third"), PAGE_LOCALHOST).headers(headers.clone()),
    );
    assert_eq!(probe.only("/third").cookie, None);

    // A request that is allowed cookies keeps the header it was given when the jar has nothing to add.
    run(
        &mut context,
        in_page(&probe.localhost("/first"), PAGE_LOCALHOST).headers(headers),
    );
    assert_eq!(probe.only("/first").cookie.as_deref(), Some("smuggled=1"));
}

#[test]
fn a_request_with_no_known_top_level_site_gets_no_cookies_and_sets_none() {
    let probe = Probe::new();
    let mut context = blocking();
    seed(&context, &probe.localhost("/"), &["session=1"]);

    // A client with no partition key and a web view no main frame has loaded in: what a worker started by a
    // page the policy has not seen would look like. Unknown is third party.
    let origin = origin_of(PAGE_LOCALHOST);
    let unknown = with_credentials(
        RequestBuilder::new(
            Some(web_view(77)),
            claim(&probe.localhost(&sets("/unknown", &["set-by-unknown=1"]))),
            Referrer::NoReferrer,
        )
        .origin(origin.clone())
        .client(client(origin, false, None)),
    );
    assert_eq!(
        context
            .state
            .net_policy
            .sites_of_request(&unknown.clone().build())
            .party,
        Party::Unknown
    );
    run(&mut context, unknown);
    assert_eq!(probe.only("/unknown").cookie, None);
    assert_eq!(jar(&context, &probe.localhost("/")), names(&["session"]));
}

#[test]
fn a_site_on_the_allow_list_gets_its_third_party_cookies_and_others_do_not() {
    let probe = Probe::new();
    let mut context = blocking();
    seed(&context, &probe.loopback("/"), &["tracker=1"]);
    context
        .state
        .net_policy
        .allow_third_party_cookies_on("localhost".to_owned(), true);

    // On localhost, 127.0.0.1's cookies flow in both directions.
    let third = probe.loopback(&sets("/allowed", &["widget=1"]));
    run(&mut context, in_page(&third, PAGE_LOCALHOST));
    assert_eq!(probe.only("/allowed").cookie.as_deref(), Some("tracker=1"));
    assert_eq!(
        jar(&context, &probe.loopback("/")),
        names(&["tracker", "widget"])
    );

    // On another site (here 127.0.0.1, with a request to localhost), still blocked.
    seed(&context, &probe.localhost("/"), &["session=1"]);
    run(
        &mut context,
        in_page(&probe.localhost("/refused"), PAGE_LOOPBACK),
    );
    assert_eq!(probe.only("/refused").cookie, None);

    // Taken off the list, blocked again.
    context
        .state
        .net_policy
        .allow_third_party_cookies_on("localhost".to_owned(), false);
    run(
        &mut context,
        in_page(&probe.loopback("/again"), PAGE_LOCALHOST),
    );
    assert_eq!(probe.only("/again").cookie, None);
}

#[test]
fn a_redirect_is_judged_at_every_hop() {
    let probe = Probe::new();
    let mut context = blocking();
    seed(&context, &probe.localhost("/"), &["session=1"]);
    seed(&context, &probe.loopback("/"), &["tracker=1"]);

    // A first-party request is redirected to another site. The redirect response sets a cookie of the first
    // party (stored), the second hop is third party (nothing sent, and what it sets is ignored).
    let second = probe.loopback(&sets("/second", &["from-the-third-party=1"]));
    let first = probe.localhost(&redirect("/first", &second, &["on-the-redirect=1"]));
    run(&mut context, in_page(&first, PAGE_LOCALHOST));

    assert_eq!(probe.only("/first").cookie.as_deref(), Some("session=1"));
    assert_eq!(probe.only("/second").cookie, None);
    assert_eq!(
        jar(&context, &probe.localhost("/")),
        names(&["on-the-redirect", "session"])
    );
    assert_eq!(jar(&context, &probe.loopback("/")), names(&["tracker"]));

    // The other way round: a third-party request redirected to the page's own site gets its cookie back for
    // the second hop.
    let back = probe.localhost("/back");
    let start = probe.loopback(&redirect("/start", &back, &[]));
    run(&mut context, in_page(&start, PAGE_LOCALHOST));
    assert_eq!(probe.only("/start").cookie, None);
    assert_eq!(
        probe.only("/back").cookie_names(),
        names(&["on-the-redirect", "session"])
    );
}

#[test]
fn a_top_level_navigation_is_first_party_whatever_site_it_goes_to() {
    let probe = Probe::new();
    let mut context = blocking();
    seed(&context, &probe.loopback("/"), &["tracker=1"]);

    // Typing 127.0.0.1 in a web view that was on localhost: the navigation is first party by construction.
    run(&mut context, typed(&probe.loopback("/page")));
    assert_eq!(probe.only("/page").cookie.as_deref(), Some("tracker=1"));

    // A link from a page on localhost to 127.0.0.1 is a top-level navigation too: its cookies flow.
    run(
        &mut context,
        from_page(&probe.loopback("/link"), PAGE_LOCALHOST),
    );
    assert_eq!(probe.only("/link").cookie.as_deref(), Some("tracker=1"));
}

// ---------------------------------------------------------------------------------------------------
// The record of the web view's main frame: frames navigated by script or by a link have the destination
// `Document` and are not top level (fact 8a of the lane brief).
// ---------------------------------------------------------------------------------------------------

#[test]
fn a_top_level_navigation_teaches_the_policy_the_page_of_its_web_view_at_every_hop() {
    let probe = Probe::new();
    let mut context = blocking();

    // localhost redirects to 127.0.0.1: the record follows the hops, and ends on the last one.
    let landing = probe.loopback("/landing");
    run(
        &mut context,
        typed(&probe.localhost(&redirect("/hop", &landing, &[]))),
    );
    assert_eq!(
        context
            .state
            .net_policy
            .recorded_top_level_site(TEST_WEBVIEW_ID)
            .as_deref(),
        Some("127.0.0.1")
    );
}

#[test]
fn a_frame_navigated_by_script_is_judged_by_the_record_and_not_as_top_level() {
    let probe = Probe::new();
    let mut context = blocking();
    seed(&context, &probe.localhost("/"), &["session=1"]);
    seed(&context, &probe.loopback("/"), &["tracker=1"]);

    // The web view is on localhost, loaded through the fetch path so that the hook that writes the record runs.
    run(&mut context, typed(&probe.localhost("/page")));
    assert_eq!(probe.only("/page").cookie.as_deref(), Some("session=1"));

    // An iframe of that page navigates to 127.0.0.1. It has the destination `Document` like the page, but its
    // client is nested: third party. No cookie goes, and the one it sets is ignored.
    let tracker = probe.loopback(&sets("/frame-third", &["frame-cookie=1"]));
    run(&mut context, in_frame(&tracker, PAGE_LOCALHOST));
    assert_eq!(probe.only("/frame-third").cookie, None);
    assert_eq!(jar(&context, &probe.loopback("/")), names(&["tracker"]));

    // An iframe that navigates to the page's own site is first party.
    run(
        &mut context,
        in_frame(&probe.localhost("/frame-first"), PAGE_LOCALHOST),
    );
    assert_eq!(
        probe.only("/frame-first").cookie.as_deref(),
        Some("session=1")
    );

    // The same navigation by the web view itself (a link in the top-level page) goes to 127.0.0.1 with its
    // cookies, because it is top level.
    run(
        &mut context,
        from_page(&probe.loopback("/top-level"), PAGE_LOCALHOST),
    );
    assert_eq!(
        probe.only("/top-level").cookie.as_deref(),
        Some("tracker=1")
    );
}

// ---------------------------------------------------------------------------------------------------
// SameSite (EC6): sending.
// ---------------------------------------------------------------------------------------------------

/// One cookie of each kind: Strict, Lax, None and no attribute.
const KINDS: [&str; 4] = [
    "s=1; SameSite=Strict",
    "l=1; SameSite=Lax",
    "n=1; SameSite=None",
    "u=1",
];

#[test]
fn every_cookie_goes_with_a_same_site_request() {
    let probe = Probe::new();
    let mut context = same_site();
    seed(&context, &probe.localhost("/"), &KINDS);
    run(
        &mut context,
        in_page(&probe.localhost("/x"), PAGE_LOCALHOST),
    );
    assert_eq!(
        probe.only("/x").cookie_names(),
        names(&["s", "l", "n", "u"])
    );
}

#[test]
fn only_none_and_unattributed_cookies_go_with_a_cross_site_subresource() {
    let probe = Probe::new();
    let mut context = same_site();
    seed(&context, &probe.localhost("/"), &KINDS);
    // A page on 127.0.0.1 loads something from localhost.
    run(&mut context, in_page(&probe.localhost("/x"), PAGE_LOOPBACK));
    assert_eq!(probe.only("/x").cookie_names(), names(&["n", "u"]));
}

#[test]
fn a_typed_address_sends_its_strict_cookies() {
    let probe = Probe::new();
    let mut context = same_site();
    seed(&context, &probe.localhost("/"), &KINDS);
    // A top-level navigation with no initiator: same-site by D8.
    run(&mut context, typed(&probe.localhost("/x")));
    assert_eq!(
        probe.only("/x").cookie_names(),
        names(&["s", "l", "n", "u"])
    );
}

#[test]
fn a_cross_site_top_level_get_sends_lax_cookies_and_not_strict_ones() {
    let probe = Probe::new();
    let mut context = same_site();
    seed(&context, &probe.localhost("/"), &KINDS);
    // A link on a page of 127.0.0.1 to localhost.
    run(
        &mut context,
        from_page(&probe.localhost("/x"), PAGE_LOOPBACK),
    );
    assert_eq!(probe.only("/x").cookie_names(), names(&["l", "n", "u"]));
}

#[test]
fn a_cross_site_top_level_post_sends_neither_lax_nor_strict_cookies() {
    let probe = Probe::new();
    let mut context = same_site();
    seed(&context, &probe.localhost("/"), &KINDS);
    // A form on a page of 127.0.0.1 posts to localhost.
    let form = from_page(&probe.localhost("/x"), PAGE_LOOPBACK).method(Method::POST);
    run(&mut context, form);
    let seen = probe.only("/x");
    assert_eq!(seen.method, Method::POST);
    assert_eq!(seen.cookie_names(), names(&["n", "u"]));
}

#[test]
fn a_frame_navigated_by_a_cross_site_page_does_not_get_lax_cookies_although_it_is_a_get_document() {
    let probe = Probe::new();
    let mut context = same_site();
    seed(&context, &probe.localhost("/"), &KINDS);
    // The web view is on 127.0.0.1 (as a main-frame load would have recorded).
    the_web_view_is_on(&context, &probe.loopback("/page"));
    // A frame of that page navigates to localhost: destination `Document`, method GET, but not top level.
    run(
        &mut context,
        in_frame(&probe.localhost("/x"), PAGE_LOOPBACK),
    );
    assert_eq!(probe.only("/x").cookie_names(), names(&["n", "u"]));
}

#[test]
fn a_redirect_across_sites_withholds_strict_cookies_from_the_hop_that_crossed() {
    let probe = Probe::new();
    let mut context = same_site();
    seed(&context, &probe.localhost("/"), &KINDS);
    seed(
        &context,
        &probe.loopback("/"),
        &["s2=1; SameSite=Strict", "l2=1; SameSite=Lax", "u2=1"],
    );

    // Typed: localhost, which redirects to 127.0.0.1.
    let second = probe.loopback("/second");
    run(
        &mut context,
        typed(&probe.localhost(&redirect("/first", &second, &[]))),
    );
    assert_eq!(
        probe.only("/first").cookie_names(),
        names(&["s", "l", "n", "u"])
    );
    // The chain crossed sites, so the second hop is cross-site: Lax goes (top-level GET), Strict does not.
    assert_eq!(probe.only("/second").cookie_names(), names(&["l2", "u2"]));
}

#[test]
fn a_redirect_inside_one_site_keeps_strict_cookies() {
    let probe = Probe::new();
    let mut context = same_site();
    seed(&context, &probe.localhost("/"), &KINDS);
    let second = probe.localhost("/second");
    run(
        &mut context,
        in_page(
            &probe.localhost(&redirect("/first", &second, &[])),
            PAGE_LOCALHOST,
        ),
    );
    assert_eq!(
        probe.only("/second").cookie_names(),
        names(&["s", "l", "n", "u"])
    );
}

#[test]
fn lax_by_default_is_applied_only_when_it_is_switched_on() {
    let probe = Probe::new();
    // Off (Mercury's setting, D8): a cookie without the attribute goes cross-site.
    let mut context = same_site();
    seed(&context, &probe.localhost("/"), &["u=1"]);
    run(
        &mut context,
        in_page(&probe.localhost("/off"), PAGE_LOOPBACK),
    );
    assert_eq!(probe.only("/off").cookie_names(), names(&["u"]));

    // On: it is Lax, so it stays at home on a subresource and goes on a top-level GET.
    let mut context = context_with(|policy| {
        policy.set_same_site_enforcement(true);
        policy.set_same_site_lax_default(true);
    });
    seed(&context, &probe.localhost("/"), &["u=1"]);
    run(
        &mut context,
        in_page(&probe.localhost("/on-sub"), PAGE_LOOPBACK),
    );
    assert_eq!(probe.only("/on-sub").cookie, None);
    run(
        &mut context,
        from_page(&probe.localhost("/on-nav"), PAGE_LOOPBACK),
    );
    assert_eq!(probe.only("/on-nav").cookie_names(), names(&["u"]));
    run(
        &mut context,
        in_page(&probe.localhost("/on-same"), PAGE_LOCALHOST),
    );
    assert_eq!(probe.only("/on-same").cookie_names(), names(&["u"]));
}

#[test]
fn nothing_is_withheld_for_samesite_when_it_is_not_enforced() {
    let probe = Probe::new();
    let mut context = new_fetch_context(None, None);
    seed(&context, &probe.localhost("/"), &KINDS);
    run(&mut context, in_page(&probe.localhost("/x"), PAGE_LOOPBACK));
    assert_eq!(
        probe.only("/x").cookie_names(),
        names(&["s", "l", "n", "u"])
    );
}

// ---------------------------------------------------------------------------------------------------
// SameSite (EC6): storing.
// ---------------------------------------------------------------------------------------------------

#[test]
fn a_same_site_response_stores_every_cookie_but_a_none_cookie_without_secure() {
    let probe = Probe::new();
    let mut context = same_site();
    let url = probe.localhost(&sets("/x", &KINDS));
    run(&mut context, in_page(&url, PAGE_LOCALHOST));
    // `n` is `SameSite=None` without `Secure`: refused (D6).
    assert_eq!(
        jar(&context, &probe.localhost("/")),
        names(&["s", "l", "u"])
    );
}

#[test]
fn the_same_response_stores_every_cookie_when_samesite_is_not_enforced() {
    let probe = Probe::new();
    let mut context = new_fetch_context(None, None);
    let url = probe.localhost(&sets("/x", &KINDS));
    run(&mut context, in_page(&url, PAGE_LOCALHOST));
    assert_eq!(
        jar(&context, &probe.localhost("/")),
        names(&["s", "l", "n", "u"])
    );
}

#[test]
fn a_cross_site_subresource_cannot_set_a_strict_or_lax_cookie() {
    let probe = Probe::new();
    let mut context = same_site();
    let url = probe.localhost(&sets("/x", &KINDS));
    // A page on 127.0.0.1 loads something from localhost, which answers with the four.
    run(&mut context, in_page(&url, PAGE_LOOPBACK));
    assert_eq!(jar(&context, &probe.localhost("/")), names(&["u"]));
}

#[test]
fn a_cross_site_top_level_navigation_may_set_a_strict_or_lax_cookie() {
    let probe = Probe::new();
    let mut context = same_site();
    let url = probe.localhost(&sets("/x", &KINDS));
    // A link on a page of 127.0.0.1 to localhost.
    run(&mut context, from_page(&url, PAGE_LOOPBACK));
    assert_eq!(
        jar(&context, &probe.localhost("/")),
        names(&["s", "l", "u"])
    );
}

#[test]
fn a_frame_navigated_by_script_cannot_set_a_strict_or_lax_cookie_across_sites() {
    let probe = Probe::new();
    let mut context = same_site();
    the_web_view_is_on(&context, &probe.loopback("/page"));
    let url = probe.localhost(&sets("/x", &KINDS));
    run(&mut context, in_frame(&url, PAGE_LOOPBACK));
    assert_eq!(jar(&context, &probe.localhost("/")), names(&["u"]));
}

#[test]
fn lax_by_default_refuses_an_unattributed_cookie_from_a_cross_site_subresource() {
    let probe = Probe::new();
    let mut context = context_with(|policy| {
        policy.set_same_site_enforcement(true);
        policy.set_same_site_lax_default(true);
    });
    let url = probe.localhost(&sets("/x", &["u=1"]));
    run(&mut context, in_page(&url, PAGE_LOOPBACK));
    assert_eq!(jar(&context, &probe.localhost("/")), names(&[]));
    // From its own site it is stored.
    let url = probe.localhost(&sets("/y", &["u=1"]));
    run(&mut context, in_page(&url, PAGE_LOCALHOST));
    assert_eq!(jar(&context, &probe.localhost("/")), names(&["u"]));
}

#[test]
fn a_none_cookie_needs_secure_and_gets_it_over_https() {
    let probe = Probe::tls();
    let mut context = new_fetch_context(None, None);
    context.state =
        Arc::new(crate::certificate_exceptions::state_that_ignores_certificate_errors());
    context.state.net_policy.set_same_site_enforcement(true);

    let page = "https://localhost:1/";
    let url = probe.localhost(&sets(
        "/x",
        &[
            "secure-none=1; SameSite=None; Secure",
            "plain-none=1; SameSite=None",
        ],
    ));
    run(&mut context, in_page(&url, page));
    assert_eq!(
        jar(&context, &probe.localhost("/")),
        names(&["secure-none"])
    );

    // And the Secure cookie is sent back over https.
    run(&mut context, in_page(&probe.localhost("/y"), page));
    assert_eq!(probe.only("/y").cookie.as_deref(), Some("secure-none=1"));
}

// ---------------------------------------------------------------------------------------------------
// Global Privacy Control (EC25).
// ---------------------------------------------------------------------------------------------------

#[test]
fn a_request_carries_sec_gpc_when_the_policy_says_so_and_not_otherwise() {
    let probe = Probe::new();
    let mut on = context_with(|policy| policy.set_global_privacy_control(true));
    run(&mut on, in_page(&probe.localhost("/on"), PAGE_LOCALHOST));
    assert_eq!(probe.only("/on").sec_gpc.as_deref(), Some("1"));

    let mut off = new_fetch_context(None, None);
    run(&mut off, in_page(&probe.localhost("/off"), PAGE_LOCALHOST));
    assert_eq!(probe.only("/off").sec_gpc, None);

    // Switched off after it was on: the same context stops sending it.
    on.state.net_policy.set_global_privacy_control(false);
    run(
        &mut on,
        in_page(&probe.localhost("/stopped"), PAGE_LOCALHOST),
    );
    assert_eq!(probe.only("/stopped").sec_gpc, None);
}

#[test]
fn every_hop_of_a_redirect_carries_sec_gpc() {
    let probe = Probe::new();
    let mut context = context_with(|policy| policy.set_global_privacy_control(true));
    let second = probe.loopback("/second");
    run(
        &mut context,
        in_page(
            &probe.localhost(&redirect("/first", &second, &[])),
            PAGE_LOCALHOST,
        ),
    );
    assert_eq!(probe.only("/first").sec_gpc.as_deref(), Some("1"));
    assert_eq!(probe.only("/second").sec_gpc.as_deref(), Some("1"));
}

#[test]
fn a_cors_preflight_and_the_request_it_precedes_carry_sec_gpc() {
    let probe = Probe::new();
    let mut context = context_with(|policy| policy.set_global_privacy_control(true));

    // A page on localhost writes to 127.0.0.1 with PUT: a cross-origin request that needs a preflight.
    let origin = origin_of(PAGE_LOCALHOST);
    let put = RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        claim(&probe.loopback("/put")),
        Referrer::NoReferrer,
    )
    .method(Method::PUT)
    .mode(RequestMode::CorsMode)
    .use_cors_preflight(true)
    .unsafe_request(true)
    .origin(origin.clone())
    .client(client(origin, false, Some(site_of(PAGE_LOCALHOST))))
    .pipeline_id(Some(TEST_PIPELINE_ID))
    .policy_container(Default::default());
    run(&mut context, put);

    let seen = probe.requests("/put");
    let preflight = seen
        .iter()
        .find(|seen| seen.method == Method::OPTIONS)
        .unwrap_or_else(|| panic!("a preflight was sent, saw {seen:?}"));
    assert_eq!(preflight.sec_gpc.as_deref(), Some("1"));
    let put = seen
        .iter()
        .find(|seen| seen.method == Method::PUT)
        .unwrap_or_else(|| panic!("the PUT was sent, saw {seen:?}"));
    assert_eq!(put.sec_gpc.as_deref(), Some("1"));
}

// ---------------------------------------------------------------------------------------------------
// The messages of the resource thread: `document.cookie` and `cookieStore`.
//
// These go through a real resource thread, which takes the process-wide policy (`NetPolicy::process()`), so
// each test holds the lock of the process policy, configures it, and puts it back when it ends.
// ---------------------------------------------------------------------------------------------------

/// The process-wide policy, configured for one test and put back to Servo's behaviour afterwards.
struct ProcessPolicy {
    _only_test_on_the_process_policy: parking_lot::MutexGuard<'static, ()>,
    allowed: Vec<&'static str>,
}

impl ProcessPolicy {
    fn configure(setup: impl FnOnce(&NetPolicy)) -> ProcessPolicy {
        let lock = PROCESS_POLICY_LOCK.lock();
        let policy = NetPolicy::process();
        put_back(&policy, &[]);
        setup(&policy);
        ProcessPolicy {
            _only_test_on_the_process_policy: lock,
            allowed: Vec::new(),
        }
    }

    fn allow_third_party_cookies_on(&mut self, site: &'static str) {
        NetPolicy::process().allow_third_party_cookies_on(site.to_owned(), true);
        self.allowed.push(site);
    }
}

impl Drop for ProcessPolicy {
    fn drop(&mut self) {
        put_back(&NetPolicy::process(), &self.allowed);
    }
}

fn put_back(policy: &NetPolicy, allowed: &[&str]) {
    policy.set_block_third_party_cookies(false);
    policy.set_same_site_enforcement(false);
    policy.set_same_site_lax_default(false);
    policy.set_global_privacy_control(false);
    policy.set_referrer_cap(false);
    for site in allowed {
        policy.allow_third_party_cookies_on((*site).to_owned(), false);
    }
}

/// A resource thread, with the channels it reports to kept alive.
struct Harness {
    thread: CoreResourceThread,
    _channels: Box<dyn std::any::Any>,
    next_store: u32,
}

impl Harness {
    fn new() -> Harness {
        let (tx, rx) = generic_channel::channel().unwrap();
        let (mtx, mrx) = generic_channel::channel().unwrap();
        let (thread, private_thread) = new_core_resource_thread(
            None,
            ProfilerChan(Some(tx)),
            MemProfilerChan(mtx),
            create_generic_embedder_proxy(),
            None,
            CACertificates::Default,
            false, /* ignore_certificate_errors */
            Arc::new(ProtocolRegistry::default()),
        );
        Harness {
            thread,
            _channels: Box::new((rx, mrx, private_thread)),
            next_store: 1,
        }
    }

    fn send(&self, message: CoreResourceMsg) {
        self.thread.send(message).unwrap();
    }

    /// Puts a cookie in the jar with the message that the policy does not filter (the one WebDriver uses).
    fn seed(&self, url: &str, cookie: &str) {
        let (sender, receiver) = generic_channel::channel().unwrap();
        let cookie = cookie::Cookie::parse(cookie.to_owned()).unwrap();
        self.send(CoreResourceMsg::SetCookieForUrl(
            parse(url),
            Serde(cookie),
            CookieSource::HTTP,
            Some(sender),
        ));
        receiver.recv().unwrap();
    }

    /// The names in the jar for a URL, read with the message that the policy does not filter.
    fn jar(&self, url: &str) -> Vec<String> {
        let (sender, receiver) = generic_channel::channel().unwrap();
        self.send(CoreResourceMsg::GetCookiesForUrl(
            parse(url),
            sender,
            CookieSource::NonHTTP,
        ));
        let mut names: Vec<String> = receiver
            .recv()
            .unwrap()
            .into_iter()
            .map(|cookie| cookie.into_inner().name().to_owned())
            .collect();
        names.sort();
        names
    }

    /// `document.cookie` of a document at `url`, in a page whose top level is `key`.
    fn document_cookie(
        &self,
        url: &str,
        key: Option<&str>,
        web_view: Option<WebViewId>,
    ) -> Vec<String> {
        let (sender, receiver) = generic_channel::channel().unwrap();
        self.send(CoreResourceMsg::GetCookieStringForDocument(
            parse(url),
            key.map(site_of),
            web_view,
            sender,
            CookieSource::NonHTTP,
        ));
        cookie_names(receiver.recv().unwrap().as_deref())
    }

    /// Writes `document.cookie = ...` for each cookie.
    fn set_document_cookies(
        &self,
        url: &str,
        key: Option<&str>,
        web_view: Option<WebViewId>,
        cookies: &[&str],
    ) {
        let cookies = cookies
            .iter()
            .map(|cookie| Serde(cookie::Cookie::parse((*cookie).to_owned()).unwrap()))
            .collect();
        self.send(CoreResourceMsg::SetCookiesForDocument(
            parse(url),
            key.map(site_of),
            web_view,
            cookies,
            CookieSource::NonHTTP,
        ));
    }

    /// A `cookieStore` of this thread: the id and the end that the answers arrive on.
    fn cookie_store(&mut self) -> (CookieStoreId, GenericReceiver<CookieAsyncResponse>) {
        let (callback, receiver) = GenericCallback::new_blocking().unwrap();
        let id = CookieStoreId {
            namespace_id: PipelineNamespaceId(4242),
            index: Index::new(self.next_store).unwrap(),
        };
        self.next_store += 1;
        self.send(CoreResourceMsg::NewCookieListener(
            id,
            callback,
            parse("http://localhost/"),
        ));
        (id, receiver)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let (sender, receiver) = generic_channel::oneshot().unwrap();
        if self.thread.send(CoreResourceMsg::Exit(sender)).is_ok() {
            let _ = receiver.recv();
        }
    }
}

const LOOPBACK_DOCUMENT: &str = "http://127.0.0.1:9000/frame";

#[test]
fn document_cookie_of_a_third_party_frame_reads_nothing_and_writes_nothing() {
    let mut policy = ProcessPolicy::configure(|policy| policy.set_block_third_party_cookies(true));
    let harness = Harness::new();
    harness.seed(LOOPBACK_DOCUMENT, "tracker=1");

    // A document on 127.0.0.1 in a page whose top level is 127.0.0.1: first party.
    assert_eq!(
        harness.document_cookie(
            LOOPBACK_DOCUMENT,
            Some(PAGE_LOOPBACK),
            Some(TEST_WEBVIEW_ID)
        ),
        names(&["tracker"])
    );
    harness.set_document_cookies(
        LOOPBACK_DOCUMENT,
        Some(PAGE_LOOPBACK),
        Some(TEST_WEBVIEW_ID),
        &["own=1"],
    );
    assert_eq!(harness.jar(LOOPBACK_DOCUMENT), names(&["own", "tracker"]));

    // The same document in a page on localhost: third party. It sees nothing, and what it writes is dropped.
    assert_eq!(
        harness.document_cookie(
            LOOPBACK_DOCUMENT,
            Some(PAGE_LOCALHOST),
            Some(TEST_WEBVIEW_ID)
        ),
        names(&[])
    );
    harness.set_document_cookies(
        LOOPBACK_DOCUMENT,
        Some(PAGE_LOCALHOST),
        Some(TEST_WEBVIEW_ID),
        &["dropped=1"],
    );
    assert_eq!(harness.jar(LOOPBACK_DOCUMENT), names(&["own", "tracker"]));

    // A document with no key and a web view the policy has not seen: unknown, which is third party.
    assert_eq!(
        harness.document_cookie(LOOPBACK_DOCUMENT, None, Some(web_view(5))),
        names(&[])
    );
    assert_eq!(
        harness.document_cookie(LOOPBACK_DOCUMENT, None, None),
        names(&[])
    );

    // On the allow list the third-party frame reads and writes again.
    policy.allow_third_party_cookies_on("localhost");
    assert_eq!(
        harness.document_cookie(
            LOOPBACK_DOCUMENT,
            Some(PAGE_LOCALHOST),
            Some(TEST_WEBVIEW_ID)
        ),
        names(&["own", "tracker"])
    );
    harness.set_document_cookies(
        LOOPBACK_DOCUMENT,
        Some(PAGE_LOCALHOST),
        Some(TEST_WEBVIEW_ID),
        &["allowed=1"],
    );
    assert_eq!(
        harness.jar(LOOPBACK_DOCUMENT),
        names(&["allowed", "own", "tracker"])
    );
}

#[test]
fn document_cookie_follows_samesite_for_a_cross_site_frame() {
    let _policy = ProcessPolicy::configure(|policy| policy.set_same_site_enforcement(true));
    let harness = Harness::new();
    harness.seed(LOOPBACK_DOCUMENT, "s=1; SameSite=Strict");
    harness.seed(LOOPBACK_DOCUMENT, "l=1; SameSite=Lax");
    harness.seed(LOOPBACK_DOCUMENT, "u=1");

    // Same site as the top level: everything is visible.
    assert_eq!(
        harness.document_cookie(
            LOOPBACK_DOCUMENT,
            Some(PAGE_LOOPBACK),
            Some(TEST_WEBVIEW_ID)
        ),
        names(&["s", "l", "u"])
    );
    // A frame of another site's page: only the cookie with no restriction.
    assert_eq!(
        harness.document_cookie(
            LOOPBACK_DOCUMENT,
            Some(PAGE_LOCALHOST),
            Some(TEST_WEBVIEW_ID)
        ),
        names(&["u"])
    );

    // Writing: the cross-site frame cannot set a Lax or Strict cookie, and `SameSite=None` needs `Secure` for
    // everyone. The unrestricted one goes in.
    harness.set_document_cookies(
        LOOPBACK_DOCUMENT,
        Some(PAGE_LOCALHOST),
        Some(TEST_WEBVIEW_ID),
        &["cross-lax=1; SameSite=Lax", "cross-plain=1"],
    );
    harness.set_document_cookies(
        LOOPBACK_DOCUMENT,
        Some(PAGE_LOOPBACK),
        Some(TEST_WEBVIEW_ID),
        &["own-lax=1; SameSite=Lax", "own-none=1; SameSite=None"],
    );
    assert_eq!(
        harness.jar(LOOPBACK_DOCUMENT),
        names(&["s", "l", "u", "cross-plain", "own-lax"])
    );
}

#[test]
fn the_document_messages_change_nothing_when_the_policy_is_off() {
    let _policy = ProcessPolicy::configure(|_| {});
    let harness = Harness::new();
    harness.seed(LOOPBACK_DOCUMENT, "s=1; SameSite=Strict");

    // A third-party frame, a cross-site frame, an unknown frame: Servo's behaviour, everything is visible.
    for (key, web_view) in [
        (Some(PAGE_LOCALHOST), Some(TEST_WEBVIEW_ID)),
        (Some(PAGE_LOOPBACK), None),
        (None, None),
    ] {
        assert_eq!(
            harness.document_cookie(LOOPBACK_DOCUMENT, key, web_view),
            names(&["s"])
        );
    }
    harness.set_document_cookies(
        LOOPBACK_DOCUMENT,
        Some(PAGE_LOCALHOST),
        None,
        &["w=1; SameSite=Lax"],
    );
    assert_eq!(harness.jar(LOOPBACK_DOCUMENT), names(&["s", "w"]));
}

#[test]
fn the_cookie_store_messages_are_answered_as_if_no_cookie_existed_while_blocking() {
    let url = "http://127.0.0.1:9000/";
    let _policy = ProcessPolicy::configure(|policy| policy.set_block_third_party_cookies(true));
    let mut harness = Harness::new();
    harness.seed(url, "sid=1");
    let (id, answers) = harness.cookie_store();

    harness.send(CoreResourceMsg::GetCookieDataForUrlAsync(
        id,
        parse(url),
        Some("sid".to_owned()),
    ));
    assert!(matches!(
        answers.recv().unwrap().data,
        CookieData::Get(None)
    ));

    harness.send(CoreResourceMsg::GetAllCookieDataForUrlAsync(
        id,
        parse(url),
        None,
    ));
    match answers.recv().unwrap().data {
        CookieData::GetAll(cookies) => assert!(cookies.is_empty()),
        other => panic!("expected GetAll, got {other:?}"),
    }

    // A write is ignored (and reported as done, like a dropped `document.cookie`), a delete deletes nothing.
    harness.send(CoreResourceMsg::SetCookieForUrlAsync(
        id,
        parse(url),
        Serde(cookie::Cookie::parse("written=1".to_owned()).unwrap()),
        CookieSource::NonHTTP,
    ));
    assert!(matches!(
        answers.recv().unwrap().data,
        CookieData::Set(Ok(()))
    ));
    harness.send(CoreResourceMsg::DeleteCookieAsync(
        id,
        parse(url),
        "sid".to_owned(),
    ));
    assert!(matches!(
        answers.recv().unwrap().data,
        CookieData::Delete(Ok(()))
    ));
    assert_eq!(harness.jar(url), names(&["sid"]));
}

#[test]
fn the_cookie_store_messages_work_as_before_while_the_policy_is_off() {
    let url = "http://127.0.0.1:9000/";
    let _policy = ProcessPolicy::configure(|_| {});
    let mut harness = Harness::new();
    harness.seed(url, "sid=1");
    let (id, answers) = harness.cookie_store();

    harness.send(CoreResourceMsg::GetCookieDataForUrlAsync(
        id,
        parse(url),
        Some("sid".to_owned()),
    ));
    match answers.recv().unwrap().data {
        CookieData::Get(Some(cookie)) => assert_eq!(cookie.into_inner().value(), "1"),
        other => panic!("expected the cookie, got {other:?}"),
    }
    harness.send(CoreResourceMsg::SetCookieForUrlAsync(
        id,
        parse(url),
        Serde(cookie::Cookie::parse("written=1".to_owned()).unwrap()),
        CookieSource::NonHTTP,
    ));
    assert!(matches!(
        answers.recv().unwrap().data,
        CookieData::Set(Ok(()))
    ));
    harness.send(CoreResourceMsg::DeleteCookieAsync(
        id,
        parse(url),
        "sid".to_owned(),
    ));
    assert!(matches!(
        answers.recv().unwrap().data,
        CookieData::Delete(Ok(()))
    ));
    assert_eq!(harness.jar(url), names(&["written"]));
}

// ---------------------------------------------------------------------------------------------------
// A policy that is not configured changes nothing.
// ---------------------------------------------------------------------------------------------------

#[test]
fn a_policy_that_is_not_configured_changes_nothing() {
    let probe = Probe::new();
    let mut context = new_fetch_context(None, None);
    seed(&context, &probe.loopback("/"), &["tracker=1"]);

    // A third-party request in a page of another site: the cookie goes, the answer's cookie is stored, and no
    // `Sec-GPC` is added.
    let url = probe.loopback(&sets("/third", &["widget=1"]));
    run(&mut context, in_page(&url, PAGE_LOCALHOST));
    let seen = probe.only("/third");
    assert_eq!(seen.cookie.as_deref(), Some("tracker=1"));
    assert_eq!(seen.sec_gpc, None);
    assert_eq!(
        jar(&context, &probe.loopback("/")),
        names(&["tracker", "widget"])
    );

    // A request with no client, no web view and no key (what the existing tests build) is the same.
    let bare = RequestBuilder::new(None, claim(&probe.loopback("/bare")), Referrer::NoReferrer)
        .origin(origin_of(PAGE_LOCALHOST));
    run(&mut context, with_credentials(bare));
    assert_eq!(
        probe.only("/bare").cookie_names(),
        names(&["tracker", "widget"])
    );
    assert_eq!(
        context.state.net_policy.stats().third_party_cookies_blocked,
        0
    );
}

// ---------------------------------------------------------------------------------------------------
// The guard: a cookie is attached and stored only behind the policy check.
// ---------------------------------------------------------------------------------------------------

#[test]
fn http_loader_attaches_and_stores_cookies_only_behind_the_policy_check() {
    let source = include_str!("../http_loader.rs");
    let lines: Vec<&str> = source.lines().collect();
    for function in ["set_request_cookies", "set_cookies_from_headers"] {
        let call = format!("{function}(");
        let mut calls = 0;
        for (index, line) in lines.iter().enumerate() {
            let line = line.trim_start();
            if line.starts_with("//") || !line.contains(&call) {
                continue;
            }
            // The definition is not a call.
            if line.starts_with("fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("pub(crate) fn ")
            {
                continue;
            }
            calls += 1;
            let before = &lines[index.saturating_sub(6)..index];
            assert!(
                before.iter().any(|earlier| {
                    let earlier = earlier.trim_start();
                    !earlier.starts_with("//") && earlier.contains("cookies_allowed(")
                }),
                "{function} is called at http_loader.rs:{} outside the network policy's `cookies_allowed` check",
                index + 1
            );
        }
        assert_eq!(
            calls, 1,
            "{function} has exactly one call in http_loader.rs (the guarded one); a second call needs its own review"
        );
    }
}
