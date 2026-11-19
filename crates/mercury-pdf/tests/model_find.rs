// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The viewer model's find state machine: late and reordered answers, wrapping, the cap.

use mercury_pdf::cache::budget;
use mercury_pdf::find::{FindProgress, FindState, HitsOutcome};
use mercury_pdf::types::{FindOptions, Rect, SearchHit};

fn hit(page: u32, char_start: u32) -> SearchHit {
    SearchHit {
        page,
        char_start,
        char_len: 3,
        rects: vec![Rect::new(
            10.0,
            10.0 + char_start as f32,
            40.0,
            20.0 + char_start as f32,
        )],
    }
}

fn hits(page: u32, starts: &[u32]) -> Vec<SearchHit> {
    starts.iter().map(|&start| hit(page, start)).collect()
}

fn current(state: &FindState) -> Option<(u32, u32)> {
    state.current().map(|hit| (hit.page, hit.char_start))
}

fn progress(current: usize, total: usize, finished: bool) -> FindProgress {
    FindProgress {
        current,
        total,
        finished,
    }
}

#[test]
fn find_a_new_search_has_no_hits_and_is_running() {
    let mut state = FindState::new();
    assert!(!state.is_active());
    assert_eq!(state.progress(), progress(0, 0, false));
    let id = state.start("needle", FindOptions::default());
    assert_ne!(id, 0);
    assert!(state.is_active());
    assert_eq!(state.query(), "needle");
    assert_eq!(state.progress(), progress(0, 0, false));
    assert!(state.progress().searching());
    assert_eq!(current(&state), None);
}

#[test]
fn find_an_empty_query_is_finished_at_once() {
    let mut state = FindState::new();
    state.start("", FindOptions::default());
    assert_eq!(state.progress(), progress(0, 0, true));
    assert!(!state.progress().searching());
}

#[test]
fn find_the_first_answer_with_hits_makes_the_current_hit() {
    let mut state = FindState::new();
    let id = state.start("a", FindOptions::default());
    // The page on screen has none; the next page has two.
    assert_eq!(
        state.hits(id, 4, Vec::new(), false),
        HitsOutcome::Accepted {
            current_changed: false
        }
    );
    assert_eq!(state.progress(), progress(0, 0, false));
    assert_eq!(
        state.hits(id, 5, hits(5, &[30, 10]), false),
        HitsOutcome::Accepted {
            current_changed: true
        }
    );
    // Hits are kept in order whatever order they came in.
    assert_eq!(current(&state), Some((5, 10)));
    assert_eq!(state.progress(), progress(1, 2, false));
    // A later answer does not move the current hit.
    assert_eq!(
        state.hits(id, 6, hits(6, &[1]), false),
        HitsOutcome::Accepted {
            current_changed: false
        }
    );
    assert_eq!(current(&state), Some((5, 10)));
    assert_eq!(state.progress(), progress(1, 3, false));
}

#[test]
fn find_answers_in_any_page_order_are_counted_in_document_order() {
    // The helper starts at the page on screen (7) and wraps to the start.
    let mut state = FindState::new();
    let id = state.start("a", FindOptions::default());
    state.hits(id, 7, hits(7, &[5]), false);
    assert_eq!(state.progress(), progress(1, 1, false));
    state.hits(id, 9, hits(9, &[5, 6]), false);
    assert_eq!(state.progress(), progress(1, 3, false));
    // The pages before the current one arrive last: the current hit stays, its number grows.
    state.hits(id, 0, hits(0, &[1, 2]), false);
    assert_eq!(current(&state), Some((7, 5)));
    assert_eq!(state.progress(), progress(3, 5, false));
    state.hits(id, 3, hits(3, &[8]), true);
    assert_eq!(state.progress(), progress(4, 6, true));
    assert!(!state.progress().searching());
    // Walking forward visits the hits in document order and wraps.
    let mut order = vec![current(&state).expect("a hit")];
    for _ in 0..6 {
        state.select_next().expect("a hit");
        order.push(current(&state).expect("a hit"));
    }
    assert_eq!(
        order,
        vec![(7, 5), (9, 5), (9, 6), (0, 1), (0, 2), (3, 8), (7, 5)]
    );
}

#[test]
fn find_previous_walks_backwards_and_wraps_to_the_last_hit() {
    let mut state = FindState::new();
    let id = state.start("a", FindOptions::default());
    state.hits(id, 2, hits(2, &[4, 9]), false);
    state.hits(id, 8, hits(8, &[1]), true);
    assert_eq!(current(&state), Some((2, 4)));
    assert_eq!(
        state
            .select_previous()
            .map(|hit| (hit.page, hit.char_start)),
        Some((8, 1))
    );
    assert_eq!(state.progress(), progress(3, 3, true));
    state.select_previous();
    assert_eq!(current(&state), Some((2, 9)));
    state.select_previous();
    assert_eq!(current(&state), Some((2, 4)));
    state.select_next();
    state.select_next();
    state.select_next();
    assert_eq!(current(&state), Some((2, 4)), "three hits, three steps");
}

#[test]
fn find_one_hit_wraps_onto_itself() {
    let mut state = FindState::new();
    let id = state.start("a", FindOptions::default());
    state.hits(id, 1, hits(1, &[3]), true);
    assert_eq!(state.select_next().map(|hit| hit.char_start), Some(3));
    assert_eq!(state.select_previous().map(|hit| hit.char_start), Some(3));
    assert_eq!(state.progress(), progress(1, 1, true));
}

#[test]
fn find_with_no_hits_has_nothing_to_move_to() {
    let mut state = FindState::new();
    let id = state.start("zzz", FindOptions::default());
    state.hits(id, 0, Vec::new(), true);
    assert!(state.select_next().is_none());
    assert!(state.select_previous().is_none());
    assert_eq!(state.progress(), progress(0, 0, true));
}

#[test]
fn find_late_answers_for_an_old_search_are_ignored() {
    let mut state = FindState::new();
    let first = state.start("a", FindOptions::default());
    state.hits(first, 0, hits(0, &[1]), false);
    // The person types on: a new query replaces everything.
    let second = state.start("ab", FindOptions::default());
    assert_ne!(first, second);
    assert_eq!(state.progress(), progress(0, 0, false));
    assert_eq!(
        state.hits(first, 1, hits(1, &[9]), true),
        HitsOutcome::Stale,
        "a late answer of the first search"
    );
    assert_eq!(
        state.progress(),
        progress(0, 0, false),
        "and not finished by it"
    );
    state.hits(second, 3, hits(3, &[2]), true);
    assert_eq!(state.progress(), progress(1, 1, true));
    assert_eq!(current(&state), Some((3, 2)));
}

#[test]
fn find_changing_an_option_is_a_new_search() {
    let mut state = FindState::new();
    let loose = state.start("Word", FindOptions::default());
    state.hits(loose, 0, hits(0, &[1, 2, 3]), true);
    let strict = state.start(
        "Word",
        FindOptions {
            match_case: true,
            whole_word: false,
        },
    );
    assert!(state.options().match_case);
    assert_eq!(state.progress(), progress(0, 0, false));
    assert_eq!(
        state.hits(loose, 1, hits(1, &[4]), true),
        HitsOutcome::Stale
    );
    state.hits(strict, 0, hits(0, &[2]), true);
    assert_eq!(state.progress(), progress(1, 1, true));
}

#[test]
fn find_the_client_can_name_the_search() {
    let mut state = FindState::new();
    let own = state.start("a", FindOptions::default());
    state.adopt_id(77);
    assert_eq!(state.id(), 77);
    assert_eq!(state.hits(own, 0, hits(0, &[1]), false), HitsOutcome::Stale);
    assert_ne!(state.hits(77, 0, hits(0, &[1]), false), HitsOutcome::Stale);
    // Adopting with nothing started does nothing.
    let mut idle = FindState::new();
    idle.adopt_id(5);
    assert!(!idle.is_active());
}

#[test]
fn find_clearing_drops_the_hits_and_ignores_what_is_still_coming() {
    let mut state = FindState::new();
    let id = state.start("a", FindOptions::default());
    state.hits(id, 0, hits(0, &[1]), false);
    state.clear();
    assert!(!state.is_active());
    assert_eq!(state.progress(), progress(0, 0, false));
    assert_eq!(state.hits(id, 1, hits(1, &[1]), true), HitsOutcome::Stale);
    assert!(state.select_next().is_none());
    assert!(state.hits_on_page(0).is_empty());
    // A new search after a clear gets a fresh id.
    assert_ne!(state.start("b", FindOptions::default()), id);
}

#[test]
fn find_an_answer_for_a_page_again_replaces_its_hits() {
    let mut state = FindState::new();
    let id = state.start("a", FindOptions::default());
    state.hits(id, 2, hits(2, &[1, 2, 3]), false);
    state.hits(id, 2, hits(2, &[7]), false);
    assert_eq!(state.progress().total, 1);
    // The current hit was replaced: the first of the new answer takes over.
    assert_eq!(current(&state), Some((2, 7)));
    state.hits(id, 2, Vec::new(), false);
    assert_eq!(state.progress().total, 0);
    assert_eq!(current(&state), None);
    assert_eq!(state.progress().current, 0);
}

#[test]
fn find_a_hit_that_names_another_page_is_dropped() {
    let mut state = FindState::new();
    let id = state.start("a", FindOptions::default());
    let mixed = vec![hit(3, 1), hit(4, 2), hit(3, 5)];
    state.hits(id, 3, mixed, true);
    assert_eq!(state.progress().total, 2);
    assert!(state.hits_on_page(4).is_empty());
    assert_eq!(state.hits_on_page(3).len(), 2);
}

#[test]
fn find_keeps_at_most_ten_thousand_hits() {
    assert_eq!(budget::FIND_HITS, 10_000);
    let mut state = FindState::new();
    let id = state.start("e", FindOptions::default());
    let many =
        |page: u32, count: u32| -> Vec<SearchHit> { (0..count).map(|i| hit(page, i)).collect() };
    state.hits(id, 0, many(0, 6_000), false);
    state.hits(id, 1, many(1, 6_000), false);
    assert_eq!(state.progress().total, 10_000, "the second page is cut");
    assert_eq!(state.hits_on_page(1).len(), 4_000);
    state.hits(id, 2, many(2, 5), true);
    assert_eq!(state.progress().total, 10_000);
    assert!(state.hits_on_page(2).is_empty());
    // Replacing a page frees room for later answers.
    state.hits(id, 0, many(0, 10), false);
    assert_eq!(state.progress().total, 4_010);
    state.hits(id, 3, many(3, 100), true);
    assert_eq!(state.progress().total, 4_110);
}

#[test]
fn find_the_current_hit_is_known_by_identity() {
    let mut state = FindState::new();
    let id = state.start("a", FindOptions::default());
    state.hits(id, 1, hits(1, &[10, 20]), false);
    state.select_next();
    let second = state.current().cloned().expect("a hit");
    assert!(state.is_current(&second));
    assert!(!state.is_current(&hit(1, 10)));
    assert!(!state.is_current(&hit(2, 20)));
}
