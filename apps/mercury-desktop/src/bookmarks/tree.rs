//! Pure conversions of the bookmark tree: walks (pre-order lists with depths) into nested
//! trees for a bookmarks file or for undo, imported trees into the store's import form, the
//! folder list of the chooser, and search matching.

use mercury_import::ImportedNode;
use mercury_store::{BookmarkId, BookmarkImport, BookmarkKind, BookmarkNode};

use crate::profile_data::bookmarks::Walked;

/// One folder as the folder chooser and the manager's tree list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FolderRow {
    pub(crate) id: BookmarkId,
    pub(crate) title: String,
    pub(crate) depth: usize,
}

/// The folders of a walk, in its order.
pub(crate) fn folder_rows(walked: &[Walked]) -> Vec<FolderRow> {
    walked
        .iter()
        .filter(|entry| entry.node.kind == BookmarkKind::Folder)
        .map(|entry| FolderRow {
            id: entry.node.id,
            title: entry.node.title.clone(),
            depth: entry.depth,
        })
        .collect()
}

/// The chooser's labels: each folder's title, indented by its depth.
pub(crate) fn chooser_labels(rows: &[FolderRow]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let title = if row.title.is_empty() {
                "(untitled)"
            } else {
                row.title.as_str()
            };
            format!("{}{title}", "\u{2003}".repeat(row.depth))
        })
        .collect()
}

/// Builds nested trees from a pre-order walk: entries at `base` depth become the top level.
/// `make` turns a node and its finished children into a tree node.
fn nest<T>(
    walked: &[Walked],
    base: usize,
    mut make: impl FnMut(&BookmarkNode, Vec<T>) -> Option<T>,
) -> Vec<T> {
    let mut top: Vec<T> = Vec::new();
    let mut open: Vec<(&BookmarkNode, Vec<T>)> = Vec::new();
    let mut close = |open: &mut Vec<(&BookmarkNode, Vec<T>)>, top: &mut Vec<T>| {
        if let Some((node, children)) = open.pop()
            && let Some(made) = make(node, children)
        {
            match open.last_mut() {
                Some((_, siblings)) => siblings.push(made),
                None => top.push(made),
            }
        }
    };
    for entry in walked.iter().filter(|entry| entry.depth >= base) {
        let level = entry.depth - base;
        while open.len() > level {
            close(&mut open, &mut top);
        }
        open.push((&entry.node, Vec::new()));
    }
    while !open.is_empty() {
        close(&mut open, &mut top);
    }
    top
}

/// The children of the walk's start, as a bookmarks file holds them. Folders carry their
/// dates; bookmarks their dates too.
pub(crate) fn imported_nodes(walked: &[Walked]) -> Vec<ImportedNode> {
    nest(walked, 1, |node, children| {
        Some(match &node.kind {
            BookmarkKind::Bookmark { url } => ImportedNode::Bookmark {
                title: node.title.clone(),
                url: url.clone(),
                added_at_ms: Some(node.added_at_ms),
            },
            BookmarkKind::Folder => ImportedNode::Folder {
                title: node.title.clone(),
                toolbar: false,
                added_at_ms: Some(node.added_at_ms),
                children,
            },
            BookmarkKind::Separator => ImportedNode::Separator,
        })
    })
}

/// What a bookmarks file of the whole tree holds: the bookmarks bar as the toolbar folder,
/// then the other bookmarks at the top level, as Chrome writes its export.
pub(crate) fn export_nodes(toolbar: &[Walked], other: &[Walked]) -> Vec<ImportedNode> {
    let bar_title = toolbar.first().map_or_else(
        || "Bookmarks Bar".to_owned(),
        |root| root.node.title.clone(),
    );
    let mut nodes = vec![ImportedNode::Folder {
        title: bar_title,
        toolbar: true,
        added_at_ms: None,
        children: imported_nodes(toolbar),
    }];
    nodes.extend(imported_nodes(other));
    nodes
}

/// An imported tree in the store's form, every node with a fresh GUID from `guid`; `None` if
/// a GUID could not be made. Toolbar folders become ordinary folders.
pub(crate) fn bookmark_imports(
    nodes: &[ImportedNode],
    guid: &mut impl FnMut() -> Option<String>,
) -> Option<Vec<BookmarkImport>> {
    nodes
        .iter()
        .map(|node| {
            Some(match node {
                ImportedNode::Folder {
                    title,
                    added_at_ms,
                    children,
                    ..
                } => BookmarkImport::Folder {
                    title: title.clone(),
                    guid: guid()?,
                    added_at_ms: *added_at_ms,
                    children: bookmark_imports(children, guid)?,
                },
                ImportedNode::Bookmark {
                    title,
                    url,
                    added_at_ms,
                } => BookmarkImport::Bookmark {
                    title: title.clone(),
                    url: url.clone(),
                    guid: guid()?,
                    added_at_ms: *added_at_ms,
                },
                ImportedNode::Separator => BookmarkImport::Separator { guid: guid()? },
            })
        })
        .collect()
}

/// The entries of an imported tree, counting everything in its folders.
pub(crate) fn count_imported(nodes: &[ImportedNode]) -> u64 {
    let mut count = 0;
    let mut stack = nodes.iter().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        count += 1;
        if let ImportedNode::Folder { children, .. } = node {
            stack.extend(children);
        }
    }
    count
}

/// The walked entry (at depth 0) and everything in it, with their own GUIDs and dates, to put
/// back after a deletion.
pub(crate) fn undo_tree(walked: &[Walked]) -> Option<BookmarkImport> {
    let base = walked.first()?.depth;
    nest(walked, base, |node, children| {
        Some(match &node.kind {
            BookmarkKind::Bookmark { url } => BookmarkImport::Bookmark {
                title: node.title.clone(),
                url: url.clone(),
                guid: node.guid.clone(),
                added_at_ms: Some(node.added_at_ms),
            },
            BookmarkKind::Folder => BookmarkImport::Folder {
                title: node.title.clone(),
                guid: node.guid.clone(),
                added_at_ms: Some(node.added_at_ms),
                children,
            },
            BookmarkKind::Separator => BookmarkImport::Separator {
                guid: node.guid.clone(),
            },
        })
    })
    .into_iter()
    .next()
}

/// A search's words, lower-cased.
pub(crate) fn search_words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// Whether a bookmark or folder matches every word in its title or URL. Separators and an
/// empty search match nothing.
pub(crate) fn matches(node: &BookmarkNode, words: &[String]) -> bool {
    if words.is_empty() {
        return false;
    }
    let title = node.title.to_lowercase();
    let url = match &node.kind {
        BookmarkKind::Bookmark { url } => url.to_lowercase(),
        BookmarkKind::Folder => String::new(),
        BookmarkKind::Separator => return false,
    };
    words
        .iter()
        .all(|word| title.contains(word.as_str()) || url.contains(word.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i64, title: &str, kind: BookmarkKind) -> BookmarkNode {
        BookmarkNode {
            id: BookmarkId(id),
            guid: format!("guid{id:08}"),
            parent: None,
            position: 0,
            title: title.to_owned(),
            kind,
            added_at_ms: id * 1000,
            modified_at_ms: 0,
        }
    }

    fn folder(depth: usize, id: i64, title: &str) -> Walked {
        Walked {
            depth,
            node: node(id, title, BookmarkKind::Folder),
        }
    }

    fn bookmark(depth: usize, id: i64, title: &str, url: &str) -> Walked {
        Walked {
            depth,
            node: node(
                id,
                title,
                BookmarkKind::Bookmark {
                    url: url.to_owned(),
                },
            ),
        }
    }

    fn separator(depth: usize, id: i64) -> Walked {
        Walked {
            depth,
            node: node(id, "", BookmarkKind::Separator),
        }
    }

    /// Bar root > [News > [A, Deep > [B]], C, ---].
    fn sample() -> Vec<Walked> {
        vec![
            folder(0, 1, "Bookmarks Bar"),
            folder(1, 2, "News"),
            bookmark(2, 3, "A", "https://a.test/"),
            folder(2, 4, "Deep"),
            bookmark(3, 5, "B", "https://b.test/"),
            bookmark(1, 6, "C", "https://c.test/"),
            separator(1, 7),
        ]
    }

    #[test]
    fn the_chooser_lists_folders_in_tree_order_indented_by_depth() {
        let rows = folder_rows(&sample());
        assert_eq!(
            rows.iter()
                .map(|row| (row.id.0, row.depth))
                .collect::<Vec<_>>(),
            vec![(1, 0), (2, 1), (4, 2)]
        );
        assert_eq!(
            chooser_labels(&rows),
            vec!["Bookmarks Bar", "\u{2003}News", "\u{2003}\u{2003}Deep"]
        );
    }

    #[test]
    fn an_untitled_folder_is_labelled() {
        let rows = folder_rows(&[folder(1, 9, "")]);
        assert_eq!(chooser_labels(&rows), vec!["\u{2003}(untitled)"]);
    }

    #[test]
    fn a_walk_nests_into_the_children_of_its_start() {
        let nodes = imported_nodes(&sample());
        assert_eq!(
            nodes,
            vec![
                ImportedNode::Folder {
                    title: "News".into(),
                    toolbar: false,
                    added_at_ms: Some(2000),
                    children: vec![
                        ImportedNode::Bookmark {
                            title: "A".into(),
                            url: "https://a.test/".into(),
                            added_at_ms: Some(3000),
                        },
                        ImportedNode::Folder {
                            title: "Deep".into(),
                            toolbar: false,
                            added_at_ms: Some(4000),
                            children: vec![ImportedNode::Bookmark {
                                title: "B".into(),
                                url: "https://b.test/".into(),
                                added_at_ms: Some(5000),
                            }],
                        },
                    ],
                },
                ImportedNode::Bookmark {
                    title: "C".into(),
                    url: "https://c.test/".into(),
                    added_at_ms: Some(6000),
                },
                ImportedNode::Separator,
            ]
        );
    }

    #[test]
    fn an_export_puts_the_bar_in_the_toolbar_folder_and_the_rest_at_the_top() {
        let other = vec![
            folder(0, 10, "Other Bookmarks"),
            bookmark(1, 11, "D", "https://d.test/"),
        ];
        let nodes = export_nodes(&sample(), &other);
        assert_eq!(nodes.len(), 2);
        let ImportedNode::Folder {
            title,
            toolbar,
            children,
            ..
        } = &nodes[0]
        else {
            panic!("the bar comes first");
        };
        assert_eq!(
            (title.as_str(), *toolbar, children.len()),
            ("Bookmarks Bar", true, 3)
        );
        assert!(matches!(&nodes[1], ImportedNode::Bookmark { title, .. } if title == "D"));
    }

    #[test]
    fn imports_get_a_fresh_guid_per_node() {
        let mut next = 0;
        let mut guid = || {
            next += 1;
            Some(format!("g{next}"))
        };
        let imports = bookmark_imports(&imported_nodes(&sample()), &mut guid).unwrap();
        let BookmarkImport::Folder { guid, children, .. } = &imports[0] else {
            panic!("a folder first");
        };
        assert_eq!(guid, "g1");
        assert!(
            matches!(&children[1], BookmarkImport::Folder { guid, children, .. }
            if guid == "g3" && matches!(&children[0], BookmarkImport::Bookmark { guid, .. } if guid == "g4"))
        );
        assert!(matches!(&imports[2], BookmarkImport::Separator { guid } if guid == "g6"));
        assert_eq!(count_imported(&imported_nodes(&sample())), 6);
    }

    #[test]
    fn an_import_without_guids_is_refused() {
        let mut guid = || None;
        assert_eq!(
            bookmark_imports(&imported_nodes(&sample()), &mut guid),
            None
        );
    }

    #[test]
    fn the_undo_tree_keeps_guids_and_dates() {
        let walked = &sample()[1..5];
        let tree = undo_tree(walked).unwrap();
        let BookmarkImport::Folder {
            title,
            guid,
            added_at_ms,
            children,
        } = tree
        else {
            panic!("the removed folder");
        };
        assert_eq!(
            (title.as_str(), guid.as_str(), added_at_ms),
            ("News", "guid00000002", Some(2000))
        );
        assert_eq!(children.len(), 2);
        assert!(
            matches!(&children[1], BookmarkImport::Folder { children, .. }
            if matches!(&children[0], BookmarkImport::Bookmark { guid, .. } if guid == "guid00000005"))
        );
        assert_eq!(undo_tree(&[]), None);
    }

    #[test]
    fn search_matches_every_word_in_title_or_url_ignoring_case() {
        let words = search_words("  RUST  docs ");
        assert_eq!(words, vec!["rust", "docs"]);
        let rust = node(
            1,
            "The Rust book",
            BookmarkKind::Bookmark {
                url: "https://doc.rust-lang.org/docs".into(),
            },
        );
        assert!(matches(&rust, &words));
        assert!(!matches(&rust, &search_words("rust python")));
        assert!(matches(&node(2, "Rust docs", BookmarkKind::Folder), &words));
        assert!(!matches(&node(3, "", BookmarkKind::Separator), &words));
        assert!(!matches(&rust, &[]));
    }
}
