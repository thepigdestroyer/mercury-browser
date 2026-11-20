// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: tests for the network policy handle and the sites of a request; see net_policy.rs.

//! The network policy handle and the sites of a request.
//!
//! No network is used: the tests build requests by hand (as script would, with a client, an origin and a
//! partition key) and ask the policy who is first party. The party matrix is the contract of decisions D3
//! and D4 of the net-1 lane: the top-level site comes from the client's key, then from a top-level
//! navigation, then from the policy's own record of the web view, and otherwise is unknown, which every
//! security decision treats as third party.

use std::sync::Arc;

use net::net_policy::{
    MAX_RECORDED_WEB_VIEWS, NetPolicy, NetPolicyStats, Party, RequestSites, site_key_of_url,
};
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

/// A navigation request as `navigation.rs` builds it: no partition key, the initiator's origin as the
/// client's origin (opaque when the browser started it, as for a typed URL), the destination `Document`.
pub(crate) fn navigation(
    url: &str,
    initiator: Option<&str>,
    nested: bool,
    web_view: WebViewId,
) -> RequestBuilder {
    let origin = initiator.map_or_else(ImmutableOrigin::new_opaque, origin_of);
    RequestBuilder::new(Some(web_view), claim(url), Referrer::NoReferrer)
        .destination(Destination::Document)
        .mode(RequestMode::Navigate)
        .origin(origin.clone())
        .client(client(origin, nested, None))
}

/// The request that loads the page of a top-level browsing context, started by the browser (a typed URL).
pub(crate) fn top_level_navigation(url: &str, web_view: WebViewId) -> RequestBuilder {
    navigation(url, None, false, web_view)
}

/// The request that loads a frame by a navigation (`<iframe src>`, `location.href`, a link): destination
/// `Document` like a top-level page, but the client is in a nested browsing context.
pub(crate) fn nested_navigation(url: &str, parent: &str, web_view: WebViewId) -> RequestBuilder {
    navigation(url, Some(parent), true, web_view)
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

    // A client that has no key (a worker's), a web view that no main frame has loaded in.
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
    assert_eq!(found.top_level_origin, None);

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
    // The record is only what note_main_frame writes: asking does not write it.
    assert_eq!(policy.recorded_web_views(), 0);

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
    let recorded = web_view(1);
    policy.note_main_frame(&top_level_navigation("https://top.example/", recorded).build());

    // Each of these has the destination `Document` and is judged by the record, not as top level.
    let nested = nested_navigation(
        "https://ads.example/frame",
        "https://top.example/",
        recorded,
    );
    let found = sites(&policy, nested);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Third);
    assert_eq!(found.top_level.as_deref(), Some("top.example"));
    assert_eq!(
        found.top_level_origin.as_deref(),
        Some("https://top.example")
    );

    // A frame that navigates to the top-level page's own site is first party.
    let nested = nested_navigation(
        "https://top.example/other",
        "https://top.example/",
        recorded,
    );
    assert_eq!(sites(&policy, nested).party, Party::First);

    // The same frame in a web view the policy has not seen is unknown.
    let nested = nested_navigation(
        "https://ads.example/frame",
        "https://top.example/",
        web_view(2),
    );
    let found = sites(&policy, nested);
    assert_eq!(found.party, Party::Unknown);
    assert_eq!(found.top_level, None);

    // Mode `Navigate` and destination `Document` with no client at all: not top level either.
    let no_client = RequestBuilder::new(
        Some(recorded),
        claim("https://ads.example/frame"),
        Referrer::NoReferrer,
    )
    .destination(Destination::Document)
    .mode(RequestMode::Navigate)
    .origin(origin_of("https://top.example/"));
    let found = sites(&policy, no_client);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Third);

    // A top-level client, but the mode is not `Navigate` (a `fetch` of a document), or the initiator is set (a
    // prefetch): not a navigation of the top-level browsing context.
    let origin = origin_of("https://top.example/");
    let fetch_of_a_document = RequestBuilder::new(
        Some(recorded),
        claim("https://ads.example/doc"),
        Referrer::NoReferrer,
    )
    .destination(Destination::Document)
    .origin(origin.clone())
    .client(client(origin.clone(), false, None));
    let found = sites(&policy, fetch_of_a_document);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Third);

    let prefetch =
        top_level_navigation("https://ads.example/doc", recorded).initiator(Initiator::Prefetch);
    let found = sites(&policy, prefetch);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Third);
}

#[test]
fn a_nested_frame_with_the_destination_iframe_is_judged_by_the_record_too() {
    let policy = NetPolicy::new();
    let recorded = web_view(3);
    policy.note_main_frame(&top_level_navigation("https://top.example/", recorded).build());
    let frame = nested_navigation(
        "https://ads.example/frame",
        "https://top.example/",
        recorded,
    )
    .destination(Destination::IFrame);
    let found = sites(&policy, frame);
    assert!(!found.top_level_navigation);
    assert_eq!(found.party, Party::Third);
}

#[test]
fn a_worker_request_is_judged_by_the_record_of_its_web_view() {
    let policy = NetPolicy::new();
    let recorded = web_view(4);
    policy.note_main_frame(&top_level_navigation("https://top.example/", recorded).build());

    // A worker's requests have no client key and no initiator, but they have a web view id.
    let worker_request = |url: &str, web_view: WebViewId| {
        let origin = origin_of("https://top.example/");
        RequestBuilder::new(Some(web_view), claim(url), Referrer::NoReferrer)
            .destination(Destination::None)
            .origin(origin.clone())
            .client(client(origin, false, None))
    };
    assert_eq!(
        sites(&policy, worker_request("https://top.example/api", recorded)).party,
        Party::First
    );
    assert_eq!(
        sites(
            &policy,
            worker_request("https://tracker.example/api", recorded)
        )
        .party,
        Party::Third
    );
    assert_eq!(
        sites(
            &policy,
            worker_request("https://top.example/api", web_view(5))
        )
        .party,
        Party::Unknown
    );
}

#[test]
fn a_documents_own_key_wins_over_the_record() {
    let policy = NetPolicy::new();
    let recorded = web_view(6);
    // The web view is on `old.example` as far as the record knows...
    policy.note_main_frame(&top_level_navigation("https://old.example/", recorded).build());
    // ...but the document asking is in a page whose top level is `new.example`.
    let origin = origin_of("https://new.example/");
    let builder = RequestBuilder::new(
        Some(recorded),
        claim("https://new.example/api"),
        Referrer::NoReferrer,
    )
    .origin(origin.clone())
    .client(client(origin, false, Some(site_of("https://new.example/"))));
    let found = sites(&policy, builder);
    assert_eq!(found.party, Party::First);
    assert_eq!(found.top_level.as_deref(), Some("new.example"));
    // The origin is filled from the record only.
    assert_eq!(found.top_level_origin, None);
}

#[test]
fn the_record_follows_the_main_frame_of_each_web_view() {
    let policy = NetPolicy::new();
    let first = web_view(10);
    let second = web_view(11);
    policy.note_main_frame(&top_level_navigation("https://a.example/", first).build());
    policy.note_main_frame(&top_level_navigation("https://b.example/", second).build());
    assert_eq!(
        policy.recorded_top_level_site(first).as_deref(),
        Some("a.example")
    );
    assert_eq!(
        policy.recorded_top_level_site(second).as_deref(),
        Some("b.example")
    );

    // The web view navigates: the record follows, at every hop.
    policy.note_main_frame(&top_level_navigation("https://c.example/", first).build());
    assert_eq!(
        policy.recorded_top_level_site(first).as_deref(),
        Some("c.example")
    );
    let redirected = top_level_navigation("https://c.example/", first).url_list(vec![
        claim("https://c.example/"),
        claim("https://d.example/"),
    ]);
    policy.note_main_frame(&redirected.build());
    assert_eq!(
        policy.recorded_top_level_site(first).as_deref(),
        Some("d.example")
    );
    assert_eq!(
        policy.recorded_top_level_site(second).as_deref(),
        Some("b.example")
    );
    assert_eq!(policy.recorded_web_views(), 2);
}

#[test]
fn only_a_top_level_navigation_of_a_web_view_writes_the_record() {
    let policy = NetPolicy::new();
    let recorded = web_view(20);

    // A subresource, a nested navigation and a navigation without a web view write nothing.
    policy.note_main_frame(
        &subresource(
            "https://a.example/x",
            "https://a.example/",
            "https://a.example/",
        )
        .build(),
    );
    policy.note_main_frame(
        &nested_navigation("https://ads.example/", "https://a.example/", recorded).build(),
    );
    let without_web_view =
        RequestBuilder::new(None, claim("https://a.example/"), Referrer::NoReferrer)
            .destination(Destination::Document)
            .mode(RequestMode::Navigate)
            .origin(ImmutableOrigin::new_opaque())
            .client(client(ImmutableOrigin::new_opaque(), false, None));
    policy.note_main_frame(&without_web_view.build());
    assert_eq!(policy.recorded_web_views(), 0);
    assert_eq!(policy.recorded_top_level_site(recorded), None);
}

#[test]
fn the_record_is_bounded_and_the_least_recently_written_web_view_goes_first() {
    let policy = NetPolicy::new();
    for n in 1..=MAX_RECORDED_WEB_VIEWS as u32 {
        policy.note_main_frame(
            &top_level_navigation(&format!("https://site{n}.example/"), web_view(n)).build(),
        );
    }
    assert_eq!(policy.recorded_web_views(), MAX_RECORDED_WEB_VIEWS);
    assert_eq!(MAX_RECORDED_WEB_VIEWS, 256);

    // Web view 1 is the oldest. It navigates again, so it is now the newest and web view 2 is the oldest.
    policy.note_main_frame(&top_level_navigation("https://again.example/", web_view(1)).build());
    assert_eq!(policy.recorded_web_views(), MAX_RECORDED_WEB_VIEWS);

    // One more web view than the bound: the oldest, web view 2, is evicted.
    policy.note_main_frame(&top_level_navigation("https://extra.example/", web_view(1000)).build());
    assert_eq!(policy.recorded_web_views(), MAX_RECORDED_WEB_VIEWS);
    assert_eq!(policy.recorded_top_level_site(web_view(2)), None);
    assert_eq!(
        policy.recorded_top_level_site(web_view(1)).as_deref(),
        Some("again.example")
    );
    assert_eq!(
        policy.recorded_top_level_site(web_view(3)).as_deref(),
        Some("site3.example")
    );
    assert_eq!(
        policy.recorded_top_level_site(web_view(1000)).as_deref(),
        Some("extra.example")
    );

    // A flood of web views never grows the record past the bound.
    for n in 2000..2600 {
        policy
            .note_main_frame(&top_level_navigation("https://flood.example/", web_view(n)).build());
    }
    assert_eq!(policy.recorded_web_views(), MAX_RECORDED_WEB_VIEWS);
    // The evicted web view is unknown again, and unknown is third party.
    let nested = nested_navigation("https://a.example/", "https://site3.example/", web_view(3));
    assert_eq!(sites(&policy, nested).party, Party::Unknown);
}

#[test]
fn a_builder_and_the_request_built_from_it_have_the_same_sites() {
    let policy = NetPolicy::new();
    let recorded = web_view(30);
    policy.note_main_frame(&top_level_navigation("https://top.example/", recorded).build());

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
        top_level_navigation("https://a.example/", recorded),
        top_level_navigation("https://a.example/", recorded).url_list(vec![
            claim("https://a.example/"),
            claim("https://b.example/"),
        ]),
        nested_navigation("https://ads.example/", "https://top.example/", recorded),
        nested_navigation("https://ads.example/", "https://top.example/", web_view(31)),
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
fn the_sites_of_a_document_come_from_its_key_then_from_the_record() {
    let policy = NetPolicy::new();
    let recorded = web_view(40);
    policy.note_main_frame(&top_level_navigation("https://top.example/", recorded).build());

    let own_page = parse("https://top.example/page");
    let frame = parse("https://frame.example/page");
    let top_key = Some(site_of("https://top.example/"));

    let found = policy.sites_of_document(&own_page, &top_key, None);
    assert_eq!(found.party, Party::First);
    assert_eq!(found.top_level.as_deref(), Some("top.example"));
    assert!(!found.top_level_navigation);

    assert_eq!(
        policy.sites_of_document(&frame, &top_key, None).party,
        Party::Third
    );

    // No key: the record of the web view.
    let found = policy.sites_of_document(&frame, &None, Some(recorded));
    assert_eq!(found.party, Party::Third);
    assert_eq!(found.top_level.as_deref(), Some("top.example"));
    assert_eq!(
        policy
            .sites_of_document(&own_page, &None, Some(recorded))
            .party,
        Party::First
    );

    // No key and no record: unknown.
    assert_eq!(
        policy.sites_of_document(&own_page, &None, None).party,
        Party::Unknown
    );
    assert_eq!(
        policy
            .sites_of_document(&own_page, &None, Some(web_view(41)))
            .party,
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
