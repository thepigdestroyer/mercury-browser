//! What an import from another browser adds that the profile does not already hold, and how
//! its bookmarks are written (import-ui lane). The store keeps every visit it is given and
//! every bookmark under a fresh GUID, so importing the same profile twice would double visit
//! counts and the bookmark tree. Here:
//!
//! - [`ProfileData::filter_new_visits`], a stepwise read-only job: a visit is already present
//!   when the profile holds a visit to the same URL at the same millisecond (counted, so two
//!   source visits in one millisecond match two stored ones, not one). It walks the source
//!   (newest first) and the stored history (newest first, through `Store::history`) together,
//!   skipping the stored rows between two source times without reading them. Each step does at
//!   most [`FILTER_STEP`] units of work, a unit being a source visit compared or a stored row
//!   read, so neither a flood of visits in one millisecond nor a dense history makes a step long.
//! - [`ProfileData::plan_bookmark_import`]: finds or creates the folder the import goes into and
//!   merges the tree into it: a folder matches an existing folder of the same title at the same
//!   place, a bookmark one with the same title and URL; matched nodes are left out. Separators
//!   are compared by count: a folder that holds `n` separators already has the first `n` of the
//!   source's, and the source's further ones are added (at the end, like every new node).
//! - [`ProfileData::insert_bookmarks`], a stepwise job writing what the plan left, at most
//!   [`BOOKMARK_STEP`] nodes per transaction whatever the shape of the tree: a folder too large
//!   for one step is written empty, and its contents follow in later steps. Each parent's new
//!   children are written in their source order.
//!
//! An interrupted import can therefore be run again from the start: what the earlier run
//! committed is recognised and skipped.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use mercury_store::{
    BookmarkId, BookmarkImport, BookmarkKind, BookmarkRoot, HistoryCursor, Store, VisitId,
};

use super::import::ImportVisit;
use super::jobs::{JobHandle, Step, Steps};
use super::{Area, Database, Payload, ProfileData, ProfileDataError, guid_or_error};

/// The most work one step of the comparison does: source visits compared plus stored visits
/// read. Sized by `filter_benchmark_100k_visits`; see the import-ui report.
pub(crate) const FILTER_STEP: u64 = 4_000;
/// The first read after a jump in the stored history; later reads at the same time double.
const FIRST_READ: u32 = 16;
/// The largest single read.
const LARGEST_READ: u32 = 1_024;
/// The most bookmark nodes written in one transaction. Sized by `bookmark_benchmark_5000_toolbar`:
/// 250 nodes took 22-48 ms at load average 60-85, 500 took 93-258 ms.
pub(crate) const BOOKMARK_STEP: usize = 250;

/// How a stepwise job of this module ended.
#[derive(Debug)]
pub(crate) enum StepOutcome {
    Completed,
    Cancelled,
    Failed(ProfileDataError),
}

/// After each step of the comparison: source visits compared so far, of all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FilterProgress {
    pub(crate) checked: u64,
    pub(crate) total: u64,
}

/// The visits not yet in the profile (newest first), and how many were.
#[derive(Debug)]
pub(crate) struct FilterSummary {
    pub(crate) new: Vec<ImportVisit>,
    pub(crate) already_present: u64,
    pub(crate) outcome: StepOutcome,
    pub(crate) longest_chunk: Duration,
    /// Stored visits read, for measurement.
    pub(crate) rows_read: u64,
}

struct VisitFilter {
    /// Source visits, newest first.
    rest: std::iter::Peekable<std::vec::IntoIter<ImportVisit>>,
    total: u64,
    checked: u64,
    new: Vec<ImportVisit>,
    already_present: u64,
    longest_chunk: Duration,
    rows_read: u64,
    /// Where the next read of the stored history starts (it lists rows strictly after it);
    /// `None` before the first read.
    cursor: Option<HistoryCursor>,
    /// No stored rows are left after `cursor`.
    exhausted: bool,
    /// The size of the next read at the current time.
    next_read: u32,
    /// Stored visits read and not yet matched, by time then URL. Times later than the source
    /// visit being compared are dropped: the source only goes back in time.
    stored: BTreeMap<i64, HashMap<String, u32>>,
}

impl VisitFilter {
    fn summary(&mut self, outcome: StepOutcome) -> Payload {
        Box::new(FilterSummary {
            new: std::mem::take(&mut self.new),
            already_present: self.already_present,
            outcome,
            longest_chunk: self.longest_chunk,
            rows_read: self.rows_read,
        })
    }

    /// The time the cursor stands at: every stored row later than it has been read or skipped.
    fn cursor_time(&self) -> i64 {
        self.cursor.map_or(i64::MAX, |cursor| cursor.visited_at_ms)
    }

    /// Reads stored rows until every row at `time` is known, within `budget` units; returns the
    /// units used, and whether all rows at `time` are known.
    fn read_down_to(
        &mut self,
        store: &Store,
        time: i64,
        budget: u64,
    ) -> mercury_store::Result<(u64, bool)> {
        // The rows between the cursor and `time` cannot match anything still to come: skip them.
        if !self.exhausted && self.cursor_time() > time {
            self.cursor = time.checked_add(1).map(|after| HistoryCursor {
                visited_at_ms: after,
                visit_id: VisitId(i64::MIN),
            });
            self.next_read = FIRST_READ;
        }
        let mut used = 0;
        while !self.exhausted && self.cursor_time() >= time {
            let limit = u64::from(self.next_read).min(budget - used);
            if limit == 0 {
                return Ok((used, false));
            }
            let limit = u32::try_from(limit).unwrap_or(LARGEST_READ);
            let page = store.history(None, self.cursor, limit)?;
            used += page.len() as u64;
            self.rows_read += page.len() as u64;
            self.exhausted = page.len() < limit as usize;
            if let Some(last) = page.last() {
                self.cursor = Some(last.cursor());
            }
            for entry in page {
                *self
                    .stored
                    .entry(entry.visited_at_ms)
                    .or_default()
                    .entry(entry.url)
                    .or_default() += 1;
            }
            self.next_read = (self.next_read * 2).min(LARGEST_READ);
        }
        Ok((used, true))
    }

    /// Takes one stored visit to `url` at `time`, if one is left.
    fn take(&mut self, url: &str, time: i64) -> bool {
        // Nothing later than `time` can match from now on.
        if let Some(after) = time.checked_add(1) {
            drop(self.stored.split_off(&after));
        }
        match self
            .stored
            .get_mut(&time)
            .and_then(|urls| urls.get_mut(url))
        {
            Some(count) if *count > 0 => {
                *count -= 1;
                true
            }
            _ => false,
        }
    }
}

impl Steps for VisitFilter {
    fn step(&mut self, database: &mut Database, _now: i64) -> Step {
        let started = Instant::now();
        let mut budget = FILTER_STEP;
        while budget > 0 {
            let Some(time) = self.rest.peek().map(|visit| visit.visited_at_ms) else {
                break;
            };
            match self.read_down_to(&database.store, time, budget) {
                Ok((used, complete)) => {
                    budget -= used;
                    if !complete || budget == 0 {
                        break;
                    }
                }
                Err(error) => {
                    self.longest_chunk = self.longest_chunk.max(started.elapsed());
                    return Step::Finished {
                        summary: self.summary(StepOutcome::Failed(error.into())),
                        wrote: false,
                    };
                }
            }
            let Some(visit) = self.rest.next() else {
                break;
            };
            budget -= 1;
            self.checked += 1;
            if self.take(&visit.url, visit.visited_at_ms) {
                self.already_present += 1;
            } else {
                self.new.push(visit);
            }
        }
        self.longest_chunk = self.longest_chunk.max(started.elapsed());
        if self.rest.peek().is_none() {
            Step::Finished {
                summary: self.summary(StepOutcome::Completed),
                wrote: false,
            }
        } else {
            Step::Continue {
                progress: Some(Box::new(FilterProgress {
                    checked: self.checked,
                    total: self.total,
                })),
                wrote: false,
            }
        }
    }

    fn cancelled(&mut self) -> Payload {
        self.summary(StepOutcome::Cancelled)
    }

    fn failed(&mut self, error: ProfileDataError) -> Payload {
        self.summary(StepOutcome::Failed(error))
    }

    fn describe(&self) -> String {
        format!(
            "comparison of imported visits, {} of {} compared",
            self.checked, self.total
        )
    }
}

/// Where an import's bookmarks go, and what of the tree is new.
#[derive(Debug)]
pub(crate) struct BookmarkPlan {
    /// The folder the import goes into.
    pub(crate) folder: BookmarkId,
    /// Whether this request created it.
    pub(crate) created_folder: bool,
    /// What to write, each under its parent, in order.
    pub(crate) sets: Vec<(BookmarkId, Vec<BookmarkImport>)>,
    /// Nodes of the tree already present (folders, bookmarks and separators).
    pub(crate) already_present: u64,
}

/// After each step of [`ProfileData::insert_bookmarks`]: what has been written so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InsertProgress {
    /// Bookmarks (not folders or separators) written.
    pub(crate) bookmarks: u64,
    /// Nodes of every kind written.
    pub(crate) nodes: u64,
}

/// The end of [`ProfileData::insert_bookmarks`].
#[derive(Debug)]
pub(crate) struct InsertSummary {
    pub(crate) bookmarks: u64,
    pub(crate) nodes: u64,
    pub(crate) outcome: StepOutcome,
    pub(crate) longest_chunk: Duration,
}

/// Nodes in `node`, itself included.
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

fn bookmarks_in(nodes: &[BookmarkImport]) -> u64 {
    let mut count = 0;
    let mut stack = nodes.iter().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        match node {
            BookmarkImport::Folder { children, .. } => stack.extend(children),
            BookmarkImport::Bookmark { .. } => count += 1,
            BookmarkImport::Separator { .. } => {}
        }
    }
    count
}

struct BookmarkInsert {
    /// Each parent's nodes still to write, in order; a parent appears once.
    work: VecDeque<(BookmarkId, VecDeque<BookmarkImport>)>,
    bookmarks: u64,
    nodes: u64,
    longest_chunk: Duration,
}

impl BookmarkInsert {
    fn summary(&self, outcome: StepOutcome) -> Payload {
        Box::new(InsertSummary {
            bookmarks: self.bookmarks,
            nodes: self.nodes,
            outcome,
            longest_chunk: self.longest_chunk,
        })
    }

    fn write_step(&mut self, store: &mut Store, now: i64) -> mercury_store::Result<()> {
        let mut budget = BOOKMARK_STEP;
        let mut sets: Vec<(BookmarkId, Vec<BookmarkImport>)> = Vec::new();
        // Folders written empty this step: (parent, GUID, contents to write later).
        let mut deferred: Vec<(BookmarkId, String, Vec<BookmarkImport>)> = Vec::new();
        'parents: while budget > 0 {
            let Some((parent, nodes)) = self.work.front_mut() else {
                break;
            };
            let parent = *parent;
            while budget > 0 {
                let Some(size) = nodes.front().map(node_count) else {
                    self.work.pop_front();
                    continue 'parents;
                };
                let node = if size <= budget {
                    budget -= size;
                    nodes.pop_front()
                } else if size > BOOKMARK_STEP {
                    // Never fits: written empty now, its contents later.
                    budget -= 1;
                    match nodes.pop_front() {
                        Some(BookmarkImport::Folder {
                            title,
                            guid,
                            added_at_ms,
                            children,
                        }) => {
                            deferred.push((parent, guid.clone(), children));
                            Some(BookmarkImport::Folder {
                                title,
                                guid,
                                added_at_ms,
                                children: Vec::new(),
                            })
                        }
                        other => other,
                    }
                } else {
                    // Fits in a step of its own: the next one.
                    break 'parents;
                };
                if let Some(node) = node {
                    match sets.last_mut() {
                        Some((last, written)) if *last == parent => written.push(node),
                        _ => sets.push((parent, vec![node])),
                    }
                }
            }
        }
        let slices = sets
            .iter()
            .map(|(parent, nodes)| (*parent, nodes.as_slice()))
            .collect::<Vec<_>>();
        let written = store.import_bookmark_sets(&slices, now)?;
        self.nodes += written;
        self.bookmarks += sets
            .iter()
            .map(|(_, nodes)| bookmarks_in(nodes))
            .sum::<u64>();
        // Find the folders written empty, by GUID, among their parents' children.
        let mut children_of: HashMap<BookmarkId, Vec<mercury_store::BookmarkNode>> = HashMap::new();
        for (parent, guid, contents) in deferred {
            let children = match children_of.entry(parent) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(store.bookmark_children(parent)?)
                }
            };
            let id = children
                .iter()
                .rev()
                .find(|node| node.guid == guid)
                .map(|node| node.id)
                .ok_or(mercury_store::StoreError::NotFound)?;
            self.work.push_back((id, contents.into()));
        }
        Ok(())
    }
}

impl Steps for BookmarkInsert {
    fn step(&mut self, database: &mut Database, now: i64) -> Step {
        let started = Instant::now();
        let before = self.nodes;
        let result = self.write_step(&mut database.store, now);
        self.longest_chunk = self.longest_chunk.max(started.elapsed());
        let wrote = self.nodes > before;
        match result {
            Err(error) => Step::Finished {
                summary: self.summary(StepOutcome::Failed(error.into())),
                wrote,
            },
            Ok(()) if self.work.is_empty() => Step::Finished {
                summary: self.summary(StepOutcome::Completed),
                wrote,
            },
            Ok(()) => Step::Continue {
                progress: Some(Box::new(InsertProgress {
                    bookmarks: self.bookmarks,
                    nodes: self.nodes,
                })),
                wrote,
            },
        }
    }

    fn cancelled(&mut self) -> Payload {
        self.summary(StepOutcome::Cancelled)
    }

    fn failed(&mut self, error: ProfileDataError) -> Payload {
        self.summary(StepOutcome::Failed(error))
    }

    fn describe(&self) -> String {
        format!("import of bookmarks, {} nodes written", self.nodes)
    }
}

impl ProfileData {
    /// Compares `visits` with the profile's history in steps of at most [`FILTER_STEP`] units
    /// of work, between other requests, and hands back those not yet present, newest first.
    /// Reads only.
    pub(crate) fn filter_new_visits(
        &self,
        mut visits: Vec<ImportVisit>,
        on_progress: impl Fn(FilterProgress) + 'static,
        on_done: impl FnOnce(FilterSummary) + 'static,
    ) -> JobHandle {
        visits.sort_by_key(|visit| std::cmp::Reverse(visit.visited_at_ms));
        let total = visits.len() as u64;
        self.start_job(
            None,
            VisitFilter {
                rest: visits.into_iter().peekable(),
                total,
                checked: 0,
                new: Vec::new(),
                already_present: 0,
                longest_chunk: Duration::ZERO,
                rows_read: 0,
                cursor: None,
                exhausted: false,
                next_read: FIRST_READ,
                stored: BTreeMap::new(),
            },
            on_progress,
            on_done,
        )
    }

    /// Finds the folder titled `folder_title` directly under `root`, or creates it at the end
    /// of `root`, and plans the import of `tree` into it (see the module documentation). Reads
    /// the existing folders the tree matches, in one request.
    pub(crate) fn plan_bookmark_import(
        &self,
        root: BookmarkRoot,
        folder_title: String,
        tree: Vec<BookmarkImport>,
        reply: impl FnOnce(Result<BookmarkPlan, ProfileDataError>) + 'static,
    ) {
        self.request(
            Some(Area::Bookmarks),
            move |db, now| plan(db.store(), root, &folder_title, tree, now),
            reply,
        );
    }

    /// Writes `sets` (each parent's new nodes, in order) in steps of at most [`BOOKMARK_STEP`]
    /// nodes, each its own transaction. Notifies [`Area::Bookmarks`] if anything was written.
    pub(crate) fn insert_bookmarks(
        &self,
        sets: Vec<(BookmarkId, Vec<BookmarkImport>)>,
        on_progress: impl Fn(InsertProgress) + 'static,
        on_done: impl FnOnce(InsertSummary) + 'static,
    ) -> JobHandle {
        let mut work: VecDeque<(BookmarkId, VecDeque<BookmarkImport>)> = VecDeque::new();
        for (parent, nodes) in sets {
            match work.iter_mut().find(|(known, _)| *known == parent) {
                Some((_, queued)) => queued.extend(nodes),
                None => work.push_back((parent, nodes.into())),
            }
        }
        self.start_job(
            Some(Area::Bookmarks),
            BookmarkInsert {
                work,
                bookmarks: 0,
                nodes: 0,
                longest_chunk: Duration::ZERO,
            },
            on_progress,
            on_done,
        )
    }
}

fn plan(
    store: &mut Store,
    root: BookmarkRoot,
    folder_title: &str,
    tree: Vec<BookmarkImport>,
    now: i64,
) -> mercury_store::Result<BookmarkPlan> {
    let root = store.bookmark_root(root)?;
    let existing = store
        .bookmark_children(root)?
        .into_iter()
        .find(|node| matches!(node.kind, BookmarkKind::Folder) && node.title == folder_title);
    let (folder, created_folder) = match existing {
        Some(existing) => (existing.id, false),
        None => {
            let guid = guid_or_error()?;
            let folder = store.add_bookmark_folder(root, None, folder_title, &guid, now)?;
            (folder, true)
        }
    };
    let mut sets = Vec::new();
    let mut already_present = 0;
    merge(store, folder, tree, &mut sets, &mut already_present)?;
    Ok(BookmarkPlan {
        folder,
        created_folder,
        sets,
        already_present,
    })
}

/// Leaves out of `nodes` what `folder` already holds, descending into matching folders, and
/// adds what is left to `sets`. The depth is bounded by the tree's, which the caller limits.
fn merge(
    store: &Store,
    folder: BookmarkId,
    nodes: Vec<BookmarkImport>,
    sets: &mut Vec<(BookmarkId, Vec<BookmarkImport>)>,
    already_present: &mut u64,
) -> mercury_store::Result<()> {
    let existing = store.bookmark_children(folder)?;
    let mut used = vec![false; existing.len()];
    let mut claim = |matches: &dyn Fn(&mercury_store::BookmarkNode) -> bool| {
        let index = existing
            .iter()
            .zip(&used)
            .position(|(node, used)| !used && matches(node))?;
        used[index] = true;
        Some(existing[index].id)
    };
    let mut fresh = Vec::new();
    let mut descend = Vec::new();
    for node in nodes {
        let found = match &node {
            BookmarkImport::Folder { title, .. } => claim(&|existing| {
                matches!(existing.kind, BookmarkKind::Folder) && existing.title == *title
            }),
            BookmarkImport::Bookmark { title, url, .. } => claim(&|existing| {
                existing.title == *title
                    && matches!(&existing.kind, BookmarkKind::Bookmark { url: stored } if stored == url)
            }),
            BookmarkImport::Separator { .. } => {
                claim(&|existing| matches!(existing.kind, BookmarkKind::Separator))
            }
        };
        match (found, node) {
            (Some(id), BookmarkImport::Folder { children, .. }) => {
                *already_present += 1;
                descend.push((id, children));
            }
            (Some(_), _) => *already_present += 1,
            (None, node) => fresh.push(node),
        }
    }
    if !fresh.is_empty() {
        sets.push((folder, fresh));
    }
    for (id, children) in descend {
        merge(store, id, children, sets, already_present)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
