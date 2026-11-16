use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use mercury_store::{BookmarkImport, BookmarkKind, BookmarkRoot, Store, VisitTransition};

use super::super::import::{ImportOutcome, ImportSummary, ImportVisit};
use super::super::{ProfileData, ProfileDataError, Timing, open_database};
use super::{BookmarkPlan, FILTER_CHUNK, FilterOutcome, FilterProgress, FilterSummary};

const NOW: i64 = 1_800_000_000_000;
const PATIENCE: Duration = Duration::from_secs(30);

/// A service on a database in a fresh directory; the test thread plays the UI thread.
struct Harness {
    data: ProfileData,
    wakes: Receiver<()>,
    directory: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("places.sqlite");
        let (wake, wakes) = mpsc::channel();
        let data = ProfileData::spawn(
            move || open_database(Some(path)),
            Arc::new(|| NOW),
            Arc::new(move || {
                let _sent = wake.send(());
            }),
            Timing::default(),
        );
        Self {
            data,
            wakes,
            directory,
        }
    }

    fn path(&self) -> PathBuf {
        self.directory.path().join("places.sqlite")
    }

    fn settle(&self, done: impl Fn() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done() {
            let left = deadline.saturating_duration_since(Instant::now());
            self.wakes
                .recv_timeout(left)
                .expect("the service should answer");
            self.data.deliver_replies();
        }
    }

    fn filter(&self, visits: Vec<ImportVisit>) -> (FilterSummary, Vec<FilterProgress>) {
        let progress = Rc::new(RefCell::new(Vec::new()));
        let summary = Rc::new(RefCell::new(None));
        let (seen, done) = (Rc::clone(&progress), Rc::clone(&summary));
        self.data.filter_new_visits(
            visits,
            move |step| seen.borrow_mut().push(step),
            move |result| *done.borrow_mut() = Some(result),
        );
        self.settle(|| summary.borrow().is_some());
        let summary = summary.borrow_mut().take().unwrap();
        (summary, progress.take())
    }

    fn import_visits(&self, visits: Vec<ImportVisit>) -> ImportSummary {
        let summary = Rc::new(RefCell::new(None));
        let done = Rc::clone(&summary);
        self.data.import_visits(
            visits,
            |_| {},
            move |result| *done.borrow_mut() = Some(result),
        );
        self.settle(|| summary.borrow().is_some());
        summary.borrow_mut().take().unwrap()
    }

    fn plan(&self, tree: Vec<BookmarkImport>) -> Result<BookmarkPlan, ProfileDataError> {
        let slot = Rc::new(RefCell::new(None));
        let fill = Rc::clone(&slot);
        self.data.plan_bookmark_import(
            BookmarkRoot::Other,
            "Imported from Firefox".to_owned(),
            tree,
            move |result| *fill.borrow_mut() = Some(result),
        );
        self.settle(|| slot.borrow().is_some());
        slot.borrow_mut().take().unwrap()
    }

    fn import_plan(&self, plan: BookmarkPlan) -> ImportSummary {
        let summary = Rc::new(RefCell::new(None));
        let done = Rc::clone(&summary);
        self.data.import_bookmarks(
            plan.sets,
            |_| {},
            move |result| *done.borrow_mut() = Some(result),
        );
        self.settle(|| summary.borrow().is_some());
        summary.borrow_mut().take().unwrap()
    }

    fn reopen(self) -> (Store, tempfile::TempDir) {
        self.data.shut_down(Instant::now() + PATIENCE);
        let store = Store::open(&self.path()).unwrap();
        (store, self.directory)
    }
}

fn visit(url: &str, at: i64) -> ImportVisit {
    ImportVisit {
        url: url.to_owned(),
        title: Some(format!("Title of {url}")),
        visited_at_ms: at,
        transition: VisitTransition::Link,
    }
}

/// `count` visits, newest first as the readers return them, over `pages` pages a minute apart.
fn history(count: usize, pages: usize) -> Vec<ImportVisit> {
    (0..count)
        .map(|index| {
            visit(
                &format!("https://site{}.example/", index % pages),
                NOW - 60_000 * index as i64,
            )
        })
        .collect()
}

fn counts(path: &std::path::Path) -> (i64, i64) {
    let conn = rusqlite::Connection::open(path).unwrap();
    let visits = conn
        .query_row("SELECT count(*) FROM visits", [], |row| row.get(0))
        .unwrap();
    let total = conn
        .query_row("SELECT sum(visit_count) FROM places", [], |row| row.get(0))
        .unwrap();
    (visits, total)
}

#[test]
fn a_new_profile_keeps_every_visit_and_reports_progress_per_chunk() {
    let harness = Harness::new();
    let visits = history(FILTER_CHUNK * 2 + 10, 300);

    let (summary, progress) = harness.filter(visits.clone());

    assert!(matches!(summary.outcome, FilterOutcome::Completed));
    assert_eq!(summary.already_present, 0);
    assert_eq!(summary.new, visits, "kept in order");
    let total = visits.len() as u64;
    assert_eq!(
        progress,
        vec![
            FilterProgress {
                checked: FILTER_CHUNK as u64,
                total
            },
            FilterProgress {
                checked: 2 * FILTER_CHUNK as u64,
                total
            },
        ]
    );
}

#[test]
fn importing_the_same_history_twice_adds_nothing_the_second_time() {
    let harness = Harness::new();
    let visits = history(3_000, 400);

    let (first, _) = harness.filter(visits.clone());
    assert_eq!(first.new.len(), 3_000);
    let imported = harness.import_visits(first.new);
    assert!(matches!(imported.outcome, ImportOutcome::Completed));

    let (second, _) = harness.filter(visits);
    assert_eq!(second.already_present, 3_000);
    assert!(second.new.is_empty());

    let path = harness.path();
    let (store, _directory) = harness.reopen();
    drop(store);
    assert_eq!(counts(&path), (3_000, 3_000), "visits and visit counts");
}

#[test]
fn an_interrupted_import_run_again_adds_only_the_rest() {
    let harness = Harness::new();
    let visits = history(1_200, 100);
    // An earlier run committed the first 500 (its first chunk) and was cancelled.
    harness.import_visits(visits[..500].to_vec());

    let (again, _) = harness.filter(visits.clone());
    assert_eq!(again.already_present, 500);
    assert_eq!(again.new, visits[500..].to_vec());
}

#[test]
fn visits_in_one_millisecond_are_matched_one_for_one_across_chunks() {
    let harness = Harness::new();
    // Two visits to one page in one millisecond already stored, three in the source; the
    // three straddle the chunk boundary, which is moved to keep them together.
    let mut visits = history(FILTER_CHUNK - 1, 50);
    let at = visits.last().unwrap().visited_at_ms - 1;
    for _ in 0..3 {
        visits.push(visit("https://same.example/", at));
    }
    visits.push(visit("https://after.example/", at - 1));
    harness.import_visits(vec![
        visit("https://same.example/", at),
        visit("https://same.example/", at),
    ]);

    let (summary, progress) = harness.filter(visits);

    assert_eq!(summary.already_present, 2);
    let same = summary
        .new
        .iter()
        .filter(|visit| visit.url == "https://same.example/")
        .count();
    assert_eq!(same, 1);
    assert_eq!(
        progress[0].checked,
        FILTER_CHUNK as u64 + 2,
        "the chunk keeps the millisecond together"
    );
}

#[test]
fn a_visit_to_another_page_or_at_another_time_is_new() {
    let harness = Harness::new();
    harness.import_visits(vec![visit("https://a.example/", NOW)]);

    let (summary, _) = harness.filter(vec![
        visit("https://a.example/", NOW + 1),
        visit("https://a.example/", NOW),
        visit("https://b.example/", NOW),
        visit("https://a.example/", NOW - 1),
    ]);

    assert_eq!(summary.already_present, 1);
    assert_eq!(
        summary
            .new
            .iter()
            .map(|visit| (visit.url.as_str(), visit.visited_at_ms))
            .collect::<Vec<_>>(),
        vec![
            ("https://a.example/", NOW + 1),
            ("https://b.example/", NOW),
            ("https://a.example/", NOW - 1),
        ]
    );
}

#[test]
fn a_cancelled_comparison_says_so() {
    let harness = Harness::new();
    let summary = Rc::new(RefCell::new(None));
    let done = Rc::clone(&summary);
    let job = harness.data.filter_new_visits(
        history(FILTER_CHUNK * 20, 100),
        |_| {},
        move |result| *done.borrow_mut() = Some(result),
    );
    job.cancel();
    harness.settle(|| summary.borrow().is_some());

    let summary = summary.borrow_mut().take().unwrap();
    assert!(matches!(summary.outcome, FilterOutcome::Cancelled));
}

fn bookmark(title: &str, url: &str) -> BookmarkImport {
    BookmarkImport::Bookmark {
        title: title.to_owned(),
        url: url.to_owned(),
        guid: super::super::new_guid().unwrap(),
        added_at_ms: Some(NOW - 1_000),
    }
}

fn folder(title: &str, children: Vec<BookmarkImport>) -> BookmarkImport {
    BookmarkImport::Folder {
        title: title.to_owned(),
        guid: super::super::new_guid().unwrap(),
        added_at_ms: None,
        children,
    }
}

fn separator() -> BookmarkImport {
    BookmarkImport::Separator {
        guid: super::super::new_guid().unwrap(),
    }
}

fn source_tree() -> Vec<BookmarkImport> {
    vec![
        folder(
            "Bookmarks Toolbar",
            vec![
                bookmark("News", "https://news.example/"),
                separator(),
                folder("Reading", vec![bookmark("Docs", "https://docs.example/")]),
            ],
        ),
        folder(
            "Other Bookmarks",
            vec![bookmark("Mail", "https://mail.example/")],
        ),
    ]
}

/// The tree under `folder` as (depth, title or URL) lines.
fn outline(store: &Store, folder: mercury_store::BookmarkId) -> Vec<String> {
    let mut lines = Vec::new();
    let mut stack = vec![(0, store.bookmark_children(folder).unwrap().into_iter())];
    while let Some((depth, nodes)) = stack.last_mut() {
        let depth = *depth;
        let Some(node) = nodes.next() else {
            stack.pop();
            continue;
        };
        let indent = "  ".repeat(depth);
        match node.kind {
            BookmarkKind::Folder => {
                lines.push(format!("{indent}{}/", node.title));
                stack.push((
                    depth + 1,
                    store.bookmark_children(node.id).unwrap().into_iter(),
                ));
            }
            BookmarkKind::Bookmark { url } => lines.push(format!("{indent}{} {url}", node.title)),
            BookmarkKind::Separator => lines.push(format!("{indent}---")),
        }
    }
    lines
}

#[test]
fn the_first_import_creates_its_folder_under_other_bookmarks_and_imports_everything() {
    let harness = Harness::new();

    let plan = harness.plan(source_tree()).unwrap();
    assert!(plan.created_folder);
    assert_eq!(plan.already_present, 0);
    let folder = plan.folder;
    let summary = harness.import_plan(plan);
    assert!(matches!(summary.outcome, ImportOutcome::Completed));
    assert_eq!(summary.imported, 7);

    let (store, _directory) = harness.reopen();
    let other = store.bookmark_root(BookmarkRoot::Other).unwrap();
    let roots = store.bookmark_children(other).unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].title, "Imported from Firefox");
    assert_eq!(roots[0].id, folder);
    assert_eq!(
        outline(&store, folder),
        vec![
            "Bookmarks Toolbar/",
            "  News https://news.example/",
            "  ---",
            "  Reading/",
            "    Docs https://docs.example/",
            "Other Bookmarks/",
            "  Mail https://mail.example/",
        ]
    );
}

#[test]
fn importing_the_same_tree_again_adds_nothing_and_a_changed_tree_adds_only_what_is_new() {
    let harness = Harness::new();
    let first = harness.plan(source_tree()).unwrap();
    harness.import_plan(first);

    let again = harness.plan(source_tree()).unwrap();
    assert!(!again.created_folder);
    assert!(again.sets.is_empty(), "nothing to add: {:?}", again.sets);
    assert_eq!(again.already_present, 7);

    let mut changed = source_tree();
    if let BookmarkImport::Folder { children, .. } = &mut changed[0] {
        children.push(bookmark("Weather", "https://weather.example/"));
        if let BookmarkImport::Folder { children, .. } = &mut children[2] {
            children.push(bookmark("Docs, renamed", "https://docs.example/"));
        }
    }
    changed.push(folder(
        "Mobile Bookmarks",
        vec![bookmark("Phone", "https://phone.example/")],
    ));
    let plan = harness.plan(changed).unwrap();
    let folder = plan.folder;
    assert_eq!(plan.already_present, 7);
    let summary = harness.import_plan(plan);
    assert_eq!(summary.imported, 4);

    let (store, _directory) = harness.reopen();
    assert_eq!(
        outline(&store, folder),
        vec![
            "Bookmarks Toolbar/",
            "  News https://news.example/",
            "  ---",
            "  Reading/",
            "    Docs https://docs.example/",
            "    Docs, renamed https://docs.example/",
            "  Weather https://weather.example/",
            "Other Bookmarks/",
            "  Mail https://mail.example/",
            "Mobile Bookmarks/",
            "  Phone https://phone.example/",
        ]
    );
}

#[test]
fn an_interrupted_bookmark_import_run_again_adds_the_trees_it_had_not_reached() {
    let harness = Harness::new();
    let first = harness.plan(source_tree()).unwrap();
    // The earlier run committed only the first top-level tree.
    let mut sets = first.sets;
    if let Some((_, nodes)) = sets.first_mut() {
        nodes.truncate(1);
    }
    harness.import_plan(BookmarkPlan { sets, ..first });

    let again = harness.plan(source_tree()).unwrap();
    assert_eq!(again.already_present, 5);
    let folder = again.folder;
    harness.import_plan(again);

    let (store, _directory) = harness.reopen();
    assert_eq!(
        outline(&store, folder).len(),
        7,
        "{:?}",
        outline(&store, folder)
    );
}

#[test]
fn an_empty_tree_creates_the_folder_and_plans_nothing() {
    let harness = Harness::new();
    let plan = harness.plan(Vec::new()).unwrap();
    assert!(plan.created_folder);
    assert!(plan.sets.is_empty());
}
