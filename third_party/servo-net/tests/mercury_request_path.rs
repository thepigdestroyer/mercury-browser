// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: tests for the request path of the net-1 lane: frame identity on `WebResourceRequest` (EC38,
// D17), the rewrite message (EC37), the referrer cap (EC14) and mixed content (EC17). New file.

//! The request path, through the interceptor round trip, as the embedder sees it.
//!
//! Most tests need no server: the embedder end of the channel (a thread, as in `fetch.rs`'s
//! `test_fetch_request_intercepted`) answers every request with a canned response, and the test reads what
//! the embedder was told. The rewrite tests use local hyper servers because their point is what reaches
//! the server. Nothing here opens a window, and the one TLS test uses the loopback certificate of
//! `test_util` as `test_fetch_with_hsts` does.
//!
//! Host names (`top.example`, `cdn.example`, `secure.example`, `insecure.example`) are never connected to:
//! a canned answer or a refusal ends the fetch first, so the tests need no `HOST_FILE`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::RecvTimeoutError;
use embedder_traits::{
    MercuryEmbedderMsg, NetMsg, WebResourceRequest, WebResourceResponse, WebResourceResponseMsg,
    WebResourceRewrite,
};
use http::StatusCode;
use http::header::{self, HeaderMap, HeaderValue};
use http_body_util::combinators::BoxBody;
use hyper::body::{Bytes, Incoming};
use hyper::{Request as HyperRequest, Response as HyperResponse};
use net::embedder::NetToEmbedderMsg;
use net::fetch::methods::FetchContext;
use net::hsts::HstsEntry;
use net::net_policy::site_key_of_url;
use net::test_util::Server;
use net_traits::blob_url_store::UrlWithBlobClaim;
use net_traits::pub_domains::Site;
use net_traits::request::{
    Destination, InsecureRequestsPolicy, Origin, Referrer, Request, RequestBuilder, RequestClient,
    RequestMode,
};
use net_traits::response::{Response, ResponseBody};
use net_traits::{IncludeSubdomains, NetworkError, ReferrerPolicy};
use servo_base::id::{TEST_WEBVIEW_ID, WebViewId};
use servo_url::{ImmutableOrigin, ServoUrl};

use crate::{
    create_generic_embedder_proxy_and_receiver, fetch_with_context, make_body, make_server,
    make_ssl_server, new_fetch_context,
};

// ---------------------------------------------------------------------------------------------
// The embedder end
// ---------------------------------------------------------------------------------------------

/// What the embedder end of the network thread's channel received.
#[derive(Debug)]
enum Seen {
    Requested(Option<WebViewId>, WebResourceRequest),
    MixedContentBlocked(Option<WebViewId>, ServoUrl, Destination),
}

/// The embedder's thread: it answers each `WebResourceRequested` with the messages `answer` returns and
/// records everything it receives, in order.
struct EmbedderEnd {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Vec<Seen>>,
}

impl EmbedderEnd {
    /// Everything the embedder received. Call it after the fetches are done: the network thread sends its
    /// messages before a fetch returns, so they are all queued, and the thread stops after a quiet moment.
    fn finish(self) -> Vec<Seen> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().expect("the embedder end panicked")
    }
}

fn embedder_end<F>(
    answer: F,
) -> (
    embedder_traits::GenericEmbedderProxy<NetToEmbedderMsg>,
    EmbedderEnd,
)
where
    F: Fn(&WebResourceRequest) -> Vec<WebResourceResponseMsg> + Send + 'static,
{
    let (proxy, receiver) = create_generic_embedder_proxy_and_receiver::<NetToEmbedderMsg>();
    // `test_util` starts the async runtime that `fetch_with_context` needs; a test without a server
    // would otherwise be the first to ask for it.
    let _ = net::test_util::create_generic_embedder_proxy::<NetToEmbedderMsg>();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let thread = std::thread::spawn(move || {
        let mut seen = Vec::new();
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(NetToEmbedderMsg::WebResourceRequested(webview_id, request, sender)) => {
                    let answers = answer(&request);
                    seen.push(Seen::Requested(webview_id, request));
                    for message in answers {
                        let _ = sender.send(message);
                    }
                },
                Ok(NetToEmbedderMsg::Mercury(
                    webview_id,
                    MercuryEmbedderMsg::Net(NetMsg::MixedContentBlocked { url, destination }),
                )) => seen.push(Seen::MixedContentBlocked(webview_id, url, destination)),
                Ok(_) => {},
                Err(RecvTimeoutError::Timeout) => {
                    if stopped.load(Ordering::SeqCst) {
                        break;
                    }
                },
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        seen
    });
    (proxy, EmbedderEnd { stop, thread })
}

fn requested(seen: &[Seen]) -> Vec<&WebResourceRequest> {
    seen.iter()
        .filter_map(|seen| match seen {
            Seen::Requested(_, request) => Some(request),
            _ => None,
        })
        .collect()
}

/// The mixed-content reports the embedder received: the web view, the URL and the destination.
fn reports(seen: &[Seen]) -> Vec<(Option<WebViewId>, String, Destination)> {
    seen.iter()
        .filter_map(|seen| match seen {
            Seen::MixedContentBlocked(webview_id, url, destination) => {
                Some((*webview_id, url.as_str().to_owned(), *destination))
            },
            _ => None,
        })
        .collect()
}

fn do_not_intercept() -> Vec<WebResourceResponseMsg> {
    vec![WebResourceResponseMsg::DoNotIntercept]
}

/// A complete answer whose response says it comes from `response_url`: status 200 and a short body.
fn canned(response_url: &ServoUrl) -> Vec<WebResourceResponseMsg> {
    vec![
        WebResourceResponseMsg::Start(WebResourceResponse::new(response_url.as_url().clone())),
        WebResourceResponseMsg::SendBodyData(b"canned".to_vec()),
        WebResourceResponseMsg::FinishLoad,
    ]
}

/// Fetch each request against an embedder that answers with `answer`; returns the responses and what the
/// embedder received. `prepare` runs on the context first (the policy, the HSTS list).
fn run(
    answer: impl Fn(&WebResourceRequest) -> Vec<WebResourceResponseMsg> + Send + 'static,
    prepare: impl FnOnce(&mut FetchContext),
    requests: Vec<Request>,
) -> (Vec<Response>, Vec<Seen>) {
    let (proxy, embedder) = embedder_end(answer);
    let mut context = new_fetch_context(None, Some(proxy));
    prepare(&mut context);
    let responses = requests
        .into_iter()
        .map(|request| fetch_with_context(request, &mut context))
        .collect();
    drop(context);
    (responses, embedder.finish())
}

/// `run` with an embedder that answers every request with a canned response for its own URL.
fn observe(prepare: impl FnOnce(&mut FetchContext), requests: Vec<Request>) -> Vec<Seen> {
    run(|request| canned(&request.url), prepare, requests).1
}

// ---------------------------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------------------------

fn parse(text: &str) -> ServoUrl {
    ServoUrl::parse(text).unwrap()
}

/// The site key the policy gives a URL.
fn key(text: &str) -> String {
    site_key_of_url(&parse(text))
}

fn client(origin: ImmutableOrigin, nested: bool, partition_key: Option<Site>) -> RequestClient {
    RequestClient {
        preloaded_resources: Default::default(),
        policy_container: Default::default(),
        origin: Origin::Origin(origin),
        is_nested_browsing_context: nested,
        insecure_requests_policy: InsecureRequestsPolicy::DoNotUpgrade,
        has_trustworthy_ancestor_origin: false,
        network_partition_key: partition_key,
    }
}

/// A request of the document at `page` for `target`: the client is the page's window, so the request has
/// the page's partition key (a document's requests do).
fn document_request(page: &str, target: &str, destination: Destination) -> Request {
    let page = parse(page);
    RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::new(parse(target), None),
        Referrer::NoReferrer,
    )
    .destination(destination)
    .origin(page.origin())
    .client(client(
        page.origin(),
        false,
        Some(Site::from_origin(&page.origin())),
    ))
    .policy_container(Default::default())
    .build()
}

/// A request of a worker of the page at `page`: its client has no partition key (a worker's never has),
/// but the request names its web view.
fn worker_request(page: &str, target: &str) -> Request {
    let page = parse(page);
    RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::new(parse(target), None),
        Referrer::NoReferrer,
    )
    .origin(page.origin())
    .client(client(page.origin(), false, None))
    .policy_container(Default::default())
    .build()
}

/// A navigation to `target`. `initiator` is the origin of the script that started it (none for a typed
/// address, whose origin is opaque); `nested` says whether the browsing context that is navigated is a
/// frame. Like the engine's navigations, it has no partition key.
fn navigation(target: &str, initiator: Option<&str>, nested: bool) -> Request {
    let origin = initiator
        .map(|initiator| parse(initiator).origin())
        .unwrap_or_else(ImmutableOrigin::new_opaque);
    RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::new(parse(target), None),
        Referrer::NoReferrer,
    )
    .destination(Destination::Document)
    .mode(RequestMode::Navigate)
    .client(client(origin, nested, None))
    .policy_container(Default::default())
    .build()
}

/// The request as the network thread has it when it reaches `http_network_or_cache_fetch`.
fn populated(mut request: Request) -> Request {
    request.populate_request_from_client();
    request
}

fn body_of(response: &Response) -> Vec<u8> {
    match &*response.actual_response().body.lock() {
        ResponseBody::Done(data) => data.clone(),
        other => panic!("the body is not complete: {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// EC38 and D17: frame identity
// ---------------------------------------------------------------------------------------------

#[test]
fn a_first_party_subresource_carries_its_site_its_initiator_and_is_not_third_party() {
    let seen = observe(
        |_| {},
        vec![document_request(
            "http://app.example/index.html",
            "http://app.example/script.js",
            Destination::Script,
        )],
    );
    let requests = requested(&seen);
    assert_eq!(requests.len(), 1);
    let request = requests[0];
    assert_eq!(request.url.as_str(), "http://app.example/script.js");
    assert_eq!(request.top_level_site, Some(key("http://app.example/")));
    assert_eq!(
        request.initiator_origin.as_deref(),
        Some("http://app.example")
    );
    assert_eq!(request.is_third_party, Some(false));
    assert!(!request.is_for_main_frame);
}

#[test]
fn a_third_party_subresource_is_marked_third_party() {
    let seen = observe(
        |_| {},
        vec![document_request(
            "http://top.example:8080/page",
            "http://cdn.example/lib.js",
            Destination::Script,
        )],
    );
    let request = requested(&seen)[0];
    assert_eq!(request.top_level_site, Some(key("http://top.example/")));
    assert_eq!(
        request.initiator_origin.as_deref(),
        Some("http://top.example:8080")
    );
    assert_eq!(request.is_third_party, Some(true));
}

#[test]
fn a_subdomain_of_the_top_level_site_is_not_third_party() {
    let seen = observe(
        |_| {},
        vec![document_request(
            "http://www.top.example/page",
            "http://static.top.example/lib.js",
            Destination::Script,
        )],
    );
    let request = requested(&seen)[0];
    assert_eq!(request.top_level_site, Some(key("http://top.example/")));
    assert_eq!(request.is_third_party, Some(false));
}

#[test]
fn a_main_frame_navigation_is_first_party_and_the_main_frame() {
    // A typed address: opaque initiator, a client that is not nested, no partition key.
    let seen = observe(
        |_| {},
        vec![navigation("http://top.example/landing", None, false)],
    );
    let request = requested(&seen)[0];
    assert!(request.is_for_main_frame);
    assert_eq!(request.destination, Destination::Document);
    assert_eq!(request.top_level_site, Some(key("http://top.example/")));
    assert_eq!(request.is_third_party, Some(false));
    assert_eq!(request.initiator_origin, None);
}

#[test]
fn a_navigation_started_by_a_script_of_another_site_reports_that_initiator() {
    let seen = observe(
        |_| {},
        vec![navigation(
            "http://top.example/landing",
            Some("http://referrer.example"),
            false,
        )],
    );
    let request = requested(&seen)[0];
    assert!(request.is_for_main_frame);
    assert_eq!(
        request.initiator_origin.as_deref(),
        Some("http://referrer.example")
    );
    assert_eq!(request.is_third_party, Some(false));
}

#[test]
fn a_nested_navigation_with_the_destination_document_is_judged_by_the_web_view_record() {
    // The main frame started loading `http://top.example/`; the engine's `http_network_or_cache_fetch` tells
    // the policy (`note_main_frame`), and here the test does. Then a frame that a script navigated to another
    // site arrives with the destination `Document`.
    let frame = navigation(
        "http://ads.example/frame.html",
        Some("http://top.example"),
        true,
    );
    let seen = observe(
        |context| {
            let main = populated(navigation("http://top.example/", None, false));
            context.state.net_policy.note_main_frame(&main);
        },
        vec![frame],
    );
    let request = requested(&seen)[0];
    assert_eq!(request.destination, Destination::Document);
    assert!(
        !request.is_for_main_frame,
        "a frame is not the main page (D17)"
    );
    assert_eq!(request.top_level_site, Some(key("http://top.example/")));
    assert_eq!(request.is_third_party, Some(true));
    assert_eq!(
        request.initiator_origin.as_deref(),
        Some("http://top.example")
    );
}

#[test]
fn a_nested_navigation_to_the_top_level_site_is_first_party() {
    let frame = navigation(
        "http://top.example/embedded.html",
        Some("http://top.example"),
        true,
    );
    let seen = observe(
        |context| {
            let main = populated(navigation("http://top.example/", None, false));
            context.state.net_policy.note_main_frame(&main);
        },
        vec![frame],
    );
    let request = requested(&seen)[0];
    assert!(!request.is_for_main_frame);
    assert_eq!(request.is_third_party, Some(false));
}

#[test]
fn a_worker_request_is_placed_by_its_web_view_alone() {
    // A worker's client has no partition key; the request has only `target_webview_id`.
    let worker = || worker_request("http://top.example/", "http://cdn.example/api");
    let seen = observe(
        |context| {
            let main = populated(navigation("http://top.example/", None, false));
            context.state.net_policy.note_main_frame(&main);
        },
        vec![worker()],
    );
    let Seen::Requested(webview_id, request) = &seen[0] else {
        panic!("expected a request: {seen:?}");
    };
    assert_eq!(*webview_id, Some(TEST_WEBVIEW_ID));
    assert_eq!(request.top_level_site, Some(key("http://top.example/")));
    assert_eq!(request.is_third_party, Some(true));
    assert!(!request.is_for_main_frame);

    // Without a record the site of the top-level document is unknown, and the embedder is told that it is,
    // not that the request is first party.
    let seen = observe(|_| {}, vec![worker()]);
    let Seen::Requested(webview_id, request) = &seen[0] else {
        panic!("expected a request: {seen:?}");
    };
    assert_eq!(*webview_id, Some(TEST_WEBVIEW_ID));
    assert_eq!(request.top_level_site, None);
    assert_eq!(request.is_third_party, None);
    assert_eq!(
        request.initiator_origin.as_deref(),
        Some("http://top.example")
    );
}

#[test]
fn is_for_main_frame_is_true_only_for_a_top_level_navigation() {
    // A request without a client has to name its origin itself.
    let without_client = |mut request: Request| {
        request.client = None;
        request.origin = Origin::Origin(ImmutableOrigin::new_opaque());
        request
    };
    let with_no_client = without_client(navigation("http://top.example/a", None, false));
    let mut iframe_without_client = without_client(navigation("http://top.example/b", None, false));
    iframe_without_client.destination = Destination::IFrame;
    let requests = vec![
        // top-level navigation
        navigation("http://top.example/c", None, false),
        // a frame navigated by a script or a link: destination Document, nested client
        navigation("http://top.example/d", Some("http://top.example"), true),
        // no client: the destination decides, as it did before Mercury
        with_no_client,
        iframe_without_client,
        // a subresource
        document_request(
            "http://top.example/",
            "http://top.example/e.js",
            Destination::Script,
        ),
    ];
    let seen = observe(|_| {}, requests);
    let flags: Vec<(String, bool)> = requested(&seen)
        .iter()
        .map(|request| (request.url.path().to_owned(), request.is_for_main_frame))
        .collect();
    let expected: Vec<(String, bool)> = [
        ("/c", true),
        ("/d", false),
        ("/a", true),
        ("/b", false),
        ("/e.js", false),
    ]
    .into_iter()
    .map(|(path, flag)| (path.to_owned(), flag))
    .collect();
    assert_eq!(flags, expected);
}

// ---------------------------------------------------------------------------------------------
// EC37: the rewrite message
// ---------------------------------------------------------------------------------------------

/// A server that records what it receives and answers 200 with `body`.
struct Recording {
    server: Server,
    url: ServoUrl,
    hits: Arc<Mutex<Vec<(String, HeaderMap)>>>,
}

impl Recording {
    fn new(body: &'static [u8]) -> Recording {
        let hits = Arc::new(Mutex::new(Vec::new()));
        let recorded = hits.clone();
        let (server, url) = make_server(
            move |request: HyperRequest<Incoming>,
                  response: &mut HyperResponse<BoxBody<Bytes, hyper::Error>>| {
                recorded
                    .lock()
                    .unwrap()
                    .push((request.uri().to_string(), request.headers().clone()));
                *response.body_mut() = make_body(body.to_vec());
            },
        );
        Recording {
            server,
            url: url.url(),
            hits,
        }
    }

    fn uris(&self) -> Vec<String> {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .map(|(uri, _)| uri.clone())
            .collect()
    }

    fn header(&self, index: usize, name: &str) -> Option<String> {
        self.hits.lock().unwrap()[index]
            .1
            .get(name)
            .map(|value| value.to_str().unwrap().to_owned())
    }

    fn close(self) {
        self.server.close();
    }
}

fn rewrite_to(url: ServoUrl) -> Vec<WebResourceResponseMsg> {
    vec![WebResourceResponseMsg::Rewrite(WebResourceRewrite {
        url: Some(url),
        ..WebResourceRewrite::default()
    })]
}

/// `url` without its `utm_source` query parameter.
fn without_utm(url: &ServoUrl) -> ServoUrl {
    let mut stripped = url.as_url().clone();
    let kept: Vec<(String, String)> = stripped
        .query_pairs()
        .filter(|(name, _)| name != "utm_source")
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    if kept.is_empty() {
        stripped.set_query(None);
    } else {
        stripped.query_pairs_mut().clear().extend_pairs(kept);
    }
    ServoUrl::from_url(stripped)
}

/// A same-origin subresource request for `url` (a `fetch()` of the page's own server).
fn same_origin_fetch(url: &ServoUrl) -> Request {
    RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::new(url.clone(), None),
        Referrer::NoReferrer,
    )
    .origin(url.origin())
    .policy_container(Default::default())
    .build()
}

#[test]
fn a_rewrite_that_strips_a_query_parameter_reaches_the_server_without_it() {
    let server = Recording::new(b"from the server");
    let requested_url = server.url.join("page?keep=1&utm_source=x").unwrap();

    let (responses, seen) = run(
        |request| {
            // A rewrite is terminal: what follows it must be ignored, not applied.
            let mut answer = rewrite_to(without_utm(&request.url));
            answer.extend(canned(&request.url));
            answer
        },
        |_| {},
        vec![same_origin_fetch(&requested_url)],
    );
    let uris = server.uris();
    server.close();

    assert_eq!(uris, vec!["/page?keep=1".to_owned()]);
    assert!(!responses[0].is_network_error());
    assert_eq!(body_of(&responses[0]), b"from the server".to_vec());
    // The embedder was asked once, and saw the URL with the parameter.
    let requests = requested(&seen);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.query(), Some("keep=1&utm_source=x"));
}

#[test]
fn a_rewrite_to_another_origin_is_refused_for_a_subresource() {
    let page = Recording::new(b"page");
    let other = Recording::new(b"other");
    let target = other.url.join("elsewhere").unwrap();

    let (responses, _) = run(
        move |_| rewrite_to(target.clone()),
        |_| {},
        vec![same_origin_fetch(&page.url.join("resource").unwrap())],
    );
    let (page_uris, other_uris) = (page.uris(), other.uris());
    page.close();
    other.close();

    assert_eq!(page_uris, vec!["/resource".to_owned()]);
    assert!(other_uris.is_empty(), "the other origin was contacted");
    assert_eq!(body_of(&responses[0]), b"page".to_vec());
}

#[test]
fn a_rewrite_to_another_origin_is_accepted_for_a_top_level_navigation() {
    let start = Recording::new(b"start");
    let landing = Recording::new(b"landing");
    let target = landing.url.join("landing?ok=1").unwrap();

    let (responses, _) = run(
        move |_| rewrite_to(target.clone()),
        |_| {},
        vec![navigation(
            start.url.join("click?to=landing").unwrap().as_str(),
            None,
            false,
        )],
    );
    let (start_uris, landing_uris) = (start.uris(), landing.uris());
    start.close();
    landing.close();

    assert!(start_uris.is_empty(), "the original address was contacted");
    assert_eq!(landing_uris, vec!["/landing?ok=1".to_owned()]);
    assert_eq!(body_of(&responses[0]), b"landing".to_vec());
}

#[test]
fn a_rewrite_to_another_origin_is_refused_for_a_frame_navigation() {
    let start = Recording::new(b"start");
    let landing = Recording::new(b"landing");
    let target = landing.url.join("landing").unwrap();

    let (responses, _) = run(
        move |_| rewrite_to(target.clone()),
        |_| {},
        vec![navigation(
            start.url.join("frame").unwrap().as_str(),
            Some("http://top.example"),
            true,
        )],
    );
    let (start_uris, landing_uris) = (start.uris(), landing.uris());
    start.close();
    landing.close();

    assert_eq!(start_uris, vec!["/frame".to_owned()]);
    assert!(landing_uris.is_empty());
    assert_eq!(body_of(&responses[0]), b"start".to_vec());
}

#[test]
fn a_rewrite_to_an_hsts_host_is_upgraded_to_https_after_it() {
    let hits = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorded = hits.clone();
    let (server, tls_url) = make_ssl_server(
        move |request: HyperRequest<Incoming>,
              response: &mut HyperResponse<BoxBody<Bytes, hyper::Error>>| {
            recorded.lock().unwrap().push(request.uri().to_string());
            *response.body_mut() = make_body(b"secure".to_vec());
        },
    );
    // The first address is never contacted: the rewrite replaces it with the TLS server's, over `http`.
    let landing = tls_url.url().join("landing").unwrap();
    assert_eq!(landing.scheme(), "http");

    let certificates = server.certificates.clone().unwrap();
    let (responses, seen) = run(
        move |_| rewrite_to(landing.clone()),
        move |context| {
            // The server certificate is self-signed: bind an exception to `localhost` (as
            // `test_fetch_with_hsts` does) and make `localhost` an HSTS host.
            for certificate in certificates.iter() {
                context
                    .state
                    .override_manager
                    .add_override("localhost", certificate)
                    .expect("localhost is a valid host name");
            }
            context.state.hsts_list.write().push(
                HstsEntry::new("localhost".to_owned(), IncludeSubdomains::NotIncluded, None)
                    .unwrap(),
            );
        },
        vec![navigation("http://127.0.0.1:49999/start", None, false)],
    );
    server.close();

    // `scheme must stay` held (http to http); HSTS then applied to the new host, so the TLS server was
    // reached, and over https.
    let response = &responses[0];
    assert!(
        !response.is_network_error(),
        "{:?}",
        response.get_network_error()
    );
    assert_eq!(
        response
            .internal_response
            .as_ref()
            .and_then(|internal| internal.url())
            .map(|url| url.scheme()),
        Some("https")
    );
    assert_eq!(*hits.lock().unwrap(), vec!["/landing".to_owned()]);
    assert_eq!(requested(&seen).len(), 1);
}

fn set_headers(url: Option<ServoUrl>, headers: &[(&str, &str)]) -> Vec<WebResourceResponseMsg> {
    vec![WebResourceResponseMsg::Rewrite(WebResourceRewrite {
        url,
        set_headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        remove_headers: Vec::new(),
    })]
}

#[test]
fn a_header_that_is_allowed_is_applied() {
    let server = Recording::new(b"ok");
    let target = server.url.join("a").unwrap();
    let (responses, _) = run(
        |_| set_headers(None, &[("X-Mercury-Test", "1")]),
        |_| {},
        vec![same_origin_fetch(&target)],
    );
    let header = server.header(0, "x-mercury-test");
    server.close();
    assert!(!responses[0].is_network_error());
    assert_eq!(header.as_deref(), Some("1"));
}

#[test]
fn a_refused_header_name_is_not_applied_and_the_rest_of_the_rewrite_with_it() {
    for refused in ["Cookie", "Authorization", "Proxy-Authorization", "Sec-GPC"] {
        let server = Recording::new(b"ok");
        let target = server.url.join("original").unwrap();
        let changed = server.url.join("changed").unwrap();
        let (responses, _) = run(
            move |_| {
                set_headers(
                    Some(changed.clone()),
                    &[("X-Mercury-Test", "1"), (refused, "a=b")],
                )
            },
            |_| {},
            vec![same_origin_fetch(&target)],
        );
        let uris = server.uris();
        let (cookie, authorization, proxy_authorization, test) = (
            server.header(0, "cookie"),
            server.header(0, "authorization"),
            server.header(0, "proxy-authorization"),
            server.header(0, "x-mercury-test"),
        );
        server.close();

        assert!(!responses[0].is_network_error(), "{refused}");
        assert_eq!(
            uris,
            vec!["/original".to_owned()],
            "{refused}: the URL changed"
        );
        assert_eq!(cookie, None, "{refused}");
        assert_eq!(authorization, None, "{refused}");
        assert_eq!(proxy_authorization, None, "{refused}");
        assert_eq!(test, None, "{refused}: the rewrite was applied in part");
    }
}

#[test]
fn a_rewrite_changes_a_redirect_hop() {
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorded = log.clone();
    let (server, url) = make_server(
        move |request: HyperRequest<Incoming>,
              response: &mut HyperResponse<BoxBody<Bytes, hyper::Error>>| {
            recorded.lock().unwrap().push(request.uri().to_string());
            if request.uri().path() == "/first" {
                response.headers_mut().insert(
                    header::LOCATION,
                    HeaderValue::from_static("/second?utm_source=x&keep=1"),
                );
                *response.status_mut() = StatusCode::FOUND;
            } else {
                *response.body_mut() = make_body(b"done".to_vec());
            }
        },
    );
    let start = url.url().join("first").unwrap();

    let (responses, seen) = run(
        |request| {
            let tracked = request
                .url
                .query()
                .is_some_and(|query| query.contains("utm_source"));
            if request.is_redirect && tracked {
                rewrite_to(without_utm(&request.url))
            } else {
                do_not_intercept()
            }
        },
        |_| {},
        vec![same_origin_fetch(&start)],
    );
    server.close();

    assert!(!responses[0].is_network_error());
    assert_eq!(body_of(&responses[0]), b"done".to_vec());
    assert_eq!(
        *log.lock().unwrap(),
        vec!["/first".to_owned(), "/second?keep=1".to_owned()]
    );
    // The embedder saw each hop's own URL, and was asked once for each.
    let hops: Vec<(String, bool)> = requested(&seen)
        .iter()
        .map(|request| {
            (
                format!(
                    "{}?{}",
                    request.url.path(),
                    request.url.query().unwrap_or("")
                ),
                request.is_redirect,
            )
        })
        .collect();
    assert_eq!(
        hops,
        vec![
            ("/first?".to_owned(), false),
            ("/second?utm_source=x&keep=1".to_owned(), true)
        ]
    );
}

#[test]
fn an_empty_rewrite_lets_the_request_go_on_unchanged() {
    let server = Recording::new(b"ok");
    let target = server.url.join("same?q=1").unwrap();
    let (responses, seen) = run(
        |_| {
            vec![WebResourceResponseMsg::Rewrite(
                WebResourceRewrite::default(),
            )]
        },
        |_| {},
        vec![same_origin_fetch(&target)],
    );
    let uris = server.uris();
    server.close();
    assert!(!responses[0].is_network_error());
    assert_eq!(uris, vec!["/same?q=1".to_owned()]);
    assert_eq!(requested(&seen).len(), 1);
}

// ---------------------------------------------------------------------------------------------
// EC14: the referrer cap
// ---------------------------------------------------------------------------------------------

/// The referrer the embedder is shown for a request from `source` to `target` under `policy`, with the
/// cap on or off.
fn referrer_for(policy: ReferrerPolicy, source: &str, target: &str, cap: bool) -> Option<String> {
    let request = RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::new(parse(target), None),
        Referrer::ReferrerUrl(parse(source)),
    )
    .origin(parse(source).origin())
    .referrer_policy(policy)
    .policy_container(Default::default())
    .build();
    let seen = observe(
        move |context| context.state.net_policy.set_referrer_cap(cap),
        vec![request],
    );
    requested(&seen)[0]
        .referrer_url
        .as_ref()
        .map(|url| url.as_str().to_owned())
}

const PAGE: &str = "http://top.example/page?q=1";
const SECURE_PAGE: &str = "https://top.example/page?q=1";
const CROSS: &str = "http://cdn.example/x.js";
const SAME: &str = "http://top.example/x.js";

fn some(text: &str) -> Option<String> {
    Some(text.to_owned())
}

#[test]
fn the_cap_trims_the_policies_that_loosen_it_for_a_cross_origin_request() {
    // unsafe-url and no-referrer-when-downgrade send the whole URL cross-origin; the cap sends the origin.
    for policy in [
        ReferrerPolicy::UnsafeUrl,
        ReferrerPolicy::NoReferrerWhenDowngrade,
    ] {
        assert_eq!(
            referrer_for(policy, PAGE, CROSS, false),
            some(PAGE),
            "{policy:?} off"
        );
        assert_eq!(
            referrer_for(policy, PAGE, CROSS, true),
            some("http://top.example/"),
            "{policy:?} on"
        );
    }
    // origin-when-cross-origin sends an origin even on a downgrade; strict-origin-when-cross-origin does not.
    assert_eq!(
        referrer_for(
            ReferrerPolicy::OriginWhenCrossOrigin,
            SECURE_PAGE,
            CROSS,
            false
        ),
        some("https://top.example/")
    );
    assert_eq!(
        referrer_for(
            ReferrerPolicy::OriginWhenCrossOrigin,
            SECURE_PAGE,
            CROSS,
            true
        ),
        None
    );
    // The same downgrade under unsafe-url: the whole URL before, nothing with the cap.
    assert_eq!(
        referrer_for(ReferrerPolicy::UnsafeUrl, SECURE_PAGE, CROSS, false),
        some(SECURE_PAGE)
    );
    assert_eq!(
        referrer_for(ReferrerPolicy::UnsafeUrl, SECURE_PAGE, CROSS, true),
        None
    );
}

#[test]
fn the_cap_keeps_the_path_for_a_same_origin_request() {
    for policy in [
        ReferrerPolicy::UnsafeUrl,
        ReferrerPolicy::NoReferrerWhenDowngrade,
        ReferrerPolicy::OriginWhenCrossOrigin,
    ] {
        for cap in [false, true] {
            assert_eq!(
                referrer_for(policy, PAGE, SAME, cap),
                some(PAGE),
                "{policy:?}, cap {cap}"
            );
        }
    }
}

#[test]
fn the_cap_never_loosens_a_policy_that_is_already_strict() {
    for cap in [false, true] {
        // strict-origin: origin only, nothing on a downgrade
        assert_eq!(
            referrer_for(ReferrerPolicy::StrictOrigin, PAGE, CROSS, cap),
            some("http://top.example/"),
            "strict-origin, cap {cap}"
        );
        assert_eq!(
            referrer_for(ReferrerPolicy::StrictOrigin, SECURE_PAGE, CROSS, cap),
            None,
            "strict-origin downgrade, cap {cap}"
        );
        // strict-origin-when-cross-origin is the cap's own floor
        assert_eq!(
            referrer_for(
                ReferrerPolicy::StrictOriginWhenCrossOrigin,
                PAGE,
                CROSS,
                cap
            ),
            some("http://top.example/"),
            "strict-origin-when-cross-origin, cap {cap}"
        );
        // same-origin: nothing cross-origin (the cap must not turn that into an origin), the URL same-origin
        assert_eq!(
            referrer_for(ReferrerPolicy::SameOrigin, PAGE, CROSS, cap),
            None,
            "same-origin cross, cap {cap}"
        );
        assert_eq!(
            referrer_for(ReferrerPolicy::SameOrigin, PAGE, SAME, cap),
            some(PAGE),
            "same-origin same, cap {cap}"
        );
        // no-referrer and origin are as strict as the cap or stricter
        assert_eq!(
            referrer_for(ReferrerPolicy::NoReferrer, PAGE, CROSS, cap),
            None,
            "no-referrer, cap {cap}"
        );
        assert_eq!(
            referrer_for(ReferrerPolicy::Origin, PAGE, CROSS, cap),
            some("http://top.example/"),
            "origin, cap {cap}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// EC17: mixed content
// ---------------------------------------------------------------------------------------------

const SECURE: &str = "https://secure.example/page";
const INSECURE_SCRIPT: &str = "http://insecure.example/app.js";

fn secure_script_request(target: &str) -> Request {
    document_request(SECURE, target, Destination::Script)
}

fn is_mixed_content_error(response: &Response) -> bool {
    matches!(
        response.get_network_error(),
        Some(NetworkError::MixedContent)
    )
}

/// Allow mixed content on the site of `SECURE`.
fn allow_mixed_content(context: &mut FetchContext) {
    context
        .state
        .net_policy
        .set_mixed_content_allowed_on(key(SECURE), true);
}

#[test]
fn a_blocked_mixed_content_request_is_reported_once() {
    let (responses, seen) = run(
        |_| do_not_intercept(),
        |_| {},
        vec![secure_script_request(INSECURE_SCRIPT)],
    );
    assert!(is_mixed_content_error(&responses[0]));
    assert_eq!(
        reports(&seen),
        vec![(
            Some(TEST_WEBVIEW_ID),
            INSECURE_SCRIPT.to_owned(),
            Destination::Script
        )],
        "{seen:?}"
    );
}

#[test]
fn every_blocked_request_is_reported_and_only_those() {
    let (responses, seen) = run(
        |request| {
            // The middle request is loopback, which is potentially trustworthy, so it is not mixed content;
            // answer it here so that nothing needs to listen.
            if request.url.host_str() == Some("127.0.0.1") {
                canned(&request.url)
            } else {
                do_not_intercept()
            }
        },
        |_| {},
        vec![
            secure_script_request("http://insecure.example/one.js"),
            secure_script_request("http://127.0.0.1:49998/two.js"),
            secure_script_request("http://insecure.example/three.js"),
        ],
    );
    assert!(is_mixed_content_error(&responses[0]));
    assert!(!responses[1].is_network_error());
    assert!(is_mixed_content_error(&responses[2]));
    let urls: Vec<String> = reports(&seen).into_iter().map(|(_, url, _)| url).collect();
    assert_eq!(
        urls,
        vec![
            "http://insecure.example/one.js".to_owned(),
            "http://insecure.example/three.js".to_owned()
        ]
    );
}

#[test]
fn mixed_content_is_allowed_on_a_site_the_user_allowed_and_nothing_is_reported() {
    let (responses, seen) = run(
        |request| canned(&request.url),
        allow_mixed_content,
        vec![secure_script_request(INSECURE_SCRIPT)],
    );
    assert!(
        !responses[0].is_network_error(),
        "{:?}",
        responses[0].get_network_error()
    );
    assert_eq!(body_of(&responses[0]), b"canned".to_vec());
    assert!(reports(&seen).is_empty(), "{seen:?}");
}

#[test]
fn the_allow_is_for_the_site_it_names() {
    // Allowed on another site: still blocked here, and reported.
    let (responses, seen) = run(
        |_| do_not_intercept(),
        |context| {
            context
                .state
                .net_policy
                .set_mixed_content_allowed_on(key("https://other.example/"), true)
        },
        vec![secure_script_request(INSECURE_SCRIPT)],
    );
    assert!(is_mixed_content_error(&responses[0]));
    assert_eq!(reports(&seen).len(), 1);

    // Allowed and then revoked: blocked again.
    let (responses, seen) = run(
        |_| do_not_intercept(),
        |context| {
            let policy = &context.state.net_policy;
            policy.set_mixed_content_allowed_on(key(SECURE), true);
            policy.set_mixed_content_allowed_on(key(SECURE), false);
        },
        vec![secure_script_request(INSECURE_SCRIPT)],
    );
    assert!(is_mixed_content_error(&responses[0]));
    assert_eq!(reports(&seen).len(), 1);
}

#[test]
fn a_request_whose_top_level_site_is_unknown_is_not_allowed() {
    // A worker's request with no record of its web view: the site is unknown, so the allow cannot apply.
    let (responses, seen) = run(
        |_| do_not_intercept(),
        allow_mixed_content,
        vec![worker_request(SECURE, INSECURE_SCRIPT)],
    );
    assert!(is_mixed_content_error(&responses[0]));
    assert_eq!(reports(&seen).len(), 1);
}

#[test]
fn a_response_that_is_mixed_content_is_blocked_and_reported_once() {
    // The request is secure and the embedder answers it with a response that says it came from `http`:
    // the response check blocks it. The report names the request's URL.
    let secure_target = "https://secure.example/app.js";
    let insecure_response = parse(INSECURE_SCRIPT);
    let (responses, seen) = run(
        {
            let insecure_response = insecure_response.clone();
            move |_| canned(&insecure_response)
        },
        |_| {},
        vec![secure_script_request(secure_target)],
    );
    assert!(is_mixed_content_error(&responses[0]));
    assert_eq!(
        reports(&seen),
        vec![(
            Some(TEST_WEBVIEW_ID),
            secure_target.to_owned(),
            Destination::Script
        )],
        "{seen:?}"
    );

    // The same answer on an allowed site goes through, with no report.
    let (responses, seen) = run(
        move |_| canned(&insecure_response),
        allow_mixed_content,
        vec![secure_script_request(secure_target)],
    );
    assert!(!responses[0].is_network_error());
    assert!(reports(&seen).is_empty(), "{seen:?}");
}

#[test]
fn a_page_that_is_not_secure_has_no_mixed_content() {
    let (responses, seen) = run(
        |request| canned(&request.url),
        |_| {},
        vec![document_request(
            "http://plain.example/page",
            INSECURE_SCRIPT,
            Destination::Script,
        )],
    );
    assert!(!responses[0].is_network_error());
    assert!(reports(&seen).is_empty());
}
