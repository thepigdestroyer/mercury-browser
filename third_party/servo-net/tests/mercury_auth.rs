// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: tests for the sign-in prompts of `fetch` and XHR 401s (EC29); see mercury_auth.rs.

//! When a 401 answered to `fetch` or XHR may open a sign-in prompt (decision D12).
//!
//! The decision matrix runs through the real [`HttpState`] and the real [`NetPolicy`] (the policy finds the
//! top-level site from the request client's partition key, which the tests set by hand). The limiter is also
//! tested on its own with a clock the tests move, so the 60 seconds of silence take no time.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use net::mercury_auth::{
    AuthLimiter, Clock, MAX_ENTRIES, PromptDecision, PromptGuard, REFUSAL_SILENCE, prompt_finished,
    subresource_prompt_allowed,
};
use net::net_policy::NetPolicy;
use net_traits::blob_url_store::UrlWithBlobClaim;
use net_traits::policy_container::PolicyContainer;
use net_traits::pub_domains::Site;
use net_traits::request::{
    Destination, InsecureRequestsPolicy, Origin, Referrer, Request, RequestBuilder, RequestClient,
    RequestMode,
};
use servo_base::id::{BrowsingContextId, Index, TEST_NAMESPACE, TEST_WEBVIEW_ID, WebViewId};
use servo_url::{ImmutableOrigin, ServoUrl};

use crate::create_http_state;

// A clock that only moves when the test says so.
struct FakeClock {
    start: Instant,
    elapsed: StdMutex<Duration>,
}

impl FakeClock {
    fn new() -> Arc<FakeClock> {
        Arc::new(FakeClock {
            start: Instant::now(),
            elapsed: StdMutex::new(Duration::ZERO),
        })
    }

    fn advance(&self, by: Duration) {
        *self.elapsed.lock().unwrap() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.start + *self.elapsed.lock().unwrap()
    }
}

fn origin_of(url: &str) -> ImmutableOrigin {
    ServoUrl::parse(url).unwrap().origin()
}

/// Another web view than `TEST_WEBVIEW_ID`.
fn web_view(number: u32) -> WebViewId {
    WebViewId::mock_for_testing(BrowsingContextId {
        namespace_id: TEST_NAMESPACE,
        index: Index::new(number).unwrap(),
    })
}

/// The client of a document at `page` whose top-level page is `top_level`.
fn client(top_level: &str, page: &str) -> RequestClient {
    RequestClient {
        preloaded_resources: Default::default(),
        policy_container: Arc::new(PolicyContainer::default()),
        origin: Origin::Origin(origin_of(page)),
        is_nested_browsing_context: top_level != page,
        insecure_requests_policy: InsecureRequestsPolicy::DoNotUpgrade,
        has_trustworthy_ancestor_origin: false,
        network_partition_key: Some(Site::from_origin(&origin_of(top_level))),
        browser_initiated_navigation: false,
    }
}

/// A `fetch` that the document at `page` (in a page whose top level is `top_level`) makes to `url`.
fn fetch_request(url: &str, page: &str, top_level: &str) -> Request {
    RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::from_url_without_having_claimed_blob(ServoUrl::parse(url).unwrap()),
        Referrer::NoReferrer,
    )
    .origin(origin_of(page))
    .mode(RequestMode::CorsMode)
    .destination(Destination::None)
    .client(client(top_level, page))
    .build()
}

/// The ordinary case: `https://app.example` fetches its own `/api` as the top-level page.
fn own_fetch() -> Request {
    fetch_request(
        "https://app.example/api",
        "https://app.example",
        "https://app.example",
    )
}

#[test]
fn a_fetch_to_the_pages_own_origin_may_prompt() {
    let state = create_http_state(None);
    assert!(subresource_prompt_allowed(&state, &own_fetch()));
}

#[test]
fn a_page_of_the_top_level_site_on_another_subdomain_prompts_for_its_own_origin() {
    // `www.app.example` embedded in a page of `app.example`: one site, and the fetch goes to the frame's own
    // origin.
    let state = create_http_state(None);
    let request = fetch_request(
        "https://www.app.example/api",
        "https://www.app.example",
        "https://app.example",
    );
    assert!(subresource_prompt_allowed(&state, &request));
}

#[test]
fn a_navigation_prompts_as_it_always_did_and_does_not_use_the_limiter() {
    let state = create_http_state(None);
    let request = RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::from_url_without_having_claimed_blob(
            ServoUrl::parse("https://app.example/private").unwrap(),
        ),
        Referrer::NoReferrer,
    )
    .mode(RequestMode::Navigate)
    .destination(Destination::Document)
    .build();
    for _ in 0..3 {
        assert!(subresource_prompt_allowed(&state, &request));
    }
    prompt_finished(&state, &request, false);
    assert!(subresource_prompt_allowed(&state, &request));
    assert!(state.net_policy.auth_limiter().is_empty());
}

#[test]
fn a_request_without_a_web_view_never_prompts() {
    let state = create_http_state(None);
    let mut request = own_fetch();
    request.target_webview_id = None;
    assert!(!subresource_prompt_allowed(&state, &request));
    assert!(state.net_policy.auth_limiter().is_empty());
}

#[test]
fn only_a_fetch_or_xhr_prompts_not_a_resource_or_a_frame() {
    let state = create_http_state(None);
    for destination in [
        Destination::Audio,
        Destination::Document,
        Destination::Font,
        Destination::Frame,
        Destination::IFrame,
        Destination::Image,
        Destination::Json,
        Destination::Script,
        Destination::ServiceWorker,
        Destination::SharedWorker,
        Destination::Style,
        Destination::Track,
        Destination::Video,
        Destination::Worker,
    ] {
        let mut request = own_fetch();
        request.destination = destination;
        assert!(
            !subresource_prompt_allowed(&state, &request),
            "{destination:?} must not prompt"
        );
    }
    assert!(state.net_policy.auth_limiter().is_empty());
}

#[test]
fn a_page_cannot_prompt_for_somebody_elses_origin() {
    let state = create_http_state(None);
    // Another site; another subdomain of the same site; another scheme; another port.
    for url in [
        "https://other.example/api",
        "https://api.app.example/api",
        "http://app.example/api",
        "https://app.example:8443/api",
    ] {
        let request = fetch_request(url, "https://app.example", "https://app.example");
        assert!(
            !subresource_prompt_allowed(&state, &request),
            "a fetch from app.example to {url} must not prompt"
        );
    }
    assert!(state.net_policy.auth_limiter().is_empty());
}

#[test]
fn a_request_whose_origin_is_still_the_client_or_opaque_never_prompts() {
    let state = create_http_state(None);

    let mut request = own_fetch();
    request.origin = Origin::Client;
    assert!(!subresource_prompt_allowed(&state, &request));

    let mut request = own_fetch();
    request.origin = Origin::Origin(ImmutableOrigin::new_opaque());
    assert!(!subresource_prompt_allowed(&state, &request));
    assert!(state.net_policy.auth_limiter().is_empty());
}

#[test]
fn an_embedded_frame_of_another_site_cannot_prompt_over_the_page() {
    let state = create_http_state(None);
    // The frame is on tracker.example, inside a page of app.example, and fetches its own origin.
    let request = fetch_request(
        "https://tracker.example/api",
        "https://tracker.example",
        "https://app.example",
    );
    assert!(!subresource_prompt_allowed(&state, &request));
    assert!(state.net_policy.auth_limiter().is_empty());
}

#[test]
fn a_request_whose_top_level_site_is_unknown_never_prompts() {
    let state = create_http_state(None);
    // A worker's request: no partition key, and the policy has no record of the web view.
    let mut request = own_fetch();
    request.client = None;
    assert!(!subresource_prompt_allowed(&state, &request));
    assert!(state.net_policy.auth_limiter().is_empty());
}

#[test]
fn one_prompt_is_open_at_a_time_per_web_view_and_origin() {
    let state = create_http_state(None);
    let request = own_fetch();
    assert!(subresource_prompt_allowed(&state, &request));
    // The same page asking again while the prompt is open.
    assert!(!subresource_prompt_allowed(&state, &request));
    assert!(!subresource_prompt_allowed(&state, &request));

    // Another web view, and another origin of the same site, are other pairs.
    let mut other_view = own_fetch();
    other_view.target_webview_id = Some(web_view(4321));
    assert!(subresource_prompt_allowed(&state, &other_view));
    let other_origin = fetch_request(
        "https://www.app.example/api",
        "https://www.app.example",
        "https://app.example",
    );
    assert!(subresource_prompt_allowed(&state, &other_origin));
    assert_eq!(state.net_policy.auth_limiter().len(), 3);
}

#[test]
fn answering_the_prompt_clears_the_record() {
    let state = create_http_state(None);
    let request = own_fetch();
    assert!(subresource_prompt_allowed(&state, &request));
    prompt_finished(&state, &request, true);
    assert!(state.net_policy.auth_limiter().is_empty());
    assert!(subresource_prompt_allowed(&state, &request));
}

#[test]
fn refusing_the_prompt_silences_the_pair() {
    let state = create_http_state(None);
    let request = own_fetch();
    assert!(subresource_prompt_allowed(&state, &request));
    prompt_finished(&state, &request, false);
    // Within the silence the 401 goes back to the page.
    assert!(!subresource_prompt_allowed(&state, &request));
    // Another web view is not silenced.
    let mut other_view = own_fetch();
    other_view.target_webview_id = Some(web_view(777));
    assert!(subresource_prompt_allowed(&state, &other_view));
}

#[test]
fn finishing_a_navigation_or_an_unrelated_request_leaves_an_open_prompt_alone() {
    let state = create_http_state(None);
    let request = own_fetch();
    assert!(subresource_prompt_allowed(&state, &request));

    // A navigation to the same origin that was answered (or not) is not this record's business.
    let navigation = RequestBuilder::new(
        Some(TEST_WEBVIEW_ID),
        UrlWithBlobClaim::from_url_without_having_claimed_blob(
            ServoUrl::parse("https://app.example/private").unwrap(),
        ),
        Referrer::NoReferrer,
    )
    .mode(RequestMode::Navigate)
    .destination(Destination::Document)
    .build();
    prompt_finished(&state, &navigation, true);
    prompt_finished(&state, &navigation, false);

    // A request for an image of the same origin neither.
    let mut image = own_fetch();
    image.destination = Destination::Image;
    prompt_finished(&state, &image, true);

    assert!(
        !subresource_prompt_allowed(&state, &request),
        "the prompt for the fetch is still open"
    );
}

#[test]
fn finishing_a_prompt_that_was_never_opened_changes_nothing() {
    let state = create_http_state(None);
    prompt_finished(&state, &own_fetch(), false);
    assert!(state.net_policy.auth_limiter().is_empty());
    // It was not silenced either.
    assert!(subresource_prompt_allowed(&state, &own_fetch()));
}

#[test]
fn the_limiter_is_the_policys_and_clones_of_the_policy_share_it() {
    let policy = NetPolicy::new();
    let copy = policy.clone();
    assert_eq!(
        policy
            .auth_limiter()
            .begin(TEST_WEBVIEW_ID, "https://a.example"),
        PromptDecision::Allowed
    );
    assert_eq!(
        copy.auth_limiter()
            .begin(TEST_WEBVIEW_ID, "https://a.example"),
        PromptDecision::InFlight
    );
}

// The limiter on its own, with a clock the tests move.

const ORIGIN_A: &str = "https://a.example";
const ORIGIN_B: &str = "https://b.example";
const ORIGIN_C: &str = "https://c.example";

#[test]
fn the_default_limiter_has_the_documented_bounds() {
    assert_eq!(MAX_ENTRIES, 256);
    assert_eq!(REFUSAL_SILENCE, Duration::from_secs(60));
    let limiter = AuthLimiter::default();
    assert!(limiter.is_empty());
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
}

#[test]
fn a_refusal_is_silent_for_sixty_seconds_and_not_a_moment_longer() {
    let clock = FakeClock::new();
    let limiter = AuthLimiter::with_clock(clock.clone());
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_A, false);

    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Silenced
    );
    clock.advance(Duration::from_secs(59));
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Silenced
    );
    clock.advance(Duration::from_millis(999));
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Silenced
    );
    clock.advance(Duration::from_millis(1));
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
}

#[test]
fn a_refusal_while_silenced_does_not_extend_the_silence() {
    // `finish` only acts on a prompt that is in flight, so the asks that were refused in the meantime
    // (and their `finish` calls, if a caller made them) cannot push the end of the silence away.
    let clock = FakeClock::new();
    let limiter = AuthLimiter::with_clock(clock.clone());
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_A, false);
    clock.advance(Duration::from_secs(30));
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Silenced
    );
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_A, false);
    clock.advance(Duration::from_secs(30));
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
}

#[test]
fn an_answer_clears_the_record_at_once() {
    let clock = FakeClock::new();
    let limiter = AuthLimiter::with_clock(clock);
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::InFlight
    );
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_A, true);
    assert!(limiter.is_empty());
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
}

#[test]
fn an_open_prompt_stays_open_however_long_it_takes() {
    let clock = FakeClock::new();
    let limiter = AuthLimiter::with_clock(clock.clone());
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
    clock.advance(Duration::from_secs(3600));
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::InFlight
    );
}

#[test]
fn the_pair_is_web_view_and_origin() {
    let limiter = AuthLimiter::with_clock(FakeClock::new());
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_B),
        PromptDecision::Allowed
    );
    assert_eq!(
        limiter.begin(web_view(2), ORIGIN_A),
        PromptDecision::Allowed
    );
    assert_eq!(limiter.len(), 3);
    // A refusal of one pair leaves the others as they are.
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_A, false);
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Silenced
    );
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_B),
        PromptDecision::InFlight
    );
    assert_eq!(
        limiter.begin(web_view(2), ORIGIN_A),
        PromptDecision::InFlight
    );
}

#[test]
fn finishing_a_pair_that_is_not_open_does_nothing() {
    let clock = FakeClock::new();
    let limiter = AuthLimiter::with_clock(clock);
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_A, false);
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_A, true);
    assert!(limiter.is_empty());
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
}

#[test]
fn the_map_is_bounded_and_every_slot_in_flight_means_no_new_prompt() {
    let limiter = AuthLimiter::with_parts(FakeClock::new(), 3, REFUSAL_SILENCE);
    assert_eq!(
        limiter.begin(web_view(1), ORIGIN_A),
        PromptDecision::Allowed
    );
    assert_eq!(
        limiter.begin(web_view(2), ORIGIN_A),
        PromptDecision::Allowed
    );
    assert_eq!(
        limiter.begin(web_view(3), ORIGIN_A),
        PromptDecision::Allowed
    );
    // Nothing may be forgotten: forgetting an open prompt would allow a second one for its pair.
    assert_eq!(limiter.begin(web_view(4), ORIGIN_A), PromptDecision::Full);
    assert_eq!(limiter.len(), 3);
    assert_eq!(
        limiter.begin(web_view(1), ORIGIN_A),
        PromptDecision::InFlight
    );

    // One prompt is over: its slot can hold the next ask.
    limiter.finish(web_view(2), ORIGIN_A, true);
    assert_eq!(
        limiter.begin(web_view(4), ORIGIN_A),
        PromptDecision::Allowed
    );
    assert_eq!(limiter.len(), 3);
}

#[test]
fn a_bound_of_zero_refuses_everything() {
    let limiter = AuthLimiter::with_parts(FakeClock::new(), 0, REFUSAL_SILENCE);
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Full
    );
    assert!(limiter.is_empty());
}

#[test]
fn a_full_map_forgets_the_oldest_silence_first() {
    let clock = FakeClock::new();
    let limiter = AuthLimiter::with_parts(clock.clone(), 2, REFUSAL_SILENCE);
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_A, false);
    clock.advance(Duration::from_secs(1));
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_B),
        PromptDecision::Allowed
    );
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_B, false);
    assert_eq!(limiter.len(), 2);

    // A third pair needs a slot: the silence of A (the older refusal) goes, B's stays.
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_C),
        PromptDecision::Allowed
    );
    assert_eq!(limiter.len(), 2);
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_B),
        PromptDecision::Silenced
    );
}

#[test]
fn a_full_map_drops_the_silences_that_have_run_out_before_anything_else() {
    let clock = FakeClock::new();
    let limiter = AuthLimiter::with_parts(clock.clone(), 2, REFUSAL_SILENCE);
    // A is refused first and runs out first; B is refused later.
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_A),
        PromptDecision::Allowed
    );
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_A, false);
    clock.advance(Duration::from_secs(40));
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_B),
        PromptDecision::Allowed
    );
    limiter.finish(TEST_WEBVIEW_ID, ORIGIN_B, false);
    clock.advance(Duration::from_secs(30));

    // A's silence is over (70 s), B's is not (30 s): a new pair takes A's slot and B stays silenced.
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_C),
        PromptDecision::Allowed
    );
    assert_eq!(
        limiter.begin(TEST_WEBVIEW_ID, ORIGIN_B),
        PromptDecision::Silenced
    );
    assert_eq!(limiter.len(), 2);
}

#[test]
fn the_default_bound_holds_under_many_pairs() {
    let clock = FakeClock::new();
    let limiter = AuthLimiter::with_clock(clock.clone());
    for number in 1..=(MAX_ENTRIES as u32 * 2) {
        let view = web_view(number);
        assert_eq!(
            limiter.begin(view, ORIGIN_A),
            PromptDecision::Allowed,
            "pair {number}"
        );
        limiter.finish(view, ORIGIN_A, false);
        clock.advance(Duration::from_millis(10));
        assert!(
            limiter.len() <= MAX_ENTRIES,
            "pair {number}: {} entries",
            limiter.len()
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The guard of a prompt in flight (net-1 review, minor 5)
// ---------------------------------------------------------------------------------------------

#[test]
fn a_guard_dropped_without_an_answer_silences_the_pair_instead_of_leaving_it_in_flight() {
    let state = create_http_state(None);
    assert!(subresource_prompt_allowed(&state, &own_fetch()));
    let guard = PromptGuard::of(&state, &own_fetch()).expect("a fetch to its own origin");
    // While the prompt is open a second one is refused.
    assert!(!subresource_prompt_allowed(&state, &own_fetch()));
    assert_eq!(
        state
            .net_policy
            .auth_limiter()
            .begin(TEST_WEBVIEW_ID, "https://app.example"),
        PromptDecision::InFlight
    );

    // The fetch future is dropped (an aborted `fetch`, a navigation away): the mark does not stay for good.
    drop(guard);
    assert_eq!(
        state
            .net_policy
            .auth_limiter()
            .begin(TEST_WEBVIEW_ID, "https://app.example"),
        PromptDecision::Silenced,
        "a refusal: silenced for the refusal period, not stuck in flight"
    );
}

#[test]
fn a_guard_that_is_finished_with_an_answer_forgets_the_pair() {
    let state = create_http_state(None);
    assert!(subresource_prompt_allowed(&state, &own_fetch()));
    PromptGuard::of(&state, &own_fetch())
        .expect("a fetch to its own origin")
        .finish(true);
    assert!(state.net_policy.auth_limiter().is_empty());
    assert!(subresource_prompt_allowed(&state, &own_fetch()));
}

#[test]
fn a_guard_that_is_finished_without_an_answer_silences_the_pair() {
    let state = create_http_state(None);
    assert!(subresource_prompt_allowed(&state, &own_fetch()));
    PromptGuard::of(&state, &own_fetch())
        .expect("a fetch to its own origin")
        .finish(false);
    assert!(!subresource_prompt_allowed(&state, &own_fetch()));
}

#[test]
fn there_is_no_guard_for_a_navigation_or_a_request_that_could_not_prompt() {
    let state = create_http_state(None);
    let other_origin = fetch_request(
        "https://other.example/api",
        "https://app.example",
        "https://app.example",
    );
    assert!(PromptGuard::of(&state, &other_origin).is_none());
}
