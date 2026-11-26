// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What the platform adapter sees. A bare text run under a paragraph is filtered out by the common
//! filter (macOS VoiceOver never reads it); a label that carries the run is kept and reads its text.
//! These tests document the consumer's behaviour that A11Y-2's text runs rely on (finding F1).

use accesskit::{Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use accesskit_consumer::{Node as ConsumerNode, Tree as Oracle, common_filter};

const WINDOW: NodeId = NodeId(0);
const PARAGRAPH: NodeId = NodeId(1);
const LABEL: NodeId = NodeId(2);
const RUN: NodeId = NodeId(3);

fn update(nodes: Vec<(NodeId, Node)>) -> TreeUpdate {
    TreeUpdate {
        nodes,
        tree: Some(Tree::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus: WINDOW,
    }
}

fn node(role: Role, children: &[NodeId]) -> Node {
    let mut node = Node::new(role);
    node.set_children(children.to_vec());
    node
}

fn text_run(value: &str) -> Node {
    let mut run = Node::new(Role::TextRun);
    run.set_value(value);
    run.set_character_lengths(vec![1; value.len()]);
    run
}

/// Every node the platform adapter would expose: the common filter applied down from the root.
fn platform_visible(oracle: &Oracle) -> Vec<String> {
    let mut seen = Vec::new();
    collect(oracle.state().root(), &mut seen);
    seen
}

fn collect(node: ConsumerNode<'_>, seen: &mut Vec<String>) {
    seen.push(format!("{:?}:{:?}", node.role(), node.value()));
    for child in node.filtered_children(&common_filter) {
        collect(child, seen);
    }
}

fn find(node: ConsumerNode<'_>, role: Role) -> Option<ConsumerNode<'_>> {
    if node.role() == role {
        return Some(node);
    }
    node.children().find_map(|child| find(child, role))
}

#[test]
fn a_bare_text_run_under_a_paragraph_is_invisible_to_the_platform() {
    let oracle = Oracle::new(
        update(vec![
            (WINDOW, node(Role::Window, &[PARAGRAPH])),
            (PARAGRAPH, node(Role::Paragraph, &[RUN])),
            (RUN, text_run("Hello")),
        ]),
        true,
    );
    let visible = platform_visible(&oracle);
    assert!(
        !visible.iter().any(|v| v.ends_with("Some(\"Hello\")")),
        "a bare text run must be filtered out, saw {visible:?}"
    );
}

#[test]
fn a_label_with_text_runs_is_visible_and_reads_its_text() {
    let oracle = Oracle::new(
        update(vec![
            (WINDOW, node(Role::Window, &[PARAGRAPH])),
            (PARAGRAPH, node(Role::Paragraph, &[LABEL])),
            (LABEL, {
                let mut label = node(Role::Label, &[RUN]);
                label.set_value("Hello");
                label
            }),
            (RUN, text_run("Hello")),
        ]),
        true,
    );
    let collected = platform_visible(&oracle);
    assert!(
        collected.iter().any(|v| v.starts_with("Label:")),
        "the label must be kept by the common filter, saw {collected:?}"
    );
    let label = find(oracle.state().root(), Role::Label).expect("the label node is in the tree");
    assert_eq!(label.value().as_deref(), Some("Hello"));
    assert!(label.supports_text_ranges());
}
