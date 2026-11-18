// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use mercury_store::Store;

use super::{ImportRun, Stage};
use crate::import_ui::fixtures;
use crate::import_ui::fixtures::{discover, read as read_profile};
use crate::import_ui::read::Choice;
use crate::import_ui::summary::{KindEnd, KindResult, ProfileResult};
use crate::profile_data::{ProfileData, Timing};

const PATIENCE: Duration = Duration::from_secs(60);
const BOTH: Choice = Choice {
    bookmarks: true,
    history: true,
};

/// A profile-data service on a database in a fresh directory; the test thread is the UI
/// thread.
struct Service {
    data: Rc<RefCell<Option<ProfileData>>>,
    wakes: Receiver<()>,
    directory: tempfile::TempDir,
}

impl Service {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("places.sqlite");
        let (wake, wakes) = mpsc::channel();
        let data = ProfileData::spawn(
            move || Store::open(&path).map_err(|error| error.to_string()),
            Arc::new(crate::profile_data::now_ms),
            Arc::new(move || {
                let _sent = wake.send(());
            }),
            Timing::default(),
            None,
        );
        Self {
            data: Rc::new(RefCell::new(Some(data))),
            wakes,
            directory,
        }
    }

    fn database(&self) -> PathBuf {
        self.directory.path().join("places.sqlite")
    }

    /// Reads `profile` and runs its import to the end; `during` sees every stage and may act.
    fn import(
        &self,
        root: &Path,
        choice: Choice,
        during: impl Fn(&ImportRun, Stage) + 'static,
    ) -> (ProfileResult, Vec<Stage>) {
        let profile = discover(&[root.to_owned()]).remove(0);
        let prepared = read_profile(&profile, choice);
        let stages = Rc::new(RefCell::new(Vec::new()));
        let result = Rc::new(RefCell::new(None));
        let data = Rc::clone(&self.data);
        let run_slot: Rc<RefCell<Option<ImportRun>>> = Rc::new(RefCell::new(None));
        let (seen, slot) = (Rc::clone(&stages), Rc::clone(&run_slot));
        let done = Rc::clone(&result);
        let run = ImportRun::new(
            profile.label(),
            profile.browser.clone(),
            Rc::new(move || data.borrow().clone()),
            move |stage| {
                seen.borrow_mut().push(stage);
                if let Some(run) = slot.borrow().as_ref() {
                    during(run, stage);
                }
            },
            move |summary| *done.borrow_mut() = Some(summary),
        );
        *run_slot.borrow_mut() = Some(run.clone());
        run.prepared(prepared);
        let deadline = Instant::now() + PATIENCE;
        while result.borrow().is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            self.wakes.recv_timeout(left).expect("the service answers");
            if let Some(data) = self.data.borrow().clone() {
                data.deliver_replies();
            }
        }
        run_slot.borrow_mut().take();
        let result = result.borrow_mut().take().unwrap();
        (result, stages.take())
    }

    /// Waits until every request sent so far has run.
    fn sync(&self) {
        let done = Rc::new(RefCell::new(false));
        let flag = Rc::clone(&done);
        let data = self.data.borrow().clone().unwrap();
        data.bookmark_root(mercury_store::BookmarkRoot::Other, move |_| {
            *flag.borrow_mut() = true;
        });
        let deadline = Instant::now() + PATIENCE;
        while !*done.borrow() {
            let left = deadline.saturating_duration_since(Instant::now());
            self.wakes.recv_timeout(left).expect("the service answers");
            data.deliver_replies();
        }
    }

    fn count(&self, sql: &str) -> i64 {
        rusqlite::Connection::open(self.database())
            .unwrap()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }
}

fn completed(imported: u64, already_present: u64, total: u64) -> Option<KindResult> {
    Some(KindResult {
        imported,
        already_present,
        skipped: 0,
        total,
        end: KindEnd::Completed,
    })
}

const VISITS: &str = "SELECT count(*) FROM visits";
const VISIT_COUNTS: &str = "SELECT coalesce(sum(visit_count), 0) FROM places";
const BOOKMARKS: &str = "SELECT count(*) FROM bookmarks WHERE kind = 1";

#[test]
fn a_firefox_profile_is_imported_with_its_counts() {
    let source = tempfile::tempdir().unwrap();
    fixtures::firefox_profile(source.path(), "Default", fixtures::SAMPLE_FIREFOX);
    let service = Service::new();

    let (result, stages) = service.import(source.path(), BOTH, |_, _| {});

    assert_eq!(
        result,
        ProfileResult {
            label: "Firefox — Default".into(),
            bookmarks: completed(3, 0, 3),
            history: completed(4, 0, 4),
        }
    );
    assert_eq!(
        stages.first(),
        Some(&Stage::Bookmarks { done: 0, total: 3 })
    );
    assert!(stages.contains(&Stage::History { done: 0, total: 4 }));
    assert_eq!(service.count(VISITS), 4);
    assert_eq!(service.count(VISIT_COUNTS), 4);
    assert_eq!(service.count(BOOKMARKS), 3);
    assert_eq!(
        service.count(
            "SELECT count(*) FROM bookmarks b JOIN bookmarks root ON root.id = b.parent_id
             WHERE b.title = 'Imported from Firefox' AND root.guid = 'menu________'"
        ),
        1,
        "one folder under Other Bookmarks"
    );
}

#[test]
fn importing_the_same_profile_twice_doubles_nothing() {
    let source = tempfile::tempdir().unwrap();
    fixtures::firefox_profile(source.path(), "Default", fixtures::SAMPLE_FIREFOX);
    let service = Service::new();
    service.import(source.path(), BOTH, |_, _| {});

    let (again, _) = service.import(source.path(), BOTH, |_, _| {});

    assert_eq!(again.bookmarks, completed(0, 3, 3));
    assert_eq!(again.history, completed(0, 4, 4));
    assert_eq!(service.count(VISITS), 4);
    assert_eq!(service.count(VISIT_COUNTS), 4);
    assert_eq!(service.count(BOOKMARKS), 3);
    assert_eq!(service.count("SELECT count(*) FROM bookmarks"), 2 + 1 + 7);
}

#[test]
fn a_cancelled_import_says_how_far_it_got_and_a_second_run_adds_exactly_the_rest() {
    let source = tempfile::tempdir().unwrap();
    let visits = 30_000;
    fixtures::firefox_profile(
        source.path(),
        "Default",
        &format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {visits})
             INSERT INTO moz_places (id, url, title)
                 SELECT i, 'https://site' || (i % 7000) || '.example/' || i, 'Page ' || i FROM n;
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {visits})
             INSERT INTO moz_historyvisits (place_id, visit_date, visit_type)
                 SELECT i, 1700000000000000 + i * 1000000, 1 FROM n;"
        ),
    );
    let service = Service::new();
    let history_only = Choice {
        bookmarks: false,
        history: true,
    };

    let (cancelled, _) = service.import(source.path(), history_only, |run, stage| {
        if let Stage::History { done, .. } = stage
            && done > 0
        {
            run.cancel();
        }
    });

    let history = cancelled.history.unwrap();
    assert_eq!(history.end, KindEnd::Cancelled);
    assert!(
        history.imported > 0 && history.imported < visits,
        "{history:?}"
    );
    assert_eq!(service.count(VISITS), history.imported as i64);

    let (again, _) = service.import(source.path(), history_only, |_, _| {});
    assert_eq!(
        again.history,
        completed(visits - history.imported, history.imported, visits)
    );
    assert_eq!(service.count(VISITS), visits as i64);
    assert_eq!(service.count(VISIT_COUNTS), visits as i64);
}

#[test]
fn a_profile_with_only_bookmarks_imports_its_bookmarks() {
    let source = tempfile::tempdir().unwrap();
    fixtures::chromium_profile(source.path(), "Profile 1", "Reading", true, false);
    let service = Service::new();

    let (result, _) = service.import(
        source.path(),
        Choice {
            bookmarks: true,
            history: false,
        },
        |_, _| {},
    );

    assert_eq!(result.bookmarks, completed(3, 0, 3));
    assert_eq!(result.history, None);
    assert_eq!(service.count(BOOKMARKS), 3);
}

#[test]
fn an_unreadable_kind_is_reported_and_the_other_is_still_imported() {
    let source = tempfile::tempdir().unwrap();
    let path = fixtures::chromium_profile(source.path(), "Default", "Person 1", true, true);
    std::fs::write(
        path.join("History"),
        b"not a database, only text pretending to be one",
    )
    .unwrap();
    let service = Service::new();

    let (result, _) = service.import(source.path(), BOTH, |_, _| {});

    assert_eq!(result.bookmarks, completed(3, 0, 3));
    assert_eq!(
        result.history,
        Some(KindResult::unreadable("the database is damaged.".into()))
    );
}

#[test]
fn without_the_profile_database_every_kind_fails_without_a_crash() {
    let source = tempfile::tempdir().unwrap();
    fixtures::firefox_profile(source.path(), "Default", fixtures::SAMPLE_FIREFOX);
    let service = Service::new();
    let data = service.data.borrow_mut().take().unwrap();
    data.deliver_replies();
    let profile = discover(&[source.path().to_owned()]).remove(0);
    let result = Rc::new(RefCell::new(None));
    let done = Rc::clone(&result);
    let run = ImportRun::new(
        profile.label(),
        profile.browser.clone(),
        Rc::new(|| None),
        |_| {},
        move |summary| *done.borrow_mut() = Some(summary),
    );

    run.prepared(read_profile(&profile, BOTH));

    let result = result.borrow_mut().take().expect("finished at once");
    for kind in [result.bookmarks.unwrap(), result.history.unwrap()] {
        assert_eq!(
            kind.end,
            KindEnd::Failed("the profile database is not available.".into())
        );
    }
}

#[test]
fn a_first_import_cancelled_before_anything_landed_leaves_no_empty_folder() {
    let source = tempfile::tempdir().unwrap();
    fixtures::firefox_profile(source.path(), "Default", fixtures::SAMPLE_FIREFOX);
    let service = Service::new();

    let (result, _) = service.import(source.path(), BOTH, |run, stage| {
        if stage == (Stage::Bookmarks { done: 0, total: 3 }) {
            run.cancel();
        }
    });

    assert_eq!(result.bookmarks.unwrap().end, KindEnd::Cancelled);
    assert_eq!(result.history.unwrap().end, KindEnd::Cancelled);
    service.sync();
    assert_eq!(
        service.count("SELECT count(*) FROM bookmarks WHERE title = 'Imported from Firefox'"),
        0
    );
    assert_eq!(service.count(VISITS), 0);
}
