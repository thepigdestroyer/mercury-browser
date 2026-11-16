//! The star's popover: name and folder of the page's bookmark, Done and Remove.

use std::rc::Rc;

use mercury_store::{BookmarkId, BookmarkNode};
use mercury_ui::MainWindow;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::star::StarState;
use super::tree::{self, FolderRow};
use super::{Bookmarks, STAR_ROOT, with};
use crate::profile_data;
use crate::profile_data::bookmarks::BookmarkEdit;

/// The bookmark the open popover edits, as it was when the popover opened.
#[derive(Debug)]
pub(crate) struct Popover {
    id: BookmarkId,
    url: String,
    title: String,
    folders: Vec<FolderRow>,
    folder_index: Option<usize>,
}

/// What closing the popover changes: a new name, and the folder to move the bookmark to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PopoverEdits {
    pub(crate) rename: Option<String>,
    pub(crate) move_to: Option<BookmarkId>,
}

/// The edits made in the popover. A blank name keeps the old one.
pub(crate) fn popover_edits(
    title: &str,
    folder_index: Option<usize>,
    folders: &[FolderRow],
    name: &str,
    chosen_index: i32,
) -> PopoverEdits {
    let name = name.trim();
    let chosen = usize::try_from(chosen_index).ok();
    PopoverEdits {
        rename: (!name.is_empty() && name != title).then(|| name.to_owned()),
        move_to: chosen
            .filter(|chosen| Some(*chosen) != folder_index)
            .and_then(|chosen| folders.get(chosen))
            .map(|row| row.id),
    }
}

pub(crate) fn connect(window: &MainWindow) {
    window.on_bookmark_star_clicked(|| {
        with(star_clicked);
    });
    window.on_bookmark_popover_done(|| {
        with(finish);
    });
    // Closing it with a click outside or Escape keeps the edits, as Done does.
    window.on_bookmark_popover_dismissed(|| {
        with(finish);
    });
    window.on_bookmark_popover_remove(|| {
        with(remove);
    });
}

/// The star's click: bookmarks the page (into [`STAR_ROOT`]) if it is not yet, then opens
/// the popover for its bookmark.
pub(crate) fn star_clicked(bookmarks: &Rc<Bookmarks>) {
    if bookmarks.popover.borrow().is_some() {
        if let Some(window) = bookmarks.window.upgrade() {
            window.invoke_close_bookmark_popover();
        }
        finish(bookmarks);
        return;
    }
    let (state, url) = {
        let star = bookmarks.star.borrow();
        (star.state(), star.page().map(str::to_owned))
    };
    let (Some(url), Some(data)) = (url, profile_data::handle()) else {
        return;
    };
    match state {
        StarState::Disabled => {}
        StarState::Bookmarked(id) => open(id, url, false),
        StarState::Checking | StarState::Unbookmarked => {
            let title = {
                let controller = bookmarks.controller.borrow();
                let title = controller.active_state().page_title().trim().to_owned();
                if title.is_empty() { url.clone() } else { title }
            };
            data.find_or_add_bookmark(STAR_ROOT, title, url.clone(), move |result| {
                with(|bookmarks| match result {
                    Ok((id, added)) => {
                        bookmarks.star.borrow_mut().added(&url, id);
                        if let Some(window) = bookmarks.window.upgrade() {
                            bookmarks.show_star(&window);
                        }
                        open(id, url, added);
                    }
                    Err(error) => log::warn!("the page could not be bookmarked: {error}"),
                });
            });
        }
    }
}

/// Reads the bookmark and the folders, then shows the popover.
fn open(id: BookmarkId, url: String, is_new: bool) {
    let Some(data) = profile_data::handle() else {
        return;
    };
    data.bookmark(id, move |node| {
        let Ok(node) = node else {
            return;
        };
        let Some(data) = profile_data::handle() else {
            return;
        };
        data.bookmark_folders(move |folders| {
            with(|bookmarks| {
                let folders = folders.map(|walked| tree::folder_rows(&walked));
                show(bookmarks, node, url, folders.unwrap_or_default(), is_new);
            });
        });
    });
}

fn show(
    bookmarks: &Rc<Bookmarks>,
    node: BookmarkNode,
    url: String,
    folders: Vec<FolderRow>,
    is_new: bool,
) {
    let Some(window) = bookmarks.window.upgrade() else {
        return;
    };
    // The page may have changed while the bookmark was read.
    if bookmarks.star.borrow().page() != Some(url.as_str()) {
        return;
    }
    let folder_index = folders.iter().position(|row| Some(row.id) == node.parent);
    let labels = tree::chooser_labels(&folders)
        .into_iter()
        .map(SharedString::from)
        .collect::<Vec<_>>();
    window.set_bookmark_popover_is_new(is_new);
    window.set_bookmark_popover_name(node.title.as_str().into());
    window.set_bookmark_popover_folders(ModelRc::new(VecModel::from(labels)));
    window.set_bookmark_popover_folder_index(folder_index.map_or(-1, |i| i as i32));
    *bookmarks.popover.borrow_mut() = Some(Popover {
        id: node.id,
        url,
        title: node.title,
        folders,
        folder_index,
    });
    window.invoke_open_bookmark_popover();
    window.window().request_redraw();
}

/// Done, or the popover closed otherwise: applies the name and folder.
fn finish(bookmarks: &Rc<Bookmarks>) {
    let Some(popover) = bookmarks.popover.borrow_mut().take() else {
        return;
    };
    let (Some(window), Some(data)) = (bookmarks.window.upgrade(), profile_data::handle()) else {
        return;
    };
    let edits = popover_edits(
        &popover.title,
        popover.folder_index,
        &popover.folders,
        &window.get_bookmark_popover_name(),
        window.get_bookmark_popover_folder_index(),
    );
    if edits.rename.is_none() && edits.move_to.is_none() {
        return;
    }
    // One request: one notification, and the first failure is the one reported.
    let edit = BookmarkEdit {
        title: edits.rename,
        url: None,
        folder: edits.move_to,
    };
    data.edit_bookmark(popover.id, edit, |result| {
        if let Err(error) = result {
            log::warn!("the bookmark could not be changed: {error}");
        }
    });
}

fn remove(bookmarks: &Rc<Bookmarks>) {
    let Some(popover) = bookmarks.popover.borrow_mut().take() else {
        return;
    };
    let Some(data) = profile_data::handle() else {
        return;
    };
    let url = popover.url;
    data.remove_bookmark(popover.id, move |result| match result {
        Ok(()) => {
            with(|bookmarks| {
                if bookmarks.star.borrow_mut().removed(&url)
                    && let Some(window) = bookmarks.window.upgrade()
                {
                    bookmarks.show_star(&window);
                }
            });
        }
        Err(error) => log::warn!("the bookmark could not be removed: {error}"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<FolderRow> {
        [(1, "Bar"), (2, "Other"), (3, "News")]
            .into_iter()
            .map(|(id, title)| FolderRow {
                id: BookmarkId(id),
                title: title.to_owned(),
                depth: 0,
            })
            .collect()
    }

    #[test]
    fn unchanged_fields_change_nothing() {
        assert_eq!(
            popover_edits("Page", Some(1), &rows(), " Page ", 1),
            PopoverEdits {
                rename: None,
                move_to: None
            }
        );
    }

    #[test]
    fn a_new_name_and_folder_are_applied() {
        assert_eq!(
            popover_edits("Page", Some(1), &rows(), "Better", 2),
            PopoverEdits {
                rename: Some("Better".into()),
                move_to: Some(BookmarkId(3))
            }
        );
    }

    #[test]
    fn a_blank_name_or_no_choice_keeps_what_there_was() {
        assert_eq!(
            popover_edits("Page", Some(1), &rows(), "   ", -1),
            PopoverEdits {
                rename: None,
                move_to: None
            }
        );
        assert_eq!(
            popover_edits("Page", Some(1), &rows(), "x", 9).move_to,
            None
        );
    }
}
