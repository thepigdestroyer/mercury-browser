// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Which tab's page tree reaches the window (`mercury_a11y::bridge`): attach and detach,
//! switches, closes, stale answers and stale updates, duplicate events, and a long generated
//! run that checks the one rule after every event: the engine builds a tab's tree exactly while
//! assistive technology is attached and the tab is the shown one, and for no other tab.

use std::collections::HashSet;

use accesskit::{Node, NodeId, Role, Tree, TreeId, TreeUpdate, Uuid};
use mercury_a11y::bridge::{Bridge, Command};

const A: u64 = 1;
const B: u64 = 2;
const C: u64 = 3;

fn tree(n: u128) -> TreeId {
    TreeId(Uuid::from_u128(n))
}

/// A one-node tree update for `tree_id`.
fn update(tree_id: TreeId) -> TreeUpdate {
    TreeUpdate {
        nodes: vec![(NodeId(0), Node::new(Role::ScrollView))],
        tree: Some(Tree::new(NodeId(0))),
        tree_id,
        focus: NodeId(0),
    }
}

use Command::{Activate, Deactivate, Graft};

#[test]
fn attaching_with_no_tab_shown_does_nothing() {
    let mut bridge = Bridge::new();
    assert_eq!(bridge.attach(), []);
    assert!(bridge.is_attached());
    assert_eq!(bridge.active_tab(), None);
}

#[test]
fn attaching_after_the_first_tab_is_shown_activates_it() {
    let mut bridge = Bridge::new();
    assert_eq!(bridge.tab_shown(A), []);
    assert_eq!(bridge.shown_tab(), Some(A));
    assert_eq!(bridge.attach(), [Activate(A)]);
    assert_eq!(bridge.active_tab(), Some(A));
}

#[test]
fn showing_the_first_tab_after_attaching_activates_it() {
    let mut bridge = Bridge::new();
    assert_eq!(bridge.attach(), []);
    assert_eq!(bridge.tab_shown(A), [Activate(A)]);
}

#[test]
fn a_tab_shown_while_detached_is_not_activated() {
    let mut bridge = Bridge::new();
    assert_eq!(bridge.tab_shown(A), []);
    assert_eq!(bridge.tab_shown(B), []);
    assert_eq!(bridge.active_tab(), None);
    // The tab shown when the technology attaches is the one that counts.
    assert_eq!(bridge.attach(), [Activate(B)]);
}

#[test]
fn detaching_deactivates_the_tab_and_withdraws_the_graft() {
    let mut bridge = Bridge::new();
    bridge.tab_shown(A);
    bridge.attach();
    bridge.activated(A, Some(tree(10)));
    assert_eq!(bridge.detach(), [Deactivate(A), Graft(None)]);
    assert_eq!(bridge.active_tab(), None);
    assert_eq!(bridge.grafted_tree(), None);
    assert!(!bridge.is_attached());
    // The shown tab is remembered: attaching again activates it again.
    assert_eq!(bridge.attach(), [Activate(A)]);
}

#[test]
fn detaching_twice_or_without_a_tab_does_nothing() {
    let mut bridge = Bridge::new();
    assert_eq!(bridge.detach(), []);
    bridge.attach();
    assert_eq!(bridge.detach(), []);
    assert_eq!(bridge.detach(), []);
}

#[test]
fn a_duplicate_attach_or_show_gives_no_second_activation() {
    let mut bridge = Bridge::new();
    bridge.tab_shown(A);
    assert_eq!(bridge.attach(), [Activate(A)]);
    assert_eq!(bridge.attach(), []);
    assert_eq!(bridge.tab_shown(A), []);
    assert_eq!(bridge.tab_shown(A), []);
    assert_eq!(bridge.active_tab(), Some(A));
}

#[test]
fn switching_tabs_deactivates_the_old_one_before_activating_the_new_one() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.activated(A, Some(tree(10)));
    // The desktop hides the previous tab, then shows the new one.
    assert_eq!(bridge.tab_hidden(A), [Deactivate(A), Graft(None)]);
    assert_eq!(bridge.tab_shown(B), [Activate(B)]);
    assert_eq!(bridge.activated(B, Some(tree(20))), [Graft(Some(tree(20)))]);
    assert_eq!(bridge.active_tab(), Some(B));
    assert_eq!(bridge.grafted_tree(), Some(tree(20)));
}

#[test]
fn a_switch_that_skips_the_hide_still_deactivates_the_old_tab() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    assert_eq!(
        bridge.tab_shown(B),
        [Deactivate(A), Graft(None), Activate(B)]
    );
}

#[test]
fn hiding_a_tab_that_is_not_the_shown_one_is_ignored() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    assert_eq!(bridge.tab_hidden(B), []);
    assert_eq!(bridge.active_tab(), Some(A));
}

#[test]
fn hiding_then_showing_the_same_tab_activates_it_again() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    assert_eq!(bridge.tab_hidden(A), [Deactivate(A), Graft(None)]);
    assert_eq!(bridge.tab_shown(A), [Activate(A)]);
}

#[test]
fn closing_the_active_tab_withdraws_the_graft_without_a_deactivation() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.activated(A, Some(tree(10)));
    // The web view is gone: the engine needs no command for it.
    assert_eq!(bridge.tab_closed(A), [Graft(None)]);
    assert_eq!(bridge.active_tab(), None);
    assert_eq!(bridge.shown_tab(), None);
    // The next tab shown is activated.
    assert_eq!(bridge.tab_shown(B), [Activate(B)]);
}

#[test]
fn closing_a_background_tab_does_nothing() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    assert_eq!(bridge.tab_closed(B), []);
    assert_eq!(bridge.active_tab(), Some(A));
}

#[test]
fn a_replaced_web_view_of_the_same_tab_is_activated_again() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.activated(A, Some(tree(10)));
    // The desktop replaces a page that never committed: closed, then shown again.
    assert_eq!(bridge.tab_closed(A), [Graft(None)]);
    assert_eq!(bridge.tab_shown(A), [Activate(A)]);
    assert_eq!(bridge.activated(A, Some(tree(11))), [Graft(Some(tree(11)))]);
}

#[test]
fn closing_while_detached_only_forgets_the_shown_tab() {
    let mut bridge = Bridge::new();
    bridge.tab_shown(A);
    assert_eq!(bridge.tab_closed(A), []);
    assert_eq!(bridge.shown_tab(), None);
    assert_eq!(bridge.attach(), []);
}

#[test]
fn the_engines_answer_is_grafted() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    assert_eq!(bridge.grafted_tree(), None);
    assert_eq!(bridge.activated(A, Some(tree(10))), [Graft(Some(tree(10)))]);
    assert_eq!(bridge.grafted_tree(), Some(tree(10)));
}

#[test]
fn an_engine_that_did_not_activate_gives_no_graft() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    assert_eq!(bridge.activated(A, None), []);
    assert_eq!(bridge.grafted_tree(), None);
    // Its tab is still the active one: its updates (if any come) pass.
    assert!(bridge.update_from(A, update(tree(10))).is_some());
}

#[test]
fn an_answer_for_a_tab_that_was_switched_away_is_undone() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.tab_shown(B);
    // The answer for A arrives after B was shown: A must not stay active in the engine, and
    // nothing is grafted for it.
    assert_eq!(bridge.activated(A, Some(tree(10))), [Deactivate(A)]);
    assert_eq!(bridge.grafted_tree(), None);
    assert_eq!(bridge.active_tab(), Some(B));
    // A stale answer that declined needs nothing.
    assert_eq!(bridge.activated(A, None), []);
}

#[test]
fn a_repeated_answer_grafts_only_what_changed() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    assert_eq!(bridge.activated(A, Some(tree(10))), [Graft(Some(tree(10)))]);
    assert_eq!(bridge.activated(A, Some(tree(10))), []);
    assert_eq!(bridge.activated(A, Some(tree(11))), [Graft(Some(tree(11)))]);
}

#[test]
fn updates_of_the_active_tab_pass_unchanged() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.activated(A, Some(tree(10)));
    let sent = update(tree(10));
    assert_eq!(bridge.update_from(A, sent.clone()), Some(sent));
    assert_eq!(bridge.stats().passed, 1);
}

#[test]
fn updates_of_another_tab_are_dropped_and_counted() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.activated(A, Some(tree(10)));
    assert_eq!(bridge.update_from(B, update(tree(20))), None);
    assert_eq!(bridge.update_from(C, update(tree(30))), None);
    assert_eq!(bridge.stats().dropped_inactive, 2);
    assert_eq!(bridge.stats().passed, 0);
}

#[test]
fn a_stale_update_of_the_previous_tab_after_a_switch_is_dropped() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.activated(A, Some(tree(10)));
    assert!(bridge.update_from(A, update(tree(10))).is_some());
    bridge.tab_hidden(A);
    bridge.tab_shown(B);
    bridge.activated(B, Some(tree(20)));
    // The engine's deactivation of A is asynchronous: updates it had queued still arrive.
    assert_eq!(bridge.update_from(A, update(tree(10))), None);
    assert!(bridge.update_from(B, update(tree(20))).is_some());
    let stats = bridge.stats();
    assert_eq!((stats.passed, stats.dropped_inactive), (2, 1));
}

#[test]
fn updates_while_detached_are_dropped_and_counted() {
    let mut bridge = Bridge::new();
    bridge.tab_shown(A);
    assert_eq!(bridge.update_from(A, update(tree(10))), None);
    bridge.attach();
    bridge.detach();
    assert_eq!(bridge.update_from(A, update(tree(10))), None);
    assert_eq!(bridge.stats().dropped_detached, 2);
}

#[test]
fn an_update_for_the_windows_own_tree_never_passes() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.activated(A, Some(tree(10)));
    assert_eq!(bridge.update_from(A, update(TreeId::ROOT)), None);
    assert_eq!(bridge.stats().dropped_root, 1);
}

#[test]
fn an_update_before_the_graft_is_known_still_passes() {
    // The window's queue holds it until the graft is pushed; dropping it would lose the full
    // tree the engine sends first.
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    assert!(bridge.update_from(A, update(tree(10))).is_some());
    bridge.activated(A, Some(tree(10)));
    assert!(bridge.update_from(A, update(tree(10))).is_some());
}

#[test]
fn two_quick_switches_leave_only_the_last_tab_active() {
    let mut bridge = Bridge::new();
    bridge.attach();
    assert_eq!(bridge.tab_shown(A), [Activate(A)]);
    assert_eq!(
        bridge.tab_shown(B),
        [Deactivate(A), Graft(None), Activate(B)]
    );
    assert_eq!(
        bridge.tab_shown(C),
        [Deactivate(B), Graft(None), Activate(C)]
    );
    // The engine's answers come back late and in order; only C's is grafted.
    assert_eq!(bridge.activated(A, Some(tree(10))), [Deactivate(A)]);
    assert_eq!(bridge.activated(B, Some(tree(20))), [Deactivate(B)]);
    assert_eq!(bridge.activated(C, Some(tree(30))), [Graft(Some(tree(30)))]);
    assert_eq!(bridge.grafted_tree(), Some(tree(30)));
    assert!(bridge.update_from(C, update(tree(30))).is_some());
    assert_eq!(bridge.update_from(A, update(tree(10))), None);
}

#[test]
fn a_restart_deactivates_and_activates_the_tab_from_scratch() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.activated(A, Some(tree(10)));
    assert_eq!(bridge.restart(), [Deactivate(A), Graft(None), Activate(A)]);
    assert_eq!(bridge.grafted_tree(), None);
    assert_eq!(bridge.activated(A, Some(tree(11))), [Graft(Some(tree(11)))]);
    assert!(bridge.is_attached());
}

#[test]
fn a_restart_while_nothing_is_active_does_nothing() {
    let mut bridge = Bridge::new();
    assert_eq!(bridge.restart(), []);
    assert!(bridge.is_attached());
    assert_eq!(bridge.tab_shown(A), [Activate(A)]);
}

#[test]
fn switching_while_detached_activates_the_new_tab_on_attach() {
    let mut bridge = Bridge::new();
    bridge.attach();
    bridge.tab_shown(A);
    bridge.detach();
    assert_eq!(bridge.tab_hidden(A), []);
    assert_eq!(bridge.tab_shown(C), []);
    assert_eq!(bridge.attach(), [Activate(C)]);
}

#[test]
fn the_counters_add_up() {
    let mut bridge = Bridge::new();
    bridge.tab_shown(A);
    bridge.update_from(A, update(tree(10))); // detached
    bridge.attach();
    bridge.update_from(A, update(tree(10))); // passes
    bridge.update_from(B, update(tree(20))); // inactive
    bridge.update_from(A, update(TreeId::ROOT)); // root
    let stats = bridge.stats();
    assert_eq!(
        (
            stats.passed,
            stats.dropped_inactive,
            stats.dropped_detached,
            stats.dropped_root
        ),
        (1, 1, 1, 1)
    );
}

/// What the engine would believe if it executed every command: which tabs are active, and what
/// hangs from the page node.
#[derive(Default)]
struct Shell {
    engine_active: HashSet<u64>,
    open: HashSet<u64>,
    grafted: Option<TreeId>,
    next_tree: u128,
}

impl Shell {
    /// Executes commands like the desktop does, answering each `Activate` at once.
    fn run(&mut self, bridge: &mut Bridge, commands: Vec<Command>) {
        let mut queue = std::collections::VecDeque::from(commands);
        while let Some(command) = queue.pop_front() {
            match command {
                Activate(tab) => {
                    if self.open.contains(&tab) {
                        self.engine_active.insert(tab);
                        self.next_tree += 1;
                        let answer = Some(tree(1000 + self.next_tree));
                        queue.extend(bridge.activated(tab, answer));
                    }
                }
                Deactivate(tab) => {
                    self.engine_active.remove(&tab);
                }
                Graft(tree) => self.grafted = tree,
            }
        }
    }
}

#[test]
fn a_long_generated_run_keeps_one_rule() {
    // A fixed pseudo-random order of events (no randomness source: the run is the same every
    // time). After every event the engine builds a tree for the shown tab while attached, and
    // for no other tab; the page node holds that tab's tree or nothing.
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut bridge = Bridge::new();
    let mut shell = Shell::default();
    for tab in 1..=4 {
        shell.open.insert(tab);
    }
    for step in 0..4000 {
        let tab = 1 + next(4);
        let commands = match next(7) {
            0 => bridge.attach(),
            1 => bridge.detach(),
            2 => bridge.tab_shown(tab),
            3 => bridge.tab_hidden(tab),
            4 => {
                shell.open.remove(&tab);
                shell.engine_active.remove(&tab);
                let commands = bridge.tab_closed(tab);
                shell.open.insert(tab); // a new tab takes the number (a replaced web view)
                commands
            }
            5 => bridge.restart(),
            _ => {
                // A late update of some tab: passes only for the active one.
                let passed = bridge.update_from(tab, update(tree(7))).is_some();
                assert_eq!(passed, bridge.active_tab() == Some(tab), "step {step}");
                Vec::new()
            }
        };
        shell.run(&mut bridge, commands);

        let wanted = if bridge.is_attached() {
            bridge.shown_tab()
        } else {
            None
        };
        assert_eq!(bridge.active_tab(), wanted, "step {step}");
        let expected: HashSet<u64> = wanted.into_iter().collect();
        assert_eq!(shell.engine_active, expected, "step {step}");
        assert_eq!(shell.grafted, bridge.grafted_tree(), "step {step}");
        if wanted.is_some() {
            assert!(
                shell.grafted.is_some(),
                "step {step}: the answer is grafted"
            );
        } else {
            assert_eq!(shell.grafted, None, "step {step}");
        }
    }
}

#[test]
fn only_the_actions_that_click_count_as_a_user_activation() {
    use accesskit::Action;
    use mercury_a11y::bridge::is_activating_action;

    for action in [
        Action::Click,
        Action::Expand,
        Action::Collapse,
        Action::Increment,
        Action::Decrement,
    ] {
        assert!(is_activating_action(action), "{action:?}");
    }
    for action in [
        Action::Focus,
        Action::Blur,
        Action::SetValue,
        Action::ReplaceSelectedText,
        Action::ScrollIntoView,
        Action::ScrollUp,
        Action::ScrollDown,
        Action::ScrollLeft,
        Action::ScrollRight,
        Action::ShowContextMenu,
        Action::SetTextSelection,
    ] {
        assert!(!is_activating_action(action), "{action:?}");
    }
}
