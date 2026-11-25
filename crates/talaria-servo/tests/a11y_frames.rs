// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Iframes in the accessibility tree (a11y-1, item 5, EE7).
//!
//! A document inside an `<iframe>` is a tree of its own (`TreeId::from(pipeline_id)`) that the
//! iframe's node grafts (decision D11). These tests load pages with frames in a headless Servo
//! through the harness of item 3 (`tests/common/a11y_harness.rs`) and check what a screen reader
//! would reach: the content of the frame is in the tree, an action on it reaches the document
//! inside the frame, and a document that is gone (a frame that is removed, a top-level page that
//! is navigated away from) leaves nothing behind, in particular no child that is grafted as the
//! page. The engine side: `third_party/servo-layout/accessibility/frames.rs` (the graft),
//! `third_party/servo-constellation/mercury/a11y.rs` (activation of the child documents) and
//! `third_party/servo/webview.rs` (a child's update is never taken for a new page).
//!
//! **Every test that starts Servo is named `headless_servo_<what>` and is `#[ignore]`d**: it
//! needs a GL context (`make test-a11y-pages`, the macOS CI job).
//!
//! The pages that come over HTTP are served by `scripts/serve-tests.sh` (the `serve-tests` binary of
//! `crates/mercury-devtools`), started once per origin: two instances are two loopback ports and so
//! two origins, which is what the cross-origin test needs. The harness loads a page from its HTML,
//! so a test that needs a URL loads a one-line document that goes there.

mod common;

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

use accesskit::{Action, Role};
use accesskit_consumer::NodeId;
use common::a11y_harness::A11yPage;

const SRCDOC_PAGE: &str = include_str!("../../../tests/pages/a11y/frames-srcdoc.html");

/// A `serve-tests` server on a loopback port, stopped when it is dropped.
struct TestServer {
    child: Child,
    port: u16,
    /// Kept open so that nothing the server writes later reaches a closed pipe.
    _stdout: BufReader<ChildStdout>,
}

impl TestServer {
    fn start() -> TestServer {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/serve-tests.sh");
        let mut child = Command::new("sh")
            .arg(&script)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("scripts/serve-tests.sh starts");
        let mut stdout = BufReader::new(child.stdout.take().expect("a piped standard output"));
        let mut first_line = String::new();
        stdout
            .read_line(&mut first_line)
            .expect("serve-tests prints its port");
        let port = first_line
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("the first line of serve-tests is a port: {first_line:?}"));
        TestServer {
            child,
            port,
            _stdout: stdout,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// A page of `tests/pages/a11y/`.
    fn page(&self, name: &str) -> String {
        self.url(&format!("/pages/a11y/{name}"))
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Load `url` in a headless Servo: the harness takes HTML, so the document it loads goes to the
/// URL, and the function returns once the page is there.
fn open(url: &str) -> A11yPage {
    let html = format!(
        "<!doctype html><title>Loading</title><p>Loading</p><script>location.replace({url:?})</script>"
    );
    let mut page = A11yPage::load(&html);
    page.spin_until(|page| js_string(page, "location.href") == url);
    page
}

fn js_string(page: &A11yPage, script: &str) -> String {
    match page.eval(script) {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    }
}

/// How many nodes with this role and accessible name the tree has, grafted subtrees included.
fn count_nodes(page: &A11yPage, role: Role, name: &str) -> usize {
    fn walk(node: accesskit_consumer::Node<'_>, role: Role, name: &str) -> usize {
        let own = usize::from(node.role() == role && node.label().as_deref() == Some(name));
        own + node
            .children()
            .map(|child| walk(child, role, name))
            .sum::<usize>()
    }
    walk(page.tree().state().root(), role, name)
}

/// Every node with this accessible name, whatever its role.
fn nodes_named(page: &A11yPage, name: &str) -> Vec<(NodeId, Role)> {
    fn walk(node: accesskit_consumer::Node<'_>, name: &str, found: &mut Vec<(NodeId, Role)>) {
        if node.label().as_deref() == Some(name) {
            found.push((node.id(), node.role()));
        }
        for child in node.children() {
            walk(child, name, found);
        }
    }
    let mut found = Vec::new();
    walk(page.tree().state().root(), name, &mut found);
    found
}

fn wait_for_heading(page: &mut A11yPage, name: &str) {
    page.spin_until(|page| count_nodes(page, Role::Heading, name) >= 1);
}

fn messages(page: &A11yPage) -> String {
    js_string(page, "window.messages.join(',')")
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_a_same_origin_iframe_is_part_of_the_tree() {
    let server = TestServer::start();
    let mut page = open(&server.page("frames-parent.html"));

    wait_for_heading(&mut page, "Child page");
    assert_eq!(count_nodes(&page, Role::Heading, "Parent page"), 1);
    assert_eq!(count_nodes(&page, Role::Heading, "Child page"), 1);
    assert_eq!(
        nodes_named(&page, "Same-origin frame").len(),
        1,
        "the iframe's own node is the graft and keeps its title as its name (D11)"
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_an_action_on_a_node_inside_an_iframe_reaches_the_frame() {
    let server = TestServer::start();
    let mut page = open(&server.page("frames-parent.html"));
    wait_for_heading(&mut page, "Child page");

    let button = page.find(Role::Button, "Child button");
    page.act(button, Action::Click, None);
    page.spin_until(|page| messages(page) == "child-pressed");

    assert_eq!(
        js_string(
            &page,
            "String(document.getElementById('frame').contentWindow.pressed)"
        ),
        "1"
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_a_srcdoc_iframe_is_part_of_the_tree() {
    let mut page = A11yPage::load(SRCDOC_PAGE);

    wait_for_heading(&mut page, "Srcdoc page");
    assert_eq!(count_nodes(&page, Role::Heading, "Srcdoc parent"), 1);
    assert_eq!(count_nodes(&page, Role::Heading, "Srcdoc page"), 1);

    // The frame is same-origin with its parent: the test hears the click through the parent.
    page.eval(
        "(() => {
            const frame = document.getElementById('frame');
            frame.contentDocument.querySelector('button').addEventListener('click', () => {
                window.srcdocClicks = (window.srcdocClicks || 0) + 1;
            });
            return 1;
        })()",
    );
    let button = page.find(Role::Button, "Srcdoc button");
    page.act(button, Action::Click, None);
    page.spin_until(|page| js_string(page, "String(window.srcdocClicks)") == "1");
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_a_cross_origin_iframe_is_part_of_the_tree() {
    let parent_server = TestServer::start();
    let child_server = TestServer::start();
    assert_ne!(
        parent_server.port, child_server.port,
        "two servers are two origins"
    );
    let url = format!(
        "{}?child={}",
        parent_server.page("frames-cross-origin.html"),
        child_server.page("frames-child.html")
    );
    let mut page = open(&url);

    wait_for_heading(&mut page, "Child page");
    assert_eq!(count_nodes(&page, Role::Heading, "Cross-origin parent"), 1);
    assert_eq!(count_nodes(&page, Role::Heading, "Child page"), 1);

    // The frame's document is another origin, perhaps another process: the action still finds
    // it, because a tree id is a pure function of the pipeline id.
    let button = page.find(Role::Button, "Child button");
    page.act(button, Action::Click, None);
    page.spin_until(|page| messages(page) == "child-pressed");
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_a_nested_iframe_is_part_of_the_tree() {
    let server = TestServer::start();
    let mut page = open(&server.page("frames-nested.html"));

    wait_for_heading(&mut page, "Child page");
    assert_eq!(count_nodes(&page, Role::Heading, "Nested page"), 1);
    assert_eq!(count_nodes(&page, Role::Heading, "Middle page"), 1);
    assert_eq!(count_nodes(&page, Role::Heading, "Child page"), 1);
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_an_iframe_added_and_removed_by_script_comes_and_goes() {
    let server = TestServer::start();
    let mut page = open(&server.page("frames-dynamic.html"));
    page.spin_until(|page| count_nodes(page, Role::Heading, "Dynamic page") == 1);
    assert_eq!(count_nodes(&page, Role::Heading, "Child page"), 0);

    page.eval("addFrame(); 1");
    wait_for_heading(&mut page, "Child page");
    assert_eq!(count_nodes(&page, Role::Heading, "Child page"), 1);

    page.eval("removeFrame(); 1");
    page.spin_until(|page| count_nodes(page, Role::Heading, "Child page") == 0);

    // And again: a second document in the same slot is a new tree, not the old one come back.
    page.eval("addFrame(); 1");
    wait_for_heading(&mut page, "Child page");
    assert_eq!(count_nodes(&page, Role::Heading, "Child page"), 1);
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_navigating_the_top_level_leaves_no_stale_child_grafted() {
    let server = TestServer::start();
    let mut page = open(&server.page("frames-parent.html"));
    wait_for_heading(&mut page, "Child page");

    // The old page's child is still sending updates for a moment after the constellation has
    // switched documents: none of them may become the page, and none may stay in the tree.
    page.eval("location.href = 'frames-other.html'; 1");
    page.spin_until(|page| {
        count_nodes(page, Role::Heading, "Other parent page") == 1
            && count_nodes(page, Role::Heading, "Second child page") == 1
    });

    assert_eq!(
        count_nodes(&page, Role::Heading, "Parent page"),
        0,
        "the old page is gone"
    );
    assert_eq!(
        count_nodes(&page, Role::Heading, "Child page"),
        0,
        "the old page's frame is gone, and was not grafted as the new page"
    );
    assert_eq!(nodes_named(&page, "Same-origin frame").len(), 0);
    assert_eq!(nodes_named(&page, "Second frame").len(), 1);
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_navigating_back_to_a_page_with_an_iframe_brings_its_content_back() {
    let server = TestServer::start();
    let mut page = open(&server.page("frames-parent.html"));
    wait_for_heading(&mut page, "Child page");

    page.eval("location.href = 'frames-other.html'; 1");
    page.spin_until(|page| count_nodes(page, Role::Heading, "Second child page") == 1);

    // The page is back from the history: its documents are activated again, with the new epoch.
    page.eval("history.back(); 1");
    page.spin_until(|page| {
        count_nodes(page, Role::Heading, "Parent page") == 1
            && count_nodes(page, Role::Heading, "Child page") == 1
    });
    assert_eq!(count_nodes(&page, Role::Heading, "Second child page"), 0);
}
