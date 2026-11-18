// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Links to schemes the engine cannot load (`mailto:`, `tel:`, an app's own scheme). The engine
//! has already been refused (`talaria-servo`'s `request_navigation`); here the link may be handed
//! to the system's handler for the scheme, but only after a user gesture in that tab and the
//! user's answer to a confirmation that names the page and the app. The confirmation is a page
//! request (`PageRequestKind::OpenExternally`) on the dialogs lane's sheet, so it waits for its
//! tab, goes when the page goes, and ignores input for its first half second. "Always allow"
//! lasts for the session and skips the confirmation, never the gesture.

use std::collections::{HashSet, VecDeque};

use talaria_core::{
    ExternalLinkRequest, NavigationVerdict, PageOrigin, PageRequest, PageRequestId,
    PageRequestKind, PendingPageRequest, navigation_verdict, remembered_origin,
};
use url::Url;

/// How many confirmations are remembered at once; the oldest beyond this (long since dropped
/// with their pages) are forgotten.
const MAX_ASKED: usize = 32;

/// Opens URLs in other apps. Tests and automated runs use doubles that only record.
pub(crate) trait ExternalOpener {
    /// The name of the app the system would open `url` with, or `None` if no app handles it.
    fn app_for(&self, url: &Url) -> Option<String>;
    /// Opens `url` in that app; false if the system refused.
    fn open(&self, url: &Url) -> bool;
}

impl ExternalOpener for Box<dyn ExternalOpener> {
    fn app_for(&self, url: &Url) -> Option<String> {
        (**self).app_for(url)
    }

    fn open(&self, url: &Url) -> bool {
        (**self).open(url)
    }
}

/// What became of a request or an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Opened,
    /// The system did not open it.
    OpenFailed,
    /// The user is to be asked with this page request.
    Ask(PageRequest),
    Refused(Refusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Not a URL for another app (a defence: only `OpenExternally` URLs are ever handed over).
    NotExternal,
    /// No click or key press in that tab just before.
    NoGesture,
    /// No app on this system handles the scheme.
    NoApp,
    /// The user said no.
    Declined,
}

/// A confirmation on its way to the user: the URL exactly as the page asked (the request shows
/// a sanitised copy), and the key "always allow" is remembered under.
struct Asked {
    id: PageRequestId,
    url: Url,
    origin: Option<String>,
}

/// The session's state: what was asked, and what is always allowed.
pub(crate) struct ExternalLinks<O> {
    opener: O,
    /// (page origin, scheme) pairs the user allowed for the session.
    always_allowed: HashSet<(String, String)>,
    asked: VecDeque<Asked>,
}

impl<O: ExternalOpener> ExternalLinks<O> {
    pub(crate) fn new(opener: O) -> Self {
        Self {
            opener,
            always_allowed: HashSet::new(),
            asked: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn opener(&self) -> &O {
        &self.opener
    }

    /// `had_gesture`: whether the tab had a user gesture just before (and it was consumed).
    pub(crate) fn request(&mut self, request: &ExternalLinkRequest, had_gesture: bool) -> Outcome {
        let Ok(url) = Url::parse(&request.url) else {
            return Outcome::Refused(Refusal::NotExternal);
        };
        let page = request
            .page_url
            .as_deref()
            .and_then(|page| Url::parse(page).ok());
        if navigation_verdict(page.as_ref(), &url) != NavigationVerdict::OpenExternally {
            return Outcome::Refused(Refusal::NotExternal);
        }
        if !had_gesture {
            return Outcome::Refused(Refusal::NoGesture);
        }
        let Some(app) = self.opener.app_for(&url) else {
            return Outcome::Refused(Refusal::NoApp);
        };
        let origin = remembered_origin(page.as_ref());
        let scheme = url.scheme().to_owned();
        if origin.as_ref().is_some_and(|origin| {
            self.always_allowed
                .contains(&(origin.clone(), scheme.clone()))
        }) {
            return self.open(&url);
        }
        let page_url = request.page_url.clone().unwrap_or_default();
        let always_label = origin.as_ref().map(|_| {
            format!(
                "Always allow {} to open {scheme} links",
                PageOrigin::from_url(&page_url).label()
            )
        });
        let ask = PageRequest {
            id: request.id,
            url: page_url,
            kind: PageRequestKind::OpenExternally {
                url: url.to_string(),
                scheme,
                app: Some(app),
                always_label,
            },
            received_at: request.received_at,
        };
        if self.asked.len() == MAX_ASKED {
            self.asked.pop_front();
        }
        self.asked.push_back(Asked {
            id: request.id,
            url,
            origin,
        });
        Outcome::Ask(ask)
    }

    /// The user answered a confirmation from the sheet: `accepted` is Open, `always` the
    /// checkbox. Returns `None` for other requests, and for a confirmation already answered.
    pub(crate) fn answered(
        &mut self,
        pending: &PendingPageRequest,
        accepted: bool,
        always: bool,
    ) -> Option<Outcome> {
        if !matches!(pending.kind(), PageRequestKind::OpenExternally { .. }) {
            return None;
        }
        let index = self
            .asked
            .iter()
            .position(|asked| asked.id == pending.id())?;
        let asked = self.asked.remove(index)?;
        if !accepted {
            return Some(Outcome::Refused(Refusal::Declined));
        }
        if always && let Some(origin) = asked.origin {
            self.always_allowed
                .insert((origin, asked.url.scheme().to_owned()));
        }
        Some(self.open(&asked.url))
    }

    fn open(&self, url: &Url) -> Outcome {
        if self.opener.open(url) {
            Outcome::Opened
        } else {
            Outcome::OpenFailed
        }
    }
}

/// How a request is named in the log: the page and the scheme, never the rest of the URL (a
/// `mailto:` address or a phone number is the user's business).
pub(crate) fn describe(tab: talaria_core::TabId, request: &ExternalLinkRequest) -> String {
    let scheme = request
        .url
        .split_once(':')
        .map_or("?", |(scheme, _)| scheme);
    format!(
        "tab {}: {} asked to open a {scheme}: link",
        tab.value(),
        PageOrigin::from_url(request.page_url.as_deref().unwrap_or_default()).label()
    )
}

/// The opener of automated runs and of tests: asks `names` which app would open a URL, and
/// records what it was asked to open; it never opens anything.
pub(crate) struct RecordingOpener<N> {
    names: N,
    opened: std::cell::RefCell<Vec<String>>,
}

impl<N: Fn(&Url) -> Option<String>> RecordingOpener<N> {
    pub(crate) fn new(names: N) -> Self {
        Self {
            names,
            opened: std::cell::RefCell::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn opened(&self) -> Vec<String> {
        self.opened.borrow().clone()
    }
}

impl<N: Fn(&Url) -> Option<String>> ExternalOpener for RecordingOpener<N> {
    fn app_for(&self, url: &Url) -> Option<String> {
        (self.names)(url)
    }

    fn open(&self, url: &Url) -> bool {
        log::info!(
            "openurl: recorded (not opened, automated run): a {}: link for {}",
            url.scheme(),
            self.app_for(url).unwrap_or_else(|| "no app".to_owned())
        );
        self.opened.borrow_mut().push(url.to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use talaria_core::{BrowserController, BrowserEvent, TabId};

    use super::*;
    use crate::activation::{consume_user_activation, note_user_activation};

    fn names(url: &Url) -> Option<String> {
        match url.scheme() {
            "mailto" => Some("Mail".to_owned()),
            "tel" => Some("FaceTime".to_owned()),
            _ => None,
        }
    }

    type Names = fn(&Url) -> Option<String>;

    fn links() -> ExternalLinks<RecordingOpener<Names>> {
        ExternalLinks::new(RecordingOpener::new(names as Names))
    }

    fn request(id: u64, url: &str, page: &str) -> ExternalLinkRequest {
        ExternalLinkRequest {
            id: PageRequestId::new(id),
            url: url.to_owned(),
            page_url: Some(page.to_owned()),
            received_at: Duration::ZERO,
        }
    }

    const PAGE: &str = "https://example.com/contact";

    /// Shows `ask` in a tab as the window would, and returns what the sheet shows.
    fn shown(ask: PageRequest) -> PendingPageRequest {
        let mut controller = BrowserController::default();
        let tab = controller.active_tab_id();
        controller.handle_event(tab, BrowserEvent::PageRequested(ask));
        controller
            .pending_page_request(tab)
            .expect("the sheet shows it")
    }

    fn ask(outcome: Outcome) -> PageRequest {
        match outcome {
            Outcome::Ask(ask) => ask,
            other => panic!("expected a confirmation, got {other:?}"),
        }
    }

    #[test]
    fn without_a_gesture_nothing_is_asked_or_opened() {
        let mut links = links();

        assert_eq!(
            links.request(&request(1, "mailto:a@example.com", PAGE), false),
            Outcome::Refused(Refusal::NoGesture)
        );
        assert!(links.opener().opened().is_empty());
    }

    #[test]
    fn the_gesture_gate_takes_one_gesture_per_request() {
        let mut links = links();
        let tab = TabId::new(31);
        let now = Instant::now();
        note_user_activation(tab, now);

        let first = consume_user_activation(tab, now);
        let second = consume_user_activation(tab, now);
        assert!(matches!(
            links.request(&request(1, "mailto:a@example.com", PAGE), first),
            Outcome::Ask(_)
        ));
        assert_eq!(
            links.request(&request(2, "tel:+15555550123", PAGE), second),
            Outcome::Refused(Refusal::NoGesture)
        );
    }

    #[test]
    fn a_gesture_in_another_tab_does_not_count() {
        let now = Instant::now();
        note_user_activation(TabId::new(42), now);

        assert!(!consume_user_activation(TabId::new(41), now));
    }

    #[test]
    fn the_confirmation_names_the_page_the_app_and_the_choice() {
        let mut links = links();

        let ask = ask(links.request(&request(5, "mailto:a@example.com", PAGE), true));

        assert_eq!(ask.id, PageRequestId::new(5));
        assert_eq!(ask.url, PAGE);
        assert_eq!(
            ask.kind,
            PageRequestKind::OpenExternally {
                url: "mailto:a@example.com".to_owned(),
                scheme: "mailto".to_owned(),
                app: Some("Mail".to_owned()),
                always_label: Some("Always allow example.com to open mailto links".to_owned()),
            }
        );
    }

    #[test]
    fn yes_opens_the_url_the_page_asked_for_once() {
        let mut links = links();
        let pending = shown(ask(
            links.request(&request(5, "mailto:a@example.com", PAGE), true)
        ));

        assert_eq!(links.answered(&pending, true, false), Some(Outcome::Opened));
        assert_eq!(links.opener().opened(), vec!["mailto:a@example.com"]);
        assert_eq!(links.answered(&pending, true, false), None, "answered once");
        assert_eq!(links.opener().opened().len(), 1);
    }

    #[test]
    fn the_original_url_is_opened_not_the_sanitised_copy_shown() {
        let mut links = links();
        let long = format!("mailto:a@example.com?body={}", "x".repeat(3_000));
        let pending = shown(ask(links.request(&request(5, &long, PAGE), true)));

        links.answered(&pending, true, false);

        assert_eq!(links.opener().opened(), vec![long]);
    }

    #[test]
    fn no_opens_nothing() {
        let mut links = links();
        let pending = shown(ask(
            links.request(&request(5, "mailto:a@example.com", PAGE), true)
        ));

        assert_eq!(
            links.answered(&pending, false, true),
            Some(Outcome::Refused(Refusal::Declined))
        );
        assert!(links.opener().opened().is_empty());
        assert!(matches!(
            links.request(&request(6, "mailto:b@example.com", PAGE), true),
            Outcome::Ask(_)
        ));
    }

    #[test]
    fn always_allow_skips_the_confirmation_for_that_origin_and_scheme_only() {
        let mut links = links();
        let pending = shown(ask(
            links.request(&request(5, "mailto:a@example.com", PAGE), true)
        ));
        links.answered(&pending, true, true);

        assert_eq!(
            links.request(
                &request(6, "mailto:b@example.com", "https://example.com/other"),
                true
            ),
            Outcome::Opened
        );
        assert_eq!(
            links.request(&request(7, "mailto:c@example.com", PAGE), false),
            Outcome::Refused(Refusal::NoGesture),
            "always allow never replaces the gesture"
        );
        assert!(
            matches!(
                links.request(&request(8, "tel:+15555550123", PAGE), true),
                Outcome::Ask(_)
            ),
            "another scheme still asks"
        );
        assert!(
            matches!(
                links.request(
                    &request(9, "mailto:d@example.com", "https://other.test/"),
                    true
                ),
                Outcome::Ask(_)
            ),
            "another origin still asks"
        );
        assert_eq!(
            links.opener().opened(),
            vec!["mailto:a@example.com", "mailto:b@example.com"]
        );
    }

    #[test]
    fn always_allow_is_not_offered_for_an_opaque_origin() {
        let mut links = links();
        let page = "file:///tmp/page.html";

        let ask = ask(links.request(&request(5, "mailto:a@example.com", page), true));
        let PageRequestKind::OpenExternally { always_label, .. } = &ask.kind else {
            panic!("not a confirmation");
        };
        assert_eq!(always_label, &None);
        let pending = shown(ask);
        links.answered(&pending, true, true);

        assert!(matches!(
            links.request(&request(6, "mailto:b@example.com", page), true),
            Outcome::Ask(_)
        ));
    }

    #[test]
    fn a_scheme_no_app_handles_is_refused_without_asking() {
        let mut links = links();

        assert_eq!(
            links.request(&request(1, "zoommtg://zoom.us/join", PAGE), true),
            Outcome::Refused(Refusal::NoApp)
        );
    }

    #[test]
    fn web_file_data_and_blocked_schemes_are_never_handed_to_another_app() {
        let mut links = links();
        for url in [
            "https://example.com/",
            "file:///etc/hosts",
            "data:,x",
            "smb://server/share",
            "x-apple.systempreferences:com.apple.preference.security",
            "not a url",
        ] {
            assert_eq!(
                links.request(&request(1, url, PAGE), true),
                Outcome::Refused(Refusal::NotExternal),
                "{url}"
            );
        }
        assert!(links.opener().opened().is_empty());
    }

    #[test]
    fn answers_to_other_requests_are_not_ours() {
        let mut links = links();
        let pending = shown(PageRequest {
            id: PageRequestId::new(5),
            url: PAGE.to_owned(),
            kind: PageRequestKind::Confirm {
                message: "Sure?".to_owned(),
            },
            received_at: Duration::ZERO,
        });

        assert_eq!(links.answered(&pending, true, true), None);
    }

    #[test]
    fn the_log_names_the_page_and_scheme_only() {
        assert_eq!(
            describe(
                TabId::new(3),
                &request(1, "mailto:secret@example.com", PAGE)
            ),
            "tab 3: example.com asked to open a mailto: link"
        );
    }
}
