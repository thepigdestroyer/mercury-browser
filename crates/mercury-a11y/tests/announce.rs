// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What the macOS adapter would announce for our live updates (EE8). Each case applies a sequence
//! of `TreeUpdate`s to `accesskit_consumer::Tree` and records what `accesskit_macos`'s event
//! generator would post: a node added with a value and `live != Off`, or a node updated with a new
//! value or live state (`accesskit_macos` `event.rs`, `node_added` and `node_updated`). The
//! handler copies that rule; it is the oracle for brief §6's "a live node speaks only when it has a
//! value".

use accesskit::{Live, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use accesskit_consumer::{
    FilterResult, Node as Seen, Tree as Oracle, TreeChangeHandler, common_filter,
};

const WINDOW: NodeId = NodeId(0);

/// Records the text the macOS adapter would announce, in order.
#[derive(Default)]
struct Speech {
    spoken: Vec<String>,
}

fn announces(node: &Seen) -> Option<String> {
    if common_filter(node) != FilterResult::Include {
        return None;
    }
    let value = node.value()?;
    (node.live() != Live::Off).then_some(value)
}

impl TreeChangeHandler for Speech {
    fn node_added(&mut self, node: &Seen) {
        if let Some(text) = announces(node) {
            self.spoken.push(text);
        }
    }

    fn node_updated(&mut self, old_node: &Seen, new_node: &Seen) {
        let old_filter = common_filter(old_node);
        let Some(text) = announces(new_node) else {
            return;
        };
        if new_node.value() != old_node.value()
            || new_node.live() != old_node.live()
            || old_filter != FilterResult::Include
        {
            self.spoken.push(text);
        }
    }

    fn focus_moved(&mut self, _: Option<&Seen>, _: Option<&Seen>) {}

    fn node_removed(&mut self, _: &Seen) {}
}

/// Applies each update in turn and returns what the adapter would have spoken.
fn spoken(updates: &[TreeUpdate]) -> Vec<String> {
    let mut oracle = Oracle::new(updates[0].clone(), true);
    let mut speech = Speech::default();
    for update in &updates[1..] {
        oracle.update_and_process_changes(update.clone(), &mut speech);
    }
    speech.spoken
}

fn window(children: &[NodeId]) -> Node {
    let mut node = Node::new(Role::Window);
    node.set_children(children.to_vec());
    node
}

fn first(children: &[NodeId], extra: Vec<(NodeId, Node)>) -> TreeUpdate {
    let mut nodes = vec![(WINDOW, window(children))];
    nodes.extend(extra);
    TreeUpdate {
        nodes,
        tree: Some(Tree::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus: WINDOW,
    }
}

fn update(children: &[NodeId], extra: Vec<(NodeId, Node)>) -> TreeUpdate {
    let mut nodes = vec![(WINDOW, window(children))];
    nodes.extend(extra);
    TreeUpdate {
        nodes,
        tree: None,
        tree_id: TreeId::ROOT,
        focus: WINDOW,
    }
}

fn alert(text: Option<&str>) -> Node {
    let mut node = Node::new(Role::Alert);
    node.set_live(Live::Assertive);
    if let Some(text) = text {
        node.set_value(text);
    }
    node
}

fn polite_log(text: &str) -> Node {
    let mut node = Node::new(Role::Log);
    node.set_live(Live::Polite);
    node.set_value(text);
    node
}

#[test]
fn loading_a_page_with_an_alert_speaks_nothing() {
    // The first sight of a live node holds its value back (brief F3, T12): no value, so no speech.
    let spoken = spoken(&[first(&[NodeId(1)], vec![(NodeId(1), alert(None))])]);
    assert_eq!(spoken, Vec::<String>::new());
}

#[test]
fn the_alert_text_changing_later_speaks_the_new_text() {
    let spoken = spoken(&[
        first(&[NodeId(1)], vec![(NodeId(1), alert(None))]),
        update(&[NodeId(1)], vec![(NodeId(1), alert(Some("Saved")))]),
    ]);
    assert_eq!(spoken, vec!["Saved".to_string()]);
}

#[test]
fn a_polite_log_speaks_only_what_was_added() {
    let spoken = spoken(&[
        first(&[NodeId(1)], vec![(NodeId(1), polite_log("Start"))]),
        update(&[NodeId(1)], vec![(NodeId(1), polite_log("Start done"))]),
    ]);
    assert_eq!(spoken.len(), 1);
    assert_eq!(spoken[0], "Start done");
}

#[test]
fn a_transient_announcement_node_speaks_when_added_and_not_when_removed() {
    let spoken = spoken(&[
        first(&[], vec![]),
        update(&[NodeId(1)], vec![(NodeId(1), alert(Some("Saved")))]),
        update(&[], vec![]),
    ]);
    assert_eq!(spoken, vec!["Saved".to_string()]);
}

#[test]
fn the_same_aria_notify_text_twice_speaks_twice() {
    let spoken = spoken(&[
        first(&[], vec![]),
        update(&[NodeId(1)], vec![(NodeId(1), alert(Some("Saved")))]),
        update(&[NodeId(2)], vec![(NodeId(2), alert(Some("Saved")))]),
    ]);
    assert_eq!(spoken, vec!["Saved".to_string(), "Saved".to_string()]);
}

#[test]
fn a_generic_container_with_live_never_speaks() {
    // Brief §6: a `GenericContainer` with `aria-live` is not a live announcement on the platform.
    // The consumer filter excludes it, so its value is never posted.
    let mut container = Node::new(Role::GenericContainer);
    container.set_live(Live::Polite);
    container.set_value("hidden");
    let spoken = spoken(&[
        first(&[], vec![]),
        update(&[NodeId(1)], vec![(NodeId(1), container)]),
    ]);
    assert_eq!(spoken, Vec::<String>::new());
}

#[test]
fn a_hidden_live_region_is_silent() {
    let mut node = alert(Some("Saved"));
    node.set_hidden();
    let spoken = spoken(&[
        first(&[], vec![]),
        update(&[NodeId(1)], vec![(NodeId(1), node)]),
    ]);
    assert_eq!(spoken, Vec::<String>::new());
}
