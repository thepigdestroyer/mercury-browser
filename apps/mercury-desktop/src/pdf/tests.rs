// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Tests of the viewer's logic with the fake PDF backend (`mercury_pdf::fake`) running in
//! process: no helper process, no PDFium. A document opens, scrolls, is selected and searched;
//! helpers are ended and come back; a helper that is lost is reported. The clipboard and the
//! buttons' actions are fakes, so nothing here touches the user's clipboard, folders or apps.
//!
//! The first half drives a [`Session`] alone; the second half drives the whole viewer ([`Pdf`])
//! with a real (headless) window.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mercury_pdf::client::{ClientEvent, PdfClient};
use mercury_pdf::error::PdfError;
use mercury_pdf::fake::{FakeBackend, FakeConfig};
use mercury_pdf::types::{FindOptions, Rotation};
use mercury_pdf::viewer::Viewport;
use mercury_ui::{MainWindow, PdfPointer, PdfScrollKey, PdfViewState, PdfViewerModel};
use slint::{ComponentHandle, Model};
use talaria_core::{BrowserController, TabId};
use url::Url;

use super::actions::fake::FakeActions;
use super::copy::MemoryClipboard;
use super::input::ScrollMove;
use super::session::{ClientFactory, DocumentFile, MAX_LIVE_HELPERS, Phase, Session, Sessions};
use super::*;
use crate::downloads::pdf_handoff::PdfFile;

// ----- fakes and helpers ---------------------------------------------------------------------------

fn url(text: &str) -> Url {
    Url::parse(text).expect("a URL")
}

/// Starts the fake backend in process; the first starts may use other configurations (a helper
/// that fails), then every start uses `fallback`.
struct FakeFactory {
    first: RefCell<VecDeque<FakeConfig>>,
    fallback: FakeConfig,
    started: Cell<usize>,
}

impl FakeFactory {
    fn new(fallback: FakeConfig) -> Self {
        Self {
            first: RefCell::new(VecDeque::new()),
            fallback,
            started: Cell::new(0),
        }
    }

    fn then_first(self, config: FakeConfig) -> Self {
        self.first.borrow_mut().push_back(config);
        self
    }
}

impl ClientFactory for FakeFactory {
    fn start(&self, events: Box<dyn Fn(ClientEvent) + Send + Sync>) -> Result<PdfClient, PdfError> {
        self.started.set(self.started.get() + 1);
        let config = self
            .first
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| self.fallback.clone());
        Ok(PdfClient::start_in_process(
            Box::new(FakeBackend::new(config)),
            events,
        ))
    }
}

fn quiet() -> Arc<dyn Fn(TabId, u64) + Send + Sync> {
    Arc::new(|_, _| {})
}

/// A private copy of a PDF in `folder`, as the hand-off makes them.
fn private_copy(folder: &tempfile::TempDir, name: &str) -> DocumentFile {
    let path = folder.path().join(name);
    std::fs::write(&path, b"%PDF-1.4\n").expect("a file");
    DocumentFile { path, owned: true }
}

fn new_session(folder: &tempfile::TempDir, tab: u64, name: &str) -> Session {
    Session::new(
        TabId::new(tab),
        url(&format!("https://a.test/{name}")),
        private_copy(folder, name),
        quiet(),
    )
}

/// Pumps the session until `done` holds; fails after ten seconds.
fn wait_for(session: &mut Session, what: &str, done: impl Fn(&Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        session.pump();
        if done(session) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; the document is {:?}",
            session.phase()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn has_exact_tile_of(session: &Session, page: u32) -> bool {
    session.model().is_some_and(|model| {
        model
            .visible_tiles()
            .iter()
            .any(|tile| tile.page == page && !tile.is_placeholder)
    })
}

fn opened(folder: &tempfile::TempDir, factory: &FakeFactory, name: &str) -> Session {
    let mut session = new_session(folder, 1, name);
    session.ensure_started(factory);
    wait_for(&mut session, "the document to open", Session::is_ready);
    session
}

// ----- a session alone -------------------------------------------------------------------------------

#[test]
fn a_document_opens_and_its_first_tiles_arrive() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(3));
    let mut session = new_session(&folder, 1, "a.pdf");
    assert_eq!(session.phase(), &Phase::Idle);
    assert!(!session.is_live(), "nothing starts before the tab is shown");
    assert_eq!(
        session.display_title(),
        "a.pdf",
        "the file name until the title is known"
    );

    session.ensure_started(&factory);
    assert_eq!(session.phase(), &Phase::Opening);
    wait_for(&mut session, "the document to open", Session::is_ready);
    assert_eq!(session.model().expect("a model").page_count(), 3);
    assert_eq!(session.display_title(), "Fake document");
    wait_for(&mut session, "the first tile", |session| {
        has_exact_tile_of(session, 0)
    });
    assert_eq!(factory.started.get(), 1);
}

#[test]
fn scrolling_asks_for_the_tiles_of_the_new_place() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(3));
    let mut session = opened(&folder, &factory, "a.pdf");
    wait_for(&mut session, "the first tile", |s| has_exact_tile_of(s, 0));

    session.scroll(ScrollMove::Bottom);
    wait_for(&mut session, "a tile of the last page", |s| {
        has_exact_tile_of(s, 2)
    });
    let model = session.model().expect("a model");
    assert_eq!(model.current_page(), 2);
    assert!(model.scroll_y() > 0.0);

    session.scroll(ScrollMove::Top);
    wait_for(&mut session, "a tile of the first page again", |s| {
        has_exact_tile_of(s, 0)
    });
    assert_eq!(session.model().expect("a model").current_page(), 0);
}

#[test]
fn a_document_opens_at_its_top_whatever_the_window_and_the_zoom() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(4));
    let mut session = new_session(&folder, 1, "a.pdf");
    session.set_viewport(Viewport {
        width: 1337.0,
        height: 411.0,
        scale_factor: 2.0,
    });
    session.set_zoom_percent(150);
    session.ensure_started(&factory);
    wait_for(&mut session, "the document to open", Session::is_ready);
    let model = session.model().expect("a model");
    assert_eq!(
        model.scroll_y(),
        0.0,
        "not scrolled by the change from the default viewport"
    );
    assert_eq!(model.scroll_x(), 0.0);
    assert_eq!(model.current_page(), 0);
    assert_eq!(model.zoom_percent(), 150);
}

#[test]
fn a_page_number_scrolls_to_that_page_and_a_page_past_the_end_to_the_last() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(3));
    let mut session = opened(&folder, &factory, "a.pdf");

    session.go_to_page_number(2);
    assert_eq!(session.model().expect("a model").current_page(), 1);
    session.go_to_page_number(99);
    assert_eq!(session.model().expect("a model").current_page(), 2);
    session.go_to_page_number(0);
    assert_eq!(
        session.model().expect("a model").current_page(),
        0,
        "page 0 does not exist: the first"
    );
}

#[test]
fn rotating_turns_the_view_a_quarter_each_time() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(2));
    let mut session = opened(&folder, &factory, "a.pdf");
    session.rotate();
    assert_eq!(
        session.model().expect("a model").rotation(),
        Rotation::Deg90
    );
    session.rotate();
    session.rotate();
    session.rotate();
    assert_eq!(session.model().expect("a model").rotation(), Rotation::Deg0);
}

#[test]
fn the_address_fragment_names_the_page_to_open_on() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(4));
    let mut session = Session::new(
        TabId::new(1),
        url("https://a.test/a.pdf#page=3"),
        private_copy(&folder, "a.pdf"),
        quiet(),
    );
    session.ensure_started(&factory);
    wait_for(&mut session, "the document to open", Session::is_ready);
    assert_eq!(session.model().expect("a model").current_page(), 2);
}

#[test]
fn a_helper_ended_to_save_memory_comes_back_with_the_view_it_had() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(3));
    let mut session = opened(&folder, &factory, "a.pdf");
    session.scroll(ScrollMove::Bottom);
    let scrolled = session.model().expect("a model").scroll_y();
    assert!(scrolled > 0.0);

    session.suspend();
    assert!(!session.is_live());
    assert_eq!(session.phase(), &Phase::Idle);
    assert!(session.model().is_some(), "the view model is kept");
    assert!(session.file().path.exists(), "the file is kept");

    session.ensure_started(&factory);
    wait_for(
        &mut session,
        "the document to open again",
        Session::is_ready,
    );
    assert_eq!(factory.started.get(), 2);
    assert_eq!(session.model().expect("a model").scroll_y(), scrolled);
    wait_for(&mut session, "the tiles of the place it was at", |s| {
        has_exact_tile_of(s, 2)
    });
}

#[test]
fn the_fifth_document_ends_the_helper_of_the_one_shown_longest_ago() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(2));
    let mut sessions = Sessions::default();
    for tab in 1..=5 {
        sessions.insert(new_session(&folder, tab, &format!("{tab}.pdf")));
    }
    assert_eq!(MAX_LIVE_HELPERS, 4);
    for tab in 1..=5 {
        let tab = TabId::new(tab);
        sessions.touch(tab);
        sessions
            .get_mut(tab)
            .expect("a session")
            .ensure_started(&factory);
        let suspended = sessions.limit_live(tab, MAX_LIVE_HELPERS);
        if tab.value() <= 4 {
            assert!(suspended.is_empty(), "four helpers are allowed");
        } else {
            assert_eq!(suspended, vec![TabId::new(1)], "the first one shown goes");
        }
    }
    assert_eq!(sessions.live_count(), 4);
    let first = sessions.get(TabId::new(1)).expect("a session");
    assert!(!first.is_live());
    assert_eq!(first.phase(), &Phase::Idle);

    // Showing the first again ends the helper of the next oldest, not the one shown.
    sessions.touch(TabId::new(1));
    sessions
        .get_mut(TabId::new(1))
        .expect("a session")
        .ensure_started(&factory);
    assert_eq!(
        sessions.limit_live(TabId::new(1), MAX_LIVE_HELPERS),
        vec![TabId::new(2)]
    );
    assert_eq!(sessions.live_count(), 4);
    assert!(sessions.get(TabId::new(1)).expect("a session").is_live());
}

#[test]
fn a_lost_helper_is_reported_once_and_try_again_starts_a_new_one() {
    let folder = tempfile::tempdir().expect("a folder");
    // The first helper's backend panics at its first request (the first tile).
    let factory = FakeFactory::new(FakeConfig::with_pages(3)).then_first(FakeConfig {
        panic_after: Some(0),
        ..FakeConfig::with_pages(3)
    });
    let mut session = new_session(&folder, 1, "a.pdf");
    session.ensure_started(&factory);
    wait_for(&mut session, "the helper to be lost", |s| {
        matches!(s.phase(), Phase::Failed(_))
    });
    let Phase::Failed(error) = session.phase().clone() else {
        panic!("the document failed");
    };
    assert_eq!(
        error.to_string(),
        "The PDF viewer stopped working on this file."
    );
    assert!(!session.is_live(), "the lost helper is let go of");
    assert_eq!(factory.started.get(), 1, "nothing restarts by itself");
    session.pump();
    assert_eq!(factory.started.get(), 1);

    session.retry(&factory);
    assert_eq!(factory.started.get(), 2);
    wait_for(
        &mut session,
        "the new helper to open the file",
        Session::is_ready,
    );
    wait_for(&mut session, "a tile from the new helper", |s| {
        has_exact_tile_of(s, 0)
    });
}

#[test]
fn a_password_is_asked_for_until_it_is_right() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig {
        password: Some("secret".to_owned()),
        ..FakeConfig::with_pages(2)
    });
    let mut session = new_session(&folder, 1, "a.pdf");
    session.ensure_started(&factory);
    wait_for(&mut session, "the password prompt", |s| {
        matches!(s.phase(), Phase::NeedsPassword { .. })
    });
    assert_eq!(session.phase(), &Phase::NeedsPassword { wrong: false });
    assert!(session.model().is_none());

    session.submit_password(&factory, zeroize::Zeroizing::new("nope".to_owned()));
    assert_eq!(session.phase(), &Phase::Opening);
    wait_for(&mut session, "the refusal", |s| {
        matches!(s.phase(), Phase::NeedsPassword { .. })
    });
    assert_eq!(session.phase(), &Phase::NeedsPassword { wrong: true });

    session.submit_password(&factory, zeroize::Zeroizing::new("secret".to_owned()));
    wait_for(&mut session, "the document to open", Session::is_ready);
    assert_eq!(factory.started.get(), 1, "the same helper took every try");
}

#[test]
fn a_file_that_is_not_a_pdf_is_an_error_and_ends_its_helper() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig {
        corrupt: true,
        ..FakeConfig::with_pages(2)
    });
    let mut session = new_session(&folder, 1, "a.pdf");
    session.ensure_started(&factory);
    wait_for(&mut session, "the error", |s| {
        matches!(s.phase(), Phase::Failed(_))
    });
    assert!(!session.is_live());
    let Phase::Failed(error) = session.phase().clone() else {
        panic!("the document failed");
    };
    assert!(error.to_string().contains("damaged"));
}

#[test]
fn ending_a_session_deletes_a_private_copy_and_never_the_users_own_file() {
    let folder = tempfile::tempdir().expect("a folder");
    let private = private_copy(&folder, "private.pdf");
    let private_path = private.path.clone();
    let mut session = Session::new(TabId::new(1), url("https://a.test/p.pdf"), private, quiet());
    session.close();
    assert!(!private_path.exists(), "a private copy is deleted");

    let own = folder.path().join("mine.pdf");
    std::fs::write(&own, b"%PDF-1.4\n").expect("a file");
    let session = Session::new(
        TabId::new(2),
        url("file:///Users/someone/mine.pdf"),
        DocumentFile {
            path: own.clone(),
            owned: false,
        },
        quiet(),
    );
    drop(session);
    assert!(own.exists(), "the user's own file is never deleted");

    // Dropping a session ends it too.
    let again = private_copy(&folder, "again.pdf");
    let again_path = again.path.clone();
    drop(Session::new(
        TabId::new(3),
        url("https://a.test/again.pdf"),
        again,
        quiet(),
    ));
    assert!(!again_path.exists());
}

#[test]
fn a_search_counts_the_matches_and_steps_between_them() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(3));
    let mut session = opened(&folder, &factory, "a.pdf");
    assert_eq!(session.find_progress(), None, "no search yet");

    session.find("quick", FindOptions::default());
    wait_for(&mut session, "the search to finish", |s| {
        s.find_progress().is_some_and(|progress| progress.finished)
    });
    let progress = session.find_progress().expect("a search");
    assert_eq!(
        (progress.current, progress.total),
        (1, 3),
        "one match on each page"
    );
    assert_eq!(session.find_query(), Some("quick"));

    assert!(session.find_step(true));
    assert_eq!(session.find_progress().expect("a search").current, 2);
    assert!(session.find_step(false));
    assert!(session.find_step(false));
    assert_eq!(
        session.find_progress().expect("a search").current,
        3,
        "previous wraps round to the last match"
    );

    session.find("no such words", FindOptions::default());
    wait_for(&mut session, "the empty search to finish", |s| {
        s.find_progress().is_some_and(|progress| progress.finished)
    });
    assert_eq!(session.find_progress().expect("a search").total, 0);

    session.find_clear();
    assert_eq!(session.find_progress(), None);
}

#[test]
fn a_search_asked_for_before_the_document_opened_runs_when_it_has() {
    let folder = tempfile::tempdir().expect("a folder");
    let factory = FakeFactory::new(FakeConfig::with_pages(2));
    let mut session = new_session(&folder, 1, "a.pdf");
    session.find("quick", FindOptions::default());
    session.ensure_started(&factory);
    wait_for(&mut session, "the search to finish", |s| {
        s.find_progress().is_some_and(|progress| progress.finished)
    });
    assert_eq!(session.find_progress().expect("a search").total, 2);
}

// ----- the whole viewer, in a headless window ------------------------------------------------------

thread_local! {
    static SLINT_READY: Cell<bool> = const { Cell::new(false) };
}

fn init_slint() {
    if !SLINT_READY.with(|ready| ready.replace(true)) {
        i_slint_backend_testing::init_no_event_loop();
    }
}

/// The viewer with a window, a controller and fakes for everything outside.
struct Rig {
    pdf: Rc<Pdf>,
    window: MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    factory: Rc<FakeFactory>,
    clipboard: Rc<MemoryClipboard>,
    actions: Rc<FakeActions>,
    folder: tempfile::TempDir,
}

impl Drop for Rig {
    fn drop(&mut self) {
        PDF.with(|current| *current.borrow_mut() = None);
    }
}

fn rig(factory: FakeFactory, diagnostics: Diagnostics) -> Rig {
    init_slint();
    let window = MainWindow::new().expect("the window builds");
    let controller = Rc::new(RefCell::new(BrowserController::default()));
    let factory = Rc::new(factory);
    let clipboard = Rc::new(MemoryClipboard::default());
    let actions = Rc::new(FakeActions::default());
    let pdf = Rc::new(Pdf::new(
        window.as_weak(),
        Rc::downgrade(&controller),
        Rc::new(Cell::new(false)),
        Tools {
            factory: Box::new(Rc::clone(&factory)),
            clipboard: Box::new(Rc::clone(&clipboard)),
            actions: Box::new(Rc::clone(&actions)),
            library_found: true,
        },
        diagnostics,
    ));
    PDF.with(|current| *current.borrow_mut() = Some(Rc::clone(&pdf)));
    connect_callbacks(&window);
    Rig {
        pdf,
        window,
        controller,
        factory,
        clipboard,
        actions,
        folder: tempfile::tempdir().expect("a folder"),
    }
}

impl Rig {
    fn tab(&self) -> TabId {
        self.controller.borrow().active_tab_id()
    }

    fn model(&self) -> PdfViewerModel<'_> {
        self.window.global::<PdfViewerModel>()
    }

    /// The active tab navigates to a PDF and the hand-off gives the viewer its private copy.
    fn open(&self, name: &str) -> std::path::PathBuf {
        self.open_with_file(name, name)
    }

    /// The same, with the private copy under another name than the address's.
    fn open_with_file(&self, name: &str, file_name: &str) -> std::path::PathBuf {
        let address = format!("https://a.test/{name}");
        self.controller
            .borrow_mut()
            .queue_navigation(&address)
            .expect("an address");
        let file = private_copy_in(self.folder.path(), file_name);
        let path = file.path.clone();
        self.pdf.open(
            self.tab(),
            &url(&address),
            PdfFile {
                path: file.path,
                owned: true,
            },
        );
        self.pdf.viewport_changed(900.0, 560.0);
        self.sync();
        path
    }

    fn sync(&self) {
        self.pdf.sync(&self.window, &self.controller.borrow());
    }

    /// Pumps the active tab's helper until `done` holds.
    fn wait_for(&self, what: &str, done: impl Fn(&Rig) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pdf.pump(self.tab());
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn state(&self) -> PdfViewState {
        self.model().get_state()
    }

    fn with_session<R>(&self, read: impl FnOnce(&Session) -> R) -> R {
        let sessions = self.pdf.sessions.borrow();
        read(sessions.get(self.tab()).expect("the tab has a document"))
    }

    /// Where a point of page `page` is, in the page area's own coordinates, as the pointer
    /// reports it.
    fn page_point(&self, page: u32, x_points: f32, y_points: f32) -> (f32, f32) {
        self.with_session(|session| {
            let model = session.model().expect("a model");
            let (left, top, width, _) = model.page_rect(page).expect("a page");
            let scale = width / 612.0;
            (
                left + x_points * scale - model.scroll_x(),
                top + y_points * scale - model.scroll_y(),
            )
        })
    }
}

fn private_copy_in(folder: &std::path::Path, name: &str) -> DocumentFile {
    let path = folder.join(name);
    std::fs::write(&path, b"%PDF-1.4\n").expect("a file");
    DocumentFile { path, owned: true }
}

fn plain_rig(pages: usize) -> Rig {
    rig(
        FakeFactory::new(FakeConfig::with_pages(pages)),
        Diagnostics::default(),
    )
}

#[test]
fn a_pdf_in_the_tab_covers_the_page_area_and_gives_the_tab_its_title() {
    let rig = plain_rig(3);
    assert!(!rig.window.get_pdf_viewer_shown());
    assert_eq!(
        tab_title(rig.tab(), "Unknown content type"),
        "Unknown content type"
    );

    rig.open("report.pdf");
    assert!(rig.window.get_pdf_viewer_shown(), "the viewer is up");
    assert!(
        !rig.window.get_web_view_visible(),
        "the engine's page below it is hidden"
    );
    assert_eq!(rig.state(), PdfViewState::Loading);
    assert_eq!(tab_title(rig.tab(), "Unknown content type"), "report.pdf");

    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    assert_eq!(rig.model().get_page_count(), 3);
    assert_eq!(rig.model().get_page(), 1);
    assert_eq!(rig.model().get_title().as_str(), "Fake document");
    assert_eq!(
        tab_title(rig.tab(), "Unknown content type"),
        "Fake document",
        "the tab strip takes the document's own title"
    );
    rig.wait_for("tiles", |rig| rig.model().get_tiles().row_count() > 0);
    assert!(rig.model().get_pages().row_count() >= 1);
    assert!(rig.model().get_content_height() > 0.0);
    assert!(
        rig.model().get_can_save_copy(),
        "a web document can be saved again"
    );
}

#[test]
fn the_keys_and_the_wheel_scroll_the_pages() {
    let rig = plain_rig(3);
    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    assert_eq!(rig.model().get_scroll_y(), 0.0);

    rig.pdf.scroll_key(PdfScrollKey::PageDown);
    let after_page = rig.model().get_scroll_y();
    assert!(
        after_page > 400.0,
        "a page key moves most of a screen: {after_page}"
    );
    rig.pdf.scroll_key(PdfScrollKey::LineDown);
    assert_eq!(rig.model().get_scroll_y(), after_page + 40.0);
    // Slint's wheel delta is the way the content moves: a negative one scrolls down.
    rig.pdf.scrolled(0.0, -100.0);
    assert_eq!(rig.model().get_scroll_y(), after_page + 140.0);
    rig.pdf.scroll_key(PdfScrollKey::Bottom);
    assert_eq!(rig.model().get_page(), 3);
    rig.wait_for("a tile of the last page", |rig| {
        rig.with_session(|session| has_exact_tile_of(session, 2))
    });
    rig.pdf.scroll_key(PdfScrollKey::Top);
    assert_eq!(rig.model().get_scroll_y(), 0.0);
    assert_eq!(rig.model().get_page(), 1);
}

/// Clicks at a point of a page: a press and a release.
fn click(rig: &Rig, x: f32, y: f32) {
    rig.pdf.pointer(PdfPointer::Down, x, y);
    rig.pdf.pointer(PdfPointer::Up, x, y);
}

#[test]
fn selecting_text_and_copying_it_writes_to_the_clipboard_sink() {
    let rig = plain_rig(2);
    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });

    // Nothing selected: Copy leaves the clipboard alone.
    rig.pdf.copy_selection();
    assert_eq!(rig.clipboard.text(), None);

    // A double click on the first word selects it (the page's text is asked for by the press).
    let (x, y) = rig.page_point(0, 40.0, 42.0);
    click(&rig, x, y);
    click(&rig, x, y);
    rig.wait_for("the word", |rig| {
        rig.with_session(|session| session.selected_text().is_some())
    });
    assert!(
        rig.model().get_selection().row_count() >= 1,
        "the selection shows"
    );
    rig.pdf.copy_selection();
    let word = rig.clipboard.text().expect("text was copied");
    assert!(word.starts_with("Page"), "{word:?}");
    assert!(!word.contains("quick"), "only a word: {word:?}");

    // A third click selects the line.
    click(&rig, x, y);
    rig.pdf.copy_selection();
    let line = rig.clipboard.text().expect("text was copied");
    assert!(line.contains("quick brown fox"), "{line:?}");

    // A drag, from another place so that it is a new click, selects what it crosses.
    let (from_x, from_y) = rig.page_point(0, 37.0, 56.0);
    let (to_x, _) = rig.page_point(0, 100.0, 56.0);
    rig.pdf.pointer(PdfPointer::Down, from_x, from_y);
    rig.pdf.pointer(PdfPointer::Moved, to_x, from_y);
    rig.pdf.pointer(PdfPointer::Up, to_x, from_y);
    rig.pdf.copy_selection();
    let dragged = rig.clipboard.text().expect("text was copied");
    assert!(dragged.starts_with("jumps"), "{dragged:?}");

    // Escape takes the selection away; what was copied stays on the clipboard.
    rig.pdf.escape();
    assert_eq!(rig.model().get_selection().row_count(), 0);
    assert_eq!(rig.clipboard.text().as_deref(), Some(dragged.as_str()));
}

#[test]
fn edit_copy_copies_only_while_a_pdf_is_in_front_and_the_keyboard_is_not_in_a_field() {
    let rig = plain_rig(2);
    assert!(!keyboard_is_in_viewer(), "no PDF in front");
    assert!(!handle_edit(true));

    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    assert!(keyboard_is_in_viewer());
    let (x, y) = rig.page_point(0, 40.0, 42.0);
    click(&rig, x, y);
    click(&rig, x, y);
    rig.wait_for("the word", |rig| {
        rig.with_session(|session| session.selected_text().is_some())
    });
    assert!(handle_edit(true), "Copy is the viewer's");
    assert!(rig.clipboard.text().is_some());
    assert!(
        handle_edit(false),
        "Cut and Paste do nothing, and the page does not get them"
    );
}

#[test]
fn the_find_bar_searches_the_document_and_closing_it_clears_the_highlights() {
    let rig = plain_rig(3);
    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    let tab = rig.tab();
    assert!(rig.pdf.has_document(tab));

    {
        let mut controller = rig.controller.borrow_mut();
        controller.open_find();
        controller.set_find_query("quick");
    }
    assert!(
        search(&rig.controller, tab),
        "a PDF is searched by the viewer"
    );
    rig.wait_for("the search to finish", |rig| {
        rig.with_session(|session| {
            session
                .find_progress()
                .is_some_and(|progress| progress.finished)
        })
    });
    sync_find_status(&rig.window);
    assert_eq!(rig.window.get_find_status().as_str(), "1 of 3");
    assert!(!rig.window.get_find_no_matches());
    rig.wait_for("highlights", |rig| rig.model().get_hits().row_count() > 0);

    assert!(step(&rig.controller, tab, true));
    sync_find_status(&rig.window);
    assert_eq!(rig.window.get_find_status().as_str(), "2 of 3");
    assert!(step(&rig.controller, tab, false));
    sync_find_status(&rig.window);
    assert_eq!(rig.window.get_find_status().as_str(), "1 of 3");

    rig.controller.borrow_mut().set_find_query("zebra");
    assert!(search(&rig.controller, tab));
    rig.wait_for("the empty search to finish", |rig| {
        rig.with_session(|session| {
            session
                .find_progress()
                .is_some_and(|progress| progress.finished)
        })
    });
    sync_find_status(&rig.window);
    assert_eq!(rig.window.get_find_status().as_str(), "No matches");
    assert!(rig.window.get_find_no_matches());

    find_closed(tab);
    assert_eq!(rig.model().get_hits().row_count(), 0);
    assert_eq!(rig.with_session(|session| session.find_progress()), None);
}

#[test]
fn a_tab_that_shows_a_page_is_not_the_viewers_to_search() {
    let rig = plain_rig(1);
    let tab = rig.tab();
    assert!(!search(&rig.controller, tab));
    assert!(!step(&rig.controller, tab, true));
    find_closed(tab);
}

#[test]
fn leaving_the_address_ends_the_document_and_removes_its_private_copy() {
    let rig = plain_rig(2);
    let path = rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    assert!(path.exists());

    // A link in the document's own address (a new fragment) is the same document.
    rig.controller
        .borrow_mut()
        .queue_navigation("https://a.test/report.pdf#page=2")
        .expect("an address");
    rig.sync();
    assert!(rig.window.get_pdf_viewer_shown());
    assert!(path.exists());

    rig.controller
        .borrow_mut()
        .queue_navigation("https://b.test/")
        .expect("an address");
    rig.sync();
    assert!(!rig.window.get_pdf_viewer_shown(), "the page shows again");
    assert!(rig.pdf.sessions.borrow().is_empty());
    assert!(!path.exists(), "the private copy is gone");
    assert_eq!(tab_title(rig.tab(), "B"), "B");
}

#[test]
fn closing_the_tab_through_the_hand_off_ends_its_document() {
    let rig = plain_rig(2);
    let path = rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    let sink = Sink;
    assert!(sink.is_available());
    sink.tab_closed(rig.tab());
    assert!(rig.pdf.sessions.borrow().is_empty());
    assert!(!path.exists());
}

#[test]
fn a_second_offer_for_the_same_tab_replaces_the_document() {
    let rig = plain_rig(2);
    let first = rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });

    // Reload: the same address is offered again with a new private copy.
    let second = rig.open_with_file("report.pdf", "reloaded.pdf");
    assert_ne!(first, second);
    assert!(!first.exists(), "the old private copy is gone");
    assert!(second.exists());
    assert_eq!(
        rig.state(),
        PdfViewState::Loading,
        "the new document starts afresh"
    );
    rig.wait_for("the new document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    assert_eq!(rig.factory.started.get(), 2, "a helper each");
}

#[test]
fn a_lost_helper_shows_the_error_and_try_again_opens_the_file_again() {
    let rig = rig(
        FakeFactory::new(FakeConfig::with_pages(3)).then_first(FakeConfig {
            panic_after: Some(0),
            ..FakeConfig::with_pages(3)
        }),
        Diagnostics::default(),
    );
    rig.open("report.pdf");
    rig.wait_for("the error", |rig| rig.state() == PdfViewState::Error);
    assert_eq!(
        rig.model().get_message().as_str(),
        "The PDF viewer stopped working on this file."
    );
    assert_eq!(rig.factory.started.get(), 1);

    // The "Try again" button of the error panel.
    rig.model().invoke_retry();
    assert_eq!(rig.factory.started.get(), 2);
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
}

#[test]
fn the_password_prompt_sends_the_password_and_says_when_it_is_wrong() {
    let rig = rig(
        FakeFactory::new(FakeConfig {
            password: Some("secret".to_owned()),
            ..FakeConfig::with_pages(2)
        }),
        Diagnostics::default(),
    );
    rig.open("report.pdf");
    rig.wait_for("the prompt", |rig| rig.state() == PdfViewState::Password);
    assert_eq!(rig.model().get_message().as_str(), "");

    rig.model().invoke_submit_password("nope".into());
    rig.wait_for("the refusal", |rig| {
        rig.state() == PdfViewState::Password && !rig.model().get_message().is_empty()
    });
    assert_eq!(
        rig.model().get_message().as_str(),
        "That password is not right."
    );

    rig.model().invoke_submit_password("secret".into());
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
}

#[test]
fn the_zoom_buttons_and_the_fit_buttons_use_the_tab_zoom() {
    let rig = plain_rig(3);
    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    assert_eq!(rig.controller.borrow().active_state().zoom_percent(), 100);
    assert_eq!(rig.model().get_fit(), mercury_ui::PdfFitMode::Width);

    rig.model().invoke_zoom_in();
    assert_eq!(rig.controller.borrow().active_state().zoom_percent(), 110);
    rig.sync();
    assert_eq!(rig.model().get_zoom_percent(), 110);
    assert_eq!(
        rig.model().get_fit(),
        mercury_ui::PdfFitMode::None,
        "110 % fits nothing"
    );
    let wider = rig.model().get_content_width();

    rig.model().invoke_zoom_out();
    rig.model().invoke_zoom_out();
    assert_eq!(rig.controller.borrow().active_state().zoom_percent(), 90);
    rig.sync();
    assert!(rig.model().get_content_width() < wider);

    // The fit buttons put the tab's zoom at 100 and say which fit.
    rig.model().invoke_fit_page();
    assert_eq!(rig.controller.borrow().active_state().zoom_percent(), 100);
    rig.sync();
    assert_eq!(rig.model().get_fit(), mercury_ui::PdfFitMode::Page);
    rig.model().invoke_fit_width();
    rig.sync();
    assert_eq!(rig.model().get_fit(), mercury_ui::PdfFitMode::Width);

    // A pinch lands on the nearest step.
    rig.model().invoke_pinch(165.0);
    assert_eq!(rig.controller.borrow().active_state().zoom_percent(), 170);
    rig.model().invoke_pinch(1.0);
    assert_eq!(rig.controller.borrow().active_state().zoom_percent(), 30);
    rig.sync();
    assert!(!rig.model().get_can_zoom_out());
    assert!(rig.model().get_can_zoom_in());
}

#[test]
fn rotate_and_go_to_page_come_from_the_toolbar() {
    let rig = plain_rig(5);
    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });

    rig.model().invoke_rotate();
    assert_eq!(
        rig.with_session(|session| session.model().map(|model| model.rotation())),
        Some(Rotation::Deg90)
    );
    rig.model().invoke_go_to_page(" 4 ".into());
    assert_eq!(rig.model().get_page(), 4);
    rig.model().invoke_go_to_page("not a number".into());
    assert_eq!(
        rig.model().get_page(),
        4,
        "text that is no page number does nothing"
    );
}

#[test]
fn a_click_on_a_link_inside_the_document_scrolls_to_its_page() {
    let rig = plain_rig(3);
    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    // The fake's link of every page is a box at the top-left of its text.
    let (x, y) = rig.page_point(0, 60.0, 42.0);
    rig.wait_for("the page's links", |rig| {
        rig.with_session(|session| session.link_at(x, y).is_some())
    });
    rig.pdf.pointer(PdfPointer::Moved, x, y);
    assert!(
        rig.model().get_over_link(),
        "the pointer is a hand over a link"
    );

    rig.pdf.pointer(PdfPointer::Down, x, y);
    rig.pdf.pointer(PdfPointer::Up, x, y);
    assert_eq!(rig.model().get_page(), 2, "the link goes to the next page");

    rig.pdf.pointer(PdfPointer::Moved, 5.0, 5.0);
    assert!(!rig.model().get_over_link());
}

#[test]
fn a_drag_over_a_link_selects_and_does_not_follow_it() {
    let rig = plain_rig(3);
    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    let (x, y) = rig.page_point(0, 40.0, 42.0);
    rig.wait_for("the page's links", |rig| {
        rig.with_session(|session| session.link_at(x, y).is_some())
    });
    rig.pdf.pointer(PdfPointer::Down, x, y);
    rig.pdf.pointer(PdfPointer::Moved, x + 40.0, y);
    rig.pdf.pointer(PdfPointer::Up, x + 40.0, y);
    assert_eq!(rig.model().get_page(), 1, "still on the first page");
}

#[test]
fn a_click_on_a_web_link_navigates_the_tab_and_a_new_address_ends_the_document() {
    let rig = plain_rig(2);
    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    // The last page's link is `https://example.com/last`.
    rig.pdf.go_to_page_typed("2");
    let (x, y) = rig.page_point(1, 60.0, 42.0);
    rig.wait_for("the last page's links", |rig| {
        rig.with_session(|session| session.link_at(x, y).is_some())
    });
    rig.pdf.pointer(PdfPointer::Down, x, y);
    rig.pdf.pointer(PdfPointer::Up, x, y);
    assert_eq!(
        rig.controller.borrow().active_state().url(),
        "https://example.com/last"
    );
    rig.sync();
    assert!(
        !rig.window.get_pdf_viewer_shown(),
        "the viewer lets the new page show"
    );
}

#[test]
fn save_a_copy_and_open_in_preview_go_through_the_actions() {
    let rig = plain_rig(2);
    let path = rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });

    rig.model().invoke_save_copy();
    assert_eq!(
        *rig.actions.saved.borrow(),
        vec![(rig.tab(), "https://a.test/report.pdf".to_owned())]
    );
    rig.model().invoke_open_externally();
    assert_eq!(
        *rig.actions.opened.borrow(),
        vec![(path, true, "Fake document".to_owned())],
        "a private copy, named by the document's title"
    );
}

#[test]
fn a_document_in_a_background_tab_waits_until_its_tab_is_shown() {
    let rig = plain_rig(2);
    let first = rig.tab();
    rig.controller
        .borrow_mut()
        .open_tab_with(
            Some("https://b.test/"),
            talaria_core::TabActivation::Foreground,
        )
        .expect("a tab");
    let second = rig.tab();
    assert_ne!(first, second);
    // The hand-off gives the first tab (now in the background) its document.
    let file = private_copy_in(rig.folder.path(), "background.pdf");
    rig.pdf.open(
        first,
        &url("https://a.test/background.pdf"),
        PdfFile {
            path: file.path.clone(),
            owned: true,
        },
    );
    rig.controller.borrow_mut().select_tab(first);
    // The tab's address is the document's only once it navigated there.
    rig.controller
        .borrow_mut()
        .queue_navigation("https://a.test/background.pdf")
        .expect("an address");
    assert_eq!(
        rig.pdf.sessions.borrow().live_count(),
        0,
        "nothing started yet"
    );
    rig.sync();
    assert_eq!(
        rig.pdf.sessions.borrow().live_count(),
        1,
        "shown: its helper starts"
    );
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    assert!(rig.window.get_pdf_viewer_shown());

    // Switching to the other tab hides the viewer and keeps the document.
    rig.controller.borrow_mut().select_tab(second);
    rig.sync();
    assert!(!rig.window.get_pdf_viewer_shown());
    assert!(rig.pdf.sessions.borrow().get(first).is_some());
}

#[test]
fn the_diagnostics_scroll_to_a_page_and_search_once_the_document_opens() {
    let rig = rig(
        FakeFactory::new(FakeConfig::with_pages(4)),
        Diagnostics {
            page: Some(3),
            find: Some("quick".to_owned()),
        },
    );
    rig.open("report.pdf");
    rig.wait_for("the document to open", |rig| {
        rig.state() == PdfViewState::Ready
    });
    assert_eq!(rig.model().get_page(), 3);
    assert!(rig.controller.borrow().active_state().find().is_open());
    assert_eq!(
        rig.controller.borrow().active_state().find().query(),
        "quick"
    );
    rig.wait_for("the search to finish", |rig| {
        rig.with_session(|session| {
            session
                .find_progress()
                .is_some_and(|progress| progress.finished)
        })
    });
    assert_eq!(
        rig.with_session(|session| session.find_progress().map(|progress| progress.total)),
        Some(4)
    );
}

#[test]
fn the_sink_says_whether_the_viewer_is_available() {
    let rig = plain_rig(1);
    assert!(Sink.is_available());
    drop(rig);
    assert!(!Sink.is_available(), "no viewer, no PDFs in it");
}

// ----- accessibility of the viewer in the real window --------------------------------------------

mod accessibility {
    use i_slint_backend_testing::{AccessibleRole, ElementHandle};

    use super::*;

    /// The control labelled `label` (the text beside a control may carry the same words).
    fn element(window: &MainWindow, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(window, label)
            .find(|element| element.accessible_role() != Some(AccessibleRole::Text))
            .unwrap_or_else(|| panic!("no control labelled {label:?}"))
    }

    fn shown(rig: &Rig) {
        rig.open("report.pdf");
        rig.wait_for("the document to open", |rig| {
            rig.state() == PdfViewState::Ready
        });
        rig.window.show().expect("the window shows");
    }

    #[test]
    fn every_control_of_the_toolbar_has_a_role_and_a_name() {
        let rig = plain_rig(3);
        shown(&rig);
        for label in [
            "Zoom out",
            "Zoom in",
            "Fit width",
            "Fit page",
            "Rotate clockwise",
            "Find in document",
            "Save a copy",
            "Open in Preview",
        ] {
            assert_eq!(
                element(&rig.window, label).accessible_role(),
                Some(AccessibleRole::Button),
                "{label}"
            );
        }
        assert_eq!(
            element(&rig.window, "PDF toolbar").accessible_role(),
            Some(AccessibleRole::Navigation)
        );
        assert_eq!(
            element(&rig.window, "Page number").accessible_role(),
            Some(AccessibleRole::TextInput)
        );
        assert_eq!(
            element(&rig.window, "PDF document, page 1 of 3").accessible_role(),
            Some(AccessibleRole::Main),
            "the page area says where it is"
        );
    }

    #[test]
    fn the_fit_buttons_say_which_is_on_and_the_zoom_buttons_say_when_they_are_spent() {
        let rig = plain_rig(3);
        shown(&rig);
        assert_eq!(
            element(&rig.window, "Fit width").accessible_checkable(),
            Some(true)
        );
        assert_eq!(
            element(&rig.window, "Fit width").accessible_checked(),
            Some(true)
        );
        assert_eq!(
            element(&rig.window, "Fit page").accessible_checked(),
            Some(false)
        );
        assert_eq!(
            element(&rig.window, "Zoom in").accessible_enabled(),
            Some(true)
        );

        rig.pdf.pinch(500.0);
        rig.sync();
        assert_eq!(
            element(&rig.window, "Zoom in").accessible_enabled(),
            Some(false)
        );
        assert_eq!(
            element(&rig.window, "Zoom out").accessible_enabled(),
            Some(true)
        );
        assert_eq!(
            element(&rig.window, "Fit width").accessible_checked(),
            Some(false)
        );
    }

    #[test]
    fn the_default_actions_do_what_a_click_does() {
        let rig = plain_rig(3);
        shown(&rig);
        element(&rig.window, "Zoom in").invoke_accessible_default_action();
        assert_eq!(rig.controller.borrow().active_state().zoom_percent(), 110);
        element(&rig.window, "Rotate clockwise").invoke_accessible_default_action();
        assert_eq!(
            rig.with_session(|session| session.model().map(|model| model.rotation())),
            Some(Rotation::Deg90)
        );
        element(&rig.window, "Save a copy").invoke_accessible_default_action();
        assert_eq!(rig.actions.saved.borrow().len(), 1);
    }

    #[test]
    fn the_password_prompt_names_its_field_and_its_sheet() {
        let rig = rig(
            FakeFactory::new(FakeConfig {
                password: Some("secret".to_owned()),
                ..FakeConfig::with_pages(2)
            }),
            Diagnostics::default(),
        );
        rig.open("report.pdf");
        rig.wait_for("the prompt", |rig| rig.state() == PdfViewState::Password);
        rig.window.show().expect("the window shows");
        assert_eq!(
            element(&rig.window, "Password").accessible_role(),
            Some(AccessibleRole::TextInput)
        );
        assert_eq!(
            element(&rig.window, "This PDF needs a password").accessible_role(),
            Some(AccessibleRole::Groupbox)
        );
    }
}
