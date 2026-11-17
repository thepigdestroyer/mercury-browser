//! Links and files other apps send to Mercury Browser, URLs a second launch hands over, and links
//! pages ask other apps to open.
//!
//! Incoming URLs (Apple events on macOS, a second launch through `handoff`) are queued until the
//! controller is attached, so that a URL that started the app is not lost, then each opens in a
//! new tab; the last one is selected. A second launch's addresses are the user's own command line
//! and open as the first launch's would.

pub(crate) mod external;
pub(crate) mod handoff;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
mod ticket;

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use mercury_ui::MainWindow;
use talaria_core::{BrowserController, BrowserEvent, PendingPageRequest, TabActivation};
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
        // downloads: these URLs come from other apps or a second launch and count as typed by
        // the user; call `note_typed_navigation` for each tab opened here when downloads-ui lands.
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
    static RUNNING_OPEN_EVENTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Before the window exists (on macOS, once the backend has made winit's application delegate):
/// starts receiving links and files from other apps, and runs the `start` diagnostics.
pub(crate) fn receive_from_other_apps(automated: bool) {
    #[cfg(target_os = "macos")]
    macos::receive_urls_from_other_apps();
    let Some((time, urls)) = diagnostic_open_events(automated) else {
        return;
    };
    match time {
        #[cfg(target_os = "macos")]
        EventTime::Start => macos::send_open_events_to_this_process(&urls),
        EventTime::Running => RUNNING_OPEN_EVENTS.with(|later| *later.borrow_mut() = urls),
        #[cfg(not(target_os = "macos"))]
        EventTime::Start => log::warn!("openurl: diagnostics: Apple events need macOS"),
    }
}

/// URLs from another app (Apple events on macOS). Call on the main thread. Only web pages and
/// the kinds of files a browser shows are opened (`accept_url_from_other_app`).
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
    let later = RUNNING_OPEN_EVENTS.with(|later| std::mem::take(&mut *later.borrow_mut()));
    #[cfg(target_os = "macos")]
    if !later.is_empty() {
        let delay = std::time::Duration::from_secs(3);
        slint::Timer::single_shot(delay, move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            macos::send_open_events_to_this_process(&later);
        });
    }
    #[cfg(not(target_os = "macos"))]
    drop(later);
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
    let _slint = crate::callback_scope::InSlintCallback::enter();
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

// Second launches.

/// The second launch's side: when the profile is held by a running Mercury Browser and the
/// command line names addresses, hands them over. True if the running browser opened them (the
/// launch then exits 0); false with the reason printed otherwise.
pub(crate) fn hand_over_to_running_browser(profile_root: &Path, addresses: &[String]) -> bool {
    if addresses.is_empty() {
        return false;
    }
    match handoff::hand_over(profile_root, addresses) {
        Ok(count) => {
            eprintln!(
                "mercury-browser: the Mercury Browser using {} opened {count} tab(s)",
                profile_root.display()
            );
            true
        }
        Err(error) => {
            eprintln!(
                "mercury-browser: could not hand the addresses to the Mercury Browser using {}: {error}",
                profile_root.display()
            );
            false
        }
    }
}

/// A profile another launch can find (not a temporary one) listens for that launch's addresses.
/// Keep the result until the run ends; dropping it removes the socket.
pub(crate) fn listen_for_second_launches(
    profile: &crate::profile::ActiveProfile,
) -> Option<handoff::HandoffListener> {
    let paths = profile.paths.as_ref().filter(|_| !profile.is_temporary())?;
    match handoff::listen(&paths.root, open_handed_over_from_any_thread) {
        Ok(listener) => Some(listener),
        Err(error) => {
            log::warn!(
                "openurl: a second launch cannot hand over addresses ({}): {error}",
                handoff::socket_path(&paths.root).display()
            );
            None
        }
    }
}

/// How long the listening thread waits for the window to start opening a second launch's tabs;
/// after that it gives up, and the window opens nothing (`HandOverTicket`).
const HAND_OVER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// How long it then waits for a window that did start opening them; opening a few tabs is quick.
const HAND_OVER_FINISH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// On the listening thread: has the window open the addresses, and waits for the result, so
/// that the second launch is told the truth.
fn open_handed_over_from_any_thread(addresses: Vec<String>) -> Result<usize, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let ticket = ticket::HandOverTicket::default();
    let window_ticket = ticket.clone();
    slint::invoke_from_event_loop(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        open_handed_over_on_main_thread(addresses, window_ticket, sender);
    })
    .map_err(|error| format!("the window cannot be reached ({error})"))?;
    match receiver.recv_timeout(HAND_OVER_TIMEOUT) {
        Ok(result) => result,
        Err(_) if ticket.abandon() => {
            Err("the window did not answer in time; nothing was opened".to_owned())
        }
        // The window started opening them just now: its answer follows.
        Err(_) => receiver
            .recv_timeout(HAND_OVER_FINISH_TIMEOUT)
            .map_err(|_| "the window started opening them but did not finish in time".to_owned())?,
    }
}

type HandOverReply = std::sync::mpsc::Sender<Result<usize, String>>;

fn open_handed_over_on_main_thread(
    addresses: Vec<String>,
    ticket: ticket::HandOverTicket,
    reply: HandOverReply,
) {
    TARGET.with(|target| {
        let target = target.borrow();
        let Some(target) = target.as_ref() else {
            let _ = reply.send(Err("the window is not ready".to_owned()));
            return;
        };
        let Ok(mut controller) = target.controller.try_borrow_mut() else {
            // Busy: try again from the event loop.
            if let Err(error) = slint::invoke_from_event_loop(move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                open_handed_over_on_main_thread(addresses, ticket, reply);
            }) {
                log::warn!("openurl: could not retry opening a second launch's addresses: {error}");
            }
            return;
        };
        if !ticket.claim_for_opening() {
            log::warn!("openurl: a second launch's addresses came too late; not opening them");
            return;
        }
        let result = open_handed_over(&mut controller, &addresses);
        drop(controller);
        if let Ok(count) = &result {
            log::info!("openurl: opened {count} tab(s) for a second launch");
            if let Some(window) = target.window.upgrade() {
                crate::ui_sync::sync_ui(&window, &target.controller.borrow());
                bring_to_front(&window);
            }
            crate::engine::request_pump();
        }
        let _ = reply.send(result);
    });
}

/// A second launch's addresses are the user's own command line: everything the address bar
/// accepts opens (as for the first launch), and nothing opens unless all of them resolve.
pub(crate) fn open_handed_over(
    controller: &mut BrowserController,
    addresses: &[String],
) -> Result<usize, String> {
    for address in addresses {
        controller
            .resolve_address(address)
            .map_err(|error| format!("'{address}': {error}"))?;
    }
    Ok(open_in_tabs(controller, addresses))
}

/// The user started Mercury again to open something: the window comes to the front, shown and
/// out of the Dock first if it was hidden or minimised. Automated runs never take focus.
fn bring_to_front(window: &MainWindow) {
    use slint::ComponentHandle;
    use slint::winit_030::WinitWindowAccessor;

    if AUTOMATED.with(Cell::get) {
        log::info!("openurl: automated run: not bringing the window to the front");
        return;
    }
    window.window().with_winit_window(|winit_window| {
        for step in steps_to_front(winit_window.is_minimized(), winit_window.is_visible()) {
            match step {
                FrontStep::Show => winit_window.set_visible(true),
                FrontStep::Unminimize => winit_window.set_minimized(false),
                FrontStep::Focus => winit_window.focus_window(),
            }
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontStep {
    Show,
    Unminimize,
    Focus,
}

/// `minimized` and `visible` as winit reports them (`None`: not known on this platform). A
/// window that may be hidden or minimised is shown and restored before it is focused; focusing
/// alone leaves a minimised window in the Dock.
fn steps_to_front(minimized: Option<bool>, visible: Option<bool>) -> Vec<FrontStep> {
    let mut steps = Vec::new();
    if visible != Some(true) {
        steps.push(FrontStep::Show);
    }
    if minimized != Some(false) {
        steps.push(FrontStep::Unminimize);
    }
    steps.push(FrontStep::Focus);
    steps
}

// Links pages ask other apps to open.

/// Called by the engine after each batch of engine events: checks the pages' requests to open
/// links in other apps, and queues a confirmation in the tab for those the user is to be asked.
pub(crate) fn handle_external_link_requests(controller: &RefCell<BrowserController>) {
    let requests = controller.borrow_mut().take_external_link_requests();
    if requests.is_empty() {
        return;
    }
    let mut asks = Vec::new();
    LINKS.with(|links| {
        let mut links = links.borrow_mut();
        let Some(links) = links.as_mut() else {
            return;
        };
        for (tab, request) in &requests {
            let now = Instant::now();
            let gesture = consume_user_activation(*tab, now);
            let outcome = links.request(request, gesture);
            log::info!(
                "openurl: {}: {}",
                external::describe(*tab, request),
                describe_outcome(&outcome)
            );
            if outcome == Outcome::Refused(Refusal::NoGesture)
                && GRANT_GESTURE.with(|flag| flag.replace(false))
            {
                log::info!(
                    "openurl: diagnostics: granting tab {} one user gesture",
                    tab.value()
                );
                note_user_activation(*tab, now);
            }
            if let Outcome::Ask(ask) = outcome {
                asks.push((*tab, ask));
            }
        }
    });
    if asks.is_empty() {
        return;
    }
    {
        let mut controller = controller.borrow_mut();
        for (tab, ask) in asks {
            controller.handle_event(tab, BrowserEvent::PageRequested(ask));
        }
    }
    TARGET.with(|target| {
        if let Some(window) = target.borrow().as_ref().and_then(|t| t.window.upgrade()) {
            crate::ui_sync::sync_ui(&window, &controller.borrow());
        }
    });
}

/// The user answered a page request on the sheet; acts on it if it is a confirmation to open a
/// link in another app. `accepted` is the sheet's Open button, `checked` its checkbox.
pub(crate) fn page_request_answered(pending: &PendingPageRequest, accepted: bool, checked: bool) {
    LINKS.with(|links| {
        let mut links = links.borrow_mut();
        let Some(outcome) = links
            .as_mut()
            .and_then(|links| links.answered(pending, accepted, checked))
        else {
            return;
        };
        log::info!(
            "openurl: confirmation {}: {}",
            pending.id().value(),
            describe_outcome(&outcome)
        );
    });
}

fn describe_outcome(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Opened => "opened in its app",
        Outcome::OpenFailed => "the system did not open it",
        Outcome::Ask(_) => "asking the user",
        Outcome::Refused(Refusal::NotExternal) => "refused: not a link for another app",
        Outcome::Refused(Refusal::NoGesture) => "refused: no user gesture in that tab",
        Outcome::Refused(Refusal::NoApp) => "refused: no app opens it",
        Outcome::Refused(Refusal::Declined) => "not opened: the user declined",
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
    fn a_second_launch_opens_whatever_the_address_bar_accepts() {
        let mut controller = BrowserController::default();

        let opened = open_handed_over(
            &mut controller,
            &["file:///tmp/x.rs".to_owned(), "file:///tmp/".to_owned()],
        );

        assert_eq!(opened, Ok(2));
        assert_eq!(controller.active_state().url(), "file:///tmp/");
        assert!(
            controller
                .tabs()
                .any(|tab| tab.state().url() == "file:///tmp/x.rs")
        );
    }

    #[test]
    fn a_second_launch_opens_nothing_unless_every_address_resolves() {
        let mut controller = BrowserController::default();

        let opened = open_handed_over(
            &mut controller,
            &["https://a.test/".to_owned(), "mailto:a@b.test".to_owned()],
        );

        assert!(matches!(opened, Err(reason) if reason.contains("mailto:a@b.test")));
        assert_eq!(controller.tabs().len(), 1);
    }

    #[test]
    fn the_second_launch_waits_longer_than_the_running_browser_does() {
        assert!(handoff::REPLY_TIMEOUT > HAND_OVER_TIMEOUT + HAND_OVER_FINISH_TIMEOUT);
    }

    #[test]
    fn a_hidden_or_minimised_window_is_restored_before_it_is_focused() {
        assert_eq!(
            steps_to_front(Some(false), Some(true)),
            vec![FrontStep::Focus]
        );
        assert_eq!(
            steps_to_front(Some(true), Some(true)),
            vec![FrontStep::Unminimize, FrontStep::Focus]
        );
        assert_eq!(
            steps_to_front(Some(false), Some(false)),
            vec![FrontStep::Show, FrontStep::Focus]
        );
        assert_eq!(
            steps_to_front(None, None),
            vec![FrontStep::Show, FrontStep::Unminimize, FrontStep::Focus]
        );
    }

    #[test]
    fn accepted_urls_are_normalised() {
        assert_eq!(
            accept_url_from_other_app("HTTPS://Example.COM"),
            Ok("https://example.com/".to_owned())
        );
    }
}
