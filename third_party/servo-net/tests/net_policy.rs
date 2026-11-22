// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: tests for the network policy handle and the sites of a request; see net_policy.rs.

//! The network policy handle and the sites of a request.
//!
//! No network is used: the tests build requests by hand (as script would, with a client, an origin and a
//! partition key) and ask the policy who is first party. The party matrix is the contract of decisions D3
//! and D4 of the net-1 lane: the top-level site comes from the client's key (script fills it for documents,
//! workers and nested navigations), then from a top-level navigation, and otherwise is unknown, which every
//! security decision treats as third party. The policy keeps no record of what a web view loads.

use std::sync::Arc;

use net::net_policy::{NetPolicy, NetPolicyStats, Party, RequestSites, site_key_of_url};
use net_traits::ReferrerPolicy;
use net_traits::blob_url_store::UrlWithBlobClaim;
use net_traits::policy_container::PolicyContainer;
use net_traits::pub_domains::Site;
use net_traits::request::{
    Destination, Initiator, InsecureRequestsPolicy, Origin, PreloadedResources, Referrer,
    RequestBuilder, RequestClient, RequestMode,
};
use servo_base::id::{BrowsingContextId, Index, PipelineNamespaceId, TEST_WEBVIEW_ID, WebViewId};
use servo_url::{ImmutableOrigin, ServoUrl};

// What the other test files of the net-1 lane build requests with (`cookie_policy.rs` and the request-path
// tests use them): a request as script would build it, with a client, an origin and a partition key.

/// One test at a time may configure the process-wide policy (`NetPolicy::process()`): the resource thread of
/// `new_core_resource_thread` takes that one, and the test that says it starts as Servo reads it.
pub(crate) static PROCESS_POLICY_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub(crate) fn parse(url: &str) -> ServoUrl {
    ServoUrl::parse(url).unwrap()
}

pub(crate) fn claim(url: &str) -> UrlWithBlobClaim {
    UrlWithBlobClaim::new(parse(url), None)
}

pub(crate) fn origin_of(url: &str) -> ImmutableOrigin {
    parse(url).origin()
}

pub(crate) fn site_of(url: &str) -> Site {
    Site::from_origin(&origin_of(url))
}

pub(crate) fn client(origin: ImmutableOrigin, nested: bool, key: Option<Site>) -> RequestClient {
    RequestClient {
        preloaded_resources: PreloadedResources::default(),
        policy_container: Arc::new(PolicyContainer::default()),
        origin: Origin::Origin(origin),
        is_nested_browsing_context: nested,
        insecure_requests_policy: InsecureRequestsPolicy::DoNotUpgrade,
        has_trustworthy_ancestor_origin: false,
        network_partition_key: key,
        browser_initiated_navigation: false,
    }
}

/// A web view that is not `TEST_WEBVIEW_ID`: the `n`th one of a test namespace.
pub(crate) fn web_view(n: u32) -> WebViewId {
    WebViewId::mock_for_testing(BrowsingContextId {
        namespace_id: PipelineNamespaceId(4321),
        index: Index::new(n).unwrap(),
    })
}

/// A subresource request that a document at `document` makes, in a page whose top level is `top_level`.
pub(crate) fn subresource(url: &str, document: &str, top_level: &str) -> RequestBuilder {
    let origin = origin_of(document);
    RequestBuilder::new(Some(TEST_WEBVIEW_ID), claim(url), Referrer::NoReferrer)
        .destination(Destination::Image)
        .origin(origin.clone())
        .client(client(origin, false, Some(site_of(top_level))))
}

/// A navigation request as `navigation.rs` builds it: the initiator's origin as the client's origin (opaque when
/// the browser started it, as for a typed URL), the destination `Document`, and no partition key (a top-level
/// load has none; `nested_navigation` is the one that carries the parent's). A load with no initiator is the
/// browser's own: `browser_initiated_navigation`.
pub(crate) fn navigation(
    url: &str,
    initiator: Option<&str>,
    nested: bool,
    web_view: WebViewId,
) -> RequestBuilder {
    let origin = initiator.map_or_else(ImmutableOrigin::new_opaque, origin_of);
    let mut request_client = client(origin.clone(), nested, None);
    request_client.browser_initiated_navigation = initiator.is_none();
    RequestBuilder::new(Some(web_view), claim(url), Referrer::NoReferrer)
        .destination(Destination::Document)
        .mode(RequestMode::Navigate)
        .origin(origin)
        .client(request_client)
}

/// The request that loads the page of a top-level browsing context, started by the browser (a typed URL).
pub(crate) fn top_level_navigation(url: &str, web_view: WebViewId) -> RequestBuilder {
    navigation(url, None, false, web_view)
}

/// The request that loads a frame by a navigation (`<iframe src>`, `location.href`, a link): destination
/// `Document` like a top-level page, but the client is in a nested browsing context. It carries the partition
/// key of the parent document, as `ScriptThread::network_partition_key_of_load` gives it.
pub(crate) fn nested_navigation(url: &str, parent: &str, web_view: WebViewId) -> RequestBuilder {
    let mut builder = navigation(url, Some(parent), true, web_view);
    if let Some(request_client) = builder.client.as_mut() {
        request_client.network_partition_key = Some(site_of(parent));
    }
    builder
}

/// A nested navigation whose parent document the script thread could not see (it lives in another process):
/// no key.
pub(crate) fn nested_navigation_without_key(
    url: &str,
    parent: &str,
    web_view: WebViewId,
) -> RequestBuilder {
    navigation(url, Some(parent), true, web_view)
}

/// A navigation that a script started from an opaque origin (a sandboxed frame, a `data:` document): the
/// client's origin is opaque, like a typed URL's, but it is not the browser's own.
pub(crate) fn script_navigation_from_opaque_origin(
    url: &str,
    web_view: WebViewId,
) -> RequestBuilder {
    let origin = ImmutableOrigin::new_opaque();
    RequestBuilder::new(Some(web_view), claim(url), Referrer::NoReferrer)
        .destination(Destination::Document)
        .mode(RequestMode::Navigate)
        .origin(origin.clone())
        .client(client(origin, false, None))
}

/// A request of a worker (or of a document) whose page is `creator`: the client carries the key of the
/// document that created it, as `prepare_workerscope_init` hands it to the worker.
pub(crate) fn worker_request(url: &str, creator: &str, web_view: WebViewId) -> RequestBuilder {
    let origin = origin_of(creator);
    RequestBuilder::new(Some(web_view), claim(url), Referrer::NoReferrer)
        .destination(Destination::None)
        .origin(origin.clone())
        .client(client(origin, false, Some(site_of(creator))))
}

pub(crate) fn sites(policy: &NetPolicy, builder: RequestBuilder) -> RequestSites {
    policy.sites_of_request(&builder.build())
}

#[test]
fn site_keys_are_scheme_less_registrable_domains() {
    let cases = [
        ("https://example.com/", "example.com"),
        ("http://example.com/", "example.com"),
        ("https://www.example.com/x?y#z", "example.com"),
        ("http://a.b.example.com:8080/", "example.com"),
        ("HTTP://WWW.EXAMPLE.COM/", "example.com"),
        ("https://example.co.uk/", "example.co.uk"),
        ("https://foo.bar.example.co.uk/", "example.co.uk"),
        // A public suffix is its own site.
        ("https://co.uk/", "co.uk"),
        // IP literals and single labels are their own key.
        ("http://127.0.0.1:3000/", "127.0.0.1"),
        ("http://[::1]:3000/", "[::1]"),
        ("http://localhost:80/", "localhost"),
        ("http://printer/", "printer"),
        // The ports and the scheme do not matter, the web socket schemes are sites too.
        ("wss://a.example.com:9000/socket", "example.com"),
        // An opaque origin has no site.
        ("data:text/plain,hello", ""),
    ];
    for (url, expected) in cases {
        assert_eq!(site_key_of_url(&parse(url)), expected, "{url}");
    }
}

#[test]
fn party_matrix_of_a_subresource_request() {
    let policy = NetPolicy::new();
    let cases = [
        // (top-level page, request URL, party)
        ("https://a.example/", "https://a.example/x", Party::First),
        // Same site, different subdomain.
        (
            "https://www.a.example/",
            "https://cdn.a.example/x",
            Party::First,
        ),
        // A different scheme on the same domain is the same site (the scheme-less rule, D4).
        ("https://a.example/", "http://a.example/x", Party::First),
        ("http://a.example/", "wss://a.example/x", Party::First),
        // Another site.
        ("https://a.example/", "https://b.example/x", Party::Third),
        // The ports do not make a site.
        ("http://localhost:1/", "http://localhost:2/x", Party::First),
        // `localhost` and `127.0.0.1` are different sites; an IP literal is its own key.
        ("http://localhost:1/", "http://127.0.0.1:1/x", Party::Third),
        ("http://127.0.0.1:1/", "http://127.0.0.1:2/x", Party::First),
        ("http://127.0.0.1/", "http://127.0.0.2/x", Party::Third),
        ("http://[::1]/", "http://[::1]:99/x", Party::First),
        ("http://[::1]/", "http://127.0.0.1/x", Party::Third),
        // A single label is its own site.
        ("http://printer/", "http://printer:8080/x", Party::First),
        ("http://printer/", "http://scanner/x", Party::Third),
        // The public suffix list decides what a site is.
        (
            "https://a.example.co.uk/",
            "https://b.example.co.uk/x",
            Party::First,
        ),
        ("https://a.co.uk/", "https://b.co.uk/x", Party::Third),
    ];
    for (top_level, url, party) in cases {
        let found = sites(&policy, subresource(url, top_level, top_level));
        assert_eq!(found.party, party, "{top_level} -> {url}");
        assert!(!found.top_level_navigation, "{top_level} -> {url}");
        assert_eq!(found.request, site_key_of_url(&parse(url)));
    }
}

#[test]
fn an_opaque_top_level_site_is_third_party_to_everything() {
    let policy = NetPolicy::new();
    // The partition key of a document whose top-level origin is opaque (a sandboxed or `data:` page).
    let opaque_key = Site::from_origin(&ImmutableOrigin::new_opaque());
    let origin = origin_of("https://a.example/");
    let builder = RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        claim("https://a.example/x"),
        Referrer::NoReferrer,
    )
    .origin(origin.clone())
    .client(client(origin, false, Some(opaque_key)));
    let found = sites(&policy, builder);
    assert_eq!(found.party, Party::Third);
    assert_eq!(found.top_level.as_deref(), Some(""));
}

#[test]
fn the_initiator_is_the_site_of_the_requests_origin() {
    let policy = NetPolicy::new();
    let found = sites(
        &policy,
        subresource(
            "https://b.example/x",
            "https://sub.a.example/",
            "https://a.example/",
        ),
    );
    assert_eq!(found.initiator.as_deref(), Some("a.example"));

    // An opaque origin (a typed URL, a sandboxed frame) has no initiator site.
    let found = sites(
        &policy,
        top_level_navigation("https://a.example/", TEST_WEBVIEW_ID),
    );
    assert_eq!(found.initiator, None);

    // A request that still says "client" takes its client's origin.
    let origin = origin_of("https://c.example/");
    let builder = RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        claim("https://b.example/x"),
        Referrer::NoReferrer,
    )
    .client(client(origin, false, Some(site_of("https://c.example/"))));
    let found = sites(&policy, builder);
    assert_eq!(found.initiator.as_deref(), Some("c.example"));
}

#[test]
fn a_request_without_a_top_level_site_is_unknown() {
    let policy = NetPolicy::new();

    // A client that has no key (a worker whose creator the script could not name).
    let origin = origin_of("https://a.example/");
    let builder = RequestBuilder::new(
        Some(web_view(900)),
        claim("https://a.example/worker-fetch"),
        Referrer::NoReferrer,
    )
    .origin(origin.clone())
    .client(client(origin.clone(), false, None));
    let found = sites(&policy, builder);
    assert_eq!(found.party, Party::Unknown);
    assert_eq!(found.top_level, None);

    // No client and no web view at all (an internal request).
    let builder = RequestBuilder::new(None, claim("https://a.example/x"), Referrer::NoReferrer)
        .origin(origin);
    let found = sites(&policy, builder);
    assert_eq!(found.party, Party::Unknown);
}

#[test]
fn a_top_level_navigation_is_first_party_by_construction_at_every_hop() {
    let policy = NetPolicy::new();
    let found = sites(
        &policy,
        top_level_navigation("https://a.example/page", TEST_WEBVIEW_ID),
    );
    assert!(found.top_level_navigation);
    assert_eq!(found.party, Party::First);
    assert_eq!(found.top_level.as_deref(), Some("a.example"));
    assert_eq!(found.request, "a.example");

    // After a redirect the current URL is on another site, and so is the top level.
    let builder = top_level_navigation("https://a.example/page", TEST_WEBVIEW_ID).url_list(vec![
        claim("https://a.example/page"),
        claim("https://b.example/landing"),
    ]);
    let found = sites(&policy, builder);
    assert!(found.top_level_navigation);
    assert_eq!(found.party, Party::First);
    assert_eq!(found.top_level.as_deref(), Some("b.example"));
    assert_eq!(found.request, "b.example");
}

#[test]
fn the_destination_document_alone_does_not_make_a_top_level_navigation() {
    let policy = NetPolicy::new();
    let web_view = web_view(1);

    // A frame navigation carries the parent's key: third party to the top-level page, or first party.
    let nested = nested_navigation("https://ads.example/frame", "https://top.example/", web_view);
    let found = sites(&policy, nested);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Third);
    assert_eq!(found.top_level.as_deref(), Some("top.example"));

    let nested = nested_navigation("https://top.example/other", "https://top.example/", web_view);
    assert_eq!(sites(&policy, nested).party, Party::First);

    // A frame whose parent the script thread could not see has no key: unknown, never first party, even when
    // it goes to the site of its initiator.
    for target in ["https://ads.example/frame", "https://top.example/other"] {
        let nested = nested_navigation_without_key(target, "https://top.example/", web_view);
        let found = sites(&policy, nested);
        assert!(!found.top_level_navigation);
        assert_eq!(found.party, Party::Unknown, "{target}");
        assert_eq!(found.top_level, None);
    }

    // Mode `Navigate` and destination `Document` with no client at all: not a top-level navigation either.
    let no_client = RequestBuilder::new(
        Some(web_view),
        claim("https://ads.example/frame"),
        Referrer::NoReferrer,
    )
    .destination(Destination::Document)
    .mode(RequestMode::Navigate)
    .origin(origin_of("https://top.example/"));
    let found = sites(&policy, no_client);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Unknown);

    // A top-level client, but the mode is not `Navigate` (a `fetch` of a document), or the initiator is set (a
    // prefetch): not a navigation of the top-level browsing context.
    let origin = origin_of("https://top.example/");
    let fetch_of_a_document = RequestBuilder::new(
        Some(web_view),
        claim("https://ads.example/doc"),
        Referrer::NoReferrer,
    )
    .destination(Destination::Document)
    .origin(origin.clone())
    .client(client(origin.clone(), false, None));
    let found = sites(&policy, fetch_of_a_document);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Unknown);

    let prefetch =
        top_level_navigation("https://ads.example/doc", web_view).initiator(Initiator::Prefetch);
    let found = sites(&policy, prefetch);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Unknown);
}

#[test]
fn a_nested_frame_with_the_destination_iframe_is_judged_by_its_parents_key_too() {
    let policy = NetPolicy::new();
    let frame = nested_navigation("https://ads.example/frame", "https://top.example/", web_view(3))
        .destination(Destination::IFrame);
    let found = sites(&policy, frame);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Third);
}

#[test]
fn a_worker_request_is_judged_by_the_key_of_the_document_that_created_it() {
    let policy = NetPolicy::new();
    let web_view = web_view(4);

    assert_eq!(
        sites(
            &policy,
            worker_request("https://top.example/api", "https://top.example/", web_view)
        )
        .party,
        Party::First
    );
    let tracker = sites(
        &policy,
        worker_request("https://tracker.example/api", "https://top.example/", web_view),
    );
    assert_eq!(tracker.party, Party::Third);
    assert_eq!(tracker.top_level.as_deref(), Some("top.example"));

    // A worker with no key (its creator is not known) is unknown, whatever the web view did or does.
    let origin = origin_of("https://top.example/");
    let keyless = RequestBuilder::new(
        Some(web_view),
        claim("https://top.example/api"),
        Referrer::NoReferrer,
    )
    .origin(origin.clone())
    .client(client(origin, false, None));
    assert_eq!(sites(&policy, keyless).party, Party::Unknown);
}

/// The page that a web view shows is the key of its documents, and nothing else: a page that starts a
/// navigation and never finishes it (cancelled, answered 204, turned into a download, stalled) leaves the
/// policy exactly as it was (net-1 review, finding 1). The first version recorded the site of every
/// top-level navigation when it started, so a worker or a frame of the old page then counted the new site as
/// its own.
#[test]
fn a_navigation_that_is_started_and_never_finished_changes_nothing_for_workers_and_frames() {
    let policy = NetPolicy::new();
    let web_view = web_view(5);
    let judged = |policy: &NetPolicy| {
        [
            // The page's own workers, with the key of the page they were created by.
            sites(
                policy,
                worker_request("https://page.example/api", "https://page.example/", web_view),
            )
            .party,
            sites(
                policy,
                worker_request("https://tracker.example/p", "https://page.example/", web_view),
            )
            .party,
            // A frame of the page that navigates.
            sites(
                policy,
                nested_navigation("https://tracker.example/f", "https://page.example/", web_view),
            )
            .party,
            sites(
                policy,
                nested_navigation("https://page.example/f", "https://page.example/", web_view),
            )
            .party,
            // A worker with no key.
            sites(
                policy,
                RequestBuilder::new(
                    Some(web_view),
                    claim("https://tracker.example/p"),
                    Referrer::NoReferrer,
                )
                .client(client(origin_of("https://page.example/"), false, None)),
            )
            .party,
        ]
    };
    let before = judged(&policy);
    assert_eq!(
        before,
        [
            Party::First,
            Party::Third,
            Party::Third,
            Party::First,
            Party::Unknown
        ]
    );

    // The page starts a top-level navigation to the tracker. Whatever becomes of it, the policy has seen
    // requests (asking for their sites is all that the network layer does with one) and nothing else.
    for _ in 0..3 {
        let navigation = top_level_navigation("https://tracker.example/x", web_view);
        assert_eq!(sites(&policy, navigation).party, Party::First);
        assert_eq!(judged(&policy), before);
    }
}

#[test]
fn a_documents_own_key_decides_and_nothing_else_does() {
    let policy = NetPolicy::new();
    let web_view = web_view(6);
    let origin = origin_of("https://new.example/");
    let builder = RequestBuilder::new(
        Some(web_view),
        claim("https://new.example/api"),
        Referrer::NoReferrer,
    )
    .origin(origin.clone())
    .client(client(origin, false, Some(site_of("https://new.example/"))));
    let found = sites(&policy, builder);
    assert_eq!(found.party, Party::First);
    assert_eq!(found.top_level.as_deref(), Some("new.example"));

    // Another web view's navigation changes nothing here.
    let _ = sites(
        &policy,
        top_level_navigation("https://old.example/", web_view),
    );
    let origin = origin_of("https://new.example/");
    let builder = RequestBuilder::new(
        Some(web_view),
        claim("https://old.example/api"),
        Referrer::NoReferrer,
    )
    .origin(origin.clone())
    .client(client(origin, false, Some(site_of("https://new.example/"))));
    assert_eq!(sites(&policy, builder).party, Party::Third);
}

#[test]
fn a_builder_and_the_request_built_from_it_have_the_same_sites() {
    let policy = NetPolicy::new();
    let web_view = web_view(30);

    let builders = [
        subresource(
            "https://b.example/x",
            "https://a.example/",
            "https://a.example/",
        ),
        subresource(
            "https://a.example/x",
            "https://a.example/",
            "https://a.example/",
        ),
        top_level_navigation("https://a.example/", web_view),
        top_level_navigation("https://a.example/", web_view).url_list(vec![
            claim("https://a.example/"),
            claim("https://b.example/"),
        ]),
        nested_navigation("https://ads.example/", "https://top.example/", web_view),
        nested_navigation_without_key("https://ads.example/", "https://top.example/", web_view),
        worker_request("https://ads.example/", "https://top.example/", web_view),
    ];
    for builder in builders {
        assert_eq!(
            policy.sites_of_builder(&builder),
            policy.sites_of_request(&builder.clone().build()),
            "{}",
            builder.url.as_str()
        );
    }
}

#[test]
fn the_sites_of_a_document_come_from_its_key_alone() {
    let policy = NetPolicy::new();
    let own_page = parse("https://top.example/page");
    let frame = parse("https://frame.example/page");
    let top_key = Some(site_of("https://top.example/"));

    let found = policy.sites_of_document(&own_page, &top_key);
    assert_eq!(found.party, Party::First);
    assert_eq!(found.top_level.as_deref(), Some("top.example"));
    assert!(!found.top_level_navigation);

    assert_eq!(
        policy.sites_of_document(&frame, &top_key).party,
        Party::Third
    );

    // No key: unknown.
    assert_eq!(
        policy.sites_of_document(&own_page, &None).party,
        Party::Unknown
    );
}

fn blocking_policy() -> NetPolicy {
    let policy = NetPolicy::new();
    policy.set_block_third_party_cookies(true);
    policy
}

#[test]
fn cookies_are_allowed_for_first_party_and_refused_for_third_and_unknown_party() {
    let policy = blocking_policy();
    let first = sites(
        &policy,
        subresource(
            "https://a.example/x",
            "https://a.example/",
            "https://a.example/",
        ),
    );
    let third = sites(
        &policy,
        subresource(
            "https://b.example/x",
            "https://a.example/",
            "https://a.example/",
        ),
    );
    let unknown = sites(
        &policy,
        RequestBuilder::new(None, claim("https://b.example/x"), Referrer::NoReferrer)
            .origin(origin_of("https://a.example/")),
    );
    assert_eq!(first.party, Party::First);
    assert_eq!(third.party, Party::Third);
    assert_eq!(unknown.party, Party::Unknown);

    assert!(policy.cookies_allowed(&first));
    assert!(!policy.cookies_allowed(&third));
    assert!(!policy.cookies_allowed(&unknown));
}

#[test]
fn a_site_on_the_allow_list_gets_its_third_party_cookies() {
    let policy = blocking_policy();
    let third = |top: &str| sites(&policy, subresource("https://widget.example/x", top, top));
    assert!(!policy.cookies_allowed(&third("https://a.example/")));

    policy.allow_third_party_cookies_on("a.example".to_owned(), true);
    assert!(policy.cookies_allowed(&third("https://a.example/")));
    // Subdomains of the allowed site are the same site; another site is not allowed.
    assert!(policy.cookies_allowed(&third("https://www.a.example/")));
    assert!(!policy.cookies_allowed(&third("https://b.example/")));

    // Written in upper case, the same key.
    policy.allow_third_party_cookies_on("B.Example".to_owned(), true);
    assert!(policy.cookies_allowed(&third("https://b.example/")));

    policy.allow_third_party_cookies_on("a.example".to_owned(), false);
    assert!(!policy.cookies_allowed(&third("https://a.example/")));
    assert!(policy.cookies_allowed(&third("https://b.example/")));

    // The empty key (an opaque site) is never allowed, and an unknown top level is never on the list.
    policy.allow_third_party_cookies_on(String::new(), true);
    let opaque_key = Site::from_origin(&ImmutableOrigin::new_opaque());
    let origin = origin_of("https://a.example/");
    let opaque = RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        claim("https://widget.example/x"),
        Referrer::NoReferrer,
    )
    .origin(origin.clone())
    .client(client(origin, false, Some(opaque_key)));
    assert!(!policy.cookies_allowed(&sites(&policy, opaque)));
    let unknown = sites(
        &policy,
        RequestBuilder::new(
            None,
            claim("https://widget.example/x"),
            Referrer::NoReferrer,
        )
        .origin(origin_of("https://a.example/")),
    );
    assert!(!policy.cookies_allowed(&unknown));
}

#[test]
fn switching_blocking_off_allows_everything_again() {
    let policy = blocking_policy();
    let third = sites(
        &policy,
        subresource(
            "https://b.example/x",
            "https://a.example/",
            "https://a.example/",
        ),
    );
    assert!(!policy.cookies_allowed(&third));
    policy.set_block_third_party_cookies(false);
    assert!(policy.cookies_allowed(&third));
    policy.set_block_third_party_cookies(true);
    assert!(!policy.cookies_allowed(&third));
}

#[test]
fn the_stats_count_each_refusal() {
    let policy = blocking_policy();
    assert_eq!(policy.stats(), NetPolicyStats::default());
    let first = sites(
        &policy,
        subresource(
            "https://a.example/x",
            "https://a.example/",
            "https://a.example/",
        ),
    );
    let third = sites(
        &policy,
        subresource(
            "https://b.example/x",
            "https://a.example/",
            "https://a.example/",
        ),
    );
    assert!(policy.cookies_allowed(&first));
    assert_eq!(policy.stats().third_party_cookies_blocked, 0);
    assert!(!policy.cookies_allowed(&third));
    assert!(!policy.cookies_allowed(&third));
    assert_eq!(policy.stats().third_party_cookies_blocked, 2);

    // An allowed or switched-off decision is not a refusal.
    policy.allow_third_party_cookies_on("a.example".to_owned(), true);
    assert!(policy.cookies_allowed(&third));
    policy.set_block_third_party_cookies(false);
    assert!(policy.cookies_allowed(&third));
    assert_eq!(policy.stats().third_party_cookies_blocked, 2);
}

const ALL_REFERRER_POLICIES: [ReferrerPolicy; 9] = [
    ReferrerPolicy::EmptyString,
    ReferrerPolicy::NoReferrer,
    ReferrerPolicy::NoReferrerWhenDowngrade,
    ReferrerPolicy::Origin,
    ReferrerPolicy::SameOrigin,
    ReferrerPolicy::OriginWhenCrossOrigin,
    ReferrerPolicy::UnsafeUrl,
    ReferrerPolicy::StrictOrigin,
    ReferrerPolicy::StrictOriginWhenCrossOrigin,
];

#[test]
fn the_referrer_cap_loosens_nothing() {
    let policy = NetPolicy::new();
    policy.set_referrer_cap(true);
    for policy_in in ALL_REFERRER_POLICIES {
        let capped = policy.cap_referrer_policy(policy_in);
        match policy_in {
            ReferrerPolicy::UnsafeUrl
            | ReferrerPolicy::NoReferrerWhenDowngrade
            | ReferrerPolicy::OriginWhenCrossOrigin => {
                assert_eq!(
                    capped,
                    ReferrerPolicy::StrictOriginWhenCrossOrigin,
                    "{policy_in:?}"
                )
            },
            // `strict-origin`, `same-origin`, `origin`, `no-referrer`, the empty string and the default
            // are never changed: `strict-origin` and `same-origin` send less, `origin` is already the origin
            // and the empty string means "use the default".
            other => assert_eq!(capped, other),
        }
    }
    policy.set_referrer_cap(false);
    for policy_in in ALL_REFERRER_POLICIES {
        assert_eq!(policy.cap_referrer_policy(policy_in), policy_in);
    }
}

#[test]
fn mixed_content_is_allowed_only_on_a_listed_top_level_site() {
    let policy = NetPolicy::new();
    let request = |top: &str| sites(&policy, subresource("http://insecure.example/x", top, top));
    assert!(!policy.mixed_content_allowed(&request("https://a.example/")));

    policy.set_mixed_content_allowed_on("a.example".to_owned(), true);
    assert!(policy.mixed_content_allowed(&request("https://a.example/")));
    assert!(policy.mixed_content_allowed(&request("https://www.a.example/")));
    assert!(!policy.mixed_content_allowed(&request("https://b.example/")));

    policy.set_mixed_content_allowed_on("a.example".to_owned(), false);
    assert!(!policy.mixed_content_allowed(&request("https://a.example/")));

    // An unknown top level is never allowed.
    let unknown = sites(
        &policy,
        RequestBuilder::new(
            None,
            claim("http://insecure.example/x"),
            Referrer::NoReferrer,
        ),
    );
    policy.set_mixed_content_allowed_on(String::new(), true);
    assert!(!policy.mixed_content_allowed(&unknown));
}

#[test]
fn a_new_policy_behaves_as_servo_does() {
    let policy = NetPolicy::new();
    assert!(!policy.block_third_party_cookies());
    assert!(!policy.same_site_enforcement());
    assert!(!policy.same_site_lax_default());
    assert!(!policy.global_privacy_control());
    assert!(!policy.referrer_cap());
    assert_eq!(policy.stats(), NetPolicyStats::default());

    // Every cookie decision says yes, whoever is asking.
    let unknown = sites(
        &policy,
        RequestBuilder::new(None, claim("https://b.example/x"), Referrer::NoReferrer),
    );
    let third = sites(
        &policy,
        subresource(
            "https://b.example/x",
            "https://a.example/",
            "https://a.example/",
        ),
    );
    assert!(policy.cookies_allowed(&unknown));
    assert!(policy.cookies_allowed(&third));
    assert_eq!(policy.stats().third_party_cookies_blocked, 0);

    // The referrer policy is returned as it is, and no mixed content is allowed (Servo blocks it).
    for policy_in in ALL_REFERRER_POLICIES {
        assert_eq!(policy.cap_referrer_policy(policy_in), policy_in);
    }
    assert!(!policy.mixed_content_allowed(&third));
}

#[test]
fn a_clone_is_the_same_policy_and_every_new_one_is_its_own() {
    let policy = NetPolicy::new();
    let clone = policy.clone();
    assert!(policy.is_same_policy(&clone));
    assert!(!policy.is_same_policy(&NetPolicy::new()));

    policy.set_block_third_party_cookies(true);
    policy.set_same_site_enforcement(true);
    policy.set_global_privacy_control(true);
    policy.set_referrer_cap(true);
    policy.set_same_site_lax_default(true);
    assert!(clone.block_third_party_cookies());
    assert!(clone.same_site_enforcement());
    assert!(clone.global_privacy_control());
    assert!(clone.referrer_cap());
    assert!(clone.same_site_lax_default());
    // Another policy is untouched.
    assert!(!NetPolicy::new().block_third_party_cookies());
}

#[test]
fn the_process_policy_is_one_instance_that_starts_as_servo() {
    // Only the tests of `cookie_policy.rs` that go through a resource thread configure it, one at a time, and put
    // it back: it is what both HttpStates of the resource thread get, so every other test of the crate that
    // goes through a resource thread runs on the unpatched behaviour.
    let _only_test_on_the_process_policy = PROCESS_POLICY_LOCK.lock();
    let policy = NetPolicy::process();
    assert!(policy.is_same_policy(&NetPolicy::process()));
    assert!(!policy.block_third_party_cookies());
    assert!(!policy.same_site_enforcement());
    assert!(!policy.same_site_lax_default());
    assert!(!policy.global_privacy_control());
    assert!(!policy.referrer_cap());
}

#[test]
fn a_policy_prints_its_switches() {
    let policy = NetPolicy::new();
    policy.set_global_privacy_control(true);
    let printed = format!("{policy:?}");
    assert!(
        printed.contains("global_privacy_control: true"),
        "{printed}"
    );
    assert!(
        printed.contains("block_third_party_cookies: false"),
        "{printed}"
    );
}
