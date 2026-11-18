// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The bookmarks bar's entries in the macOS menu bar's Bookmarks menu, which the menus lane
//! owns and builds: its `menus/seams.rs` calls [`bookmarks_bar_items`] and [`open_bookmark`].
//! The menu's three fixed items ("Bookmark This Page…", "Show Bookmarks Bar", "Bookmarks
//! Manager") run this lane's `ShortcutAction`s, so the menu and the keys share one action.
//!
//! [`bookmarks_bar_items`] is called when the UI syncs, never from inside AppKit: it only reads
//! the bar's entries as last read from the database. Items carry the bookmark's id, not its
//! index, so a choice made after the bar changed still opens the bookmark the menu showed, or
//! nothing if it is gone.

use mercury_store::{BookmarkId, BookmarkKind, BookmarkNode};
use mercury_ui::MainWindow;
use talaria_core::BrowserController;

use super::with;

/// The menu items for the bar's entries, in order: id and title. Separators are left out (the
/// seam has no separator item); folders are included and open in the manager.
pub(crate) fn bar_items(nodes: &[BookmarkNode]) -> Vec<(BookmarkId, String)> {
    nodes
        .iter()
        .filter(|node| node.kind != BookmarkKind::Separator)
        .map(|node| {
            let title = if node.title.is_empty() {
                "(untitled)".to_owned()
            } else {
                node.title.clone()
            };
            (node.id, title)
        })
        .collect()
}

/// The bar entry `id`, if the bar still holds it.
pub(crate) fn bar_entry(nodes: &[BookmarkNode], id: BookmarkId) -> Option<&BookmarkNode> {
    nodes.iter().find(|node| node.id == id)
}

/// The bar's first level, as the Bookmarks menu lists it.
pub(crate) fn bookmarks_bar_items(_controller: &BrowserController) -> Vec<(BookmarkId, String)> {
    with(|bookmarks| bar_items(bookmarks.bar.borrow().nodes())).unwrap_or_default()
}

/// Opens the bar entry `id`: a bookmark in the current tab, a folder in the manager. An id the
/// bar no longer holds is ignored.
pub(crate) fn open_bookmark(_window: &MainWindow, id: BookmarkId) {
    with(|bookmarks| {
        let node = bar_entry(bookmarks.bar.borrow().nodes(), id).cloned();
        match node.map(|node| node.kind) {
            Some(BookmarkKind::Bookmark { url }) => bookmarks.open_url(&url, false),
            Some(BookmarkKind::Folder) => super::manager::show_folder(bookmarks, id),
            Some(BookmarkKind::Separator) | None => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i64, title: &str, kind: BookmarkKind) -> BookmarkNode {
        BookmarkNode {
            id: BookmarkId(id),
            guid: String::new(),
            parent: None,
            position: 0,
            title: title.to_owned(),
            kind,
            added_at_ms: 0,
            modified_at_ms: 0,
        }
    }

    fn bar() -> Vec<BookmarkNode> {
        vec![
            node(
                7,
                "A",
                BookmarkKind::Bookmark {
                    url: "https://a.test/".into(),
                },
            ),
            node(8, "", BookmarkKind::Separator),
            node(9, "News", BookmarkKind::Folder),
            node(10, "", BookmarkKind::Folder),
        ]
    }

    #[test]
    fn menu_items_carry_ids_and_leave_out_separators() {
        assert_eq!(
            bar_items(&bar()),
            vec![
                (BookmarkId(7), "A".to_owned()),
                (BookmarkId(9), "News".to_owned()),
                (BookmarkId(10), "(untitled)".to_owned()),
            ]
        );
    }

    #[test]
    fn an_id_names_the_same_entry_after_the_bar_changed() {
        let mut changed = bar();
        changed.remove(0);
        assert_eq!(
            bar_entry(&changed, BookmarkId(9)).map(|n| n.title.as_str()),
            Some("News")
        );
        assert!(
            bar_entry(&changed, BookmarkId(7)).is_none(),
            "gone: ignored"
        );
    }
}
