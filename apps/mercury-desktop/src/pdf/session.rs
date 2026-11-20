// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! One PDF document in one tab: its helper process (a `PdfClient`), its view model (a
//! `ViewerModel`), and everything between them.
//!
//! A session knows nothing of Slint, the window or the controller, so it is tested with the fake
//! backend and no window. It works like this:
//!
//! - The client delivers its answers on its own thread. They go into an [`Inbox`], which wakes the
//!   UI thread (the `wake` function the owner supplies: in the application it posts to Slint's
//!   event loop); the UI thread calls [`Session::pump`], which takes the answers out and feeds the
//!   model.
//! - After every change (an answer, a scroll, a zoom) the model's plan is carried out: cancels
//!   first, then requests, then page text and links ([`Session::replan`]).
//! - A background tab's document is not opened until its tab is shown ([`Phase::Idle`]), and a
//!   shown document's helper may be ended to stay within [`MAX_LIVE_HELPERS`]
//!   ([`Sessions::limit_live`]); the model keeps the scroll position, the zoom and the rotation,
//!   and showing the tab again starts a new helper.
//! - A helper that is lost is not restarted by itself ([`Phase::Failed`]): the viewer says so and
//!   the user decides ("Try again" is [`Session::retry`]).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mercury_pdf::client::{ClientEvent, PdfClient, RequestId, WorkerLaunch};
use mercury_pdf::error::PdfError;
use mercury_pdf::find::FindProgress;
use mercury_pdf::types::{DocumentInfo, FindOptions, LinkTarget, Rotation, TileRequest};
use mercury_pdf::viewer::{Fit, ViewerModel, Viewport};
use talaria_core::TabId;
use url::Url;
use zeroize::Zeroizing;

use super::input::ScrollMove;

/// The most helper processes alive at once; the document of the tab shown least recently ends
/// its helper when another would exceed this.
pub(super) const MAX_LIVE_HELPERS: usize = 4;

/// The longest title the toolbar and the tab strip take from a document.
const MAX_TITLE_CHARS: usize = 200;

/// Where a document is.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Phase {
    /// No helper: the tab has not been shown yet, or its helper was ended.
    Idle,
    /// The helper is opening the file.
    Opening,
    /// The file is encrypted; `wrong` says the last password was refused.
    NeedsPassword {
        wrong: bool,
    },
    Ready,
    /// The file could not be shown, or the helper was lost. Nothing is retried by itself.
    Failed(PdfError),
}

/// The file to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DocumentFile {
    pub(super) path: PathBuf,
    /// A private cache copy, deleted when the session ends; `false` for the user's own file.
    pub(super) owned: bool,
}

/// How a helper is started: the real one runs this program with `--pdf-worker`; tests run the
/// fake backend in the same process.
pub(super) trait ClientFactory {
    fn start(&self, events: Box<dyn Fn(ClientEvent) + Send + Sync>) -> Result<PdfClient, PdfError>;
}

/// A factory shared by several owners (tests keep a handle to count the starts).
impl<T: ClientFactory + ?Sized> ClientFactory for std::rc::Rc<T> {
    fn start(&self, events: Box<dyn Fn(ClientEvent) + Send + Sync>) -> Result<PdfClient, PdfError> {
        (**self).start(events)
    }
}

/// The helper process of this browser, with the PDFium library at `library`.
pub(super) struct WorkerFactory {
    pub(super) library: Option<PathBuf>,
}

impl ClientFactory for WorkerFactory {
    fn start(&self, events: Box<dyn Fn(ClientEvent) + Send + Sync>) -> Result<PdfClient, PdfError> {
        let launch = WorkerLaunch::current_exe(self.library.clone()).map_err(|error| {
            PdfError::Io(format!("could not find the browser's own program: {error}"))
        })?;
        PdfClient::start(launch, events)
    }
}

/// What a client delivered and the UI thread has not taken yet. One per client.
pub(super) struct Inbox {
    events: Mutex<VecDeque<ClientEvent>>,
    /// A wake-up is on its way: one per batch, not one per event.
    scheduled: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inbox {
    pub(super) fn new(wake: Box<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(VecDeque::new()),
            scheduled: AtomicBool::new(false),
            wake,
        })
    }

    /// From the client's thread.
    fn push(&self, event: ClientEvent) {
        lock(&self.events).push_back(event);
        if !self.scheduled.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }

    /// From the UI thread: everything that arrived, in order.
    fn take(&self) -> Vec<ClientEvent> {
        self.scheduled.store(false, Ordering::Release);
        lock(&self.events).drain(..).collect()
    }
}

/// What a pump did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pumped {
    /// Something arrived: the window may need redrawing.
    pub(super) changed: bool,
    /// The document has just opened.
    pub(super) became_ready: bool,
}

/// A tile request as a map key (the request has floating-point fields).
type RequestKey = (u32, u32, u16, u32, u32, u32, u32);

fn request_key(request: &TileRequest) -> RequestKey {
    (
        request.page,
        request.scale.to_bits(),
        request.rotation.degrees(),
        request.x,
        request.y,
        request.width,
        request.height,
    )
}

pub(super) struct Session {
    pub(super) tab: TabId,
    /// The address the tab is on while the document shows. The tab leaving it ends the session.
    pub(super) url: Url,
    file: DocumentFile,
    phase: Phase,
    client: Option<PdfClient>,
    inbox: Option<Arc<Inbox>>,
    /// Counts started clients, so that a wake-up for an ended one is recognised.
    generation: u64,
    wake: Arc<dyn Fn(TabId, u64) + Send + Sync>,
    model: Option<ViewerModel>,
    requests: HashMap<RequestKey, RequestId>,
    /// The page area and the tab's zoom, which the model takes when it exists.
    viewport: Viewport,
    zoom_percent: u16,
    display_title: String,
    /// `#page=N` of the address: the page to open on (counted from 0).
    initial_page: Option<u32>,
    /// A find asked for while no document was open: started when it opens.
    pending_find: Option<(String, FindOptions)>,
    /// The helper's id of the find that is running.
    client_find: Option<u64>,
    /// When the tab was last shown, from `Sessions::touch`.
    last_shown: u64,
}

impl Session {
    pub(super) fn new(
        tab: TabId,
        url: Url,
        file: DocumentFile,
        wake: Arc<dyn Fn(TabId, u64) + Send + Sync>,
    ) -> Self {
        let display_title = display_title(None, &url);
        let initial_page = page_from_fragment(&url);
        Self {
            tab,
            url,
            file,
            phase: Phase::Idle,
            client: None,
            inbox: None,
            generation: 0,
            wake,
            model: None,
            requests: HashMap::new(),
            viewport: Viewport::default(),
            zoom_percent: 100,
            display_title,
            initial_page,
            pending_find: None,
            client_find: None,
            last_shown: 0,
        }
    }

    pub(super) fn phase(&self) -> &Phase {
        &self.phase
    }

    pub(super) fn model(&self) -> Option<&ViewerModel> {
        self.model.as_ref()
    }

    pub(super) fn file(&self) -> &DocumentFile {
        &self.file
    }

    /// The document's title, or the file's name: for the tab strip, the window and the toolbar.
    pub(super) fn display_title(&self) -> &str {
        &self.display_title
    }

    /// Whether a helper is alive for this document.
    pub(super) fn is_live(&self) -> bool {
        self.client.is_some()
    }

    #[cfg(test)]
    pub(super) fn is_ready(&self) -> bool {
        self.phase == Phase::Ready
    }

    // ----- the helper -----------------------------------------------------------------------

    /// Starts a helper and has it open the file, if none runs and none failed.
    pub(super) fn ensure_started(&mut self, factory: &dyn ClientFactory) {
        if self.phase == Phase::Idle && self.client.is_none() {
            self.start(factory, None);
        }
    }

    fn start(&mut self, factory: &dyn ClientFactory, password: Option<&Zeroizing<String>>) {
        self.end_helper();
        self.generation += 1;
        let wake = Arc::clone(&self.wake);
        let (tab, generation) = (self.tab, self.generation);
        let inbox = Inbox::new(Box::new(move || wake(tab, generation)));
        let sink = Arc::clone(&inbox);
        match factory.start(Box::new(move |event| sink.push(event))) {
            Ok(client) => {
                client.open(&self.file.path, password.map(|password| password.as_str()));
                self.client = Some(client);
                self.inbox = Some(inbox);
                self.phase = Phase::Opening;
            }
            Err(error) => self.phase = Phase::Failed(error),
        }
    }

    /// Ends the helper (the client's drop kills and reaps it) and forgets what was asked of it.
    fn end_helper(&mut self) {
        self.client = None;
        self.inbox = None;
        self.requests.clear();
        self.client_find = None;
    }

    /// Ends the helper to free its process; the model keeps the view, and showing the tab again
    /// opens the file again.
    pub(super) fn suspend(&mut self) {
        if self.client.is_none() {
            return;
        }
        self.end_helper();
        self.phase = Phase::Idle;
    }

    /// "Try again" after a failure: a new helper, the same file.
    pub(super) fn retry(&mut self, factory: &dyn ClientFactory) {
        if matches!(self.phase, Phase::Failed(_)) {
            self.start(factory, None);
        }
    }

    /// The password the user typed (cleared when it has been sent).
    pub(super) fn submit_password(
        &mut self,
        factory: &dyn ClientFactory,
        password: Zeroizing<String>,
    ) {
        if !matches!(self.phase, Phase::NeedsPassword { .. }) {
            return;
        }
        match self.client.as_ref() {
            Some(client) => {
                client.open(&self.file.path, Some(password.as_str()));
                self.phase = Phase::Opening;
            }
            None => self.start(factory, Some(&password)),
        }
    }

    /// Ends the session: the helper and, for a private copy, its file.
    pub(super) fn close(&mut self) {
        self.end_helper();
        if self.file.owned
            && let Err(error) = std::fs::remove_file(&self.file.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("could not remove a private PDF copy: {error}");
        }
        self.file.owned = false;
    }

    // ----- answers --------------------------------------------------------------------------

    /// Takes what the helper has answered and applies it.
    pub(super) fn pump(&mut self) -> Pumped {
        let mut pumped = Pumped::default();
        let Some(inbox) = self.inbox.clone() else {
            return pumped;
        };
        for event in inbox.take() {
            pumped.changed = true;
            self.apply(event, &mut pumped);
            if self.inbox.is_none() {
                // The helper was lost: nothing more is expected from it.
                break;
            }
        }
        if pumped.changed {
            self.replan();
        }
        pumped
    }

    fn apply(&mut self, event: ClientEvent, pumped: &mut Pumped) {
        match event {
            ClientEvent::Opened(Ok(info)) => self.opened(info, pumped),
            ClientEvent::Opened(Err(error)) => match error {
                PdfError::PasswordRequired => self.phase = Phase::NeedsPassword { wrong: false },
                PdfError::PasswordIncorrect => self.phase = Phase::NeedsPassword { wrong: true },
                error => {
                    self.end_helper();
                    self.phase = Phase::Failed(error);
                }
            },
            ClientEvent::Tile {
                id,
                request,
                result,
            } => {
                let key = request_key(&request);
                if self.requests.get(&key) == Some(&id) {
                    self.requests.remove(&key);
                }
                let Some(model) = self.model.as_mut() else {
                    return;
                };
                match result {
                    Ok(tile) => model.tile_arrived(&request, tile),
                    // Cancelled by us: not a failure, and the tile may be wanted again.
                    Err(PdfError::Cancelled) => {}
                    Err(_) => model.tile_failed(&request),
                }
            }
            ClientEvent::PageText { page, result } => {
                if let Some(model) = self.model.as_mut() {
                    model.set_page_text(page, result.unwrap_or_default());
                }
            }
            ClientEvent::Links { page, result } => {
                if let Some(model) = self.model.as_mut() {
                    model.set_links(page, result.unwrap_or_default());
                }
            }
            ClientEvent::Outline(_) => {}
            ClientEvent::Found {
                find,
                page,
                hits,
                finished,
            } => {
                if let Some(model) = self.model.as_mut() {
                    model.find_hits(find, page, hits, finished);
                }
            }
            ClientEvent::WorkerLost(error) => {
                self.end_helper();
                self.phase = Phase::Failed(error);
            }
        }
    }

    fn opened(&mut self, info: DocumentInfo, pumped: &mut Pumped) {
        match self.model.as_mut() {
            // The helper came back after it was ended: the model keeps the view.
            Some(model) => {
                model.forget_pending();
                model.set_viewport(self.viewport);
                model.set_zoom_percent(self.zoom_percent);
            }
            None => {
                let mut model = ViewerModel::new(info);
                model.set_viewport(self.viewport);
                model.set_zoom_percent(self.zoom_percent);
                // A new model starts in a default viewport; changing it keeps the point at the
                // centre where it was, which would open the document scrolled. A document opens
                // at its top.
                model.set_scroll(0.0, 0.0);
                if let Some(page) = self.initial_page.take() {
                    model.go_to_page(page, None);
                }
                self.display_title = display_title(model.title(), &self.url);
                self.model = Some(model);
            }
        }
        self.requests.clear();
        self.phase = Phase::Ready;
        pumped.became_ready = true;
        // A search that was on when the helper ended runs again, as a search asked for before
        // the document opened does.
        let again = self
            .model
            .as_ref()
            .filter(|model| model.find_is_active())
            .map(|model| (model.find_query().to_owned(), model.find_options()));
        let wanted = self.pending_find.take().or(again);
        if let Some((query, options)) = wanted.filter(|(query, _)| !query.is_empty()) {
            self.find(&query, options);
        }
    }

    /// Carries out what the model wants next: cancels, tiles, page text, links.
    fn replan(&mut self) {
        if self.phase != Phase::Ready {
            return;
        }
        let (Some(model), Some(client)) = (self.model.as_mut(), self.client.as_ref()) else {
            return;
        };
        let plan = model.plan();
        for cancelled in &plan.cancel {
            if let Some(id) = self.requests.remove(&request_key(cancelled)) {
                client.cancel_tile(id);
            }
        }
        for (request, priority) in plan.requests {
            let id = client.request_tile(request, priority);
            self.requests.insert(request_key(&request), id);
        }
        for page in plan.page_text {
            client.request_page_text(page);
        }
        for page in plan.links {
            client.request_links(page);
        }
    }

    // ----- the view -------------------------------------------------------------------------

    /// The page area's size and the display's pixel ratio.
    pub(super) fn set_viewport(&mut self, viewport: Viewport) {
        if viewport == self.viewport {
            return;
        }
        self.viewport = viewport;
        if let Some(model) = self.model.as_mut() {
            model.set_viewport(viewport);
        }
        self.replan();
    }

    /// The tab's zoom, which is the viewer's zoom.
    pub(super) fn set_zoom_percent(&mut self, percent: u16) {
        if percent == self.zoom_percent {
            return;
        }
        self.zoom_percent = percent;
        if let Some(model) = self.model.as_mut() {
            model.set_zoom_percent(percent);
        }
        self.replan();
    }

    pub(super) fn zoom_percent(&self) -> u16 {
        self.zoom_percent
    }

    /// "Fit width" and "Fit page": what 100 % means. The caller also puts the tab's zoom at 100.
    pub(super) fn set_fit(&mut self, fit: Fit) {
        if let Some(model) = self.model.as_mut() {
            model.set_fit(fit);
        }
        self.replan();
    }

    pub(super) fn rotate(&mut self) {
        if let Some(model) = self.model.as_mut() {
            let rotation: Rotation = model.rotation().turned_clockwise();
            model.set_rotation(rotation);
        }
        self.replan();
    }

    pub(super) fn scroll(&mut self, movement: ScrollMove) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        match movement {
            ScrollMove::By(dx, dy) => {
                model.set_scroll(model.scroll_x() + dx, model.scroll_y() + dy);
            }
            ScrollMove::Top => model.set_scroll_y(0.0),
            ScrollMove::Bottom => model.set_scroll_y(f32::MAX),
        }
        self.replan();
    }

    /// The page asked for in the page field: counted from 1, kept within the document.
    pub(super) fn go_to_page_number(&mut self, number: u32) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        model.go_to_page(number.saturating_sub(1), None);
        self.replan();
    }

    /// A link inside the document.
    pub(super) fn go_to_page(&mut self, page: u32, y: Option<f32>) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        model.go_to_page(page, y);
        self.replan();
    }

    // ----- pointer and text -----------------------------------------------------------------

    /// A press at a point of the page area (the scroll position is added here).
    pub(super) fn pointer_down(&mut self, x: f32, y: f32, clicks: u8) {
        if let Some(model) = self.model.as_mut() {
            let (x, y) = (x + model.scroll_x(), y + model.scroll_y());
            model.pointer_down(x, y, clicks);
        }
        self.replan();
    }

    pub(super) fn pointer_drag(&mut self, x: f32, y: f32) {
        if let Some(model) = self.model.as_mut() {
            let (x, y) = (x + model.scroll_x(), y + model.scroll_y());
            model.pointer_drag(x, y);
        }
        self.replan();
    }

    pub(super) fn pointer_up(&mut self) {
        if let Some(model) = self.model.as_mut() {
            model.pointer_up();
        }
    }

    pub(super) fn clear_selection(&mut self) {
        if let Some(model) = self.model.as_mut() {
            model.clear_selection();
        }
    }

    /// The selected text, for Copy.
    pub(super) fn selected_text(&self) -> Option<String> {
        self.model.as_ref()?.selected_text()
    }

    /// The link under a point of the page area.
    pub(super) fn link_at(&self, x: f32, y: f32) -> Option<LinkTarget> {
        let model = self.model.as_ref()?;
        model.link_at(x + model.scroll_x(), y + model.scroll_y())
    }

    // ----- find -----------------------------------------------------------------------------

    /// Searches the document for `query`, replacing any search; an empty query only clears.
    pub(super) fn find(&mut self, query: &str, options: FindOptions) {
        self.cancel_find();
        let Some(model) = self.model.as_mut() else {
            // Not open yet: asked again when it is.
            self.pending_find = (!query.is_empty()).then(|| (query.to_owned(), options));
            return;
        };
        model.find_start(query, options);
        if query.is_empty() {
            return;
        }
        if let Some(client) = self.client.as_ref() {
            let id = client.find(query, options, model.current_page());
            model.find_bind(id);
            self.client_find = Some(id);
        }
    }

    /// The next (or previous) hit, scrolled into view; false when there is none.
    pub(super) fn find_step(&mut self, forward: bool) -> bool {
        let Some(model) = self.model.as_mut() else {
            return false;
        };
        let moved = if forward {
            model.find_next()
        } else {
            model.find_previous()
        };
        self.replan();
        moved
    }

    /// Ends the search and takes the highlights away.
    pub(super) fn find_clear(&mut self) {
        self.cancel_find();
        self.pending_find = None;
        if let Some(model) = self.model.as_mut() {
            model.find_clear();
        }
    }

    fn cancel_find(&mut self) {
        if let (Some(id), Some(client)) = (self.client_find.take(), self.client.as_ref()) {
            client.cancel_find(id);
        }
    }

    pub(super) fn find_progress(&self) -> Option<FindProgress> {
        let model = self.model.as_ref()?;
        model.find_is_active().then(|| model.find_status())
    }

    /// The text of the search that is on, if one is.
    pub(super) fn find_query(&self) -> Option<&str> {
        let model = self.model.as_ref()?;
        model.find_is_active().then(|| model.find_query())
    }

    pub(super) fn find_options(&self) -> Option<FindOptions> {
        let model = self.model.as_ref()?;
        model.find_is_active().then(|| model.find_options())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Ending the session by dropping it must not leave a private copy behind.
        self.close();
    }
}

/// The sessions of a window, by tab, with the order the tabs were shown in.
#[derive(Default)]
pub(super) struct Sessions {
    sessions: HashMap<TabId, Session>,
    clock: u64,
}

impl Sessions {
    /// Adds the session of a tab. A session the tab already had ends (its private copy goes).
    pub(super) fn insert(&mut self, session: Session) {
        self.sessions.insert(session.tab, session);
    }

    pub(super) fn get(&self, tab: TabId) -> Option<&Session> {
        self.sessions.get(&tab)
    }

    pub(super) fn get_mut(&mut self, tab: TabId) -> Option<&mut Session> {
        self.sessions.get_mut(&tab)
    }

    /// Ends and forgets the session of a tab.
    pub(super) fn remove(&mut self, tab: TabId) -> bool {
        // Dropping the session closes it.
        self.sessions.remove(&tab).is_some()
    }

    pub(super) fn tabs(&self) -> Vec<TabId> {
        self.sessions.keys().copied().collect()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Notes that the tab is being shown now.
    pub(super) fn touch(&mut self, tab: TabId) {
        self.clock += 1;
        let clock = self.clock;
        if let Some(session) = self.sessions.get_mut(&tab) {
            session.last_shown = clock;
        }
    }

    /// How many helpers are alive.
    pub(super) fn live_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.is_live())
            .count()
    }

    /// Ends helpers, the one whose tab was shown longest ago first and never `keep`'s, until at
    /// most `limit` are alive. Returns the tabs whose documents were suspended.
    pub(super) fn limit_live(&mut self, keep: TabId, limit: usize) -> Vec<TabId> {
        let mut suspended = Vec::new();
        while self.live_count() > limit {
            let oldest = self
                .sessions
                .values()
                .filter(|session| session.is_live() && session.tab != keep)
                .min_by_key(|session| session.last_shown)
                .map(|session| session.tab);
            let Some(tab) = oldest else {
                break;
            };
            if let Some(session) = self.sessions.get_mut(&tab) {
                session.suspend();
            }
            suspended.push(tab);
        }
        suspended
    }
}

/// The title shown for a document: its own, cleaned of anything that is not text, else the
/// file's name from the address, else "PDF document".
pub(super) fn display_title(title: Option<&str>, url: &Url) -> String {
    if let Some(title) = title {
        let cleaned = clean_text(title);
        if !cleaned.is_empty() {
            return cleaned;
        }
    }
    file_name(url).unwrap_or_else(|| "PDF document".to_owned())
}

/// Text from a document, for the interface: control characters become spaces, runs of white
/// space one space, and the length is bounded.
pub(super) fn clean_text(text: &str) -> String {
    let mut cleaned = String::new();
    let mut previous_space = true;
    for character in text.chars() {
        if is_invisible_format(character) {
            continue;
        }
        let character = if character.is_control() || character.is_whitespace() {
            ' '
        } else {
            character
        };
        if character == ' ' {
            if previous_space {
                continue;
            }
            previous_space = true;
        } else {
            previous_space = false;
        }
        cleaned.push(character);
        if cleaned.chars().count() >= MAX_TITLE_CHARS {
            break;
        }
    }
    cleaned.trim().to_owned()
}

/// Zero-width and direction-changing characters, which a title could use to pass as another one.
fn is_invisible_format(character: char) -> bool {
    matches!(
        character,
        '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}'
    )
}

/// The last part of the address's path, with percent-escapes undone; `None` when there is none.
fn file_name(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.rfind(|segment| !segment.is_empty())?;
    let decoded = percent_decode(segment);
    let cleaned = clean_text(&decoded);
    (!cleaned.is_empty()).then_some(cleaned)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(pair) = bytes.get(index + 1..index + 3)
            && pair.iter().all(u8::is_ascii_hexdigit)
            && let Ok(pair) = std::str::from_utf8(pair)
            && let Ok(value) = u8::from_str_radix(pair, 16)
        {
            decoded.push(value);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// `#page=N` (counted from 1) of an address, as a page index counted from 0.
fn page_from_fragment(url: &Url) -> Option<u32> {
    let fragment = url.fragment()?;
    fragment
        .split('&')
        .find_map(|part| part.strip_prefix("page="))
        .and_then(|number| number.parse::<u32>().ok())
        .filter(|number| *number >= 1)
        .map(|number| number - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a URL")
    }

    #[test]
    fn a_title_is_cleaned_and_the_file_name_stands_in_when_there_is_none() {
        let address = url("https://example.com/files/Annual%20report%20%282026%29.pdf?x=1");
        assert_eq!(
            display_title(Some("  Annual\u{0}  report\n2026 "), &address),
            "Annual report 2026"
        );
        assert_eq!(display_title(None, &address), "Annual report (2026).pdf");
        assert_eq!(
            display_title(Some(" \n\t"), &address),
            "Annual report (2026).pdf"
        );
        assert_eq!(
            display_title(None, &url("https://example.com/")),
            "PDF document"
        );
        assert_eq!(
            display_title(None, &url("https://example.com/a/b/")),
            "b",
            "a trailing slash does not hide the name"
        );
    }

    #[test]
    fn a_title_is_bounded() {
        let long = "x".repeat(5_000);
        assert_eq!(clean_text(&long).chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn a_bad_percent_escape_is_left_as_it_is() {
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
        assert_eq!(percent_decode("%e2%82%ac"), "\u{20ac}");
    }

    #[test]
    fn the_page_parameter_of_the_address_picks_the_first_page() {
        assert_eq!(
            page_from_fragment(&url("https://a.test/x.pdf#page=3")),
            Some(2)
        );
        assert_eq!(
            page_from_fragment(&url("https://a.test/x.pdf#view=Fit&page=12")),
            Some(11)
        );
        assert_eq!(
            page_from_fragment(&url("https://a.test/x.pdf#page=0")),
            None
        );
        assert_eq!(
            page_from_fragment(&url("https://a.test/x.pdf#page=x")),
            None
        );
        assert_eq!(page_from_fragment(&url("https://a.test/x.pdf")), None);
    }
}
