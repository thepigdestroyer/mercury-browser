//! Bulk import of visits from another browser, as a stepwise job (see `jobs`): chunks of
//! [`IMPORT_CHUNK`] visits, each one transaction. Bookmarks are imported by the bookmarks and
//! import work's own stepwise jobs on `Store::import_bookmark_sets`.
//!
//! **Refused entries are skipped.** A visit the store refuses (a URL longer than
//! `MAX_URL_BYTES`) is left out and its chunk is tried again; the summary and the progress
//! count what was skipped. Any other failure stops the import.
//!
//! **Not atomic as a whole; resumable.** A cancel, a shutdown or a failure stops the import
//! after the chunks already committed, which are always a prefix of the input. To resume, call
//! again with the input minus its first [`ImportSummary::resume_from`] visits.
//!
//! Chunking does not change what the store keeps: visit counts, typed counts and last-visit
//! times are recomputed from the visit rows, and a page's title is that of its newest titled
//! visit whichever chunk brings it. The one difference is a tie at the same millisecond in
//! two chunks: the later chunk's title wins, where a single call would keep the greater.

#![cfg_attr(not(test), expect(dead_code, reason = "called by the import work"))]

use std::time::{Duration, Instant};

use mercury_store::{StoreError, VisitImport, VisitTransition};

use super::jobs::{JobHandle, Step, Steps};
use super::{Area, Database, Payload, ProfileData, ProfileDataError};

/// Visits per chunk. Sized by `import_benchmark_100k_visits` (see the lane report for the figures).
pub(crate) const IMPORT_CHUNK: usize = 500;

/// One visit to import (an owned `mercury_store::VisitImport`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportVisit {
    pub(crate) url: String,
    pub(crate) title: Option<String>,
    pub(crate) visited_at_ms: i64,
    pub(crate) transition: VisitTransition,
}

/// After each chunk: visits dealt with so far, imported or
/// skipped, of all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportProgress {
    pub(crate) done: u64,
    pub(crate) skipped: u64,
    pub(crate) total: u64,
}

/// How an import ended.
#[derive(Debug)]
pub(crate) enum ImportOutcome {
    Completed,
    Cancelled,
    /// A chunk failed for another reason than a refused entry; nothing of it was imported.
    Failed(ProfileDataError),
}

/// The end of an import.
#[derive(Debug)]
pub(crate) struct ImportSummary {
    /// Visits stored.
    pub(crate) imported: u64,
    /// Visits refused and left out.
    pub(crate) skipped: u64,
    /// All entries given.
    pub(crate) total: u64,
    /// Visits of the input dealt with, as a prefix. Resume by leaving out this many.
    pub(crate) resume_from: u64,
    pub(crate) outcome: ImportOutcome,
    /// The longest chunk (with its retries), which is the longest any request waited behind
    /// this import.
    pub(crate) longest_chunk: Duration,
}

/// Counts and timing shared by both kinds of import.
#[derive(Default)]
struct Tally {
    imported: u64,
    skipped: u64,
    total: u64,
    resume_from: u64,
    longest_chunk: Duration,
}

impl Tally {
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

    /// After a chunk that started at `started`: the next step.
    fn after_chunk(&mut self, started: Instant, imported: u64, finished: bool) -> Step {
        self.longest_chunk = self.longest_chunk.max(started.elapsed());
        if finished {
            Step::Finished {
                summary: self.summary(ImportOutcome::Completed),
                wrote: imported > 0,
            }
        } else {
            Step::Continue {
                progress: Some(Box::new(ImportProgress {
                    done: self.imported + self.skipped,
                    skipped: self.skipped,
                    total: self.total,
                })),
                wrote: imported > 0,
            }
        }
    }

    fn failed(&mut self, started: Instant, error: StoreError) -> Step {
        self.longest_chunk = self.longest_chunk.max(started.elapsed());
        Step::Finished {
            summary: self.summary(ImportOutcome::Failed(ProfileDataError::Store(error))),
            wrote: false,
        }
    }

    fn describe(&self, what: &str) -> String {
        format!(
            "import of {what}, {} of {} imported, {} skipped",
            self.imported, self.total, self.skipped
        )
    }
}

struct VisitSteps {
    visits: Vec<ImportVisit>,
    tally: Tally,
}

impl Steps for VisitSteps {
    fn step(&mut self, database: &mut Database, now: i64) -> Step {
        let started = Instant::now();
        let start = self.tally.resume_from as usize;
        let end = (start + IMPORT_CHUNK).min(self.visits.len());
        // The store refuses a visit only for its URL's length: those are left out at once, and
        // the retry below is the fallback.
        let mut left_out: Vec<usize> = (start..end)
            .filter(|&index| self.visits[index].url.len() > mercury_store::MAX_URL_BYTES)
            .collect();
        loop {
            let kept = (start..end)
                .filter(|index| !left_out.contains(index))
                .collect::<Vec<_>>();
            let chunk = kept
                .iter()
                .map(|&index| {
                    let visit = &self.visits[index];
                    VisitImport {
                        url: &visit.url,
                        title: visit.title.as_deref(),
                        visited_at_ms: visit.visited_at_ms,
                        transition: visit.transition,
                    }
                })
                .collect::<Vec<_>>();
            match database.store.import_visits(&chunk, now) {
                Ok(imported) => {
                    self.tally.imported += imported;
                    self.tally.skipped += left_out.len() as u64;
                    self.tally.resume_from = end as u64;
                    return self
                        .tally
                        .after_chunk(started, imported, end == self.visits.len());
                }
                Err(StoreError::ImportEntryRefused { index, reason }) => {
                    log::debug!("import: visit {} left out: {reason}", kept[index]);
                    left_out.push(kept[index]);
                }
                Err(error) => return self.tally.failed(started, error),
            }
        }
    }

    fn cancelled(&mut self) -> Payload {
        self.tally.summary(ImportOutcome::Cancelled)
    }

    fn failed(&mut self, error: ProfileDataError) -> Payload {
        self.tally.summary(ImportOutcome::Failed(error))
    }

    fn describe(&self) -> String {
        self.tally.describe("visits")
    }
}

impl ProfileData {
    /// Imports visits in chunks. `on_progress` runs after each chunk but the last, `on_done`
    /// once at the end. Notifies [`Area::History`] when it ends, if anything was imported. See
    /// the module documentation for refused entries and resuming.
    pub(crate) fn import_visits(
        &self,
        visits: Vec<ImportVisit>,
        on_progress: impl Fn(ImportProgress) + 'static,
        on_done: impl FnOnce(ImportSummary) + 'static,
    ) -> JobHandle {
        let total = visits.len() as u64;
        self.start_job(
            Some(Area::History),
            VisitSteps {
                visits,
                tally: Tally {
                    total,
                    ..Tally::default()
                },
            },
            on_progress,
            on_done,
        )
    }
}
