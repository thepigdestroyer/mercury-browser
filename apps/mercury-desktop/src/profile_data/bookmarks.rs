// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

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

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::import::{IMPORT_CHUNK, ImportOutcome, ImportProgress, ImportSummary};
use super::jobs::{JobHandle, Step, Steps};
use super::{Area, Change, Database, Payload, ProfileData, ProfileDataError, guid_or_error};

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

/// What an edit of one entry changes; `None` leaves that part as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BookmarkEdit {
    pub(crate) title: Option<String>,
    /// Bookmarks only.
    pub(crate) url: Option<String>,
    /// Moves the entry to the end of this folder.
    pub(crate) folder: Option<BookmarkId>,
}

/// A deleted entry put back: its new id, and whether it went back to its old place (`false`:
/// it is at the end of its folder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Restored {
    pub(crate) id: BookmarkId,
    pub(crate) in_place: bool,
}

/// The folder a created folder goes into: one that exists, or one created earlier in the same
/// request (by its index there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderRef {
    Existing(BookmarkId),
    Created(usize),
}

/// A folder to create, empty, at the end of its parent, with the GUID and date it is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewFolder {
    pub(crate) parent: FolderRef,
    pub(crate) title: String,
    pub(crate) guid: String,
    pub(crate) added_at_ms: Option<i64>,
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

// Pure helpers of the walks, shared with the bookmarks UI (which builds export trees with
// `nest`).

/// Builds nested trees from a pre-order walk: entries at `base` depth become the top level.
/// `make` turns a node and its finished children into a tree node.
pub(crate) fn nest<T>(
    walked: &[Walked],
    base: usize,
    mut make: impl FnMut(&BookmarkNode, Vec<T>) -> Option<T>,
) -> Vec<T> {
    let mut top: Vec<T> = Vec::new();
    let mut open: Vec<(&BookmarkNode, Vec<T>)> = Vec::new();
    let mut close = |open: &mut Vec<(&BookmarkNode, Vec<T>)>, top: &mut Vec<T>| {
        if let Some((node, children)) = open.pop()
            && let Some(made) = make(node, children)
        {
            match open.last_mut() {
                Some((_, siblings)) => siblings.push(made),
                None => top.push(made),
            }
        }
    };
    for entry in walked.iter().filter(|entry| entry.depth >= base) {
        let level = entry.depth - base;
        while open.len() > level {
            close(&mut open, &mut top);
        }
        open.push((&entry.node, Vec::new()));
    }
    while !open.is_empty() {
        close(&mut open, &mut top);
    }
    top
}

/// The walked entry (at depth 0) and everything in it, with their own GUIDs and dates, to put
/// back after a deletion.
pub(crate) fn undo_tree(walked: &[Walked]) -> Option<BookmarkImport> {
    let base = walked.first()?.depth;
    nest(walked, base, |node, children| {
        Some(match &node.kind {
            BookmarkKind::Bookmark { url } => BookmarkImport::Bookmark {
                title: node.title.clone(),
                url: url.clone(),
                guid: node.guid.clone(),
                added_at_ms: Some(node.added_at_ms),
            },
            BookmarkKind::Folder => BookmarkImport::Folder {
                title: node.title.clone(),
                guid: node.guid.clone(),
                added_at_ms: Some(node.added_at_ms),
                children,
            },
            BookmarkKind::Separator => BookmarkImport::Separator {
                guid: node.guid.clone(),
            },
        })
    })
    .into_iter()
    .next()
}

/// A search's words, lower-cased.
pub(crate) fn search_words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// Whether a bookmark or folder matches every word in its title or URL. Separators and an
/// empty search match nothing.
pub(crate) fn matches(node: &BookmarkNode, words: &[String]) -> bool {
    if words.is_empty() {
        return false;
    }
    let title = node.title.to_lowercase();
    let url = match &node.kind {
        BookmarkKind::Bookmark { url } => url.to_lowercase(),
        BookmarkKind::Folder => String::new(),
        BookmarkKind::Separator => return false,
    };
    words
        .iter()
        .all(|word| title.contains(word.as_str()) || url.contains(word.as_str()))
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
                let words = search_words(&query);
                Ok(walk_roots(db.store())?
                    .into_iter()
                    .filter(|entry| entry.depth > 0)
                    .map(|entry| entry.node)
                    .filter(|node| matches(node, &words))
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
                    tree: undo_tree(&walked).ok_or(mercury_store::StoreError::InvalidState(
                        "the deleted entry was empty",
                    ))?,
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
        reply: impl FnOnce(Result<Restored, ProfileDataError>) + 'static,
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
                // The import committed: a failed move leaves the entry at the end, and says so.
                let in_place = match store.move_bookmark(
                    restored.id,
                    removed.parent,
                    Some(removed.position),
                    now,
                ) {
                    Ok(()) => true,
                    Err(error) => {
                        log::warn!("a restored bookmark stays at the end of its folder: {error}");
                        false
                    }
                };
                db.changed(Change::folders([removed.parent]));
                Ok(Restored {
                    id: restored.id,
                    in_place,
                })
            },
            reply,
        );
    }

    /// Edits an entry in one request: its URL, then its name, then its folder; the first
    /// failure is the reply (what came before it stays).
    pub(crate) fn edit_bookmark(
        &self,
        id: BookmarkId,
        edit: BookmarkEdit,
        reply: impl FnOnce(Result<(), ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let mut change = listing_of(db, id)?;
                if let Some(folder) = edit.folder {
                    change.merge(Change::folders([folder]));
                }
                // Noted first: a failure after a part succeeded still tells the listeners.
                db.changed(change);
                let store = db.store();
                if let Some(url) = &edit.url {
                    store.set_bookmark_url(id, url, now)?;
                }
                if let Some(title) = &edit.title {
                    store.rename_bookmark(id, title, now)?;
                }
                if let Some(folder) = edit.folder {
                    store.move_bookmark(id, folder, None, now)?;
                }
                Ok(())
            },
            reply,
        );
    }

    /// Creates empty folders in order; replies with their ids, in the same order.
    pub(crate) fn create_folders(
        &self,
        folders: Vec<NewFolder>,
        reply: impl FnOnce(Result<Vec<BookmarkId>, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let mut ids: Vec<BookmarkId> = Vec::with_capacity(folders.len());
                let mut parents = Vec::new();
                for folder in folders {
                    let parent = match folder.parent {
                        FolderRef::Existing(id) => id,
                        FolderRef::Created(index) => {
                            *ids.get(index)
                                .ok_or(mercury_store::StoreError::InvalidArgument(
                                    "a folder's parent must be created before it",
                                ))?
                        }
                    };
                    // An import of one empty folder keeps the GUID and date it is given.
                    let empty = BookmarkImport::Folder {
                        title: folder.title,
                        guid: folder.guid,
                        added_at_ms: folder.added_at_ms,
                        children: Vec::new(),
                    };
                    let store = db.store();
                    store.import_bookmarks(parent, std::slice::from_ref(&empty), now)?;
                    let created = store
                        .bookmark_children(parent)?
                        .pop()
                        .ok_or(mercury_store::StoreError::NotFound)?;
                    ids.push(created.id);
                    parents.push(parent);
                }
                db.changed(Change::folders(parents));
                Ok(ids)
            },
            reply,
        );
    }

    /// Moves each folder to its place (index) in its parent: per parent, in descending order
    /// of place (see `bookmarks::import_plan`).
    pub(crate) fn place_folders(
        &self,
        places: Vec<(BookmarkId, BookmarkId, u32)>,
        reply: impl FnOnce(Result<(), ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| {
                let mut places = places;
                places.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)));
                let parents = places.iter().map(|place| place.1).collect::<Vec<_>>();
                db.changed(Change::folders(parents));
                let store = db.store();
                for (folder, parent, place) in places {
                    store.move_bookmark(folder, parent, Some(place), now)?;
                }
                Ok(())
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the bookmarks UI edits through edit_bookmark")
    )]
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the bookmarks UI edits through edit_bookmark")
    )]
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

// The stepwise bookmark import (the service's visit import is in `import`). Chunks of whole
// top-level trees, up to `IMPORT_CHUNK` nodes unless one tree is larger: importers split large
// folders into sets of their own first (see `bookmarks::import_plan`). Each chunk is one
// transaction; an entry the store refuses (a URL too long, nesting too deep, a GUID in use) is
// left out with everything in it and its chunk is tried again.

/// The nodes of one tree, counting its descendants.
fn node_count(node: &BookmarkImport) -> usize {
    let mut count = 0;
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        count += 1;
        if let BookmarkImport::Folder { children, .. } = node {
            stack.extend(children);
        }
    }
    count
}

/// Removes the node at `index` in a pre-order walk of all the trees of `sets`, in order, with
/// everything in it; returns how many nodes went.
fn remove_in_pre_order(sets: &mut [(BookmarkId, Vec<BookmarkImport>)], index: usize) -> usize {
    fn walk(nodes: &mut Vec<BookmarkImport>, index: &mut usize) -> Option<usize> {
        let mut position = 0;
        while position < nodes.len() {
            if *index == 0 {
                return Some(node_count(&nodes.remove(position)));
            }
            *index -= 1;
            if let BookmarkImport::Folder { children, .. } = &mut nodes[position]
                && let Some(removed) = walk(children, index)
            {
                return Some(removed);
            }
            position += 1;
        }
        None
    }
    let mut index = index;
    sets.iter_mut()
        .find_map(|(_, nodes)| walk(nodes, &mut index))
        .unwrap_or(0)
}

struct BookmarkSteps {
    /// Top-level nodes with their parents and sizes, in order, consumed from the front.
    nodes: VecDeque<(BookmarkId, BookmarkImport, usize)>,
    imported: u64,
    skipped: u64,
    total: u64,
    resume_from: u64,
    longest_chunk: Duration,
}

impl BookmarkSteps {
    fn summary(&self, outcome: ImportOutcome) -> Payload {
        Box::new(ImportSummary {
            imported: self.imported,
            skipped: self.skipped,
            total: self.total,
            resume_from: self.resume_from,
            outcome,
            longest_chunk: self.longest_chunk,
        })
    }
}

impl Steps for BookmarkSteps {
    fn step(&mut self, database: &mut Database, now: i64) -> Step {
        let started = Instant::now();
        let mut chunk: Vec<(BookmarkId, Vec<BookmarkImport>)> = Vec::new();
        let mut size = 0;
        let mut top_level = 0;
        while let Some((_, _, count)) = self.nodes.front() {
            if size > 0 && size + count > IMPORT_CHUNK {
                break;
            }
            let Some((parent, node, count)) = self.nodes.pop_front() else {
                break;
            };
            size += count;
            top_level += 1;
            match chunk.last_mut() {
                Some((last, nodes)) if *last == parent => nodes.push(node),
                _ => chunk.push((parent, vec![node])),
            }
        }
        let mut skipped = 0;
        loop {
            let sets = chunk
                .iter()
                .map(|(parent, nodes)| (*parent, nodes.as_slice()))
                .collect::<Vec<_>>();
            match database.store.import_bookmark_sets(&sets, now) {
                Ok(imported) => {
                    for (parent, _) in &chunk {
                        database.changed(Change::folders([*parent]));
                    }
                    self.imported += imported;
                    self.skipped += skipped as u64;
                    self.resume_from += top_level;
                    self.longest_chunk = self.longest_chunk.max(started.elapsed());
                    let wrote = imported > 0;
                    return if self.nodes.is_empty() {
                        Step::Finished {
                            summary: self.summary(ImportOutcome::Completed),
                            wrote,
                        }
                    } else {
                        Step::Continue {
                            progress: Some(Box::new(ImportProgress {
                                done: self.imported + self.skipped,
                                skipped: self.skipped,
                                total: self.total,
                            })),
                            wrote,
                        }
                    };
                }
                Err(mercury_store::StoreError::ImportEntryRefused { index, reason }) => {
                    let removed = remove_in_pre_order(&mut chunk, index);
                    log::debug!("import: bookmark node left out ({removed} with it): {reason}");
                    if removed == 0 {
                        return self.failed_chunk(
                            started,
                            mercury_store::StoreError::InvalidArgument(
                                "a refused bookmark could not be found",
                            ),
                        );
                    }
                    skipped += removed;
                }
                Err(error) => return self.failed_chunk(started, error),
            }
        }
    }

    fn cancelled(&mut self) -> Payload {
        self.summary(ImportOutcome::Cancelled)
    }

    fn failed(&mut self, error: ProfileDataError) -> Payload {
        self.summary(ImportOutcome::Failed(error))
    }

    fn describe(&self) -> String {
        format!(
            "import of bookmarks, {} of {} imported, {} skipped",
            self.imported, self.total, self.skipped
        )
    }
}

impl BookmarkSteps {
    fn failed_chunk(&mut self, started: Instant, error: mercury_store::StoreError) -> Step {
        self.longest_chunk = self.longest_chunk.max(started.elapsed());
        Step::Finished {
            summary: self.summary(ImportOutcome::Failed(ProfileDataError::Store(error))),
            wrote: false,
        }
    }
}

impl ProfileData {
    /// Appends each tree under its parent folder, in order, in chunks of whole top-level nodes
    /// (see above). The trees carry their own GUIDs (make them with `new_guid`). Progress and
    /// the summary count nodes; `resume_from` counts top-level nodes. Notifies
    /// [`Area::Bookmarks`] with the parent folders when it ends, if anything was imported.
    pub(crate) fn import_bookmarks(
        &self,
        sets: Vec<(BookmarkId, Vec<BookmarkImport>)>,
        on_progress: impl Fn(ImportProgress) + 'static,
        on_done: impl FnOnce(ImportSummary) + 'static,
    ) -> JobHandle {
        let nodes = sets
            .into_iter()
            .flat_map(|(parent, nodes)| {
                nodes.into_iter().map(move |node| {
                    let count = node_count(&node);
                    (parent, node, count)
                })
            })
            .collect::<VecDeque<_>>();
        let total = nodes.iter().map(|(_, _, count)| *count as u64).sum();
        self.start_job(
            Some(Area::Bookmarks),
            BookmarkSteps {
                nodes,
                imported: 0,
                skipped: 0,
                total,
                resume_from: 0,
                longest_chunk: Duration::ZERO,
            },
            on_progress,
            on_done,
        )
    }
}
