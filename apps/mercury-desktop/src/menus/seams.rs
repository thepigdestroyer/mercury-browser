// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Menu items whose actions other lanes provide. Each item stays disabled until its lane sets
//! the flag below to `true` and fills in the function, which the menus call through
//! `MenuCommand`. Nothing else in the menus needs to change.

use mercury_ui::MainWindow;
use talaria_core::BrowserController;

/// Session lane: File ▸ "Reopen Closed Tab" (Shift+Cmd+T). When the lane adds a
/// `ShortcutAction` for it, change the item's command to `MenuCommand::Shortcut(…)` in
/// `model.rs`; the table test then checks that `shortcuts.rs` owns the key.
pub(crate) const REOPEN_CLOSED_TAB_AVAILABLE: bool = false;
/// Session lane: History ▸ "Restore Previous Session".
pub(crate) const RESTORE_PREVIOUS_SESSION_AVAILABLE: bool = false;

/// Bookmarks lane: the Bookmarks menu's bar entries ("Bookmark This Page…" Cmd+D, "Show
/// Bookmarks Bar" Shift+Cmd+B and "Bookmarks Manager" Option+Cmd+B run the lane's shortcuts).
pub(crate) const BOOKMARKS_AVAILABLE: bool = true;

/// Session lane: reopen the most recently closed tab.
pub(crate) fn reopen_closed_tab(_window: &MainWindow) {}

/// Session lane: `restore-previous-session`.
pub(crate) fn restore_previous_session(_window: &MainWindow) {}

/// Bookmarks lane: a bookmark's stable id in its store, so a menu item still names the same
/// bookmark if the bar changes after the menu opened.
pub(crate) use mercury_store::BookmarkId;

/// Bookmarks lane: the bookmarks bar folder's first level, in order: id and title. Called when
/// the UI syncs (never from inside AppKit); the Bookmarks menu lists them when it opens.
pub(crate) fn bookmarks_bar_items(controller: &BrowserController) -> Vec<(BookmarkId, String)> {
    crate::bookmarks::menu::bookmarks_bar_items(controller)
}

/// Bookmarks lane: open the bookmark `id`: a bookmark in the current tab, a folder in the
/// bookmarks manager; an id that no longer exists is ignored.
pub(crate) fn open_bookmark(window: &MainWindow, id: BookmarkId) {
    crate::bookmarks::menu::open_bookmark(window, id);
}
