// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The order in which a page's AccessKit subtrees may reach the window's adapter. Pure state (no
//! winit, no Slint), so the vendored Slint backend only calls it and its tests run in the light
//! pool against `accesskit_consumer`, the tree the platform adapters keep, which panics where
//! AccessKit would.
//!
//! The rules come from AccessKit's own invariants, not from Servo:
//!
//! 1. **A tree is pushed only after a graft that names it has been pushed.** The page's tree `U`
//!    (the web view's: a scroll view with one graft) hangs from the window's "Page content"
//!    node; the document's tree `D` hangs from `U`'s graft; an iframe's tree `F` hangs from a
//!    node of `D`. The rule is generic: an update for tree X waits in X's queue until some
//!    update already pushed contains a node whose `tree_id` is X. Updates of different trees may
//!    arrive in any order, so there is one queue per tree and no queue blocks another.
//! 2. **The first update of a tree carries its tree data** (`tree: Some`). A tree whose graft is
//!    known but whose first update lacks it is not created from that update: the update is
//!    dropped and counted.
//! 3. **A focus that cannot be followed is not pushed.** A node of the focus chain that is a
//!    graft whose subtree does not exist makes the consumer panic. An update whose focus is such
//!    a node is pushed with the tree's root focused instead, and the wanted focus is pushed as a
//!    focus-only update as soon as the chain resolves.
//! 4. **A tree whose graft is gone is gone.** When a pushed update removes a graft (replaces its
//!    tree id, or removes the node or an ancestor of it), the tree and the trees grafted inside
//!    it are forgotten: their queued updates are dropped and a late update for them is refused
//!    (the consumer would panic: "subtree does not exist").
//! 5. **No queue grows without bound.** A queue holds at most [`QUEUE_LIMIT`] updates; beyond it
//!    the shape of the foreign trees is no longer known to be complete, so all foreign state is
//!    dropped and [`Drained::reset`] tells the shell to deactivate and activate the tab again
//!    (a full tree follows).
//!
//! The state keeps the *shape* of every tree it released (node ids, children, grafts), never a
//! node's role, label or value: a page's tree is the page's text, and nothing here can print it
//! (`Debug` shows counts only).
//!
//! The design is credited to servoshell's graft queue (`ports/servoshell/desktop/gui.rs` at
//! `v0.8.0`, MPL-2.0), read for the idea; no line of it is used.
//!
//! Owner: item 1 (`graft`).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use accesskit::{ActionRequest, NodeId, TreeId, TreeUpdate};

/// The most updates that may wait for one tree. The 257th makes [`GraftState::offer`] reset.
pub const QUEUE_LIMIT: usize = 256;

/// How many forgotten tree ids are remembered, so that a late update for one is refused instead
/// of waiting for a graft that will never come.
const FORGOTTEN_LIMIT: usize = 256;

/// The longest chain of grafts a focus is followed through before it counts as unresolved.
const FOCUS_CHAIN_LIMIT: usize = 64;

/// What [`GraftState::offer`] did with an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offered {
    /// Waiting in the tree's queue; [`GraftState::take_ready`] releases it when it may go.
    Queued,
    /// Not kept, and why.
    Rejected(Rejection),
}

/// Why an update was not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// It addresses the window's own tree. A page must never be able to replace the chrome.
    RootTree,
    /// Its tree was forgotten (its graft is gone, the tab was switched away, or a reset).
    ForgottenTree,
    /// Its tree's queue is full: everything foreign was dropped and a reset is owed.
    QueueFull,
}

/// What the shell does with an action request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The window's own tree: Slint handles it.
    Own,
    /// A tree of the page: forward it to the engine.
    Foreign,
    /// A tree nobody knows (forgotten, never grafted): do nothing.
    Unknown,
}

/// What [`GraftState::take_ready`] released.
#[derive(Default)]
pub struct Drained {
    /// Updates to push, in this order, exactly as they are. May include a focus-only update of
    /// the window's own tree (rule 3).
    pub updates: Vec<TreeUpdate>,
    /// A queue overflowed since the last call: all foreign state was dropped, the shell must
    /// deactivate the page's accessibility and activate it again.
    pub reset: bool,
}

impl fmt::Debug for Drained {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Drained")
            .field("updates", &self.updates.len())
            .field("reset", &self.reset)
            .finish()
    }
}

/// Counters, for the report and the tests. Never contain content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraftStats {
    /// Foreign updates released.
    pub pushed: u64,
    /// Updates refused or dropped because their tree was forgotten.
    pub dropped_forgotten: u64,
    /// Updates that named the window's own tree.
    pub dropped_root: u64,
    /// First updates of a tree without tree data.
    pub dropped_malformed: u64,
    /// Updates dropped because a queue overflowed.
    pub dropped_overflow: u64,
    /// Updates whose focus was replaced by the tree's root until the chain resolves.
    pub focus_deferred: u64,
    /// Overflows reported through [`Drained::reset`].
    pub resets: u64,
}

/// The shape of one tree: ids, children and grafts, nothing else.
#[derive(Debug)]
struct Shape {
    root: NodeId,
    focus: NodeId,
    nodes: HashMap<NodeId, ShapeNode>,
}

#[derive(Debug)]
struct ShapeNode {
    children: Box<[NodeId]>,
    graft: Option<TreeId>,
}

/// What an update did to the grafts of a tree.
#[derive(Default)]
struct Effects {
    /// Grafts that appeared or moved to another node: tree grafted, node holding the graft.
    added: Vec<(TreeId, NodeId)>,
    /// Trees whose graft node is gone.
    removed: Vec<TreeId>,
}

impl Shape {
    fn new(root: NodeId) -> Self {
        Self {
            root,
            focus: root,
            nodes: HashMap::new(),
        }
    }

    /// Applies an update the way `accesskit_consumer` does: a node of the update replaces the
    /// node of the same id, and a child that no node of the update lists any more becomes
    /// unreachable, with everything below it.
    fn apply(&mut self, update: &TreeUpdate) -> Effects {
        let mut added: Vec<(TreeId, NodeId)> = Vec::new();
        let mut gone: HashSet<TreeId> = HashSet::new();
        let mut unreachable: HashSet<NodeId> = HashSet::new();
        let mut seen: HashSet<NodeId> = HashSet::new();
        let mut new_root = None;

        if let Some(tree) = &update.tree {
            if tree.root != self.root {
                unreachable.insert(self.root);
            }
            self.root = tree.root;
            new_root = Some(tree.root);
        }

        for (id, node) in &update.nodes {
            unreachable.remove(id);
            for child in node.children() {
                seen.insert(*child);
                unreachable.remove(child);
            }
            let graft = node.tree_id();
            match self.nodes.get_mut(id) {
                Some(old) => {
                    for child in old.children.iter() {
                        if !seen.contains(child) {
                            unreachable.insert(*child);
                        }
                    }
                    if old.graft != graft {
                        if let Some(previous) = old.graft {
                            gone.insert(previous);
                        }
                        if let Some(tree) = graft {
                            added.push((tree, *id));
                        }
                    }
                    old.children = node.children().into();
                    old.graft = graft;
                }
                None => {
                    if let Some(tree) = graft {
                        added.push((tree, *id));
                    }
                    self.nodes.insert(
                        *id,
                        ShapeNode {
                            children: node.children().into(),
                            graft,
                        },
                    );
                }
            }
        }

        for start in unreachable {
            let mut stack = vec![start];
            while let Some(id) = stack.pop() {
                let Some(node) = self.nodes.remove(&id) else {
                    continue;
                };
                if let Some(tree) = node.graft {
                    gone.insert(tree);
                }
                for child in node.children.iter() {
                    if !seen.contains(child) && new_root != Some(*child) {
                        stack.push(*child);
                    }
                }
            }
        }

        // A tree whose graft only moved to another node of the same update is still grafted.
        let removed = gone
            .into_iter()
            .filter(|tree| !added.iter().any(|(grafted, _)| grafted == tree))
            .collect();
        Effects { added, removed }
    }
}

/// Whether the head of a queue may go now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Readiness {
    Ready,
    Wait,
    Drop,
}

/// The order of a window's trees: what has been pushed, what waits, what is forgotten.
#[derive(Default)]
pub struct GraftState {
    /// Every tree pushed so far, the window's own (`TreeId::ROOT`) included.
    shapes: HashMap<TreeId, Shape>,
    /// Where each grafted tree's graft node lives: the tree that holds it, and its node.
    grafts: HashMap<TreeId, (TreeId, NodeId)>,
    /// Updates waiting for their tree's graft, one queue per tree.
    queues: HashMap<TreeId, VecDeque<TreeUpdate>>,
    /// The trees that have a queue, in the order they got one: drained in this order.
    order: Vec<TreeId>,
    /// Trees forgotten recently, oldest first.
    forgotten: VecDeque<TreeId>,
    /// Focus that was replaced by the tree's root until its chain resolves (rule 3).
    pending_focus: BTreeMap<TreeId, NodeId>,
    reset_owed: bool,
    stats: GraftStats,
}

impl fmt::Debug for GraftState {
    /// Counts only: a tree update is the page's text and is never printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraftState")
            .field("trees", &self.shapes.len())
            .field("grafts", &self.grafts.len())
            .field("queued", &self.queued())
            .field("forgotten", &self.forgotten.len())
            .field("stats", &self.stats)
            .finish()
    }
}

impl GraftState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an update of the window's own tree that is about to be pushed (every one of them,
    /// the full rebuilds and the incremental ones): learns the grafts it adds or removes and, if
    /// its focus cannot be followed yet, replaces the focus (rule 3). Push it, then call
    /// [`take_ready`](Self::take_ready) and push what that returns after it.
    ///
    /// An update without tree data before any update with it is ignored (nothing to attach it
    /// to).
    pub fn push_root(&mut self, update: &mut TreeUpdate) {
        update.tree_id = TreeId::ROOT;
        self.record(update);
    }

    /// Takes in an update of a page's tree. Nothing is pushed by this call.
    pub fn offer(&mut self, update: TreeUpdate) -> Offered {
        let tree = update.tree_id;
        if tree == TreeId::ROOT {
            self.stats.dropped_root += 1;
            return Offered::Rejected(Rejection::RootTree);
        }
        if self.forgotten.contains(&tree) {
            self.stats.dropped_forgotten += 1;
            return Offered::Rejected(Rejection::ForgottenTree);
        }
        if !self.order.contains(&tree) {
            self.order.push(tree);
        }
        let queue = self.queues.entry(tree).or_default();
        queue.push_back(update);
        if queue.len() > QUEUE_LIMIT {
            self.overflow();
            return Offered::Rejected(Rejection::QueueFull);
        }
        Offered::Queued
    }

    /// Releases every update that may be pushed now, in the order to push them, and records
    /// them as pushed. Pushing all of them, in order, is the caller's part of the contract.
    pub fn take_ready(&mut self) -> Drained {
        let mut updates = Vec::new();
        loop {
            let mut progress = false;
            for tree in self.order.clone() {
                while let Some(front) = self.queues.get(&tree).and_then(|queue| queue.front()) {
                    match self.readiness(tree, front) {
                        Readiness::Wait => break,
                        Readiness::Drop => {
                            self.queues.get_mut(&tree).and_then(VecDeque::pop_front);
                            self.stats.dropped_malformed += 1;
                            progress = true;
                        }
                        Readiness::Ready => {
                            let Some(mut update) =
                                self.queues.get_mut(&tree).and_then(VecDeque::pop_front)
                            else {
                                break;
                            };
                            self.record(&mut update);
                            self.stats.pushed += 1;
                            updates.push(update);
                            progress = true;
                        }
                    }
                }
            }
            progress |= self.restore_focus(&mut updates);
            if !progress {
                break;
            }
        }
        self.queues.retain(|_, queue| !queue.is_empty());
        let queues = &self.queues;
        self.order.retain(|tree| queues.contains_key(tree));
        let reset = std::mem::take(&mut self.reset_owed);
        if reset {
            self.stats.resets += 1;
        }
        Drained { updates, reset }
    }

    /// Forgets a tree and the trees grafted inside it: their queued updates are dropped and
    /// later ones refused. The shell calls it when it stops showing a tree (a tab switch, a
    /// deactivation); a graft that disappears from a pushed update does the same by itself.
    pub fn forget_tree(&mut self, tree: TreeId) {
        self.remove_tree(tree);
    }

    /// Forgets everything, the window's own tree included: the adapter's tree is rebuilt from a
    /// full update (assistive technology detached and attached again).
    pub fn clear(&mut self) {
        let stats = self.stats;
        *self = Self::default();
        self.stats = stats;
    }

    /// Where an action request goes. A request for a tree that is not part of what the adapter
    /// holds is never forwarded.
    pub fn route(&self, request: &ActionRequest) -> Route {
        if request.target_tree == TreeId::ROOT {
            Route::Own
        } else if self.shapes.contains_key(&request.target_tree) {
            Route::Foreign
        } else {
            Route::Unknown
        }
    }

    /// Whether the adapter holds this tree: its first update has been pushed and its graft is
    /// still there.
    pub fn is_created(&self, tree: TreeId) -> bool {
        self.shapes.contains_key(&tree)
    }

    /// Whether a graft that names this tree has been pushed (its updates may go).
    pub fn is_grafted(&self, tree: TreeId) -> bool {
        self.grafts.contains_key(&tree)
    }

    /// Whether the window's own focus is on a graft, so that the consumer's focused node lies in
    /// a page's tree. `accesskit_consumer` 0.38 cannot take the host window gaining or losing
    /// focus then: it rebuilds its focus from the *local* id of the focused node used as an id of
    /// the window's tree, and panics when the window's tree has no such node ("Focused ID ... is
    /// not in the node list"). The shell moves the window's focus off the page for the moment of
    /// the change (see `mercury_graft::before_host_focus_change`).
    pub fn root_focus_is_in_a_page(&self) -> bool {
        self.shapes.get(&TreeId::ROOT).is_some_and(|shape| {
            shape
                .nodes
                .get(&shape.focus)
                .is_some_and(|node| node.graft.is_some())
        })
    }

    /// How many updates wait, over all trees.
    pub fn queued(&self) -> usize {
        self.queues.values().map(VecDeque::len).sum()
    }

    /// How many updates wait for this tree.
    pub fn queued_for(&self, tree: TreeId) -> usize {
        self.queues.get(&tree).map_or(0, VecDeque::len)
    }

    pub fn stats(&self) -> GraftStats {
        self.stats
    }

    fn readiness(&self, tree: TreeId, update: &TreeUpdate) -> Readiness {
        if !self.grafts.contains_key(&tree) {
            return Readiness::Wait;
        }
        if !self.shapes.contains_key(&tree) && update.tree.is_none() {
            return Readiness::Drop;
        }
        Readiness::Ready
    }

    /// Records a pushed update: replaces a focus that cannot be followed, applies the update to
    /// the tree's shape and acts on the grafts it added and removed.
    fn record(&mut self, update: &mut TreeUpdate) {
        let tree = update.tree_id;
        if update.tree.is_none() && !self.shapes.contains_key(&tree) {
            return;
        }
        let wanted = update.focus;
        if self.focus_resolves(tree, wanted, &update.nodes) {
            self.pending_focus.remove(&tree);
        } else {
            let root = update
                .tree
                .as_ref()
                .map(|data| data.root)
                .or_else(|| self.shapes.get(&tree).map(|shape| shape.root))
                .unwrap_or(wanted);
            update.focus = root;
            self.pending_focus.insert(tree, wanted);
            self.stats.focus_deferred += 1;
        }
        let shape = self.shapes.entry(tree).or_insert_with(|| {
            Shape::new(update.tree.as_ref().map_or(update.focus, |data| data.root))
        });
        let effects = shape.apply(update);
        shape.focus = update.focus;

        for (grafted, node) in effects.added {
            self.grafts.insert(grafted, (tree, node));
            self.forgotten.retain(|forgotten| *forgotten != grafted);
        }
        for gone in effects.removed {
            self.remove_tree(gone);
        }
    }

    /// Whether following the focus from `node` of `tree` through grafts reaches a node that is
    /// not a graft. `own` are the nodes of the update being recorded, which are not in the shape
    /// yet.
    fn focus_resolves(
        &self,
        tree: TreeId,
        node: NodeId,
        own: &[(NodeId, accesskit::Node)],
    ) -> bool {
        let mut tree = tree;
        let mut node = node;
        let mut own = Some(own);
        for _ in 0..FOCUS_CHAIN_LIMIT {
            let in_update = own.and_then(|nodes| nodes.iter().find(|(id, _)| *id == node));
            let graft = match in_update {
                Some((_, data)) => data.tree_id(),
                None => self
                    .shapes
                    .get(&tree)
                    .and_then(|shape| shape.nodes.get(&node))
                    .and_then(|node| node.graft),
            };
            let Some(target) = graft else { return true };
            let Some(shape) = self.shapes.get(&target) else {
                return false;
            };
            tree = target;
            node = shape.focus;
            own = None;
        }
        false
    }

    /// Pushes the focus that rule 3 replaced, for every tree whose chain resolves now.
    fn restore_focus(&mut self, updates: &mut Vec<TreeUpdate>) -> bool {
        let mut progress = false;
        let wanted: Vec<(TreeId, NodeId)> = self
            .pending_focus
            .iter()
            .map(|(tree, node)| (*tree, *node))
            .collect();
        for (tree, node) in wanted {
            let still_there = self
                .shapes
                .get(&tree)
                .is_some_and(|shape| shape.nodes.contains_key(&node));
            if !still_there {
                self.pending_focus.remove(&tree);
                continue;
            }
            if self.focus_resolves(tree, node, &[]) {
                let mut update = TreeUpdate {
                    nodes: Vec::new(),
                    tree: None,
                    tree_id: tree,
                    focus: node,
                };
                self.record(&mut update);
                updates.push(update);
                progress = true;
            }
        }
        progress
    }

    fn remove_tree(&mut self, first: TreeId) {
        let mut work = vec![first];
        while let Some(tree) = work.pop() {
            if tree == TreeId::ROOT {
                continue;
            }
            self.grafts.remove(&tree);
            self.pending_focus.remove(&tree);
            if let Some(queue) = self.queues.remove(&tree) {
                self.stats.dropped_forgotten += queue.len() as u64;
            }
            self.order.retain(|queued| *queued != tree);
            if !self.forgotten.contains(&tree) {
                if self.forgotten.len() == FORGOTTEN_LIMIT {
                    self.forgotten.pop_front();
                }
                self.forgotten.push_back(tree);
            }
            if let Some(shape) = self.shapes.remove(&tree) {
                work.extend(shape.nodes.values().filter_map(|node| node.graft));
            }
        }
    }

    /// A queue is full: nothing foreign can be trusted to be complete.
    fn overflow(&mut self) {
        self.stats.dropped_overflow += self.queued() as u64;
        let trees: Vec<TreeId> = self
            .shapes
            .keys()
            .chain(self.grafts.keys())
            .chain(self.queues.keys())
            .copied()
            .filter(|tree| *tree != TreeId::ROOT)
            .collect();
        // The dropped updates are counted above, not again as forgotten ones.
        self.queues.clear();
        for tree in trees {
            self.remove_tree(tree);
        }
        self.reset_owed = true;
    }
}

/// A panic caught by [`guarded`]. It carries nothing: a payload could hold the page's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panicked;

/// Runs `work`, which hands foreign data to code that asserts (AccessKit's consumer panics on a
/// tree it cannot accept), and turns a panic into a value. The caller deactivates the page's
/// accessibility for the window and logs one line with no content; the browser keeps running.
/// The panic hook still reports the panic where the build's setting says.
pub fn guarded<T>(work: impl FnOnce() -> T) -> Result<T, Panicked> {
    catch_unwind(AssertUnwindSafe(work)).map_err(|_| Panicked)
}
