//! The seam for the macOS menu bar's Bookmarks menu (the menus lane owns the menu; this lane
//! fills it). The menus lane's `menus/seams.rs` should:
//!
//! - set `BOOKMARKS_AVAILABLE = true`;
//! - build the menu from [`bookmarks_menu_entries`] whenever it opens (`menuNeedsUpdate:`, on
//!   the Slint side, never inside AppKit): its first three entries are its fixed items, then a
//!   separator and the bookmarks bar's first level;
//! - call [`activate`] with the entry the user chose. `bookmark_this_page`,
//!   `toggle_bookmarks_bar` and `show_bookmarks_manager` in its seam become
//!   `activate(&BookmarksMenuEntry::…)`, or the functions of the same names in
//!   `crate::bookmarks`; `open_bookmarks_bar_item(window, index)` becomes
//!   [`open_bookmarks_bar_item`]`(index)`.

use mercury_store::{BookmarkKind, BookmarkNode};

use super::star::StarState;
use super::with;

/// One entry of the Bookmarks menu.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "called by the menus lane's Bookmarks seam, not merged yet"
    )
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BookmarksMenuEntry {
    /// "Bookmark This Page…" ⌘D (Ctrl+D): enabled when the page can be bookmarked.
    BookmarkThisPage {
        enabled: bool,
    },
    /// "Show Bookmarks Bar" ⇧⌘B, checked while the bar is shown.
    ShowBookmarksBar {
        checked: bool,
    },
    /// "Bookmarks Manager" ⌥⌘B (Ctrl+Shift+O).
    BookmarksManager,
    Separator,
    /// An entry of the bookmarks bar, by its index in the bar; a folder opens in the manager.
    BarItem {
        index: usize,
        title: String,
        is_folder: bool,
    },
}

/// The menu for the star's state, the bar's visibility and the bar's entries.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "called by the menus lane's Bookmarks seam, not merged yet"
    )
)]
pub(crate) fn menu_entries(
    star: StarState,
    bar_shown: bool,
    bar: &[BookmarkNode],
) -> Vec<BookmarksMenuEntry> {
    let mut entries = vec![
        BookmarksMenuEntry::BookmarkThisPage {
            enabled: star != StarState::Disabled,
        },
        BookmarksMenuEntry::ShowBookmarksBar { checked: bar_shown },
        BookmarksMenuEntry::BookmarksManager,
    ];
    if !bar.is_empty() {
        entries.push(BookmarksMenuEntry::Separator);
    }
    entries.extend(bar.iter().enumerate().map(|(index, node)| match node.kind {
        BookmarkKind::Separator => BookmarksMenuEntry::Separator,
        _ => BookmarksMenuEntry::BarItem {
            index,
            title: if node.title.is_empty() {
                "(untitled)".to_owned()
            } else {
                node.title.clone()
            },
            is_folder: node.kind == BookmarkKind::Folder,
        },
    }));
    entries
}

/// The Bookmarks menu as it should look now.
#[expect(
    dead_code,
    reason = "called by the menus lane's Bookmarks seam, not merged yet"
)]
pub(crate) fn bookmarks_menu_entries() -> Vec<BookmarksMenuEntry> {
    with(|bookmarks| {
        menu_entries(
            bookmarks.star.borrow().state(),
            bookmarks.bar_shown.get(),
            bookmarks.bar.borrow().nodes(),
        )
    })
    .unwrap_or_default()
}

/// Runs a chosen entry.
#[expect(
    dead_code,
    reason = "called by the menus lane's Bookmarks seam, not merged yet"
)]
pub(crate) fn activate(entry: &BookmarksMenuEntry) {
    match entry {
        BookmarksMenuEntry::BookmarkThisPage { .. } => super::bookmark_this_page(),
        BookmarksMenuEntry::ShowBookmarksBar { .. } => super::toggle_bookmarks_bar(),
        BookmarksMenuEntry::BookmarksManager => super::show_bookmarks_manager(),
        BookmarksMenuEntry::Separator => {}
        BookmarksMenuEntry::BarItem { index, .. } => open_bookmarks_bar_item(*index),
    }
}

/// Opens the bar's entry at `index`: a bookmark in the current tab, a folder in the manager.
#[expect(
    dead_code,
    reason = "called by the menus lane's Bookmarks seam, not merged yet"
)]
pub(crate) fn open_bookmarks_bar_item(index: usize) {
    with(|bookmarks| {
        let node = bookmarks.bar.borrow().nodes().get(index).cloned();
        match node.map(|node| (node.id, node.kind)) {
            Some((_, BookmarkKind::Bookmark { url })) => bookmarks.open_url(&url, false),
            Some((id, BookmarkKind::Folder)) => super::manager::show_folder(bookmarks, id),
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use mercury_store::BookmarkId;

    use super::*;

    fn node(title: &str, kind: BookmarkKind) -> BookmarkNode {
        BookmarkNode {
            id: BookmarkId(1),
            guid: String::new(),
            parent: None,
            position: 0,
            title: title.to_owned(),
            kind,
            added_at_ms: 0,
            modified_at_ms: 0,
        }
    }

    #[test]
    fn the_menu_has_its_three_items_then_the_bar() {
        let bar = [
            node(
                "A",
                BookmarkKind::Bookmark {
                    url: "https://a.test/".into(),
                },
            ),
            node("", BookmarkKind::Separator),
            node("News", BookmarkKind::Folder),
        ];
        assert_eq!(
            menu_entries(StarState::Unbookmarked, true, &bar),
            vec![
                BookmarksMenuEntry::BookmarkThisPage { enabled: true },
                BookmarksMenuEntry::ShowBookmarksBar { checked: true },
                BookmarksMenuEntry::BookmarksManager,
                BookmarksMenuEntry::Separator,
                BookmarksMenuEntry::BarItem {
                    index: 0,
                    title: "A".into(),
                    is_folder: false
                },
                BookmarksMenuEntry::Separator,
                BookmarksMenuEntry::BarItem {
                    index: 2,
                    title: "News".into(),
                    is_folder: true
                },
            ]
        );
    }

    #[test]
    fn a_page_that_cannot_be_bookmarked_disables_the_first_item() {
        let entries = menu_entries(StarState::Disabled, false, &[]);
        assert_eq!(
            entries,
            vec![
                BookmarksMenuEntry::BookmarkThisPage { enabled: false },
                BookmarksMenuEntry::ShowBookmarksBar { checked: false },
                BookmarksMenuEntry::BookmarksManager,
            ]
        );
    }
}
