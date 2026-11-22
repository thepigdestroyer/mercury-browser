// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The `OpTracker` of lane med-1b (decision D3) on its own: the invariant that after `retire()` or a
//! newer `begin()` the old operation never finishes again (item 1, `core`). The tests of the tracker
//! together with a real backend are in `serial_backend.rs`.

mod model_support;

use mercury_mse::OpTracker;
use mercury_mse::api::OpId;
use model_support::Rng;

#[test]
fn serial_a_fresh_tracker_has_nothing_pending_and_accepts_nothing() {
    let mut t = OpTracker::new();
    assert_eq!(t.pending(), None);
    assert!(!t.finish(OpId(0)));
    assert!(!t.finish(OpId(1)));
}

#[test]
fn serial_finish_of_the_pending_operation_is_true_exactly_once() {
    let mut t = OpTracker::new();
    t.begin(OpId(7));
    assert_eq!(t.pending(), Some(OpId(7)));
    assert!(t.finish(OpId(7)));
    assert_eq!(t.pending(), None);
    assert!(!t.finish(OpId(7)), "a duplicate event is stale");
}

#[test]
fn serial_an_event_for_another_operation_leaves_the_pending_one_alone() {
    let mut t = OpTracker::new();
    t.begin(OpId(7));
    assert!(!t.finish(OpId(6)));
    assert!(!t.finish(OpId(8)));
    assert_eq!(t.pending(), Some(OpId(7)));
    assert!(t.finish(OpId(7)));
}

#[test]
fn serial_retire_makes_the_event_in_flight_stale() {
    let mut t = OpTracker::new();
    t.begin(OpId(1));
    t.retire();
    assert_eq!(t.pending(), None);
    assert!(!t.finish(OpId(1)));
}

#[test]
fn serial_the_stale_event_after_abort_and_a_new_append_is_dropped() {
    // The blocker of round 1: abort() and a new appendBuffer() in one task, then the old event arrives.
    let mut t = OpTracker::new();
    t.begin(OpId(1));
    t.retire();
    t.begin(OpId(2));
    assert!(
        !t.finish(OpId(1)),
        "the stale AppendDone must not end the new append"
    );
    assert_eq!(
        t.pending(),
        Some(OpId(2)),
        "the new append is still pending"
    );
    assert!(t.finish(OpId(2)));
}

#[test]
fn serial_a_newer_begin_replaces_the_pending_one() {
    let mut t = OpTracker::new();
    t.begin(OpId(1));
    t.begin(OpId(2));
    assert!(!t.finish(OpId(1)));
    assert!(t.finish(OpId(2)));
}

#[test]
fn serial_retire_with_nothing_pending_is_harmless() {
    let mut t = OpTracker::new();
    t.retire();
    t.retire();
    assert_eq!(t.pending(), None);
    t.begin(OpId(3));
    assert!(t.finish(OpId(3)));
}

#[test]
fn serial_property_the_tracker_matches_a_model() {
    // A model with the rule written the long way: the set of operations that may still complete is
    // at most the latest begun one and only until it completes or something retires it.
    for seed in 1..=500u64 {
        let mut rng = Rng::new(seed);
        let mut t = OpTracker::new();
        let mut next = 0u64;
        let mut begun: Vec<OpId> = Vec::new();
        let mut live: Option<OpId> = None;
        for step in 0..200 {
            let ctx = format!("seed {seed} step {step}");
            match rng.below(4) {
                0 => {
                    let op = OpId(next);
                    next += 1;
                    begun.push(op);
                    t.begin(op);
                    live = Some(op);
                }
                1 => {
                    t.retire();
                    live = None;
                }
                _ => {
                    if begun.is_empty() {
                        continue;
                    }
                    let op = begun[rng.below(begun.len())];
                    let got = t.finish(op);
                    let want = live == Some(op);
                    assert_eq!(got, want, "{ctx}: finish({op})");
                    if want {
                        live = None;
                    }
                }
            }
            assert_eq!(t.pending(), live, "{ctx}");
        }
    }
}

// ----------------------------------------------------------------------------------------------
// The kind of the pending operation (med-1b review fix, finding 1)

#[test]
fn serial_the_tracker_knows_whether_the_pending_operation_is_a_removal() {
    let mut t = OpTracker::new();
    assert!(!t.pending_is_remove());
    t.begin_remove(OpId(1));
    assert!(t.pending_is_remove());
    assert_eq!(t.pending(), Some(OpId(1)));
    // A removal is retired like any operation, and its kind goes with it.
    t.retire();
    assert!(!t.pending_is_remove());
    t.begin_remove(OpId(2));
    assert!(t.finish(OpId(2)));
    assert!(!t.pending_is_remove());
    // A newer append replaces a removal, kind included; and the other way round.
    t.begin_remove(OpId(3));
    t.begin(OpId(4));
    assert!(!t.pending_is_remove());
    t.begin_remove(OpId(5));
    assert!(t.pending_is_remove());
    // A stale event does not touch the kind.
    assert!(!t.finish(OpId(4)));
    assert!(t.pending_is_remove());
}
