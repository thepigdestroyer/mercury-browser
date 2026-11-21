// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Which tab's page tree reaches the window: per-tab routing, active tab only (OD-4). A pure state
//! machine over tab numbers and AccessKit types; the shell executes the commands it returns and
//! reports what the engine answered.
//!
//! What the shell tells the bridge: assistive technology attached or detached ([`attach`],
//! [`detach`]), a tab shown, hidden or closed ([`tab_shown`], [`tab_hidden`], [`tab_closed`]),
//! the tree id the engine gave a tab it was told to activate ([`activated`]), and every tree
//! update a tab's web view delivered ([`update_from`]).
//!
//! What the bridge decides: a tab's page is *active* (the engine builds its tree) exactly while
//! assistive technology is attached **and** the tab is the shown one (D5 of the lane brief). Every
//! call re-reads that one rule, so a duplicate event, a switch that skips the hide, or two quick
//! switches in a row all end in the same state and produce only the commands the change needs:
//!
//! * [`Command::Deactivate`] the engine stops building the tab's tree;
//! * [`Command::Activate`] the engine starts (and answers with the tab's tree id, which the shell
//!   reports through [`activated`]);
//! * [`Command::Graft`] the window's "Page content" node now hangs this tree, or nothing.
//!
//! The commands of one call are to be executed in order. The bridge never sees a node: it
//! forwards or drops a whole [`TreeUpdate`] by the tab it came from, and counts the drops. What
//! may be pushed when (graft before subtree) is [`crate::graft`]'s part.
//!
//! [`attach`]: Bridge::attach
//! [`detach`]: Bridge::detach
//! [`tab_shown`]: Bridge::tab_shown
//! [`tab_hidden`]: Bridge::tab_hidden
//! [`tab_closed`]: Bridge::tab_closed
//! [`activated`]: Bridge::activated
//! [`update_from`]: Bridge::update_from
//!
//! Owner: item 1 (`graft`).

use accesskit::{Action, TreeId, TreeUpdate};

/// Whether an action of assistive technology counts as a user activation of the page (OD-5, D6 of
/// the lane brief): the actions that do what a click does. Focus, scrolling, setting a value and
/// the like do not: they are not a gesture, and a page must not be able to open a window after
/// one. The action channel is the embedder's only; a page cannot send an action request.
pub fn is_activating_action(action: Action) -> bool {
    matches!(
        action,
        Action::Click | Action::Expand | Action::Collapse | Action::Increment | Action::Decrement
    )
}

/// What the shell does next. Executed in the order a call returned them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Tell the engine to build this tab's accessibility tree; report the tree id it answers
    /// through [`Bridge::activated`]. The tab may be gone by now: do nothing then.
    Activate(u64),
    /// Tell the engine to stop. The tab may be gone by now: do nothing then.
    Deactivate(u64),
    /// Hang this tree from the window's "Page content" node, or nothing (`None`).
    Graft(Option<TreeId>),
}

/// How many updates passed and why the others did not. Never contain content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeStats {
    /// Updates of the active tab, forwarded.
    pub passed: u64,
    /// Updates of a tab that is not the active one (hidden, closed, or deactivated a moment ago).
    pub dropped_inactive: u64,
    /// Updates that arrived while no assistive technology is attached.
    pub dropped_detached: u64,
    /// Updates addressed to the window's own tree: a page must never replace the chrome.
    pub dropped_root: u64,
}

/// The tab the engine was told to build a tree for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Activation {
    tab: u64,
    /// The engine's answer: `None` until [`Bridge::activated`] is called, or when it declined.
    tree: Option<TreeId>,
}

/// Per-tab routing of page trees to one window.
#[derive(Debug, Default)]
pub struct Bridge {
    attached: bool,
    shown: Option<u64>,
    active: Option<Activation>,
    stats: BridgeStats,
}

impl Bridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Assistive technology attached to the window (its first tree request): the shown tab, if
    /// any, becomes active.
    pub fn attach(&mut self) -> Vec<Command> {
        self.attached = true;
        self.reconcile(false)
    }

    /// Assistive technology went away: no tab is active, nothing hangs from the page node.
    pub fn detach(&mut self) -> Vec<Command> {
        self.attached = false;
        self.reconcile(false)
    }

    /// Deactivates and activates again, from scratch: the engine sends a full tree with a new
    /// tree id. Used when the window dropped what it had queued ([`crate::graft::Drained::reset`]).
    pub fn restart(&mut self) -> Vec<Command> {
        let mut commands = self.detach();
        commands.extend(self.attach());
        commands
    }

    /// The tab is the one the window shows now. A previously shown tab that was not reported
    /// hidden is deactivated here.
    pub fn tab_shown(&mut self, tab: u64) -> Vec<Command> {
        self.shown = Some(tab);
        self.reconcile(false)
    }

    /// The tab is not shown any more. A tab that is not the shown one is ignored.
    pub fn tab_hidden(&mut self, tab: u64) -> Vec<Command> {
        if self.shown == Some(tab) {
            self.shown = None;
        }
        self.reconcile(false)
    }

    /// The tab is gone (or its web view was replaced). The engine needs no command for a web
    /// view that does not exist any more, so only the graft is withdrawn.
    pub fn tab_closed(&mut self, tab: u64) -> Vec<Command> {
        if self.shown == Some(tab) {
            self.shown = None;
        }
        let closed_active = self.active.is_some_and(|active| active.tab == tab);
        self.reconcile(closed_active)
    }

    /// The engine answered an [`Command::Activate`] with the tab's tree id (`None`: it did not
    /// activate, for example because the preference is off). Returns the graft to do, and, if
    /// the tab stopped being the active one meanwhile, the deactivation the engine still needs.
    pub fn activated(&mut self, tab: u64, tree: Option<TreeId>) -> Vec<Command> {
        match &mut self.active {
            Some(active) if active.tab == tab => {
                if active.tree == tree {
                    return Vec::new();
                }
                active.tree = tree;
                vec![Command::Graft(tree)]
            }
            _ if tree.is_some() => vec![Command::Deactivate(tab)],
            _ => Vec::new(),
        }
    }

    /// A tree update of a tab's web view. Returns it when it may go on to the window (the tab is
    /// the active one), otherwise drops it and counts why. An update of the active tab passes
    /// even before its graft is known: [`crate::graft::GraftState`] holds it until the graft is
    /// pushed, and dropping it would lose the full tree the engine sends first.
    pub fn update_from(&mut self, tab: u64, update: TreeUpdate) -> Option<TreeUpdate> {
        if !self.attached {
            self.stats.dropped_detached += 1;
            return None;
        }
        if update.tree_id == TreeId::ROOT {
            self.stats.dropped_root += 1;
            return None;
        }
        if self.active.is_some_and(|active| active.tab == tab) {
            self.stats.passed += 1;
            Some(update)
        } else {
            self.stats.dropped_inactive += 1;
            None
        }
    }

    /// Whether assistive technology is attached.
    pub fn is_attached(&self) -> bool {
        self.attached
    }

    /// The tab the window shows (the engine may or may not build its tree).
    pub fn shown_tab(&self) -> Option<u64> {
        self.shown
    }

    /// The tab whose tree the engine builds for the window.
    pub fn active_tab(&self) -> Option<u64> {
        self.active.map(|active| active.tab)
    }

    /// The tree hanging from the page node now.
    pub fn grafted_tree(&self) -> Option<TreeId> {
        self.active.and_then(|active| active.tree)
    }

    pub fn stats(&self) -> BridgeStats {
        self.stats
    }

    /// Makes the active tab the one the rule says (`attached` and shown), returning the
    /// commands that change needs. `active_is_gone`: the tab being replaced no longer exists, so
    /// it is not deactivated.
    fn reconcile(&mut self, active_is_gone: bool) -> Vec<Command> {
        let wanted = if self.attached { self.shown } else { None };
        let current = self.active.map(|active| active.tab);
        if wanted == current {
            return Vec::new();
        }
        let mut commands = Vec::new();
        if let Some(active) = self.active.take() {
            if !active_is_gone {
                commands.push(Command::Deactivate(active.tab));
            }
            commands.push(Command::Graft(None));
        }
        if let Some(tab) = wanted {
            self.active = Some(Activation { tab, tree: None });
            commands.push(Command::Activate(tab));
        }
        commands
    }
}
