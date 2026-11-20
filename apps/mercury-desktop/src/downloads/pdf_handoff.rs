// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The PDF hand-off: a page the engine could not show that is a PDF goes to Mercury's own
//! viewer, to the system's PDF app, or to an ordinary download, as the setting `web.pdf` says.
//!
//! The decision is `mercury_downloads::pdf_disposition` (a pure table); this module carries it
//! out. Everything here runs on the UI thread and does no network work itself: it answers with
//! [`Action`]s that the downloads module performs (it owns the engine and the transfer manager).
//!
//! # The seam to the viewer
//!
//! [`PdfViewerSink`] is what the viewer implements and [`install_viewer`] registers once at
//! start-up. With no sink installed, or one that says it is not available (no PDFium library),
//! a PDF behaves as `open_in_system_viewer`.
//!
//! # The viewer's fetch
//!
//! For the viewer the file is fetched again, as a download is, with the same request (the
//! engine's user agent, the URL's cookies, the origin as referrer) into a private folder
//! (`<profile>/pdf-view/`, mode 0700, files 0600). That fetch is never listed as a download:
//! [`PdfHandoff::events`] takes its events out of the stream before the list sees them.
//!
//! - One fetch per tab: a newer offer for the tab cancels the older fetch.
//! - The response says `Content-Disposition: attachment` (`Started::attachment`): the copy is
//!   dropped and the offer goes back through the ordinary download rules ([`Action::Replay`]).
//! - The finished file must hold `%PDF-` in its first 1,024 bytes, else it is dropped and the
//!   ordinary rules apply, with a one-line note.
//! - The tab stays on the PDF's address (no `GoBack`). When the fetch is done the viewer is
//!   handed the file (owned, it deletes it) if the tab still shows the address; otherwise the
//!   file is deleted. A closed tab cancels its fetch and removes its file.
//! - The fetch stops at [`PRIVATE_COPY_MAX_BYTES`], what the viewer opens at most (a response
//!   that says it is bigger is refused before a file is made, a body that outgrows it is cut off
//!   and its file removed): a page can send the tab to a PDF that never ends, and nothing here
//!   asks the user first. The fetch then fails and a one-line note says why.
//! - A private file is removed by whatever ends its use: the tab closing or leaving the address,
//!   a newer offer, the end of the run (`pdf::release` and [`clear_private_folder`]), and, for
//!   what an abrupt end left, the next start ([`sweep_at_start`]).
//! - `file:` addresses (only ones the user started) are read where they are, never copied.
//!
//! The system's PDF app is only ever started by [`Opener`], for a download the user started,
//! once it has finished, and never in an automated run (`--screenshot`, `--background`), which
//! only logs that it would have.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use mercury_downloads::{
    Attachment, Disposition, DownloadEvent, DownloadId, DownloadRequest, Facts, PDF_MARKER_WINDOW,
    PdfPreference, file_name_for, looks_like_pdf, pdf_disposition,
};
use mercury_settings::PdfAction;
use talaria_core::{DownloadOffer, TabId};
use url::Url;

use super::request::{RequestContext, build_request, referrer_for};

/// The most bytes the viewer's private fetch writes: what the viewer opens at most.
pub(super) const PRIVATE_COPY_MAX_BYTES: u64 = mercury_pdf::types::limits::MAX_FILE_BYTES;
/// How long an ordinary download waits to be marked "open when finished" (the system viewer):
/// the engine may keep it waiting briefly.
const OPEN_NOTE_LIFETIME: Duration = Duration::from_secs(60);

/// A PDF for the viewer to show.
#[allow(
    dead_code,
    reason = "read by the PDF viewer (apps/mercury-desktop/src/pdf), which lands with it"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PdfFile {
    pub path: PathBuf,
    /// A private cache copy, which the viewer must delete when it is done; `false` for a file
    /// read where the user has it (`file:`), which it must never delete.
    pub owned: bool,
}

/// What the PDF viewer offers the hand-off.
pub(crate) trait PdfViewerSink {
    /// The PDFium library was found: the viewer can show a document.
    fn is_available(&self) -> bool;
    /// Show `file` in `tab`, lazily when the tab is in the background; `url` is the address the
    /// tab is on.
    fn open(&self, tab: TabId, url: &Url, file: PdfFile);
    /// The tab was closed: end its document and delete an owned file.
    fn tab_closed(&self, tab: TabId);
}

thread_local! {
    static VIEWER: RefCell<Option<Rc<dyn PdfViewerSink>>> = const { RefCell::new(None) };
    static CONFIG: RefCell<Option<Config>> = const { RefCell::new(None) };
}

/// Registers the viewer; called once at start-up by the viewer's own wiring.
#[allow(
    dead_code,
    reason = "called by the PDF viewer's wiring (apps/mercury-desktop/src/pdf); until it lands there is no sink and PDFs use the system viewer"
)]
pub(crate) fn install_viewer(sink: Rc<dyn PdfViewerSink>) {
    VIEWER.with(|viewer| *viewer.borrow_mut() = Some(sink));
}

fn viewer() -> Option<Rc<dyn PdfViewerSink>> {
    VIEWER.with(|viewer| viewer.borrow().clone())
}

/// Where the private files go and whether this is an automated run.
#[derive(Debug, Clone)]
pub(super) struct Config {
    pub(super) cache_dir: PathBuf,
    pub(super) automated: bool,
}

/// Called once at start-up (like `location::configure`). `profile_root` is the run's profile;
/// without one the private folder is a folder of this process in the temporary directory.
pub(super) fn configure(profile_root: Option<&Path>, automated: bool) {
    let cache_dir = match profile_root {
        Some(root) => root.join("pdf-view"),
        None => {
            std::env::temp_dir().join(format!("mercury-browser-pdf-view-{}", std::process::id()))
        }
    };
    CONFIG.with(|config| {
        *config.borrow_mut() = Some(Config {
            cache_dir,
            automated,
        });
    });
}

/// What `configure` set, or a folder in the temporary directory before it ran.
pub(super) fn configured() -> Config {
    CONFIG
        .with(|config| config.borrow().clone())
        .unwrap_or_else(|| Config {
            cache_dir: std::env::temp_dir()
                .join(format!("mercury-browser-pdf-view-{}", std::process::id())),
            automated: true,
        })
}

/// Opens a finished download with the system's PDF app. The real one is LaunchServices through
/// `/usr/bin/open`; tests use a fake and never open anything.
pub(super) trait Opener {
    fn open(&self, path: &Path);
}

/// The setting as the downloads crate's own choice (it does not depend on the settings crate).
#[must_use]
pub(super) fn preference_of(action: PdfAction) -> PdfPreference {
    match action {
        PdfAction::OpenInMercury => PdfPreference::OpenInViewer,
        PdfAction::OpenInSystemViewer => PdfPreference::OpenInSystemViewer,
        PdfAction::Download => PdfPreference::Download,
    }
}

/// What the downloads module knows about how the navigation started (the raw facts of
/// `offers::Navigation`; `None` for a navigation it never saw).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NavigationFacts {
    /// The user started it (a click, a typed address), whatever tab it was in.
    pub(super) user_started: bool,
    /// The tab was the one in front when the request started.
    pub(super) foreground: bool,
    /// The page the tab showed when it started.
    pub(super) from: Option<String>,
}

/// One PDF offer.
pub(super) struct Offered<'a> {
    pub(super) tab: TabId,
    pub(super) offer: &'a DownloadOffer,
    pub(super) navigation: Option<&'a NavigationFacts>,
    /// The tab is the one the user is looking at now.
    pub(super) foreground: bool,
    pub(super) action: PdfAction,
}

/// What the hand-off made of an offer.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Not the hand-off's: the ordinary download rules apply, to the letter.
    Ordinary,
    /// Taken. The actions say what to do next (possibly nothing).
    Handled(Vec<Action>),
}

/// An offer to go through the ordinary rules again, after the hand-off gave it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Replay {
    pub(super) tab: TabId,
    pub(super) offer: DownloadOffer,
    pub(super) navigation: Option<NavigationFacts>,
}

/// What the downloads module is to do for the hand-off.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Action {
    /// Ask the engine for the user agent and cookies of `url`, then call
    /// [`PdfHandoff::request_for`] with `token`.
    Fetch { token: u64, url: Url },
    /// Stop a transfer; its file is removed by the download crate.
    Cancel(DownloadId),
    /// Put the offer through the ordinary rules again (it is the hand-off's no longer).
    Replay(Replay),
    /// A line in the downloads list.
    Note { name: String, message: String },
}

/// What [`PdfHandoff::request_for`] found.
#[derive(Debug)]
pub(super) enum Prepared {
    /// Start this transfer, then call [`PdfHandoff::fetch_started`].
    Go(Box<DownloadRequest>),
    /// The fetch is no longer wanted (a newer offer, a closed tab): do nothing.
    Gone,
    /// It cannot be done: run these actions.
    Failed(Vec<Action>),
}

struct View {
    tab: TabId,
    offer: DownloadOffer,
    url: Url,
    referrer: Option<Url>,
    navigation: Option<NavigationFacts>,
    transfer: Option<DownloadId>,
    /// `false` once superseded, closed, dropped for an attachment: its events are swallowed
    /// and its file deleted.
    live: bool,
}

#[derive(Default)]
struct State {
    next_token: u64,
    views: HashMap<u64, View>,
    by_tab: HashMap<TabId, u64>,
    by_transfer: HashMap<DownloadId, u64>,
    /// Ordinary downloads to open with the system's PDF app once they have finished.
    open_after: HashSet<DownloadId>,
    /// Addresses whose ordinary download is to be opened once it has started.
    pending_open: Vec<(String, Instant)>,
    /// Offers the hand-off sent back to the ordinary rules itself.
    replays: HashSet<(TabId, String)>,
}

/// The hand-off's state: the fetches under way and the downloads to open.
pub(super) struct PdfHandoff {
    config: Config,
    opener: Box<dyn Opener>,
    random_name: Box<dyn Fn() -> String>,
    state: RefCell<State>,
}

impl PdfHandoff {
    pub(super) fn new(
        config: Config,
        opener: Box<dyn Opener>,
        random_name: Box<dyn Fn() -> String>,
    ) -> Self {
        Self {
            config,
            opener,
            random_name,
            state: RefCell::new(State::default()),
        }
    }

    /// One page the engine could not show.
    pub(super) fn offered(&self, offered: &Offered<'_>, now: Instant) -> Outcome {
        let Offered {
            tab,
            offer,
            navigation,
            foreground,
            action,
        } = *offered;
        let Ok(url) = Url::parse(&offer.url) else {
            return Outcome::Ordinary;
        };
        if self
            .state
            .borrow_mut()
            .replays
            .remove(&(tab, offer.url.clone()))
        {
            return Outcome::Ordinary;
        }
        let user_started = navigation.is_some_and(|navigation| navigation.user_started);
        let available = viewer().is_some_and(|sink| sink.is_available());
        let facts = Facts {
            mime_type: &offer.mime_type,
            scheme: url.scheme(),
            user_started,
            foreground,
            posted: offer.posted,
            attachment: Attachment::Unknown,
            automated: self.config.automated,
        };
        match pdf_disposition(preference_of(action), available, &facts) {
            // Every refusal is the ordinary rules' too (they need `http` or `https`, a GET and
            // a tab in front), with their notes and their going back.
            Disposition::NotPdf | Disposition::Download | Disposition::Refuse(_) => {
                Outcome::Ordinary
            }
            Disposition::DownloadThenOpen => {
                let mut state = self.state.borrow_mut();
                state.pending_open.retain(|(_, noted)| {
                    now.saturating_duration_since(*noted) < OPEN_NOTE_LIFETIME
                });
                state.pending_open.push((offer.url.clone(), now));
                Outcome::Ordinary
            }
            Disposition::DownloadWouldOpen => {
                log::info!(
                    "automated run: would open the downloaded PDF {} with the system's PDF app",
                    offer.url
                );
                Outcome::Ordinary
            }
            Disposition::OpenInViewer { read_in_place } => {
                if read_in_place {
                    self.open_in_place(tab, &url, offer)
                } else {
                    self.start_view(tab, url, offer, navigation)
                }
            }
        }
    }

    /// A `file:` PDF the user opened: read where it is.
    fn open_in_place(&self, tab: TabId, url: &Url, offer: &DownloadOffer) -> Outcome {
        let Ok(path) = url.to_file_path() else {
            return Outcome::Ordinary;
        };
        let name = file_name_for(None, url, Some(&offer.mime_type), None);
        if !holds_a_pdf(&path) {
            return Outcome::Handled(vec![Action::Note {
                name,
                message: "This file is not a PDF, so it was not opened".to_owned(),
            }]);
        }
        match viewer() {
            Some(sink) => {
                sink.open(tab, url, PdfFile { path, owned: false });
                Outcome::Handled(Vec::new())
            }
            None => Outcome::Ordinary,
        }
    }

    /// A PDF over the web: fetch it privately, for the viewer.
    fn start_view(
        &self,
        tab: TabId,
        url: Url,
        offer: &DownloadOffer,
        navigation: Option<&NavigationFacts>,
    ) -> Outcome {
        let referrer = offer
            .referrer
            .as_deref()
            .and_then(|referrer| Url::parse(referrer).ok());
        let referrer = referrer_for(referrer.as_ref());
        let mut state = self.state.borrow_mut();
        state.next_token += 1;
        let token = state.next_token;
        let mut actions = Vec::new();
        // One fetch per tab: the newer offer wins.
        if let Some(older) = state.by_tab.insert(tab, token) {
            actions.extend(retire(&mut state, older));
        }
        state.views.insert(
            token,
            View {
                tab,
                offer: offer.clone(),
                url: url.clone(),
                referrer: referrer.clone(),
                navigation: navigation.cloned(),
                transfer: None,
                live: true,
            },
        );
        actions.push(Action::Fetch { token, url });
        Outcome::Handled(actions)
    }

    /// The engine answered for fetch `token`: the transfer to start, if it is still wanted.
    pub(super) fn request_for(
        &self,
        token: u64,
        user_agent: String,
        cookies: Vec<(String, String)>,
    ) -> Prepared {
        let state = self.state.borrow();
        let Some(view) = state.views.get(&token).filter(|view| view.live) else {
            return Prepared::Gone;
        };
        if let Err(error) = create_private_dir(&self.config.cache_dir) {
            log::warn!("cannot create {}: {error}", self.config.cache_dir.display());
            drop(state);
            return Prepared::Failed(
                self.fetch_failed(token, "A private folder for the PDF could not be made"),
            );
        }
        Prepared::Go(Box::new(build_request(
            view.url.clone(),
            RequestContext {
                referrer: view.referrer.clone(),
                user_agent,
                cookies,
                destination_dir: self.config.cache_dir.clone(),
                suggested_name: Some((self.random_name)()),
                resume: None,
                // A page can send the tab to a PDF that never ends: the private copy stops at the
                // largest file the viewer opens (the helper refuses more anyway).
                max_bytes: Some(PRIVATE_COPY_MAX_BYTES),
            },
        )))
    }

    /// The transfer for fetch `token` was started as `id`.
    pub(super) fn fetch_started(&self, token: u64, id: DownloadId) -> Vec<Action> {
        let mut state = self.state.borrow_mut();
        let Some(view) = state.views.get_mut(&token) else {
            return vec![Action::Cancel(id)];
        };
        view.transfer = Some(id);
        let live = view.live;
        state.by_transfer.insert(id, token);
        if live {
            Vec::new()
        } else {
            vec![Action::Cancel(id)]
        }
    }

    /// Fetch `token` could not be started (no engine, no downloads): forget it and say so.
    pub(super) fn fetch_failed(&self, token: u64, message: &str) -> Vec<Action> {
        let mut state = self.state.borrow_mut();
        let Some(view) = state.views.remove(&token) else {
            return Vec::new();
        };
        if state.by_tab.get(&view.tab) == Some(&token) {
            state.by_tab.remove(&view.tab);
        }
        if !view.live {
            return Vec::new();
        }
        vec![Action::Note {
            name: view_name(&view),
            message: message.to_owned(),
        }]
    }

    /// An ordinary download of `url` started as `id`: if the hand-off asked for it to be opened
    /// when it finishes (the system viewer), remember that.
    pub(super) fn download_started(&self, url: &Url, id: DownloadId, now: Instant) {
        let mut state = self.state.borrow_mut();
        state
            .pending_open
            .retain(|(_, noted)| now.saturating_duration_since(*noted) < OPEN_NOTE_LIFETIME);
        if let Some(position) = state
            .pending_open
            .iter()
            .position(|(noted, _)| noted == url.as_str())
        {
            state.pending_open.remove(position);
            state.open_after.insert(id);
        }
    }

    /// The download events, in order: the hand-off's own are taken out and acted on; the rest
    /// are returned for the downloads list (after a look at the ones whose file is to be
    /// opened). `still_wanted` says whether the tab still shows the address.
    pub(super) fn events(
        &self,
        events: Vec<DownloadEvent>,
        still_wanted: &dyn Fn(TabId, &Url) -> bool,
    ) -> (Vec<DownloadEvent>, Vec<Action>) {
        let mut state = self.state.borrow_mut();
        let mut rest = Vec::new();
        let mut actions = Vec::new();
        // Files for the viewer, handed over once the state is released: the viewer may call back.
        let mut opens = Vec::new();
        for event in events {
            let id = event_id(&event);
            if let Some(token) = state.by_transfer.get(&id).copied() {
                self.view_event(
                    &mut state,
                    token,
                    id,
                    event,
                    still_wanted,
                    (&mut actions, &mut opens),
                );
                continue;
            }
            match &event {
                DownloadEvent::Finished { target_path, .. } if state.open_after.remove(&id) => {
                    self.opener.open(target_path);
                }
                DownloadEvent::Failed { .. } | DownloadEvent::Cancelled { .. } => {
                    state.open_after.remove(&id);
                }
                _ => {}
            }
            rest.push(event);
        }
        drop(state);
        if let Some(sink) = viewer() {
            for (tab, url, file) in opens {
                sink.open(tab, &url, file);
            }
        }
        (rest, actions)
    }

    fn view_event(
        &self,
        state: &mut State,
        token: u64,
        id: DownloadId,
        event: DownloadEvent,
        still_wanted: &dyn Fn(TabId, &Url) -> bool,
        (actions, opens): (&mut Vec<Action>, &mut Vec<(TabId, Url, PdfFile)>),
    ) {
        match event {
            DownloadEvent::Started {
                attachment,
                partial_path,
                ..
            } => {
                make_private(&partial_path);
                let Some(view) = state.views.get_mut(&token) else {
                    return;
                };
                if view.live && attachment {
                    // The server means the file to be saved, not shown.
                    view.live = false;
                    let (tab, offer, navigation) =
                        (view.tab, view.offer.clone(), view.navigation.clone());
                    if state.by_tab.get(&tab) == Some(&token) {
                        state.by_tab.remove(&tab);
                    }
                    state.replays.insert((tab, offer.url.clone()));
                    actions.push(Action::Cancel(id));
                    actions.push(Action::Replay(Replay {
                        tab,
                        offer,
                        navigation,
                    }));
                }
            }
            DownloadEvent::Progress { .. } => {}
            DownloadEvent::Finished { target_path, .. } => {
                let Some(view) = state.views.remove(&token) else {
                    return;
                };
                state.by_transfer.remove(&id);
                if state.by_tab.get(&view.tab) == Some(&token) {
                    state.by_tab.remove(&view.tab);
                }
                if !view.live || !still_wanted(view.tab, &view.url) {
                    delete(&target_path);
                    return;
                }
                make_private(&target_path);
                if !holds_a_pdf(&target_path) {
                    delete(&target_path);
                    state.replays.insert((view.tab, view.offer.url.clone()));
                    actions.push(Action::Note {
                        name: view_name(&view),
                        message: "This file is not a PDF, so it was saved as a download".to_owned(),
                    });
                    actions.push(Action::Replay(Replay {
                        tab: view.tab,
                        offer: view.offer,
                        navigation: view.navigation,
                    }));
                    return;
                }
                match viewer() {
                    Some(_) => opens.push((
                        view.tab,
                        view.url,
                        PdfFile {
                            path: target_path,
                            owned: true,
                        },
                    )),
                    None => {
                        delete(&target_path);
                        state.replays.insert((view.tab, view.offer.url.clone()));
                        actions.push(Action::Replay(Replay {
                            tab: view.tab,
                            offer: view.offer,
                            navigation: view.navigation,
                        }));
                    }
                }
            }
            DownloadEvent::Failed { error, .. } => {
                let Some(view) = state.views.remove(&token) else {
                    return;
                };
                state.by_transfer.remove(&id);
                if state.by_tab.get(&view.tab) == Some(&token) {
                    state.by_tab.remove(&view.tab);
                }
                if view.live {
                    actions.push(Action::Note {
                        name: view_name(&view),
                        message: format!("The PDF could not be opened: {error}"),
                    });
                }
            }
            DownloadEvent::Cancelled { .. } => {
                if let Some(view) = state.views.remove(&token)
                    && state.by_tab.get(&view.tab) == Some(&token)
                {
                    state.by_tab.remove(&view.tab);
                }
                state.by_transfer.remove(&id);
            }
        }
    }

    /// `tab` was closed: its fetch is cancelled and the viewer told.
    pub(super) fn tab_closed(&self, tab: TabId) -> Vec<Action> {
        let mut state = self.state.borrow_mut();
        state.replays.retain(|(replayed, _)| *replayed != tab);
        let mut actions = Vec::new();
        if let Some(token) = state.by_tab.remove(&tab) {
            actions.extend(retire(&mut state, token));
        }
        drop(state);
        if let Some(sink) = viewer() {
            sink.tab_closed(tab);
        }
        actions
    }

    /// Whether the hand-off sent this offer back to the ordinary rules itself (and so will not
    /// take it again). It is consumed by `offered`.
    #[cfg(test)]
    fn replay_pending(&self, tab: TabId, url: &str) -> bool {
        self.state.borrow().replays.contains(&(tab, url.to_owned()))
    }

    /// How many fetches are under way or being wound down.
    #[cfg(test)]
    fn fetches(&self) -> usize {
        self.state.borrow().views.len()
    }
}

/// Stops fetch `token`: its events are swallowed from now on and its file deleted. A fetch still
/// waiting for the engine is simply forgotten.
fn retire(state: &mut State, token: u64) -> Vec<Action> {
    let Some(view) = state.views.get_mut(&token) else {
        return Vec::new();
    };
    view.live = false;
    match view.transfer {
        Some(id) => vec![Action::Cancel(id)],
        None => {
            state.views.remove(&token);
            Vec::new()
        }
    }
}

fn view_name(view: &View) -> String {
    file_name_for(None, &view.url, Some(&view.offer.mime_type), None)
}

fn event_id(event: &DownloadEvent) -> DownloadId {
    match event {
        DownloadEvent::Started { id, .. }
        | DownloadEvent::Progress { id, .. }
        | DownloadEvent::Finished { id, .. }
        | DownloadEvent::Failed { id, .. }
        | DownloadEvent::Cancelled { id } => *id,
    }
}

/// Whether the file starts like a PDF: a regular file with `%PDF-` in its first kibibyte.
fn holds_a_pdf(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return false;
    }
    let mut head = Vec::with_capacity(PDF_MARKER_WINDOW);
    if file
        .take(PDF_MARKER_WINDOW as u64)
        .read_to_end(&mut head)
        .is_err()
    {
        return false;
    }
    looks_like_pdf(&head)
}

fn delete(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("could not remove {}: {error}", path.display());
    }
}

/// Only the user may read a private copy (mode 0600).
fn make_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("could not make {} private: {error}", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// The private folder, readable by the user only (mode 0700).
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Start-up: removes the files an earlier run left in the private folder, on a thread of its own.
///
/// Every file already there is an earlier run's: the profile is locked by one running Mercury, and
/// this run has fetched nothing yet (a profile-less run uses a folder named for its process). The
/// cut-off is taken before the thread starts, so a file this run makes meanwhile is never removed.
pub(super) fn sweep_at_start() {
    let dir = configured().cache_dir;
    let started = SystemTime::now();
    std::thread::spawn(move || {
        let removed = sweep(&dir, started);
        if removed > 0 {
            log::info!(
                "removed {removed} PDF file(s) an earlier run left in {}",
                dir.display()
            );
        }
    });
}

/// Removes the files in `dir` last changed before `started`, the time this run began: what an
/// earlier run left behind (it was ended abruptly; a normal end removes its own). Only regular
/// files are touched, and nothing in a sub-folder. Returns how many were removed.
pub(super) fn sweep(dir: &Path, started: SystemTime) -> usize {
    remove_regular_files(dir, |modified| {
        // A file that cannot say when it changed is not known to be this run's: it goes.
        modified.is_none_or(|modified| modified < started)
    })
}

/// The end of the run: removes every file still in the private folder. Called after the viewer's
/// sessions have deleted theirs and the download manager has stopped, so what is left belongs to
/// nobody: a fetch that finished as the run ended, a partial file. Returns how many were removed.
pub(crate) fn clear_private_folder() -> usize {
    let dir = configured().cache_dir;
    let removed = remove_regular_files(&dir, |_| true);
    if removed > 0 {
        log::info!("removed {removed} PDF file(s) from {}", dir.display());
    }
    removed
}

/// Removes the regular files directly in `dir` for which `remove` (given when the file was last
/// changed, if it says) answers yes. Returns how many were removed.
fn remove_regular_files(dir: &Path, remove: impl Fn(Option<SystemTime>) -> bool) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.file_type().is_file() {
            continue;
        }
        if remove(metadata.modified().ok()) && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn tab(value: u64) -> TabId {
        TabId::new(value)
    }

    struct FakeSink {
        available: bool,
        opened: RefCell<Vec<(TabId, String, PdfFile)>>,
        closed: RefCell<Vec<TabId>>,
    }

    impl PdfViewerSink for FakeSink {
        fn is_available(&self) -> bool {
            self.available
        }

        fn open(&self, tab: TabId, url: &Url, file: PdfFile) {
            self.opened.borrow_mut().push((tab, url.to_string(), file));
        }

        fn tab_closed(&self, tab: TabId) {
            self.closed.borrow_mut().push(tab);
        }
    }

    fn install(available: bool) -> Rc<FakeSink> {
        let sink = Rc::new(FakeSink {
            available,
            opened: RefCell::new(Vec::new()),
            closed: RefCell::new(Vec::new()),
        });
        install_viewer(Rc::clone(&sink) as Rc<dyn PdfViewerSink>);
        sink
    }

    #[derive(Clone, Default)]
    struct FakeOpener {
        opened: Rc<RefCell<Vec<PathBuf>>>,
    }

    impl Opener for FakeOpener {
        fn open(&self, path: &Path) {
            self.opened.borrow_mut().push(path.to_path_buf());
        }
    }

    struct Rig {
        handoff: PdfHandoff,
        opener: FakeOpener,
        folder: tempfile::TempDir,
    }

    fn rig(automated: bool) -> Rig {
        let folder = tempfile::tempdir().unwrap();
        let opener = FakeOpener::default();
        let counter = Cell::new(0_u32);
        let handoff = PdfHandoff::new(
            Config {
                cache_dir: folder.path().join("pdf-view"),
                automated,
            },
            Box::new(opener.clone()),
            Box::new(move || {
                counter.set(counter.get() + 1);
                format!("random-{}.pdf", counter.get())
            }),
        );
        Rig {
            handoff,
            opener,
            folder,
        }
    }

    fn offer(url: &str) -> DownloadOffer {
        DownloadOffer {
            url: url.to_owned(),
            mime_type: "application/pdf".to_owned(),
            posted: Some(false),
            referrer: Some("https://page.example/article?id=7".to_owned()),
        }
    }

    fn typed() -> NavigationFacts {
        NavigationFacts {
            user_started: true,
            foreground: true,
            from: Some("https://page.example/".to_owned()),
        }
    }

    fn page_started() -> NavigationFacts {
        NavigationFacts {
            user_started: false,
            foreground: true,
            from: Some("https://page.example/".to_owned()),
        }
    }

    fn offered(
        rig: &Rig,
        tab_id: u64,
        offer: &DownloadOffer,
        navigation: Option<&NavigationFacts>,
        foreground: bool,
        action: PdfAction,
    ) -> Outcome {
        rig.handoff.offered(
            &Offered {
                tab: tab(tab_id),
                offer,
                navigation,
                foreground,
                action,
            },
            Instant::now(),
        )
    }

    fn token_of(outcome: &Outcome) -> u64 {
        match outcome {
            Outcome::Handled(actions) => actions
                .iter()
                .find_map(|action| match action {
                    Action::Fetch { token, .. } => Some(*token),
                    _ => None,
                })
                .expect("a Fetch action"),
            other => panic!("expected the hand-off to take it, got {other:?}"),
        }
    }

    fn started(id: u64, attachment: bool, partial: &Path) -> DownloadEvent {
        DownloadEvent::Started {
            id: DownloadId(id),
            file_name: "doc.pdf".into(),
            target_path: partial.with_extension(""),
            partial_path: partial.to_path_buf(),
            total_bytes: Some(10),
            mime_type: Some("application/pdf".into()),
            final_url: Url::parse("https://files.example/doc.pdf").unwrap(),
            etag: None,
            last_modified: None,
            attachment,
        }
    }

    fn finished(id: u64, path: &Path) -> DownloadEvent {
        DownloadEvent::Finished {
            id: DownloadId(id),
            target_path: path.to_path_buf(),
            received_bytes: 10,
        }
    }

    fn always(_: TabId, _: &Url) -> bool {
        true
    }

    /// Starts a viewer fetch for tab 1 of `url` and returns its token, with the transfer `id`
    /// registered.
    fn fetch(rig: &Rig, tab_id: u64, url: &str, id: u64) -> u64 {
        let outcome = offered(
            rig,
            tab_id,
            &offer(url),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        );
        let token = token_of(&outcome);
        assert!(matches!(
            rig.handoff.request_for(token, "UA".into(), Vec::new()),
            Prepared::Go(_)
        ));
        assert_eq!(rig.handoff.fetch_started(token, DownloadId(id)), Vec::new());
        token
    }

    fn write_pdf(rig: &Rig, name: &str) -> PathBuf {
        let path = rig.folder.path().join(name);
        fs::write(&path, b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n1 0 obj\n").unwrap();
        path
    }

    // ── No viewer: the system's PDF app ─────────────────────────────────────────────────

    #[test]
    fn with_no_sink_installed_a_user_started_pdf_is_downloaded_then_opened_by_the_system() {
        let rig = rig(false);
        let url = "https://files.example/doc.pdf";
        let outcome = offered(
            &rig,
            1,
            &offer(url),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        );
        // The ordinary rules download it; the hand-off only remembers to open the result.
        assert_eq!(outcome, Outcome::Ordinary);
        rig.handoff
            .download_started(&Url::parse(url).unwrap(), DownloadId(5), Instant::now());

        let file = write_pdf(&rig, "doc.pdf");
        let (rest, actions) = rig.handoff.events(vec![finished(5, &file)], &always);
        assert_eq!(rest.len(), 1, "the list still sees the download");
        assert!(actions.is_empty());
        assert_eq!(*rig.opener.opened.borrow(), vec![file]);
    }

    #[test]
    fn a_pdf_a_page_started_is_never_opened_by_the_system_even_without_a_viewer() {
        let rig = rig(false);
        let url = "https://files.example/doc.pdf";
        let outcome = offered(
            &rig,
            1,
            &offer(url),
            Some(&page_started()),
            true,
            PdfAction::OpenInMercury,
        );
        assert_eq!(outcome, Outcome::Ordinary);
        rig.handoff
            .download_started(&Url::parse(url).unwrap(), DownloadId(5), Instant::now());
        let file = write_pdf(&rig, "doc.pdf");
        rig.handoff.events(vec![finished(5, &file)], &always);
        assert!(rig.opener.opened.borrow().is_empty());
    }

    #[test]
    fn a_viewer_that_is_not_available_counts_as_no_viewer() {
        let rig = rig(false);
        let _sink = install(false);
        let url = "https://files.example/doc.pdf";
        let outcome = offered(
            &rig,
            1,
            &offer(url),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        );
        assert_eq!(outcome, Outcome::Ordinary);
        assert_eq!(rig.handoff.fetches(), 0);
    }

    #[test]
    fn the_system_viewer_setting_opens_a_user_started_download_only() {
        let rig = rig(false);
        let _sink = install(true);
        let url = "https://files.example/a.pdf";
        assert_eq!(
            offered(
                &rig,
                1,
                &offer(url),
                Some(&typed()),
                true,
                PdfAction::OpenInSystemViewer
            ),
            Outcome::Ordinary
        );
        rig.handoff
            .download_started(&Url::parse(url).unwrap(), DownloadId(2), Instant::now());
        let file = write_pdf(&rig, "a.pdf");
        rig.handoff.events(vec![finished(2, &file)], &always);
        assert_eq!(*rig.opener.opened.borrow(), vec![file]);

        let other = "https://files.example/b.pdf";
        assert_eq!(
            offered(
                &rig,
                1,
                &offer(other),
                Some(&page_started()),
                true,
                PdfAction::OpenInSystemViewer
            ),
            Outcome::Ordinary
        );
        rig.handoff
            .download_started(&Url::parse(other).unwrap(), DownloadId(3), Instant::now());
        let second = write_pdf(&rig, "b.pdf");
        rig.handoff.events(vec![finished(3, &second)], &always);
        assert_eq!(rig.opener.opened.borrow().len(), 1, "b.pdf was not opened");
    }

    #[test]
    fn an_automated_run_never_opens_anything_and_still_downloads() {
        let rig = rig(true);
        let _sink = install(true);
        let url = "https://files.example/a.pdf";
        assert_eq!(
            offered(
                &rig,
                1,
                &offer(url),
                Some(&typed()),
                true,
                PdfAction::OpenInSystemViewer
            ),
            Outcome::Ordinary
        );
        rig.handoff
            .download_started(&Url::parse(url).unwrap(), DownloadId(2), Instant::now());
        let file = write_pdf(&rig, "a.pdf");
        let (rest, _) = rig.handoff.events(vec![finished(2, &file)], &always);
        assert_eq!(rest.len(), 1);
        assert!(rig.opener.opened.borrow().is_empty());
        // The viewer, in an automated run, is used as usual.
        let viewer_url = "https://files.example/v.pdf";
        let outcome = offered(
            &rig,
            2,
            &offer(viewer_url),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        );
        assert!(token_of(&outcome) > 0);
    }

    #[test]
    fn the_download_setting_leaves_everything_to_the_ordinary_rules() {
        let rig = rig(false);
        let _sink = install(true);
        let outcome = offered(
            &rig,
            1,
            &offer("https://files.example/a.pdf"),
            Some(&typed()),
            true,
            PdfAction::Download,
        );
        assert_eq!(outcome, Outcome::Ordinary);
        assert_eq!(rig.handoff.fetches(), 0);
    }

    #[test]
    fn something_that_is_not_a_pdf_is_not_the_hand_offs() {
        let rig = rig(false);
        let _sink = install(true);
        let mut zip = offer("https://files.example/a.zip");
        zip.mime_type = "application/zip".to_owned();
        assert_eq!(
            offered(
                &rig,
                1,
                &zip,
                Some(&typed()),
                true,
                PdfAction::OpenInMercury
            ),
            Outcome::Ordinary
        );
    }

    // ── The viewer ──────────────────────────────────────────────────────────────────────

    #[test]
    fn a_pdf_for_the_viewer_is_fetched_privately_with_the_downloads_request() {
        let rig = rig(false);
        let sink = install(true);
        let url = "https://files.example/doc.pdf";
        let outcome = offered(
            &rig,
            1,
            &offer(url),
            Some(&page_started()),
            true,
            PdfAction::OpenInMercury,
        );
        let Outcome::Handled(actions) = &outcome else {
            panic!("{outcome:?}")
        };
        assert_eq!(
            actions,
            &vec![Action::Fetch {
                token: 1,
                url: Url::parse(url).unwrap(),
            }]
        );
        let Prepared::Go(request) =
            rig.handoff
                .request_for(1, "Agent/1".into(), vec![("session".into(), "abc".into())])
        else {
            panic!("a request")
        };
        assert_eq!(request.url.as_str(), url);
        assert_eq!(request.user_agent, "Agent/1");
        assert_eq!(request.cookie_header.as_deref(), Some("session=abc"));
        assert_eq!(
            request.referrer.as_ref().map(Url::as_str),
            Some("https://page.example/")
        );
        assert_eq!(request.destination_dir, rig.folder.path().join("pdf-view"));
        assert_eq!(request.suggested_name.as_deref(), Some("random-1.pdf"));
        // A page can send the tab to a PDF that never ends: the copy stops at what the viewer
        // opens at most, 512 MiB.
        assert_eq!(
            request.max_bytes,
            Some(mercury_pdf::types::limits::MAX_FILE_BYTES)
        );
        assert_eq!(request.max_bytes, Some(512 * 1024 * 1024));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&request.destination_dir)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert!(sink.opened.borrow().is_empty());
    }

    #[test]
    fn a_finished_pdf_is_handed_to_the_viewer_owned_and_is_not_listed() {
        let rig = rig(false);
        let sink = install(true);
        let url = "https://files.example/doc.pdf";
        fetch(&rig, 1, url, 7);
        let cache = rig.folder.path().join("pdf-view");
        fs::create_dir_all(&cache).unwrap();
        let partial = cache.join("random-1.pdf.part");
        fs::write(&partial, b"%PDF-1.7").unwrap();
        let done = write_pdf(&rig, "random-1.pdf");

        let (rest, actions) = rig.handoff.events(
            vec![
                started(7, false, &partial),
                DownloadEvent::Progress {
                    id: DownloadId(7),
                    received_bytes: 8,
                    total_bytes: Some(8),
                    bytes_per_second: 1,
                },
                finished(7, &done),
            ],
            &always,
        );
        assert!(rest.is_empty(), "the fetch is never listed as a download");
        assert!(actions.is_empty());
        let opened = sink.opened.borrow();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].0, tab(1));
        assert_eq!(opened[0].1, url);
        assert_eq!(
            opened[0].2,
            PdfFile {
                path: done.clone(),
                owned: true
            }
        );
        assert!(done.exists(), "the viewer owns it now");
        assert_eq!(rig.handoff.fetches(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&done).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // No GoBack and nothing else: the hand-off asked for no action.
        assert!(rig.opener.opened.borrow().is_empty());
    }

    #[test]
    fn events_of_other_downloads_pass_through_untouched_and_in_order() {
        let rig = rig(false);
        let _sink = install(true);
        fetch(&rig, 1, "https://files.example/doc.pdf", 7);
        let other = DownloadEvent::Progress {
            id: DownloadId(99),
            received_bytes: 1,
            total_bytes: None,
            bytes_per_second: 1,
        };
        let (rest, _) = rig.handoff.events(
            vec![
                other.clone(),
                DownloadEvent::Cancelled { id: DownloadId(98) },
            ],
            &always,
        );
        assert_eq!(rest.len(), 2);
        assert!(matches!(rest[0], DownloadEvent::Progress { .. }));
        assert!(matches!(rest[1], DownloadEvent::Cancelled { .. }));
    }

    #[test]
    fn an_attachment_is_dropped_and_goes_through_the_ordinary_rules() {
        let rig = rig(false);
        let sink = install(true);
        let url = "https://files.example/report.pdf";
        fetch(&rig, 1, url, 7);
        let cache = rig.folder.path().join("pdf-view");
        fs::create_dir_all(&cache).unwrap();
        let partial = cache.join("random-1.pdf.part");
        fs::write(&partial, b"%PDF-").unwrap();

        let (rest, actions) = rig
            .handoff
            .events(vec![started(7, true, &partial)], &always);
        assert!(rest.is_empty());
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], Action::Cancel(DownloadId(7)));
        let Action::Replay(replay) = &actions[1] else {
            panic!("{actions:?}")
        };
        assert_eq!(replay.tab, tab(1));
        assert_eq!(replay.offer.url, url);
        assert_eq!(replay.navigation.as_ref(), Some(&typed()));
        assert!(rig.handoff.replay_pending(tab(1), url));

        // The cancel's own event is swallowed; nothing reaches the viewer.
        let (rest, actions) = rig.handoff.events(
            vec![DownloadEvent::Cancelled { id: DownloadId(7) }],
            &always,
        );
        assert!(rest.is_empty() && actions.is_empty());
        assert!(sink.opened.borrow().is_empty());
        assert_eq!(rig.handoff.fetches(), 0);

        // Offered again, it is the ordinary rules' (once).
        let again = offered(
            &rig,
            1,
            &offer(url),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        );
        assert_eq!(again, Outcome::Ordinary);
        assert!(!rig.handoff.replay_pending(tab(1), url));
        let third = offered(
            &rig,
            1,
            &offer(url),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        );
        assert!(
            matches!(third, Outcome::Handled(_)),
            "the replay was used up"
        );
    }

    #[test]
    fn a_body_that_is_not_a_pdf_is_deleted_and_saved_as_a_download_with_a_note() {
        let rig = rig(false);
        let sink = install(true);
        let url = "https://files.example/doc.pdf";
        fetch(&rig, 1, url, 7);
        let bad = rig.folder.path().join("bad.pdf");
        fs::write(&bad, b"<html>sorry</html>").unwrap();

        let (rest, actions) = rig.handoff.events(vec![finished(7, &bad)], &always);
        assert!(rest.is_empty());
        assert!(!bad.exists());
        assert!(sink.opened.borrow().is_empty());
        assert!(matches!(actions[0], Action::Note { .. }));
        assert!(matches!(actions[1], Action::Replay(_)));
    }

    #[test]
    fn a_newer_offer_for_the_tab_cancels_the_older_fetch_and_drops_its_file() {
        let rig = rig(false);
        let sink = install(true);
        fetch(&rig, 1, "https://files.example/old.pdf", 7);
        let newer = offered(
            &rig,
            1,
            &offer("https://files.example/new.pdf"),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        );
        let Outcome::Handled(actions) = &newer else {
            panic!("{newer:?}")
        };
        assert_eq!(actions[0], Action::Cancel(DownloadId(7)));
        assert!(matches!(actions[1], Action::Fetch { token: 2, .. }));

        // The older fetch finishes anyway (it was past the point of cancelling): deleted,
        // never shown.
        let stale = write_pdf(&rig, "old.pdf");
        let (rest, _) = rig.handoff.events(vec![finished(7, &stale)], &always);
        assert!(rest.is_empty());
        assert!(!stale.exists());
        assert!(sink.opened.borrow().is_empty());
        assert_eq!(rig.handoff.fetches(), 1, "only the newer fetch is left");
    }

    #[test]
    fn a_fetch_replaced_before_the_engine_answered_is_gone() {
        let rig = rig(false);
        let _sink = install(true);
        let first = token_of(&offered(
            &rig,
            1,
            &offer("https://files.example/old.pdf"),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        ));
        let second = token_of(&offered(
            &rig,
            1,
            &offer("https://files.example/new.pdf"),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        ));
        assert_ne!(first, second);
        assert!(matches!(
            rig.handoff.request_for(first, "UA".into(), Vec::new()),
            Prepared::Gone
        ));
        assert!(matches!(
            rig.handoff.request_for(second, "UA".into(), Vec::new()),
            Prepared::Go(_)
        ));
    }

    #[test]
    fn a_tab_that_moved_on_gets_no_viewer_and_the_file_is_deleted() {
        let rig = rig(false);
        let sink = install(true);
        fetch(&rig, 1, "https://files.example/doc.pdf", 7);
        let done = write_pdf(&rig, "doc.pdf");
        let (rest, actions) = rig.handoff.events(vec![finished(7, &done)], &|_, _| false);
        assert!(rest.is_empty() && actions.is_empty());
        assert!(!done.exists());
        assert!(sink.opened.borrow().is_empty());
    }

    #[test]
    fn a_closed_tab_cancels_its_fetch_and_tells_the_viewer() {
        let rig = rig(false);
        let sink = install(true);
        fetch(&rig, 3, "https://files.example/doc.pdf", 7);
        let actions = rig.handoff.tab_closed(tab(3));
        assert_eq!(actions, vec![Action::Cancel(DownloadId(7))]);
        assert_eq!(*sink.closed.borrow(), vec![tab(3)]);
        let done = write_pdf(&rig, "doc.pdf");
        rig.handoff.events(vec![finished(7, &done)], &always);
        assert!(!done.exists());
        assert!(sink.opened.borrow().is_empty());
        // Closing a tab with no fetch is quiet.
        assert!(rig.handoff.tab_closed(tab(4)).is_empty());
    }

    #[test]
    fn a_failed_fetch_is_a_note_and_leaves_nothing_behind() {
        let rig = rig(false);
        let _sink = install(true);
        fetch(&rig, 1, "https://files.example/doc.pdf", 7);
        let (rest, actions) = rig.handoff.events(
            vec![DownloadEvent::Failed {
                id: DownloadId(7),
                error: "the site did not send the file again".into(),
                resumable: false,
            }],
            &always,
        );
        assert!(rest.is_empty());
        let [Action::Note { name, message }] = actions.as_slice() else {
            panic!("{actions:?}")
        };
        assert_eq!(name, "doc.pdf");
        assert!(message.contains("the site did not send the file again"));
        assert_eq!(rig.handoff.fetches(), 0);
    }

    #[test]
    fn a_fetch_that_outgrows_the_limit_is_a_note_and_never_reaches_the_viewer() {
        let rig = rig(false);
        let sink = install(true);
        fetch(&rig, 1, "https://files.example/endless.pdf", 7);
        // The download crate stops the body at the limit and says so (its own test is
        // `pdf_fetch::a_pdf_that_never_ends_is_stopped_at_the_limit_and_leaves_no_file`).
        let (rest, actions) = rig.handoff.events(
            vec![DownloadEvent::Failed {
                id: DownloadId(7),
                error: "the file is larger than the 512 MiB limit".into(),
                resumable: false,
            }],
            &always,
        );
        assert!(rest.is_empty(), "never listed as a download");
        let [Action::Note { name, message }] = actions.as_slice() else {
            panic!("{actions:?}")
        };
        assert_eq!(name, "endless.pdf");
        assert_eq!(
            message,
            "The PDF could not be opened: the file is larger than the 512 MiB limit"
        );
        assert!(sink.opened.borrow().is_empty());
        assert_eq!(rig.handoff.fetches(), 0);
    }

    #[test]
    fn a_fetch_that_cannot_start_is_a_note() {
        let rig = rig(false);
        let _sink = install(true);
        let token = token_of(&offered(
            &rig,
            1,
            &offer("https://files.example/doc.pdf"),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        ));
        let actions = rig
            .handoff
            .fetch_failed(token, "The browser engine was not available; try again");
        assert!(matches!(actions.as_slice(), [Action::Note { .. }]));
        assert_eq!(rig.handoff.fetches(), 0);
    }

    // ── Which tab, which navigation ─────────────────────────────────────────────────────

    #[test]
    fn a_background_tab_shows_a_pdf_only_when_the_user_started_it() {
        let rig = rig(false);
        let _sink = install(true);
        let url = "https://files.example/doc.pdf";
        // A page's navigation in a tab in the background: the ordinary rules refuse it.
        assert_eq!(
            offered(
                &rig,
                2,
                &offer(url),
                Some(&NavigationFacts {
                    user_started: false,
                    foreground: false,
                    from: None
                }),
                false,
                PdfAction::OpenInMercury
            ),
            Outcome::Ordinary
        );
        assert_eq!(rig.handoff.fetches(), 0);
        // A Ctrl-click: fetched now, the viewer opens it when the tab is shown.
        let ctrl_click = NavigationFacts {
            user_started: true,
            foreground: false,
            from: Some("https://page.example/".into()),
        };
        let outcome = offered(
            &rig,
            2,
            &offer(url),
            Some(&ctrl_click),
            false,
            PdfAction::OpenInMercury,
        );
        assert!(token_of(&outcome) > 0);
    }

    #[test]
    fn a_forms_post_is_left_to_the_ordinary_rules_which_refuse_it() {
        let rig = rig(false);
        let _sink = install(true);
        let mut posted = offer("https://files.example/doc.pdf");
        posted.posted = Some(true);
        assert_eq!(
            offered(
                &rig,
                1,
                &posted,
                Some(&typed()),
                true,
                PdfAction::OpenInMercury
            ),
            Outcome::Ordinary
        );
        assert_eq!(rig.handoff.fetches(), 0);
    }

    #[test]
    fn blob_and_data_pdfs_are_left_to_the_ordinary_rules() {
        let rig = rig(false);
        let _sink = install(true);
        for address in [
            "blob:https://page.example/1234",
            "data:application/pdf;base64,JVBERi0=",
        ] {
            assert_eq!(
                offered(
                    &rig,
                    1,
                    &offer(address),
                    Some(&typed()),
                    true,
                    PdfAction::OpenInMercury
                ),
                Outcome::Ordinary,
                "{address}"
            );
        }
    }

    #[test]
    fn a_typed_file_pdf_opens_in_place_and_is_never_owned() {
        let rig = rig(false);
        let sink = install(true);
        let file = write_pdf(&rig, "local.pdf");
        let url = Url::from_file_path(&file).unwrap();
        let outcome = offered(
            &rig,
            1,
            &offer(url.as_str()),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        );
        assert_eq!(outcome, Outcome::Handled(Vec::new()));
        let opened = sink.opened.borrow();
        assert_eq!(opened.len(), 1);
        assert_eq!(
            opened[0].2,
            PdfFile {
                path: file.clone(),
                owned: false
            }
        );
        assert_eq!(rig.handoff.fetches(), 0);
        assert!(file.exists());
    }

    #[test]
    fn a_file_pdf_a_page_sent_the_tab_to_is_left_to_the_ordinary_rules() {
        let rig = rig(false);
        let sink = install(true);
        let file = write_pdf(&rig, "local.pdf");
        let url = Url::from_file_path(&file).unwrap();
        let outcome = offered(
            &rig,
            1,
            &offer(url.as_str()),
            Some(&page_started()),
            true,
            PdfAction::OpenInMercury,
        );
        assert_eq!(outcome, Outcome::Ordinary);
        assert!(sink.opened.borrow().is_empty());
    }

    #[test]
    fn a_file_that_is_not_a_pdf_is_not_given_to_the_viewer() {
        let rig = rig(false);
        let sink = install(true);
        let file = rig.folder.path().join("notes.pdf");
        fs::write(&file, b"just text").unwrap();
        let url = Url::from_file_path(&file).unwrap();
        let outcome = offered(
            &rig,
            1,
            &offer(url.as_str()),
            Some(&typed()),
            true,
            PdfAction::OpenInMercury,
        );
        let Outcome::Handled(actions) = outcome else {
            panic!("{outcome:?}")
        };
        assert!(matches!(actions.as_slice(), [Action::Note { .. }]));
        assert!(sink.opened.borrow().is_empty());
        assert!(file.exists());
    }

    // ── Start-up ────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_start_up_sweep_removes_what_an_earlier_run_left_and_nothing_of_this_one() {
        let folder = tempfile::tempdir().unwrap();
        // Whole seconds, so that a file system that keeps no finer time still tells the same
        // answer: the run began at a time it can store exactly.
        let started = SystemTime::UNIX_EPOCH
            + Duration::from_secs(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            );
        let set_modified = |path: &Path, when: SystemTime| {
            let handle = fs::File::options().write(true).open(path).unwrap();
            handle.set_modified(when).unwrap();
        };
        // An earlier run's, from a minute ago and from long ago (a crash left it, however recent).
        let minute_ago = folder.path().join("minute-ago.pdf");
        let long_ago = folder.path().join("long-ago.pdf.part");
        fs::write(&minute_ago, b"%PDF-").unwrap();
        fs::write(&long_ago, b"%PDF-").unwrap();
        set_modified(&minute_ago, started - Duration::from_secs(60));
        set_modified(&long_ago, started - Duration::from_secs(30 * 24 * 60 * 60));
        // This run's, made as the sweep thread starts (the very moment counts as this run's).
        let at_start = folder.path().join("at-start.pdf");
        let later = folder.path().join("later.pdf.part");
        fs::write(&at_start, b"%PDF-").unwrap();
        fs::write(&later, b"%PDF-").unwrap();
        set_modified(&at_start, started);
        set_modified(&later, started + Duration::from_secs(5));
        // Only regular files at the top are touched.
        fs::create_dir(folder.path().join("sub")).unwrap();
        fs::write(folder.path().join("sub").join("inner.pdf"), b"%PDF-").unwrap();

        assert_eq!(sweep(folder.path(), started), 2);
        assert!(!minute_ago.exists() && !long_ago.exists());
        assert!(at_start.exists() && later.exists());
        assert!(folder.path().join("sub").join("inner.pdf").exists());
        // A folder that is not there is nothing to sweep.
        assert_eq!(sweep(&folder.path().join("missing"), started), 0);
    }

    #[test]
    fn the_end_of_the_run_clears_the_private_folder_of_what_nobody_owns() {
        let profile = tempfile::tempdir().unwrap();
        configure(Some(profile.path()), false);
        let dir = profile.path().join("pdf-view");
        // Nothing made yet: nothing to do.
        assert_eq!(clear_private_folder(), 0);
        fs::create_dir_all(dir.join("sub")).unwrap();
        // A fetch that finished as the run ended, and one that was cut off.
        fs::write(dir.join("finished.pdf"), b"%PDF-").unwrap();
        fs::write(dir.join("cut.pdf.part"), b"%PDF-").unwrap();
        fs::write(dir.join("sub").join("inner.pdf"), b"%PDF-").unwrap();

        assert_eq!(clear_private_folder(), 2);
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "only the sub-folder"
        );
        assert!(dir.join("sub").join("inner.pdf").exists());
        assert_eq!(clear_private_folder(), 0);
    }

    #[test]
    fn the_setting_maps_onto_the_crates_own_choice() {
        assert_eq!(
            preference_of(PdfAction::OpenInMercury),
            PdfPreference::OpenInViewer
        );
        assert_eq!(
            preference_of(PdfAction::OpenInSystemViewer),
            PdfPreference::OpenInSystemViewer
        );
        assert_eq!(preference_of(PdfAction::Download), PdfPreference::Download);
    }
}
