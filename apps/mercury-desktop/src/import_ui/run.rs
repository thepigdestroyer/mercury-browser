//! One profile's import, driven on the UI thread once its files have been read: bookmarks
//! first (find or create the folder, leave out what is already there, import the rest in
//! chunks), then history (leave out visits already there, import the rest in chunks). Every
//! step is a request to the profile-data service; between chunks the service answers other
//! requests, so browsing, history recording and the star button keep working.

use std::cell::RefCell;
use std::rc::Rc;

use mercury_store::{BookmarkId, BookmarkImport, BookmarkRoot};

use super::convert::{Converted, bookmark_count, bookmarks_in_first, folder_title};
use super::read::Prepared;
use super::summary::{KindEnd, KindResult, ProfileResult};
use crate::profile_data::import::{ImportOutcome, ImportProgress, ImportSummary, ImportVisit};
use crate::profile_data::import_merge::{FilterOutcome, FilterProgress, FilterSummary};
use crate::profile_data::jobs::JobHandle;
use crate::profile_data::{ProfileData, ProfileDataError};

/// Where the service handle comes from: `profile_data::handle` in the browser. Asked afresh
/// at every step, so a run never keeps a service alive.
pub(crate) type DataSource = Rc<dyn Fn() -> Option<ProfileData>>;

type Finish = Box<dyn FnOnce(ProfileResult)>;

/// Where a run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Reading,
    Bookmarks { done: u64, total: u64 },
    CheckingHistory { checked: u64, total: u64 },
    History { done: u64, total: u64 },
}

impl Stage {
    pub(crate) fn title(self, label: &str) -> String {
        match self {
            Stage::Reading => format!("Reading {label}…"),
            Stage::Bookmarks { .. } => format!("Importing bookmarks from {label}…"),
            Stage::CheckingHistory { .. } => {
                "Checking which visits are already in Mercury Browser…".to_owned()
            }
            Stage::History { .. } => format!("Importing history from {label}…"),
        }
    }

    pub(crate) fn detail(self) -> String {
        use super::summary::thousands;
        match self {
            Stage::Reading => String::new(),
            Stage::Bookmarks { done, total } => {
                format!("{} of {} bookmarks", thousands(done), thousands(total))
            }
            Stage::CheckingHistory { checked, total } => {
                format!(
                    "{} of {} visits checked",
                    thousands(checked),
                    thousands(total)
                )
            }
            Stage::History { done, total } => {
                format!("{} of {} visits", thousands(done), thousands(total))
            }
        }
    }

    /// How far the stage is, if that is known.
    pub(crate) fn fraction(self) -> Option<f32> {
        let (done, total) = match self {
            Stage::Reading => return None,
            Stage::Bookmarks { done, total } | Stage::History { done, total } => (done, total),
            Stage::CheckingHistory { checked, total } => (checked, total),
        };
        Some(if total == 0 {
            1.0
        } else {
            (done as f64 / total as f64) as f32
        })
    }
}

/// A run; clones share it.
#[derive(Clone)]
pub(crate) struct ImportRun(Rc<Inner>);

struct Inner {
    label: String,
    browser: String,
    data: DataSource,
    on_stage: Box<dyn Fn(Stage)>,
    on_finish: RefCell<Option<Finish>>,
    state: RefCell<RunState>,
}

#[derive(Default)]
struct RunState {
    cancelled: bool,
    job: Option<JobHandle>,
    bookmarks: Option<KindResult>,
    history: Option<KindResult>,
    pending_history: Option<Result<Converted<Vec<ImportVisit>>, String>>,
}

fn store_error(error: &ProfileDataError) -> String {
    format!("{error}.")
}

impl ImportRun {
    /// A run for the profile shown as `label` of `browser`, waiting for its files to be read.
    pub(crate) fn new(
        label: String,
        browser: String,
        data: DataSource,
        on_stage: impl Fn(Stage) + 'static,
        on_finish: impl FnOnce(ProfileResult) + 'static,
    ) -> Self {
        Self(Rc::new(Inner {
            label,
            browser,
            data,
            on_stage: Box::new(on_stage),
            on_finish: RefCell::new(Some(Box::new(on_finish))),
            state: RefCell::default(),
        }))
    }

    /// Stops the run between chunks; the summary says what was imported so far.
    pub(crate) fn cancel(&self) {
        let job = {
            let mut state = self.0.state.borrow_mut();
            state.cancelled = true;
            state.job.clone()
        };
        if let Some(job) = job {
            job.cancel();
        }
    }

    /// The profile's files have been read: import what was read.
    pub(crate) fn prepared(&self, prepared: Prepared) {
        self.0.state.borrow_mut().pending_history = prepared.history;
        match prepared.bookmarks {
            None => self.history(),
            Some(Err(reason)) => {
                self.0.state.borrow_mut().bookmarks = Some(KindResult::unreadable(reason));
                self.history();
            }
            Some(Ok(converted)) => self.bookmarks(converted),
        }
    }

    fn cancelled(&self) -> bool {
        self.0.state.borrow().cancelled
    }

    fn bookmarks(&self, converted: Converted<Vec<BookmarkImport>>) {
        let total = bookmark_count(&converted.items);
        let skipped = converted.skipped;
        let stopped = move |end| KindResult {
            imported: 0,
            already_present: 0,
            skipped,
            total,
            end,
        };
        if self.cancelled() {
            self.0.state.borrow_mut().bookmarks = Some(stopped(KindEnd::Cancelled));
            return self.history();
        }
        let Some(data) = (self.0.data)() else {
            self.0.state.borrow_mut().bookmarks = Some(stopped(KindEnd::Failed(unavailable())));
            return self.history();
        };
        (self.0.on_stage)(Stage::Bookmarks { done: 0, total });
        let run = self.clone();
        data.plan_bookmark_import(
            BookmarkRoot::Other,
            folder_title(&self.0.browser),
            converted.items,
            move |plan| match plan {
                Err(error) => {
                    run.0.state.borrow_mut().bookmarks =
                        Some(stopped(KindEnd::Failed(store_error(&error))));
                    run.history();
                }
                Ok(plan) => {
                    log::debug!(
                        "importing bookmarks into folder {:?} (new: {}); {} nodes already there",
                        plan.folder,
                        plan.created_folder,
                        plan.already_present
                    );
                    let new = plan
                        .sets
                        .iter()
                        .map(|(_, nodes)| bookmark_count(nodes))
                        .sum::<u64>();
                    run.import_bookmarks(plan.sets, total, total - new, skipped);
                }
            },
        );
    }

    fn import_bookmarks(
        &self,
        sets: Vec<(BookmarkId, Vec<BookmarkImport>)>,
        total: u64,
        already_present: u64,
        skipped: u64,
    ) {
        let result = move |imported, end| KindResult {
            imported,
            already_present,
            skipped,
            total,
            end,
        };
        let data = match (self.cancelled(), (self.0.data)()) {
            (true, _) => {
                self.0.state.borrow_mut().bookmarks = Some(result(0, KindEnd::Cancelled));
                return self.history();
            }
            (false, None) => {
                self.0.state.borrow_mut().bookmarks =
                    Some(result(0, KindEnd::Failed(unavailable())));
                return self.history();
            }
            (false, Some(data)) => data,
        };
        // Kept to count the bookmarks among the nodes a stopped import had imported.
        let shape = Rc::new(sets.clone());
        let progress_shape = Rc::clone(&shape);
        let progress_run = self.clone();
        let run = self.clone();
        let job = data.import_bookmarks(
            sets,
            move |progress: ImportProgress| {
                (progress_run.0.on_stage)(Stage::Bookmarks {
                    done: already_present + bookmarks_in_first(&progress_shape, progress.done),
                    total,
                });
            },
            move |summary: ImportSummary| {
                log::debug!("bookmark import: longest chunk {:?}", summary.longest_chunk);
                // Chunks are whole top-level nodes, so the nodes dealt with (imported or
                // refused) are a prefix of the pre-order walk. The service counts refusals in
                // nodes; they are taken as bookmarks here (exact unless a whole folder was
                // refused, which the conversion's own checks make unlikely).
                let dealt_with = bookmarks_in_first(&shape, summary.imported + summary.skipped);
                let refused = summary.skipped.min(dealt_with);
                let mut kind = result(dealt_with - refused, end_of_import(&summary.outcome));
                kind.skipped += refused;
                run.0.state.borrow_mut().bookmarks = Some(kind);
                run.history();
            },
        );
        self.0.state.borrow_mut().job = Some(job);
    }

    fn history(&self) {
        let pending = self.0.state.borrow_mut().pending_history.take();
        let converted = match pending {
            None => return self.finish(),
            Some(Err(reason)) => {
                self.0.state.borrow_mut().history = Some(KindResult::unreadable(reason));
                return self.finish();
            }
            Some(Ok(converted)) => converted,
        };
        let total = converted.items.len() as u64;
        let skipped = converted.skipped;
        let stopped = move |end| KindResult {
            imported: 0,
            already_present: 0,
            skipped,
            total: total + skipped,
            end,
        };
        let data = match (self.cancelled(), (self.0.data)()) {
            (true, _) => {
                self.0.state.borrow_mut().history = Some(stopped(KindEnd::Cancelled));
                return self.finish();
            }
            (false, None) => {
                self.0.state.borrow_mut().history = Some(stopped(KindEnd::Failed(unavailable())));
                return self.finish();
            }
            (false, Some(data)) => data,
        };
        (self.0.on_stage)(Stage::CheckingHistory { checked: 0, total });
        let progress_run = self.clone();
        let run = self.clone();
        let job = data.filter_new_visits(
            converted.items,
            move |progress: FilterProgress| {
                (progress_run.0.on_stage)(Stage::CheckingHistory {
                    checked: progress.checked,
                    total: progress.total,
                });
            },
            move |summary: FilterSummary| {
                log::debug!(
                    "visit comparison: longest chunk {:?}",
                    summary.longest_chunk
                );
                match summary.outcome {
                    FilterOutcome::Completed => {
                        run.import_visits(summary.new, summary.already_present, skipped)
                    }
                    FilterOutcome::Cancelled => {
                        run.0.state.borrow_mut().history = Some(stopped(KindEnd::Cancelled));
                        run.finish();
                    }
                    FilterOutcome::Failed(error) => {
                        run.0.state.borrow_mut().history =
                            Some(stopped(KindEnd::Failed(store_error(&error))));
                        run.finish();
                    }
                }
            },
        );
        self.0.state.borrow_mut().job = Some(job);
    }

    fn import_visits(&self, visits: Vec<ImportVisit>, already_present: u64, skipped: u64) {
        let total = visits.len() as u64 + already_present;
        let result = move |imported, end| KindResult {
            imported,
            already_present,
            skipped,
            total: total + skipped,
            end,
        };
        let data = match (self.cancelled(), (self.0.data)()) {
            (true, _) => {
                self.0.state.borrow_mut().history = Some(result(0, KindEnd::Cancelled));
                return self.finish();
            }
            (false, None) => {
                self.0.state.borrow_mut().history = Some(result(0, KindEnd::Failed(unavailable())));
                return self.finish();
            }
            (false, Some(data)) => data,
        };
        (self.0.on_stage)(Stage::History {
            done: already_present,
            total,
        });
        let progress_run = self.clone();
        let run = self.clone();
        let job = data.import_visits(
            visits,
            move |progress: ImportProgress| {
                (progress_run.0.on_stage)(Stage::History {
                    done: already_present + progress.done,
                    total,
                });
            },
            move |summary: ImportSummary| {
                log::debug!("visit import: longest chunk {:?}", summary.longest_chunk);
                let mut kind = result(summary.imported, end_of_import(&summary.outcome));
                kind.skipped += summary.skipped;
                run.0.state.borrow_mut().history = Some(kind);
                run.finish();
            },
        );
        self.0.state.borrow_mut().job = Some(job);
    }

    fn finish(&self) {
        let result = {
            let mut state = self.0.state.borrow_mut();
            state.job = None;
            ProfileResult {
                label: self.0.label.clone(),
                bookmarks: state.bookmarks.take(),
                history: state.history.take(),
            }
        };
        let finish = self.0.on_finish.borrow_mut().take();
        if let Some(finish) = finish {
            finish(result);
        }
    }
}

fn end_of_import(outcome: &ImportOutcome) -> KindEnd {
    match outcome {
        ImportOutcome::Completed => KindEnd::Completed,
        ImportOutcome::Cancelled => KindEnd::Cancelled,
        ImportOutcome::Failed(error) => KindEnd::Failed(store_error(error)),
    }
}

fn unavailable() -> String {
    "the profile database is not available.".to_owned()
}

#[cfg(test)]
mod tests;
