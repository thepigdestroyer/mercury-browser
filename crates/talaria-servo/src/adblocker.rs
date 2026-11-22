// SPDX-License-Identifier: MPL-2.0
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use adblock::Engine;
use adblock::request::Request;
use content_security_policy::Destination;
use log::{debug, info};
// net: `WebResourceRewrite`, the answer that changes a request's URL and lets it go on (EC37).
use servo::{
    WebResourceLoad, WebResourceRequest, WebResourceResponse, WebResourceRewrite, WebView,
};
use url::Url;

use crate::exit_clearing::EngineSwitches;

/// Where a compiled engine is handed to the content blocker from any thread. The blocker takes
/// it at its next decision, so loads that started earlier finish with the engine they had.
/// Only the newest engine offered is kept.
#[derive(Clone, Default)]
pub struct FilterEngineSlot {
    offered: Arc<Mutex<Option<Vec<u8>>>>,
}

impl FilterEngineSlot {
    /// Offers a serialized engine (adblock-rust =0.13.5, as `mercury-filter-lists` builds it).
    pub fn offer(&self, serialized: Vec<u8>) {
        *self
            .offered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(serialized);
    }

    fn take(&self) -> Option<Vec<u8>> {
        self.offered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

pub(crate) struct ContentBlocker {
    /// `None` until the filter lists have been loaded: until then nothing is blocked.
    engine: RefCell<Option<Rc<Engine>>>,
    incoming: FilterEngineSlot,
    blocked_requests: Cell<u64>,
    /// Requests let through because no engine was ready yet.
    unchecked_requests: Cell<u64>,
    /// Whether the user wants blocking; read at every load.
    switches: EngineSwitches,
}

impl ContentBlocker {
    pub(crate) fn new(switches: EngineSwitches) -> Self {
        info!("ad and tracker blocking is inactive until the filter lists are loaded");
        Self {
            engine: RefCell::new(None),
            incoming: FilterEngineSlot::default(),
            blocked_requests: Cell::new(0),
            unchecked_requests: Cell::new(0),
            switches,
        }
    }

    /// Where the filter-list service hands its engines.
    pub(crate) fn engine_slot(&self) -> FilterEngineSlot {
        self.incoming.clone()
    }

    /// The engine for the next decision: a newly offered one if there is one, else the
    /// current one. A decision holds its own handle, so a swap never changes it midway.
    fn current_engine(&self) -> Option<Rc<Engine>> {
        if let Some(serialized) = self.incoming.take() {
            let mut engine = Engine::default();
            let started = std::time::Instant::now();
            let loaded = engine.deserialize(&serialized);
            debug!(
                "filter-list engine of {} bytes deserialised in {:.2} ms",
                serialized.len(),
                started.elapsed().as_secs_f64() * 1000.0
            );
            match loaded {
                Ok(()) => {
                    let first = self.engine.borrow().is_none();
                    *self.engine.borrow_mut() = Some(Rc::new(engine));
                    if first {
                        info!(
                            "ad and tracker blocking active ({} requests were loaded before the filter lists were ready)",
                            self.unchecked_requests.get()
                        );
                    } else {
                        info!("ad and tracker blocking now uses the updated filter lists");
                    }
                }
                Err(error) => {
                    log::warn!(
                        "the filter-list engine could not be loaded ({error:?}); keeping the previous one"
                    );
                }
            }
        }
        self.engine.borrow().clone()
    }

    pub(crate) fn handle_web_resource_load(
        &self,
        web_view: Option<&WebView>,
        load: WebResourceLoad,
    ) {
        let source_url = load
            .request()
            .referrer_url
            .as_ref()
            .map(|url| url.as_url().clone())
            .or_else(|| web_view.and_then(WebView::url));

        match self.verdict(load.request(), source_url.as_ref()) {
            Verdict::Allow => return,
            // net: a `$removeparam` filter matched. The network thread applies the new URL (it keeps the
            // scheme and the origin, and refuses anything else) and goes on; nobody is asked again.
            Verdict::Rewrite(rewritten) => {
                debug!(
                    "removed query parameters from a web resource URL for {}",
                    rewritten.host_str().unwrap_or("(no host)")
                );
                load.rewrite(rewrite_to(rewritten));
                return;
            }
            Verdict::Block => {}
        }

        let blocked_url = load.request().url.as_url().clone();
        let blocked_count = self.blocked_requests.get() + 1;
        self.blocked_requests.set(blocked_count);
        if blocked_count == 1 {
            info!("blocked the first ad or tracker resource: {blocked_url}");
        } else {
            debug!("blocked web resource #{blocked_count}: {blocked_url}");
        }

        if is_document(load.request().destination) {
            // A cancelled document load (a frame by `src`, a frame navigated by script or a
            // link, which Servo fetches as `Document`) leaves the frame without a document, and
            // its page's `load` event never fires (tests/pages/adblock-blocked-frame.html: exit 3
            // before this); every blocked document gets an empty 403 instead. Images and scripts
            // end in a network error as before: their pages still finish loading.
            crate::openurl::answer_forbidden(load, blocked_url);
        } else {
            load.intercept(WebResourceResponse::new(blocked_url))
                .cancel();
        }
    }

    /// What to do with one request. `source_url` is the page the request is for, as the `$domain` option
    /// sees it (the referrer, else the web view's URL).
    fn verdict(&self, request: &WebResourceRequest, source_url: Option<&Url>) -> Verdict {
        self.verdict_for(
            request.url.as_str(),
            source_url.map_or(request.url.as_str(), Url::as_str),
            resource_type(request.destination),
            request.method.as_str(),
            request.is_for_main_frame,
            // net: the network thread's own answer to "is this request third party"; it knows the
            // top-level site of every request, which a referrer does not tell (EC38).
            request.is_third_party,
        )
    }

    #[cfg(test)]
    fn should_block_url(
        &self,
        url: &str,
        source_url: &str,
        resource_type: &str,
        method: &str,
        is_for_main_frame: bool,
    ) -> bool {
        self.verdict_for(
            url,
            source_url,
            resource_type,
            method,
            is_for_main_frame,
            None,
        ) == Verdict::Block
    }

    /// `third_party` is the network thread's verdict when it has one (`Some`), and then decides what the
    /// filters' `third-party` option means; with `None` the engine works it out from `url` and
    /// `source_url`, as it did before the network thread said.
    fn verdict_for(
        &self,
        url: &str,
        source_url: &str,
        resource_type: &str,
        method: &str,
        is_for_main_frame: bool,
        third_party: Option<bool>,
    ) -> Verdict {
        // Brave can show a dedicated interstitial for blocked top-level navigations. Mercury does
        // not have that UI yet, so allowing the main document avoids turning a matched navigation
        // into an unexplained blank tab while still blocking its ads and trackers. A main frame is not
        // rewritten either: changing the address of the page being opened is not decided yet.
        if is_for_main_frame || !self.switches.content_blocking() {
            return Verdict::Allow;
        }

        let Some(engine) = self.current_engine() else {
            self.unchecked_requests
                .set(self.unchecked_requests.get() + 1);
            return Verdict::Allow;
        };
        let Some(request) = adblock_request(url, source_url, resource_type, method, third_party)
        else {
            debug!("could not classify web resource for ad blocking: {url}");
            return Verdict::Allow;
        };

        let result = engine.check_network_request(&request);
        if result.should_block() {
            return Verdict::Block;
        }
        // net: `$removeparam` (only for a request that is not blocked, the engine says so): the same
        // request without the parameters the lists name. Only a URL that parses and really differs counts.
        match result
            .rewritten_url
            .as_deref()
            .and_then(|rewritten| Url::parse(rewritten).ok())
        {
            Some(rewritten) if rewritten.as_str() != url => Verdict::Rewrite(rewritten),
            _ => Verdict::Allow,
        }
    }
}

/// What the content blocker decides about one request.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Let it go on as it is.
    Allow,
    /// Answer it with nothing.
    Block,
    /// Let it go on, with this URL in place of its own (a `$removeparam` filter matched).
    Rewrite(Url),
}

/// The engine's view of a request. With the network thread's third-party flag (`Some`) the request is built
/// with `Request::preparsed`, so the flag, not the engine's own comparison of two host names, decides what
/// `third-party` means; without it, or for a URL with no host, the engine parses and decides as it always did.
fn adblock_request(
    url: &str,
    source_url: &str,
    resource_type: &str,
    method: &str,
    third_party: Option<bool>,
) -> Option<Request> {
    if let Some(third_party) = third_party
        && let Ok(parsed) = Url::parse(url)
        && let Some(hostname) = parsed.host_str()
    {
        let source_hostname = Url::parse(source_url)
            .ok()
            .and_then(|source| source.host_str().map(str::to_owned))
            .unwrap_or_default();
        return Some(Request::preparsed(
            url,
            hostname,
            &source_hostname,
            resource_type,
            third_party,
            method,
        ));
    }
    Request::new(url, source_url, resource_type, method).ok()
}

/// A rewrite of the request's URL and nothing else.
fn rewrite_to(url: Url) -> WebResourceRewrite {
    WebResourceRewrite {
        url: Some(url.into()),
        ..WebResourceRewrite::default()
    }
}

fn is_document(destination: Destination) -> bool {
    matches!(
        destination,
        Destination::Document | Destination::IFrame | Destination::Frame
    )
}

fn resource_type(destination: Destination) -> &'static str {
    match destination {
        Destination::Document => "document",
        Destination::Frame | Destination::IFrame => "subdocument",
        Destination::Font => "font",
        Destination::Image => "image",
        Destination::Audio | Destination::Track | Destination::Video => "media",
        Destination::Embed | Destination::Object => "object",
        Destination::Script
        | Destination::AudioWorklet
        | Destination::PaintWorklet
        | Destination::ServiceWorker
        | Destination::SharedWorker
        | Destination::Text
        | Destination::Worker => "script",
        Destination::Style => "stylesheet",
        Destination::Report => "csp_report",
        Destination::Json | Destination::None => "xmlhttprequest",
        Destination::Manifest | Destination::WebIdentity => "other",
        Destination::Xslt => "xslt",
    }
}

#[cfg(test)]
mod tests {
    use adblock::lists::{FilterSet, ParseOptions, RuleTypes};

    use super::*;

    /// An engine compiled from `rules`, as the filter-list service hands it over.
    fn compiled(rules: &[&str]) -> Vec<u8> {
        let mut filter_set = FilterSet::new(false);
        filter_set.add_filter_list(
            rules.join("\n"),
            ParseOptions {
                rule_types: RuleTypes::NetworkOnly,
                ..ParseOptions::default()
            },
        );
        Engine::new_with_filter_set(filter_set).serialize()
    }

    /// The bundled lists, compiled the way the filter-list service compiles them.
    fn bundled_engine() -> Vec<u8> {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/adblock");
        let mut filter_set = FilterSet::new(false);
        for file in [
            "easylist.txt",
            "easyprivacy.txt",
            "brave-firstparty.txt",
            "brave-specific.txt",
            "brave-unbreak.txt",
        ] {
            let text = std::fs::read_to_string(directory.join(file)).expect("a bundled list");
            filter_set.add_filter_list(
                text,
                ParseOptions {
                    rule_types: RuleTypes::NetworkOnly,
                    ..ParseOptions::default()
                },
            );
        }
        Engine::new_with_filter_set(filter_set).serialize()
    }

    fn blocker_with(engine: Vec<u8>, switches: EngineSwitches) -> ContentBlocker {
        let blocker = ContentBlocker::new(switches);
        blocker.engine_slot().offer(engine);
        blocker
    }

    fn blocks(blocker: &ContentBlocker, url: &str) -> bool {
        blocker.should_block_url(url, "https://example.com/article", "script", "GET", false)
    }

    #[test]
    fn nothing_is_blocked_until_an_engine_arrives() {
        let blocker = ContentBlocker::new(EngineSwitches::default());
        assert!(!blocks(&blocker, "https://ads.example/a.js"));
        assert_eq!(blocker.unchecked_requests.get(), 1);

        blocker.engine_slot().offer(compiled(&["||ads.example^"]));

        assert!(blocks(&blocker, "https://ads.example/a.js"));
    }

    #[test]
    fn a_new_engine_is_used_from_the_next_decision_and_an_old_one_finishes_its_own() {
        let blocker = blocker_with(compiled(&["||ads.example^"]), EngineSwitches::default());
        assert!(blocks(&blocker, "https://ads.example/a.js"));
        // A decision in progress holds the engine it started with.
        let in_flight = blocker.current_engine().expect("an engine");

        blocker
            .engine_slot()
            .offer(compiled(&["||trackers.example^"]));
        let request = Request::new(
            "https://ads.example/a.js",
            "https://example.com/",
            "script",
            "GET",
        )
        .unwrap();
        assert!(in_flight.check_network_request(&request).should_block());

        assert!(!blocks(&blocker, "https://ads.example/a.js"));
        assert!(blocks(&blocker, "https://trackers.example/t.js"));
    }

    #[test]
    fn an_engine_that_does_not_load_keeps_the_previous_one() {
        let blocker = blocker_with(compiled(&["||ads.example^"]), EngineSwitches::default());
        assert!(blocks(&blocker, "https://ads.example/a.js"));

        blocker.engine_slot().offer(b"damaged".to_vec());

        assert!(blocks(&blocker, "https://ads.example/a.js"));
    }

    #[test]
    fn only_the_newest_offered_engine_is_taken() {
        let blocker = ContentBlocker::new(EngineSwitches::default());
        blocker.engine_slot().offer(compiled(&["||ads.example^"]));
        blocker
            .engine_slot()
            .offer(compiled(&["||trackers.example^"]));

        assert!(!blocks(&blocker, "https://ads.example/a.js"));
        assert!(blocks(&blocker, "https://trackers.example/t.js"));
    }

    #[test]
    fn bundled_brave_filters_block_ads_and_trackers() {
        let blocker = blocker_with(bundled_engine(), EngineSwitches::default());

        assert!(blocker.should_block_url(
            "https://googleads.g.doubleclick.net/pagead/id",
            "https://example.com/article",
            "script",
            "GET",
            false,
        ));
        assert!(blocker.should_block_url(
            "https://www.google-analytics.com/analytics.js",
            "https://example.com/article",
            "script",
            "GET",
            false,
        ));
        assert!(!blocker.should_block_url(
            "https://example.com/assets/app.js",
            "https://example.com/article",
            "script",
            "GET",
            false,
        ));
    }

    #[test]
    fn a_switched_off_blocker_blocks_nothing_until_switched_on_again() {
        let switches = EngineSwitches::default();
        let blocker = blocker_with(compiled(&["||doubleclick.net^"]), switches.clone());
        let ad = |blocker: &ContentBlocker| {
            blocker.should_block_url(
                "https://googleads.g.doubleclick.net/pagead/id",
                "https://example.com/article",
                "script",
                "GET",
                false,
            )
        };

        switches.set_content_blocking(false);
        assert!(!ad(&blocker));

        switches.set_content_blocking(true);
        assert!(ad(&blocker));
    }

    #[test]
    fn top_level_navigation_is_left_to_the_browser() {
        let blocker = blocker_with(compiled(&["||doubleclick.net^"]), EngineSwitches::default());

        assert!(!blocker.should_block_url(
            "https://googleads.g.doubleclick.net/pagead/id",
            "https://example.com/",
            "document",
            "GET",
            true,
        ));
    }

    #[test]
    fn every_document_load_is_answered_instead_of_cancelled() {
        // Frames by `src`, frames navigated by script or a link (`Document`), and main frames.
        assert!(is_document(Destination::IFrame));
        assert!(is_document(Destination::Frame));
        assert!(is_document(Destination::Document));
        assert!(!is_document(Destination::Image));
        assert!(!is_document(Destination::Script));
        assert!(!is_document(Destination::Style));
    }

    #[test]
    fn servo_destinations_map_to_adblock_resource_types() {
        assert_eq!(resource_type(Destination::Image), "image");
        assert_eq!(resource_type(Destination::Script), "script");
        assert_eq!(resource_type(Destination::IFrame), "subdocument");
        assert_eq!(resource_type(Destination::Video), "media");
        assert_eq!(resource_type(Destination::None), "xmlhttprequest");
    }

    // net: the third-party flag of the network thread and `$removeparam` (EC38, EC37).

    fn verdict(
        blocker: &ContentBlocker,
        url: &str,
        source_url: &str,
        third_party: Option<bool>,
    ) -> Verdict {
        // `fetch` and XHR. adblock 0.13.5 applies a `$removeparam` rule that names no resource type to documents,
        // sub-documents and `xmlhttprequest` requests (`filters/network.rs`), so this is one type that gets
        // rewritten; `a_typeless_removeparam_rewrites_..._and_not_...` pins the others.
        blocker.verdict_for(url, source_url, "xmlhttprequest", "GET", false, third_party)
    }

    fn removeparam_blocker() -> ContentBlocker {
        blocker_with(
            compiled(&["||example.com^$removeparam=utm_source"]),
            EngineSwitches::default(),
        )
    }

    /// A `$removeparam` rule that names no resource type is for documents, sub-documents and `fetch`/XHR; an
    /// image, a script, a style sheet, a font or a media request is left alone (net-1 review, minor 7; the first
    /// version of this test used `script` and failed, which is how the engine's rule was found). Mercury never
    /// rewrites the page being opened (the main frame), but a frame that a script or a link navigated is a
    /// document that is not the main frame (D17), and a sub-document is a frame too.
    #[test]
    fn a_typeless_removeparam_rewrites_documents_frames_and_xhr_and_not_images_or_scripts() {
        let blocker = removeparam_blocker();
        let url = "https://example.com/page?utm_source=news&id=7";
        let verdict_of = |resource_type: &str, is_for_main_frame: bool| {
            blocker.verdict_for(
                url,
                "https://news.example/",
                resource_type,
                "GET",
                is_for_main_frame,
                None,
            )
        };
        let stripped = || Verdict::Rewrite(Url::parse("https://example.com/page?id=7").unwrap());

        for rewritten in ["xmlhttprequest", "subdocument", "document"] {
            assert_eq!(verdict_of(rewritten, false), stripped(), "{rewritten}");
        }
        for left_alone in [
            "image",
            "script",
            "stylesheet",
            "font",
            "media",
            "object",
            "other",
            "xslt",
        ] {
            assert_eq!(
                verdict_of(left_alone, false),
                Verdict::Allow,
                "{left_alone}"
            );
        }
        // The page being opened is never rewritten, whatever the rule says.
        assert_eq!(verdict_of("document", true), Verdict::Allow);
    }

    #[test]
    fn a_removeparam_filter_rewrites_the_url_without_the_parameter() {
        let blocker = removeparam_blocker();

        assert_eq!(
            verdict(
                &blocker,
                "https://example.com/page?utm_source=news&id=7",
                "https://news.example/",
                None,
            ),
            Verdict::Rewrite(Url::parse("https://example.com/page?id=7").unwrap())
        );
        // The same with the network thread's flag: the rewrite does not depend on it.
        assert_eq!(
            verdict(
                &blocker,
                "https://example.com/page?id=7&utm_source=news",
                "https://news.example/",
                Some(true),
            ),
            Verdict::Rewrite(Url::parse("https://example.com/page?id=7").unwrap())
        );
    }

    #[test]
    fn a_url_without_the_parameter_is_left_alone() {
        let blocker = removeparam_blocker();

        assert_eq!(
            verdict(
                &blocker,
                "https://example.com/page?id=7",
                "https://news.example/",
                None,
            ),
            Verdict::Allow
        );
        // Another site is not named by the rule.
        assert_eq!(
            verdict(
                &blocker,
                "https://other.example/page?utm_source=news",
                "https://news.example/",
                None,
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn a_request_that_is_blocked_is_blocked_not_rewritten() {
        let blocker = blocker_with(
            compiled(&[
                "||example.com^$removeparam=utm_source",
                "||example.com/ads/",
            ]),
            EngineSwitches::default(),
        );

        assert_eq!(
            verdict(
                &blocker,
                "https://example.com/ads/a.js?utm_source=news",
                "https://news.example/",
                None,
            ),
            Verdict::Block
        );
    }

    #[test]
    fn a_main_frame_and_a_switched_off_blocker_are_never_rewritten() {
        let blocker = removeparam_blocker();
        let url = "https://example.com/page?utm_source=news&id=7";

        assert_eq!(
            blocker.verdict_for(
                url,
                "https://news.example/",
                "document",
                "GET",
                true,
                Some(true)
            ),
            Verdict::Allow,
            "a main frame goes as it is"
        );

        blocker.switches.set_content_blocking(false);
        assert_eq!(
            verdict(&blocker, url, "https://news.example/", None),
            Verdict::Allow,
            "with blocking off nothing is rewritten"
        );
    }

    #[test]
    fn nothing_is_rewritten_until_an_engine_arrives() {
        let blocker = ContentBlocker::new(EngineSwitches::default());
        assert_eq!(
            verdict(
                &blocker,
                "https://example.com/page?utm_source=news",
                "https://news.example/",
                None,
            ),
            Verdict::Allow
        );
        assert_eq!(blocker.unchecked_requests.get(), 1);
    }

    fn third_party_only_blocker() -> ContentBlocker {
        blocker_with(
            compiled(&["||tracker.example^$third-party"]),
            EngineSwitches::default(),
        )
    }

    #[test]
    fn without_the_flag_the_engine_compares_the_two_hosts_as_before() {
        let blocker = third_party_only_blocker();

        // The page is another site: third party, blocked.
        assert_eq!(
            verdict(
                &blocker,
                "https://tracker.example/t.js",
                "https://news.example/",
                None,
            ),
            Verdict::Block
        );
        // The page is the tracker's own site: first party, not blocked.
        assert_eq!(
            verdict(
                &blocker,
                "https://tracker.example/t.js",
                "https://tracker.example/",
                None,
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn the_network_threads_flag_decides_what_third_party_means() {
        let blocker = third_party_only_blocker();

        // The referrer is another site (so the engine alone would say third party), but the network thread
        // knows the top-level page is the tracker's own site: first party, not blocked.
        assert_eq!(
            verdict(
                &blocker,
                "https://tracker.example/t.js",
                "https://news.example/",
                Some(false),
            ),
            Verdict::Allow
        );
        // The referrer is the tracker's own site (the engine alone would say first party), but the network
        // thread knows the top-level page is another site: third party, blocked.
        assert_eq!(
            verdict(
                &blocker,
                "https://tracker.example/t.js",
                "https://tracker.example/",
                Some(true),
            ),
            Verdict::Block
        );
    }

    #[test]
    fn the_domain_option_still_sees_the_page_the_request_is_for() {
        let blocker = blocker_with(
            compiled(&["||tracker.example^$domain=news.example"]),
            EngineSwitches::default(),
        );

        assert_eq!(
            verdict(
                &blocker,
                "https://tracker.example/t.js",
                "https://news.example/article",
                Some(true),
            ),
            Verdict::Block
        );
        assert_eq!(
            verdict(
                &blocker,
                "https://tracker.example/t.js",
                "https://other.example/article",
                Some(true),
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn a_url_with_no_host_goes_the_old_way_with_or_without_the_flag() {
        let blocker = third_party_only_blocker();

        for flag in [None, Some(true), Some(false)] {
            assert_eq!(
                verdict(
                    &blocker,
                    "data:text/plain,hello",
                    "https://news.example/",
                    flag,
                ),
                Verdict::Allow,
                "{flag:?}"
            );
        }
        // And an empty source has no host: the request is third party for the engine.
        assert_eq!(
            verdict(&blocker, "https://tracker.example/t.js", "", None),
            Verdict::Block
        );
        assert_eq!(
            verdict(&blocker, "https://tracker.example/t.js", "", Some(true)),
            Verdict::Block
        );
    }

    #[test]
    fn a_rewrite_carries_the_new_url_and_nothing_else() {
        let url = Url::parse("https://example.com/page?id=7").unwrap();
        let rewrite = rewrite_to(url.clone());

        assert_eq!(
            rewrite.url.as_ref().map(|rewritten| rewritten.as_str()),
            Some(url.as_str())
        );
        assert!(rewrite.set_headers.is_empty());
        assert!(rewrite.remove_headers.is_empty());
    }
}
