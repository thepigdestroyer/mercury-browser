//! Importing bookmarks and history from other browsers (import-ui lane): the import window
//! and the pipeline behind it.
//!
//! - `sources`: which profiles there are, and where to look (never the real environment in
//!   automated runs, and only when the window opens).
//! - `read` and `convert`: reading a profile on a background thread through `mercury-import`,
//!   with its copies in the profile's private copy area (`copies`).
//! - `run`: sending what was read to the profile-data service in bounded steps, leaving out
//!   what the profile already holds (`profile_data::import_merge`).
//! - `summary`: what happened, in words; `marker`: notes of unfinished imports.
//!
//! The window is opened with [`show_import_window`], the one entry point for the File menu
//! ("Import Bookmarks and Settings…") and the bookmarks manager. "Import bookmarks from a
//! file…" calls [`seams::import_bookmarks_from_file`], which the bookmarks lane fills in.

// Until the File menu (menus lane) calls `show_import_window`, only the `diagnostics` build
// reaches the window.
#![cfg_attr(
    not(feature = "diagnostics"),
    expect(dead_code, reason = "reached through the File menu (menus lane)")
)]

mod convert;
mod copies;
#[cfg(feature = "diagnostics")]
mod diagnostic;
#[cfg(test)]
mod fixtures;
mod marker;
mod read;
mod run;
pub(crate) mod seams;
mod sources;
mod summary;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use mercury_import::ReadContext;
use mercury_ui::{ImportPage, ImportProfileRow, ImportSummaryLine, ImportWindow, MainWindow};
use slint::{ComponentHandle, ModelRc, VecModel};

#[cfg(feature = "diagnostics")]
pub(crate) use diagnostic::run_diagnostic_import;
use read::{Choice, Prepared, read_profile};
use run::{ImportRun, Stage};
use sources::{FoundProfile, Search, directories, discover, search_for};
use summary::{ProfileResult, summary_lines};

thread_local! {
    static IMPORT: RefCell<ImportUi> = RefCell::new(ImportUi::default());
}

#[derive(Default)]
struct ImportUi {
    search: Option<Search>,
    /// Mercury's profile directory, where the notes of unfinished imports are kept.
    profile_dir: Option<PathBuf>,
    /// Where reads copy the other browser's databases; `None` if it could not be prepared,
    /// in which case reads fail and say so.
    copies: Option<PathBuf>,
    window: Option<ImportWindow>,
    found: Vec<FoundProfile>,
    run: Option<ImportRun>,
    /// The read in progress (discovery or a profile's files), to cancel it.
    reading: Option<ReadContext>,
    /// Counts window openings and runs, so that late results of an earlier one are dropped.
    generation: u64,
}

fn with<T>(f: impl FnOnce(&mut ImportUi) -> T) -> T {
    IMPORT.with(|ui| f(&mut ui.borrow_mut()))
}

fn current_window() -> Option<ImportWindow> {
    with(|ui| ui.window.as_ref().map(ComponentHandle::clone_strong))
}

/// Called once at start-up: where discovery may look in this run, and where the profile is
/// (`default_profile` when it is the user's default profile rather than a `--profile-dir` or a
/// temporary one). Prepares the copy area, removing what an earlier run left in it.
pub(crate) fn configure(
    automated: bool,
    fixture: Option<&Path>,
    profile_dir: Option<PathBuf>,
    default_profile: bool,
) {
    let copies = profile_dir.as_deref().and_then(|root| {
        let cache = copies::mercury_cache_dir();
        let area = copies::area_for(root, default_profile && cache.is_some(), cache.as_deref());
        match copies::prepare(&area) {
            Ok(()) => Some(area),
            Err(error) => {
                log::warn!("could not prepare the import copy area: {error}");
                None
            }
        }
    });
    with(|ui| {
        ui.search = Some(search_for(automated, fixture));
        ui.profile_dir = profile_dir;
        ui.copies = copies;
    });
}

/// Where the window looks for profiles in this run.
fn window_search() -> Search {
    with(|ui| ui.search.clone().unwrap_or(Search::Nowhere))
}

/// A context for a new read, remembered so that it can be cancelled.
fn new_read() -> ReadContext {
    with(|ui| {
        let context = ReadContext::new(
            ui.copies
                .clone()
                .unwrap_or_else(|| PathBuf::from("/nonexistent/import-copies")),
        );
        if let Some(previous) = ui.reading.replace(context.clone()) {
            previous.cancel();
        }
        context
    })
}

/// Opens the import window, or brings it forward if it is open. Discovery starts now, on a
/// background thread; nothing is looked for before.
#[expect(dead_code, reason = "called by the File menu (menus lane)")]
pub(crate) fn show_import_window(_main_window: &MainWindow) {
    open_window();
}

fn open_window() {
    if let Some(window) = current_window() {
        if let Err(error) = window.show() {
            log::warn!("could not show the import window: {error}");
        }
        return;
    }
    let window = match ImportWindow::new() {
        Ok(window) => window,
        Err(error) => {
            log::error!("could not create the import window: {error}");
            return;
        }
    };
    connect(&window);
    window.set_page(ImportPage::Looking);
    window.set_can_import_from_file(seams::IMPORT_FROM_FILE_AVAILABLE);
    let search = window_search();
    let context = new_read();
    let generation = with(|ui| {
        ui.generation += 1;
        ui.window = Some(window.clone_strong());
        ui.generation
    });
    if let Err(error) = window.show() {
        log::error!("could not show the import window: {error}");
    }
    spawn("import discovery", move || {
        let found = discover(&directories(&search), &context);
        let _queued = slint::invoke_from_event_loop(move || discovered(generation, found));
    });
}

/// Starts `work` on its own thread, never waited for: at quit its read is cancelled, and what
/// it leaves in the copy area is removed at the next start.
fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        log::error!("could not start the {name} thread: {error}");
    }
}

fn connect(window: &ImportWindow) {
    window.on_start_import(start_selected);
    window.on_cancel_import(cancel);
    window.on_continue_import(continue_unfinished);
    window.on_import_from_file(|| {
        if let Some(window) = current_window() {
            seams::import_bookmarks_from_file(&window);
        }
    });
    window.on_close_window(close_later);
    window.window().on_close_requested(|| {
        close_later();
        slint::CloseRequestResponse::KeepWindowShown
    });
}

fn discovered(generation: u64, found: Vec<FoundProfile>) {
    let (window, profile_dir) = with(|ui| {
        if ui.generation != generation {
            return (None, None);
        }
        ui.found = found.clone();
        ui.reading = None;
        (
            ui.window.as_ref().map(ComponentHandle::clone_strong),
            ui.profile_dir.clone(),
        )
    });
    let Some(window) = window else {
        return;
    };
    let rows = found
        .iter()
        .map(|profile| ImportProfileRow {
            browser: profile.browser.clone().into(),
            name: profile.name.clone().into(),
            holds: profile.holds().into(),
            bookmarks: profile.bookmarks,
            history: profile.history,
        })
        .collect::<Vec<_>>();
    window.set_profiles(ModelRc::new(VecModel::from(rows)));
    if let Some(first) = found.first() {
        window.set_selected(0);
        window.set_import_bookmarks(first.bookmarks);
        window.set_import_history(first.history);
    }
    let notice = unfinished(profile_dir.as_deref(), &found)
        .map(|(profile, _)| format!("The import from {} did not finish.", profile.label()))
        .unwrap_or_default();
    window.set_unfinished_notice(notice.into());
    window.set_page(if found.is_empty() {
        ImportPage::NothingFound
    } else {
        ImportPage::Choose
    });
    #[cfg(feature = "diagnostics")]
    diagnostic::after_discovery();
}

/// The first unfinished import whose profile is among those found, and what it imported.
fn unfinished(
    profile_dir: Option<&Path>,
    found: &[FoundProfile],
) -> Option<(FoundProfile, Choice)> {
    marker::load(profile_dir?).into_iter().find_map(|note| {
        let profile = found.iter().find(|profile| profile.path == note.path)?;
        Some((
            profile.clone(),
            Choice {
                bookmarks: note.bookmarks && profile.bookmarks,
                history: note.history && profile.history,
            },
        ))
    })
}

fn start_selected() {
    let Some(window) = current_window() else {
        return;
    };
    let index = usize::try_from(window.get_selected()).unwrap_or(0);
    let Some(profile) = with(|ui| ui.found.get(index).cloned()) else {
        return;
    };
    let choice = Choice {
        bookmarks: window.get_import_bookmarks() && profile.bookmarks,
        history: window.get_import_history() && profile.history,
    };
    begin(&window, profile, choice);
}

fn continue_unfinished() {
    let (window, profile_dir, found) = with(|ui| {
        (
            ui.window.as_ref().map(ComponentHandle::clone_strong),
            ui.profile_dir.clone(),
            ui.found.clone(),
        )
    });
    let Some(window) = window else {
        return;
    };
    if let Some((profile, choice)) = unfinished(profile_dir.as_deref(), &found) {
        begin(&window, profile, choice);
    }
}

fn begin(window: &ImportWindow, profile: FoundProfile, choice: Choice) {
    if !choice.bookmarks && !choice.history || with(|ui| ui.run.is_some()) {
        return;
    }
    let label = profile.label();
    let profile_dir = with(|ui| ui.profile_dir.clone());
    if let Some(dir) = &profile_dir {
        let note = marker::Unfinished {
            browser: profile.browser.clone(),
            name: profile.name.clone(),
            path: profile.path.clone(),
            bookmarks: choice.bookmarks,
            history: choice.history,
        };
        if let Err(error) = marker::add(dir, &note) {
            log::warn!("could not note the import as unfinished: {error}");
        }
    }
    window.set_cancelling(false);
    window.set_page(ImportPage::Importing);
    show_stage(window, &label, Stage::Reading);
    let generation = with(|ui| {
        ui.generation += 1;
        ui.generation
    });
    let stage_label = label.clone();
    let source = profile.path.clone();
    let run = ImportRun::new(
        label,
        profile.browser.clone(),
        Rc::new(crate::profile_data::handle),
        move |stage| {
            if let Some(window) = window_of(generation) {
                show_stage(&window, &stage_label, stage);
            }
        },
        move |result| finished(generation, &source, result),
    );
    with(|ui| ui.run = Some(run));
    let context = new_read();
    spawn("import reader", move || {
        let prepared = read_profile(&profile, choice, &context, crate::profile_data::now_ms());
        let _queued = slint::invoke_from_event_loop(move || read_done(generation, prepared));
    });
}

/// The window, if `generation` is still the current one.
fn window_of(generation: u64) -> Option<ImportWindow> {
    with(|ui| {
        (ui.generation == generation)
            .then(|| ui.window.as_ref().map(ComponentHandle::clone_strong))
            .flatten()
    })
}

fn show_stage(window: &ImportWindow, label: &str, stage: Stage) {
    window.set_progress_title(stage.title(label).into());
    window.set_progress_detail(stage.detail().into());
    window.set_progress_known(stage.fraction().is_some());
    window.set_progress(stage.fraction().unwrap_or(0.0));
}

fn read_done(generation: u64, prepared: Prepared) {
    let run = with(|ui| {
        if ui.generation == generation {
            ui.reading = None;
            ui.run.clone()
        } else {
            None
        }
    });
    if let Some(run) = run {
        run.prepared(prepared);
    }
}

/// A run ended. The note of its profile is updated whatever happened since; the window only
/// if it still shows this run (`generation`).
fn finished(generation: u64, source: &Path, result: ProfileResult) {
    let profile_dir = with(|ui| ui.profile_dir.clone());
    if let Some(dir) = &profile_dir
        && !result.interrupted()
        && let Err(error) = marker::remove(dir, source)
    {
        log::warn!("could not remove the note of an unfinished import: {error}");
    }
    let current = with(|ui| {
        let current = ui.generation == generation;
        if current {
            ui.run = None;
        }
        current
    });
    if !current {
        return;
    }
    let lines = summary_lines(&result);
    if let Some(window) = current_window() {
        window.set_summary_title(
            if result.interrupted() {
                "Import stopped"
            } else if result.failed() {
                "Import failed"
            } else {
                "Import finished"
            }
            .into(),
        );
        window.set_summary(ModelRc::new(VecModel::from(
            lines
                .iter()
                .map(|line| ImportSummaryLine {
                    text: line.text.clone().into(),
                    problem: line.problem,
                })
                .collect::<Vec<_>>(),
        )));
        window.set_unfinished_notice(Default::default());
        window.set_page(ImportPage::Summary);
    }
    #[cfg(feature = "diagnostics")]
    diagnostic::after_profile(&lines);
}

fn cancel() {
    let (window, run, reading) = with(|ui| {
        (
            ui.window.as_ref().map(ComponentHandle::clone_strong),
            ui.run.clone(),
            ui.reading.clone(),
        )
    });
    if let Some(reading) = reading {
        reading.cancel();
    }
    if let Some(run) = run {
        if let Some(window) = window {
            window.set_cancelling(true);
        }
        run.cancel();
    }
}

/// Hides the window and drops it once its callback has returned. An import still running is
/// cancelled between steps.
fn close_later() {
    cancel();
    let window = with(|ui| {
        ui.generation += 1;
        ui.run = None;
        ui.window.take()
    });
    if let Some(window) = window {
        if let Err(error) = window.hide() {
            log::warn!("could not hide the import window: {error}");
        }
        slint::Timer::single_shot(Duration::ZERO, move || drop(window));
    }
}

/// The browser window is closing: close the import window too, so that the event loop ends.
pub(crate) fn main_window_closing() {
    let open = with(|ui| ui.window.is_some());
    if open {
        close_later();
    }
}

/// At quit, after the event loop: tells a read in progress to stop, without waiting for it.
/// Its copy is removed when it stops, or, if the process ends first, at the next start
/// (`copies::prepare`). The note of an unfinished import stays.
pub(crate) fn shut_down() {
    let reading = with(|ui| {
        ui.run = None;
        ui.window = None;
        ui.reading.take()
    });
    if let Some(reading) = reading {
        reading.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_automated_run_window_searches_nowhere_and_a_normal_one_the_platform() {
        configure(true, None, None, false);
        assert_eq!(window_search(), Search::Nowhere);
        assert_eq!(directories(&window_search()), Vec::<PathBuf>::new());

        let fixture = tempfile::tempdir().unwrap();
        configure(true, Some(fixture.path()), None, false);
        assert_eq!(window_search(), Search::Fixture(fixture.path().to_owned()));

        // Only compared: `directories(&Search::Platform)` is never called in tests.
        configure(false, None, None, false);
        assert_eq!(window_search(), Search::Platform);
    }

    #[test]
    fn configuring_a_profile_prepares_its_copy_area_inside_it() {
        let profile = tempfile::tempdir().unwrap();
        let leftover = profile.path().join("import-copies/mercury-import-x");
        std::fs::create_dir_all(&leftover).unwrap();

        configure(true, None, Some(profile.path().to_owned()), false);

        assert_eq!(
            with(|ui| ui.copies.clone()),
            Some(profile.path().join("import-copies"))
        );
        assert!(!leftover.exists());
        assert_eq!(new_read().copy_dir(), profile.path().join("import-copies"));
    }
}
