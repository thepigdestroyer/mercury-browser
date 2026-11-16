//! Links to schemes the engine cannot load (`mailto:`, `tel:`, an app's own scheme). The engine
//! has already been refused (`talaria-servo`'s `request_navigation`); here the link may be handed
//! to the system's handler for the scheme, but only after a user gesture in that tab and the
//! user's confirmation, which names the page and the app. "Always allow" lasts for the session
//! and skips the confirmation, never the gesture.

use std::collections::{HashMap, HashSet};

use talaria_core::{
    ExternalLinkRequest, ExternalOpenPrompt, NavigationVerdict, TabId, external_open_prompt,
    navigation_verdict, page_display_name, remembered_origin,
};
use url::Url;

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

/// The user's answer to the confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptAnswer {
    Open { always: bool },
    DontOpen,
}

/// What became of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Opened,
    /// The system did not open it.
    OpenFailed,
    /// Waiting for the user's answer (shown when the tab is active).
    Asking,
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
    /// The tab already has a confirmation waiting.
    AlreadyAsking,
    /// The user said no.
    Declined,
    /// The tab closed or moved to another page before the user answered.
    Stale,
}

struct Waiting {
    url: Url,
    page: Option<Url>,
    /// The page's address as the tab shows it, to tell whether the tab has moved on.
    page_address: String,
    app: String,
}

/// The session's state: what is waiting for an answer, and what is always allowed.
pub(crate) struct ExternalLinks<O> {
    opener: O,
    /// (page origin, scheme) pairs the user allowed for the session.
    always_allowed: HashSet<(String, String)>,
    waiting: HashMap<TabId, Waiting>,
}

impl<O: ExternalOpener> ExternalLinks<O> {
    pub(crate) fn new(opener: O) -> Self {
        Self {
            opener,
            always_allowed: HashSet::new(),
            waiting: HashMap::new(),
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
        if let Some(origin) = remembered_origin(page.as_ref())
            && self
                .always_allowed
                .contains(&(origin, url.scheme().to_owned()))
        {
            return self.open(&url);
        }
        if self.waiting.contains_key(&request.tab_id) {
            return Outcome::Refused(Refusal::AlreadyAsking);
        }
        self.waiting.insert(
            request.tab_id,
            Waiting {
                url,
                page,
                page_address: shown_address(request.page_url.as_deref()),
                app,
            },
        );
        Outcome::Asking
    }

    /// The confirmation waiting for `tab`, if the tab still shows the page that asked.
    /// `current_address` is the address the tab shows now (`None` if the tab has closed); a
    /// confirmation whose page has gone is dropped and returned as `Err(Stale)`.
    pub(crate) fn prompt_for(
        &mut self,
        tab: TabId,
        current_address: Option<&str>,
    ) -> Option<Result<ExternalOpenPrompt, Refusal>> {
        let waiting = self.waiting.get(&tab)?;
        if current_address != Some(waiting.page_address.as_str()) {
            self.waiting.remove(&tab);
            return Some(Err(Refusal::Stale));
        }
        Some(Ok(external_open_prompt(
            waiting.page.as_ref(),
            &waiting.url,
            Some(&waiting.app),
        )))
    }

    /// Applies the user's answer to `tab`'s confirmation. Each confirmation is answered once.
    pub(crate) fn answer(&mut self, tab: TabId, answer: PromptAnswer) -> Option<Outcome> {
        let waiting = self.waiting.remove(&tab)?;
        Some(match answer {
            PromptAnswer::DontOpen => Outcome::Refused(Refusal::Declined),
            PromptAnswer::Open { always } => {
                if always && let Some(origin) = remembered_origin(waiting.page.as_ref()) {
                    self.always_allowed
                        .insert((origin, waiting.url.scheme().to_owned()));
                }
                self.open(&waiting.url)
            }
        })
    }

    /// Drops the confirmations of tabs that no longer exist.
    pub(crate) fn forget_closed_tabs(&mut self, is_open: impl Fn(TabId) -> bool) {
        self.waiting.retain(|tab, _| is_open(*tab));
    }

    fn open(&self, url: &Url) -> Outcome {
        if self.opener.open(url) {
            Outcome::Opened
        } else {
            Outcome::OpenFailed
        }
    }
}

/// The address a tab shows for a page: the engine's blank page shows as empty.
fn shown_address(page_url: Option<&str>) -> String {
    match page_url {
        None | Some("about:blank") => String::new(),
        Some(page_url) => page_url.to_owned(),
    }
}

/// How a request is named in the log: the page and the scheme, never the rest of the URL (a
/// `mailto:` address or a phone number is the user's business).
pub(crate) fn describe(request: &ExternalLinkRequest) -> String {
    let page = request
        .page_url
        .as_deref()
        .and_then(|page| Url::parse(page).ok());
    let scheme = request
        .url
        .split_once(':')
        .map_or("?", |(scheme, _)| scheme);
    format!(
        "tab {}: {} asked to open a {scheme}: link",
        request.tab_id.value(),
        page_display_name(page.as_ref())
    )
}

/// The confirmation seam. The dialogs lane's sheet is not merged yet, so until it is, the
/// confirmation is only written to the log, and answered here: a normal run answers "Don't
/// open" (nothing may open without the user's confirmation), an automated run answers "Open"
/// (its opener only records; see `RecordingOpener`), so that the whole path can be checked.
/// Replace this function with the sheet at merge; it is the only place that answers.
pub(crate) fn confirm_external_open(prompt: &ExternalOpenPrompt, automated: bool) -> PromptAnswer {
    log::info!(
        "openurl: confirmation \"{}\" ({}) [{}] [Cancel]{}",
        prompt.message,
        prompt
            .detail
            .split_once(':')
            .map_or("", |(scheme, _)| scheme),
        prompt.open_label,
        prompt
            .always_label
            .as_ref()
            .map_or_else(String::new, |label| format!(" [ ] {label}"))
    );
    if automated {
        log::info!(
            "openurl: automated run: answering \"{}\"",
            prompt.open_label
        );
        PromptAnswer::Open { always: false }
    } else {
        log::warn!(
            "openurl: no confirmation sheet yet (dialogs lane); answering \"Cancel\", nothing opens"
        );
        PromptAnswer::DontOpen
    }
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
    use std::time::Instant;

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

    fn request(tab: u64, url: &str, page: &str) -> ExternalLinkRequest {
        ExternalLinkRequest {
            tab_id: TabId::new(tab),
            url: url.to_owned(),
            page_url: Some(page.to_owned()),
        }
    }

    const PAGE: &str = "https://example.com/contact";

    #[test]
    fn without_a_gesture_nothing_is_asked_or_opened() {
        let mut links = links();

        assert_eq!(
            links.request(&request(1, "mailto:a@example.com", PAGE), false),
            Outcome::Refused(Refusal::NoGesture)
        );
        assert_eq!(links.prompt_for(TabId::new(1), Some(PAGE)), None);
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
        assert_eq!(
            links.request(&request(31, "mailto:a@example.com", PAGE), first),
            Outcome::Asking
        );
        assert_eq!(
            links.request(&request(31, "tel:+15555550123", PAGE), second),
            Outcome::Refused(Refusal::NoGesture)
        );
    }

    #[test]
    fn a_gesture_in_another_tab_does_not_count() {
        let tab = TabId::new(41);
        let now = Instant::now();
        note_user_activation(TabId::new(42), now);

        assert!(!consume_user_activation(tab, now));
    }

    #[test]
    fn with_a_gesture_the_user_is_asked_and_their_yes_opens_the_link() {
        let mut links = links();
        let tab = TabId::new(1);

        assert_eq!(
            links.request(&request(1, "mailto:a@example.com", PAGE), true),
            Outcome::Asking
        );
        let prompt = links
            .prompt_for(tab, Some(PAGE))
            .expect("a confirmation waits")
            .expect("the page is still shown");
        assert_eq!(prompt.message, "example.com wants to open Mail");
        assert_eq!(
            links.answer(tab, PromptAnswer::Open { always: false }),
            Some(Outcome::Opened)
        );
        assert_eq!(links.opener().opened(), vec!["mailto:a@example.com"]);
        assert_eq!(
            links.answer(tab, PromptAnswer::DontOpen),
            None,
            "answered once"
        );
    }

    #[test]
    fn no_opens_nothing() {
        let mut links = links();
        let tab = TabId::new(1);
        links.request(&request(1, "mailto:a@example.com", PAGE), true);

        assert_eq!(
            links.answer(tab, PromptAnswer::DontOpen),
            Some(Outcome::Refused(Refusal::Declined))
        );
        assert!(links.opener().opened().is_empty());
    }

    #[test]
    fn always_allow_skips_the_confirmation_for_that_origin_and_scheme_only() {
        let mut links = links();
        let tab = TabId::new(1);
        links.request(&request(1, "mailto:a@example.com", PAGE), true);
        links.answer(tab, PromptAnswer::Open { always: true });

        assert_eq!(
            links.request(
                &request(1, "mailto:b@example.com", "https://example.com/other"),
                true
            ),
            Outcome::Opened
        );
        assert_eq!(
            links.request(&request(1, "mailto:c@example.com", PAGE), false),
            Outcome::Refused(Refusal::NoGesture),
            "always allow never replaces the gesture"
        );
        assert_eq!(
            links.request(&request(2, "tel:+15555550123", PAGE), true),
            Outcome::Asking,
            "another scheme still asks"
        );
        assert_eq!(
            links.request(
                &request(3, "mailto:d@example.com", "https://other.test/"),
                true
            ),
            Outcome::Asking,
            "another origin still asks"
        );
        assert_eq!(
            links.opener().opened(),
            vec!["mailto:a@example.com", "mailto:b@example.com"]
        );
    }

    #[test]
    fn always_allow_is_not_remembered_for_an_opaque_origin() {
        let mut links = links();
        let tab = TabId::new(1);
        let page = "file:///tmp/page.html";
        links.request(&request(1, "mailto:a@example.com", page), true);
        let prompt = links
            .prompt_for(tab, Some(page))
            .expect("a confirmation waits")
            .expect("the page is still shown");
        assert_eq!(prompt.always_label, None);
        links.answer(tab, PromptAnswer::Open { always: true });

        assert_eq!(
            links.request(&request(1, "mailto:b@example.com", page), true),
            Outcome::Asking
        );
    }

    #[test]
    fn a_scheme_no_app_handles_is_refused_without_asking() {
        let mut links = links();

        assert_eq!(
            links.request(&request(1, "zoommtg://zoom.us/join", PAGE), true),
            Outcome::Refused(Refusal::NoApp)
        );
        assert_eq!(links.prompt_for(TabId::new(1), Some(PAGE)), None);
    }

    #[test]
    fn web_and_file_urls_are_never_handed_to_another_app() {
        let mut links = links();
        for url in [
            "https://example.com/",
            "file:///etc/hosts",
            "data:,x",
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
    fn one_confirmation_per_tab_at_a_time() {
        let mut links = links();
        links.request(&request(1, "mailto:a@example.com", PAGE), true);

        assert_eq!(
            links.request(&request(1, "tel:+15555550123", PAGE), true),
            Outcome::Refused(Refusal::AlreadyAsking)
        );
        assert_eq!(
            links.request(&request(2, "tel:+15555550123", PAGE), true),
            Outcome::Asking
        );
    }

    #[test]
    fn a_confirmation_is_dropped_when_its_page_has_gone() {
        let mut links = links();
        let tab = TabId::new(1);
        links.request(&request(1, "mailto:a@example.com", PAGE), true);

        assert_eq!(
            links.prompt_for(tab, Some("https://example.com/elsewhere")),
            Some(Err(Refusal::Stale))
        );
        assert_eq!(links.prompt_for(tab, Some(PAGE)), None);
        assert!(links.opener().opened().is_empty());
    }

    #[test]
    fn a_confirmation_waits_while_its_tab_is_in_the_background() {
        let mut links = links();
        links.request(&request(2, "mailto:a@example.com", PAGE), true);

        // The active tab (1) has nothing waiting; tab 2's confirmation is kept for later.
        assert_eq!(links.prompt_for(TabId::new(1), Some("")), None);
        assert!(matches!(
            links.prompt_for(TabId::new(2), Some(PAGE)),
            Some(Ok(_))
        ));
    }

    #[test]
    fn closing_the_tab_drops_its_confirmation() {
        let mut links = links();
        links.request(&request(2, "mailto:a@example.com", PAGE), true);

        links.forget_closed_tabs(|tab| tab != TabId::new(2));

        assert_eq!(links.prompt_for(TabId::new(2), Some(PAGE)), None);
    }

    #[test]
    fn the_blank_page_is_compared_as_the_empty_address() {
        let mut links = links();
        links.request(&request(1, "mailto:a@example.com", "about:blank"), true);

        assert!(matches!(
            links.prompt_for(TabId::new(1), Some("")),
            Some(Ok(_))
        ));
    }

    #[test]
    fn the_log_names_the_page_and_scheme_only() {
        assert_eq!(
            describe(&request(3, "mailto:secret@example.com", PAGE)),
            "tab 3: example.com asked to open a mailto: link"
        );
    }

    #[test]
    fn a_normal_run_never_opens_without_the_sheet() {
        let prompt = external_open_prompt(
            Some(&Url::parse(PAGE).expect("valid")),
            &Url::parse("mailto:a@example.com").expect("valid"),
            Some("Mail"),
        );
        assert_eq!(
            confirm_external_open(&prompt, false),
            PromptAnswer::DontOpen
        );
        assert_eq!(
            confirm_external_open(&prompt, true),
            PromptAnswer::Open { always: false }
        );
    }
}
