// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Site security in a tab's delegate: HTTPS-only upgrades of top-level loads, and recognising
//! Mercury's error pages when a page finishes loading.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use mercury_site_security::{ERROR_PAGE_PROBE_SCRIPT, ErrorPage, parse_error_page_marker};
use servo::{JSValue, WebResourceLoad, WebResourceResponse};
use talaria_core::{BrowserEvent, ErrorPageKind, HttpsPolicy, HttpsUpgradeChain, TabEvent, TabId};

use crate::resources::error_page_secret;

/// The policy the browser set, shared by every tab's delegate.
pub(crate) type SharedHttpsPolicy = Rc<RefCell<HttpsPolicy>>;

/// One tab's part.
pub(crate) struct TabHttpsUpgrades {
    policy: SharedHttpsPolicy,
    /// Upgrades since the tab's last finished load (loop protection; see `HttpsUpgradeChain`).
    chain: RefCell<HttpsUpgradeChain>,
}

impl TabHttpsUpgrades {
    pub(crate) fn new(policy: SharedHttpsPolicy) -> Self {
        Self {
            policy,
            chain: RefCell::default(),
        }
    }

    /// A load finished in the tab; the next upgrade starts a new chain.
    pub(crate) fn load_finished(&self) {
        self.chain.borrow_mut().load_finished();
    }

    /// Applies HTTPS-only mode to a load. Returns the load if it goes ahead unchanged; a load
    /// that is upgraded or refused is cancelled here, and the event says what happened; the
    /// controller then loads the HTTPS address (`BrowserEvent::HttpsUpgraded`).
    pub(crate) fn intercept(
        &self,
        load: WebResourceLoad,
        send: impl Fn(BrowserEvent),
    ) -> Option<WebResourceLoad> {
        let request = load.request();
        let url = request.url.as_url().clone();
        let decision = self.chain.borrow_mut().decide(
            &self.policy.borrow(),
            &url,
            request.method.as_str() == "GET",
            request.is_for_main_frame,
        );
        let Some(event) = HttpsUpgradeChain::event_for(&url, &decision) else {
            return Some(load);
        };
        load.intercept(WebResourceResponse::new(url.clone()))
            .cancel();
        log::info!("HTTPS-only mode: {decision:?} for {url}");
        send(event);
        None
    }
}

/// Asks the finished page whether it is one of Mercury's error pages, and reports it if so.
pub(crate) fn probe_error_page(web_view: &servo::WebView, tab_id: TabId, sender: Sender<TabEvent>) {
    let Some(url) = web_view.url() else {
        return;
    };
    web_view.evaluate_javascript(ERROR_PAGE_PROBE_SCRIPT, move |result| {
        let Ok(JSValue::String(marker)) = result else {
            return;
        };
        let kind = match parse_error_page_marker(&marker, error_page_secret()) {
            Some(ErrorPage::Network) => ErrorPageKind::Network,
            Some(ErrorPage::Certificate) => ErrorPageKind::Certificate,
            None => return,
        };
        let _send_result = sender.send(TabEvent::new(
            tab_id,
            BrowserEvent::ErrorPageShown {
                kind,
                url: url.to_string(),
            },
        ));
    });
}
