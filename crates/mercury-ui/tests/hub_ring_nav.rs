// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The hub ring's key navigation (`hub_ring::nav`), every key of the SPEC §5 table on the 22-tab
//! mockup ring with its three pinned beads. Pure Rust, no window. Needs `--features test-support`
//! for the fixtures.
//!
//! Mockup ring: group 1 (key A) has tabs 101..=105, group 2 (S) 106..=109, group 3 (D) 110..=115,
//! group 4 (F) 116..=118, group 5 (G) 119..=120, group 6 (H) 121..=122; the beads are 200..=202.

use mercury_ui::hub_ring::LOOSE_GROUP_ID;
use mercury_ui::hub_ring::fixtures;
use mercury_ui::hub_ring::nav::{Cursor, FocusOrder, Intent, NavKey, NavOutcome, NavState, step};

/// A focus order plus the state the controller keeps, so a test reads as a key sequence.
struct Run {
    order: FocusOrder,
    state: NavState,
}

impl Run {
    fn new(order: FocusOrder, cursor: Cursor) -> Self {
        Self {
            order,
            state: NavState {
                cursor,
                ..NavState::default()
            },
        }
    }

    fn mockup(cursor: Cursor) -> Self {
        Self::new(
            fixtures::focus_order(&fixtures::mockup_input(false)),
            cursor,
        )
    }

    fn key(&mut self, key: NavKey) -> NavOutcome {
        let outcome = step(&self.order, &self.state, key);
        self.state = outcome.state;
        outcome
    }

    fn cursor(&self) -> Cursor {
        self.state.cursor
    }
}

fn order(tabs: &[(i32, i32)], beads: &[i32]) -> FocusOrder {
    let mut groups: Vec<(i32, Option<char>)> = Vec::new();
    for &(_, group) in tabs {
        if !groups.iter().any(|&(id, _)| id == group) {
            groups.push((group, None));
        }
    }
    FocusOrder::new(tabs, &groups, beads)
}

// ---- the fixture is what the tests below assume ------------------------------------------

#[test]
fn fixture_order_is_22_tabs_6_groups_3_beads() {
    let order = fixtures::focus_order(&fixtures::mockup_input(false));
    assert_eq!(order.tabs.len(), 22);
    assert_eq!(order.groups.len(), 6);
    assert_eq!(order.beads, vec![200, 201, 202]);
    assert_eq!(order.tabs[0].id, 101);
    assert_eq!(order.tabs[21].id, 122);
    let keys: Vec<Option<char>> = order.groups.iter().map(|g| g.key).collect();
    assert_eq!(
        keys,
        ['A', 'S', 'D', 'F', 'G', 'H'].map(Some).to_vec(),
        "keys are upper case"
    );
}

// ---- ← → ----------------------------------------------------------------------------------

#[test]
fn right_walks_every_tab_then_every_bead_then_wraps() {
    let mut run = Run::mockup(Cursor::Tab(101));
    let mut seen = vec![run.cursor()];
    for _ in 0..24 {
        let outcome = run.key(NavKey::Right);
        assert!(outcome.handled);
        assert_eq!(outcome.intent, None, "walking asks the host for nothing");
        seen.push(run.cursor());
    }
    let expected: Vec<Cursor> = (101..=122)
        .map(Cursor::Tab)
        .chain((200..=202).map(Cursor::Bead))
        .collect();
    assert_eq!(seen, expected, "22 tabs in ring order, then 3 beads");
    run.key(NavKey::Right);
    assert_eq!(run.cursor(), Cursor::Tab(101), "after the last bead: wrap");
}

#[test]
fn left_walks_back_through_beads_into_the_last_tab_and_wraps() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.key(NavKey::Left);
    assert_eq!(run.cursor(), Cursor::Bead(202), "left of the first tab");
    run.key(NavKey::Left);
    assert_eq!(run.cursor(), Cursor::Bead(201));
    run.key(NavKey::Left);
    assert_eq!(run.cursor(), Cursor::Bead(200));
    run.key(NavKey::Left);
    assert_eq!(run.cursor(), Cursor::Tab(122), "left of the first bead");
    run.key(NavKey::Left);
    assert_eq!(run.cursor(), Cursor::Tab(121));
}

#[test]
fn from_no_cursor_right_is_the_first_tab_and_left_the_last_tab() {
    let mut run = Run::mockup(Cursor::None);
    assert!(run.key(NavKey::Right).handled);
    assert_eq!(run.cursor(), Cursor::Tab(101));
    let mut run = Run::mockup(Cursor::None);
    assert!(run.key(NavKey::Left).handled);
    assert_eq!(run.cursor(), Cursor::Tab(122));
}

#[test]
fn without_beads_the_arrows_wrap_among_the_tabs() {
    let mut run = Run::new(order(&[(1, 1), (2, 1), (3, 2)], &[]), Cursor::Tab(3));
    run.key(NavKey::Right);
    assert_eq!(run.cursor(), Cursor::Tab(1));
    run.key(NavKey::Left);
    assert_eq!(run.cursor(), Cursor::Tab(3));
}

#[test]
fn with_only_beads_the_arrows_walk_the_beads() {
    let mut run = Run::new(order(&[], &[7, 8]), Cursor::None);
    run.key(NavKey::Right);
    assert_eq!(run.cursor(), Cursor::Bead(7));
    run.key(NavKey::Right);
    assert_eq!(run.cursor(), Cursor::Bead(8));
    run.key(NavKey::Right);
    assert_eq!(run.cursor(), Cursor::Bead(7));
    run.key(NavKey::Left);
    assert_eq!(run.cursor(), Cursor::Bead(8));
}

#[test]
fn an_empty_ring_swallows_the_arrows_and_keeps_no_cursor() {
    let mut run = Run::new(FocusOrder::default(), Cursor::None);
    for key in [NavKey::Left, NavKey::Right, NavKey::Up, NavKey::Down] {
        let outcome = run.key(key);
        assert!(outcome.handled);
        assert_eq!(outcome.intent, None);
        assert_eq!(run.cursor(), Cursor::None);
    }
}

#[test]
fn a_cursor_on_a_tab_that_is_gone_starts_over_like_no_cursor() {
    let mut run = Run::mockup(Cursor::Tab(999));
    run.key(NavKey::Right);
    assert_eq!(run.cursor(), Cursor::Tab(101));
}

// ---- ↑ ↓ ----------------------------------------------------------------------------------

#[test]
fn down_goes_to_the_first_tab_of_the_next_group_not_the_same_slot() {
    let mut run = Run::mockup(Cursor::Tab(103));
    run.key(NavKey::Down);
    assert_eq!(run.cursor(), Cursor::Tab(106), "group 2's first tab");
    run.key(NavKey::Down);
    assert_eq!(run.cursor(), Cursor::Tab(110));
    run.key(NavKey::Down);
    run.key(NavKey::Down);
    run.key(NavKey::Down);
    assert_eq!(run.cursor(), Cursor::Tab(121), "group 6's first tab");
    run.key(NavKey::Down);
    assert_eq!(run.cursor(), Cursor::Tab(101), "after the last group: wrap");
}

#[test]
fn up_goes_to_the_first_tab_of_the_previous_group_with_wrap() {
    let mut run = Run::mockup(Cursor::Tab(113));
    run.key(NavKey::Up);
    assert_eq!(run.cursor(), Cursor::Tab(106), "group 2's first tab");
    run.key(NavKey::Up);
    assert_eq!(run.cursor(), Cursor::Tab(101));
    run.key(NavKey::Up);
    assert_eq!(
        run.cursor(),
        Cursor::Tab(121),
        "before the first group: wrap"
    );
}

#[test]
fn up_and_down_from_a_bead_or_no_cursor_enter_the_first_or_last_group() {
    for start in [Cursor::Bead(201), Cursor::None] {
        let mut run = Run::mockup(start);
        run.key(NavKey::Down);
        assert_eq!(
            run.cursor(),
            Cursor::Tab(101),
            "down: first group's first tab"
        );
        let mut run = Run::mockup(start);
        run.key(NavKey::Up);
        assert_eq!(run.cursor(), Cursor::Tab(121), "up: last group's first tab");
    }
}

#[test]
fn up_and_down_with_one_group_stay_on_its_first_tab() {
    let mut run = Run::new(order(&[(1, 5), (2, 5), (3, 5)], &[]), Cursor::Tab(3));
    run.key(NavKey::Down);
    assert_eq!(run.cursor(), Cursor::Tab(1));
    let mut run = Run::new(order(&[(1, 5), (2, 5), (3, 5)], &[]), Cursor::Tab(3));
    run.key(NavKey::Up);
    assert_eq!(run.cursor(), Cursor::Tab(1));
}

// ---- ⏎ -----------------------------------------------------------------------------------

#[test]
fn enter_activates_the_tab_or_the_bead_and_nothing_without_a_cursor() {
    let mut run = Run::mockup(Cursor::Tab(108));
    let outcome = run.key(NavKey::Enter);
    assert_eq!(outcome.intent, Some(Intent::Activate(108)));
    assert!(outcome.handled);
    assert_eq!(run.cursor(), Cursor::Tab(108));

    let mut run = Run::mockup(Cursor::Bead(201));
    assert_eq!(
        run.key(NavKey::Enter).intent,
        Some(Intent::ActivateBead(201))
    );

    let mut run = Run::mockup(Cursor::None);
    assert_eq!(run.key(NavKey::Enter).intent, None);
}

// ---- ⌘⌫ -----------------------------------------------------------------------------------

#[test]
fn close_tab_moves_the_cursor_to_the_next_tab() {
    let mut run = Run::mockup(Cursor::Tab(105));
    let outcome = run.key(NavKey::CloseTab);
    assert_eq!(outcome.intent, Some(Intent::Close(105)));
    assert!(outcome.handled);
    assert_eq!(
        run.cursor(),
        Cursor::Tab(106),
        "the next tab in ring order, across the group edge"
    );
}

#[test]
fn close_the_last_tab_moves_the_cursor_to_the_previous_tab_not_a_bead() {
    let mut run = Run::mockup(Cursor::Tab(122));
    let outcome = run.key(NavKey::CloseTab);
    assert_eq!(outcome.intent, Some(Intent::Close(122)));
    assert_eq!(run.cursor(), Cursor::Tab(121));
}

#[test]
fn close_the_only_tab_leaves_no_cursor() {
    let mut run = Run::new(order(&[(1, 1)], &[7]), Cursor::Tab(1));
    let outcome = run.key(NavKey::CloseTab);
    assert_eq!(outcome.intent, Some(Intent::Close(1)));
    assert_eq!(run.cursor(), Cursor::None);
}

#[test]
fn close_a_bead_moves_to_the_next_bead_else_the_previous_else_the_last_tab() {
    let mut run = Run::mockup(Cursor::Bead(200));
    assert_eq!(run.key(NavKey::CloseTab).intent, Some(Intent::Close(200)));
    assert_eq!(run.cursor(), Cursor::Bead(201), "next bead");

    let mut run = Run::mockup(Cursor::Bead(202));
    assert_eq!(run.key(NavKey::CloseTab).intent, Some(Intent::Close(202)));
    assert_eq!(
        run.cursor(),
        Cursor::Bead(201),
        "last bead: the previous one"
    );

    let mut run = Run::new(order(&[(1, 1), (2, 1)], &[9]), Cursor::Bead(9));
    assert_eq!(run.key(NavKey::CloseTab).intent, Some(Intent::Close(9)));
    assert_eq!(run.cursor(), Cursor::Tab(2), "the only bead: the last tab");

    let mut run = Run::new(order(&[], &[9]), Cursor::Bead(9));
    run.key(NavKey::CloseTab);
    assert_eq!(run.cursor(), Cursor::None, "nothing left");
}

#[test]
fn close_without_a_cursor_does_nothing() {
    let mut run = Run::mockup(Cursor::None);
    assert_eq!(run.key(NavKey::CloseTab).intent, None);
    assert_eq!(run.cursor(), Cursor::None);
}

// ---- ⌥← ⌥→ (the index is the insertion slot, counted with the tab still in place) ---------

#[test]
fn move_left_inserts_one_slot_earlier() {
    // Tab 103 is the third tab (position 2) of group 1 (5 tabs): slot 1.
    let mut run = Run::mockup(Cursor::Tab(103));
    let outcome = run.key(NavKey::MoveLeft);
    assert_eq!(
        outcome.intent,
        Some(Intent::Move {
            tab: 103,
            group: 1,
            index: 1
        })
    );
    assert!(outcome.handled);
    assert_eq!(run.cursor(), Cursor::Tab(103), "the focus stays on the tab");
}

#[test]
fn move_right_inserts_two_slots_later_because_the_tab_is_still_in_place() {
    let mut run = Run::mockup(Cursor::Tab(103));
    assert_eq!(
        run.key(NavKey::MoveRight).intent,
        Some(Intent::Move {
            tab: 103,
            group: 1,
            index: 4
        })
    );
    // Tab 104 is at position 3 of 5: slot 5 is the end of the group.
    let mut run = Run::mockup(Cursor::Tab(104));
    assert_eq!(
        run.key(NavKey::MoveRight).intent,
        Some(Intent::Move {
            tab: 104,
            group: 1,
            index: 5
        })
    );
    // Position 1 moving left goes to slot 0.
    let mut run = Run::mockup(Cursor::Tab(102));
    assert_eq!(
        run.key(NavKey::MoveLeft).intent,
        Some(Intent::Move {
            tab: 102,
            group: 1,
            index: 0
        })
    );
}

#[test]
fn move_at_the_ends_of_a_group_is_handled_without_an_intent() {
    for (tab, key) in [
        (101, NavKey::MoveLeft),
        (105, NavKey::MoveRight),
        (106, NavKey::MoveLeft),
        (122, NavKey::MoveRight),
    ] {
        let mut run = Run::mockup(Cursor::Tab(tab));
        let outcome = run.key(key);
        assert!(outcome.handled, "tab {tab} {key:?}");
        assert_eq!(outcome.intent, None, "tab {tab} {key:?}");
        assert_eq!(run.cursor(), Cursor::Tab(tab));
    }
}

#[test]
fn a_group_of_one_tab_cannot_move_within_itself() {
    let mut run = Run::new(order(&[(1, 1), (2, 2)], &[]), Cursor::Tab(2));
    for key in [NavKey::MoveLeft, NavKey::MoveRight] {
        let outcome = run.key(key);
        assert!(outcome.handled);
        assert_eq!(outcome.intent, None);
    }
}

// ---- ⌥↑ ⌥↓ -------------------------------------------------------------------------------

#[test]
fn move_to_the_next_group_goes_to_its_end_slot() {
    // Group 2 has 4 tabs (106..=109) and group 3 has 6.
    let mut run = Run::mockup(Cursor::Tab(108));
    assert_eq!(
        run.key(NavKey::MoveToNextGroup).intent,
        Some(Intent::Move {
            tab: 108,
            group: 3,
            index: 6
        })
    );
    assert_eq!(run.cursor(), Cursor::Tab(108));
    assert_eq!(
        run.key(NavKey::MoveToPrevGroup).intent,
        Some(Intent::Move {
            tab: 108,
            group: 1,
            index: 5
        })
    );
}

#[test]
fn move_to_a_group_does_not_wrap() {
    let mut run = Run::mockup(Cursor::Tab(103));
    let outcome = run.key(NavKey::MoveToPrevGroup);
    assert!(outcome.handled);
    assert_eq!(outcome.intent, None, "first group, previous");

    let mut run = Run::mockup(Cursor::Tab(122));
    let outcome = run.key(NavKey::MoveToNextGroup);
    assert!(outcome.handled);
    assert_eq!(outcome.intent, None, "last group, next");
}

#[test]
fn move_to_and_from_the_loose_group() {
    let order = FocusOrder::new(
        &[(1, 1), (2, 1), (3, LOOSE_GROUP_ID), (4, LOOSE_GROUP_ID)],
        &[(1, Some('A')), (LOOSE_GROUP_ID, None)],
        &[],
    );
    let mut run = Run::new(order.clone(), Cursor::Tab(2));
    assert_eq!(
        run.key(NavKey::MoveToNextGroup).intent,
        Some(Intent::Move {
            tab: 2,
            group: LOOSE_GROUP_ID,
            index: 2
        })
    );
    let mut run = Run::new(order, Cursor::Tab(3));
    assert_eq!(
        run.key(NavKey::MoveToPrevGroup).intent,
        Some(Intent::Move {
            tab: 3,
            group: 1,
            index: 2
        })
    );
}

#[test]
fn beads_and_no_cursor_cannot_be_moved() {
    for start in [Cursor::Bead(200), Cursor::None] {
        for key in [
            NavKey::MoveLeft,
            NavKey::MoveRight,
            NavKey::MoveToPrevGroup,
            NavKey::MoveToNextGroup,
        ] {
            let mut run = Run::mockup(start);
            let outcome = run.key(key);
            assert!(outcome.handled, "{start:?} {key:?}");
            assert_eq!(outcome.intent, None, "{start:?} {key:?}");
            assert_eq!(run.cursor(), start);
        }
    }
}

// ---- M ------------------------------------------------------------------------------------

#[test]
fn mute_names_the_focused_tab_or_bead() {
    let mut run = Run::mockup(Cursor::Tab(115));
    let outcome = run.key(NavKey::Mute);
    assert_eq!(outcome.intent, Some(Intent::Mute(115)));
    assert!(outcome.handled);
    let mut run = Run::mockup(Cursor::Bead(200));
    assert_eq!(run.key(NavKey::Mute).intent, Some(Intent::Mute(200)));
    let mut run = Run::mockup(Cursor::None);
    assert_eq!(run.key(NavKey::Mute).intent, None);
}

// ---- typing and Esc -----------------------------------------------------------------------

#[test]
fn a_printable_key_goes_to_the_query() {
    let mut run = Run::mockup(Cursor::Tab(103));
    let outcome = run.key(NavKey::Type('t'));
    assert_eq!(outcome.intent, Some(Intent::TypeChar('t')));
    assert!(outcome.handled);
    assert!(run.state.query_active, "the query now has text");
    assert_eq!(run.cursor(), Cursor::Tab(103), "typing keeps the focus");
}

#[test]
fn escape_clears_the_query_first_and_closes_the_second_time() {
    let mut run = Run::mockup(Cursor::Tab(103));
    run.key(NavKey::Type('t'));
    run.key(NavKey::Type('r'));
    let first = run.key(NavKey::Escape);
    assert_eq!(first.intent, Some(Intent::ClearQuery));
    assert!(first.handled);
    assert!(!run.state.query_active);
    let second = run.key(NavKey::Escape);
    assert_eq!(second.intent, Some(Intent::ClosePopover));
    assert!(second.handled);
}

#[test]
fn escape_without_a_query_closes_the_popover() {
    let mut run = Run::mockup(Cursor::None);
    let outcome = run.key(NavKey::Escape);
    assert_eq!(outcome.intent, Some(Intent::ClosePopover));
    assert!(outcome.handled);
}

#[test]
fn escape_with_a_query_the_controller_set_clears_it() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.state.query_active = true;
    assert_eq!(run.key(NavKey::Escape).intent, Some(Intent::ClearQuery));
}

// ---- ⌥ and the hold (SPEC §5.1) ------------------------------------------------------------

#[test]
fn option_down_arms_the_hold_once() {
    let mut run = Run::mockup(Cursor::Tab(101));
    let first = run.key(NavKey::OptionDown);
    assert_eq!(first.intent, Some(Intent::BeginHold));
    assert!(first.handled);
    assert!(run.state.option_down);
    assert!(!run.state.held, "armed, not yet held");
    let repeat = run.key(NavKey::OptionDown);
    assert_eq!(
        repeat.intent, None,
        "a key repeat does not restart the timer"
    );
    assert!(repeat.handled);
    assert!(run.state.option_down);
}

#[test]
fn option_up_before_the_timer_cancels_the_hold() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.key(NavKey::OptionDown);
    let outcome = run.key(NavKey::OptionUp);
    assert_eq!(outcome.intent, Some(Intent::CancelHold));
    assert!(outcome.handled);
    assert!(!run.state.option_down);
    assert!(!run.state.held);
}

#[test]
fn the_timer_turns_the_key_look_on_only_while_option_is_down() {
    let mut run = Run::mockup(Cursor::Tab(101));
    let stray = run.key(NavKey::HoldElapsed);
    assert!(!stray.handled, "no ⌥, no hold");
    assert!(!run.state.held);

    run.key(NavKey::OptionDown);
    let outcome = run.key(NavKey::HoldElapsed);
    assert!(outcome.handled);
    assert_eq!(outcome.intent, None);
    assert!(run.state.held);
    assert_eq!(run.state.held_group, None, "no group chosen yet");
}

#[test]
fn option_up_when_held_without_a_group_cancels_and_resets() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.key(NavKey::OptionDown);
    run.key(NavKey::HoldElapsed);
    let outcome = run.key(NavKey::OptionUp);
    assert_eq!(outcome.intent, Some(Intent::CancelHold));
    assert!(outcome.handled);
    assert!(!run.state.option_down);
    assert!(!run.state.held);
    assert_eq!(run.state.held_group, None);
}

#[test]
fn a_stray_option_up_changes_nothing() {
    let mut run = Run::mockup(Cursor::Tab(101));
    let before = run.state;
    let outcome = run.key(NavKey::OptionUp);
    assert_eq!(outcome.intent, None);
    assert!(!outcome.handled);
    assert_eq!(run.state, before);
}

#[test]
fn a_group_letter_selects_the_group_in_either_case_and_turns_the_look_on() {
    for letter in ['s', 'S'] {
        let mut run = Run::mockup(Cursor::Tab(101));
        run.key(NavKey::OptionDown);
        let outcome = run.key(NavKey::Letter(letter));
        assert!(outcome.handled);
        assert_eq!(outcome.intent, None);
        assert_eq!(run.state.held_group, Some(2), "S is Lisbon trip, id 2");
        assert!(run.state.held, "a letter does not wait for the timer");
        assert!(run.state.option_down);
    }
}

#[test]
fn a_letter_picks_another_group_when_pressed_again() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.key(NavKey::OptionDown);
    run.key(NavKey::Letter('S'));
    run.key(NavKey::Letter('g'));
    assert_eq!(run.state.held_group, Some(5), "G is Work");
}

#[test]
fn a_letter_without_a_group_is_handled_and_changes_nothing() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.key(NavKey::OptionDown);
    let before = run.state;
    let outcome = run.key(NavKey::Letter('z'));
    assert!(outcome.handled);
    assert_eq!(outcome.intent, None);
    assert_eq!(run.state, before);

    run.key(NavKey::Letter('d'));
    let chosen = run.state;
    run.key(NavKey::Letter('z'));
    assert_eq!(
        run.state, chosen,
        "an unknown letter keeps the chosen group"
    );
}

#[test]
fn a_group_without_a_key_cannot_be_chosen_by_letter() {
    let order = FocusOrder::new(
        &[(1, 1), (2, LOOSE_GROUP_ID)],
        &[(1, Some('A')), (LOOSE_GROUP_ID, None)],
        &[],
    );
    let mut run = Run::new(order, Cursor::Tab(1));
    run.key(NavKey::OptionDown);
    let before = run.state;
    assert!(run.key(NavKey::Letter('l')).handled);
    assert_eq!(run.state, before);
}

#[test]
fn a_letter_without_option_is_not_handled() {
    let mut run = Run::mockup(Cursor::Tab(101));
    let before = run.state;
    let outcome = run.key(NavKey::Letter('s'));
    assert!(!outcome.handled);
    assert_eq!(run.state, before);
}

#[test]
fn a_digit_activates_the_nth_tab_of_the_chosen_group() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.key(NavKey::OptionDown);
    run.key(NavKey::Letter('s'));
    let outcome = run.key(NavKey::Digit(2));
    assert!(outcome.handled);
    assert_eq!(
        outcome.intent,
        Some(Intent::ActivateDigit { group: 2, n: 2 })
    );
    let last = run.key(NavKey::Digit(4));
    assert_eq!(
        last.intent,
        Some(Intent::ActivateDigit { group: 2, n: 4 }),
        "Lisbon trip has exactly 4 tabs"
    );
}

#[test]
fn a_digit_beyond_the_groups_tabs_is_handled_and_does_nothing() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.key(NavKey::OptionDown);
    run.key(NavKey::Letter('s'));
    let outcome = run.key(NavKey::Digit(5));
    assert!(outcome.handled);
    assert_eq!(outcome.intent, None);
    assert_eq!(run.state.held_group, Some(2), "the group stays chosen");

    run.key(NavKey::Letter('g'));
    assert_eq!(run.key(NavKey::Digit(3)).intent, None, "Work has 2 tabs");
    assert_eq!(
        run.key(NavKey::Digit(2)).intent,
        Some(Intent::ActivateDigit { group: 5, n: 2 })
    );
}

#[test]
fn a_digit_without_a_chosen_group_is_not_handled() {
    let mut run = Run::mockup(Cursor::Tab(101));
    let outcome = run.key(NavKey::Digit(1));
    assert!(!outcome.handled);
    assert_eq!(outcome.intent, None);
    run.key(NavKey::OptionDown);
    run.key(NavKey::HoldElapsed);
    assert!(
        !run.key(NavKey::Digit(1)).handled,
        "held, but no letter yet"
    );
}

#[test]
fn only_digits_one_to_nine_count() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.key(NavKey::OptionDown);
    run.key(NavKey::Letter('d'));
    for n in [0, 10, 200] {
        let outcome = run.key(NavKey::Digit(n));
        assert!(!outcome.handled, "digit {n}");
        assert_eq!(outcome.intent, None, "digit {n}");
    }
    assert_eq!(
        run.key(NavKey::Digit(6)).intent,
        Some(Intent::ActivateDigit { group: 3, n: 6 })
    );
}

#[test]
fn option_up_with_a_chosen_group_commits_it_and_resets_the_hold() {
    let mut run = Run::mockup(Cursor::Tab(101));
    run.key(NavKey::OptionDown);
    run.key(NavKey::Letter('s'));
    let outcome = run.key(NavKey::OptionUp);
    assert_eq!(outcome.intent, Some(Intent::CommitGroup(2)));
    assert!(outcome.handled);
    assert!(!run.state.option_down);
    assert!(!run.state.held);
    assert_eq!(run.state.held_group, None);
    assert!(
        !run.key(NavKey::Digit(1)).handled,
        "after the commit a digit has no group"
    );
}

#[test]
fn the_whole_letter_digit_release_sequence_of_the_ring_keys_frame() {
    let mut run = Run::mockup(Cursor::Tab(101));
    let mut intents = Vec::new();
    for key in [
        NavKey::OptionDown,
        NavKey::HoldElapsed,
        NavKey::Letter('S'),
        NavKey::Digit(2),
        NavKey::OptionUp,
    ] {
        intents.push(run.key(key).intent);
    }
    assert_eq!(
        intents,
        vec![
            Some(Intent::BeginHold),
            None,
            None,
            Some(Intent::ActivateDigit { group: 2, n: 2 }),
            Some(Intent::CommitGroup(2)),
        ]
    );
}

// ---- keys that mean something else while ⌥ is down -----------------------------------------

#[test]
fn plain_keys_are_not_handled_while_option_is_down() {
    for key in [
        NavKey::Left,
        NavKey::Right,
        NavKey::Up,
        NavKey::Down,
        NavKey::Enter,
        NavKey::CloseTab,
        NavKey::Mute,
        NavKey::Type('x'),
    ] {
        let mut run = Run::mockup(Cursor::Tab(103));
        run.key(NavKey::OptionDown);
        let before = run.state;
        let outcome = run.key(key);
        assert!(!outcome.handled, "{key:?}");
        assert_eq!(outcome.intent, None, "{key:?}");
        assert_eq!(run.state, before, "{key:?}");
    }
}

#[test]
fn the_move_keys_work_while_option_is_down() {
    let mut run = Run::mockup(Cursor::Tab(103));
    run.key(NavKey::OptionDown);
    assert!(matches!(
        run.key(NavKey::MoveLeft).intent,
        Some(Intent::Move { tab: 103, .. })
    ));
    assert!(matches!(
        run.key(NavKey::MoveToNextGroup).intent,
        Some(Intent::Move { tab: 103, .. })
    ));
}

#[test]
fn the_state_the_machine_does_not_own_is_left_alone() {
    // `step` does not mutate its input state; the caller takes `outcome.state`.
    let order = fixtures::focus_order(&fixtures::mockup_input(false));
    let state = NavState {
        cursor: Cursor::Tab(101),
        ..NavState::default()
    };
    let outcome = step(&order, &state, NavKey::Right);
    assert_eq!(state.cursor, Cursor::Tab(101));
    assert_eq!(outcome.state.cursor, Cursor::Tab(102));
}
