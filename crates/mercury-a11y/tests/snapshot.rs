// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The tree snapshot text (`mercury_a11y::snapshot`) and the golden files (`mercury_a11y::golden`),
//! on hand-built trees fed to `accesskit_consumer::Tree`, the tree the platform adapters keep.
//! Test names start with `snapshot_` or `golden_`.
//!
//! Rewrite the committed files with:
//! `MERCURY_A11Y_BLESS=1 cargo test --release -p mercury-a11y --test snapshot`

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use accesskit::{
    Action, Affine, AriaCurrent, HasPopup, Invalid, Node, NodeId, Orientation, Rect, Role, Toggled,
    Tree, TreeId, TreeUpdate, Uuid,
};
use accesskit_consumer::{Node as ConsumerNode, Tree as Consumer, TreeChangeHandler};
use mercury_a11y::golden::{self, assert_golden, check_golden, header, without_header};
use mercury_a11y::snapshot::{DumpOptions, describe, dump, role_name};

struct NoChanges;

impl TreeChangeHandler for NoChanges {
    fn node_added(&mut self, _: &ConsumerNode) {}
    fn node_updated(&mut self, _: &ConsumerNode, _: &ConsumerNode) {}
    fn focus_moved(&mut self, _: Option<&ConsumerNode>, _: Option<&ConsumerNode>) {}
    fn node_removed(&mut self, _: &ConsumerNode) {}
}

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

fn graft(role: Role, tree: TreeId) -> Node {
    let mut node = Node::new(role);
    node.set_tree_id(tree);
    node
}

fn update(tree: TreeId, nodes: Vec<(u64, Node)>) -> TreeUpdate {
    TreeUpdate {
        nodes: nodes
            .into_iter()
            .map(|(id, node)| (NodeId(id), node))
            .collect(),
        tree: Some(Tree::new(NodeId(0))),
        tree_id: tree,
        focus: NodeId(0),
    }
}

/// A tree made of the updates in order (the first is the root tree's).
fn consumer(updates: Vec<TreeUpdate>) -> Consumer {
    let mut updates = updates.into_iter();
    let mut tree = Consumer::new(updates.next().expect("a root update"), true);
    for update in updates {
        tree.update_and_process_changes(update, &mut NoChanges);
    }
    tree
}

const WEB_VIEW: u128 = 1;
const DOCUMENT: u128 = 2;
const FRAME: u128 = 3;

/// The window's tree with the page node grafting the web view `U`, which grafts the document `D`,
/// which has an iframe grafting `F`: the nesting a page has in the browser.
fn page_with_graft() -> Consumer {
    let window = update(
        TreeId::ROOT,
        vec![
            (0, node(Role::Window, &[1, 2])),
            (1, labelled(Role::Navigation, "Toolbar")),
            (2, {
                let mut page = graft(Role::Main, tree_id(WEB_VIEW));
                page.set_label("Page content");
                page
            }),
        ],
    );
    let web_view = update(
        tree_id(WEB_VIEW),
        vec![
            (0, node(Role::ScrollView, &[1])),
            (1, graft(Role::GenericContainer, tree_id(DOCUMENT))),
        ],
    );
    let mut heading = labelled(Role::Heading, "Welcome");
    heading.set_level(1);
    let mut text = Node::new(Role::TextRun);
    text.set_value("Hello, world.");
    let mut link = labelled(Role::Link, "Docs");
    link.set_url("https://example.com/docs");
    link.add_action(Action::Click);
    link.add_action(Action::Focus);
    let mut name = labelled(Role::TextInput, "Name");
    name.set_required();
    name.set_value("Ada");
    name.set_placeholder("Your name");
    name.set_description("As on your passport");
    let mut remember = labelled(Role::CheckBox, "Remember me");
    remember.set_toggled(Toggled::False);
    let mut menu = labelled(Role::Button, "Menu");
    menu.set_expanded(false);
    menu.set_has_popup(HasPopup::Menu);
    let mut hidden = labelled(Role::GenericContainer, "Never read");
    hidden.set_hidden();
    hidden.set_children(vec![NodeId(10)]);
    let mut secret = Node::new(Role::TextRun);
    secret.set_value("under a hidden parent");
    let mut frame = labelled(Role::Iframe, "Embedded");
    frame.set_tree_id(tree_id(FRAME));
    let document = update(
        tree_id(DOCUMENT),
        vec![
            (0, node(Role::RootWebArea, &[1, 2, 4, 5, 6, 7, 8, 9])),
            (1, heading),
            (2, node(Role::Paragraph, &[3])),
            (3, text),
            (4, link),
            (5, name),
            (6, remember),
            (7, menu),
            (8, hidden),
            (9, frame),
            (10, secret),
        ],
    );
    let mut inner = labelled(Role::Heading, "In the frame");
    inner.set_level(2);
    let frame_tree = update(
        tree_id(FRAME),
        vec![(0, node(Role::RootWebArea, &[1])), (1, inner)],
    );
    consumer(vec![window, web_view, document, frame_tree])
}

#[test]
fn snapshot_prints_one_indented_line_per_node() {
    let dump = dump(&page_with_graft(), &DumpOptions::default());
    let expected = r#"window
  navigation "Toolbar"
  graft "Page content"
    scroll-view
      graft
        root-web-area
          heading "Welcome" [level 1]
          paragraph
            text-run value="Hello, world."
          link "Docs" url="https://example.com/docs"
          text-input "Name" [required] value="Ada" desc="As on your passport" placeholder="Your name"
          check-box "Remember me" [unchecked]
          button "Menu" [collapsed, has-popup=menu]
          graft "Embedded"
            root-web-area
              heading "In the frame" [level 2]
"#;
    assert_eq!(dump, expected);
}

#[test]
fn snapshot_leaves_hidden_subtrees_out_unless_asked() {
    let tree = page_with_graft();
    let plain = dump(&tree, &DumpOptions::default());
    assert!(!plain.contains("Never read"));
    assert!(!plain.contains("under a hidden parent"));
    let with_hidden = dump(&tree, &DumpOptions::default().with_hidden());
    assert!(with_hidden.contains("generic-container \"Never read\" [hidden]"));
    assert!(with_hidden.contains("text-run value=\"under a hidden parent\""));
}

#[test]
fn snapshot_never_prints_a_tree_id_or_a_node_id() {
    let text = dump(&page_with_graft(), &DumpOptions::default().with_actions());
    for needle in ["0000", "Uuid", "NodeId", "#1", "tree"] {
        assert!(!text.contains(needle), "{needle:?} in:\n{text}");
    }
}

#[test]
fn snapshot_is_deterministic_and_ends_with_a_newline() {
    let tree = page_with_graft();
    let once = dump(&tree, &DumpOptions::default());
    assert_eq!(once, dump(&tree, &DumpOptions::default()));
    assert!(once.ends_with('\n'));
    assert!(!once.ends_with("\n\n"));
}

#[test]
fn snapshot_graft_without_its_subtree_has_no_children() {
    // The window's graft arrives before the web view's own tree: the order the adapter allows.
    let window = update(
        TreeId::ROOT,
        vec![
            (0, node(Role::Window, &[1])),
            (1, graft(Role::Main, tree_id(WEB_VIEW))),
        ],
    );
    let tree = consumer(vec![window]);
    assert_eq!(dump(&tree, &DumpOptions::default()), "window\n  graft\n");
}

#[test]
fn snapshot_role_names_are_lower_case_words() {
    let cases = [
        (Role::TextInput, "text-input"),
        (Role::ListBoxOption, "list-box-option"),
        (Role::RootWebArea, "root-web-area"),
        (Role::PdfRoot, "pdf-root"),
        (Role::DocBiblioRef, "doc-biblio-ref"),
        (Role::Heading, "heading"),
        (Role::TextRun, "text-run"),
        (Role::UrlInput, "url-input"),
        (Role::MenuItemCheckBox, "menu-item-check-box"),
        (Role::GenericContainer, "generic-container"),
    ];
    for (role, name) in cases {
        assert_eq!(role_name(role), name);
    }
}

#[test]
fn snapshot_strings_are_quoted_and_escaped_onto_one_line() {
    let mut weird = labelled(Role::Button, "He said \"hi\"\nand \\ left\t\u{7}");
    weird.set_value("line1\r\nline2");
    weird.set_description("tab\there");
    let tree = consumer(vec![update(
        TreeId::ROOT,
        vec![(0, node(Role::Window, &[1])), (1, weird)],
    )]);
    let text = dump(&tree, &DumpOptions::default());
    let line = text.lines().nth(1).expect("the button line");
    assert_eq!(
        line,
        r#"  button "He said \"hi\"\nand \\ left\t\u{7}" value="line1\r\nline2" desc="tab\there""#
    );
    assert_eq!(text.lines().count(), 2);
}

#[test]
fn snapshot_states_have_a_fixed_order_and_words() {
    let mut combo = labelled(Role::ComboBox, "Fruit");
    combo.set_disabled();
    combo.set_read_only();
    combo.set_required();
    combo.set_busy();
    combo.set_modal();
    combo.set_multiselectable();
    combo.set_toggled(Toggled::Mixed);
    combo.set_expanded(true);
    combo.set_selected(false);
    combo.set_invalid(Invalid::Spelling);
    combo.set_has_popup(HasPopup::Listbox);
    combo.set_aria_current(AriaCurrent::Page);
    combo.set_orientation(Orientation::Vertical);
    combo.set_level(3);
    combo.set_position_in_set(2);
    combo.set_size_of_set(9);
    combo.set_numeric_value(5.5);
    combo.set_min_numeric_value(0.0);
    combo.set_max_numeric_value(10.0);
    combo.set_numeric_value_step(0.5);
    let mut pressed = labelled(Role::Button, "Bold");
    pressed.set_toggled(Toggled::True);
    let mut unpressed = labelled(Role::Button, "Italic");
    unpressed.set_toggled(Toggled::False);
    let tree = consumer(vec![update(
        TreeId::ROOT,
        vec![
            (0, node(Role::Window, &[1, 2, 3])),
            (1, combo),
            (2, pressed),
            (3, unpressed),
        ],
    )]);
    let text = dump(&tree, &DumpOptions::default());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[1],
        "  combo-box \"Fruit\" [disabled, read-only, required, busy, modal, multiselectable, mixed, expanded, unselected, invalid=spelling, has-popup=listbox, current=page, vertical, level 3, item 2 of 9, now 5.5, min 0, max 10, step 0.5]"
    );
    assert_eq!(lines[2], "  button \"Bold\" [pressed]");
    assert_eq!(lines[3], "  button \"Italic\" [unpressed]");
}

#[test]
fn snapshot_bounds_are_rounded_to_a_pixel_and_off_by_default() {
    let mut button = labelled(Role::Button, "Go");
    button.set_bounds(Rect {
        x0: 10.4,
        y0: 20.5,
        x1: 110.49,
        y1: 50.6,
    });
    let mut window = node(Role::Window, &[1]);
    window.set_transform(Affine::translate((100.0, 0.0)));
    let tree = consumer(vec![update(TreeId::ROOT, vec![(0, window), (1, button)])]);
    let off = dump(&tree, &DumpOptions::default());
    assert!(!off.contains("bounds"));
    let on = dump(&tree, &DumpOptions::default().with_bounds());
    // The parent's transform composes into the node's box.
    assert!(
        on.contains("button \"Go\" bounds=110,21,210,51"),
        "dump:\n{on}"
    );
}

#[test]
fn snapshot_actions_are_listed_in_accesskit_order_when_asked() {
    let mut button = labelled(Role::Button, "Go");
    button.add_action(Action::ShowContextMenu);
    button.add_action(Action::Focus);
    button.add_action(Action::Click);
    button.add_action(Action::ScrollIntoView);
    let tree = consumer(vec![update(
        TreeId::ROOT,
        vec![(0, node(Role::Window, &[1])), (1, button)],
    )]);
    assert!(!dump(&tree, &DumpOptions::default()).contains("actions"));
    let text = dump(&tree, &DumpOptions::default().with_actions());
    assert!(
        text.contains("button \"Go\" actions=click,focus,scroll-into-view,show-context-menu"),
        "dump:\n{text}"
    );
}

#[test]
fn snapshot_a_very_deep_tree_does_not_overflow_the_stack() {
    let depth = 4_000u64;
    // The consumer is recursive: build the tree on a thread with a big stack, then dump it on one with
    // a small stack, where a recursive dump would overflow long before the last level.
    let text = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let mut nodes = Vec::new();
            for id in 0..depth {
                let role = if id == 0 {
                    Role::Window
                } else {
                    Role::GenericContainer
                };
                let children: &[u64] = if id + 1 < depth { &[id + 1] } else { &[] };
                nodes.push((id, node(role, children)));
            }
            let tree = consumer(vec![update(TreeId::ROOT, nodes)]);
            std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .stack_size(128 << 10)
                    .spawn_scoped(scope, || dump(&tree, &DumpOptions::default()))
                    .expect("spawn the dump thread")
                    .join()
                    .expect("the dump does not overflow its small stack")
            })
        })
        .expect("spawn the tree thread")
        .join()
        .expect("the tree is built");
    assert_eq!(text.lines().count(), depth as usize);
    let last = text.lines().last().expect("a last line");
    assert!(last.starts_with(&" ".repeat(2 * (depth as usize - 1))));
}

#[test]
fn snapshot_describe_is_the_line_without_indentation() {
    let tree = page_with_graft();
    let root = tree.state().root();
    assert_eq!(describe(&root, &DumpOptions::default()), "window");
    let first = dump(&tree, &DumpOptions::default());
    assert!(first.starts_with("window\n"));
}

// ---------------------------------------------------------------------------------------------
// golden files
// ---------------------------------------------------------------------------------------------

#[test]
fn golden_page_with_graft_matches_the_committed_file() {
    assert_golden(
        "page-with-graft",
        &dump(&page_with_graft(), &DumpOptions::default()),
    );
}

#[test]
fn golden_page_with_graft_with_actions_and_hidden_matches_the_committed_file() {
    assert_golden(
        "page-with-graft-all",
        &dump(
            &page_with_graft(),
            &DumpOptions::default().with_actions().with_hidden(),
        ),
    );
}

static FOLDER: AtomicUsize = AtomicUsize::new(0);

/// A fresh scratch folder under `$TMPDIR/opencode/lanes/a11y-1/`, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let number = FOLDER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir()
            .join("opencode")
            .join("lanes")
            .join("a11y-1")
            .join(format!("golden-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const COMMAND: &str = "MERCURY_A11Y_BLESS=1 cargo test --example";

/// What a failed check says (it panics).
fn failure_of(check: impl FnOnce()) -> String {
    let payload = catch_unwind(AssertUnwindSafe(check)).expect_err("the check should fail");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
        .unwrap_or_default()
}

#[test]
fn golden_blessing_writes_the_header_and_comparing_ignores_it() {
    let scratch = Scratch::new();
    check_golden(&scratch.0, "tree", "root\n  child\n", true, COMMAND);
    let written = std::fs::read_to_string(scratch.0.join("tree.txt")).expect("written file");
    let lines: Vec<&str> = written.lines().collect();
    assert_eq!(
        lines[0],
        "# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0"
    );
    assert_eq!(
        lines[1],
        "# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors"
    );
    assert_eq!(lines[2], format!("# Rewrite with: {COMMAND}"));
    assert_eq!(lines.len(), 5);
    assert_eq!(without_header(&written), "root\n  child\n");
    // The same text compares equal; a header edited by hand does not matter.
    check_golden(&scratch.0, "tree", "root\n  child\n", false, COMMAND);
    std::fs::write(
        scratch.0.join("tree.txt"),
        "# anything\n#\n# at all\nroot\n  child\n",
    )
    .expect("edit");
    check_golden(&scratch.0, "tree", "root\n  child\n", false, COMMAND);
}

#[test]
fn golden_header_is_three_comment_lines() {
    let text = header(COMMAND);
    assert_eq!(text.lines().count(), 3);
    assert!(text.lines().all(|line| line.starts_with("# ")));
    assert!(text.contains("PolyForm-Noncommercial-1.0.0"));
    assert!(text.contains(COMMAND));
    assert_eq!(golden::BLESS_VARIABLE, "MERCURY_A11Y_BLESS");
}

#[test]
fn golden_a_missing_file_fails_and_says_how_to_create_it() {
    let scratch = Scratch::new();
    let message = failure_of(|| check_golden(&scratch.0, "absent", "root\n", false, COMMAND));
    assert!(message.contains("no golden file"), "{message}");
    assert!(message.contains(COMMAND), "{message}");
    assert!(
        message.contains("root"),
        "the text that would be written: {message}"
    );
    // Nothing was written.
    assert!(!scratch.0.join("absent.txt").exists());
}

#[test]
fn golden_a_difference_fails_with_a_line_diff() {
    let scratch = Scratch::new();
    check_golden(&scratch.0, "tree", "a\nb\nc\n", true, COMMAND);
    let message = failure_of(|| check_golden(&scratch.0, "tree", "a\nB\nc\nd\n", false, COMMAND));
    assert!(message.contains("changed"), "{message}");
    assert!(message.contains("- b"), "{message}");
    assert!(message.contains("+ B"), "{message}");
    assert!(message.contains("+ d"), "{message}");
    assert!(message.contains("  a"), "{message}");
    assert!(message.contains(COMMAND), "{message}");
    // The file was not touched by the failure.
    let kept = std::fs::read_to_string(scratch.0.join("tree.txt")).expect("kept file");
    assert_eq!(without_header(&kept), "a\nb\nc\n");
}

#[test]
fn golden_names_are_words_joined_by_hyphens() {
    let scratch = Scratch::new();
    for bad in [
        "",
        "Has Space",
        "../escape",
        "a/b",
        "dot.txt",
        "under_score",
    ] {
        let message = failure_of(|| check_golden(&scratch.0, bad, "x\n", true, COMMAND));
        assert!(
            message.contains("words joined by hyphens"),
            "{bad:?}: {message}"
        );
    }
    check_golden(&scratch.0, "fine-name-2", "x\n", true, COMMAND);
    assert!(scratch.0.join("fine-name-2.txt").exists());
}

#[test]
fn golden_windows_line_ends_compare_equal() {
    let scratch = Scratch::new();
    std::fs::create_dir_all(&scratch.0).expect("folder");
    std::fs::write(
        scratch.0.join("crlf.txt"),
        "# header\r\nroot\r\n  child\r\n",
    )
    .expect("file");
    check_golden(&scratch.0, "crlf", "root\n  child\n", false, COMMAND);
}

#[test]
fn golden_a_very_long_difference_still_reports_the_first_one() {
    let scratch = Scratch::new();
    let long: String = (0..3000).map(|n| format!("line {n}\n")).collect();
    check_golden(&scratch.0, "long", &long, true, COMMAND);
    let changed = long.replacen("line 1500\n", "line 1500 changed\n", 1);
    let message = failure_of(|| check_golden(&scratch.0, "long", &changed, false, COMMAND));
    assert!(message.contains("- line 1500"), "{message}");
    assert!(message.contains("+ line 1500 changed"), "{message}");
}
