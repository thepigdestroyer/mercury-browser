// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! How a bookmarks file is imported in chunks of bounded size, whatever its shape.
//!
//! The service's stepwise import makes its chunks of whole top-level nodes, and real exports
//! put nearly everything in one or two top-level folders. So every folder holding more than
//! [`IMPORT_CHUNK`](crate::profile_data::import::IMPORT_CHUNK) nodes is created up front
//! (recursively), and its children become a set of their own. Created folders are appended to
//! their parents before the rest arrives, so afterwards each is moved to its place among its
//! siblings: per parent, in descending order of place, which leaves every sibling where the
//! file had it (each move only shifts entries before its target, all of which are already in
//! order).

use mercury_store::{BookmarkId, BookmarkImport};

use crate::profile_data::bookmarks::{FolderRef, NewFolder};

/// A file's import: the folders to create first (in pre-order, each with its place among its
/// siblings), then the sets to import, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportPlan {
    pub(crate) folders: Vec<(NewFolder, u32)>,
    pub(crate) sets: Vec<(FolderRef, Vec<BookmarkImport>)>,
}

/// Plans importing `nodes` into `target` so that no top-level node of a set is larger than
/// `chunk` nodes unless it is a bookmark or a separator.
pub(crate) fn plan_import(
    target: BookmarkId,
    nodes: Vec<BookmarkImport>,
    chunk: usize,
) -> ImportPlan {
    let mut plan = ImportPlan {
        folders: Vec::new(),
        sets: Vec::new(),
    };
    // Folders still to split, in pre-order: their parent and children. A stack, not
    // recursion; the parser bounds the depth anyway.
    let mut pending = vec![(FolderRef::Existing(target), nodes)];
    while let Some((parent, children)) = pending.pop() {
        let mut small = Vec::new();
        let mut large = Vec::new();
        for (place, node) in children.into_iter().enumerate() {
            match node {
                BookmarkImport::Folder {
                    title,
                    guid,
                    added_at_ms,
                    children,
                } if node_count_of(&children) + 1 > chunk => {
                    let created = FolderRef::Created(plan.folders.len());
                    plan.folders.push((
                        NewFolder {
                            parent,
                            title,
                            guid,
                            added_at_ms,
                        },
                        u32::try_from(place).unwrap_or(u32::MAX),
                    ));
                    large.push((created, children));
                }
                node => small.push(node),
            }
        }
        if !small.is_empty() {
            plan.sets.push((parent, small));
        }
        // Pre-order: this folder's large children are split before its later siblings'.
        pending.extend(large.into_iter().rev());
    }
    plan
}

/// The nodes of these trees, counting everything in their folders.
fn node_count_of(nodes: &[BookmarkImport]) -> usize {
    let mut count = 0;
    let mut stack = nodes.iter().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        count += 1;
        if let BookmarkImport::Folder { children, .. } = node {
            stack.extend(children);
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: BookmarkId = BookmarkId(42);

    fn mark(title: &str) -> BookmarkImport {
        BookmarkImport::Bookmark {
            title: title.to_owned(),
            url: format!("https://{title}.test/"),
            guid: format!("g-{title}"),
            added_at_ms: None,
        }
    }

    fn folder(title: &str, children: Vec<BookmarkImport>) -> BookmarkImport {
        BookmarkImport::Folder {
            title: title.to_owned(),
            guid: format!("g-{title}"),
            added_at_ms: Some(1_000),
            children,
        }
    }

    fn marks(prefix: &str, count: usize) -> Vec<BookmarkImport> {
        (0..count).map(|n| mark(&format!("{prefix}{n}"))).collect()
    }

    fn new_folder(parent: FolderRef, title: &str) -> NewFolder {
        NewFolder {
            parent,
            title: title.to_owned(),
            guid: format!("g-{title}"),
            added_at_ms: Some(1_000),
        }
    }

    #[test]
    fn small_trees_are_one_set_and_create_nothing() {
        let nodes = vec![mark("a"), folder("f", marks("m", 10)), mark("b")];
        let plan = plan_import(TARGET, nodes.clone(), 500);
        assert!(plan.folders.is_empty());
        assert_eq!(plan.sets, vec![(FolderRef::Existing(TARGET), nodes)]);
    }

    #[test]
    fn a_folder_of_exactly_one_chunk_is_not_split() {
        // The folder and 499 bookmarks: 500 nodes.
        let nodes = vec![folder("f", marks("m", 499))];
        let plan = plan_import(TARGET, nodes, 500);
        assert!(plan.folders.is_empty());
    }

    #[test]
    fn a_large_folder_is_created_first_and_its_children_are_their_own_set() {
        let plan = plan_import(TARGET, vec![folder("All", marks("m", 2_000))], 500);
        assert_eq!(
            plan.folders,
            vec![(new_folder(FolderRef::Existing(TARGET), "All"), 0)]
        );
        assert_eq!(plan.sets, vec![(FolderRef::Created(0), marks("m", 2_000))]);
    }

    #[test]
    fn created_folders_keep_their_place_among_their_siblings() {
        let nodes = vec![
            mark("a"),
            folder("Big", marks("big", 600)),
            mark("b"),
            folder(
                "Outer",
                vec![mark("e"), folder("Inner", marks("in", 700)), mark("f")],
            ),
            folder("Small", vec![mark("c")]),
        ];
        let plan = plan_import(TARGET, nodes, 500);
        assert_eq!(
            plan.folders,
            vec![
                (new_folder(FolderRef::Existing(TARGET), "Big"), 1),
                (new_folder(FolderRef::Existing(TARGET), "Outer"), 3),
                (new_folder(FolderRef::Created(1), "Inner"), 1),
            ]
        );
        assert_eq!(
            plan.sets,
            vec![
                (
                    FolderRef::Existing(TARGET),
                    vec![mark("a"), mark("b"), folder("Small", vec![mark("c")])]
                ),
                (FolderRef::Created(0), marks("big", 600)),
                (FolderRef::Created(1), vec![mark("e"), mark("f")]),
                (FolderRef::Created(2), marks("in", 700)),
            ]
        );
    }
}
