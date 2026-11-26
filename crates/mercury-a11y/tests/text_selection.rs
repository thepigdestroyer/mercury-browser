// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Offsets and selections across text runs: a DOM offset maps to a run and back, a selection is
//! clipped to the runs of one label, and the consumer agrees on what the selection covers.

use accesskit::{Node, NodeId, Role, TextPosition, TextSelection, Tree, TreeId, TreeUpdate};
use accesskit_consumer::{Node as ConsumerNode, Tree as Oracle};
use mercury_a11y::golden::assert_golden;
use mercury_a11y::text::{
    Direction, LineInput, RunSpec, TextPositionSpec, build_runs, clip_selection,
    dom_offset_to_position, position_to_dom_offset,
};

const WINDOW: NodeId = NodeId(0);
const LABEL: NodeId = NodeId(1);
const RUN_ONE: NodeId = NodeId(2);
const RUN_TWO: NodeId = NodeId(3);

/// "Hello " then "world", as two runs on one line; the DOM offsets run 0..=11.
fn two_runs() -> Vec<RunSpec> {
    let mut runs = Vec::new();
    for (text, first_offset) in [("Hello ", 0_usize), ("world", 6)] {
        let advances = vec![1.0_f32; text.len()];
        let offsets: Vec<usize> = (first_offset..=first_offset + text.len()).collect();
        let line = LineInput {
            text,
            advances: &advances,
            dom_offsets: &offsets,
            direction: Direction::LeftToRight,
            ends_with_hard_break: false,
        };
        runs.extend(build_runs(&line).runs);
    }
    runs
}

fn find_label(node: ConsumerNode<'_>) -> Option<ConsumerNode<'_>> {
    if node.role() == Role::Label {
        return Some(node);
    }
    node.children().find_map(find_label)
}

fn at(run: usize, character_index: usize) -> TextPositionSpec {
    TextPositionSpec {
        run,
        character_index,
    }
}

#[test]
fn a_dom_offset_maps_to_a_run_and_back() {
    let runs = two_runs();
    for offset in 0..=11 {
        let position = dom_offset_to_position(&runs, offset)
            .unwrap_or_else(|| panic!("offset {offset} maps to a run"));
        assert_eq!(
            position_to_dom_offset(&runs, position),
            Some(offset),
            "offset {offset} round-trips"
        );
    }
    assert_eq!(dom_offset_to_position(&runs, 6), Some(at(1, 0)));
}

#[test]
fn an_offset_at_the_end_is_the_end_of_the_last_run() {
    let runs = two_runs();
    let last = runs.len() - 1;
    let end = runs[last].character_lengths.len();
    assert_eq!(dom_offset_to_position(&runs, 11), Some(at(last, end)));
}

#[test]
fn collapsed_whitespace_maps_to_the_next_character() {
    let text = "a b";
    let advances = [1.0_f32; 3];
    let offsets = [0_usize, 1, 3, 4];
    let line = LineInput {
        text,
        advances: &advances,
        dom_offsets: &offsets,
        direction: Direction::LeftToRight,
        ends_with_hard_break: false,
    };
    let runs = build_runs(&line).runs;
    assert_eq!(
        dom_offset_to_position(&runs, 2),
        dom_offset_to_position(&runs, 3)
    );
}

#[test]
fn clip_selection_inside_one_label() {
    let runs = two_runs();
    let clipped = clip_selection(&runs, 2, 8).expect("the selection touches the runs");
    assert_eq!(clipped.anchor, at(0, 2));
    assert_eq!(clipped.focus, at(1, 2));
}

#[test]
fn clip_selection_that_starts_before_this_label_starts_at_the_first_run() {
    let runs = two_runs();
    let clipped = clip_selection(&runs, 0, 3).expect("the selection touches the runs");
    assert_eq!(clipped.anchor, at(0, 0));
    assert_eq!(clipped.focus, at(0, 3));
}

#[test]
fn clip_selection_that_does_not_touch_returns_none() {
    let runs = two_runs();
    assert_eq!(clip_selection(&runs, 20, 25), None);
}

#[test]
fn a_backward_selection_keeps_its_anchor_after_its_focus() {
    let runs = two_runs();
    let clipped = clip_selection(&runs, 8, 2).expect("the selection touches the runs");
    assert_eq!(clipped.anchor, at(1, 2));
    assert_eq!(clipped.focus, at(0, 2));
}

#[test]
fn the_consumer_reads_the_selection_the_clip_describes() {
    let runs = two_runs();
    let clipped = clip_selection(&runs, 2, 8).expect("the selection touches the runs");

    let mut label = Node::new(Role::Label);
    label.set_children(vec![RUN_ONE, RUN_TWO]);
    label.set_value("Hello world");
    let mut root = Node::new(Role::Window);
    root.set_children(vec![LABEL]);
    let mut run_one = Node::new(Role::TextRun);
    run_one.set_value("Hello ");
    run_one.set_character_lengths(vec![1; 6]);
    let mut run_two = Node::new(Role::TextRun);
    run_two.set_value("world");
    run_two.set_character_lengths(vec![1; 5]);
    run_two.set_word_starts(vec![0]);
    run_one.set_word_starts(vec![0]);
    let anchor = TextPosition {
        node: RUN_ONE,
        character_index: clipped.anchor.character_index,
    };
    let focus = TextPosition {
        node: RUN_TWO,
        character_index: clipped.focus.character_index,
    };
    label.set_text_selection(TextSelection { anchor, focus });

    let update = TreeUpdate {
        nodes: vec![
            (WINDOW, root),
            (LABEL, label),
            (RUN_ONE, run_one),
            (RUN_TWO, run_two),
        ],
        tree: Some(Tree::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus: WINDOW,
    };
    let oracle = Oracle::new(update, true);
    let label_node = find_label(oracle.state().root()).expect("the label is in the tree");
    let selected = label_node
        .text_selection()
        .expect("the label has a selection");
    assert_eq!(selected.text(), "llo wo");
}

#[test]
fn golden_text_label() {
    let runs = two_runs();
    let mut out = String::new();
    out.push_str("label value=\"Hello world\"\n");
    for run in &runs {
        out.push_str(&format!(
            "  text-run value={:?} lengths={:?} words={:?}\n",
            run.text, run.character_lengths, run.word_starts
        ));
    }
    if let Some(clipped) = clip_selection(&runs, 2, 8) {
        out.push_str(&format!(
            "  selection=({}@{}, {}@{})\n",
            clipped.anchor.run,
            clipped.anchor.character_index,
            clipped.focus.run,
            clipped.focus.character_index
        ));
    }
    assert_golden("text-label", &out);
}
