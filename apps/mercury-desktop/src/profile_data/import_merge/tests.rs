// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use mercury_store::{BookmarkImport, BookmarkKind, BookmarkRoot, Store, VisitTransition};

use super::super::import::{ImportOutcome, ImportSummary, ImportVisit};
use super::super::{ProfileData, ProfileDataError, Timing, open_database};
use super::{
    BOOKMARK_STEP, BookmarkPlan, FILTER_STEP, FilterProgress, FilterSummary, InsertProgress,
    InsertSummary, StepOutcome,
};

const NOW: i64 = 1_800_000_000_000;
const PATIENCE: Duration = Duration::from_secs(30);
/// The service has to keep answering, not to finish by a fixed time: a wait fails when no
/// wake-up comes for `QUIET` (a stuck service) or when `LIMIT` has passed in all (a service
/// that wakes for ever and never finishes). Under load a long import is slow but keeps waking.
const QUIET: Duration = Duration::from_secs(30);
const LIMIT: Duration = Duration::from_secs(600);

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
            None,
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

    /// Delivers replies until `done` holds. Every delivery, progress included, wakes the UI
    /// and so restarts the quiet period; `LIMIT` bounds the wait in all.
    fn settle(&self, done: impl Fn() -> bool) {
        let limit = Instant::now() + LIMIT;
        while !done() {
            let left = limit.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "the service did not finish within {LIMIT:?}"
            );
            self.wakes
                .recv_timeout(left.min(QUIET))
                .expect("the service should answer: no wake-up within the quiet period");
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

    fn import_plan(&self, plan: BookmarkPlan) -> InsertSummary {
        self.insert(plan).0
    }

    fn insert(&self, plan: BookmarkPlan) -> (InsertSummary, Vec<InsertProgress>) {
        let progress = Rc::new(RefCell::new(Vec::new()));
        let summary = Rc::new(RefCell::new(None));
        let (seen, done) = (Rc::clone(&progress), Rc::clone(&summary));
        self.data.insert_bookmarks(
            plan.sets,
            move |step| seen.borrow_mut().push(step),
            move |result| *done.borrow_mut() = Some(result),
        );
        self.settle(|| summary.borrow().is_some());
        let summary = summary.borrow_mut().take().unwrap();
        (summary, progress.take())
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

/// Every step did at most `FILTER_STEP` units of work: compared source visits are counted by
/// the progress; stored rows read are bounded by the same budget in the code.
fn assert_bounded_steps(progress: &[FilterProgress]) {
    let mut before = 0;
    for step in progress {
        assert!(step.checked - before <= FILTER_STEP, "{progress:?}");
        before = step.checked;
    }
}

#[test]
fn a_new_profile_keeps_every_visit_and_reports_progress_per_step() {
    let harness = Harness::new();
    let visits = history(FILTER_STEP as usize * 2 + 10, 300);

    let (summary, progress) = harness.filter(visits.clone());

    assert!(matches!(summary.outcome, StepOutcome::Completed));
    assert_eq!(summary.already_present, 0);
    assert_eq!(summary.new, visits, "kept newest first");
    let total = visits.len() as u64;
    assert_eq!(
        progress,
        vec![
            FilterProgress {
                checked: FILTER_STEP,
                total
            },
            FilterProgress {
                checked: 2 * FILTER_STEP,
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
fn visits_in_one_millisecond_are_matched_one_for_one_across_steps() {
    let harness = Harness::new();
    // Two visits to one page in one millisecond already stored, three in the source, placed
    // across the boundary of the first step.
    // The first step reads the two stored rows and compares 3,998 source visits, the first
    // of the three in that millisecond among them.
    let mut visits = history(FILTER_STEP as usize - 3, 50);
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
        progress.first().map(|step| step.checked),
        Some(FILTER_STEP - 2),
        "the millisecond is split across steps"
    );
    assert_bounded_steps(&progress);
}

#[test]
fn same_millisecond_flood_stays_bounded() {
    let harness = Harness::new();
    // 12,000 source visits and 3,000 stored ones, all in one millisecond.
    let at = NOW - 1_000;
    let flood = |count: usize| {
        (0..count)
            .map(|index| visit(&format!("https://flood{}.example/", index % 3), at))
            .collect::<Vec<_>>()
    };
    harness.import_visits(flood(3_000));

    let (summary, progress) = harness.filter(flood(12_000));

    assert!(matches!(summary.outcome, StepOutcome::Completed));
    assert_eq!(summary.already_present, 3_000);
    assert_eq!(summary.new.len(), 9_000);
    assert!(progress.len() >= 3, "{} steps", progress.len() + 1);
    assert_bounded_steps(&progress);

    // And the other way round: one source visit against 20,000 stored in its millisecond takes
    // several steps of reading.
    harness.import_visits(flood(17_000));
    let progress_calls = Rc::new(RefCell::new(0));
    let summary = Rc::new(RefCell::new(None));
    let (count, done) = (Rc::clone(&progress_calls), Rc::clone(&summary));
    harness.data.filter_new_visits(
        vec![
            visit("https://flood0.example/", at),
            visit("https://earlier.example/", at - 5),
        ],
        move |_| *count.borrow_mut() += 1,
        move |result| *done.borrow_mut() = Some(result),
    );
    harness.settle(|| summary.borrow().is_some());
    let summary = summary.borrow_mut().take().unwrap();
    assert_eq!(summary.already_present, 1);
    assert_eq!(summary.rows_read, 20_000);
    assert!(
        *progress_calls.borrow() >= 4,
        "{} steps",
        progress_calls.borrow()
    );
}

#[test]
fn a_sparse_source_over_a_dense_history_reads_little_of_it() {
    let harness = Harness::new();
    // 50,000 stored visits a second apart; 100 source visits 500 seconds apart, every other one
    // already stored.
    let stored = (0..50_000)
        .map(|index| {
            visit(
                &format!("https://dense{}.example/", index % 1000),
                NOW - 1_000 * index,
            )
        })
        .collect::<Vec<_>>();
    harness.import_visits(stored.clone());
    let source = (0..100)
        .map(|index| {
            let original = &stored[index * 500];
            if index % 2 == 0 {
                original.clone()
            } else {
                visit("https://elsewhere.example/", original.visited_at_ms)
            }
        })
        .collect::<Vec<_>>();

    let (summary, _) = harness.filter(source);

    assert_eq!(summary.already_present, 50);
    assert_eq!(summary.new.len(), 50);
    assert!(
        summary.rows_read < 5_000,
        "only the rows near each source time are read: {}",
        summary.rows_read
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
        history(FILTER_STEP as usize * 20, 100),
        |_| {},
        move |result| *done.borrow_mut() = Some(result),
    );
    job.cancel();
    harness.settle(|| summary.borrow().is_some());

    let summary = summary.borrow_mut().take().unwrap();
    assert!(matches!(summary.outcome, StepOutcome::Cancelled));
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
    assert!(matches!(summary.outcome, StepOutcome::Completed));
    assert_eq!((summary.nodes, summary.bookmarks), (7, 3));

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
    assert_eq!(summary.nodes, 4);

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

#[test]
fn a_separator_added_in_the_source_since_the_last_import_is_imported() {
    let harness = Harness::new();
    let first = vec![folder(
        "Bookmarks Toolbar",
        vec![bookmark("A", "https://a.example/"), separator()],
    )];
    let plan = harness.plan(first).unwrap();
    harness.import_plan(plan);

    let again = vec![folder(
        "Bookmarks Toolbar",
        vec![
            bookmark("A", "https://a.example/"),
            separator(),
            bookmark("B", "https://b.example/"),
            separator(),
        ],
    )];
    let plan = harness.plan(again).unwrap();
    assert_eq!(
        plan.already_present, 3,
        "the folder, A and the first separator"
    );
    let folder_id = plan.folder;
    harness.import_plan(plan);

    let (store, _directory) = harness.reopen();
    assert_eq!(
        outline(&store, folder_id),
        vec![
            "Bookmarks Toolbar/",
            "  A https://a.example/",
            "  ---",
            "  B https://b.example/",
            "  ---",
        ]
    );
}

/// A toolbar of `count` bookmarks the way people keep them: most in folders, one folder far
/// larger than a step, some loose.
fn big_toolbar(count: usize) -> Vec<BookmarkImport> {
    let mut bookmarks = (0..count)
        .map(|index| bookmark(&format!("B{index}"), &format!("https://b{index}.example/")));
    let loose = bookmarks.by_ref().take(count / 10).collect::<Vec<_>>();
    let small = folder("Small", bookmarks.by_ref().take(count / 10).collect());
    let huge = folder(
        "Huge",
        vec![
            folder("Inner", bookmarks.by_ref().take(count / 2).collect()),
            separator(),
        ]
        .into_iter()
        .chain(bookmarks)
        .collect(),
    );
    let mut children = loose;
    children.insert(5, huge);
    children.push(small);
    vec![folder("Bookmarks Toolbar", children)]
}

#[test]
fn a_big_tree_is_written_a_bounded_step_at_a_time_in_order() {
    let harness = Harness::new();
    let tree = big_toolbar(3_000);

    let plan = harness.plan(tree).unwrap();
    let folder_id = plan.folder;
    let (summary, progress) = harness.insert(plan);

    assert!(matches!(summary.outcome, StepOutcome::Completed));
    assert_eq!(summary.bookmarks, 3_000);
    assert_eq!(summary.nodes, 3_000 + 4 + 1);
    let mut before = 0;
    for step in &progress {
        assert!(step.nodes - before <= BOOKMARK_STEP as u64, "{progress:?}");
        before = step.nodes;
    }
    assert!(progress.len() >= 6, "{} steps", progress.len() + 1);

    let (store, _directory) = harness.reopen();
    let lines = outline(&store, folder_id);
    assert_eq!(lines.len(), 3_005);
    assert_eq!(
        lines[..8],
        [
            "Bookmarks Toolbar/",
            "  B0 https://b0.example/",
            "  B1 https://b1.example/",
            "  B2 https://b2.example/",
            "  B3 https://b3.example/",
            "  B4 https://b4.example/",
            "  Huge/",
            "    Inner/",
        ]
    );
    assert_eq!(lines[8], "      B600 https://b600.example/");
    assert_eq!(lines.last().unwrap(), "    B599 https://b599.example/");
    assert_eq!(lines[lines.len() - 301], "  Small/");
}

/// How long one chunk of the comparison holds the service, on 100,000 visits over 20,000 pages
/// against a profile that already holds them all (the most stored rows a chunk reads) and
/// against an empty one. Run with
/// `cargo test --release -p mercury-desktop filter_benchmark -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn filter_benchmark_100k_visits() {
    let harness = Harness::new();
    let visits = history(100_000, 20_000);

    let started = Instant::now();
    let (fresh, _) = harness.filter(visits.clone());
    println!(
        "empty profile: {:?} in all, longest step {:?}",
        started.elapsed(),
        fresh.longest_chunk
    );
    let imported = harness.import_visits(fresh.new);
    println!("import: longest chunk {:?}", imported.longest_chunk);

    let started = Instant::now();
    let (again, _) = harness.filter(visits);
    assert_eq!(again.already_present, 100_000);
    println!(
        "everything present: {:?} in all, longest step {:?}, {} rows read",
        started.elapsed(),
        again.longest_chunk,
        again.rows_read
    );
}

/// How long one step of a bookmark import holds the service: a 5,000-bookmark toolbar, first
/// imported, then planned again (the merge reads the whole existing tree in one request).
/// `cargo test --release -p mercury-desktop bookmark_benchmark -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn bookmark_benchmark_5000_toolbar() {
    let harness = Harness::new();
    let started = Instant::now();
    let plan = harness.plan(big_toolbar(5_000)).unwrap();
    println!("first plan: {:?}", started.elapsed());
    let started = Instant::now();
    let (summary, progress) = harness.insert(plan);
    println!(
        "insert: {:?} in all, {} steps, longest step {:?}",
        started.elapsed(),
        progress.len() + 1,
        summary.longest_chunk
    );
    let started = Instant::now();
    let again = harness.plan(big_toolbar(5_000)).unwrap();
    println!(
        "second plan (merge against 5,000): {:?}, {} already present",
        started.elapsed(),
        again.already_present
    );
}
