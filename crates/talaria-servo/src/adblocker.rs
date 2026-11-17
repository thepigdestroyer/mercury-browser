use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use adblock::Engine;
use adblock::request::Request;
use content_security_policy::Destination;
use log::{debug, info};
use servo::{WebResourceLoad, WebResourceRequest, WebResourceResponse, WebView};
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
            match engine.deserialize(&serialized) {
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

        if !self.should_block(load.request(), source_url.as_ref()) {
            return;
        }

        let blocked_url = load.request().url.as_url().clone();
        let blocked_count = self.blocked_requests.get() + 1;
        self.blocked_requests.set(blocked_count);
        if blocked_count == 1 {
            info!("blocked the first ad or tracker resource: {blocked_url}");
        } else {
            debug!("blocked web resource #{blocked_count}: {blocked_url}");
        }

        load.intercept(WebResourceResponse::new(blocked_url))
            .cancel();
    }

    fn should_block(&self, request: &WebResourceRequest, source_url: Option<&Url>) -> bool {
        self.should_block_url(
            request.url.as_str(),
            source_url.map_or(request.url.as_str(), Url::as_str),
            resource_type(request.destination),
            request.method.as_str(),
            request.is_for_main_frame,
        )
    }

    fn should_block_url(
        &self,
        url: &str,
        source_url: &str,
        resource_type: &str,
        method: &str,
        is_for_main_frame: bool,
    ) -> bool {
        // Brave can show a dedicated interstitial for blocked top-level navigations. Mercury does
        // not have that UI yet, so allowing the main document avoids turning a matched navigation
        // into an unexplained blank tab while still blocking its ads and trackers.
        if is_for_main_frame || !self.switches.content_blocking() {
            return false;
        }

        let Some(engine) = self.current_engine() else {
            self.unchecked_requests
                .set(self.unchecked_requests.get() + 1);
            return false;
        };
        let Ok(request) = Request::new(url, source_url, resource_type, method) else {
            debug!("could not classify web resource for ad blocking: {url}");
            return false;
        };

        engine.check_network_request(&request).should_block()
    }
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
            "ublock-filters.txt",
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
    fn servo_destinations_map_to_adblock_resource_types() {
        assert_eq!(resource_type(Destination::Image), "image");
        assert_eq!(resource_type(Destination::Script), "script");
        assert_eq!(resource_type(Destination::IFrame), "subdocument");
        assert_eq!(resource_type(Destination::Video), "media");
        assert_eq!(resource_type(Destination::None), "xmlhttprequest");
    }
}
