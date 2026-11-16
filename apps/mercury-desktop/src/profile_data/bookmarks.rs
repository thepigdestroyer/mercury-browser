//! Bookmark requests: reading and editing the tree. New nodes get a fresh GUID
//! ([`new_guid`](super::new_guid)). Writes notify [`Area::Bookmarks`] with the folders whose
//! listing changed (both folders for a move).
//! Each reply runs on the UI thread with `Result<T, ProfileDataError>`.
//!
//! The requests after the store's own (the folder list, search, deletion with undo, the tree
//! for export) walk the tree on the service thread with `bookmark_children`, one folder at a
//! time: the store offers no paging and no bookmark search, so only what the caller needs (the
//! folders, the matches) crosses to the UI thread.

use mercury_store::{BookmarkId, BookmarkImport, BookmarkKind, BookmarkNode, BookmarkRoot, Store};

use super::{Area, Change, Database, ProfileData, ProfileDataError, guid_or_error};

/// The change hint for editing node `id`: the folder that lists it (the node itself for a
/// root).
fn listing_of(db: &mut Database, id: BookmarkId) -> mercury_store::Result<Change> {
    let node = db.store().bookmark(id)?;
    Ok(Change::folders([node.parent.unwrap_or(id)]))
}

/// A node of a walk, with its depth below the node the walk started at (which is at 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Walked {
    pub(crate) depth: usize,
    pub(crate) node: BookmarkNode,
}

/// A deleted entry and everything that was in it, as it was: enough to put it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemovedBookmark {
    pub(crate) parent: BookmarkId,
    pub(crate) position: u32,
    /// The entry's title, for "Undo delete of …".
    pub(crate) title: String,
    /// The entry with its original GUIDs and dates.
    pub(crate) tree: BookmarkImport,
}

/// The most results a search returns.
pub(crate) const MAX_SEARCH_RESULTS: usize = 1_000;

/// `start` and everything below it, in pre-order, without recursion.
pub(crate) fn walk(store: &Store, start: BookmarkId) -> mercury_store::Result<Vec<Walked>> {
    let mut walked = Vec::new();
    let mut stack = vec![Walked {
        depth: 0,
        node: store.bookmark(start)?,
    }];
    while let Some(entry) = stack.pop() {
        if entry.node.kind == BookmarkKind::Folder {
            let children = store.bookmark_children(entry.node.id)?;
            stack.extend(children.into_iter().rev().map(|node| Walked {
                depth: entry.depth + 1,
                node,
            }));
        }
        walked.push(entry);
    }
    Ok(walked)
}

/// Both roots, the bookmarks bar first, walked.
fn walk_roots(store: &Store) -> mercury_store::Result<Vec<Walked>> {
    let mut walked = walk(store, store.bookmark_root(BookmarkRoot::Toolbar)?)?;
    walked.extend(walk(store, store.bookmark_root(BookmarkRoot::Other)?)?);
    Ok(walked)
}

impl ProfileData {
    /// The star: the first bookmark of `url`, or a new one with `title` at the end of
    /// `root`, in one job so that a click on a page whose lookup is still pending cannot add a
    /// second bookmark. Replies with the bookmark and whether it was added.
    pub(crate) fn find_or_add_bookmark(
        &self,
        root: BookmarkRoot,
        title: String,
        url: String,
        reply: impl FnOnce(Result<(BookmarkId, bool), ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let store = db.store();
                if let Some(id) = store.bookmark_for_url(&url)? {
                    // Nothing was written: no folder's listing changed.
                    db.changed(Change::folders([]));
                    return Ok((id, false));
                }
                let parent = store.bookmark_root(root)?;
                let guid = guid_or_error()?;
                let id = store.add_bookmark(parent, None, &title, &url, &guid, now)?;
                db.changed(Change::folders([parent]));
                Ok((id, true))
            },
            reply,
        );
    }

    /// Every folder of both roots (the bookmarks bar's first), in pre-order with their depth.
    pub(crate) fn bookmark_folders(
        &self,
        reply: impl FnOnce(Result<Vec<Walked>, ProfileDataError>) + 'static,
    ) {
        self.request(
            None,
            |db, _| {
                Ok(walk_roots(db.store())?
                    .into_iter()
                    .filter(|entry| entry.node.kind == BookmarkKind::Folder)
                    .collect())
            },
            reply,
        );
    }

    /// The bookmarks and folders whose title or URL contains every word of `query` (ignoring
    /// case), in tree order, at most [`MAX_SEARCH_RESULTS`].
    pub(crate) fn search_bookmarks(
        &self,
        query: String,
        reply: impl FnOnce(Result<Vec<BookmarkNode>, ProfileDataError>) + 'static,
    ) {
        self.request(
            None,
            move |db, _| {
                let words = crate::bookmarks::tree::search_words(&query);
                Ok(walk_roots(db.store())?
                    .into_iter()
                    .filter(|entry| entry.depth > 0)
                    .map(|entry| entry.node)
                    .filter(|node| crate::bookmarks::tree::matches(node, &words))
                    .take(MAX_SEARCH_RESULTS)
                    .collect())
            },
            reply,
        );
    }

    /// Both roots walked, for writing a bookmarks file.
    pub(crate) fn bookmark_trees(
        &self,
        reply: impl FnOnce(Result<(Vec<Walked>, Vec<Walked>), ProfileDataError>) + 'static,
    ) {
        self.request(
            None,
            |db, _| {
                let store = db.store();
                let toolbar = walk(store, store.bookmark_root(BookmarkRoot::Toolbar)?)?;
                let other = walk(store, store.bookmark_root(BookmarkRoot::Other)?)?;
                Ok((toolbar, other))
            },
            reply,
        );
    }

    /// Removes a node and everything in it, replying with what is needed to put it back.
    pub(crate) fn remove_bookmark_keeping_undo(
        &self,
        id: BookmarkId,
        reply: impl FnOnce(Result<RemovedBookmark, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let store = db.store();
                let walked = walk(store, id)?;
                let Some(first) = walked.first() else {
                    return Err(mercury_store::StoreError::NotFound);
                };
                let Some(parent) = first.node.parent else {
                    return Err(mercury_store::StoreError::CannotModifyRoot);
                };
                let removed = RemovedBookmark {
                    parent,
                    position: first.node.position,
                    title: first.node.title.clone(),
                    tree: crate::bookmarks::tree::undo_tree(&walked).ok_or(
                        mercury_store::StoreError::InvalidState("the deleted entry was empty"),
                    )?,
                };
                store.remove_bookmark(id, now)?;
                db.changed(Change::folders([parent]));
                Ok(removed)
            },
            reply,
        );
    }

    /// Puts a removed entry back where it was (its old position, or the end if the folder
    /// has fewer entries now), with its GUIDs and dates. Fails with `NotFound` if its folder
    /// is gone.
    pub(crate) fn restore_bookmark(
        &self,
        removed: RemovedBookmark,
        reply: impl FnOnce(Result<BookmarkId, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let store = db.store();
                store.import_bookmarks(removed.parent, std::slice::from_ref(&removed.tree), now)?;
                let restored = store
                    .bookmark_children(removed.parent)?
                    .pop()
                    .ok_or(mercury_store::StoreError::NotFound)?;
                store.move_bookmark(restored.id, removed.parent, Some(removed.position), now)?;
                db.changed(Change::folders([removed.parent]));
                Ok(restored.id)
            },
            reply,
        );
    }
}

impl ProfileData {
    /// The id of a root folder.
    pub(crate) fn bookmark_root(
        &self,
        root: BookmarkRoot,
        reply: impl FnOnce(Result<BookmarkId, ProfileDataError>) + 'static,
    ) {
        self.request(None, move |db, _| db.store().bookmark_root(root), reply);
    }

    /// The children of a folder, by position.
    pub(crate) fn bookmark_children(
        &self,
        folder: BookmarkId,
        reply: impl FnOnce(Result<Vec<BookmarkNode>, ProfileDataError>) + 'static,
    ) {
        self.request(
            None,
            move |db, _| db.store().bookmark_children(folder),
            reply,
        );
    }

    /// One node.
    pub(crate) fn bookmark(
        &self,
        id: BookmarkId,
        reply: impl FnOnce(Result<BookmarkNode, ProfileDataError>) + 'static,
    ) {
        self.request(None, move |db, _| db.store().bookmark(id), reply);
    }

    /// The first bookmark of exactly this URL, if any (the star button).
    pub(crate) fn bookmark_for_url(
        &self,
        url: String,
        reply: impl FnOnce(Result<Option<BookmarkId>, ProfileDataError>) + 'static,
    ) {
        self.request(None, move |db, _| db.store().bookmark_for_url(&url), reply);
    }

    /// Adds a bookmark in `parent` at `position` (`None` appends).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the star uses find_or_add_bookmark; kept for the import work"
        )
    )]
    pub(crate) fn add_bookmark(
        &self,
        parent: BookmarkId,
        position: Option<u32>,
        title: String,
        url: String,
        reply: impl FnOnce(Result<BookmarkId, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let guid = guid_or_error()?;
                db.changed(Change::folders([parent]));
                db.store()
                    .add_bookmark(parent, position, &title, &url, &guid, now)
            },
            reply,
        );
    }

    /// Adds a folder in `parent` at `position` (`None` appends).
    pub(crate) fn add_bookmark_folder(
        &self,
        parent: BookmarkId,
        position: Option<u32>,
        title: String,
        reply: impl FnOnce(Result<BookmarkId, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let guid = guid_or_error()?;
                db.changed(Change::folders([parent]));
                db.store()
                    .add_bookmark_folder(parent, position, &title, &guid, now)
            },
            reply,
        );
    }

    /// Adds a separator in `parent` at `position` (`None` appends).
    pub(crate) fn add_bookmark_separator(
        &self,
        parent: BookmarkId,
        position: Option<u32>,
        reply: impl FnOnce(Result<BookmarkId, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let guid = guid_or_error()?;
                db.changed(Change::folders([parent]));
                db.store()
                    .add_bookmark_separator(parent, position, &guid, now)
            },
            reply,
        );
    }

    /// Renames a node.
    pub(crate) fn rename_bookmark(
        &self,
        id: BookmarkId,
        title: String,
        reply: impl FnOnce(Result<(), ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let change = listing_of(db, id)?;
                db.changed(change);
                db.store().rename_bookmark(id, &title, now)
            },
            reply,
        );
    }

    /// Points a bookmark at another URL.
    pub(crate) fn set_bookmark_url(
        &self,
        id: BookmarkId,
        url: String,
        reply: impl FnOnce(Result<(), ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let change = listing_of(db, id)?;
                db.changed(change);
                db.store().set_bookmark_url(id, &url, now)
            },
            reply,
        );
    }

    /// Moves a node into `parent` at `position` (`None` appends).
    pub(crate) fn move_bookmark(
        &self,
        id: BookmarkId,
        parent: BookmarkId,
        position: Option<u32>,
        reply: impl FnOnce(Result<(), ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let mut change = listing_of(db, id)?;
                change.merge(Change::folders([parent]));
                db.changed(change);
                db.store().move_bookmark(id, parent, position, now)
            },
            reply,
        );
    }

    /// Removes a node and everything in it.
    pub(crate) fn remove_bookmark(
        &self,
        id: BookmarkId,
        reply: impl FnOnce(Result<(), ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let change = listing_of(db, id)?;
                db.changed(change);
                db.store().remove_bookmark(id, now)
            },
            reply,
        );
    }
}
