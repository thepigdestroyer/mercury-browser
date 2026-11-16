//! Turns what `mercury-import` read into what the profile database takes: bookmark trees with
//! fresh GUIDs, visits as links. Runs on the reading thread.

use mercury_import::{ImportedNode, ImportedVisit};
use mercury_store::{
    BOOKMARK_IMPORT_MAX_DEPTH, BookmarkImport, MAX_TITLE_CHARS, MAX_URL_BYTES, VisitTransition,
};

use crate::profile_data::import::ImportVisit;

/// What is sent, and how many entries were left out because the store would refuse them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Converted<T> {
    pub(crate) items: T,
    pub(crate) skipped: u64,
}

/// The system's random source failed, so no GUIDs could be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NoGuids;

/// The folder an import from `browser` goes into, under "Other Bookmarks".
pub(crate) fn folder_title(browser: &str) -> String {
    format!("Imported from {browser}")
}

/// `title` as the store keeps it, so that a second import compares equal titles.
fn stored_title(title: String) -> String {
    match title.char_indices().nth(MAX_TITLE_CHARS) {
        Some((end, _)) => title[..end].to_owned(),
        None => title,
    }
}

/// The tree with a fresh GUID on every node, from `guid`.
pub(crate) fn bookmarks_for_store(
    nodes: Vec<ImportedNode>,
    guid: &mut dyn FnMut() -> Option<String>,
) -> Result<Converted<Vec<BookmarkImport>>, NoGuids> {
    let mut skipped = 0;
    let items = convert_level(nodes, 1, guid, &mut skipped)?;
    Ok(Converted { items, skipped })
}

fn convert_level(
    nodes: Vec<ImportedNode>,
    depth: usize,
    guid: &mut dyn FnMut() -> Option<String>,
    skipped: &mut u64,
) -> Result<Vec<BookmarkImport>, NoGuids> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        // import-ui: until the service skips refused rows itself (history lane), leave out
        // what the store would refuse: addresses over MAX_URL_BYTES, nodes nested too deeply.
        if depth > BOOKMARK_IMPORT_MAX_DEPTH {
            *skipped += count_nodes(&node);
            continue;
        }
        let converted = match node {
            ImportedNode::Folder {
                title,
                added_at_ms,
                children,
                ..
            } => BookmarkImport::Folder {
                title: stored_title(title),
                guid: guid().ok_or(NoGuids)?,
                added_at_ms,
                children: convert_level(children, depth + 1, guid, skipped)?,
            },
            ImportedNode::Bookmark { url, .. } if url.len() > MAX_URL_BYTES => {
                *skipped += 1;
                continue;
            }
            ImportedNode::Bookmark {
                title,
                url,
                added_at_ms,
            } => BookmarkImport::Bookmark {
                title: stored_title(title),
                url,
                guid: guid().ok_or(NoGuids)?,
                added_at_ms,
            },
            ImportedNode::Separator => BookmarkImport::Separator {
                guid: guid().ok_or(NoGuids)?,
            },
        };
        out.push(converted);
    }
    Ok(out)
}

fn count_nodes(node: &ImportedNode) -> u64 {
    match node {
        ImportedNode::Folder { children, .. } => 1 + children.iter().map(count_nodes).sum::<u64>(),
        _ => 1,
    }
}

/// Visits as links, newest first as read.
pub(crate) fn visits_for_store(visits: Vec<ImportedVisit>) -> Converted<Vec<ImportVisit>> {
    let before = visits.len() as u64;
    // import-ui: until the service skips refused rows itself (history lane), leave out
    // visits whose address the store would refuse (longer than MAX_URL_BYTES).
    let items = visits
        .into_iter()
        .filter(|visit| visit.url.len() <= MAX_URL_BYTES)
        .map(|visit| ImportVisit {
            url: visit.url,
            title: visit.title.map(stored_title),
            visited_at_ms: visit.visited_at_ms,
            transition: VisitTransition::Link,
        })
        .collect::<Vec<_>>();
    Converted {
        skipped: before - items.len() as u64,
        items,
    }
}

/// The bookmarks (not folders or separators) among `nodes`, at every depth.
pub(crate) fn bookmark_count(nodes: &[BookmarkImport]) -> u64 {
    nodes
        .iter()
        .map(|node| match node {
            BookmarkImport::Folder { children, .. } => bookmark_count(children),
            BookmarkImport::Bookmark { .. } => 1,
            BookmarkImport::Separator { .. } => 0,
        })
        .sum()
}

/// The bookmarks among the first `nodes` nodes of a pre-order walk of `sets`: what an import
/// that stopped after `nodes` nodes had imported.
pub(crate) fn bookmarks_in_first<P>(sets: &[(P, Vec<BookmarkImport>)], nodes: u64) -> u64 {
    let mut left = nodes;
    let mut bookmarks = 0;
    let mut stack = sets
        .iter()
        .rev()
        .flat_map(|(_, nodes)| nodes.iter().rev())
        .collect::<Vec<_>>();
    while left > 0 {
        let Some(node) = stack.pop() else {
            break;
        };
        left -= 1;
        match node {
            BookmarkImport::Folder { children, .. } => stack.extend(children.iter().rev()),
            BookmarkImport::Bookmark { .. } => bookmarks += 1,
            BookmarkImport::Separator { .. } => {}
        }
    }
    bookmarks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> impl FnMut() -> Option<String> {
        let mut next = 0;
        move || {
            next += 1;
            Some(format!("guid{next:08}"))
        }
    }

    fn bookmark(title: &str, url: &str) -> ImportedNode {
        ImportedNode::Bookmark {
            title: title.into(),
            url: url.into(),
            added_at_ms: Some(1_700_000_000_000),
        }
    }

    fn folder(title: &str, children: Vec<ImportedNode>) -> ImportedNode {
        ImportedNode::Folder {
            title: title.into(),
            toolbar: false,
            added_at_ms: None,
            children,
        }
    }

    #[test]
    fn every_node_gets_a_fresh_guid_and_keeps_its_place() {
        let tree = vec![
            folder(
                "Bookmarks Toolbar",
                vec![
                    bookmark("News", "https://news.example/"),
                    ImportedNode::Separator,
                ],
            ),
            bookmark("Mail", "https://mail.example/"),
        ];

        let converted = bookmarks_for_store(tree, &mut counter()).unwrap();

        assert_eq!(converted.skipped, 0);
        assert_eq!(
            converted.items,
            vec![
                BookmarkImport::Folder {
                    title: "Bookmarks Toolbar".into(),
                    guid: "guid00000001".into(),
                    added_at_ms: None,
                    children: vec![
                        BookmarkImport::Bookmark {
                            title: "News".into(),
                            url: "https://news.example/".into(),
                            guid: "guid00000002".into(),
                            added_at_ms: Some(1_700_000_000_000),
                        },
                        BookmarkImport::Separator {
                            guid: "guid00000003".into()
                        },
                    ],
                },
                BookmarkImport::Bookmark {
                    title: "Mail".into(),
                    url: "https://mail.example/".into(),
                    guid: "guid00000004".into(),
                    added_at_ms: Some(1_700_000_000_000),
                },
            ]
        );
    }

    #[test]
    fn what_the_store_would_refuse_is_left_out_and_counted() {
        let long = format!("https://long.example/{}", "a".repeat(MAX_URL_BYTES));
        let mut deep = bookmark("Deep", "https://deep.example/");
        for level in 0..BOOKMARK_IMPORT_MAX_DEPTH {
            deep = folder(&format!("Level {level}"), vec![deep]);
        }
        let tree = vec![
            bookmark("Long", &long),
            bookmark("Fine", "https://fine.example/"),
            deep,
        ];

        let converted = bookmarks_for_store(tree, &mut counter()).unwrap();

        // The outermost folder is at depth 1; the bookmark at depth 101 is refused.
        assert_eq!(converted.skipped, 2);
        assert_eq!(converted.items.len(), 2);

        let visits = visits_for_store(vec![
            ImportedVisit {
                url: long,
                title: None,
                visited_at_ms: 2,
            },
            ImportedVisit {
                url: "https://fine.example/".into(),
                title: Some("Fine".into()),
                visited_at_ms: 1,
            },
        ]);
        assert_eq!(visits.skipped, 1);
        assert_eq!(
            visits.items,
            vec![ImportVisit {
                url: "https://fine.example/".into(),
                title: Some("Fine".into()),
                visited_at_ms: 1,
                transition: VisitTransition::Link,
            }]
        );
    }

    #[test]
    fn titles_are_cut_as_the_store_cuts_them() {
        let long_title = "é".repeat(MAX_TITLE_CHARS + 10);
        let converted = bookmarks_for_store(
            vec![bookmark(&long_title, "https://a.example/")],
            &mut counter(),
        )
        .unwrap();
        let BookmarkImport::Bookmark { title, .. } = &converted.items[0] else {
            panic!("a bookmark");
        };
        assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn without_guids_the_conversion_fails() {
        assert_eq!(
            bookmarks_for_store(vec![bookmark("A", "https://a.example/")], &mut || None),
            Err(NoGuids)
        );
    }

    #[test]
    fn bookmarks_are_counted_in_the_whole_tree_and_in_an_imported_prefix() {
        let tree = bookmarks_for_store(
            vec![
                folder(
                    "Toolbar",
                    vec![
                        bookmark("A", "https://a.example/"),
                        ImportedNode::Separator,
                        folder("Inner", vec![bookmark("B", "https://b.example/")]),
                    ],
                ),
                bookmark("C", "https://c.example/"),
            ],
            &mut counter(),
        )
        .unwrap()
        .items;
        assert_eq!(bookmark_count(&tree), 3);
        let sets = vec![((), tree)];
        // Pre-order: Toolbar, A, separator, Inner, B, C.
        assert_eq!(bookmarks_in_first(&sets, 0), 0);
        assert_eq!(bookmarks_in_first(&sets, 2), 1);
        assert_eq!(bookmarks_in_first(&sets, 4), 1);
        assert_eq!(bookmarks_in_first(&sets, 5), 2);
        assert_eq!(bookmarks_in_first(&sets, 6), 3);
        assert_eq!(bookmarks_in_first(&sets, 60), 3);
    }

    #[test]
    fn the_folder_is_named_after_the_browser() {
        assert_eq!(folder_title("Firefox"), "Imported from Firefox");
    }
}
