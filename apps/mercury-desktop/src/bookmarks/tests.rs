// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The bookmark operations the star, the bar and the manager use, against a database file
//! through the profile-data service, and the bookmarks-file round trip.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use mercury_import::{ImportedNode, parse_netscape_bookmarks, write_netscape_bookmarks};
use mercury_store::{BookmarkId, BookmarkKind, BookmarkRoot, Store, StoreError};

use super::file::{FileImportResult, import_into_new_folder};
use super::tree;
use crate::profile_data::bookmarks::BookmarkEdit;
use crate::profile_data::import::ImportOutcome;
use crate::profile_data::import::{IMPORT_CHUNK, ImportProgress};
use crate::profile_data::{ProfileData, ProfileDataError, Timing};
use mercury_store::BookmarkImport;

const NOW: i64 = 1_800_000_000_000;

struct Service {
    data: ProfileData,
    wakes: Receiver<()>,
    _directory: tempfile::TempDir,
}

impl Service {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("places.sqlite");
        let (wake, wakes) = mpsc::channel();
        let data = ProfileData::spawn(
            move || Store::open(&path).map_err(|error| error.to_string()),
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
            _directory: directory,
        }
    }

    fn settle(&self, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done() {
            self.wakes
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("the service should answer");
            self.data.deliver_replies();
        }
    }

    fn ask<T: 'static>(
        &self,
        request: impl FnOnce(&ProfileData, Box<dyn FnOnce(Result<T, ProfileDataError>)>),
    ) -> Result<T, ProfileDataError> {
        let slot = Rc::new(RefCell::new(None));
        let fill = Rc::clone(&slot);
        request(
            &self.data,
            Box::new(move |result| *fill.borrow_mut() = Some(result)),
        );
        self.settle(|| slot.borrow().is_some());
        slot.borrow_mut().take().unwrap()
    }

    fn root(&self, root: BookmarkRoot) -> BookmarkId {
        self.ask(|data, reply| data.bookmark_root(root, reply))
            .unwrap()
    }

    fn folder(&self, parent: BookmarkId, title: &str) -> BookmarkId {
        let title = title.to_owned();
        self.ask(|data, reply| data.add_bookmark_folder(parent, None, title, reply))
            .unwrap()
    }

    fn bookmark(&self, parent: BookmarkId, title: &str, url: &str) -> BookmarkId {
        let (title, url) = (title.to_owned(), url.to_owned());
        self.ask(|data, reply| data.add_bookmark(parent, None, title, url, reply))
            .unwrap()
    }

    fn titles(&self, folder: BookmarkId) -> Vec<String> {
        self.ask(|data, reply| data.bookmark_children(folder, reply))
            .unwrap()
            .into_iter()
            .map(|node| match node.kind {
                BookmarkKind::Separator => "---".to_owned(),
                _ => node.title,
            })
            .collect()
    }
}

#[test]
fn the_star_adds_to_other_bookmarks_once() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    let ask = |title: &str| {
        let title = title.to_owned();
        service.ask(|data, reply| {
            data.find_or_add_bookmark(
                BookmarkRoot::Other,
                title,
                "https://a.test/".to_owned(),
                reply,
            )
        })
    };
    let (id, added) = ask("A page").unwrap();
    assert!(added);
    let (again, added_again) = ask("Another title").unwrap();
    assert_eq!((again, added_again), (id, false));
    assert_eq!(service.titles(other), vec!["A page"]);
    let found = service
        .ask(|data, reply| data.bookmark_for_url("https://a.test/".to_owned(), reply))
        .unwrap();
    assert_eq!(found, Some(id));
}

#[test]
fn the_star_refuses_what_the_store_refuses() {
    let service = Service::new();
    let long = format!(
        "https://a.test/{}",
        "x".repeat(mercury_store::MAX_URL_BYTES)
    );
    let result = service.ask(|data, reply| {
        data.find_or_add_bookmark(BookmarkRoot::Other, "Long".to_owned(), long, reply)
    });
    assert!(matches!(
        result,
        Err(ProfileDataError::Store(StoreError::InvalidArgument(_)))
    ));
}

#[test]
fn the_folder_list_holds_both_roots_in_tree_order() {
    let service = Service::new();
    let bar = service.root(BookmarkRoot::Toolbar);
    let other = service.root(BookmarkRoot::Other);
    let news = service.folder(bar, "News");
    service.bookmark(news, "A", "https://a.test/");
    service.folder(news, "Deep");
    service.folder(other, "Later");
    let folders = service
        .ask(|data, reply| data.bookmark_folders(reply))
        .unwrap();
    let rows = tree::folder_rows(&folders);
    assert_eq!(
        tree::chooser_labels(&rows),
        vec![
            "Bookmarks Bar",
            "\u{2003}News",
            "\u{2003}\u{2003}Deep",
            "Other Bookmarks",
            "\u{2003}Later",
        ]
    );
}

#[test]
fn a_deleted_folder_comes_back_where_it_was_with_its_contents() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    service.bookmark(other, "First", "https://1.test/");
    let folder = service.folder(other, "Folder");
    let inner = service.bookmark(folder, "Inner", "https://inner.test/");
    service.bookmark(other, "Last", "https://3.test/");
    let guid = |id| {
        service
            .ask(|data, reply| data.bookmark(id, reply))
            .unwrap()
            .guid
    };
    let (folder_guid, inner_guid) = (guid(folder), guid(inner));

    let removed = service
        .ask(|data, reply| data.remove_bookmark_keeping_undo(folder, reply))
        .unwrap();
    assert_eq!((removed.parent, removed.position), (other, 1));
    assert_eq!(removed.title, "Folder");
    assert_eq!(service.titles(other), vec!["First", "Last"]);
    let gone =
        service.ask(|data, reply| data.bookmark_for_url("https://inner.test/".into(), reply));
    assert_eq!(gone.unwrap(), None);

    let restored = service
        .ask(|data, reply| data.restore_bookmark(removed, reply))
        .unwrap();
    assert!(restored.in_place);
    let restored = restored.id;
    assert_eq!(service.titles(other), vec!["First", "Folder", "Last"]);
    assert_eq!(guid(restored), folder_guid);
    assert_eq!(service.titles(restored), vec!["Inner"]);
    let inner_again = service
        .ask(|data, reply| data.bookmark_for_url("https://inner.test/".into(), reply))
        .unwrap()
        .unwrap();
    assert_eq!(guid(inner_again), inner_guid);
}

#[test]
fn a_root_cannot_be_deleted() {
    let service = Service::new();
    let bar = service.root(BookmarkRoot::Toolbar);
    let result = service.ask(|data, reply| data.remove_bookmark_keeping_undo(bar, reply));
    assert!(matches!(
        result,
        Err(ProfileDataError::Store(StoreError::CannotModifyRoot))
    ));
}

#[test]
fn moving_up_down_and_into_folders() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    let a = service.bookmark(other, "A", "https://a.test/");
    let folder = service.folder(other, "F");
    let inner = service.folder(folder, "Inner");
    let mv = |id, parent, position| {
        service.ask(|data, reply| data.move_bookmark(id, parent, position, reply))
    };
    // Down: from 0 to 1; up again.
    mv(a, other, Some(1)).unwrap();
    assert_eq!(service.titles(other), vec!["F", "A"]);
    mv(a, other, Some(0)).unwrap();
    assert_eq!(service.titles(other), vec!["A", "F"]);
    mv(a, folder, None).unwrap();
    assert_eq!(service.titles(folder), vec!["Inner", "A"]);
    let cycle = mv(folder, inner, None);
    assert!(matches!(
        cycle,
        Err(ProfileDataError::Store(StoreError::InvalidMove))
    ));
}

#[test]
fn renaming_new_separators_and_url_edits() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    let a = service.bookmark(other, "A", "https://a.test/");
    service
        .ask(|data, reply| data.rename_bookmark(a, "Renamed".into(), reply))
        .unwrap();
    service
        .ask(|data, reply| data.add_bookmark_separator(other, Some(0), reply))
        .unwrap();
    assert_eq!(service.titles(other), vec!["---", "Renamed"]);
    service
        .ask(|data, reply| data.set_bookmark_url(a, "https://b.test/".into(), reply))
        .unwrap();
    let node = service.ask(|data, reply| data.bookmark(a, reply)).unwrap();
    assert_eq!(
        node.kind,
        BookmarkKind::Bookmark {
            url: "https://b.test/".into()
        }
    );
}

#[test]
fn search_finds_titles_and_urls_across_both_roots() {
    let service = Service::new();
    let bar = service.root(BookmarkRoot::Toolbar);
    let other = service.root(BookmarkRoot::Other);
    service.bookmark(bar, "Rust book", "https://doc.rust-lang.org/book/");
    let folder = service.folder(other, "Reading");
    service.bookmark(folder, "Crates", "https://crates.io/rust");
    service.bookmark(folder, "Python", "https://python.org/");
    let search = |query: &str| {
        let query = query.to_owned();
        service
            .ask(|data, reply| data.search_bookmarks(query, reply))
            .unwrap()
            .into_iter()
            .map(|node| node.title)
            .collect::<Vec<_>>()
    };
    assert_eq!(search("RUST"), vec!["Rust book", "Crates"]);
    assert_eq!(search("reading"), vec!["Reading"]);
    assert_eq!(search("rust book"), vec!["Rust book"]);
    assert!(search("nothing").is_empty());
    assert!(search("").is_empty());
}

#[test]
fn ten_thousand_bookmarks_are_listed_and_searched_on_the_service_thread() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    let mut guid = 0;
    let mut make_guid = || {
        guid += 1;
        Some(format!("g{guid:011}"))
    };
    let nodes = (0..100)
        .map(|folder| ImportedNode::Folder {
            title: format!("Folder {folder}"),
            toolbar: false,
            added_at_ms: None,
            children: (0..100)
                .map(|page| ImportedNode::Bookmark {
                    title: format!("Page {folder}-{page}"),
                    url: format!("https://site{folder}.test/{page}"),
                    added_at_ms: None,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let imports = tree::bookmark_imports(&nodes, &mut make_guid).unwrap();
    let done = Rc::new(RefCell::new(None));
    let fill = Rc::clone(&done);
    let _job = service.data.import_bookmarks(
        vec![(other, imports)],
        |_| {},
        move |summary| *fill.borrow_mut() = Some(summary),
    );
    service.settle(|| done.borrow().is_some());
    let summary = done.borrow_mut().take().unwrap();
    assert!(matches!(summary.outcome, ImportOutcome::Completed));
    assert_eq!(summary.imported, 10_100);

    let started = Instant::now();
    let folders = service
        .ask(|data, reply| data.bookmark_folders(reply))
        .unwrap();
    assert_eq!(folders.len(), 102);
    let results = service
        .ask(|data, reply| data.search_bookmarks("page 42-".into(), reply))
        .unwrap();
    assert_eq!(results.len(), 100);
    let everything = service
        .ask(|data, reply| data.search_bookmarks("page".into(), reply))
        .unwrap();
    assert_eq!(
        everything.len(),
        crate::profile_data::bookmarks::MAX_SEARCH_RESULTS
    );
    println!(
        "folder list and two searches of 10,100 nodes: {:?}",
        started.elapsed()
    );
}

#[test]
fn an_exported_tree_imports_back_into_a_new_folder_unchanged() {
    let service = Service::new();
    let bar = service.root(BookmarkRoot::Toolbar);
    let other = service.root(BookmarkRoot::Other);
    let news = service.folder(bar, "News & views");
    service.bookmark(news, "A <page>", "https://a.test/path?q=1&r=2");
    service
        .ask(|data, reply| data.add_bookmark_separator(news, None, reply))
        .unwrap();
    service.bookmark(bar, "Ünïcode", "https://b.test/");
    service.bookmark(other, "Elsewhere", "file:///Users/someone/page.html");

    let (toolbar, rest) = service
        .ask(|data, reply| data.bookmark_trees(reply))
        .unwrap();
    let exported = tree::export_nodes(&toolbar, &rest);
    let html = write_netscape_bookmarks(&exported);
    let parsed = parse_netscape_bookmarks(&html).unwrap();
    // Dates are whole seconds in the file; the clock here is a whole second.
    assert_eq!(parsed, exported);

    let target = service.folder(other, "Imported 2026-11-16");
    let mut guid = || crate::profile_data::new_guid();
    let imports = tree::bookmark_imports(&parsed, &mut guid).unwrap();
    let done = Rc::new(RefCell::new(None));
    let fill = Rc::clone(&done);
    let _job = service.data.import_bookmarks(
        vec![(target, imports)],
        |_| {},
        move |summary| *fill.borrow_mut() = Some(summary),
    );
    service.settle(|| done.borrow().is_some());
    let summary = done.borrow_mut().take().unwrap();
    assert_eq!((summary.imported, summary.total), (6, 6));

    let (_, walked) = service
        .ask(|data, reply| data.bookmark_trees(reply))
        .unwrap();
    let imported_folder = walked
        .iter()
        .position(|entry| entry.node.id == target)
        .unwrap();
    let depth = walked[imported_folder].depth;
    let end = walked[imported_folder + 1..]
        .iter()
        .position(|entry| entry.depth <= depth)
        .map_or(walked.len(), |offset| imported_folder + 1 + offset);
    let again = tree::imported_nodes(
        &walked[imported_folder..end]
            .iter()
            .map(|entry| crate::profile_data::bookmarks::Walked {
                depth: entry.depth - depth,
                node: entry.node.clone(),
            })
            .collect::<Vec<_>>(),
    );
    // The toolbar folder comes back as an ordinary folder inside the new one.
    let mut expected = exported;
    if let ImportedNode::Folder { toolbar, .. } = &mut expected[0] {
        *toolbar = false;
    }
    let strip_dates = |nodes: &mut Vec<ImportedNode>| {
        fn strip(nodes: &mut [ImportedNode]) {
            for node in nodes {
                match node {
                    ImportedNode::Folder {
                        added_at_ms,
                        children,
                        ..
                    } => {
                        *added_at_ms = None;
                        strip(children);
                    }
                    ImportedNode::Bookmark { added_at_ms, .. } => *added_at_ms = None,
                    ImportedNode::Separator => {}
                }
            }
        }
        strip(nodes);
    };
    let mut again = again;
    strip_dates(&mut again);
    strip_dates(&mut expected);
    assert_eq!(again, expected);
}

#[test]
fn the_lanes_requests_name_the_folders_they_change() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    let folder = service.folder(other, "F");
    let seen = Rc::new(RefCell::new(Vec::new()));
    let record = Rc::clone(&seen);
    let _listener = service
        .data
        .on_change(crate::profile_data::Area::Bookmarks, move |change| {
            record.borrow_mut().push(change.folders.clone());
        });
    // A notification may come in a delivery after its write's reply: a read flushes those of
    // the setup, which are not part of the test.
    service.root(BookmarkRoot::Toolbar);
    seen.borrow_mut().clear();
    let take = || std::mem::take(&mut *seen.borrow_mut());
    let star = |title: &str| {
        let title = title.to_owned();
        service
            .ask(|data, reply| {
                data.find_or_add_bookmark(
                    BookmarkRoot::Other,
                    title,
                    "https://a.test/".to_owned(),
                    reply,
                )
            })
            .unwrap()
    };
    let (a, _) = star("A");
    service.settle(|| !seen.borrow().is_empty());
    assert_eq!(take(), vec![Some(vec![other])]);
    star("A again");
    service.settle(|| !seen.borrow().is_empty());
    assert_eq!(take(), vec![Some(Vec::new())], "found, nothing written");

    let inner = service.bookmark(folder, "Inner", "https://inner.test/");
    service.settle(|| !seen.borrow().is_empty());
    take();
    let removed = service
        .ask(|data, reply| data.remove_bookmark_keeping_undo(inner, reply))
        .unwrap();
    service.settle(|| !seen.borrow().is_empty());
    assert_eq!(take(), vec![Some(vec![folder])]);
    service
        .ask(|data, reply| data.restore_bookmark(removed, reply))
        .unwrap();
    service.settle(|| !seen.borrow().is_empty());
    assert_eq!(take(), vec![Some(vec![folder])]);
    let _ = a;
}

/// Fills a scratch profile's database for the end-to-end check (run with `--ignored`):
/// `MERCURY_FILL_PLACES=<profile>/places.sqlite MERCURY_FILL_URL=<start URL>`. The bar gets a
/// bookmark of the start URL, a folder, and enough long titles to need "»".
#[test]
#[ignore = "a tool for the end-to-end check, not a test"]
fn fill_a_scratch_profile() {
    let path = std::env::var_os("MERCURY_FILL_PLACES").expect("MERCURY_FILL_PLACES");
    let url = std::env::var("MERCURY_FILL_URL").expect("MERCURY_FILL_URL");
    let mut store = Store::open(std::path::Path::new(&path)).unwrap();
    let bar = store.bookmark_root(BookmarkRoot::Toolbar).unwrap();
    let mut serial = 0;
    let mut guid = || {
        serial += 1;
        format!("e2e{serial:09}")
    };
    store
        .add_bookmark(bar, None, "Colours test page", &url, &guid(), NOW)
        .unwrap();
    let news = store
        .add_bookmark_folder(bar, None, "News", &guid(), NOW)
        .unwrap();
    store
        .add_bookmark(news, None, "Example", "https://example.com/", &guid(), NOW)
        .unwrap();
    for title in [
        "Rust documentation",
        "A page with a rather long title that is cut short",
        "Weather forecast for the week",
        "Recipes",
        "Maps",
        "Another long title to push the bar past its width",
        "Music",
        "Photos",
    ] {
        let url = format!("https://{}.example/", title.len());
        store
            .add_bookmark(bar, None, title, &url, &guid(), NOW)
            .unwrap();
    }
}

fn mark(title: &str) -> BookmarkImport {
    BookmarkImport::Bookmark {
        title: title.to_owned(),
        url: format!("https://{}.test/", title.replace(' ', "-")),
        guid: crate::profile_data::new_guid().unwrap(),
        added_at_ms: None,
    }
}

fn import_folder(title: &str, children: Vec<BookmarkImport>) -> BookmarkImport {
    BookmarkImport::Folder {
        title: title.to_owned(),
        guid: crate::profile_data::new_guid().unwrap(),
        added_at_ms: None,
        children,
    }
}

fn marks(prefix: &str, count: usize) -> Vec<BookmarkImport> {
    (0..count).map(|n| mark(&format!("{prefix} {n}"))).collect()
}

/// Runs a file import to its end; returns the result and every progress report.
fn run_file_import(
    service: &Service,
    nodes: Vec<BookmarkImport>,
) -> (FileImportResult, Vec<ImportProgress>) {
    let progress = Rc::new(RefCell::new(Vec::new()));
    let done = Rc::new(RefCell::new(None));
    let (record, fill) = (Rc::clone(&progress), Rc::clone(&done));
    import_into_new_folder(
        &service.data,
        "Imported 2026-11-16".to_owned(),
        nodes,
        move |report| record.borrow_mut().push(report),
        move |result| *fill.borrow_mut() = Some(result),
    );
    service.settle(|| done.borrow().is_some());
    let result = done.borrow_mut().take().unwrap().unwrap();
    let progress = progress.borrow().clone();
    (result, progress)
}

/// No chunk stored more than `IMPORT_CHUNK` nodes.
fn assert_chunks_bounded(progress: &[ImportProgress], total: u64) {
    let mut before = 0;
    for report in progress.iter().map(|report| report.done).chain([total]) {
        assert!(
            report - before <= IMPORT_CHUNK as u64,
            "{before} -> {report}"
        );
        before = report;
    }
}

#[test]
fn a_file_import_keeps_the_files_order_and_splits_large_folders() {
    let service = Service::new();
    let nodes = vec![
        mark("a"),
        import_folder("Big", marks("big", 1_200)),
        mark("b"),
        import_folder("Small", vec![mark("c"), mark("d")]),
        import_folder(
            "Outer",
            vec![
                mark("e"),
                import_folder("Inner", marks("inner", 700)),
                mark("f"),
            ],
        ),
    ];
    let (result, progress) = run_file_import(&service, nodes);
    let folder = result.folder.expect("something was imported");
    assert_eq!(result.summary.imported, 1_910);
    assert_eq!(result.summary.total, 1_910);
    assert_eq!(
        service.titles(folder),
        vec!["a", "Big", "b", "Small", "Outer"]
    );
    let children = |title: &str, parent| {
        service
            .ask(|data, reply| data.bookmark_children(parent, reply))
            .unwrap()
            .into_iter()
            .find(|node| node.title == title)
            .unwrap()
            .id
    };
    let big = children("Big", folder);
    assert_eq!(service.titles(big).len(), 1_200);
    assert_eq!(service.titles(big)[0], "big 0");
    let outer = children("Outer", folder);
    assert_eq!(service.titles(outer), vec!["e", "Inner", "f"]);
    assert_eq!(service.titles(children("Inner", outer)).len(), 700);
    assert_chunks_bounded(&progress, 1_910 - 3);
}

#[test]
fn one_folder_of_two_thousand_bookmarks_is_imported_in_chunks() {
    let service = Service::new();
    let (result, progress) =
        run_file_import(&service, vec![import_folder("All", marks("m", 2_000))]);
    assert_eq!(result.summary.imported, 2_001);
    assert!(progress.len() >= 3, "{} progress reports", progress.len());
    assert_chunks_bounded(&progress, 2_000);
    println!(
        "one folder of 2,000: longest chunk {:?}",
        result.summary.longest_chunk
    );
}

#[test]
fn a_ten_thousand_entry_file_is_imported_in_bounded_chunks() {
    let service = Service::new();
    let nodes = vec![import_folder(
        "Bookmarks bar",
        (0..10)
            .map(|n| import_folder(&format!("Folder {n}"), marks(&format!("f{n}"), 999)))
            .collect(),
    )];
    let (result, progress) = run_file_import(&service, nodes);
    assert_eq!(result.summary.imported, 10_001);
    assert_chunks_bounded(&progress, 10_000 - 10);
    println!(
        "10,001 entries: longest chunk {:?}, {} progress reports",
        result.summary.longest_chunk,
        progress.len()
    );
}

#[test]
fn an_import_that_stores_nothing_removes_its_new_folder() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    let too_long = BookmarkImport::Bookmark {
        title: "Long".into(),
        url: format!(
            "https://a.test/{}",
            "x".repeat(mercury_store::MAX_URL_BYTES)
        ),
        guid: crate::profile_data::new_guid().unwrap(),
        added_at_ms: None,
    };
    let (result, _) = run_file_import(&service, vec![too_long]);
    assert_eq!(result.folder, None);
    assert_eq!(result.summary.imported, 0);
    assert_eq!(result.summary.skipped, 1);
    assert!(
        service.titles(other).is_empty(),
        "{:?}",
        service.titles(other)
    );
}

#[test]
fn an_edit_is_one_request_and_one_notification() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    let folder = service.folder(other, "F");
    let a = service.bookmark(other, "A", "https://a.test/");
    let seen = Rc::new(RefCell::new(Vec::new()));
    let record = Rc::clone(&seen);
    let _listener = service
        .data
        .on_change(crate::profile_data::Area::Bookmarks, move |change| {
            record.borrow_mut().push(change.folders.clone());
        });
    // A notification may come in a delivery after its write's reply: a read flushes those of
    // the setup, which are not part of the test.
    service.root(BookmarkRoot::Toolbar);
    seen.borrow_mut().clear();
    service
        .ask(|data, reply| {
            data.edit_bookmark(
                a,
                BookmarkEdit {
                    title: Some("Renamed".into()),
                    url: Some("https://b.test/".into()),
                    folder: Some(folder),
                },
                reply,
            )
        })
        .unwrap();
    service.settle(|| !seen.borrow().is_empty());
    let mut hint = seen.borrow_mut().pop().unwrap().unwrap();
    hint.sort();
    assert_eq!(hint, vec![other, folder]);
    assert!(seen.borrow().is_empty(), "one notification");
    let node = service.ask(|data, reply| data.bookmark(a, reply)).unwrap();
    assert_eq!(node.title, "Renamed");
    assert_eq!(node.parent, Some(folder));
    assert_eq!(
        node.kind,
        BookmarkKind::Bookmark {
            url: "https://b.test/".into()
        }
    );
}

#[test]
fn a_refused_url_edit_is_reported_and_the_name_is_left_alone() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    let a = service.bookmark(other, "A", "https://a.test/");
    let result = service.ask(|data, reply| {
        data.edit_bookmark(
            a,
            BookmarkEdit {
                title: Some("Renamed".into()),
                url: Some(format!(
                    "https://a.test/{}",
                    "x".repeat(mercury_store::MAX_URL_BYTES)
                )),
                folder: None,
            },
            reply,
        )
    });
    assert!(matches!(
        result,
        Err(ProfileDataError::Store(StoreError::InvalidArgument(_)))
    ));
    assert_eq!(service.titles(other), vec!["A"]);
}

#[test]
fn undo_into_a_deleted_folder_fails_and_stores_nothing() {
    let service = Service::new();
    let other = service.root(BookmarkRoot::Other);
    let folder = service.folder(other, "F");
    let inner = service.bookmark(folder, "Inner", "https://inner.test/");
    let removed = service
        .ask(|data, reply| data.remove_bookmark_keeping_undo(inner, reply))
        .unwrap();
    service
        .ask(|data, reply| data.remove_bookmark(folder, reply))
        .unwrap();
    let result = service.ask(|data, reply| data.restore_bookmark(removed, reply));
    assert!(matches!(
        result,
        Err(ProfileDataError::Store(StoreError::NotFound))
    ));
    let found = service
        .ask(|data, reply| data.bookmark_for_url("https://inner.test/".into(), reply))
        .unwrap();
    assert_eq!(found, None);
}

// The stepwise bookmark import, moved here from the profile-data service with its job.

fn bookmark_tree(count: usize) -> Vec<BookmarkImport> {
    (0..count)
        .map(|n| BookmarkImport::Folder {
            title: format!("Folder {n}"),
            guid: crate::profile_data::new_guid().unwrap(),
            added_at_ms: None,
            children: (0..3)
                .map(|m| BookmarkImport::Bookmark {
                    title: format!("Mark {n}.{m}"),
                    url: format!("https://{n}.test/{m}"),
                    guid: crate::profile_data::new_guid().unwrap(),
                    added_at_ms: Some(NOW),
                })
                .collect(),
        })
        .collect()
}

#[test]
fn a_bookmark_import_runs_in_chunks_of_whole_trees_and_notifies_once() {
    let service = Service::new();
    let changes = Rc::new(RefCell::new(0));
    let count = Rc::clone(&changes);
    let _listener = service
        .data
        .on_change(crate::profile_data::Area::Bookmarks, move |_| {
            *count.borrow_mut() += 1
        });
    let other = service
        .ask(|data, reply| data.bookmark_root(BookmarkRoot::Other, reply))
        .unwrap();
    let toolbar = service
        .ask(|data, reply| data.bookmark_root(BookmarkRoot::Toolbar, reply))
        .unwrap();
    // 1,000 folders of 4 nodes after one on the toolbar: chunks of 125 trees, 500 nodes.
    let trees = bookmark_tree(1000);
    let progress = Rc::new(RefCell::new(Vec::new()));
    let summary = Rc::new(RefCell::new(None));
    let (seen, done) = (Rc::clone(&progress), Rc::clone(&summary));

    let _job = service.data.import_bookmarks(
        vec![(toolbar, bookmark_tree(1)), (other, trees)],
        move |p| seen.borrow_mut().push(p.done),
        move |s| *done.borrow_mut() = Some(s),
    );
    service.settle(|| summary.borrow().is_some());
    // The notification follows the summary, possibly in a later delivery.
    service.settle(|| *changes.borrow() > 0);

    let summary = summary.borrow_mut().take().unwrap();
    assert!(matches!(summary.outcome, ImportOutcome::Completed));
    assert_eq!((summary.imported, summary.total), (4004, 4004));
    assert_eq!(
        *progress.borrow(),
        (1..=8).map(|n| n * IMPORT_CHUNK as u64).collect::<Vec<_>>()
    );
    assert_eq!(*changes.borrow(), 1);
    let positions = service
        .ask(|data, reply| data.bookmark_children(other, reply))
        .unwrap()
        .into_iter()
        .map(|node| node.position)
        .collect::<Vec<_>>();
    assert_eq!(positions, (0..1000).collect::<Vec<_>>());
}

#[test]
fn a_refused_bookmark_folder_is_skipped_with_its_contents() {
    let service = Service::new();
    let other = service
        .ask(|data, reply| data.bookmark_root(BookmarkRoot::Other, reply))
        .unwrap();
    let mut trees = bookmark_tree(600);
    let BookmarkImport::Folder { guid, .. } = &trees[0] else {
        unreachable!()
    };
    let reused = guid.clone();
    let BookmarkImport::Folder { guid, .. } = &mut trees[550] else {
        unreachable!()
    };
    *guid = reused;

    let summary = Rc::new(RefCell::new(None));
    let done = Rc::clone(&summary);
    let _job = service.data.import_bookmarks(
        vec![(other, trees)],
        |_| {},
        move |s| *done.borrow_mut() = Some(s),
    );
    service.settle(|| summary.borrow().is_some());

    let summary = summary.borrow_mut().take().unwrap();
    assert!(
        matches!(summary.outcome, ImportOutcome::Completed),
        "{:?}",
        summary.outcome
    );
    // Folder 550 (a GUID already used by folder 0) and its three bookmarks are left out.
    assert_eq!(
        (summary.imported, summary.skipped, summary.resume_from),
        (2396, 4, 600)
    );
    assert_eq!(
        service
            .ask(|data, reply| data.bookmark_children(other, reply))
            .unwrap()
            .len(),
        599
    );
}

#[test]
fn a_bookmark_import_notifies_the_folders_it_filled() {
    let service = Service::new();
    let changes = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen = Rc::clone(&changes);
    let _listener = service
        .data
        .on_change(crate::profile_data::Area::Bookmarks, move |change| {
            seen.borrow_mut().push(format!("{:?}", change.folders))
        });
    let other = service
        .ask(|data, reply| data.bookmark_root(BookmarkRoot::Other, reply))
        .unwrap();
    let summary = Rc::new(RefCell::new(None));
    let done = Rc::clone(&summary);

    let _job = service.data.import_bookmarks(
        vec![(other, bookmark_tree(300))],
        |_| {},
        move |s| *done.borrow_mut() = Some(s),
    );
    service.settle(|| summary.borrow().is_some() && !changes.borrow().is_empty());

    assert_eq!(*changes.borrow(), vec![format!("{:?}", Some(vec![other]))]);
}
