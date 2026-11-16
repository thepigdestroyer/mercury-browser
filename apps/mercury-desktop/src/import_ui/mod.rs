//! Importing bookmarks and history from other browsers (import-ui lane): the import window
//! and the pipeline behind it.
//!
//! - `sources`: which profiles there are, and where to look (never the real environment in
//!   automated runs, and only when the window opens).
//! - `read` and `convert`: reading a profile on a background thread through `mercury-import`.
//! - `run`: sending what was read to the profile-data service in chunks, leaving out what the
//!   profile already holds (`profile_data::import_merge`).
//! - `summary`: what happened, in words; `marker`: a note of an unfinished import.
//!
//! The window is opened with [`show_import_window`], the one entry point for the File menu
//! ("Import Bookmarks and Settings…") and the bookmarks manager. "Import bookmarks from a
//! file…" calls [`seams::import_bookmarks_from_file`], which the bookmarks lane fills in.

mod convert;
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
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mercury_ui::{ImportPage, ImportProfileRow, ImportSummaryLine, ImportWindow, MainWindow};
use slint::{ComponentHandle, ModelRc, VecModel};

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
    /// Mercury's profile directory, where the note of an unfinished import is kept.
    profile_dir: Option<PathBuf>,
    window: Option<ImportWindow>,
    found: Vec<FoundProfile>,
    run: Option<ImportRun>,
    /// Discovery or reading, on its own thread.
    worker: Option<JoinHandle<()>>,
    /// Counts window openings and runs, so that late results of an earlier one are dropped.
    generation: u64,
    diagnostic: Option<Diagnostic>,
}

/// `--import-from`: import every profile found, print the summaries, then quit.
struct Diagnostic {
    next: usize,
    snapshot: Option<PathBuf>,
    failed: bool,
}

fn with<T>(f: impl FnOnce(&mut ImportUi) -> T) -> T {
    IMPORT.with(|ui| f(&mut ui.borrow_mut()))
}

/// Called once at start-up: where discovery may look in this run, and where the profile is.
pub(crate) fn configure(automated: bool, fixture: Option<&Path>, profile_dir: Option<PathBuf>) {
    with(|ui| {
        ui.search = Some(search_for(automated, fixture));
        ui.profile_dir = profile_dir;
    });
}

/// Opens the import window, or brings it forward if it is open. Discovery starts now, on a
/// background thread; nothing is looked for before.
#[expect(dead_code, reason = "called by the File menu (menus lane)")]
pub(crate) fn show_import_window(_main_window: &MainWindow) {
    open_window();
}

fn open_window() {
    let existing = with(|ui| ui.window.as_ref().map(ComponentHandle::clone_strong));
    if let Some(window) = existing {
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
    let search = with(|ui| {
        ui.generation += 1;
        ui.window = Some(window.clone_strong());
        ui.search.clone().unwrap_or(Search::Nowhere)
    });
    let generation = with(|ui| ui.generation);
    if let Err(error) = window.show() {
        log::error!("could not show the import window: {error}");
    }
    spawn_worker("import discovery", move || {
        let found = discover(&directories(&search));
        let _queued = slint::invoke_from_event_loop(move || discovered(generation, found));
    });
}

fn spawn_worker(name: &str, work: impl FnOnce() + Send + 'static) {
    let previous = with(|ui| ui.worker.take());
    if let Some(previous) = previous
        && !previous.is_finished()
    {
        // A read the user walked away from: it ends on its own, and its result is dropped.
        log::debug!("an earlier import thread is still reading");
    }
    match std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        Ok(handle) => with(|ui| ui.worker = Some(handle)),
        Err(error) => log::error!("could not start the {name} thread: {error}"),
    }
}

fn connect(window: &ImportWindow) {
    window.on_start_import(start_selected);
    window.on_cancel_import(cancel);
    window.on_continue_import(continue_unfinished);
    window.on_import_from_file(|| {
        if let Some(window) = with(|ui| ui.window.as_ref().map(ComponentHandle::clone_strong)) {
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
    if with(|ui| ui.diagnostic.is_some()) {
        diagnostic_next();
    }
}

/// The profile of an unfinished import, if it is among those found, and what it imported.
fn unfinished<'a>(
    profile_dir: Option<&Path>,
    found: &'a [FoundProfile],
) -> Option<(&'a FoundProfile, Choice)> {
    let note = marker::load(profile_dir?)?;
    let profile = found.iter().find(|profile| profile.path == note.path)?;
    Some((
        profile,
        Choice {
            bookmarks: note.bookmarks && profile.bookmarks,
            history: note.history && profile.history,
        },
    ))
}

fn start_selected() {
    let Some(window) = with(|ui| ui.window.as_ref().map(ComponentHandle::clone_strong)) else {
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
        begin(&window, profile.clone(), choice);
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
        if let Err(error) = marker::save(dir, &note) {
            log::warn!("could not note the import as unfinished: {error}");
        }
    }
    window.set_cancelling(false);
    window.set_page(ImportPage::Importing);
    show_stage(window, &label, Stage::Reading);
    let stage_label = label.clone();
    let run = ImportRun::new(
        label,
        profile.browser.clone(),
        Rc::new(crate::profile_data::handle),
        move |stage| {
            if let Some(window) = with(|ui| ui.window.as_ref().map(ComponentHandle::clone_strong)) {
                show_stage(&window, &stage_label, stage);
            }
        },
        finished,
    );
    let generation = with(|ui| {
        ui.generation += 1;
        ui.run = Some(run);
        ui.generation
    });
    spawn_worker("import reader", move || {
        let prepared = read_profile(&profile, choice);
        let _queued = slint::invoke_from_event_loop(move || read_done(generation, prepared));
    });
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
            ui.run.clone()
        } else {
            None
        }
    });
    if let Some(run) = run {
        run.prepared(prepared);
    }
}

fn finished(result: ProfileResult) {
    let (window, profile_dir) = with(|ui| {
        ui.run = None;
        (
            ui.window.as_ref().map(ComponentHandle::clone_strong),
            ui.profile_dir.clone(),
        )
    });
    if let Some(dir) = &profile_dir
        && !result.interrupted()
        && let Err(error) = marker::clear(dir)
    {
        log::warn!("could not remove the note of an unfinished import: {error}");
    }
    let lines = summary_lines(&result);
    if let Some(window) = &window {
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
    if with(|ui| ui.diagnostic.is_some()) {
        for line in &lines {
            println!(
                "mercury-browser: import: {}{}",
                if line.problem { "problem: " } else { "" },
                line.text
            );
        }
        with(|ui| {
            if let Some(diagnostic) = ui.diagnostic.as_mut() {
                diagnostic.failed |= lines.iter().any(|line| line.problem);
            }
        });
        diagnostic_next();
    }
}

fn cancel() {
    let (window, run) = with(|ui| {
        (
            ui.window.as_ref().map(ComponentHandle::clone_strong),
            ui.run.clone(),
        )
    });
    if let Some(run) = run {
        if let Some(window) = window {
            window.set_cancelling(true);
        }
        run.cancel();
    }
}

/// Hides the window and drops it once its callback has returned. An import still running is
/// cancelled between chunks.
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

/// At quit, after the event loop: waits for a reading thread to finish (its copies of the
/// other browser's files are removed when it does), until `deadline` at the latest.
pub(crate) fn shut_down(deadline: Instant) {
    let worker = with(|ui| {
        ui.run = None;
        ui.window = None;
        ui.worker.take()
    });
    if let Some(worker) = worker {
        while !worker.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if worker.is_finished() {
            let _joined = worker.join();
        } else {
            log::warn!("an import thread was still reading at quit");
        }
    }
}

/// Diagnostic (`--import-from <directory>`, with `configure` given the same directory):
/// opens the import window, imports everything every profile found holds, prints each
/// summary, saves a picture of the window to `snapshot` if given, and quits.
pub(crate) fn run_diagnostic_import(snapshot: Option<PathBuf>) {
    with(|ui| {
        ui.diagnostic = Some(Diagnostic {
            next: 0,
            snapshot,
            failed: false,
        });
    });
    open_window();
}

fn diagnostic_next() {
    let (window, profile, index) = with(|ui| {
        let next = ui
            .diagnostic
            .as_ref()
            .map_or(0, |diagnostic| diagnostic.next);
        if let Some(diagnostic) = ui.diagnostic.as_mut() {
            diagnostic.next += 1;
        }
        (
            ui.window.as_ref().map(ComponentHandle::clone_strong),
            ui.found.get(next).cloned(),
            next,
        )
    });
    let Some(window) = window else {
        let _quit = slint::quit_event_loop();
        return;
    };
    match profile {
        Some(profile) => {
            window.set_selected(i32::try_from(index).unwrap_or(0));
            let choice = Choice {
                bookmarks: profile.bookmarks,
                history: profile.history,
            };
            begin(&window, profile, choice);
        }
        None => {
            if index == 0 {
                println!("mercury-browser: import: no profile was found");
            }
            let diagnostic = with(|ui| ui.diagnostic.take());
            // Let the last page lay out and draw before it is captured.
            slint::Timer::single_shot(Duration::from_millis(300), move || {
                if let Some(path) = diagnostic.as_ref().and_then(|d| d.snapshot.as_ref()) {
                    save_snapshot(&window, path);
                }
                if diagnostic.is_some_and(|d| d.failed) {
                    println!("mercury-browser: import: finished with problems");
                } else {
                    println!("mercury-browser: import: finished");
                }
                let _quit = slint::quit_event_loop();
            });
        }
    }
}

fn save_snapshot(window: &ImportWindow, path: &Path) {
    let saved = window
        .window()
        .take_snapshot()
        .map_err(|error| error.to_string())
        .and_then(|pixels| {
            image::save_buffer_with_format(
                path,
                pixels.as_bytes(),
                pixels.width(),
                pixels.height(),
                image::ExtendedColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .map_err(|error| error.to_string())
        });
    match saved {
        Ok(()) => println!("mercury-browser: import: saved {}", path.display()),
        Err(error) => eprintln!(
            "mercury-browser: could not save the import window to {}: {error}",
            path.display()
        ),
    }
}
