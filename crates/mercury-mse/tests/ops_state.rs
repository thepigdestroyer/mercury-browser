// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `SourceBufferOps`, the operation state machine of the DOM's `SourceBuffer` (med-1b review fix,
//! findings 1 and 5a), on its own: which events each transition returns, that `abort()` is refused while
//! a removal is pending (MSE `abort()` step 3) and that a stale completion event changes nothing. The
//! tests with a real backend are in `ops_backend.rs` and `serial_backend.rs`.

mod model_support;

use mercury_mse::api::OpId;
use mercury_mse::source_buffer_ops::{AbortRefused, SbEvent, SourceBufferOps};
use model_support::Rng;

const UPDATE_END: [SbEvent; 2] = [SbEvent::Update, SbEvent::UpdateEnd];
const ABORTED: [SbEvent; 2] = [SbEvent::Abort, SbEvent::UpdateEnd];

#[test]
fn ops_a_fresh_buffer_is_not_updating_and_abort_does_nothing() {
    let mut ops = SourceBufferOps::new();
    assert!(!ops.updating());
    assert_eq!(ops.pending(), None);
    assert!(!ops.pending_is_remove());
    assert_eq!(ops.check_abort(), Ok(()));
    assert_eq!(ops.abort(), Ok(Vec::new()));
    assert!(!ops.updating());
    assert!(ops.finished(OpId(0)).is_empty());
    assert!(ops.failed(OpId(0)).is_empty());
    assert!(ops.abort_pending().is_empty());
}

#[test]
fn ops_begin_returns_updatestart_and_sets_updating() {
    let mut ops = SourceBufferOps::new();
    assert_eq!(ops.begin_append(OpId(1)), SbEvent::UpdateStart);
    assert!(ops.updating());
    assert_eq!(ops.pending(), Some(OpId(1)));
    assert!(!ops.pending_is_remove());
    assert_eq!(ops.finished(OpId(1)), UPDATE_END);
    assert_eq!(ops.begin_remove(OpId(2)), SbEvent::UpdateStart);
    assert!(ops.updating());
    assert_eq!(ops.pending(), Some(OpId(2)));
    assert!(ops.pending_is_remove());
}

#[test]
fn ops_event_names_are_the_ones_the_page_sees() {
    let names: Vec<&str> = [
        SbEvent::UpdateStart,
        SbEvent::Update,
        SbEvent::UpdateEnd,
        SbEvent::Error,
        SbEvent::Abort,
    ]
    .into_iter()
    .map(SbEvent::name)
    .collect();
    assert_eq!(
        names,
        ["updatestart", "update", "updateend", "error", "abort"]
    );
}

// (a) abort() after remove() is refused, and the removal is not retired by the refusal.
#[test]
fn ops_abort_after_a_remove_is_refused_and_the_removal_still_completes_once() {
    let mut ops = SourceBufferOps::new();
    let _ = ops.begin_remove(OpId(5));

    let refused = ops
        .abort()
        .expect_err("abort() during a removal must throw");
    assert!(
        refused.reason.contains("range removal"),
        "the reason names the algorithm: {refused}"
    );
    assert_eq!(ops.check_abort(), Err(refused));
    assert!(ops.updating(), "a refused abort() changes nothing");
    assert_eq!(ops.pending(), Some(OpId(5)));
    assert!(ops.pending_is_remove());

    // Again and again: still refused, still pending.
    assert!(ops.abort().is_err());
    assert!(ops.updating());

    // The backend's RemoveDone arrives later: exactly one update and one updateend.
    assert_eq!(ops.finished(OpId(5)), UPDATE_END);
    assert!(!ops.updating());
    assert!(!ops.pending_is_remove());
    assert!(ops.finished(OpId(5)).is_empty(), "never twice");

    // With the removal completed, abort() is allowed and has nothing to abort.
    assert_eq!(ops.abort(), Ok(Vec::new()));
}

#[test]
fn ops_a_refused_abort_leaves_removesourcebuffer_working() {
    let mut ops = SourceBufferOps::new();
    let _ = ops.begin_remove(OpId(9));
    assert!(ops.abort().is_err());
    // removeSourceBuffer() step 2 aborts whatever is updating, a removal included.
    assert_eq!(ops.abort_pending(), ABORTED);
    assert!(!ops.updating());
    assert!(
        ops.finished(OpId(9)).is_empty(),
        "the RemoveDone in flight is stale"
    );
}

// (b) abort() after an append retires it.
#[test]
fn ops_abort_after_an_append_retires_it_and_the_stale_event_is_dropped() {
    let mut ops = SourceBufferOps::new();
    let _ = ops.begin_append(OpId(1));

    assert_eq!(ops.abort(), Ok(ABORTED.to_vec()));
    assert!(!ops.updating());
    assert_eq!(ops.pending(), None);

    // The page appends again in the same task; the old operation's event arrives afterwards.
    let _ = ops.begin_append(OpId(2));
    assert!(
        ops.finished(OpId(1)).is_empty(),
        "stale: must not end the new append"
    );
    assert!(
        ops.failed(OpId(1)).is_empty(),
        "a stale error is dropped too"
    );
    assert!(ops.updating(), "the new append is still pending");
    assert_eq!(ops.pending(), Some(OpId(2)));

    assert_eq!(ops.finished(OpId(2)), UPDATE_END);
    assert!(!ops.updating());
}

#[test]
fn ops_a_stale_event_does_not_clear_updating_of_a_newer_operation() {
    let mut ops = SourceBufferOps::new();
    let _ = ops.begin_append(OpId(1));
    assert_eq!(ops.finished(OpId(1)), UPDATE_END);
    let _ = ops.begin_remove(OpId(3));
    // An event for an operation that is not the pending one: nothing changes.
    assert!(ops.finished(OpId(1)).is_empty());
    assert!(ops.failed(OpId(2)).is_empty());
    assert!(ops.finished(OpId(4)).is_empty());
    assert!(ops.updating());
    assert!(ops.pending_is_remove());
    assert_eq!(ops.finished(OpId(3)), UPDATE_END);
}

#[test]
fn ops_a_failed_append_ends_with_error_and_updateend_once() {
    let mut ops = SourceBufferOps::new();
    let _ = ops.begin_append(OpId(4));
    assert_eq!(ops.failed(OpId(4)), [SbEvent::Error, SbEvent::UpdateEnd]);
    assert!(!ops.updating());
    assert!(ops.failed(OpId(4)).is_empty());
    assert!(ops.finished(OpId(4)).is_empty());
}

#[test]
fn ops_removing_the_buffer_while_updating_aborts_and_retires() {
    let mut ops = SourceBufferOps::new();
    let _ = ops.begin_append(OpId(1));
    assert_eq!(ops.abort_pending(), ABORTED);
    ops.detach();
    assert!(!ops.updating());
    assert!(ops.finished(OpId(1)).is_empty());
    // Not updating: no events at all.
    assert!(ops.abort_pending().is_empty());
}

#[test]
fn ops_detach_is_silent_and_retires() {
    let mut ops = SourceBufferOps::new();
    let _ = ops.begin_remove(OpId(1));
    ops.detach();
    assert!(!ops.updating());
    assert!(!ops.pending_is_remove());
    assert_eq!(ops.pending(), None);
    assert!(ops.finished(OpId(1)).is_empty());
    // After a close the buffer is gone, but a fresh tracker state would behave: abort is fine.
    assert_eq!(ops.abort(), Ok(Vec::new()));
}

#[test]
fn ops_the_abort_refusal_is_an_error_value_with_a_message() {
    let mut ops = SourceBufferOps::new();
    let _ = ops.begin_remove(OpId(1));
    let refused: AbortRefused = ops.abort().unwrap_err();
    let boxed: Box<dyn std::error::Error> = Box::new(refused);
    assert!(!boxed.to_string().is_empty());
}

// (d) The sequence property: updating() == pending().is_some() after every call, and every answer
// agrees with a model written the long way.
#[test]
fn ops_property_updating_is_pending_and_every_answer_matches_a_model() {
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Kind {
        Append,
        Remove,
    }
    for seed in 1..=1_000u64 {
        let mut rng = Rng::new(seed);
        let mut ops = SourceBufferOps::new();
        let mut next = 0u64;
        let mut issued: Vec<OpId> = Vec::new();
        // The model: the one pending operation and its kind.
        let mut live: Option<(OpId, Kind)> = None;
        for step in 0..200 {
            let ctx = format!("seed {seed} step {step}");
            match rng.below(8) {
                0 | 1 => {
                    let op = OpId(next);
                    next += 1;
                    issued.push(op);
                    assert_eq!(ops.begin_append(op), SbEvent::UpdateStart, "{ctx}");
                    live = Some((op, Kind::Append));
                }
                2 => {
                    let op = OpId(next);
                    next += 1;
                    issued.push(op);
                    assert_eq!(ops.begin_remove(op), SbEvent::UpdateStart, "{ctx}");
                    live = Some((op, Kind::Remove));
                }
                3 | 4 => {
                    let got = ops.abort();
                    match live {
                        Some((_, Kind::Remove)) => {
                            assert!(got.is_err(), "{ctx}: abort during a removal succeeded");
                            assert_eq!(ops.check_abort(), got.map(|_| ()), "{ctx}");
                        }
                        Some((_, Kind::Append)) => {
                            assert_eq!(got, Ok(ABORTED.to_vec()), "{ctx}");
                            live = None;
                        }
                        None => assert_eq!(got, Ok(Vec::new()), "{ctx}"),
                    }
                }
                5 => {
                    let want = if live.is_some() {
                        ABORTED.to_vec()
                    } else {
                        Vec::new()
                    };
                    assert_eq!(ops.abort_pending(), want, "{ctx}");
                    live = None;
                }
                6 => {
                    if rng.chance(30) {
                        ops.detach();
                        live = None;
                    } else if !issued.is_empty() {
                        let op = issued[rng.below(issued.len())];
                        let got = ops.failed(op);
                        if live.map(|(o, _)| o) == Some(op) {
                            assert_eq!(got, [SbEvent::Error, SbEvent::UpdateEnd], "{ctx}");
                            live = None;
                        } else {
                            assert!(got.is_empty(), "{ctx}: stale failed({op}) gave {got:?}");
                        }
                    }
                }
                _ => {
                    if issued.is_empty() {
                        continue;
                    }
                    let op = issued[rng.below(issued.len())];
                    let got = ops.finished(op);
                    if live.map(|(o, _)| o) == Some(op) {
                        assert_eq!(got, UPDATE_END, "{ctx}");
                        live = None;
                    } else {
                        assert!(got.is_empty(), "{ctx}: stale finished({op}) gave {got:?}");
                    }
                }
            }
            assert_eq!(ops.updating(), ops.pending().is_some(), "{ctx}");
            assert_eq!(ops.pending(), live.map(|(o, _)| o), "{ctx}");
            assert_eq!(
                ops.pending_is_remove(),
                matches!(live, Some((_, Kind::Remove))),
                "{ctx}"
            );
            assert!(
                !ops.pending_is_remove() || ops.updating(),
                "{ctx}: a pending removal means updating"
            );
        }
    }
}
