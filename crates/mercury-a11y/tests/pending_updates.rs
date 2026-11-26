// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Merging the tree updates that wait for the main thread (`mercury_a11y::graft::PendingUpdates`,
//! EE11). The oracle is `accesskit_consumer::Tree`: applying the merged update must give the same
//! tree as applying the sequence it replaces.

use accesskit::{Node, NodeId, Role, Tree, TreeId, TreeUpdate, Uuid};
use accesskit_consumer::{Node as ConsumerNode, Tree as Oracle, TreeChangeHandler};
use mercury_a11y::graft::PendingUpdates;
use mercury_a11y::snapshot::{DumpOptions, dump};

const ROOT: NodeId = NodeId(0);

fn tree_id(n: u128) -> TreeId {
    TreeId(Uuid::from_u128(n))
}

fn node(role: Role, children: &[u64]) -> Node {
    let mut node = Node::new(role);
    node.set_children(children.iter().map(|id| NodeId(*id)).collect::<Vec<_>>());
    node
}

fn labelled(role: Role, label: &str) -> Node {
    let mut node = Node::new(role);
    node.set_label(label);
    node
}

/// A first update of `tree`: a root with children `1` and `2`, focus on the root.
fn first(tree: TreeId, label: &str) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![
            (ROOT, node(Role::Window, &[1, 2])),
            (NodeId(1), labelled(Role::Button, label)),
            (NodeId(2), labelled(Role::Link, "link")),
        ],
        tree: Some(Tree::new(ROOT)),
        tree_id: tree,
        focus: ROOT,
    }
}

/// An update of `tree` that changes only the label of node 1 and its focus.
fn relabel(tree: TreeId, label: &str, focus: NodeId) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![(NodeId(1), labelled(Role::Button, label))],
        tree: None,
        tree_id: tree,
        focus,
    }
}

/// An update of `tree` that adds a node `id` (a child of nothing: the consumer would refuse it
/// alone, so these only ever go to the merge, never to an oracle).
fn extra(tree: TreeId, id: u64) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![(NodeId(id), labelled(Role::Link, "extra"))],
        tree: None,
        tree_id: tree,
        focus: ROOT,
    }
}

struct Quiet;

impl TreeChangeHandler for Quiet {
    fn node_added(&mut self, _: &ConsumerNode) {}
    fn node_updated(&mut self, _: &ConsumerNode, _: &ConsumerNode) {}
    fn focus_moved(&mut self, _: Option<&ConsumerNode>, _: Option<&ConsumerNode>) {}
    fn node_removed(&mut self, _: &ConsumerNode) {}
}

fn label_of(update: &TreeUpdate, id: NodeId) -> Option<String> {
    update
        .nodes
        .iter()
        .find(|(node_id, _)| *node_id == id)
        .and_then(|(_, node)| node.label().map(str::to_owned))
}

#[test]
fn two_updates_for_one_tree_merge_later_node_wins() {
    let tree = tree_id(1);
    let mut pending = PendingUpdates::new(64);
    pending.push(first(tree, "old"));
    pending.push(relabel(tree, "new", ROOT));

    let merged = pending.take().expect("one merged update");
    assert_eq!(merged.tree_id, tree);
    assert_eq!(label_of(&merged, NodeId(1)).as_deref(), Some("new"));
    assert_eq!(pending.take(), None);
}

#[test]
fn the_last_focus_wins() {
    let tree = tree_id(1);
    let mut pending = PendingUpdates::new(64);
    pending.push(first(tree, "a"));
    pending.push(relabel(tree, "a", NodeId(2)));
    pending.push(relabel(tree, "a", NodeId(1)));

    assert_eq!(pending.take().expect("merged").focus, NodeId(1));
}

#[test]
fn a_tree_field_is_kept_from_the_first_update_that_has_one() {
    let tree = tree_id(1);
    let mut pending = PendingUpdates::new(64);
    pending.push(relabel(tree, "a", ROOT));
    pending.push(first(tree, "b"));
    pending.push(relabel(tree, "c", ROOT));

    let merged = pending.take().expect("merged");
    assert_eq!(merged.tree, Some(Tree::new(ROOT)));
    assert_eq!(label_of(&merged, NodeId(1)).as_deref(), Some("c"));
}

#[test]
fn different_trees_stay_separate() {
    let page = tree_id(1);
    let frame = tree_id(2);
    let mut pending = PendingUpdates::new(64);
    pending.push(first(page, "page"));
    pending.push(first(frame, "frame"));
    pending.push(relabel(page, "page 2", ROOT));

    let first_out = pending.take().expect("page");
    let second_out = pending.take().expect("frame");
    assert_eq!(pending.take(), None);
    assert_eq!(first_out.tree_id, page);
    assert_eq!(label_of(&first_out, NodeId(1)).as_deref(), Some("page 2"));
    assert_eq!(second_out.tree_id, frame);
    assert_eq!(label_of(&second_out, NodeId(1)).as_deref(), Some("frame"));
}

#[test]
fn merging_never_exceeds_max_nodes_and_starts_a_new_update_past_it() {
    let tree = tree_id(1);
    let mut pending = PendingUpdates::new(3);
    pending.push(first(tree, "a"));
    pending.push(relabel(tree, "b", ROOT));
    assert_eq!(pending.merged(), 1);
    pending.push(extra(tree, 4));
    assert_eq!(pending.merged(), 1);

    let full = pending.take().expect("first entry");
    assert_eq!(full.nodes.len(), 3);
    assert_eq!(label_of(&full, NodeId(1)).as_deref(), Some("b"));
    let rest = pending.take().expect("second entry");
    assert_eq!(rest.nodes.len(), 1);
    assert_eq!(pending.take(), None);
}

#[test]
fn the_merged_update_applies_to_the_consumer_tree_like_the_sequence() {
    let base = first(TreeId::ROOT, "one");
    let sequence = vec![
        relabel(TreeId::ROOT, "two", NodeId(2)),
        relabel(TreeId::ROOT, "three", ROOT),
    ];

    let mut sequenced = Oracle::new(base.clone(), true);
    for update in sequence.iter().cloned() {
        sequenced.update_and_process_changes(update, &mut Quiet);
    }

    let mut merged_tree = Oracle::new(base, true);
    let mut pending = PendingUpdates::new(64);
    for update in sequence {
        pending.push(update);
    }
    let merged = pending.take().expect("merged");
    assert_eq!(pending.take(), None);
    merged_tree.update_and_process_changes(merged, &mut Quiet);

    assert_eq!(
        dump(&merged_tree, &DumpOptions::default()),
        dump(&sequenced, &DumpOptions::default())
    );
}
