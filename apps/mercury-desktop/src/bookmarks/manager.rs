// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The bookmarks manager: a separate window, like the settings window (a Slint window needs
//! no web view, uses the same widgets, and can be rendered headless). The folder tree on the
//! left, the selected folder's contents or the search results on the right, an editor under
//! them. Moves are by buttons and keys (Alt+Up/Down) and the "Move to" chooser; there is no
//! drag and drop (Slint 1.17 has no drag-and-drop between list rows to build it on cheaply).
//!
//! Large trees: the store has no paging, so a folder's contents are read whole, but only on
//! the service thread; the UI gets one folder's rows (or at most
//! [`MAX_SEARCH_RESULTS`](crate::profile_data::bookmarks::MAX_SEARCH_RESULTS) matches) in a
//! `ListView`, which creates rows only for what is on screen.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use mercury_store::{BookmarkId, BookmarkKind, BookmarkNode, BookmarkRoot, StoreError};
use mercury_ui::{BookmarkMenuRow, BookmarkRowKind, BookmarksManager};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use talaria_core::{NavigationTarget, classify_address};

use super::bar::MenuEntry;
use super::bar::menu_row;
use super::tree::{self, FolderRow};
use super::{Bookmarks, file, with};
use crate::profile_data::bookmarks::{BookmarkEdit, MAX_SEARCH_RESULTS, RemovedBookmark};
use crate::profile_data::{self, ProfileDataError};

/// What the manager shows and has selected.
#[derive(Default)]
struct View {
    folders: Vec<FolderRow>,
    folder: Option<BookmarkId>,
    rows: Vec<BookmarkNode>,
    selected: Option<BookmarkId>,
    /// Selects this entry when it appears (a new folder, a restored entry).
    select_next: Option<BookmarkId>,
    search: String,
    undo: Option<RemovedBookmark>,
}

pub(crate) struct Manager {
    window: BookmarksManager,
    bookmarks: Weak<Bookmarks>,
    view: RefCell<View>,
    /// Outdates replies to older reads.
    generation: Cell<u64>,
    /// Holds re-reads back while an import runs.
    gate: RefCell<RefreshGate>,
    /// Waits for typing to pause before searching.
    search_timer: slint::Timer,
}

/// How long typing in the search field must pause before the search runs.
const SEARCH_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

/// Opens the manager, making it the first time.
pub(crate) fn show(bookmarks: &Rc<Bookmarks>) {
    let existing = bookmarks.manager.borrow().clone();
    let manager = match existing {
        Some(manager) => manager,
        None => match Manager::new(bookmarks) {
            Ok(manager) => {
                *bookmarks.manager.borrow_mut() = Some(Rc::clone(&manager));
                manager
            }
            Err(error) => {
                log::error!("the bookmarks manager could not be opened: {error}");
                return;
            }
        },
    };
    if let Err(error) = manager.window.show() {
        log::error!("the bookmarks manager could not be shown: {error}");
        return;
    }
    manager.refresh();
    manager.window.invoke_focus_list();
}

/// Opens the manager at a folder (a folder chosen in the Bookmarks menu).
pub(crate) fn show_folder(bookmarks: &Rc<Bookmarks>, folder: BookmarkId) {
    show(bookmarks);
    if let Some(manager) = bookmarks.manager.borrow().clone() {
        {
            let mut view = manager.view.borrow_mut();
            view.folder = Some(folder);
            view.selected = None;
            view.search.clear();
        }
        manager.window.set_search_text(SharedString::default());
        manager.refresh();
    }
}

/// The address an edit asks for: a full URL as the address bar would read it, or why not.
pub(crate) fn edited_url(input: &str) -> Result<String, &'static str> {
    match classify_address(input) {
        Ok(NavigationTarget::Url(url)) => Ok(tree::without_credentials(url.as_str())),
        Ok(NavigationTarget::SearchQuery(_)) | Err(_) => {
            Err("Enter a full address, such as https://example.com/.")
        }
    }
}

/// A name as the manager stores it, trimmed; a blank name is refused, as the popover does.
pub(crate) fn validate_name(name: &str) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a name.");
    }
    Ok(name.to_owned())
}

/// Coalesces the manager's re-reads: none while an import runs (each of its chunks notifies),
/// one when it ends if anything changed meanwhile.
#[derive(Debug, Default)]
pub(crate) struct RefreshGate {
    busy: bool,
    pending: bool,
}

impl RefreshGate {
    /// A change arrived: whether to re-read now.
    pub(crate) fn changed(&mut self) -> bool {
        self.pending |= self.busy;
        !self.busy
    }

    /// An import started or ended: whether to re-read now (it ended and something changed).
    pub(crate) fn set_busy(&mut self, busy: bool) -> bool {
        self.busy = busy;
        !busy && std::mem::take(&mut self.pending)
    }

    pub(crate) fn busy(&self) -> bool {
        self.busy
    }
}

/// A plain sentence for a failed edit.
pub(crate) fn problem_text(error: &ProfileDataError) -> String {
    match error {
        ProfileDataError::Store(StoreError::InvalidMove) => {
            "A folder cannot be moved into itself or a folder inside it.".to_owned()
        }
        ProfileDataError::Store(StoreError::CannotModifyRoot) => {
            "The bookmarks bar and Other Bookmarks cannot be changed this way.".to_owned()
        }
        ProfileDataError::Store(StoreError::NotFound) => {
            "That entry, or the folder it was in, no longer exists.".to_owned()
        }
        ProfileDataError::Store(StoreError::InvalidArgument(reason)) => {
            format!("The bookmark was not changed: {reason}.")
        }
        other => format!("The bookmark was not changed: {other}."),
    }
}

/// The heading over the right-hand list.
pub(crate) fn heading(search: &str, results: usize, folder: Option<&str>) -> String {
    if search.trim().is_empty() {
        return folder.unwrap_or("Bookmarks").to_owned();
    }
    match results {
        0 => format!("No results for “{}”", search.trim()),
        n if n >= MAX_SEARCH_RESULTS => format!("The first {n} results for “{}”", search.trim()),
        1 => format!("1 result for “{}”", search.trim()),
        n => format!("{n} results for “{}”", search.trim()),
    }
}

impl Manager {
    fn new(bookmarks: &Rc<Bookmarks>) -> Result<Rc<Self>, slint::PlatformError> {
        let window = BookmarksManager::new()?;
        // Its own copy of `ChromePalette`, following the appearance setting.
        crate::settings::apply_chrome_theme(&window);
        // menus lane: reinstall Mercury's menu when this window becomes key
        #[cfg(target_os = "macos")]
        crate::menus::reinstall_when_key(window.window(), "the bookmarks manager became key");
        let manager = Rc::new(Self {
            window,
            bookmarks: Rc::downgrade(bookmarks),
            view: RefCell::new(View::default()),
            generation: Cell::new(0),
            gate: RefCell::new(RefreshGate::default()),
            search_timer: slint::Timer::default(),
        });
        manager.connect();
        Ok(manager)
    }

    pub(crate) fn window(&self) -> &BookmarksManager {
        &self.window
    }

    fn connect(self: &Rc<Self>) {
        let on = |manager: &Rc<Self>| Rc::downgrade(manager);
        let this = on(self);
        self.window.on_folder_selected(move |index| {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.select_folder(index);
            }
        });
        let this = on(self);
        self.window.on_row_selected(move |index| {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.select_row(index);
            }
        });
        let this = on(self);
        self.window.on_row_opened(move |index, new_tab| {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.open_row(index, new_tab);
            }
        });
        let this = on(self);
        self.window.on_search_edited(move |text| {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.view.borrow_mut().search = text.to_string();
                // Searched once typing pauses, not on every key: each search walks the tree.
                let later = Rc::downgrade(&manager);
                manager
                    .search_timer
                    .start(slint::TimerMode::SingleShot, SEARCH_DELAY, move || {
                        let _slint = crate::callback_scope::InSlintCallback::enter();
                        if let Some(manager) = later.upgrade() {
                            manager.load_rows();
                        }
                    });
            }
        });
        let this = on(self);
        self.window.on_save_edits(move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.save_edits();
            }
        });
        let this = on(self);
        self.window.on_new_folder(move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.add(true);
            }
        });
        let this = on(self);
        self.window.on_new_separator(move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.add(false);
            }
        });
        let this = on(self);
        self.window.on_delete_selected(move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.delete_selected();
            }
        });
        let this = on(self);
        self.window.on_undo_delete(move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.undo_delete();
            }
        });
        let this = on(self);
        self.window.on_move_up(move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.move_by(-1);
            }
        });
        let this = on(self);
        self.window.on_move_down(move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.move_by(1);
            }
        });
        let this = on(self);
        self.window.on_move_to(move |index| {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                manager.move_to(index);
            }
        });
        let this = on(self);
        self.window.on_import_file(move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                file::import_from_file(&manager);
            }
        });
        let this = on(self);
        self.window.on_export_file(move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(manager) = this.upgrade() {
                file::export_to_file(&manager);
            }
        });
    }

    /// Whether an import from a file is running.
    pub(crate) fn busy(&self) -> bool {
        self.gate.borrow().busy()
    }

    /// An import started or ended; re-reads once at its end if anything changed meanwhile.
    pub(crate) fn set_busy(self: &Rc<Self>, busy: bool) {
        self.window.set_busy(busy);
        let refresh = self.gate.borrow_mut().set_busy(busy);
        if refresh {
            self.refresh();
        }
    }

    /// The bookmarks changed: re-reads now, or after the import that is running.
    pub(crate) fn changed(self: &Rc<Self>) {
        let now = self.gate.borrow_mut().changed();
        if now {
            self.refresh();
        }
    }

    pub(crate) fn set_status(&self, status: &str) {
        self.window.set_status(status.into());
    }

    fn set_problem(&self, problem: &str) {
        self.window.set_problem(problem.into());
    }

    /// Reads the folders and the list again (after a change, or when shown).
    pub(crate) fn refresh(self: &Rc<Self>) {
        if !self.window.window().is_visible() {
            return;
        }
        let Some(data) = profile_data::handle() else {
            self.set_status("Bookmarks are not available in this run.");
            return;
        };
        if let Some(reason) = data.unavailable_reason() {
            self.set_status(&format!(
                "Bookmarks are not available in this run: {reason}"
            ));
            self.window.set_can_add(false);
        }
        let this = Rc::downgrade(self);
        data.bookmark_folders(move |folders| {
            let Some(manager) = this.upgrade() else {
                return;
            };
            match folders {
                Ok(walked) => {
                    let rows = tree::folder_rows(&walked);
                    {
                        let mut view = manager.view.borrow_mut();
                        if view
                            .folder
                            .is_none_or(|id| !rows.iter().any(|row| row.id == id))
                        {
                            view.folder = rows.iter().find(|row| row.depth == 0).map(|row| row.id);
                        }
                        view.folders = rows;
                    }
                    manager.show_folders();
                    manager.load_rows();
                }
                Err(error) => manager.set_status(&error.to_string()),
            }
        });
    }

    fn show_folders(&self) {
        let view = self.view.borrow();
        let rows = view
            .folders
            .iter()
            .map(|row| BookmarkMenuRow {
                title: row.title.as_str().into(),
                url: SharedString::default(),
                kind: BookmarkRowKind::Folder,
                depth: i32::try_from(row.depth).unwrap_or(i32::MAX),
            })
            .collect::<Vec<_>>();
        self.window.set_folders(ModelRc::new(VecModel::from(rows)));
        let index = view
            .folders
            .iter()
            .position(|row| Some(row.id) == view.folder);
        self.window
            .set_folder_index(index.map_or(-1, |index| index as i32));
        let labels = tree::chooser_labels(&view.folders)
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>();
        self.window
            .set_move_targets(ModelRc::new(VecModel::from(labels)));
    }

    /// Reads the selected folder's contents, or the search results.
    fn load_rows(self: &Rc<Self>) {
        let Some(data) = profile_data::handle() else {
            return;
        };
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let (search, folder) = {
            let view = self.view.borrow();
            (view.search.trim().to_owned(), view.folder)
        };
        let this = Rc::downgrade(self);
        let deliver = move |rows: Result<Vec<BookmarkNode>, ProfileDataError>| {
            let Some(manager) = this.upgrade() else {
                return;
            };
            if manager.generation.get() != generation {
                return;
            }
            match rows {
                Ok(rows) => manager.show_rows(rows),
                Err(error) => manager.set_status(&error.to_string()),
            }
        };
        if !search.is_empty() {
            data.search_bookmarks(search, deliver);
        } else if let Some(folder) = folder {
            data.bookmark_children(folder, deliver);
        }
    }

    fn show_rows(&self, rows: Vec<BookmarkNode>) {
        let (model, index, title) = {
            let mut view = self.view.borrow_mut();
            if let Some(next) = view.select_next.take() {
                view.selected = Some(next);
            }
            if view
                .selected
                .is_some_and(|id| !rows.iter().any(|row| row.id == id))
            {
                view.selected = None;
            }
            let model = rows
                .iter()
                .map(|node| menu_row(&MenuEntry::Node(node.clone())))
                .collect::<Vec<_>>();
            let index = rows.iter().position(|row| Some(row.id) == view.selected);
            let folder_title = view
                .folders
                .iter()
                .find(|row| Some(row.id) == view.folder)
                .map(|row| row.title.clone());
            let title = heading(&view.search, rows.len(), folder_title.as_deref());
            view.rows = rows;
            (model, index, title)
        };
        self.window.set_rows(ModelRc::new(VecModel::from(model)));
        self.window.set_heading(title.into());
        self.window
            .set_searching(!self.view.borrow().search.trim().is_empty());
        self.window
            .set_row_index(index.map_or(-1, |index| index as i32));
        self.show_editor();
    }

    /// Fills the editor and the move buttons for the selection.
    fn show_editor(&self) {
        let view = self.view.borrow();
        let selected = view
            .selected
            .and_then(|id| view.rows.iter().find(|row| row.id == id));
        let searching = !view.search.trim().is_empty();
        match selected {
            Some(node) if node.kind != BookmarkKind::Separator => {
                self.window.set_editing(true);
                self.window
                    .set_editing_bookmark(matches!(node.kind, BookmarkKind::Bookmark { .. }));
                self.window.set_edit_name(node.title.as_str().into());
                let url = match &node.kind {
                    BookmarkKind::Bookmark { url } => url.as_str(),
                    _ => "",
                };
                self.window.set_edit_url(url.into());
                let parent = view
                    .folders
                    .iter()
                    .position(|row| Some(row.id) == node.parent);
                self.window
                    .set_move_index(parent.map_or(0, |index| index as i32));
            }
            _ => self.window.set_editing(false),
        }
        let position = selected.map(|node| node.position);
        let count = view.rows.len() as u32;
        self.window
            .set_can_move_up(!searching && position.is_some_and(|position| position > 0));
        self.window
            .set_can_move_down(!searching && position.is_some_and(|position| position + 1 < count));
    }

    fn select_folder(self: &Rc<Self>, index: i32) {
        let id = usize::try_from(index)
            .ok()
            .and_then(|index| self.view.borrow().folders.get(index).map(|row| row.id));
        let Some(id) = id else {
            return;
        };
        {
            let mut view = self.view.borrow_mut();
            view.folder = Some(id);
            view.selected = None;
            view.search.clear();
        }
        self.window.set_search_text(SharedString::default());
        self.window.set_folder_index(index);
        self.set_problem("");
        self.load_rows();
    }

    fn select_row(&self, index: i32) {
        {
            let mut view = self.view.borrow_mut();
            view.selected = usize::try_from(index)
                .ok()
                .and_then(|index| view.rows.get(index).map(|row| row.id));
        }
        self.set_problem("");
        self.show_editor();
    }

    fn selected(&self) -> Option<BookmarkNode> {
        let view = self.view.borrow();
        let id = view.selected?;
        view.rows.iter().find(|row| row.id == id).cloned()
    }

    fn open_row(self: &Rc<Self>, index: i32, new_tab: bool) {
        let node = usize::try_from(index)
            .ok()
            .and_then(|index| self.view.borrow().rows.get(index).cloned());
        let Some(node) = node else {
            return;
        };
        match node.kind {
            BookmarkKind::Bookmark { url } => {
                if let Some(bookmarks) = self.bookmarks.upgrade() {
                    bookmarks.open_url(&url, new_tab);
                }
            }
            BookmarkKind::Folder => {
                let index = self
                    .view
                    .borrow()
                    .folders
                    .iter()
                    .position(|row| row.id == node.id);
                if let Some(index) = index {
                    self.select_folder(index as i32);
                }
            }
            BookmarkKind::Separator => {}
        }
    }

    /// Replies to an edit: shows a problem, or clears it. The change notification that
    /// follows a successful edit re-reads the lists.
    fn report(self: &Rc<Self>) -> impl FnOnce(Result<(), ProfileDataError>) + 'static {
        let this = Rc::downgrade(self);
        move |result| {
            if let Some(manager) = this.upgrade() {
                match result {
                    Ok(()) => manager.set_problem(""),
                    Err(error) => manager.set_problem(&problem_text(&error)),
                }
            }
        }
    }

    fn save_edits(self: &Rc<Self>) {
        let (Some(node), Some(data)) = (self.selected(), profile_data::handle()) else {
            return;
        };
        let name = match validate_name(&self.window.get_edit_name()) {
            Ok(name) => name,
            Err(problem) => {
                self.set_problem(problem);
                return;
            }
        };
        let url = match &node.kind {
            BookmarkKind::Bookmark { url } => {
                let typed = self.window.get_edit_url();
                if typed.trim() == url {
                    None
                } else {
                    match edited_url(&typed) {
                        Ok(edited) => Some(edited),
                        Err(problem) => {
                            self.set_problem(problem);
                            return;
                        }
                    }
                }
            }
            _ => None,
        };
        let edit = BookmarkEdit {
            title: (name != node.title).then_some(name),
            url,
            folder: None,
        };
        // One request: one reply (the first failure is shown) and one notification.
        if edit != BookmarkEdit::default() {
            data.edit_bookmark(node.id, edit, self.report());
        }
    }

    /// A new folder (named "New folder", selected for renaming) or separator, after the
    /// selection or at the end of the folder shown (Other Bookmarks while searching).
    fn add(self: &Rc<Self>, folder: bool) {
        let Some(data) = profile_data::handle() else {
            return;
        };
        let (parent, position) = {
            let view = self.view.borrow();
            let selected = view
                .selected
                .and_then(|id| view.rows.iter().find(|row| row.id == id));
            match (view.search.trim().is_empty(), view.folder, selected) {
                (true, Some(folder), Some(node)) => (Some(folder), Some(node.position + 1)),
                (true, Some(folder), None) => (Some(folder), None),
                _ => (None, None),
            }
        };
        let this = Rc::downgrade(self);
        let added = move |result: Result<BookmarkId, ProfileDataError>| {
            if let Some(manager) = this.upgrade() {
                match result {
                    Ok(id) => {
                        manager.view.borrow_mut().select_next = Some(id);
                        manager.set_problem("");
                    }
                    Err(error) => manager.set_problem(&problem_text(&error)),
                }
            }
        };
        let add = move |data: &profile_data::ProfileData, parent| {
            if folder {
                data.add_bookmark_folder(parent, position, "New folder".to_owned(), added);
            } else {
                data.add_bookmark_separator(parent, position, added);
            }
        };
        match parent {
            Some(parent) => add(&data, parent),
            None => data.bookmark_root(BookmarkRoot::Other, move |root| {
                if let (Ok(root), Some(data)) = (root, profile_data::handle()) {
                    add(&data, root);
                }
            }),
        }
    }

    fn delete_selected(self: &Rc<Self>) {
        let (Some(node), Some(data)) = (self.selected(), profile_data::handle()) else {
            return;
        };
        // The entry after it is selected next, or the one before it at the end.
        let next = {
            let view = self.view.borrow();
            let index = view.rows.iter().position(|row| row.id == node.id);
            index.and_then(|index| {
                view.rows
                    .get(index + 1)
                    .or_else(|| {
                        index
                            .checked_sub(1)
                            .and_then(|before| view.rows.get(before))
                    })
                    .map(|row| row.id)
            })
        };
        let this = Rc::downgrade(self);
        data.remove_bookmark_keeping_undo(node.id, move |result| {
            let Some(manager) = this.upgrade() else {
                return;
            };
            match result {
                Ok(removed) => {
                    let label = undo_label(&removed);
                    {
                        let mut view = manager.view.borrow_mut();
                        view.undo = Some(removed);
                        view.select_next = next;
                    }
                    manager.window.set_undo_label(label.into());
                    manager.window.set_can_undo(true);
                    manager.set_problem("");
                }
                Err(error) => manager.set_problem(&problem_text(&error)),
            }
        });
    }

    fn undo_delete(self: &Rc<Self>) {
        let Some(data) = profile_data::handle() else {
            return;
        };
        let Some(removed) = self.view.borrow_mut().undo.take() else {
            return;
        };
        self.window.set_can_undo(false);
        // Kept until the restore succeeds: a failed undo can be tried again.
        let kept = removed.clone();
        let this = Rc::downgrade(self);
        data.restore_bookmark(removed, move |result| {
            let Some(manager) = this.upgrade() else {
                return;
            };
            match result {
                Ok(restored) => {
                    manager.view.borrow_mut().select_next = Some(restored.id);
                    manager.set_problem("");
                    if !restored.in_place {
                        manager.set_status("Restored at the end of its folder.");
                    }
                }
                Err(error) => {
                    manager.view.borrow_mut().undo = Some(kept);
                    manager.window.set_can_undo(true);
                    manager.set_problem(&problem_text(&error));
                }
            }
        });
    }

    fn move_by(self: &Rc<Self>, step: i32) {
        let (Some(node), Some(data)) = (self.selected(), profile_data::handle()) else {
            return;
        };
        if !self.view.borrow().search.trim().is_empty() {
            return;
        }
        let Some(parent) = node.parent else {
            return;
        };
        let Some(position) = node.position.checked_add_signed(step) else {
            return;
        };
        data.move_bookmark(node.id, parent, Some(position), self.report());
    }

    fn move_to(self: &Rc<Self>, index: i32) {
        let (Some(node), Some(data)) = (self.selected(), profile_data::handle()) else {
            return;
        };
        let target = usize::try_from(index)
            .ok()
            .and_then(|index| self.view.borrow().folders.get(index).map(|row| row.id));
        let Some(target) = target else {
            return;
        };
        if Some(target) == node.parent {
            return;
        }
        data.move_bookmark(node.id, target, None, self.report());
    }
}

fn undo_label(removed: &RemovedBookmark) -> String {
    let what = match &removed.tree {
        mercury_store::BookmarkImport::Separator { .. } => {
            return "Undo delete of separator".into();
        }
        _ if removed.title.is_empty() => "(untitled)".to_owned(),
        _ => removed.title.chars().take(40).collect(),
    };
    format!("Undo delete of “{what}”")
}

/// The window, for the file panels and the import's progress.
pub(crate) fn current() -> Option<Rc<Manager>> {
    with(|bookmarks| bookmarks.manager.borrow().clone()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_name_is_refused() {
        assert_eq!(validate_name("  News  "), Ok("News".to_owned()));
        assert!(validate_name("   ").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn refreshes_wait_for_an_import_and_then_happen_once() {
        let mut gate = RefreshGate::default();
        assert!(gate.changed(), "idle: at once");
        assert!(!gate.set_busy(true), "nothing pending yet");
        assert!(!gate.changed());
        assert!(!gate.changed());
        assert!(
            gate.set_busy(false),
            "one refresh for what changed meanwhile"
        );
        assert!(!gate.set_busy(true));
        assert!(!gate.set_busy(false), "nothing changed during this import");
    }

    #[test]
    fn edited_addresses_must_be_full_urls() {
        assert_eq!(
            edited_url(" https://example.com/a "),
            Ok("https://example.com/a".to_owned())
        );
        assert_eq!(
            edited_url("example.com"),
            Ok("https://example.com/".to_owned())
        );
        assert!(edited_url("two words").is_err());
        assert!(edited_url("").is_err());
        assert!(edited_url("ftp://example.com/").is_err());
        assert_eq!(
            edited_url("https://me:pw@example.com/"),
            Ok("https://example.com/".to_owned())
        );
    }

    #[test]
    fn headings_name_the_folder_or_count_the_results() {
        assert_eq!(heading("", 3, Some("News")), "News");
        assert_eq!(heading("  ", 3, None), "Bookmarks");
        assert_eq!(heading("rust", 0, None), "No results for “rust”");
        assert_eq!(heading("rust", 1, None), "1 result for “rust”");
        assert_eq!(heading(" rust ", 7, None), "7 results for “rust”");
        assert_eq!(
            heading("a", MAX_SEARCH_RESULTS, None),
            format!("The first {MAX_SEARCH_RESULTS} results for “a”")
        );
    }

    #[test]
    fn problems_are_plain_sentences() {
        assert_eq!(
            problem_text(&ProfileDataError::Store(StoreError::InvalidMove)),
            "A folder cannot be moved into itself or a folder inside it."
        );
        assert!(
            problem_text(&ProfileDataError::Store(StoreError::InvalidArgument(
                "URL too long"
            )))
            .contains("URL too long")
        );
    }

    #[test]
    fn the_undo_button_names_what_was_deleted() {
        let removed = |title: &str, tree| RemovedBookmark {
            parent: BookmarkId(1),
            position: 0,
            title: title.to_owned(),
            tree,
        };
        let folder = mercury_store::BookmarkImport::Folder {
            title: "News".into(),
            guid: "g".into(),
            added_at_ms: None,
            children: Vec::new(),
        };
        assert_eq!(
            undo_label(&removed("News", folder.clone())),
            "Undo delete of “News”"
        );
        assert_eq!(
            undo_label(&removed("", folder)),
            "Undo delete of “(untitled)”"
        );
        assert_eq!(
            undo_label(&removed(
                "",
                mercury_store::BookmarkImport::Separator { guid: "s".into() }
            )),
            "Undo delete of separator"
        );
    }
}
