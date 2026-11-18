// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Bookmarks: the star button, the bookmarks bar, the bookmarks manager and bookmark files.
//!
//! Everything runs on the UI thread and talks to the profile database through the
//! profile-data service. The state lives in one thread-local [`Bookmarks`]; Slint callbacks
//! and service replies reach it through [`with`], and never keep it borrowed across a call
//! that could come back into it.
//!
//! **Refreshing:** the views (the bar, the manager's lists, the star) are re-read only when the
//! service reports a change to the bookmarks, which also follows this module's own writes; the
//! replies to those writes update only what the write itself decided (the star of the page it
//! bookmarked, the selection, the undo entry), so no edit is read back twice.

pub(crate) mod bar;
pub(crate) mod file;
pub(crate) mod import_plan;
pub(crate) mod layout;
pub(crate) mod manager;
pub(crate) mod menu;
pub(crate) mod popover;
pub(crate) mod star;
pub(crate) mod tree;

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use mercury_store::{BookmarkId, BookmarkRoot};
use mercury_ui::MainWindow;
use slint::ComponentHandle;
use talaria_core::{BrowserController, LoadState, NavigationOrigin, TabActivation};

use crate::profile_data::{self, Area};
use crate::settings::SettingsHandle;
use crate::ui_sync::sync_ui;

/// The root new bookmarks from the star go into: "Other Bookmarks". The bar stays what the
/// user put there instead of growing with every starred page (Firefox and Safari keep their
/// bars curated the same way); the popover's folder chooser moves a bookmark to the bar.
pub(crate) const STAR_ROOT: BookmarkRoot = BookmarkRoot::Other;

thread_local! {
    static BOOKMARKS: RefCell<Option<Rc<Bookmarks>>> = const { RefCell::new(None) };
}

/// The bookmarks features of the browser window.
pub(crate) struct Bookmarks {
    window: slint::Weak<MainWindow>,
    controller: Rc<RefCell<BrowserController>>,
    settings: Weak<SettingsHandle>,
    star: RefCell<star::StarModel>,
    error_page_titles: Vec<String>,
    popover: RefCell<Option<popover::Popover>>,
    bar: RefCell<bar::Bar>,
    bar_shown: Cell<bool>,
    manager: RefCell<Option<Rc<manager::Manager>>>,
}

/// Runs `action` with the bookmarks state, if it is set up.
pub(crate) fn with<R>(action: impl FnOnce(&Rc<Bookmarks>) -> R) -> Option<R> {
    let bookmarks = BOOKMARKS.with(|current| current.borrow().clone())?;
    Some(action(&bookmarks))
}

/// The appearance setting "show bookmarks bar".
fn show_bookmarks_bar_setting(settings: &SettingsHandle) -> bool {
    settings.settings().appearance.show_bookmarks_bar
}

/// Drops the bookmarks state, the manager window with it. Called at the end of the run, with
/// the other windows (`post_run.rs`); it only drops.
pub(crate) fn release() {
    let bookmarks = BOOKMARKS.with(|current| current.borrow_mut().take());
    if let Some(bookmarks) = bookmarks {
        bookmarks.manager.borrow_mut().take();
    }
}

/// Wires the star, the bar and their popups to the window, and starts listening for changes.
/// Call once, after `profile_data::start` and the window's other callbacks.
pub(crate) fn connect(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    settings: &Rc<SettingsHandle>,
) {
    let bookmarks = Rc::new(Bookmarks {
        window: window.as_weak(),
        controller,
        settings: Rc::downgrade(settings),
        star: RefCell::new(star::StarModel::default()),
        error_page_titles: talaria_servo::error_page_titles(),
        popover: RefCell::new(None),
        bar: RefCell::new(bar::Bar::new()),
        bar_shown: Cell::new(show_bookmarks_bar_setting(settings)),
        manager: RefCell::new(None),
    });
    BOOKMARKS.with(|current| *current.borrow_mut() = Some(Rc::clone(&bookmarks)));
    window.set_bookmarks_bar_shown(bookmarks.bar_shown.get());
    // The bar follows the setting, whoever changes it (Cmd+Shift+B, the menu, the settings
    // window). The listener reaches the state through `with`: it holds nothing itself.
    settings.subscribe(|| {
        with(|bookmarks| bookmarks.follow_bar_setting());
    });
    popover::connect(window);
    bar::connect(window);
    if let Some(data) = profile_data::handle() {
        // `handle()` inside the listener rather than a captured clone: no `Rc` cycle.
        let _listener = data.on_change(Area::Bookmarks, |change| {
            let folders = change.folders.clone();
            with(|bookmarks| bookmarks.bookmarks_changed(folders.as_deref()));
        });
    }
    bookmarks.reload_bar();
    let controller = bookmarks.controller.borrow();
    bookmarks.sync_star(window, &controller);
}

/// Called by `sync_ui` whenever the window shows the controller's state: the star follows the
/// active tab's page (every committed navigation and every tab switch passes here).
pub(crate) fn sync(window: &MainWindow, controller: &BrowserController) {
    with(|bookmarks| bookmarks.sync_star(window, controller));
}

/// Cmd+D (Ctrl+D elsewhere), the star, and the Bookmarks menu's "Bookmark This Page".
pub(crate) fn bookmark_this_page() {
    with(popover::star_clicked);
}

/// Cmd+Shift+B (Ctrl+Shift+B elsewhere) and the menu's "Show Bookmarks Bar".
/// Saves the opposite of what the bar shows now; the bar follows when the setting changes.
pub(crate) fn toggle_bookmarks_bar() {
    with(|bookmarks| {
        let show = !bookmarks.bar_shown.get();
        match bookmarks.settings.upgrade() {
            Some(settings) => settings.update(|settings| {
                settings.appearance.show_bookmarks_bar = show;
            }),
            // No settings in this run: the bar changes for the session only.
            None => bookmarks.show_bar(show),
        }
    });
}

/// Cmd+Alt+B on macOS (Ctrl+Shift+O elsewhere) and the menu's "Bookmarks Manager".
pub(crate) fn show_bookmarks_manager() {
    with(manager::show);
}

impl Bookmarks {
    fn follow_bar_setting(&self) {
        if let Some(settings) = self.settings.upgrade() {
            self.show_bar(show_bookmarks_bar_setting(&settings));
        }
    }

    fn show_bar(&self, show: bool) {
        if self.bar_shown.replace(show) == show {
            return;
        }
        if let Some(window) = self.window.upgrade() {
            window.set_bookmarks_bar_shown(show);
            window.window().request_redraw();
        }
    }

    /// The bookmarks changed; `folders` are the folders whose listing changed (`None`: any
    /// may have). An empty hint (a write that found nothing to change) refreshes nothing; the
    /// bar is read again only when its own folder is among those changed.
    fn bookmarks_changed(self: &Rc<Self>, folders: Option<&[BookmarkId]>) {
        if folders.is_some_and(<[BookmarkId]>::is_empty) {
            return;
        }
        if let Some(lookup) = self.star.borrow_mut().refresh() {
            self.look_up_star(lookup);
        }
        if bar::affected_by(self.bar.borrow().root(), folders) {
            self.reload_bar();
        }
        bar::folder_changed(self, folders);
        let manager = self.manager.borrow().clone();
        if let Some(manager) = manager {
            manager.changed();
        }
    }

    fn sync_star(self: &Rc<Self>, window: &MainWindow, controller: &BrowserController) {
        let state = controller.active_state();
        // Bookmarks never keep a user name or password; the page is looked up without them.
        let page = star::bookmarkable_url(
            state.url(),
            state.page_title(),
            matches!(state.load_state(), LoadState::Failed { .. }),
            &self.error_page_titles,
        )
        .map(tree::without_credentials);
        let lookup = self.star.borrow_mut().show_page(page.as_deref());
        self.show_star(window);
        if let Some(lookup) = lookup {
            self.look_up_star(lookup);
        }
    }

    fn look_up_star(self: &Rc<Self>, lookup: star::Lookup) {
        let Some(data) = profile_data::handle() else {
            return;
        };
        let token = lookup.token;
        data.bookmark_for_url(lookup.url, move |result| {
            with(|bookmarks| {
                let answer = result.map_err(|error| log::debug!("star lookup failed: {error}"));
                if bookmarks.star.borrow_mut().answer(token, answer)
                    && let Some(window) = bookmarks.window.upgrade()
                {
                    bookmarks.show_star(&window);
                }
            });
        });
    }

    fn show_star(&self, window: &MainWindow) {
        let state = self.star.borrow().state();
        window.set_bookmark_star_enabled(state != star::StarState::Disabled);
        window.set_bookmark_star_filled(matches!(state, star::StarState::Bookmarked(_)));
    }

    fn reload_bar(self: &Rc<Self>) {
        bar::reload(self);
    }

    /// Opens `url` from a bookmark: in the active tab, or in a new background tab.
    pub(crate) fn open_url(&self, url: &str, new_tab: bool) {
        let opened = {
            let mut controller = self.controller.borrow_mut();
            if new_tab {
                controller
                    .open_tab_with(Some(url), TabActivation::Background)
                    .map(|tab| controller.set_navigation_origin(tab, NavigationOrigin::Bookmark))
            } else {
                controller.queue_navigation(url).map(|()| {
                    let tab = controller.active_tab_id();
                    controller.set_navigation_origin(tab, NavigationOrigin::Bookmark)
                })
            }
        };
        if let Err(error) = opened {
            log::warn!("the bookmark could not be opened: {error}");
            return;
        }
        if let Some(window) = self.window.upgrade() {
            sync_ui(&window, &self.controller.borrow());
            window.window().request_redraw();
        }
    }
}
