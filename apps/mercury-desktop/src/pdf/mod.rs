// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The PDF viewer in the browser window (pdf-viewer lane).
//!
//! A PDF the engine cannot show goes through the downloads hand-off
//! (`downloads/pdf_handoff.rs`), which fetches it into a private file and offers it to
//! [`PdfViewerSink`] (here: [`Sink`]). The viewer is then an overlay on the tab's page area
//! (`ui/pdf/pdf-viewer.slint`), drawn by Mercury; the tab keeps the PDF's address and the engine's
//! page below the overlay stays hidden.
//!
//! - `session.rs`: one document in one tab: its helper process, its view model, its state. No
//!   Slint, no window: tested with the fake backend.
//! - `view.rs`: from the view model to the window's rows.
//! - `input.rs`: the pointer, the wheel, the keys and links, as small pure pieces.
//! - `copy.rs`, `actions.rs`: the clipboard and the two ways out (save a copy, open in another
//!   app), behind traits so that tests touch neither the user's clipboard nor their files.
//! - `find.rs`: the words the find bar says.
//!
//! This file ties them to the window: the Slint callbacks, the sink the hand-off calls, and the
//! hooks other modules call (`sync_window` from `ui_sync`, the tab title, the find bar, Edit >
//! Copy). Everything runs on the UI thread. No `RefCell` here is held while a Slint setter or a
//! controller call runs, because those may call back into this module.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mercury_pdf::error::PdfError;
use mercury_pdf::types::FindOptions;
use mercury_pdf::viewer::{Fit, Viewport};
use mercury_ui::{MainWindow, PdfPointer, PdfScrollKey, PdfViewerModel};
use slint::ComponentHandle;
use talaria_core::{BrowserController, NavigationOrigin, TabId};
use url::Url;
use zeroize::Zeroizing;

use self::actions::{ExternalActions, SystemActions};
use self::copy::{ClipboardSink, clipboard_for_run};
use self::input::{
    LinkAction, PointerTracker, ScrollMove, link_action, nearest_zoom_step, scroll_move,
    wheel_scroll,
};
use self::session::{
    ClientFactory, DocumentFile, MAX_LIVE_HELPERS, Phase, Session, Sessions, WorkerFactory,
};
use self::view::{Facts, FitButton, ViewCache, ViewState};
use crate::downloads::pdf_handoff::{PdfFile, PdfViewerSink, install_viewer};
use crate::ui_sync::sync_ui;

mod actions;
mod copy;
mod find;
mod input;
mod session;
#[cfg(test)]
mod tests;
mod view;

/// The height of the viewer's toolbar (`ui/pdf/pdf-toolbar.slint`); the pages are below it.
const TOOLBAR_HEIGHT: f32 = 40.0;

/// The tab zoom at which "fit width" and "fit page" mean what they say.
const FIT_ZOOM_PERCENT: u16 = 100;

thread_local! {
    static PDF: RefCell<Option<Rc<Pdf>>> = const { RefCell::new(None) };
}

fn current() -> Option<Rc<Pdf>> {
    PDF.with(|pdf| pdf.borrow().clone())
}

/// `--pdf-page` and `--pdf-find`: diagnostics for runs with `--screenshot`, which cannot press
/// keys. Applied once, to the first document that opens in the active tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Diagnostics {
    /// Scroll to this page (counted from 1).
    pub(crate) page: Option<u32>,
    /// Open the find bar and search for this text.
    pub(crate) find: Option<String>,
}

/// What `install` needs to know about the run.
pub(crate) struct Options {
    /// A `--screenshot` or `--background` run: it never touches the user's clipboard, downloads
    /// folder or other apps.
    pub(crate) automated: bool,
    pub(crate) diagnostics: Diagnostics,
}

/// Sets the viewer up: registers it with the PDF hand-off, connects the window's viewer
/// callbacks, and removes old copies handed to other apps. Call once, after the window and the
/// controller exist and before the first page can load.
pub(crate) fn install(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    web_view_focused: Rc<Cell<bool>>,
    options: Options,
) {
    let library = mercury_pdfium::locate_library();
    let pdf = Rc::new(Pdf::new(
        window.as_weak(),
        Rc::downgrade(&controller),
        web_view_focused,
        Tools {
            factory: Box::new(WorkerFactory {
                library: library.clone(),
            }),
            clipboard: clipboard_for_run(options.automated),
            actions: Box::new(SystemActions {
                automated: options.automated,
            }),
            library_found: library.is_some(),
        },
        options.diagnostics,
    ));
    if library.is_none() {
        log::info!("no PDFium library was found: PDFs are not shown in the viewer");
    }
    PDF.with(|current| *current.borrow_mut() = Some(pdf));
    install_viewer(Rc::new(Sink));
    connect_callbacks(window);
    actions::sweep_old_copies();
}

/// The end of the run, called by `post_run.rs` (which also runs inside AppKit's terminate callback
/// at log-out, so this starts no event loop and touches no Slint state): ends every document,
/// which stops its helper and deletes its private copy, and then clears what is left in the
/// private folder (a fetch that finished as the run ended, a cut-off partial file). A copy the
/// user opened in another app stays: that app may still be reading it, and a later start removes
/// it once it is a day old. Safe to call when the viewer was never installed.
pub(crate) fn release() {
    let pdf = PDF.with(|pdf| pdf.try_borrow_mut().ok().and_then(|mut pdf| pdf.take()));
    if let Some(pdf) = pdf {
        pdf.end_all();
    }
    crate::downloads::pdf_handoff::clear_private_folder();
}

/// What the PDF hand-off calls.
struct Sink;

impl PdfViewerSink for Sink {
    fn is_available(&self) -> bool {
        current().is_some_and(|pdf| pdf.library_found)
    }

    fn open(&self, tab: TabId, url: &Url, file: PdfFile) {
        match current() {
            Some(pdf) => pdf.open(tab, url, file),
            None => {
                // No viewer after all: a private copy is nobody's but ours to delete.
                if file.owned {
                    let _removed = std::fs::remove_file(&file.path);
                }
            }
        }
    }

    fn tab_closed(&self, tab: TabId) {
        if let Some(pdf) = current() {
            pdf.close_tab(tab);
        }
    }
}

struct Pdf {
    window: slint::Weak<MainWindow>,
    controller: Weak<RefCell<BrowserController>>,
    /// Whether the page has the keyboard, which the find bar's Cmd+F moves away.
    web_view_focused: Rc<Cell<bool>>,
    factory: Box<dyn ClientFactory>,
    clipboard: Box<dyn ClipboardSink>,
    actions: Box<dyn ExternalActions>,
    /// The PDFium library was found.
    library_found: bool,
    /// Wakes the UI thread when a helper has answered: `(tab, generation of the client)`.
    wake: Arc<dyn Fn(TabId, u64) + Send + Sync>,
    sessions: RefCell<Sessions>,
    cache: RefCell<ViewCache>,
    pointer: RefCell<PointerTracker>,
    /// The tab whose viewer is up in the window.
    shown: Cell<Option<TabId>>,
    /// The page area's size, as the window reported it.
    area: Cell<Option<(f32, f32)>>,
    find_open: Cell<bool>,
    over_link: Cell<bool>,
    diagnostics: RefCell<Diagnostics>,
}

/// What the viewer uses outside the window: how helpers start, the clipboard, the buttons'
/// actions, and whether PDFium was found. Fakes in tests.
struct Tools {
    factory: Box<dyn ClientFactory>,
    clipboard: Box<dyn ClipboardSink>,
    actions: Box<dyn ExternalActions>,
    library_found: bool,
}

impl Pdf {
    fn new(
        window: slint::Weak<MainWindow>,
        controller: Weak<RefCell<BrowserController>>,
        web_view_focused: Rc<Cell<bool>>,
        tools: Tools,
        diagnostics: Diagnostics,
    ) -> Self {
        Self {
            window,
            controller,
            web_view_focused,
            factory: tools.factory,
            clipboard: tools.clipboard,
            actions: tools.actions,
            library_found: tools.library_found,
            wake: Arc::new(|tab, _generation| {
                let _queued = slint::invoke_from_event_loop(move || {
                    let _slint = crate::callback_scope::InSlintCallback::enter();
                    if let Some(pdf) = current() {
                        pdf.pump(tab);
                    }
                });
            }),
            sessions: RefCell::new(Sessions::default()),
            cache: RefCell::new(ViewCache::default()),
            pointer: RefCell::new(PointerTracker::default()),
            shown: Cell::new(None),
            area: Cell::new(None),
            find_open: Cell::new(false),
            over_link: Cell::new(false),
            diagnostics: RefCell::new(diagnostics),
        }
    }

    // ----- from the hand-off ------------------------------------------------------------------

    fn open(&self, tab: TabId, url: &Url, file: PdfFile) {
        let session = Session::new(
            tab,
            url.clone(),
            DocumentFile {
                path: file.path,
                owned: file.owned,
            },
            Arc::clone(&self.wake),
        );
        // A document the tab already had ends here, its private copy with it.
        self.sessions.borrow_mut().insert(session);
        self.schedule_sync();
    }

    fn close_tab(&self, tab: TabId) {
        let removed = self.sessions.borrow_mut().remove(tab);
        if removed {
            self.schedule_sync();
        }
    }

    /// Ends every document: each helper stops and each private copy is deleted. Only for the end
    /// of the run (`release`): the window is not synced afterwards.
    fn end_all(&self) {
        // Taken out of the cell first: dropping a session ends its helper, which takes a moment.
        let sessions = match self.sessions.try_borrow_mut() {
            Ok(mut sessions) => std::mem::take(&mut *sessions),
            Err(_) => {
                log::warn!(
                    "the PDF documents were busy at the end of the run; the next start clears their copies"
                );
                return;
            }
        };
        drop(sessions);
    }

    /// Has the window sync itself soon: a document arrived or went, and the sync decides what
    /// the page area shows. A timer, because the hand-off may call while the controller is
    /// borrowed.
    fn schedule_sync(&self) {
        let window = self.window.clone();
        let controller = self.controller.clone();
        slint::Timer::single_shot(Duration::ZERO, move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let (Some(window), Some(controller)) = (window.upgrade(), controller.upgrade())
                && let Ok(controller) = controller.try_borrow()
            {
                sync_ui(&window, &controller);
                window.window().request_redraw();
            }
        });
    }

    // ----- the window ---------------------------------------------------------------------------

    /// Decides what the page area shows: ends the documents whose tab moved on, and puts the
    /// active tab's document (if it has one) in front of the engine's page.
    fn sync(&self, window: &MainWindow, controller: &BrowserController) {
        if self.sessions.borrow().is_empty() && self.shown.get().is_none() {
            return;
        }
        self.end_documents_that_moved_on(controller);
        let active = controller.active_tab_id();
        if self.sessions.borrow().get(active).is_none() {
            if self.shown.take().is_some() {
                window.set_pdf_viewer_shown(false);
                window.global::<PdfViewerModel>().set_text_focused(false);
                self.over_link.set(false);
                self.pointer.borrow_mut().cancel();
            }
            return;
        }
        let state = controller.active_state();
        let newly_shown = self.shown.replace(Some(active)) != Some(active);
        self.find_open.set(state.find().is_open());
        if newly_shown {
            // Set before the viewer exists: it reads this when it starts.
            self.set_take_focus(window);
        }
        window.set_pdf_viewer_shown(true);
        window.set_web_view_visible(false);
        let viewport = self.viewport_for(window);
        let title = {
            let mut sessions = self.sessions.borrow_mut();
            sessions.touch(active);
            if let Some(session) = sessions.get_mut(active) {
                session.set_viewport(viewport);
                session.set_zoom_percent(state.zoom_percent());
                session.ensure_started(&*self.factory);
            }
            for tab in sessions.limit_live(active, MAX_LIVE_HELPERS) {
                log::info!("tab {}: its PDF helper ended to save memory", tab.value());
            }
            sessions
                .get(active)
                .map(|session| session.display_title().to_owned())
        };
        if let Some(title) = title {
            window.set_page_title(title.as_str().into());
        }
        self.refresh_in(window);
    }

    /// A document ends when its tab is gone or shows another address (the fragment, which a
    /// PDF's own links may change, does not count).
    fn end_documents_that_moved_on(&self, controller: &BrowserController) {
        let mut sessions = self.sessions.borrow_mut();
        for tab in sessions.tabs() {
            let stays = sessions.get(tab).is_some_and(|session| {
                controller
                    .tabs()
                    .find(|candidate| candidate.id() == tab)
                    .is_some_and(|candidate| same_document(candidate.state().url(), &session.url))
            });
            if !stays {
                sessions.remove(tab);
            }
        }
    }

    /// The page area as the model wants it: its size, and the display's pixel ratio.
    fn viewport_for(&self, window: &MainWindow) -> Viewport {
        let (width, height) = self.area.get().unwrap_or_else(|| {
            // Before the viewer reports its size: the whole page area less the toolbar.
            (
                window.get_web_view_width(),
                (window.get_web_view_height() - TOOLBAR_HEIGHT).max(0.0),
            )
        });
        Viewport {
            width,
            height,
            scale_factor: window.window().scale_factor(),
        }
    }

    /// Whether the viewer should take the keyboard when it starts: not from the address bar or
    /// the find bar while the user types in them.
    fn set_take_focus(&self, window: &MainWindow) {
        let global = window.global::<PdfViewerModel>();
        let others_typing = window.get_text_input_focused() && !global.get_text_focused();
        global.set_take_focus(!others_typing);
    }

    fn refresh(&self) {
        if let Some(window) = self.window.upgrade() {
            self.refresh_in(&window);
        }
    }

    /// Writes the shown document's state to the window.
    fn refresh_in(&self, window: &MainWindow) {
        let Some(tab) = self.shown.get() else {
            return;
        };
        let update = {
            let sessions = self.sessions.borrow();
            let Some(session) = sessions.get(tab) else {
                return;
            };
            let facts = self.facts(session);
            self.cache.borrow_mut().prepare(facts, session.model())
        };
        // The viewer may be about to ask for the keyboard (the password field).
        self.set_take_focus(window);
        update.write(&window.global::<PdfViewerModel>());
        // A search answer moves the count in the find bar, and only the find bar's own sync
        // writes it: without this the bar keeps saying "Searching…" once the hits are in.
        self.write_find_status(window);
    }

    /// The find bar's count and colour, from the shown document's search (nothing when no search
    /// of a document is on).
    fn write_find_status(&self, window: &MainWindow) {
        let Some(progress) = self
            .shown_document(session::Session::find_progress)
            .flatten()
        else {
            return;
        };
        window.set_find_status(find::status_text(&progress).as_str().into());
        window.set_find_no_matches(find::has_no_matches(&progress));
    }

    fn facts(&self, session: &Session) -> Facts {
        let (state, message) = match session.phase() {
            Phase::Idle | Phase::Opening => (ViewState::Loading, String::new()),
            Phase::NeedsPassword { wrong } => (
                ViewState::Password,
                if *wrong {
                    PdfError::PasswordIncorrect.to_string()
                } else {
                    String::new()
                },
            ),
            Phase::Ready => (ViewState::Ready, String::new()),
            Phase::Failed(error) => (ViewState::Error, error.to_string()),
        };
        let fit = match session
            .model()
            .map(|model| (model.zoom_percent(), model.fit()))
        {
            Some((FIT_ZOOM_PERCENT, Fit::Width)) => FitButton::Width,
            Some((FIT_ZOOM_PERCENT, Fit::Page)) => FitButton::Page,
            _ => FitButton::None,
        };
        Facts {
            state,
            title: session.display_title().to_owned(),
            message,
            zoom_percent: session.zoom_percent(),
            fit,
            // A file the user has is already saved; for an address that is not the web there is
            // nothing to download.
            can_save_copy: matches!(session.url.scheme(), "http" | "https"),
            can_open_externally: true,
            find_open: self.find_open.get(),
            over_link: self.over_link.get(),
        }
    }

    /// A helper answered.
    fn pump(&self, tab: TabId) {
        let pumped = {
            let mut sessions = self.sessions.borrow_mut();
            match sessions.get_mut(tab) {
                Some(session) => session.pump(),
                None => return,
            }
        };
        if pumped.became_ready {
            self.apply_diagnostics(tab);
            // The tab strip and the window title take the document's own title.
            self.schedule_sync();
        }
        if pumped.changed && self.shown.get() == Some(tab) {
            self.refresh();
            self.request_redraw();
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window.upgrade() {
            window.window().request_redraw();
        }
    }

    // ----- what the window asks for ---------------------------------------------------------------

    /// The shown document, changed by `change`, then the window brought up to date.
    fn with_shown(&self, change: impl FnOnce(&mut Session)) {
        let Some(tab) = self.shown.get() else {
            return;
        };
        {
            let mut sessions = self.sessions.borrow_mut();
            let Some(session) = sessions.get_mut(tab) else {
                return;
            };
            change(session);
        }
        self.refresh();
        self.request_redraw();
    }

    fn viewport_changed(&self, width: f32, height: f32) {
        self.area.set(Some((width, height)));
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let viewport = self.viewport_for(&window);
        self.with_shown(|session| session.set_viewport(viewport));
    }

    fn scrolled(&self, delta_x: f32, delta_y: f32) {
        let (dx, dy) = wheel_scroll(delta_x, delta_y);
        self.with_shown(|session| session.scroll(ScrollMove::By(dx, dy)));
    }

    fn scroll_key(&self, key: PdfScrollKey) {
        let height = self.area.get().map_or(0.0, |(_, height)| height);
        self.with_shown(|session| session.scroll(scroll_move(key, height)));
    }

    fn pointer(&self, kind: PdfPointer, x: f32, y: f32) {
        match kind {
            PdfPointer::Down => {
                let clicks = self.pointer.borrow_mut().down(Instant::now(), x, y);
                self.with_shown(|session| session.pointer_down(x, y, clicks));
            }
            PdfPointer::Moved => {
                let pressed = self.pointer.borrow().is_pressed();
                if pressed {
                    if self.pointer.borrow_mut().moved(x, y) {
                        self.with_shown(|session| session.pointer_drag(x, y));
                    }
                } else {
                    self.hover(x, y);
                }
            }
            PdfPointer::Up => {
                let click = self.pointer.borrow_mut().up();
                self.with_shown(Session::pointer_up);
                if let Some((x, y)) = click {
                    self.follow_link_at(x, y);
                }
            }
            PdfPointer::Cancel => {
                self.pointer.borrow_mut().cancel();
                self.with_shown(Session::pointer_up);
            }
        }
    }

    /// The cursor is a hand over a link.
    fn hover(&self, x: f32, y: f32) {
        let Some(tab) = self.shown.get() else {
            return;
        };
        let over = self
            .sessions
            .borrow()
            .get(tab)
            .is_some_and(|session| session.link_at(x, y).is_some());
        if self.over_link.replace(over) != over {
            self.refresh();
        }
    }

    /// A click on a link: inside the document it scrolls; an `http` or `https` address is an
    /// ordinary navigation of the tab, as a click on a link in a page; anything else does
    /// nothing.
    fn follow_link_at(&self, x: f32, y: f32) {
        let Some(tab) = self.shown.get() else {
            return;
        };
        let action = {
            let sessions = self.sessions.borrow();
            let Some(session) = sessions.get(tab) else {
                return;
            };
            let Some(target) = session.link_at(x, y) else {
                return;
            };
            link_action(
                &target,
                session.model().map_or(0, |model| model.page_count()),
            )
        };
        match action {
            LinkAction::GoToPage { page, y } => {
                self.with_shown(|session| session.go_to_page(page, y))
            }
            LinkAction::Navigate(url) => self.navigate(tab, &url),
            LinkAction::Ignore => {}
        }
    }

    fn navigate(&self, tab: TabId, url: &Url) {
        let (Some(window), Some(controller)) = (self.window.upgrade(), self.controller.upgrade())
        else {
            return;
        };
        let started = {
            let Ok(mut controller) = controller.try_borrow_mut() else {
                return;
            };
            // The user clicked: a download or a PDF it leads to was asked for.
            crate::downloads::note_typed_url(tab, url.as_str());
            let result = controller.queue_navigation(url.as_str());
            if result.is_ok() {
                controller.set_navigation_origin(tab, NavigationOrigin::Link);
            }
            result
        };
        match started {
            Ok(()) => {
                window.invoke_accept_address_draft();
                crate::engine::request_pump();
            }
            Err(error) => log::warn!("a link in a PDF could not be followed: {error}"),
        }
        if let Ok(controller) = controller.try_borrow() {
            sync_ui(&window, &controller);
        }
        window.window().request_redraw();
    }

    /// The tab zoom to the step nearest to the pinch's.
    fn pinch(&self, percent: f32) {
        let target = nearest_zoom_step(percent);
        self.set_tab_zoom(target);
    }

    /// Steps the tab's zoom to `target` (one of the zoom steps) and brings the window up to date.
    fn set_tab_zoom(&self, target: u16) {
        let (Some(window), Some(controller)) = (self.window.upgrade(), self.controller.upgrade())
        else {
            return;
        };
        {
            let Ok(mut controller) = controller.try_borrow_mut() else {
                return;
            };
            step_zoom_to(&mut controller, target);
        }
        if let Ok(controller) = controller.try_borrow() {
            sync_ui(&window, &controller);
        }
        window.window().request_redraw();
    }

    fn zoom_in(&self) {
        self.zoom_by(true);
    }

    fn zoom_out(&self) {
        self.zoom_by(false);
    }

    fn zoom_by(&self, larger: bool) {
        let (Some(window), Some(controller)) = (self.window.upgrade(), self.controller.upgrade())
        else {
            return;
        };
        {
            let Ok(mut controller) = controller.try_borrow_mut() else {
                return;
            };
            if larger {
                controller.zoom_in();
            } else {
                controller.zoom_out();
            }
        }
        if let Ok(controller) = controller.try_borrow() {
            sync_ui(&window, &controller);
        }
        window.window().request_redraw();
    }

    fn fit(&self, fit: Fit) {
        self.with_shown(|session| session.set_fit(fit));
        self.set_tab_zoom(FIT_ZOOM_PERCENT);
    }

    fn go_to_page_typed(&self, text: &str) {
        let Ok(number) = text.trim().parse::<u32>() else {
            return;
        };
        self.with_shown(|session| session.go_to_page_number(number));
    }

    fn open_find_bar(&self) {
        let (Some(window), Some(controller)) = (self.window.upgrade(), self.controller.upgrade())
        else {
            return;
        };
        crate::find::open_find_bar(&window, &controller, &self.web_view_focused);
    }

    fn save_copy(&self) {
        let Some((tab, url)) = self.shown_document(|session| (session.tab, session.url.clone()))
        else {
            return;
        };
        if let Err(message) = self.actions.save_copy(tab, &url) {
            log::warn!("a PDF could not be saved: {message}");
        }
    }

    fn open_externally(&self) {
        let Some((file, name)) = self
            .shown_document(|session| (session.file().clone(), session.display_title().to_owned()))
        else {
            return;
        };
        if let Err(message) = self.actions.open_externally(&file.path, file.owned, &name) {
            log::warn!("a PDF could not be opened in another app: {message}");
        }
    }

    fn shown_document<T>(&self, read: impl FnOnce(&Session) -> T) -> Option<T> {
        let tab = self.shown.get()?;
        self.sessions.borrow().get(tab).map(read)
    }

    fn submit_password(&self, password: &str) {
        let password = Zeroizing::new(password.to_owned());
        self.with_shown(|session| session.submit_password(&*self.factory, password));
    }

    fn retry(&self) {
        self.with_shown(|session| session.retry(&*self.factory));
    }

    /// Cmd+C and Edit > Copy: the selected text to the clipboard.
    fn copy_selection(&self) {
        let text = self.shown_document(Session::selected_text).flatten();
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            self.clipboard.write_text(&text);
        }
    }

    fn escape(&self) {
        self.with_shown(Session::clear_selection);
    }

    // ----- find -----------------------------------------------------------------------------------

    fn has_document(&self, tab: TabId) -> bool {
        self.sessions
            .try_borrow()
            .is_ok_and(|sessions| sessions.get(tab).is_some())
    }

    /// The tab's find bar says what to search for; the document is searched for it.
    fn search(&self, tab: TabId, query: &str, options: FindOptions) {
        {
            let Ok(mut sessions) = self.sessions.try_borrow_mut() else {
                return;
            };
            let Some(session) = sessions.get_mut(tab) else {
                return;
            };
            session.find(query, options);
        }
        self.refresh();
        self.request_redraw();
    }

    /// Return and the arrow buttons of the bar: the next or previous match. A search the bar
    /// has not started yet (the user pressed Return before the typing pause ended) starts now.
    fn step(&self, tab: TabId, query: &str, options: FindOptions, forward: bool) {
        let searching_it = self
            .sessions
            .try_borrow()
            .ok()
            .and_then(|sessions| {
                sessions.get(tab).map(|session| {
                    session.find_query() == Some(query) && session.find_options() == Some(options)
                })
            })
            .unwrap_or(false);
        if !searching_it {
            self.search(tab, query, options);
            return;
        }
        {
            let Ok(mut sessions) = self.sessions.try_borrow_mut() else {
                return;
            };
            if let Some(session) = sessions.get_mut(tab) {
                session.find_step(forward);
            }
        }
        self.refresh();
        self.request_redraw();
    }

    /// The bar closed: the highlights go and the keyboard returns to the pages.
    fn find_closed(&self, tab: TabId) {
        if let Ok(mut sessions) = self.sessions.try_borrow_mut()
            && let Some(session) = sessions.get_mut(tab)
        {
            session.find_clear();
        }
        self.find_open.set(false);
        self.refresh();
        self.request_redraw();
        // After the bar's own handler, which hands the keyboard to the engine's page.
        let window = self.window.clone();
        slint::Timer::single_shot(Duration::ZERO, move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(window) = window.upgrade() {
                focus_viewer(&window);
            }
        });
    }

    // ----- diagnostics ----------------------------------------------------------------------------

    /// `--pdf-page` and `--pdf-find`, once, for the first document that opens in the active tab.
    fn apply_diagnostics(&self, tab: TabId) {
        let (Some(window), Some(controller)) = (self.window.upgrade(), self.controller.upgrade())
        else {
            return;
        };
        let wanted = controller
            .try_borrow()
            .is_ok_and(|controller| controller.active_tab_id() == tab);
        if !wanted {
            return;
        }
        let Diagnostics { page, find } = std::mem::take(&mut *self.diagnostics.borrow_mut());
        if let Some(page) = page
            && let Ok(mut sessions) = self.sessions.try_borrow_mut()
            && let Some(session) = sessions.get_mut(tab)
        {
            session.go_to_page_number(page);
        }
        if let Some(query) = find {
            if let Ok(mut controller) = controller.try_borrow_mut() {
                controller.open_find();
                controller.set_find_query(&query);
            }
            self.find_open.set(true);
            self.search(tab, &query, FindOptions::default());
            crate::find::sync_find_bar(&window, controller.borrow().active_state());
        }
    }
}

/// Moves the keyboard to the viewer's pages (or its password field).
fn focus_viewer(window: &MainWindow) {
    let global = window.global::<PdfViewerModel>();
    global.set_focus_serial(global.get_focus_serial() + 1);
}

/// Whether `address` (a tab's address as the engine reports it) is the document's, ignoring the
/// fragment.
fn same_document(address: &str, document: &Url) -> bool {
    let Ok(mut address) = Url::parse(address) else {
        return false;
    };
    let mut document = document.clone();
    address.set_fragment(None);
    document.set_fragment(None);
    address == document
}

/// Steps the active tab's zoom to `target`, one of the zoom steps, by the controller's own
/// steps (so that the engine's zoom follows).
fn step_zoom_to(controller: &mut BrowserController, target: u16) {
    for _ in 0..talaria_core::ZOOM_STEPS.len() {
        let zoom = controller.active_state().zoom_percent();
        if zoom == target {
            break;
        }
        if zoom < target {
            controller.zoom_in();
        } else {
            controller.zoom_out();
        }
    }
}

// ----- hooks other modules call --------------------------------------------------------------------

/// `ui_sync`: decides what the page area shows. A document in front hides the engine's page.
pub(crate) fn sync_window(window: &MainWindow, controller: &BrowserController) {
    if let Some(pdf) = current() {
        pdf.sync(window, controller);
    }
}

/// The tab strip and the window title: the document's own title while a PDF is in the tab,
/// otherwise `page_title`.
pub(crate) fn tab_title(tab: TabId, page_title: &str) -> String {
    current()
        .and_then(|pdf| {
            let sessions = pdf.sessions.try_borrow().ok()?;
            sessions
                .get(tab)
                .map(|session| session.display_title().to_owned())
        })
        .unwrap_or_else(|| page_title.to_owned())
}

/// The end of `find::sync_find_bar`: while a PDF is in front the bar shows the viewer's search.
pub(crate) fn sync_find_status(window: &MainWindow) {
    if let Some(pdf) = current() {
        pdf.write_find_status(window);
    }
}

/// `find`: the typing pause ended (or Return, or match case changed). Searches the tab's
/// document, if it has one, for the find bar's text; false means the tab shows a page.
pub(crate) fn search(controller: &RefCell<BrowserController>, tab: TabId) -> bool {
    let Some(pdf) = current().filter(|pdf| pdf.has_document(tab)) else {
        return false;
    };
    let (query, options) = find_request(controller);
    pdf.search(tab, &query, options);
    true
}

/// `find`: Next or Previous. False means the tab shows a page.
pub(crate) fn step(controller: &RefCell<BrowserController>, tab: TabId, forward: bool) -> bool {
    let Some(pdf) = current().filter(|pdf| pdf.has_document(tab)) else {
        return false;
    };
    let (query, options) = find_request(controller);
    pdf.step(tab, &query, options, forward);
    true
}

/// `find`: the bar was closed.
pub(crate) fn find_closed(tab: TabId) {
    if let Some(pdf) = current().filter(|pdf| pdf.has_document(tab)) {
        pdf.find_closed(tab);
    }
}

/// The find bar's text and options, from the controller's find state of the active tab.
fn find_request(controller: &RefCell<BrowserController>) -> (String, FindOptions) {
    let controller = controller.borrow();
    let find = controller.active_state().find();
    (
        find.query().to_owned(),
        FindOptions {
            match_case: find.match_case(),
            whole_word: false,
        },
    )
}

/// `menus`: a PDF is in front and the keyboard is not in a text field, so the Edit menu's items
/// are the viewer's.
pub(crate) fn keyboard_is_in_viewer() -> bool {
    let Some(pdf) = current() else {
        return false;
    };
    let Some(window) = pdf.window.upgrade() else {
        return false;
    };
    pdf.shown.get().is_some() && !window.get_text_input_focused()
}

/// `menus`: an Edit menu item while a PDF is in front. Copy copies the selected text; the other
/// items do nothing in a viewer. True when it was the viewer's.
pub(crate) fn handle_edit(copy: bool) -> bool {
    if !keyboard_is_in_viewer() {
        return false;
    }
    if copy && let Some(pdf) = current() {
        pdf.copy_selection();
    }
    true
}

// ----- the window's callbacks -------------------------------------------------------------------------

fn connect_callbacks(window: &MainWindow) {
    let global = window.global::<PdfViewerModel>();
    global.on_viewport_changed(|width, height| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.viewport_changed(width, height);
        }
    });
    global.on_scrolled(|delta_x, delta_y| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.scrolled(delta_x, delta_y);
        }
    });
    global.on_pointer(|kind, x, y| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.pointer(kind, x, y);
        }
    });
    global.on_scroll_key(|key| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.scroll_key(key);
        }
    });
    global.on_pinch(|percent| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.pinch(percent);
        }
    });
    global.on_go_to_page(|text| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.go_to_page_typed(text.as_str());
        }
    });
    global.on_zoom_in(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.zoom_in();
        }
    });
    global.on_zoom_out(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.zoom_out();
        }
    });
    global.on_fit_width(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.fit(Fit::Width);
        }
    });
    global.on_fit_page(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.fit(Fit::Page);
        }
    });
    global.on_rotate(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.with_shown(Session::rotate);
        }
    });
    global.on_find(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.open_find_bar();
        }
    });
    global.on_save_copy(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.save_copy();
        }
    });
    global.on_open_externally(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.open_externally();
        }
    });
    global.on_submit_password(|password| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.submit_password(password.as_str());
        }
    });
    global.on_retry(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.retry();
        }
    });
    global.on_copy_selection(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.copy_selection();
        }
    });
    global.on_escape(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(pdf) = current() {
            pdf.escape();
        }
    });
}
