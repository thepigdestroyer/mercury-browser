// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Diagnostic (`--import-from <directory>`, only in builds with the `diagnostics` feature):
//! opens the import window, imports everything every profile found under the directory holds,
//! prints each summary, saves a picture of the window if asked, and quits. Released builds
//! cannot import a whole directory without asking.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mercury_ui::ImportWindow;
use slint::ComponentHandle;

use super::read::Choice;
use super::summary::SummaryLine;
use super::{begin, current_window, open_window, with};

struct Diagnostic {
    next: usize,
    snapshot: Option<PathBuf>,
    failed: bool,
}

thread_local! {
    static DIAGNOSTIC: RefCell<Option<Diagnostic>> = const { RefCell::new(None) };
}

/// Starts the diagnostic import; `configure` must have been given the directory.
pub(crate) fn run_diagnostic_import(snapshot: Option<PathBuf>) {
    DIAGNOSTIC.with(|diagnostic| {
        *diagnostic.borrow_mut() = Some(Diagnostic {
            next: 0,
            snapshot,
            failed: false,
        });
    });
    open_window();
}

fn active() -> bool {
    DIAGNOSTIC.with(|diagnostic| diagnostic.borrow().is_some())
}

pub(super) fn after_discovery() {
    if active() {
        next();
    }
}

pub(super) fn after_profile(lines: &[SummaryLine]) {
    if !active() {
        return;
    }
    for line in lines {
        println!(
            "mercury-browser: import: {}{}",
            if line.problem { "problem: " } else { "" },
            line.text
        );
    }
    DIAGNOSTIC.with(|diagnostic| {
        if let Some(diagnostic) = diagnostic.borrow_mut().as_mut() {
            diagnostic.failed |= lines.iter().any(|line| line.problem);
        }
    });
    next();
}

fn next() {
    let index = DIAGNOSTIC.with(|diagnostic| {
        let mut diagnostic = diagnostic.borrow_mut();
        let state = diagnostic.as_mut()?;
        state.next += 1;
        Some(state.next - 1)
    });
    let (Some(index), Some(window)) = (index, current_window()) else {
        let _quit = slint::quit_event_loop();
        return;
    };
    match with(|ui| ui.found.get(index).cloned()) {
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
            let diagnostic = DIAGNOSTIC.with(|diagnostic| diagnostic.borrow_mut().take());
            // Let the last page lay out and draw before it is captured.
            slint::Timer::single_shot(Duration::from_millis(300), move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
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
