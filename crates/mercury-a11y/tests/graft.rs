// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The order in which a page's trees may reach the window's adapter (`mercury_a11y::graft`).
//! Every test feeds what the state releases to `accesskit_consumer::Tree`, the tree the platform
//! adapters keep, which panics where AccessKit would: a test that passes proves the sequence
//! the window would push is one AccessKit accepts. The arrival order tests run *every*
//! interleaving of the trees' own (ordered) update sequences.
//!
//! The trees: the window's (`ROOT`: a window with a toolbar and the "Page content" node that
//! carries the graft), the web view's `U` (a scroll view with one graft), the document's `D`,
//! an iframe's `F`.

use accesskit::{Action, ActionRequest, Node, NodeId, Role, Tree, TreeId, TreeUpdate, Uuid};
use accesskit_consumer::{Tree as Oracle, TreeChangeHandler as ChangeHandler};
use mercury_a11y::graft::{GraftState, Offered, QUEUE_LIMIT, Rejection, Route, guarded};

const WINDOW: NodeId = NodeId(0);
const TOOLBAR: NodeId = NodeId(1);
const PAGE: NodeId = NodeId(2);

fn tree(n: u128) -> TreeId {
    TreeId(Uuid::from_u128(n))
}

const U: u128 = 1;
const D: u128 = 2;
const F: u128 = 3;
const U2: u128 = 11;
const D2: u128 = 12;

fn id(n: u128) -> TreeId {
    tree(n)
}

fn node(role: Role, children: &[u64]) -> Node {
    let mut node = Node::new(role);
    node.set_children(children.iter().map(|id| NodeId(*id)).collect::<Vec<_>>());
    node
}

fn graft(role: Role, tree: TreeId) -> Node {
    let mut node = Node::new(role);
    node.set_tree_id(tree);
    node
}

/// The window's own tree. `page_tree` is the tree the "Page content" node grafts.
fn root_update(page_tree: Option<TreeId>, focus: NodeId) -> TreeUpdate {
    let mut page = Node::new(Role::Main);
    page.set_label("Page content");
    if let Some(page_tree) = page_tree {
        page.set_tree_id(page_tree);
    }
    TreeUpdate {
        nodes: vec![
            (WINDOW, node(Role::Window, &[1, 2])),
            (TOOLBAR, Node::new(Role::Navigation)),
            (PAGE, page),
        ],
        tree: Some(Tree::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// The window's own tree without the page node.
fn root_without_page() -> TreeUpdate {
    TreeUpdate {
        nodes: vec![
            (WINDOW, node(Role::Window, &[1])),
            (TOOLBAR, Node::new(Role::Navigation)),
        ],
        tree: Some(Tree::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus: TOOLBAR,
    }
}

/// The web view's tree `u`, grafting the document `d`: what Servo sends first and again when
/// the document changes. Focus on its root.
fn u_first(u: u128, d: u128) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![
            (NodeId(0), node(Role::ScrollView, &[1])),
            (NodeId(1), graft(Role::GenericContainer, id(d))),
        ],
        tree: Some(Tree::new(NodeId(0))),
        tree_id: id(u),
        focus: NodeId(0),
    }
}

/// Servo moves the web view's focus to the graft once the document's first tree is out.
fn u_focus_graft(u: u128) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![],
        tree: None,
        tree_id: id(u),
        focus: NodeId(1),
    }
}

/// The document's first tree: a heading, a link (focused) and an iframe node, which grafts
/// `iframe` if given.
fn d_first(d: u128, iframe: Option<u128>) -> TreeUpdate {
    let mut heading = Node::new(Role::Heading);
    heading.set_label("Title");
    let mut link = Node::new(Role::Link);
    link.set_label("Read more");
    let frame = match iframe {
        Some(f) => graft(Role::Iframe, id(f)),
        None => Node::new(Role::Iframe),
    };
    TreeUpdate {
        nodes: vec![
            (NodeId(0), node(Role::Document, &[1, 2, 3])),
            (NodeId(1), heading),
            (NodeId(2), link),
            (NodeId(3), frame),
        ],
        tree: Some(Tree::new(NodeId(0))),
        tree_id: id(d),
        focus: NodeId(2),
    }
}

/// A later update of the document: the link's label changed.
fn d_next(d: u128) -> TreeUpdate {
    let mut link = Node::new(Role::Link);
    link.set_label("Changed");
    TreeUpdate {
        nodes: vec![(NodeId(2), link)],
        tree: None,
        tree_id: id(d),
        focus: NodeId(2),
    }
}

/// The document's focus moves to the iframe node.
fn d_focus_iframe(d: u128) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![],
        tree: None,
        tree_id: id(d),
        focus: NodeId(3),
    }
}

/// A script removed the iframe: node 3 is no longer a child of the document.
fn d_remove_iframe(d: u128) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![(NodeId(0), node(Role::Document, &[1, 2]))],
        tree: None,
        tree_id: id(d),
        focus: NodeId(2),
    }
}

/// The iframe moved to another node: node 4 holds the graft now.
fn d_move_iframe(d: u128, f: u128) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![
            (NodeId(0), node(Role::Document, &[1, 2, 4])),
            (NodeId(4), graft(Role::Iframe, id(f))),
        ],
        tree: None,
        tree_id: id(d),
        focus: NodeId(2),
    }
}

/// A document whose iframe sits inside a section.
fn d_nested(d: u128, f: u128) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![
            (NodeId(0), node(Role::Document, &[1])),
            (NodeId(1), node(Role::GenericContainer, &[3])),
            (NodeId(3), graft(Role::Iframe, id(f))),
        ],
        tree: Some(Tree::new(NodeId(0))),
        tree_id: id(d),
        focus: NodeId(0),
    }
}

/// The section lost its children, and with them the iframe.
fn d_unnest(d: u128) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![(NodeId(1), node(Role::GenericContainer, &[]))],
        tree: None,
        tree_id: id(d),
        focus: NodeId(0),
    }
}

/// An iframe's first tree: one paragraph, focused.
fn f_first(f: u128) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![
            (NodeId(0), node(Role::Document, &[1])),
            (NodeId(1), Node::new(Role::Paragraph)),
        ],
        tree: Some(Tree::new(NodeId(0))),
        tree_id: id(f),
        focus: NodeId(1),
    }
}

fn f_next(f: u128) -> TreeUpdate {
    let mut paragraph = Node::new(Role::Paragraph);
    paragraph.set_value("changed");
    TreeUpdate {
        nodes: vec![(NodeId(1), paragraph)],
        tree: None,
        tree_id: id(f),
        focus: NodeId(1),
    }
}

struct Quiet;

impl ChangeHandler for Quiet {
    fn node_added(&mut self, _: &accesskit_consumer::Node<'_>) {}
    fn node_updated(&mut self, _: &accesskit_consumer::Node<'_>, _: &accesskit_consumer::Node<'_>) {
    }
    fn focus_moved(
        &mut self,
        _: Option<&accesskit_consumer::Node<'_>>,
        _: Option<&accesskit_consumer::Node<'_>>,
    ) {
    }
    fn node_removed(&mut self, _: &accesskit_consumer::Node<'_>) {}
}

/// The state under test and the consumer fed with exactly what it releases.
struct Rig {
    state: GraftState,
    oracle: Oracle,
    /// Foreign updates pushed to the oracle, and window focus updates.
    pushed: Vec<(TreeId, usize)>,
    resets: usize,
}

impl Rig {
    /// The adapter's initial tree: the window with a plain page node, focus on the toolbar.
    fn new() -> Self {
        let mut state = GraftState::new();
        let mut first = root_update(None, TOOLBAR);
        state.push_root(&mut first);
        let oracle = Oracle::new(first, true);
        Self {
            state,
            oracle,
            pushed: Vec::new(),
            resets: 0,
        }
    }

    fn push(&mut self, update: TreeUpdate) {
        self.pushed.push((update.tree_id, update.nodes.len()));
        self.oracle.update_and_process_changes(update, &mut Quiet);
    }

    /// The window pushes an update of its own tree.
    fn root(&mut self, mut update: TreeUpdate) {
        self.state.push_root(&mut update);
        self.push(update);
        self.settle();
    }

    /// The shell hands the window an update of a page's tree.
    fn offer(&mut self, update: TreeUpdate) -> Offered {
        let offered = self.state.offer(update);
        self.settle();
        offered
    }

    /// Pushes what the state releases.
    fn settle(&mut self) {
        let drained = self.state.take_ready();
        if drained.reset {
            self.resets += 1;
        }
        for update in drained.updates {
            self.push(update);
        }
    }

    fn step(&mut self, step: Step) {
        match step {
            Step::Root(update) => self.root(update),
            Step::Offer(update) => {
                self.offer(update);
            }
        }
    }

    fn has_tree(&self, tree: u128) -> bool {
        self.oracle.state().subtree_root(id(tree)).is_some()
    }

    fn focused_role(&self) -> Option<Role> {
        self.oracle.state().focus().map(|node| node.role())
    }
}

#[derive(Clone)]
enum Step {
    Root(TreeUpdate),
    Offer(TreeUpdate),
}

/// Runs `check` on every interleaving of the sequences that keeps each sequence's own order, and
/// returns how many there were.
fn for_each_interleaving(sequences: &[Vec<Step>], mut check: impl FnMut(Vec<Step>)) -> usize {
    fn go(
        sequences: &[Vec<Step>],
        taken: &mut Vec<usize>,
        current: &mut Vec<Step>,
        total: usize,
        count: &mut usize,
        check: &mut impl FnMut(Vec<Step>),
    ) {
        if current.len() == total {
            *count += 1;
            check(current.clone());
            return;
        }
        for which in 0..sequences.len() {
            if taken[which] < sequences[which].len() {
                current.push(sequences[which][taken[which]].clone());
                taken[which] += 1;
                go(sequences, taken, current, total, count, check);
                taken[which] -= 1;
                current.pop();
            }
        }
    }
    let total = sequences.iter().map(Vec::len).sum();
    let mut count = 0;
    go(
        sequences,
        &mut vec![0; sequences.len()],
        &mut Vec::new(),
        total,
        &mut count,
        &mut check,
    );
    count
}

fn run(steps: Vec<Step>) -> Rig {
    let mut rig = Rig::new();
    for step in steps {
        rig.step(step);
    }
    rig
}

// --- arrival order -----------------------------------------------------------------------------

#[test]
fn the_trees_in_the_order_servo_sends_them() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    assert_eq!(rig.offer(u_first(U, D)), Offered::Queued);
    rig.offer(d_first(D, None));
    rig.offer(u_focus_graft(U));
    rig.offer(d_next(D));

    assert!(rig.has_tree(U) && rig.has_tree(D));
    assert_eq!(rig.state.queued(), 0);
    // The focus follows the window's page node through the web view's graft into the document.
    assert_eq!(rig.focused_role(), Some(Role::Link));
    assert_eq!(rig.state.stats().pushed, 4);
}

#[test]
fn every_arrival_order_of_the_window_the_web_view_and_the_document_is_accepted() {
    let sequences = [
        vec![Step::Root(root_update(Some(id(U)), PAGE))],
        vec![Step::Offer(u_first(U, D)), Step::Offer(u_focus_graft(U))],
        vec![Step::Offer(d_first(D, None)), Step::Offer(d_next(D))],
    ];
    let count = for_each_interleaving(&sequences, |steps| {
        let rig = run(steps);
        assert!(rig.has_tree(U) && rig.has_tree(D));
        assert_eq!(rig.state.queued(), 0);
        assert_eq!(rig.focused_role(), Some(Role::Link));
    });
    assert_eq!(count, 30);
}

#[test]
fn every_arrival_order_with_an_iframe_is_accepted() {
    let sequences = [
        vec![Step::Root(root_update(Some(id(U)), PAGE))],
        vec![Step::Offer(u_first(U, D)), Step::Offer(u_focus_graft(U))],
        vec![
            Step::Offer(d_first(D, Some(F))),
            Step::Offer(d_focus_iframe(D)),
        ],
        vec![Step::Offer(f_first(F)), Step::Offer(f_next(F))],
    ];
    let count = for_each_interleaving(&sequences, |steps| {
        let rig = run(steps);
        assert!(rig.has_tree(U) && rig.has_tree(D) && rig.has_tree(F));
        assert_eq!(rig.state.queued(), 0);
        // Window -> web view -> document -> iframe: the focus ends in the iframe's paragraph.
        assert_eq!(rig.focused_role(), Some(Role::Paragraph));
    });
    assert_eq!(count, 630);
}

#[test]
fn every_arrival_order_when_the_document_is_replaced_by_a_navigation() {
    // The web view's tree grafts D, then D2 (a navigation). D's updates may still arrive after.
    let sequences = [
        vec![Step::Root(root_update(Some(id(U)), PAGE))],
        vec![
            Step::Offer(u_first(U, D)),
            Step::Offer(u_focus_graft(U)),
            Step::Offer(u_first(U, D2)),
            Step::Offer(u_focus_graft(U)),
        ],
        vec![Step::Offer(d_first(D, None)), Step::Offer(d_next(D))],
        vec![Step::Offer(d_first(D2, None))],
    ];
    let count = for_each_interleaving(&sequences, |steps| {
        let rig = run(steps);
        assert!(rig.has_tree(U) && rig.has_tree(D2));
        assert!(!rig.has_tree(D), "the replaced document is gone");
        assert_eq!(rig.state.queued(), 0);
        assert_eq!(rig.focused_role(), Some(Role::Link));
    });
    assert_eq!(count, 840);
}

#[test]
fn every_arrival_order_when_a_script_removes_the_iframe() {
    // F's updates may arrive after D no longer holds the iframe node.
    let sequences = [
        vec![Step::Root(root_update(Some(id(U)), PAGE))],
        vec![Step::Offer(u_first(U, D)), Step::Offer(u_focus_graft(U))],
        vec![
            Step::Offer(d_first(D, Some(F))),
            Step::Offer(d_remove_iframe(D)),
        ],
        vec![Step::Offer(f_first(F)), Step::Offer(f_next(F))],
    ];
    let count = for_each_interleaving(&sequences, |steps| {
        let rig = run(steps);
        assert!(rig.has_tree(D));
        assert!(!rig.has_tree(F), "the removed iframe's tree is gone");
        assert_eq!(rig.state.queued(), 0);
        assert_eq!(rig.focused_role(), Some(Role::Link));
    });
    assert_eq!(count, 630);
}

#[test]
fn every_arrival_order_when_the_iframe_moves_to_another_node() {
    let sequences = [
        vec![Step::Root(root_update(Some(id(U)), PAGE))],
        vec![Step::Offer(u_first(U, D)), Step::Offer(u_focus_graft(U))],
        vec![
            Step::Offer(d_first(D, Some(F))),
            Step::Offer(d_move_iframe(D, F)),
        ],
        vec![Step::Offer(f_first(F)), Step::Offer(f_next(F))],
    ];
    for_each_interleaving(&sequences, |steps| {
        let rig = run(steps);
        // The graft only moved: the iframe's tree is still there, unless its first update
        // arrived after the move and... it still is: the graft exists in both cases.
        assert!(rig.has_tree(F));
        assert_eq!(rig.state.queued(), 0);
    });
}

// --- the rules one at a time -------------------------------------------------------------------

#[test]
fn an_update_waits_for_its_graft_and_goes_right_after_it() {
    let mut rig = Rig::new();
    assert_eq!(rig.offer(u_first(U, D)), Offered::Queued);
    assert_eq!(rig.offer(d_first(D, None)), Offered::Queued);
    assert_eq!(rig.state.queued(), 2);
    assert_eq!(rig.state.queued_for(id(U)), 1);
    assert!(!rig.has_tree(U));
    assert!(!rig.state.is_grafted(id(U)));

    rig.root(root_update(Some(id(U)), PAGE));
    assert!(rig.state.is_grafted(id(U)) && rig.state.is_grafted(id(D)));
    assert!(rig.state.is_created(id(U)) && rig.state.is_created(id(D)));
    assert!(rig.has_tree(U) && rig.has_tree(D));
    assert_eq!(rig.state.queued(), 0);
}

#[test]
fn a_tree_that_nothing_grafts_waits_without_blocking_the_others() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    for _ in 0..5 {
        rig.offer(d_next(F)); // an unrelated tree nobody grafts
    }
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, None));
    assert!(rig.has_tree(U) && rig.has_tree(D));
    assert_eq!(rig.state.queued(), 5);
    assert_eq!(rig.state.queued_for(id(F)), 5);
}

#[test]
fn updates_of_one_tree_go_in_the_order_they_were_offered() {
    let mut rig = Rig::new();
    rig.offer(d_first(D, None));
    rig.offer(d_next(D));
    rig.offer(u_first(U, D));
    rig.root(root_update(Some(id(U)), PAGE));
    let order: Vec<(TreeId, usize)> = rig
        .pushed
        .iter()
        .copied()
        .filter(|(tree, _)| *tree == id(D))
        .collect();
    // The first tree of D (four nodes), then its change (one node).
    assert_eq!(order, [(id(D), 4), (id(D), 1)]);
}

#[test]
fn the_first_update_of_a_grafted_tree_without_tree_data_is_dropped() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    // An incremental update with no first tree before it would panic the consumer.
    rig.offer(u_focus_graft(U));
    assert_eq!(rig.state.queued(), 0);
    assert_eq!(rig.state.stats().dropped_malformed, 1);
    assert!(!rig.has_tree(U));
    // A proper first tree afterwards works.
    rig.offer(u_first(U, D));
    assert!(rig.has_tree(U));
}

#[test]
fn a_tree_resent_in_full_replaces_the_first_one() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, None));
    rig.offer(d_first(D, None));
    rig.offer(u_first(U, D));
    assert!(rig.has_tree(U) && rig.has_tree(D));
    assert_eq!(rig.state.queued(), 0);
}

#[test]
fn updates_that_name_the_windows_own_tree_are_refused() {
    let mut rig = Rig::new();
    let hostile = root_update(None, PAGE);
    assert_eq!(rig.offer(hostile), Offered::Rejected(Rejection::RootTree));
    assert_eq!(rig.state.stats().dropped_root, 1);
    assert_eq!(rig.state.queued(), 0);
    assert_eq!(rig.pushed.len(), 0);
}

// --- focus ---------------------------------------------------------------------------------------

#[test]
fn a_window_focus_on_the_page_node_waits_for_the_pages_tree() {
    let mut rig = Rig::new();
    // The page node has focus and a graft, but no tree: the consumer would panic if the focus
    // were followed. The window's root is focused instead.
    rig.root(root_update(Some(id(U)), PAGE));
    assert_eq!(rig.focused_role(), Some(Role::Window));
    assert_eq!(rig.state.stats().focus_deferred, 1);

    // The web view's tree arrives: the window's focus on the page node is pushed after it.
    rig.offer(u_first(U, D));
    assert_eq!(rig.focused_role(), Some(Role::ScrollView));
    // Its focus on the graft waits for the document, then goes.
    rig.offer(u_focus_graft(U));
    assert_eq!(rig.focused_role(), Some(Role::ScrollView));
    rig.offer(d_first(D, None));
    assert_eq!(rig.focused_role(), Some(Role::Link));
}

#[test]
fn a_focus_on_an_iframe_node_waits_for_the_iframes_tree() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(u_focus_graft(U));
    rig.offer(d_first(D, Some(F)));
    rig.offer(d_focus_iframe(D));
    assert_eq!(rig.focused_role(), Some(Role::Document));
    rig.offer(f_first(F));
    assert_eq!(rig.focused_role(), Some(Role::Paragraph));
}

#[test]
fn a_focus_that_was_replaced_is_dropped_if_its_node_went_away() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(u_focus_graft(U));
    rig.offer(d_first(D, Some(F)));
    rig.offer(d_focus_iframe(D));
    // The iframe node is removed before its tree ever arrives: nothing to restore.
    rig.offer(d_remove_iframe(D));
    assert_eq!(rig.focused_role(), Some(Role::Link));
    rig.offer(f_first(F));
    assert!(!rig.has_tree(F));
    assert_eq!(rig.focused_role(), Some(Role::Link));
}

#[test]
fn a_focus_restored_for_the_window_is_a_focus_only_update_of_its_tree() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    let before = rig.pushed.len();
    rig.offer(u_first(U, D));
    // The web view's tree, then the window's focus update (no nodes).
    assert_eq!(&rig.pushed[before..], [(id(U), 2), (TreeId::ROOT, 0)]);
}

// --- forgetting ------------------------------------------------------------------------------------

#[test]
fn a_tab_switch_forgets_the_old_tree_and_refuses_its_late_updates() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(u_focus_graft(U));
    rig.offer(d_first(D, Some(F)));
    rig.offer(f_first(F));
    assert!(rig.has_tree(F));

    // The shell switches tabs: the old tree is forgotten, the page node grafts the new one.
    rig.state.forget_tree(id(U));
    assert_eq!(
        rig.offer(d_next(D)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
    assert_eq!(
        rig.offer(f_next(F)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
    rig.root(root_update(Some(id(U2)), PAGE));
    assert!(!rig.has_tree(U) && !rig.has_tree(D) && !rig.has_tree(F));

    rig.offer(u_first(U2, D2));
    rig.offer(d_first(D2, None));
    rig.offer(u_focus_graft(U2));
    assert!(rig.has_tree(U2) && rig.has_tree(D2));
    assert_eq!(rig.focused_role(), Some(Role::Link));
    // A late update of the old tab after all that is still refused.
    assert_eq!(
        rig.offer(d_next(D)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
    assert_eq!(rig.state.stats().dropped_forgotten, 3);
}

#[test]
fn forgetting_a_tree_drops_what_waits_for_it() {
    let mut rig = Rig::new();
    for _ in 0..3 {
        rig.offer(d_next(D));
    }
    assert_eq!(rig.state.queued(), 3);
    rig.state.forget_tree(id(D));
    assert_eq!(rig.state.queued(), 0);
    assert_eq!(
        rig.offer(d_next(D)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
}

#[test]
fn a_tree_whose_graft_was_replaced_cannot_come_back_as_a_stale_update() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, None));
    // A navigation: the web view's tree now grafts D2.
    rig.offer(u_first(U, D2));
    assert!(!rig.has_tree(D));
    assert_eq!(
        rig.offer(d_next(D)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
    rig.offer(d_first(D2, None));
    assert!(rig.has_tree(D2));
}

#[test]
fn a_document_that_comes_back_is_accepted_again() {
    // Back-forward cache: D is grafted, replaced by D2, then grafted again.
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, None));
    rig.offer(u_first(U, D2));
    rig.offer(d_first(D2, None));
    assert!(!rig.has_tree(D));
    rig.offer(u_first(U, D));
    assert_eq!(rig.offer(d_first(D, None)), Offered::Queued);
    assert!(rig.has_tree(D) && !rig.has_tree(D2));
}

#[test]
fn removing_a_section_forgets_the_iframe_inside_it() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_nested(D, F));
    rig.offer(f_first(F));
    assert!(rig.has_tree(F));
    rig.offer(d_unnest(D));
    assert!(!rig.has_tree(F));
    assert_eq!(
        rig.offer(f_next(F)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
}

#[test]
fn removing_the_page_node_from_the_window_forgets_the_whole_page() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, Some(F)));
    rig.offer(f_first(F));
    rig.root(root_without_page());
    assert!(!rig.has_tree(U) && !rig.has_tree(D) && !rig.has_tree(F));
    assert_eq!(
        rig.offer(d_next(D)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
    assert_eq!(
        rig.offer(f_next(F)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
    assert_eq!(rig.state.queued(), 0);
}

#[test]
fn a_window_rebuild_that_keeps_the_graft_changes_nothing() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, None));
    rig.offer(u_focus_graft(U));
    // `reload_tree()` rebuilds the whole window tree: the same page node, the same graft.
    rig.root(root_update(Some(id(U)), TOOLBAR));
    rig.root(root_update(Some(id(U)), PAGE));
    assert!(rig.has_tree(U) && rig.has_tree(D));
    assert_eq!(rig.offer(d_next(D)), Offered::Queued);
    assert_eq!(rig.focused_role(), Some(Role::Link));
}

#[test]
fn removing_the_graft_from_the_page_node_forgets_the_tree_and_a_new_graft_revives_it() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, None));
    rig.root(root_update(None, TOOLBAR));
    assert!(!rig.has_tree(U) && !rig.has_tree(D));
    assert_eq!(
        rig.offer(d_next(D)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
    // The same tree grafted again (the tab was deactivated and activated again, as a test of
    // the rule, with a first tree that carries its data).
    rig.root(root_update(Some(id(U)), TOOLBAR));
    assert_eq!(rig.offer(u_first(U, D)), Offered::Queued);
    assert_eq!(rig.offer(d_first(D, None)), Offered::Queued);
    assert!(rig.has_tree(U) && rig.has_tree(D));
}

// --- limits and reset ---------------------------------------------------------------------------

#[test]
fn a_queue_that_overflows_drops_everything_foreign_and_asks_for_a_reset() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, None));
    assert!(rig.has_tree(D));

    // Updates for a tree nobody grafts pile up.
    for _ in 0..QUEUE_LIMIT {
        assert_eq!(rig.offer(f_next(F)), Offered::Queued);
    }
    assert_eq!(rig.state.queued_for(id(F)), QUEUE_LIMIT);
    assert_eq!(rig.resets, 0);
    assert_eq!(
        rig.offer(f_next(F)),
        Offered::Rejected(Rejection::QueueFull)
    );
    assert_eq!(rig.resets, 1);
    assert_eq!(rig.state.queued(), 0);
    assert_eq!(rig.state.stats().dropped_overflow, QUEUE_LIMIT as u64 + 1);
    assert_eq!(rig.state.stats().resets, 1);

    // Everything foreign is forgotten, the late updates are refused...
    assert!(!rig.state.is_created(id(U)) && !rig.state.is_created(id(D)));
    assert_eq!(
        rig.offer(d_next(D)),
        Offered::Rejected(Rejection::ForgottenTree)
    );
    assert_eq!(
        rig.offer(f_next(F)),
        Offered::Rejected(Rejection::ForgottenTree)
    );

    // ...and the shell's deactivation and new activation (new tree ids) works from scratch.
    rig.root(root_update(None, TOOLBAR));
    assert!(!rig.has_tree(U) && !rig.has_tree(D));
    rig.root(root_update(Some(id(U2)), PAGE));
    rig.offer(u_first(U2, D2));
    rig.offer(d_first(D2, None));
    rig.offer(u_focus_graft(U2));
    assert!(rig.has_tree(U2) && rig.has_tree(D2));
    assert_eq!(rig.focused_role(), Some(Role::Link));
}

#[test]
fn exactly_the_limit_does_not_overflow() {
    let mut state = GraftState::new();
    for _ in 0..QUEUE_LIMIT {
        assert_eq!(state.offer(f_next(F)), Offered::Queued);
    }
    assert!(!state.take_ready().reset);
    assert_eq!(state.queued(), QUEUE_LIMIT);
}

#[test]
fn clearing_forgets_the_window_too_and_a_new_adapter_tree_starts_again() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, None));
    let pushed = rig.state.stats().pushed;

    // Assistive technology detached and attached again: AccessKit builds a new consumer tree
    // from a new initial update, and the state starts over.
    rig.state.clear();
    assert_eq!(rig.state.queued(), 0);
    assert!(!rig.state.is_created(id(U)));
    assert!(!rig.state.is_created(TreeId::ROOT));
    assert_eq!(rig.state.stats().pushed, pushed, "the counters survive");
    let mut again = Rig::new();
    again.state = std::mem::take(&mut rig.state);
    let mut first = root_update(None, TOOLBAR);
    again.state.push_root(&mut first);
    again.oracle = Oracle::new(first, true);
    again.root(root_update(Some(id(U)), PAGE));
    again.offer(u_first(U, D));
    again.offer(d_first(D, None));
    again.offer(u_focus_graft(U));
    assert!(again.has_tree(U) && again.has_tree(D));
    assert_eq!(again.focused_role(), Some(Role::Link));
}

#[test]
fn an_update_of_the_windows_tree_without_tree_data_before_any_is_ignored() {
    let mut state = GraftState::new();
    let mut focus_only = TreeUpdate {
        nodes: vec![],
        tree: None,
        tree_id: TreeId::ROOT,
        focus: PAGE,
    };
    state.push_root(&mut focus_only);
    assert_eq!(focus_only.focus, PAGE, "nothing to follow: left as it is");
    assert!(!state.is_created(TreeId::ROOT), "no tree was made from it");
    assert_eq!(state.take_ready().updates.len(), 0);
}

// --- actions -------------------------------------------------------------------------------------

fn request(tree: TreeId) -> ActionRequest {
    ActionRequest {
        action: Action::Click,
        target_tree: tree,
        target_node: NodeId(2),
        data: None,
    }
}

#[test]
fn an_action_for_the_windows_tree_is_the_windows_own() {
    let state = GraftState::new();
    assert_eq!(state.route(&request(TreeId::ROOT)), Route::Own);
}

#[test]
fn an_action_for_a_tree_the_adapter_holds_goes_to_the_engine() {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, Some(F)));
    rig.offer(f_first(F));
    for tree in [U, D, F] {
        assert_eq!(
            rig.state.route(&request(id(tree))),
            Route::Foreign,
            "tree {tree}"
        );
    }
}

#[test]
fn an_action_for_a_tree_nobody_holds_goes_nowhere() {
    let mut rig = Rig::new();
    // Never grafted, and grafted but without its first tree: the technology cannot have seen
    // either, so a request naming one is not from it.
    assert_eq!(rig.state.route(&request(id(D))), Route::Unknown);
    rig.root(root_update(Some(id(U)), PAGE));
    assert_eq!(rig.state.route(&request(id(U))), Route::Unknown);
    rig.offer(u_first(U, D));
    assert_eq!(rig.state.route(&request(id(U))), Route::Foreign);
    assert_eq!(rig.state.route(&request(id(D))), Route::Unknown);
    // After a tab switch the old tree's actions are not forwarded either.
    rig.state.forget_tree(id(U));
    assert_eq!(rig.state.route(&request(id(U))), Route::Unknown);
}

// --- the panic guard ----------------------------------------------------------------------------

#[test]
fn the_oracle_panics_on_a_subtree_without_a_graft_and_the_guard_catches_it() {
    // Pushing D before any graft names it is what the state exists to prevent: the consumer
    // refuses it by panicking, and `guarded` turns that into a value.
    let mut rig = Rig::new();
    let bad = d_first(D, None);
    let result = guarded(|| rig.oracle.update_and_process_changes(bad, &mut Quiet));
    assert!(result.is_err());

    // Through the state the same update just waits.
    let mut safe = Rig::new();
    assert_eq!(safe.offer(d_first(D, None)), Offered::Queued);
    assert_eq!(safe.state.queued(), 1);
}

#[test]
fn the_guard_passes_a_value_through() {
    assert_eq!(guarded(|| 7), Ok(7));
}

#[test]
fn a_caught_panic_carries_no_payload() {
    let result = guarded(|| -> u8 { panic!("a page's own secret text") });
    let error = result.unwrap_err();
    assert!(!format!("{error:?}").contains("secret"));
}

// --- privacy ---------------------------------------------------------------------------------------

#[test]
fn nothing_the_state_prints_contains_page_text() {
    let mut state = GraftState::new();
    let mut secret = d_first(D, None);
    for (_, node) in &mut secret.nodes {
        node.set_label("SECRET-LABEL");
        node.set_value("SECRET-VALUE");
    }
    state.offer(secret);
    let drained = state.take_ready();
    let printed = format!("{state:?} {:?} {drained:?}", state.stats());
    assert!(!printed.contains("SECRET"), "{printed}");

    // After the tree was released too: the state kept its shape, not a label.
    let mut full = Rig::new();
    full.root(root_update(Some(id(U)), PAGE));
    let mut secret = u_first(U, D);
    secret.nodes[0].1.set_label("SECRET-LABEL");
    full.offer(secret);
    assert!(full.has_tree(U));
    let printed = format!("{:?}", full.state);
    assert!(!printed.contains("SECRET"), "{printed}");
    assert!(printed.len() < 400, "{printed}");
}

// --- counters --------------------------------------------------------------------------------------

#[test]
fn the_counters_count_what_happened() {
    let mut rig = Rig::new();
    rig.offer(root_update(None, PAGE)); // window tree: refused
    rig.root(root_update(Some(id(U)), PAGE)); // focus deferred
    rig.offer(u_focus_graft(U)); // first update without data: dropped
    rig.offer(u_first(U, D));
    rig.offer(d_first(D, None));
    rig.state.forget_tree(id(U));
    rig.offer(d_next(D)); // forgotten
    let stats = rig.state.stats();
    assert_eq!(stats.dropped_root, 1);
    assert_eq!(stats.dropped_malformed, 1);
    assert_eq!(stats.dropped_forgotten, 1);
    assert!(stats.focus_deferred >= 1);
    // The web view's first tree and the document's first tree went; the focus-only update of
    // the window's tree that followed the first of them is not counted among the page's.
    assert_eq!(stats.pushed, 2);
}

// --- the host window gaining or losing focus ---------------------------------------------------------

/// A document whose focused node has a local id the window's own tree does not have (Servo's
/// node ids are not small numbers).
fn d_with_high_ids(d: u128) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![
            (NodeId(0), node(Role::Document, &[77])),
            (NodeId(77), Node::new(Role::Link)),
        ],
        tree: Some(Tree::new(NodeId(0))),
        tree_id: id(d),
        focus: NodeId(77),
    }
}

/// The window with the focus in a page whose focused node has an id the window does not have.
fn rig_with_the_focus_in_a_page() -> Rig {
    let mut rig = Rig::new();
    rig.root(root_update(Some(id(U)), PAGE));
    rig.offer(u_first(U, D));
    rig.offer(u_focus_graft(U));
    rig.offer(d_with_high_ids(D));
    assert_eq!(rig.focused_role(), Some(Role::Link));
    rig
}

fn host_focus_changes(rig: &mut Rig) {
    for focused in [false, true, false, true] {
        rig.oracle
            .update_host_focus_state_and_process_changes(focused, &mut Quiet);
    }
}

#[test]
fn accesskit_consumer_panics_when_the_host_changes_focus_while_the_focus_is_in_a_page() {
    // Pins an upstream defect (accesskit_consumer 0.38): the host-focus update rebuilds the
    // consumer's focus from the local id of the focused node, used as an id of the window's own
    // tree. `mercury_graft::before_host_focus_change` works around it. When this test fails, the
    // consumer was fixed: delete the workaround and this test.
    let mut rig = rig_with_the_focus_in_a_page();
    assert!(rig.state.root_focus_is_in_a_page());
    let result = guarded(|| host_focus_changes(&mut rig));
    assert!(
        result.is_err(),
        "the consumer no longer panics: drop the workaround"
    );
}

#[test]
fn the_host_changing_focus_is_safe_once_the_windows_focus_is_moved_off_the_page_for_the_moment() {
    let mut rig = rig_with_the_focus_in_a_page();
    // What `before_host_focus_change` pushes, and then what the rebuild after the event pushes.
    assert!(rig.state.root_focus_is_in_a_page());
    rig.root(TreeUpdate {
        nodes: vec![],
        tree: None,
        tree_id: TreeId::ROOT,
        focus: WINDOW,
    });
    assert!(!rig.state.root_focus_is_in_a_page());
    assert_eq!(rig.focused_role(), Some(Role::Window));
    host_focus_changes(&mut rig);
    rig.root(TreeUpdate {
        nodes: vec![],
        tree: None,
        tree_id: TreeId::ROOT,
        focus: PAGE,
    });
    assert!(rig.state.root_focus_is_in_a_page());
    assert_eq!(rig.focused_role(), Some(Role::Link));
}

#[test]
fn the_windows_focus_is_in_a_page_only_while_it_is_on_a_grafted_node() {
    let mut rig = Rig::new();
    assert!(!rig.state.root_focus_is_in_a_page());
    // On the page node, but nothing is grafted yet: the focus was replaced.
    rig.root(root_update(Some(id(U)), PAGE));
    assert!(!rig.state.root_focus_is_in_a_page());
    // The web view's tree arrives and the focus on the page node is restored.
    rig.offer(u_first(U, D));
    assert!(rig.state.root_focus_is_in_a_page());
    // Focus on the toolbar.
    rig.root(root_update(Some(id(U)), TOOLBAR));
    assert!(!rig.state.root_focus_is_in_a_page());
    // A window without a graft node on the focus.
    rig.root(root_update(None, PAGE));
    assert!(!rig.state.root_focus_is_in_a_page());
    assert!(!GraftState::new().root_focus_is_in_a_page());
}
