// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! Tests of the accessibility helper `common/a11y.rs` itself: reading order, hidden children,
//! the labelling rule (with failing inputs), the golden-file round trip and the bounds helper.
//!
//! They run on `MainWindow` and on nodes built by hand from one of its elements. A small
//! `slint::slint!` component would be simpler, but a `slint!` macro in a test crate carries no
//! debug info (it is switched on by the `SLINT_EMIT_DEBUG_INFO` environment variable, which
//! only `mercury-ui`'s build script sets, through the `test-support` feature), so the
//! `ElementHandle` queries would see nothing in it. Checked: a probe component yielded no nodes.

mod common;

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use common::a11y::{
    self, Exempt, NEEDS_LABEL, Node, Rect, assert_bounds, assert_labelled, assert_labelled_except,
    check_snapshot, control, control_in, dump, reading_order, subtree,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use mercury_ui::MainWindow;

fn main_window() -> MainWindow {
    a11y::init();
    let window = MainWindow::new().unwrap();
    a11y::open_window(&window, 1180.0, 760.0);
    window
}

/// A node made by hand on top of a real handle (the handle only matters for geometry, and for
/// the properties the helper reads from it, which the window's first element leaves unset).
fn made_up(handle: &ElementHandle, role: AccessibleRole, label: &str) -> Node {
    Node {
        role,
        label: label.to_string(),
        description: String::new(),
        value: String::new(),
        enabled: true,
        checked: None,
        selected: None,
        item: None,
        handle: handle.clone(),
    }
}

/// The text of a panic raised by `work`, or `None` when it did not panic.
fn panic_message(work: impl FnOnce()) -> Option<String> {
    let payload = catch_unwind(AssertUnwindSafe(work)).err()?;
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
        .unwrap_or_default();
    Some(message)
}

fn assert_panics_with(work: impl FnOnce(), needle: &str) {
    let message =
        panic_message(work).unwrap_or_else(|| panic!("expected a panic about {needle:?}"));
    assert!(
        message.contains(needle),
        "the panic does not mention {needle:?}:\n{message}"
    );
}

#[test]
fn init_may_be_called_twice_on_one_thread() {
    a11y::init();
    a11y::init();
    let window = MainWindow::new().unwrap();
    a11y::open_window(&window, 1180.0, 760.0);
    assert_eq!(a11y::window_size(&window), (1180.0, 760.0));
}

#[test]
fn reading_order_is_the_item_tree_order() {
    let window = main_window();
    let nodes = reading_order(&window);
    // The three captions of the new-tab page, which the lane that labels the window leaves as
    // plain text, come in the order they are laid out, top to bottom.
    let captions: Vec<&str> = nodes
        .iter()
        .filter(|node| node.role == AccessibleRole::Text)
        .map(|node| node.label.as_str())
        .filter(|label| {
            [
                "Mercury Browser",
                "Fast, lightweight and private by default",
                "Type an address",
            ]
            .iter()
            .any(|caption| label.starts_with(caption))
        })
        .collect();
    assert_eq!(
        captions,
        [
            "Mercury Browser",
            "Fast, lightweight and private by default",
            "Type an address or search with DuckDuckGo."
        ]
    );
    // The strip of tabs comes before the page, and no node is listed twice.
    let first_tab_list = nodes
        .iter()
        .position(|node| node.role == AccessibleRole::TabList);
    let title = nodes
        .iter()
        .position(|node| node.label == "Mercury Browser");
    assert!(first_tab_list.unwrap() < title.unwrap());
    // Reading twice gives the same text: the dump holds nothing that changes from run to run.
    assert_eq!(dump(&nodes), dump(&reading_order(&window)));
}

#[test]
fn a_hidden_child_is_left_out() {
    let window = main_window();
    window.set_is_new_tab(false);
    window.set_web_view_visible(true);
    window.set_zoom_percent(125);
    let shown = reading_order(&window);
    assert!(
        shown.iter().any(|node| node.label.contains("125")),
        "the zoom badge is not listed:\n{}",
        dump(&shown)
    );
    assert!(
        shown
            .iter()
            .any(|node| node.role == AccessibleRole::TabList)
    );

    // `chrome-hidden` sets `visible: false` on the strip and the toolbar: nothing in them is read.
    window.set_chrome_hidden(true);
    let hidden = reading_order(&window);
    assert!(
        hidden.iter().all(|node| !node.label.contains("125")),
        "the zoom badge is read although the toolbar is hidden:\n{}",
        dump(&hidden)
    );
    assert!(
        hidden
            .iter()
            .all(|node| node.role != AccessibleRole::TabList)
    );
    assert!(hidden.len() < shown.len());
}

#[test]
fn a_subtree_is_the_element_then_its_descendants() {
    let window = main_window();
    let nodes = reading_order(&window);
    let strip = nodes
        .iter()
        .find(|node| node.role == AccessibleRole::TabList)
        .unwrap();
    let below = subtree(&strip.handle);
    assert_eq!(below[0].role, AccessibleRole::TabList);
    assert_eq!(below[0].label, strip.label);
    let strip_rect = strip.rect();
    for node in &below[1..] {
        assert!(
            strip_rect.contains(&node.rect()),
            "{} lies outside its tab strip {strip_rect:?}",
            node.label
        );
    }
    // The subtree is the strip's part of the window's order, in the same order.
    let at = nodes
        .iter()
        .position(|node| node.role == AccessibleRole::TabList)
        .unwrap();
    let from_window: Vec<String> = nodes[at..at + below.len()]
        .iter()
        .map(|n| n.label.clone())
        .collect();
    let from_subtree: Vec<String> = below.iter().map(|n| n.label.clone()).collect();
    assert_eq!(from_window, from_subtree);
}

#[test]
fn control_finds_exactly_one_element() {
    let window = main_window();
    let found = control(&window, AccessibleRole::Text, "Mercury Browser");
    assert_eq!(found.accessible_label().unwrap(), "Mercury Browser");

    assert_panics_with(
        || {
            control(&window, AccessibleRole::Button, "No such button");
        },
        "no button labelled \"No such button\"",
    );
    // The failure lists what the window has under that role and label, to fix a wrong guess.
    assert_panics_with(
        || {
            control(&window, AccessibleRole::Button, "Mercury Browser");
        },
        "text | Mercury Browser",
    );

    let nodes = reading_order(&window);
    let handle = nodes[0].handle.clone();
    let twice = vec![
        made_up(&handle, AccessibleRole::Button, "Twice"),
        made_up(&handle, AccessibleRole::Button, "Twice"),
    ];
    assert_panics_with(
        || {
            control_in(&twice, AccessibleRole::Button, "Twice");
        },
        "2 button labelled \"Twice\", expected exactly one",
    );
}

#[test]
fn the_labelling_rule_passes_labelled_controls() {
    let window = main_window();
    let handle = reading_order(&window)[0].handle.clone();
    let good = vec![
        made_up(&handle, AccessibleRole::Navigation, "Toolbar"),
        made_up(&handle, AccessibleRole::Button, "Back"),
        made_up(&handle, AccessibleRole::Button, "Forward"),
        made_up(&handle, AccessibleRole::Search, "Address bar"),
        made_up(&handle, AccessibleRole::TextInput, "Address and search"),
        made_up(&handle, AccessibleRole::Text, "Same words"),
        made_up(&handle, AccessibleRole::Text, "Same words"),
        // The same role and label are fine when the rows have an index.
        Node {
            item: Some((0, 2)),
            ..made_up(&handle, AccessibleRole::Tab, "Page")
        },
        Node {
            item: Some((1, 2)),
            ..made_up(&handle, AccessibleRole::Tab, "Page")
        },
        // The same label under another role is another control.
        made_up(&handle, AccessibleRole::Region, "Back"),
    ];
    assert_labelled(&good, "good nodes");
    assert_labelled(&[], "no nodes");
}

#[test]
fn the_labelling_rule_fails_what_it_should() {
    let window = main_window();
    let handle = reading_order(&window)[0].handle.clone();
    let one = |role, label: &str| vec![made_up(&handle, role, label)];

    for role in NEEDS_LABEL {
        assert_panics_with(
            || assert_labelled(&one(*role, ""), "unlabelled"),
            &format!("has no label: {} |", a11y::role_name(*role)),
        );
    }
    assert_panics_with(
        || assert_labelled(&one(AccessibleRole::TextInput, ""), "empty field"),
        "a text field with neither label nor placeholder",
    );
    assert_panics_with(
        || assert_labelled(&one(AccessibleRole::Image, "Logo"), "image"),
        "an image is exposed",
    );
    for glyph in ["‹", "›", "↻", "⌂", "⋮", "×", "+", "→", "✓", "..."] {
        assert_panics_with(
            || assert_labelled(&one(AccessibleRole::Button, glyph), "glyph"),
            "the label is only a glyph or punctuation",
        );
    }
    assert_panics_with(
        || assert_labelled(&one(AccessibleRole::Text, "→"), "glyph text"),
        "the label is only a glyph or punctuation",
    );
    let twins = vec![
        made_up(&handle, AccessibleRole::Button, "Retry"),
        made_up(&handle, AccessibleRole::Button, "Retry"),
    ];
    assert_panics_with(
        || assert_labelled(&twins, "twins"),
        "another button has the same label",
    );
    // The failure names the state under test and prints the reading order.
    assert_panics_with(
        || assert_labelled(&twins, "the Retry state"),
        "the Retry state",
    );
    assert_panics_with(
        || assert_labelled(&twins, "twins"),
        "reading order:\nbutton | Retry |",
    );
}

#[test]
fn an_exemption_needs_a_reason_and_a_match() {
    let window = main_window();
    let handle = reading_order(&window)[0].handle.clone();
    let unlabelled = vec![made_up(&handle, AccessibleRole::Button, "")];
    let exempt = Exempt {
        role: AccessibleRole::Button,
        label: "",
        reason: "a test of the exemption list",
    };
    assert_labelled_except(&unlabelled, "exempt", &[exempt]);

    // An exemption that matches nothing is itself a failure: the list cannot go stale.
    let labelled = vec![made_up(&handle, AccessibleRole::Button, "Back")];
    assert_panics_with(
        || {
            assert_labelled_except(
                &labelled,
                "stale",
                &[Exempt {
                    role: AccessibleRole::Button,
                    label: "",
                    reason: "no longer true",
                }],
            )
        },
        "matches nothing: remove it",
    );
    assert_panics_with(
        || {
            assert_labelled_except(
                &unlabelled,
                "no reason",
                &[Exempt {
                    role: AccessibleRole::Button,
                    label: "",
                    reason: "",
                }],
            )
        },
        "an exemption needs a reason",
    );
}

#[test]
fn the_dump_is_one_line_per_node_with_four_columns() {
    let window = main_window();
    let handle = reading_order(&window)[0].handle.clone();
    let nodes = vec![
        Node {
            enabled: false,
            ..made_up(&handle, AccessibleRole::Button, "Back")
        },
        Node {
            description: "Pinned".to_string(),
            selected: Some(true),
            item: Some((0, 3)),
            ..made_up(&handle, AccessibleRole::Tab, "Docs")
        },
        Node {
            checked: Some(false),
            ..made_up(&handle, AccessibleRole::Checkbox, "Cookies")
        },
        Node {
            value: "abc".to_string(),
            ..made_up(&handle, AccessibleRole::TextInput, "Address")
        },
        made_up(&handle, AccessibleRole::Region, "Two\nlines"),
        made_up(&handle, AccessibleRole::Main, ""),
    ];
    assert_eq!(
        dump(&nodes),
        "button | Back | | disabled\n\
         tab | Docs | Pinned | selected, item 0 of 3\n\
         checkbox | Cookies | | unchecked\n\
         text-input | Address | | value \"abc\"\n\
         region | Two\\nlines | |\n\
         main | | |\n"
    );
}

/// A scratch folder that is removed when the test ends, even when it fails.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mercury-a11y-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_golden_file_is_blessed_compared_and_diffed() {
    let scratch = Scratch::new("snapshot");
    let dir = &scratch.0;
    let text = "button | Back | |\ntext | Hello | |\n";

    // No file yet: the failure says how to create it and what it would hold.
    assert_panics_with(
        || check_snapshot(dir, "round-trip", text, false),
        "Create it with: MERCURY_A11Y_BLESS=1 cargo test --release -p mercury-ui --features test-support --test 'a11y_*'",
    );
    assert_panics_with(
        || check_snapshot(dir, "round-trip", text, false),
        "text | Hello | |",
    );

    // Blessing writes the header (the two SPDX lines, a description, the bless command) and the dump.
    check_snapshot(dir, "round-trip", text, true);
    let written = fs::read_to_string(dir.join("round-trip.txt")).unwrap();
    let lines: Vec<&str> = written.lines().collect();
    assert_eq!(
        lines[0],
        "# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0"
    );
    assert_eq!(
        lines[1],
        "# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors"
    );
    assert!(lines[2].starts_with("# "));
    assert!(lines[3].starts_with("# Rewrite with: MERCURY_A11Y_BLESS=1 "));
    assert_eq!(&lines[4..], ["button | Back | |", "text | Hello | |"]);

    // Comparing passes, and ignores the leading `# ` lines whatever they say.
    check_snapshot(dir, "round-trip", text, false);
    let edited_header = written.replace("# Rewrite with", "# Changed comment, bless with");
    fs::write(dir.join("round-trip.txt"), edited_header).unwrap();
    check_snapshot(dir, "round-trip", text, false);

    // A changed word or a changed order fails with a diff.
    let message = panic_message(|| {
        check_snapshot(
            dir,
            "round-trip",
            "button | Go back | |\ntext | Hello | |\n",
            false,
        )
    })
    .expect("a changed label fails");
    assert!(message.contains("- button | Back | |"), "{message}");
    assert!(message.contains("+ button | Go back | |"), "{message}");
    assert!(message.contains("  text | Hello | |"), "{message}");
    assert!(message.contains("MERCURY_A11Y_BLESS=1"), "{message}");
    assert_panics_with(
        || {
            check_snapshot(
                dir,
                "round-trip",
                "text | Hello | |\nbutton | Back | |\n",
                false,
            )
        },
        "the reading order of round-trip changed",
    );
    // A line with `#` in the middle of the dump is not a comment.
    check_snapshot(dir, "hash", "text | Issue #1 | |\n", true);
    check_snapshot(dir, "hash", "text | Issue #1 | |\n", false);

    // Blessing again overwrites.
    check_snapshot(dir, "round-trip", "text | New | |\n", true);
    check_snapshot(dir, "round-trip", "text | New | |\n", false);

    // A name is plain words.
    assert_panics_with(
        || check_snapshot(dir, "../escape", text, true),
        "lower-case words",
    );
}

#[test]
fn the_real_golden_folder_is_the_crates_tests_folder() {
    let dir = a11y::snapshot_dir();
    assert!(
        dir.ends_with("crates/mercury-ui/tests/a11y-snapshots"),
        "{}",
        dir.display()
    );
}

#[test]
fn rectangles_contain_and_compare() {
    let outer = Rect::new(10.0, 20.0, 100.0, 50.0);
    assert!(outer.contains(&Rect::new(10.0, 20.0, 100.0, 50.0)));
    assert!(outer.contains(&Rect::new(20.0, 30.0, 10.0, 10.0)));
    assert!(!outer.contains(&Rect::new(9.0, 20.0, 10.0, 10.0)));
    assert!(!outer.contains(&Rect::new(20.0, 30.0, 100.0, 10.0)));
    assert!(!outer.contains(&Rect::new(20.0, 65.0, 10.0, 10.0)));
    assert_eq!((outer.right(), outer.bottom()), (110.0, 70.0));
    assert!(outer.same_as(&Rect::new(10.004, 20.0, 100.0, 50.0)));
    assert!(!outer.same_as(&Rect::new(10.5, 20.0, 100.0, 50.0)));
}

#[test]
fn bounds_pass_inside_the_window_and_fail_outside() {
    let window = main_window();
    let nodes = reading_order(&window);
    let title: Vec<Node> = nodes
        .iter()
        .filter(|n| n.label == "Mercury Browser")
        .cloned()
        .collect();
    assert_eq!(title.len(), 1);
    assert_bounds(&title, a11y::window_size(&window));
    // The title is laid out at x 300, y 372, 580 wide: a smaller window does not hold it.
    assert_panics_with(
        || assert_bounds(&title, (400.0, 400.0)),
        "outside the 400 x 400 window",
    );
    assert_panics_with(
        || assert_bounds(&title, (400.0, 400.0)),
        "text | Mercury Browser",
    );
    // `assert_inside` and `assert_rect` read the same rectangle.
    let strip = nodes
        .iter()
        .find(|n| n.role == AccessibleRole::TabList)
        .unwrap();
    a11y::assert_rect(strip, strip.rect());
    assert_panics_with(
        || a11y::assert_rect(strip, Rect::new(0.0, 0.0, 1.0, 1.0)),
        "expected Rect",
    );
    assert_panics_with(|| a11y::assert_inside(&title[0], strip), "is not inside");
}
