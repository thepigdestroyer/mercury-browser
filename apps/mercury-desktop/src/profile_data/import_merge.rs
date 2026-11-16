//! What an import from another browser adds that the profile does not already hold (import-ui
//! lane). The store keeps every visit it is given and every bookmark under a fresh GUID, so
//! importing the same profile twice would double visit counts and the bookmark tree. These two
//! requests run before the service's own chunked imports (`import`) and leave out what is
//! already there:
//!
//! - [`ProfileData::filter_new_visits`], a stepwise read-only job: a visit is already present
//!   when the profile holds a visit to the same URL at the same millisecond (counted, so two
//!   source visits in one millisecond match two stored ones, not one).
//! - [`ProfileData::plan_bookmark_import`]: finds or creates the folder the import goes into
//!   and, if it already existed, merges the tree into it: a folder matches an existing folder
//!   of the same title at the same place, a bookmark one with the same title and URL; matched
//!   nodes are left out, separators in a folder that already existed too.
//!
//! An interrupted import can therefore be run again from the start: what the earlier run
//! committed is recognised and skipped.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use mercury_store::{
    BookmarkId, BookmarkImport, BookmarkKind, BookmarkRoot, HistoryCursor, Store, VisitId,
};

use super::import::ImportVisit;
use super::jobs::{JobHandle, Step, Steps};
use super::{Area, Database, Payload, ProfileData, ProfileDataError, guid_or_error};

/// Source visits compared per chunk of the filter job (a chunk is extended to keep visits of one
/// millisecond together). Sized by `filter_benchmark_100k_visits`; see the import-ui report.
pub(crate) const FILTER_CHUNK: usize = 2_000;
/// Stored visits read per query while a chunk is compared.
const HISTORY_PAGE: u32 = 1_000;

/// After each chunk of the filter job: source visits compared so far, of all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FilterProgress {
    pub(crate) checked: u64,
    pub(crate) total: u64,
}

/// How the filter job ended.
#[derive(Debug)]
pub(crate) enum FilterOutcome {
    Completed,
    Cancelled,
    Failed(ProfileDataError),
}

/// The visits not yet in the profile, in the order given, and how many were.
#[derive(Debug)]
pub(crate) struct FilterSummary {
    pub(crate) new: Vec<ImportVisit>,
    pub(crate) already_present: u64,
    pub(crate) outcome: FilterOutcome,
    pub(crate) longest_chunk: Duration,
}

struct VisitFilter {
    rest: std::iter::Peekable<std::vec::IntoIter<ImportVisit>>,
    total: u64,
    checked: u64,
    new: Vec<ImportVisit>,
    already_present: u64,
    longest_chunk: Duration,
}

impl VisitFilter {
    fn summary(&mut self, outcome: FilterOutcome) -> Payload {
        Box::new(FilterSummary {
            new: std::mem::take(&mut self.new),
            already_present: self.already_present,
            outcome,
            longest_chunk: self.longest_chunk,
        })
    }
}

impl Steps for VisitFilter {
    fn step(&mut self, database: &mut Database, _now: i64) -> Step {
        let started = Instant::now();
        let mut chunk = Vec::with_capacity(FILTER_CHUNK);
        while let Some(visit) = self.rest.next_if(|next| {
            chunk.len() < FILTER_CHUNK
                || chunk
                    .last()
                    .is_some_and(|last: &ImportVisit| last.visited_at_ms == next.visited_at_ms)
        }) {
            chunk.push(visit);
        }
        if let (Some(first), Some(last)) = (chunk.first(), chunk.last()) {
            let (low, high) = (
                first.visited_at_ms.min(last.visited_at_ms),
                first.visited_at_ms.max(last.visited_at_ms),
            );
            let mut known = match stored_visits(&database.store, low, high) {
                Ok(known) => known,
                Err(error) => {
                    return Step::Finished {
                        summary: self.summary(FilterOutcome::Failed(error.into())),
                        wrote: false,
                    };
                }
            };
            for visit in chunk {
                if known.take(&visit.url, visit.visited_at_ms) {
                    self.already_present += 1;
                } else {
                    self.new.push(visit);
                }
                self.checked += 1;
            }
        }
        self.longest_chunk = self.longest_chunk.max(started.elapsed());
        if self.rest.peek().is_none() {
            Step::Finished {
                summary: self.summary(FilterOutcome::Completed),
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
        self.summary(FilterOutcome::Cancelled)
    }

    fn failed(&mut self, error: ProfileDataError) -> Payload {
        self.summary(FilterOutcome::Failed(error))
    }

    fn describe(&self) -> String {
        format!(
            "comparison of imported visits, {} of {} compared",
            self.checked, self.total
        )
    }
}

/// How many stored visits each (URL, time) has, for visits from `low` to `high` inclusive.
fn stored_visits(store: &Store, low: i64, high: i64) -> mercury_store::Result<VisitCounts> {
    let mut counts = VisitCounts::default();
    // Listing is newest first and strictly after the cursor: start just after `high`.
    let mut before = high.checked_add(1).map(|after_high| HistoryCursor {
        visited_at_ms: after_high,
        visit_id: VisitId(i64::MIN),
    });
    loop {
        let page = store.history(None, before, HISTORY_PAGE)?;
        let full = page.len() == HISTORY_PAGE as usize;
        before = page.last().map(|entry| entry.cursor());
        for entry in page {
            if entry.visited_at_ms < low {
                return Ok(counts);
            }
            counts.add(entry.url, entry.visited_at_ms);
        }
        if !full {
            return Ok(counts);
        }
    }
}

/// Stored visits by time, then URL.
#[derive(Default)]
struct VisitCounts(HashMap<i64, Vec<(String, u32)>>);

impl VisitCounts {
    fn add(&mut self, url: String, at: i64) {
        let urls = self.0.entry(at).or_default();
        match urls.iter_mut().find(|(known, _)| *known == url) {
            Some((_, count)) => *count += 1,
            None => urls.push((url, 1)),
        }
    }

    /// Takes one stored visit to `url` at `at`, if one is left.
    fn take(&mut self, url: &str, at: i64) -> bool {
        let Some((_, count)) = self
            .0
            .get_mut(&at)
            .and_then(|urls| urls.iter_mut().find(|(known, _)| known == url))
        else {
            return false;
        };
        if *count == 0 {
            return false;
        }
        *count -= 1;
        true
    }
}

/// Where an import's bookmarks go, and what of the tree is new.
#[derive(Debug)]
pub(crate) struct BookmarkPlan {
    /// The folder the import goes into.
    pub(crate) folder: BookmarkId,
    /// Whether this request created it.
    pub(crate) created_folder: bool,
    /// What to import, each under its parent: everything under `folder` when it was created,
    /// otherwise only the nodes not already present.
    pub(crate) sets: Vec<(BookmarkId, Vec<BookmarkImport>)>,
    /// Nodes of the tree already present (folders, bookmarks and separators).
    pub(crate) already_present: u64,
}

impl ProfileData {
    /// Compares `visits` with the profile's history in chunks of [`FILTER_CHUNK`], between
    /// other requests, and hands back those not yet present. Reads only. `visits` should be
    /// ordered by time (either way), as the readers return them, so each chunk spans a short
    /// time; any order gives the right answer.
    pub(crate) fn filter_new_visits(
        &self,
        visits: Vec<ImportVisit>,
        on_progress: impl Fn(FilterProgress) + 'static,
        on_done: impl FnOnce(FilterSummary) + 'static,
    ) -> JobHandle {
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
            },
            on_progress,
            on_done,
        )
    }

    /// Finds the folder titled `folder_title` directly under `root`, or creates it at the end
    /// of `root`, and plans the import of `tree` into it (see the module documentation).
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
    let Some(existing) = existing else {
        let guid = guid_or_error()?;
        let folder = store.add_bookmark_folder(root, None, folder_title, &guid, now)?;
        return Ok(BookmarkPlan {
            folder,
            created_folder: true,
            sets: if tree.is_empty() {
                Vec::new()
            } else {
                vec![(folder, tree)]
            },
            already_present: 0,
        });
    };
    let mut sets = Vec::new();
    let mut already_present = 0;
    merge(store, existing.id, tree, &mut sets, &mut already_present)?;
    Ok(BookmarkPlan {
        folder: existing.id,
        created_folder: false,
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
        match node {
            BookmarkImport::Folder {
                title,
                guid,
                added_at_ms,
                children,
            } => {
                let found = claim(&|existing| {
                    matches!(existing.kind, BookmarkKind::Folder) && existing.title == title
                });
                match found {
                    Some(id) => {
                        *already_present += 1;
                        descend.push((id, children));
                    }
                    None => fresh.push(BookmarkImport::Folder {
                        title,
                        guid,
                        added_at_ms,
                        children,
                    }),
                }
            }
            BookmarkImport::Bookmark {
                ref title, ref url, ..
            } => {
                let found = claim(&|existing| {
                    existing.title == *title
                        && matches!(&existing.kind, BookmarkKind::Bookmark { url: stored } if stored == url)
                });
                match found {
                    Some(_) => *already_present += 1,
                    None => fresh.push(node),
                }
            }
            BookmarkImport::Separator { .. } => *already_present += 1,
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
