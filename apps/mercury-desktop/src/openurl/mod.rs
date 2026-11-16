//! Links and files other apps send to Mercury Browser, URLs a second launch hands over, and links
//! pages ask other apps to open.
//!
//! Incoming URLs (Apple events on macOS, a second launch through `handoff`) are queued until the
//! controller is attached, so that a URL that started the app is not lost, then each opens in a
//! new tab; the last one is selected.

pub(crate) mod external;
pub(crate) mod handoff;
#[cfg(target_os = "macos")]
pub(crate) mod macos;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use mercury_ui::MainWindow;
use talaria_core::{BrowserController, TabActivation, TabId};
use url::Url;

use crate::activation::{consume_user_activation, note_user_activation};
use external::{ExternalLinks, ExternalOpener, Outcome, RecordingOpener, Refusal};

/// Grants the tab one gesture after its first request is refused for lack of one, so that an
/// automated run can check the path a click would take without any input being synthesised.
/// Read only in automated runs.
const GRANT_GESTURE_VARIABLE: &str = "MERCURY_DIAGNOSTIC_GESTURE_AFTER_REFUSAL";

/// Diagnostics, read only in automated runs on macOS: `start <URL>…` or `running <URL>…` sends
/// this process the Apple events another app would send for the URLs, before the window and
/// controller exist (`start`) or three seconds after (`running`).
const OPEN_EVENTS_VARIABLE: &str = "MERCURY_DIAGNOSTIC_OPEN_EVENTS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventTime {
    Start,
    Running,
}

pub(crate) fn parse_diagnostic_open_events(value: &str) -> Option<(EventTime, Vec<String>)> {
    let mut words = value.split_whitespace();
    let time = match words.next()? {
        "start" => EventTime::Start,
        "running" => EventTime::Running,
        _ => return None,
    };
    let urls = words.map(str::to_owned).collect::<Vec<_>>();
    (!urls.is_empty()).then_some((time, urls))
}

pub(crate) fn diagnostic_open_events(automated: bool) -> Option<(EventTime, Vec<String>)> {
    if !automated {
        return None;
    }
    let value = std::env::var(OPEN_EVENTS_VARIABLE).ok()?;
    let parsed = parse_diagnostic_open_events(&value);
    if parsed.is_none() {
        log::warn!("openurl: diagnostics: ignoring {OPEN_EVENTS_VARIABLE}={value:?}");
    }
    parsed
}

/// URLs waiting for the controller.
#[derive(Debug, Default)]
pub(crate) struct IncomingQueue {
    pending: Vec<String>,
    attached: bool,
}

impl IncomingQueue {
    /// Queues `urls`; true if they should be opened now (the controller is attached).
    pub(crate) fn push(&mut self, urls: impl IntoIterator<Item = String>) -> bool {
        self.pending.extend(urls);
        self.attached && !self.pending.is_empty()
    }

    /// The controller exists: returns what arrived before it.
    pub(crate) fn attach(&mut self) -> Vec<String> {
        self.attached = true;
        self.take()
    }

    pub(crate) fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }
}

/// Which URLs from other apps are opened: web pages, and local files of the kinds a browser
/// shows (HTML, text, images, PDF). Returns the URL as the parser writes it.
pub(crate) fn accept_url_from_other_app(text: &str) -> Result<String, String> {
    const FILE_EXTENSIONS: [&str; 22] = [
        "html", "htm", "xhtml", "xht", "shtml", "txt", "text", "md", "markdown", "log", "csv",
        "json", "xml", "svg", "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "pdf",
    ];
    let url = Url::parse(text).map_err(|error| format!("not a URL ({error})"))?;
    match url.scheme() {
        "http" | "https" => Ok(url.into()),
        "file" => {
            let extension = url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .and_then(|name| name.rsplit_once('.'))
                .map(|(_, extension)| extension.to_ascii_lowercase());
            match extension {
                Some(extension) if FILE_EXTENSIONS.contains(&extension.as_str()) => Ok(url.into()),
                _ => Err("not a kind of file Mercury Browser shows".to_owned()),
            }
        }
        scheme => Err(format!("{scheme}: URLs are not opened from other apps")),
    }
}

/// Opens each URL in a new tab, in order; the last one is selected. Returns how many opened.
pub(crate) fn open_in_tabs(controller: &mut BrowserController, urls: &[String]) -> usize {
    let mut opened = 0;
    for (index, url) in urls.iter().enumerate() {
        let activation = if index + 1 == urls.len() {
            TabActivation::Foreground
        } else {
            TabActivation::Background
        };
        match controller.open_tab_with(Some(url), activation) {
            Ok(_) => opened += 1,
            Err(error) => log::warn!("openurl: could not open {url}: {error}"),
        }
    }
    opened
}

struct Target {
    window: slint::Weak<MainWindow>,
    controller: Rc<RefCell<BrowserController>>,
}

type Links = ExternalLinks<Box<dyn ExternalOpener>>;

thread_local! {
    static INCOMING: RefCell<IncomingQueue> = RefCell::default();
    static TARGET: RefCell<Option<Target>> = const { RefCell::new(None) };
    static LINKS: RefCell<Option<Links>> = const { RefCell::new(None) };
    static AUTOMATED: Cell<bool> = const { Cell::new(false) };
    static GRANT_GESTURE: Cell<bool> = const { Cell::new(false) };
}

/// URLs from another app or a second launch. Call on the main thread.
pub(crate) fn deliver(urls: Vec<String>, source: &str) {
    let accepted = urls
        .into_iter()
        .filter_map(|url| match accept_url_from_other_app(&url) {
            Ok(url) => Some(url),
            Err(reason) => {
                log::warn!("openurl: not opening a URL from {source}: {reason}");
                None
            }
        })
        .collect::<Vec<_>>();
    if accepted.is_empty() {
        return;
    }
    log::info!("openurl: {} URL(s) from {source}", accepted.len());
    if INCOMING.with(|incoming| incoming.borrow_mut().push(accepted)) {
        schedule_opening();
    }
}

/// From any thread: hands URLs to the main thread's `deliver`.
pub(crate) fn deliver_from_any_thread(urls: Vec<String>, source: &'static str) {
    if let Err(error) = slint::invoke_from_event_loop(move || deliver(urls, source)) {
        log::warn!("openurl: could not pass URLs from {source} to the window: {error}");
    }
}

/// The controller exists: URLs that arrived before it open now, later ones as they come.
pub(crate) fn attach(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    automated: bool,
) {
    use slint::ComponentHandle;

    AUTOMATED.with(|flag| flag.set(automated));
    GRANT_GESTURE.with(|flag| {
        flag.set(automated && std::env::var_os(GRANT_GESTURE_VARIABLE).is_some());
    });
    LINKS.with(|links| *links.borrow_mut() = Some(ExternalLinks::new(opener(automated))));
    TARGET.with(|target| {
        *target.borrow_mut() = Some(Target {
            window: window.as_weak(),
            controller,
        });
    });
    let early = INCOMING.with(|incoming| incoming.borrow_mut().attach());
    if !early.is_empty() {
        log::info!(
            "openurl: {} URL(s) arrived before the window; opening them",
            early.len()
        );
        INCOMING.with(|incoming| incoming.borrow_mut().push(early));
        schedule_opening();
    }
}

/// Automated runs never open another app: their opener only records.
fn opener(automated: bool) -> Box<dyn ExternalOpener> {
    #[cfg(target_os = "macos")]
    let names = macos::app_for_url;
    #[cfg(not(target_os = "macos"))]
    let names = |_: &Url| None;
    if automated {
        Box::new(RecordingOpener::new(names))
    } else {
        system_opener()
    }
}

#[cfg(target_os = "macos")]
fn system_opener() -> Box<dyn ExternalOpener> {
    Box::new(macos::WorkspaceOpener)
}

#[cfg(not(target_os = "macos"))]
fn system_opener() -> Box<dyn ExternalOpener> {
    /// Opening links in other apps is not implemented here yet; every request reports no app.
    struct NoOpener;
    impl ExternalOpener for NoOpener {
        fn app_for(&self, _url: &Url) -> Option<String> {
            None
        }
        fn open(&self, _url: &Url) -> bool {
            false
        }
    }
    Box::new(NoOpener)
}

fn schedule_opening() {
    if let Err(error) = slint::invoke_from_event_loop(open_pending) {
        log::warn!("openurl: could not schedule opening URLs: {error}");
    }
}

fn open_pending() {
    TARGET.with(|target| {
        let target = target.borrow();
        let Some(target) = target.as_ref() else {
            return;
        };
        let Ok(mut controller) = target.controller.try_borrow_mut() else {
            schedule_opening();
            return;
        };
        let urls = INCOMING.with(|incoming| incoming.borrow_mut().take());
        if urls.is_empty() {
            return;
        }
        let opened = open_in_tabs(&mut controller, &urls);
        log::info!("openurl: opened {opened} tab(s)");
        drop(controller);
        if let Some(window) = target.window.upgrade() {
            crate::ui_sync::sync_ui(&window, &target.controller.borrow());
        }
        crate::engine::request_pump();
    });
}

/// Called by the engine after each batch of engine events: answers the pages' requests to open
/// links in other apps, and shows the active tab's waiting confirmation.
pub(crate) fn handle_external_link_requests(controller: &RefCell<BrowserController>) {
    let (requests, active_tab, addresses) = {
        let mut controller = controller.borrow_mut();
        let requests = controller.take_external_link_requests();
        let addresses = controller
            .tabs()
            .map(|tab| (tab.id(), tab.state().url().to_owned()))
            .collect::<HashMap<TabId, String>>();
        (requests, controller.active_tab_id(), addresses)
    };
    LINKS.with(|links| {
        let mut links = links.borrow_mut();
        let Some(links) = links.as_mut() else {
            return;
        };
        for request in &requests {
            let now = Instant::now();
            let gesture = consume_user_activation(request.tab_id, now);
            let outcome = links.request(request, gesture);
            log::info!(
                "openurl: {}: {}",
                external::describe(request),
                describe_outcome(&outcome)
            );
            if outcome == Outcome::Refused(Refusal::NoGesture)
                && GRANT_GESTURE.with(|flag| flag.replace(false))
            {
                log::info!(
                    "openurl: diagnostics: granting tab {} one user gesture",
                    request.tab_id.value()
                );
                note_user_activation(request.tab_id, now);
            }
        }
        links.forget_closed_tabs(|tab| addresses.contains_key(&tab));
        match links.prompt_for(active_tab, addresses.get(&active_tab).map(String::as_str)) {
            None => {}
            Some(Err(refusal)) => log::info!(
                "openurl: tab {}: {}",
                active_tab.value(),
                describe_outcome(&Outcome::Refused(refusal))
            ),
            Some(Ok(prompt)) => {
                let answer = external::confirm_external_open(&prompt, AUTOMATED.with(Cell::get));
                if let Some(outcome) = links.answer(active_tab, answer) {
                    log::info!(
                        "openurl: tab {}: {}",
                        active_tab.value(),
                        describe_outcome(&outcome)
                    );
                }
            }
        }
    });
}

fn describe_outcome(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Opened => "opened in its app",
        Outcome::OpenFailed => "the system did not open it",
        Outcome::Asking => "asking the user",
        Outcome::Refused(Refusal::NotExternal) => "refused: not a link for another app",
        Outcome::Refused(Refusal::NoGesture) => "refused: no user gesture in that tab",
        Outcome::Refused(Refusal::NoApp) => "refused: no app opens it",
        Outcome::Refused(Refusal::AlreadyAsking) => "refused: the tab is already asking",
        Outcome::Refused(Refusal::Declined) => "not opened: the user declined",
        Outcome::Refused(Refusal::Stale) => "dropped: the page that asked has gone",
    }
}

#[cfg(test)]
mod tests {
    use talaria_core::LoadState;

    use super::*;

    #[test]
    fn urls_that_arrive_before_the_controller_wait_for_it() {
        let mut queue = IncomingQueue::default();

        assert!(!queue.push(vec!["https://a.test/".to_owned()]));
        assert!(!queue.push(vec!["https://b.test/".to_owned()]));
        assert_eq!(
            queue.attach(),
            vec!["https://a.test/".to_owned(), "https://b.test/".to_owned()]
        );
        assert!(queue.push(vec!["https://c.test/".to_owned()]));
        assert_eq!(queue.take(), vec!["https://c.test/".to_owned()]);
        assert!(!queue.push(Vec::new()), "nothing to open");
    }

    #[test]
    fn each_url_opens_a_tab_and_the_last_is_selected() {
        let mut controller = BrowserController::default();
        let first = controller.active_tab_id();

        let opened = open_in_tabs(
            &mut controller,
            &[
                "https://a.test/".to_owned(),
                "file:///tmp/b.html".to_owned(),
            ],
        );

        assert_eq!(opened, 2);
        let tabs = controller
            .tabs()
            .map(|tab| {
                (
                    tab.state().url().to_owned(),
                    tab.state().load_state().clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tabs,
            vec![
                (String::new(), LoadState::Idle),
                ("https://a.test/".to_owned(), LoadState::Loading),
                ("file:///tmp/b.html".to_owned(), LoadState::Loading),
            ]
        );
        assert_ne!(controller.active_tab_id(), first);
        assert_eq!(controller.active_state().url(), "file:///tmp/b.html");
    }

    #[test]
    fn web_pages_and_viewable_files_are_accepted_from_other_apps() {
        for url in [
            "https://example.com/a?b#c",
            "http://127.0.0.1:8000/",
            "file:///Users/me/page.HTML",
            "file:///Users/me/notes.txt",
            "file:///Users/me/photo.jpeg",
            "file:///Users/me/paper.pdf",
            "file:///Users/me/a%20b.svg",
        ] {
            assert!(accept_url_from_other_app(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn other_urls_from_other_apps_are_refused() {
        for url in [
            "mailto:a@example.com",
            "javascript:alert(1)",
            "data:text/html,x",
            "about:blank",
            "file:///Users/me/",
            "file:///Users/me/script.command",
            "file:///Applications/Calculator.app",
            "file:///Users/me/noextension",
            "not a url",
        ] {
            assert!(accept_url_from_other_app(url).is_err(), "{url}");
        }
    }

    #[test]
    fn diagnostic_open_events_name_a_time_and_urls() {
        assert_eq!(
            parse_diagnostic_open_events("start https://a.test/ file:///tmp/b.html"),
            Some((
                EventTime::Start,
                vec![
                    "https://a.test/".to_owned(),
                    "file:///tmp/b.html".to_owned()
                ]
            ))
        );
        assert_eq!(
            parse_diagnostic_open_events(" running https://a.test/ "),
            Some((EventTime::Running, vec!["https://a.test/".to_owned()]))
        );
        assert_eq!(parse_diagnostic_open_events("running"), None);
        assert_eq!(parse_diagnostic_open_events("later https://a.test/"), None);
        assert_eq!(parse_diagnostic_open_events(""), None);
        assert_eq!(diagnostic_open_events(false), None, "never in a normal run");
    }

    #[test]
    fn accepted_urls_are_normalised() {
        assert_eq!(
            accept_url_from_other_app("HTTPS://Example.COM"),
            Ok("https://example.com/".to_owned())
        );
    }
}
