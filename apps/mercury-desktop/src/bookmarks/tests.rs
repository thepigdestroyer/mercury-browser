//! The bookmark operations the star, the bar and the manager use, against a database file
//! through the profile-data service, and the bookmarks-file round trip.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use mercury_import::{ImportedNode, parse_netscape_bookmarks, write_netscape_bookmarks};
use mercury_store::{BookmarkId, BookmarkKind, BookmarkRoot, Store, StoreError};

use super::tree;
use crate::profile_data::import::ImportOutcome;
use crate::profile_data::{ProfileData, ProfileDataError, Timing};

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
