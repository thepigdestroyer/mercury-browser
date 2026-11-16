//! Pure conversions of the bookmark tree: walks (pre-order lists with depths) into nested
//! trees for a bookmarks file or for undo, imported trees into the store's import form, the
//! folder list of the chooser, and search matching.

use mercury_import::ImportedNode;
use mercury_store::{BookmarkId, BookmarkImport, BookmarkKind};

use crate::profile_data::bookmarks::{Walked, nest};

/// `url` without a user name or password, as history records it: a bookmark (made, imported
/// or exported) never keeps credentials. Text that is not an absolute URL is returned as is.
pub(crate) fn without_credentials(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) if !parsed.username().is_empty() || parsed.password().is_some() => {
            // Both fail only for URLs that cannot have credentials, which then have none.
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.into()
        }
        _ => url.to_owned(),
    }
}

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

/// The children of the walk's start, as a bookmarks file holds them. Folders carry their
/// dates; bookmarks their dates too.
pub(crate) fn imported_nodes(walked: &[Walked]) -> Vec<ImportedNode> {
    nest(walked, 1, |node, children| {
        Some(match &node.kind {
            BookmarkKind::Bookmark { url } => ImportedNode::Bookmark {
                title: node.title.clone(),
                url: without_credentials(url),
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
                    url: without_credentials(url),
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

#[cfg(test)]
mod tests {
    use mercury_store::BookmarkNode;

    use super::*;
    use crate::profile_data::bookmarks::{matches, search_words, undo_tree};

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
    fn credentials_are_stripped_from_urls() {
        assert_eq!(
            without_credentials("https://user:secret@a.test/x?q=1#f"),
            "https://a.test/x?q=1#f"
        );
        assert_eq!(without_credentials("http://user@a.test/"), "http://a.test/");
        assert_eq!(without_credentials("https://a.test/"), "https://a.test/");
        assert_eq!(
            without_credentials("file:///tmp/a.html"),
            "file:///tmp/a.html"
        );
        assert_eq!(without_credentials("not a url"), "not a url");
    }

    #[test]
    fn imports_and_exports_drop_credentials() {
        let walked = vec![
            folder(0, 1, "Bar"),
            bookmark(1, 2, "Secret", "https://me:pw@a.test/"),
        ];
        assert_eq!(
            imported_nodes(&walked),
            vec![ImportedNode::Bookmark {
                title: "Secret".into(),
                url: "https://a.test/".into(),
                added_at_ms: Some(2000),
            }]
        );
        let mut guid = || Some("g".to_owned());
        let imports = bookmark_imports(
            &[ImportedNode::Bookmark {
                title: "Secret".into(),
                url: "https://me:pw@a.test/".into(),
                added_at_ms: None,
            }],
            &mut guid,
        )
        .unwrap();
        assert!(
            matches!(&imports[0], BookmarkImport::Bookmark { url, .. } if url == "https://a.test/")
        );
    }

    #[test]
    fn an_export_escapes_titles_and_urls() {
        let walked = vec![
            folder(0, 1, "Bar"),
            bookmark(
                1,
                2,
                "He said \"hi\" </a><script>alert(1)</script>",
                "https://a.test/?q=\"</a><script>",
            ),
        ];
        let html = mercury_import::write_netscape_bookmarks(&export_nodes(&walked, &[]));
        assert!(!html.contains("<script>"), "{html}");
        assert!(!html.contains("</a><"), "{html}");
        assert!(
            html.contains("He said &quot;hi&quot; &lt;/a&gt;&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{html}"
        );
        assert!(
            html.contains("HREF=\"https://a.test/?q=&quot;&lt;/a&gt;&lt;script&gt;\""),
            "{html}"
        );
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
