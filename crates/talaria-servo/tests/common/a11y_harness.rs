// SPDX-License-Identifier: MPL-2.0
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Adapted from Servo's accessibility tests: https://github.com/servo/servo, tag v0.8.0, files
// `tests/accessibility.rs` (`build_test`, `build_webview_and_tree`, `build_tree`, `wait_for_min_updates`,
// `find_first_matching_node`) and `tests/common/mod.rs` (MPL-2.0; they are under
// `third_party/servo/tests/` in this repository). Mercury: a11y-1 item 3 (`core-states`, EE12). What
// is Mercury's own: the `A11yPage` API below, which item 5's `a11y_actions.rs` and `a11y_frames.rs`
// code against; the queue that holds a tree's updates until a graft names it (an iframe's tree can
// arrive before its parent's graft); `find`, `act`, `eval` and `snapshot`. The entry for
// THIRD-PARTY-NOTICES.md is in the item's notes.
//
// **Nothing here runs on a machine that cannot render.** `SoftwareRenderingContext` is a GL context;
// every test that starts a page is `#[ignore]`d and named `headless_servo_<what>` (see
// `a11y_pages.rs`), and `make test-a11y-pages` runs them on a Mac that can.

//! A page in a headless Servo, read the way an assistive technology reads it.
//!
//! [`A11yPage`] loads a page with accessibility on, keeps the `accesskit_consumer::Tree` a platform
//! adapter would keep (the window's tree, the web view's tree and the document's, grafted into each
//! other as the browser grafts them), and lets a test find a node by role and name, press an action,
//! run a script in the page and print the tree as stable text.

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use accesskit_consumer::{Node as TreeNode, Tree as ConsumerTree, TreeChangeHandler};
use mercury_a11y::snapshot::{DumpOptions, describe, dump_from};
use servo::{JSValue, LoadStatus, Preferences, WebView, WebViewBuilder};
use url::Url;

use super::{ServoTest, WebViewDelegateImpl, evaluate_javascript};

/// How long a wait lasts before the test fails (20 seconds, as the brief fixes it).
pub const BUDGET: Duration = Duration::from_secs(20);

/// How long [`A11yPage::act`] waits for the tree to change after an action before it decides the
/// action changed nothing the tree shows.
pub const ACT_SETTLE: Duration = Duration::from_secs(3);

/// How long the tree must stay unchanged for [`A11yPage::settle`] to call it settled.
pub const QUIET: Duration = Duration::from_millis(250);

/// The folder of the test pages: `tests/pages/a11y/` of the repository.
pub fn pages_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("pages")
        .join("a11y")
}

/// A page in a headless Servo with accessibility on.
pub struct A11yPage {
    // Dropped first, before the Servo that owns it (fields drop in order).
    webview: WebView,
    delegate: Rc<WebViewDelegateImpl>,
    tree: ConsumerTree,
    /// The web view's own tree: the one `ROOT`'s graft names.
    web_view_tree: TreeId,
    /// Every tree some applied update has a graft for (the web view's, from the start).
    grafted: HashSet<TreeId>,
    /// Updates whose tree no graft names yet, in arrival order.
    pending: Vec<TreeUpdate>,
    /// How many updates were applied to the tree.
    applied: usize,
    last_update: Instant,
    focused_graft: bool,
    servo_test: ServoTest,
}

struct NoChanges;

impl TreeChangeHandler for NoChanges {
    fn node_added(&mut self, _: &TreeNode) {}
    fn node_updated(&mut self, _: &TreeNode, _: &TreeNode) {}
    fn focus_moved(&mut self, _: Option<&TreeNode>, _: Option<&TreeNode>) {}
    fn node_removed(&mut self, _: &TreeNode) {}
}

/// The window's own tree: a root and one graft node for the web view's tree. This stands in for the
/// node an embedder builds (see servoshell's `gui.rs`); it sets no bounds, which is what an embedder
/// does, since AccessKit consumers exclude graft nodes from the presented tree.
fn window_update(web_view_tree: TreeId, focus: NodeId) -> TreeUpdate {
    let mut root = Node::new(Role::GenericContainer);
    root.set_children(vec![NodeId(1)]);
    let mut graft = Node::new(Role::GenericContainer);
    graft.set_tree_id(web_view_tree);
    TreeUpdate {
        nodes: vec![(NodeId(0), root), (NodeId(1), graft)],
        tree: Some(Tree::new(NodeId(0))),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// `html` as a `data:` URL (every byte but the unreserved ones percent-encoded).
fn data_url(html: &str) -> Url {
    let mut text = String::from("data:text/html;charset=utf-8,");
    for byte in html.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                text.push(byte as char);
            }
            _ => text.push_str(&format!("%{byte:02X}")),
        }
    }
    Url::parse(&text).expect("a data URL is a URL")
}

/// A script's result as JSON (Servo's own value type has no `serde_json` form).
fn to_json(value: JSValue) -> serde_json::Value {
    use serde_json::Value;
    match value {
        JSValue::Undefined | JSValue::Null => Value::Null,
        JSValue::Boolean(boolean) => Value::Bool(boolean),
        JSValue::Number(number) => {
            serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number)
        }
        JSValue::String(text)
        | JSValue::Element(text)
        | JSValue::ShadowRoot(text)
        | JSValue::Frame(text)
        | JSValue::Window(text) => Value::String(text),
        JSValue::Array(items) => Value::Array(items.into_iter().map(to_json).collect()),
        JSValue::Object(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key, to_json(value)))
                .collect(),
        ),
    }
}

impl A11yPage {
    /// Load this HTML as a page (a `data:` URL), with accessibility on, and wait for its first full
    /// tree: the load has finished and the document's tree has arrived and settled.
    pub fn load(html: &str) -> A11yPage {
        Self::start(data_url(html))
    }

    /// Load `tests/pages/a11y/<name>` (the file name, `.html` included) the same way.
    pub fn load_page(name: &str) -> A11yPage {
        let path = pages_dir().join(name);
        let html = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("cannot read the test page {}: {error}", path.display())
        });
        Self::load(&html)
    }

    /// Load a URL (a page served by `scripts/serve-tests.sh`, for a test that needs a real origin).
    pub fn load_url(url: &str) -> A11yPage {
        Self::start(Url::parse(url).unwrap_or_else(|error| panic!("{url:?} is not a URL: {error}")))
    }

    fn start(url: Url) -> A11yPage {
        let servo_test = ServoTest::new_with_builder(|builder| {
            let mut preferences = Preferences::default();
            // The pages are in the page itself or on localhost: a proxy would only interfere.
            preferences.network_http_proxy_uri = String::new();
            preferences.network_https_proxy_uri = String::new();
            preferences.accessibility_enabled = true;
            // The product keeps this off (it is expensive); a test wants the tree checked.
            preferences.expensive_accessibility_test_assertions_enabled = true;
            builder.preferences(preferences)
        });
        let delegate = Rc::new(WebViewDelegateImpl::default());
        let webview = WebViewBuilder::new(servo_test.servo(), servo_test.rendering_context.clone())
            .delegate(delegate.clone())
            .url(url)
            .build();
        let web_view_tree = webview
            .set_accessibility_active(true)
            .expect("accessibility is off in this Servo: the page has no tree to read");
        let tree = ConsumerTree::new(window_update(web_view_tree, NodeId(0)), true);
        let mut page = A11yPage {
            webview,
            delegate,
            tree,
            web_view_tree,
            grafted: HashSet::from([web_view_tree]),
            pending: Vec::new(),
            applied: 0,
            last_update: Instant::now(),
            focused_graft: false,
            servo_test,
        };
        page.spin_until(|page| {
            page.webview.load_status() == LoadStatus::Complete && page.root_web_area().is_some()
        });
        page.settle();
        page
    }

    /// The tree a platform adapter would hold: the window's tree, with the web view's tree grafted
    /// into it and the document's (and any iframe's) grafted into that.
    pub fn tree(&self) -> &ConsumerTree {
        &self.tree
    }

    /// The page's web view, for a test that navigates or sends input.
    pub fn webview(&self) -> &WebView {
        &self.webview
    }

    /// The Servo the page runs in.
    pub fn servo_test(&self) -> &ServoTest {
        &self.servo_test
    }

    /// How many tree updates have been applied so far.
    pub fn updates_applied(&self) -> usize {
        self.applied
    }

    /// What the page wrote to its console, in order.
    pub fn console_messages(&self) -> Vec<String> {
        self.delegate
            .console_messages
            .borrow()
            .iter()
            .map(|(_, message)| message.clone())
            .collect()
    }

    /// The first document's root (the page, not the window above it), if it has arrived.
    pub fn root_web_area(&self) -> Option<TreeNode<'_>> {
        let mut stack = vec![self.tree.state().root()];
        while let Some(node) = stack.pop() {
            if node.role() == Role::RootWebArea {
                return Some(node);
            }
            let children: Vec<TreeNode<'_>> = node.children().collect();
            stack.extend(children.into_iter().rev());
        }
        None
    }

    /// The nodes of the page a screen reader can reach, in document order, the page's root first. A
    /// node flagged hidden and everything under it is left out; documents inside iframes are part of
    /// it.
    pub fn nodes(&self) -> Vec<TreeNode<'_>> {
        let Some(root) = self.root_web_area() else {
            return Vec::new();
        };
        let mut nodes = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.data().is_hidden() {
                continue;
            }
            nodes.push(node);
            let children: Vec<TreeNode<'_>> = node.children().collect();
            stack.extend(children.into_iter().rev());
        }
        nodes
    }

    /// Every node of every tree, hidden ones and the window's included: for a check that something
    /// is *nowhere* in the tree.
    pub fn all_nodes(&self) -> Vec<TreeNode<'_>> {
        let mut nodes = Vec::new();
        let mut stack = vec![self.tree.state().root()];
        while let Some(node) = stack.pop() {
            nodes.push(node);
            let children: Vec<TreeNode<'_>> = node.children().collect();
            stack.extend(children.into_iter().rev());
        }
        nodes
    }

    /// The first node, in any tree, one of whose properties contains `needle` (what `Debug` shows of
    /// the node: label, value, description, placeholder, URL, every string), as the line `snapshot`
    /// would print for it. `None` when `needle` is nowhere: the check for a password's text.
    pub fn property_containing(&self, needle: &str) -> Option<String> {
        self.all_nodes()
            .into_iter()
            .find(|node| format!("{:?}", node.data()).contains(needle))
            .map(|node| describe(&node, &DumpOptions::default().with_hidden()))
    }

    /// The only node with this role and exact name, among the page's visible nodes. **Panics** when
    /// there is none or more than one, listing the nodes that come close.
    #[track_caller]
    pub fn find(&self, role: Role, name: &str) -> NodeId {
        let nodes = self.nodes();
        let found: Vec<&TreeNode<'_>> = nodes
            .iter()
            .filter(|node| node.role() == role && node.data().label() == Some(name))
            .collect();
        match found.as_slice() {
            [one] => one.locate().0,
            [] => panic!(
                "no {role:?} named {name:?} in the page; nodes with that role or that name:\n{}",
                self.candidates(&nodes, role, name)
            ),
            many => panic!(
                "{} nodes are a {role:?} named {name:?}, expected exactly one:\n{}",
                many.len(),
                many.iter()
                    .map(|node| format!("  {}", describe(node, &DumpOptions::default())))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        }
    }

    fn candidates(&self, nodes: &[TreeNode<'_>], role: Role, name: &str) -> String {
        let mut lines: Vec<String> = nodes
            .iter()
            .filter(|node| node.role() == role || node.data().label() == Some(name))
            .take(30)
            .map(|node| format!("  {}", describe(node, &DumpOptions::default())))
            .collect();
        if lines.is_empty() {
            lines.push("  (none; the page's tree follows)".to_string());
            lines.push(self.snapshot());
        }
        lines.join("\n")
    }

    /// The page's node with this id (the id `find` returned). **Panics** when no node has it or
    /// when ids of two trees (a page and an iframe) collide.
    #[track_caller]
    pub fn node(&self, target: NodeId) -> TreeNode<'_> {
        let nodes = self.nodes();
        let mut matches = nodes.into_iter().filter(|node| node.locate().0 == target);
        let first = matches
            .next()
            .unwrap_or_else(|| panic!("no node {target:?} in the page"));
        assert!(
            matches.next().is_none(),
            "node id {target:?} is in two trees: the page and a frame"
        );
        first
    }

    /// The text of the node's subtree: the values of its text runs, in order, joined as they are.
    pub fn text_of(&self, node: &TreeNode<'_>) -> String {
        let mut text = String::new();
        let mut stack = vec![*node];
        while let Some(node) = stack.pop() {
            if node.data().is_hidden() {
                continue;
            }
            if node.role() == Role::TextRun
                && let Some(value) = node.data().value()
            {
                text.push_str(value);
            }
            let children: Vec<TreeNode<'_>> = node.children().collect();
            stack.extend(children.into_iter().rev());
        }
        text
    }

    /// The page's tree as stable text (`mercury_a11y::snapshot::dump`, no bounds), from the page's
    /// root down: no window above it.
    pub fn snapshot(&self) -> String {
        self.snapshot_with(&DumpOptions::default())
    }

    /// [`snapshot`](Self::snapshot) with other options (actions, hidden nodes, bounds).
    pub fn snapshot_with(&self, options: &DumpOptions) -> String {
        match self.root_web_area() {
            Some(root) => dump_from(&root, options),
            None => dump_from(&self.tree.state().root(), options),
        }
    }

    /// Apply the updates the page has sent so far, in order, each tree's after the graft that names
    /// it. Updates for a tree no graft names yet wait.
    fn pump(&mut self) {
        let incoming: Vec<TreeUpdate> = self
            .delegate
            .last_accesskit_tree_updates
            .borrow_mut()
            .drain(..)
            .collect();
        self.pending.extend(incoming);
        while let Some(index) = self
            .pending
            .iter()
            .position(|update| self.grafted.contains(&update.tree_id))
        {
            let update = self.pending.remove(index);
            for (_, node) in &update.nodes {
                if let Some(tree) = node.tree_id() {
                    self.grafted.insert(tree);
                }
            }
            self.tree.update_and_process_changes(update, &mut NoChanges);
            self.applied += 1;
            self.last_update = Instant::now();
        }
        // Once the document is there, focus the window's graft node so that the focus follows the
        // grafts into the page (Servo's own harness does the same after building the tree).
        if !self.focused_graft && self.root_web_area().is_some() {
            self.focused_graft = true;
            let focus = TreeUpdate {
                nodes: vec![],
                tree: Some(Tree::new(NodeId(0))),
                tree_id: TreeId::ROOT,
                focus: NodeId(1),
            };
            self.tree.update_and_process_changes(focus, &mut NoChanges);
        }
    }

    /// Spin Servo and apply the page's tree updates until `condition` holds. **Panics** after
    /// [`BUDGET`] (20 s), with the page's tree.
    #[track_caller]
    pub fn spin_until(&mut self, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + BUDGET;
        loop {
            self.pump();
            if condition(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the condition did not hold within {} s; the page's tree is:\n{}",
                BUDGET.as_secs(),
                self.snapshot_with(&DumpOptions::default().with_hidden())
            );
            self.servo_test.servo().spin_event_loop();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Spin until no update has arrived for [`QUIET`] (bounded by [`BUDGET`]).
    pub fn settle(&mut self) {
        self.pump();
        self.last_update = Instant::now();
        let deadline = Instant::now() + BUDGET;
        while self.last_update.elapsed() < QUIET && Instant::now() < deadline {
            self.servo_test.servo().spin_event_loop();
            std::thread::sleep(Duration::from_millis(1));
            self.pump();
        }
    }

    /// Send an action to the node the way an assistive technology does (the engine's
    /// `forward_accessibility_action`), then spin until the next tree update arrives and the tree
    /// settles. An action that changes nothing the tree shows (focusing what has focus) sends no
    /// update: after [`ACT_SETTLE`] the call returns anyway, and the test checks the effect it
    /// expects through [`eval`](Self::eval).
    #[track_caller]
    pub fn act(&mut self, target: NodeId, action: Action, data: Option<ActionData>) {
        let tree = self.tree_of(target);
        let before = self.applied;
        self.servo_test
            .servo()
            .forward_accessibility_action(ActionRequest {
                action,
                target_tree: tree,
                target_node: target,
                data,
            });
        let deadline = Instant::now() + ACT_SETTLE;
        while self.applied == before && Instant::now() < deadline {
            self.servo_test.servo().spin_event_loop();
            std::thread::sleep(Duration::from_millis(1));
            self.pump();
        }
        self.settle();
    }

    /// The tree a page node lives in (the document's, or an iframe's).
    #[track_caller]
    fn tree_of(&self, target: NodeId) -> TreeId {
        self.node(target).locate().1
    }

    /// Run a script in the page and return what it evaluated to, as JSON. **Panics** if the script
    /// throws. Waits for the load to finish first.
    #[track_caller]
    pub fn eval(&self, script: &str) -> serde_json::Value {
        match evaluate_javascript(&self.servo_test, self.webview.clone(), script) {
            Ok(value) => to_json(value),
            Err(error) => panic!("the script failed: {error:?}\n{script}"),
        }
    }

    /// The tree id of the web view (the one the window's graft names): for a test of the grafts.
    pub fn web_view_tree(&self) -> TreeId {
        self.web_view_tree
    }
}
