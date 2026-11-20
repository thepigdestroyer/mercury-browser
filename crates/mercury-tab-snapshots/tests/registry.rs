// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Tests of the registry: the store and the scheduler kept in step. All time is injected, and the
//! snapshots are made by hand. The one rule above the others: the picture of a tab that was
//! closed while it was being made never enters the store.

use std::time::Duration;

use mercury_tab_snapshots::downscale::Limits;
use mercury_tab_snapshots::frame::RgbaFrame;
use mercury_tab_snapshots::registry::{FinishOutcome, SnapshotRegistry, Stats};
use mercury_tab_snapshots::schedule::{
    CaptureReason, CaptureRequest, FAILURE_BACKOFF, IN_FLIGHT_TIMEOUT, MAX_CONSECUTIVE_FAILURES,
    PERIODIC_INTERVAL, Skip,
};
use mercury_tab_snapshots::store::{DEFAULT_CAP_BYTES, ENTRY_OVERHEAD_BYTES, Snapshot};

type Tab = u32;
type Registry = SnapshotRegistry<Tab>;

const A: Tab = 1;
const B: Tab = 2;
const C: Tab = 3;
const D: Tab = 4;

/// The size of the rendering surface in these tests.
const SIZE: (u32, u32) = (1000, 600);

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A snapshot of `width` by `height` pixels, all bytes `fill`.
fn snapshot(width: u32, height: u32, fill: u8) -> Snapshot {
    let pixels = vec![fill; width as usize * height as usize * 4];
    let frame = RgbaFrame::new(width, height, pixels).expect("a checked size");
    Snapshot::new(frame, secs(1), SIZE)
}

/// What a snapshot of that size costs against the cap.
fn cost(width: u32, height: u32) -> usize {
    width as usize * height as usize * 4 + ENTRY_OVERHEAD_BYTES
}

/// What the tests' 10 by 10 snapshots cost: 656 bytes.
fn small_cost() -> usize {
    cost(10, 10)
}

fn registry(cap_bytes: usize) -> Registry {
    SnapshotRegistry::new(cap_bytes, Limits::DEFAULT)
}

/// A presented frame that must not start a capture (a tab's first frame is never due).
#[track_caller]
fn quiet_frame(registry: &mut Registry, tab: Tab, at: Duration) {
    assert_eq!(
        registry.frame_presented(tab, SIZE, at),
        Err(Skip::NotDue),
        "frame of tab {tab} at {at:?}"
    );
}

/// The request inside an `Ok`, or a failure that names the refusal.
#[track_caller]
fn leave(registry: &mut Registry, tab: Tab, at: Duration) -> CaptureRequest<Tab> {
    match registry.tab_leaving(tab, SIZE, at) {
        Ok(request) => request,
        Err(skip) => panic!("tab {tab} leaving at {at:?} was refused: {skip:?}"),
    }
}

/// The tab shows a frame at `at`, leaves a second later and its picture comes back a second after
/// that. Returns what the registry did with it.
#[track_caller]
fn capture_tab(
    registry: &mut Registry,
    tab: Tab,
    at: Duration,
    snapshot: Snapshot,
) -> FinishOutcome {
    quiet_frame(registry, tab, at);
    let request = leave(registry, tab, at + secs(1));
    registry.finish(request, Some(snapshot), at + secs(2))
}

// ----- the default value -----

#[test]
fn a_default_registry_has_the_default_cap_and_limits() {
    let registry: Registry = SnapshotRegistry::default();

    assert_eq!(registry.limits(), Limits::DEFAULT);
    assert_eq!(
        registry.stats(),
        Stats {
            cap_bytes: DEFAULT_CAP_BYTES,
            ..Stats::default()
        }
    );
    assert_eq!(DEFAULT_CAP_BYTES, 128_000_000);
}

#[test]
fn the_limits_are_the_ones_the_registry_was_made_with() {
    let limits = Limits::new(960, 600).expect("both sides are above zero");

    let registry: Registry = SnapshotRegistry::new(1000, limits);

    assert_eq!(registry.limits(), limits);
    assert_eq!(registry.limits().max_width(), 960);
}

// ----- present, leave, finish, get -----

#[test]
fn a_tab_that_leaves_is_stored_and_found() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));

    let request = leave(&mut registry, A, secs(6));
    assert_eq!(request.key(), A);
    assert_eq!(request.reason(), CaptureReason::Leaving);
    assert_eq!(request.surface(), SIZE);

    let wanted = snapshot(10, 10, 7);
    assert_eq!(
        registry.finish(request, Some(wanted.clone()), secs(7)),
        FinishOutcome::Stored { evicted: 0 }
    );
    assert_eq!(registry.peek(A), Some(&wanted));
    assert_eq!(registry.get(A), Some(&wanted));
    assert_eq!(registry.get(B), None);
    assert_eq!(registry.peek(B), None);
}

#[test]
fn a_tab_that_stays_active_is_stored_by_its_periodic_capture() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));

    let request = registry
        .frame_presented(A, SIZE, secs(5) + PERIODIC_INTERVAL)
        .expect("the thirty seconds are over");
    assert_eq!(request.reason(), CaptureReason::Periodic);

    let wanted = snapshot(10, 10, 9);
    assert_eq!(
        registry.finish(request, Some(wanted.clone()), secs(36)),
        FinishOutcome::Stored { evicted: 0 }
    );
    assert_eq!(registry.peek(A), Some(&wanted));
}

#[test]
fn an_on_demand_capture_is_stored_too() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));

    let request = registry
        .capture_now(A, SIZE, secs(5))
        .expect("a frame was shown");
    assert_eq!(request.reason(), CaptureReason::OnDemand);

    assert_eq!(
        registry.finish(request, Some(snapshot(10, 10, 1)), secs(5)),
        FinishOutcome::Stored { evicted: 0 }
    );
    assert!(registry.peek(A).is_some());
}

#[test]
fn a_newer_snapshot_replaces_the_tabs_older_one() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    capture_tab(&mut registry, A, secs(5), snapshot(10, 10, 1));
    quiet_frame(&mut registry, A, secs(20));
    let request = leave(&mut registry, A, secs(21));

    let newer = snapshot(10, 10, 2);
    assert_eq!(
        registry.finish(request, Some(newer.clone()), secs(22)),
        FinishOutcome::Stored { evicted: 0 }
    );

    assert_eq!(registry.peek(A), Some(&newer));
    assert_eq!(registry.stats().entries, 1);
    assert_eq!(registry.stats().used_bytes, small_cost());
    assert_eq!(registry.stats().stored, 2);
}

#[test]
fn what_the_scheduler_refuses_it_refuses_through_the_registry() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    assert_eq!(
        registry.tab_leaving(A, SIZE, secs(1)),
        Err(Skip::NoFrame),
        "no frame was shown yet"
    );

    quiet_frame(&mut registry, A, secs(5));
    assert_eq!(registry.tab_leaving(B, SIZE, secs(6)), Err(Skip::WrongTab));
    assert_eq!(
        registry.tab_leaving(A, (1000, 700), secs(6)),
        Err(Skip::SizeChanged)
    );
    assert_eq!(registry.capture_now(B, SIZE, secs(6)), Err(Skip::WrongTab));
}

// ----- a tab that closes while its picture is being made -----

#[test]
fn a_result_for_a_tab_forgotten_meanwhile_is_discarded_and_never_stored() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));
    let request = leave(&mut registry, A, secs(6));

    // The tab closes while the worker shrinks its frame.
    registry.forget(A);

    assert_eq!(
        registry.finish(request, Some(snapshot(10, 10, 5)), secs(7)),
        FinishOutcome::Discarded
    );
    assert_eq!(
        registry.peek(A),
        None,
        "the closed tab's picture is not kept"
    );
    assert_eq!(registry.get(A), None);
    let stats = registry.stats();
    assert_eq!((stats.entries, stats.used_bytes), (0, 0));
    assert_eq!((stats.stored, stats.discarded), (0, 1));
}

#[test]
fn forgetting_a_tab_removes_its_older_snapshot_and_frees_the_capture_slot() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    capture_tab(&mut registry, A, secs(5), snapshot(10, 10, 1));
    quiet_frame(&mut registry, A, secs(20));
    let in_flight = leave(&mut registry, A, secs(21));
    assert!(registry.peek(A).is_some(), "the older picture is there");

    registry.forget(A);

    assert_eq!(registry.peek(A), None);
    // The slot of the capture in flight is free: another tab may be captured at once.
    quiet_frame(&mut registry, B, secs(22));
    let request = leave(&mut registry, B, secs(23));
    assert_eq!(
        registry.finish(in_flight, Some(snapshot(10, 10, 3)), secs(23)),
        FinishOutcome::Discarded
    );
    assert_eq!(
        registry.finish(request, Some(snapshot(10, 10, 4)), secs(24)),
        FinishOutcome::Stored { evicted: 0 }
    );
    assert_eq!(registry.peek(A), None);
    assert!(registry.peek(B).is_some());
}

#[test]
fn a_late_result_does_not_land_on_a_tab_shown_again_under_the_same_key() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));
    let old_request = leave(&mut registry, A, secs(6));
    registry.forget(A);
    quiet_frame(&mut registry, A, secs(8));

    assert_eq!(
        registry.finish(old_request, Some(snapshot(10, 10, 2)), secs(9)),
        FinishOutcome::Discarded
    );
    assert_eq!(registry.peek(A), None);
}

#[test]
fn a_result_that_arrives_after_the_request_timed_out_is_discarded() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));
    let request = leave(&mut registry, A, secs(6));

    // Longer than the timeout later, the next decision gives the request up (a failure).
    let later = secs(6) + IN_FLIGHT_TIMEOUT + secs(1);
    assert_eq!(registry.capture_now(A, SIZE, later), Err(Skip::Backoff));

    assert_eq!(
        registry.finish(request, Some(snapshot(10, 10, 2)), later),
        FinishOutcome::Discarded
    );
    assert_eq!(registry.peek(A), None);
}

#[test]
fn a_result_from_before_the_rendering_context_was_made_again_is_discarded() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));
    let request = leave(&mut registry, A, secs(6));

    registry.rendering_changed();

    assert_eq!(
        registry.finish(request, Some(snapshot(10, 10, 2)), secs(7)),
        FinishOutcome::Discarded
    );
    assert_eq!(registry.peek(A), None);
}

// ----- failures -----

#[test]
fn a_failed_capture_stores_nothing_and_the_next_one_waits_out_the_backoff() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));
    let request = leave(&mut registry, A, secs(6));

    assert_eq!(
        registry.finish(request, None, secs(6)),
        FinishOutcome::Failed
    );

    assert_eq!(registry.peek(A), None);
    assert_eq!(registry.stats().failed, 1);
    assert_eq!(
        registry.capture_now(
            A,
            SIZE,
            secs(6) + FAILURE_BACKOFF - Duration::from_millis(1)
        ),
        Err(Skip::Backoff)
    );
    let retry = registry
        .capture_now(A, SIZE, secs(6) + FAILURE_BACKOFF)
        .expect("the backoff is over");
    assert_eq!(
        registry.finish(retry, Some(snapshot(10, 10, 6)), secs(12)),
        FinishOutcome::Stored { evicted: 0 }
    );
}

#[test]
fn a_failure_for_a_tab_forgotten_meanwhile_is_discarded_not_counted() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));
    let request = leave(&mut registry, A, secs(6));
    registry.forget(A);

    assert_eq!(
        registry.finish(request, None, secs(7)),
        FinishOutcome::Discarded
    );

    let stats = registry.stats();
    assert_eq!((stats.failed, stats.discarded), (0, 1));
}

#[test]
fn captures_stop_after_three_failures_in_a_row_until_the_context_is_made_again() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    quiet_frame(&mut registry, A, secs(5));
    for attempt in 0..u64::from(MAX_CONSECUTIVE_FAILURES) {
        let at = secs(10 + attempt * 10);
        let request = registry
            .capture_now(A, SIZE, at)
            .expect("the backoff is over");
        assert_eq!(registry.finish(request, None, at), FinishOutcome::Failed);
    }

    assert_eq!(
        registry.capture_now(A, SIZE, secs(100)),
        Err(Skip::Disabled)
    );

    registry.rendering_changed();
    // Captures are on again, but the first frame of the new context starts the tab's periodic
    // clock afresh (it may be a partial first paint), so the capture comes 30 s later.
    assert_eq!(
        registry.frame_presented(A, SIZE, secs(101)),
        Err(Skip::NotDue),
        "captures are on again, not due yet"
    );
    let request = registry
        .frame_presented(A, SIZE, secs(131))
        .expect("captures are on again");
    assert_eq!(request.reason(), CaptureReason::Periodic);
}

// ----- the memory cap -----

#[test]
fn a_full_store_evicts_the_oldest_picture_and_counts_it() {
    let mut registry = registry(2 * small_cost());
    assert_eq!(
        capture_tab(&mut registry, A, secs(5), snapshot(10, 10, 1)),
        FinishOutcome::Stored { evicted: 0 }
    );
    assert_eq!(
        capture_tab(&mut registry, B, secs(10), snapshot(10, 10, 2)),
        FinishOutcome::Stored { evicted: 0 }
    );

    assert_eq!(
        capture_tab(&mut registry, C, secs(15), snapshot(10, 10, 3)),
        FinishOutcome::Stored { evicted: 1 }
    );

    assert_eq!(registry.peek(A), None, "the least recently used went");
    assert!(registry.peek(B).is_some());
    assert!(registry.peek(C).is_some());
    let stats = registry.stats();
    assert_eq!((stats.entries, stats.used_bytes), (2, 2 * small_cost()));
    assert_eq!((stats.stored, stats.evicted), (3, 1));
}

#[test]
fn one_large_picture_can_evict_several_small_ones() {
    let mut registry = registry(2000);
    for (tab, at) in [(A, 5), (B, 10), (C, 15)] {
        capture_tab(&mut registry, tab, secs(at), snapshot(10, 10, 1));
    }
    assert_eq!(registry.stats().used_bytes, 3 * small_cost());

    // 20 by 10 pixels cost 1,056: two of the small ones must go.
    assert_eq!(
        capture_tab(&mut registry, D, secs(20), snapshot(20, 10, 4)),
        FinishOutcome::Stored { evicted: 2 }
    );

    assert_eq!(registry.stats().evicted, 2);
    assert_eq!(registry.peek(A), None);
    assert_eq!(registry.peek(B), None);
    assert!(registry.peek(C).is_some());
    assert!(registry.peek(D).is_some());
}

#[test]
fn peek_does_not_refresh_a_picture_and_get_does() {
    for refresh in [false, true] {
        let mut registry = registry(2 * small_cost());
        capture_tab(&mut registry, A, secs(5), snapshot(10, 10, 1));
        capture_tab(&mut registry, B, secs(10), snapshot(10, 10, 2));

        if refresh {
            assert!(registry.get(A).is_some());
        } else {
            assert!(registry.peek(A).is_some());
        }
        capture_tab(&mut registry, C, secs(15), snapshot(10, 10, 3));

        if refresh {
            assert!(registry.peek(A).is_some(), "get made A the newest");
            assert_eq!(registry.peek(B), None, "so B went");
        } else {
            assert_eq!(registry.peek(A), None, "peek left A the oldest");
            assert!(registry.peek(B).is_some());
        }
    }
}

#[test]
fn a_lower_cap_evicts_at_once_oldest_first() {
    let mut registry = registry(2000);
    for (tab, at) in [(A, 5), (B, 10), (C, 15)] {
        capture_tab(&mut registry, tab, secs(at), snapshot(10, 10, 1));
    }

    assert_eq!(registry.set_cap_bytes(700), 2);

    assert_eq!(registry.peek(A), None);
    assert_eq!(registry.peek(B), None);
    assert!(registry.peek(C).is_some());
    let stats = registry.stats();
    assert_eq!(
        (stats.entries, stats.used_bytes, stats.cap_bytes),
        (1, small_cost(), 700)
    );
    assert_eq!(stats.evicted, 2, "a lower cap counts its evictions too");
}

#[test]
fn a_higher_cap_evicts_nothing_and_a_cap_of_zero_empties_the_store() {
    let mut registry = registry(2000);
    capture_tab(&mut registry, A, secs(5), snapshot(10, 10, 1));

    assert_eq!(registry.set_cap_bytes(10_000), 0);
    assert_eq!(registry.stats().cap_bytes, 10_000);
    assert!(registry.peek(A).is_some());

    assert_eq!(registry.set_cap_bytes(0), 1);
    let stats = registry.stats();
    assert_eq!(
        (stats.entries, stats.used_bytes, stats.cap_bytes),
        (0, 0, 0)
    );
    assert_eq!(stats.evicted, 1);
}

#[test]
fn a_picture_larger_than_the_whole_cap_is_not_stored() {
    let mut registry = registry(small_cost() - 1);

    assert_eq!(
        capture_tab(&mut registry, A, secs(5), snapshot(10, 10, 1)),
        FinishOutcome::TooLarge
    );

    assert_eq!(registry.peek(A), None);
    let stats = registry.stats();
    assert_eq!((stats.entries, stats.used_bytes), (0, 0));
    assert_eq!((stats.stored, stats.too_large, stats.failed), (0, 1, 0));
    // The capture itself worked, so the scheduler does not retry it for the same frame.
    assert_eq!(
        registry.capture_now(A, SIZE, secs(8)),
        Err(Skip::NoNewFrame)
    );
}

#[test]
fn a_picture_too_large_to_store_leaves_the_tabs_older_one_in_place() {
    let mut registry = registry(600);
    assert_eq!(
        capture_tab(&mut registry, A, secs(5), snapshot(4, 4, 1)),
        FinishOutcome::Stored { evicted: 0 }
    );
    quiet_frame(&mut registry, A, secs(20));
    let request = leave(&mut registry, A, secs(21));

    // 10 by 10 pixels cost 656, more than the cap of 600.
    assert_eq!(
        registry.finish(request, Some(snapshot(10, 10, 2)), secs(22)),
        FinishOutcome::TooLarge
    );

    assert_eq!(registry.peek(A), Some(&snapshot(4, 4, 1)));
}

// ----- the rest of the surface -----

#[test]
fn a_rendering_context_made_again_keeps_the_stored_pictures_and_forgets_the_frame() {
    let mut registry = registry(DEFAULT_CAP_BYTES);
    capture_tab(&mut registry, A, secs(5), snapshot(10, 10, 1));
    let before = registry.stats();

    registry.rendering_changed();

    assert_eq!(registry.stats(), before);
    assert_eq!(registry.peek(A), Some(&snapshot(10, 10, 1)));
    assert_eq!(
        registry.tab_leaving(A, SIZE, secs(30)),
        Err(Skip::NoFrame),
        "what the context holds is unknown until the next frame"
    );
}

#[test]
fn the_live_fields_of_the_stats_follow_the_store() {
    let mut registry = registry(5000);
    assert_eq!(
        registry.stats(),
        Stats {
            cap_bytes: 5000,
            ..Stats::default()
        }
    );

    capture_tab(&mut registry, A, secs(5), snapshot(10, 10, 1));
    capture_tab(&mut registry, B, secs(10), snapshot(20, 10, 1));
    let stats = registry.stats();
    assert_eq!(stats.entries, 2);
    assert_eq!(stats.used_bytes, small_cost() + cost(20, 10));

    registry.forget(A);
    let stats = registry.stats();
    assert_eq!(stats.entries, 1);
    assert_eq!(stats.used_bytes, cost(20, 10));
}

#[test]
fn the_counters_add_up_to_the_number_of_results() {
    let mut registry = registry(2 * small_cost());
    let mut results = 0;

    // Three stored (the third evicts one), then a picture too large for the cap.
    for (tab, at) in [(A, 5), (B, 10), (C, 15)] {
        capture_tab(&mut registry, tab, secs(at), snapshot(10, 10, 1));
        results += 1;
    }
    capture_tab(&mut registry, D, secs(20), snapshot(30, 30, 1));
    results += 1;

    // A failure.
    quiet_frame(&mut registry, 5, secs(100));
    let request = leave(&mut registry, 5, secs(101));
    registry.finish(request, None, secs(102));
    results += 1;

    // A result for a tab that closed.
    quiet_frame(&mut registry, 6, secs(200));
    let request = leave(&mut registry, 6, secs(201));
    registry.forget(6);
    registry.finish(request, Some(snapshot(10, 10, 1)), secs(202));
    results += 1;

    let stats = registry.stats();
    assert_eq!(
        (stats.stored, stats.too_large, stats.failed, stats.discarded),
        (3, 1, 1, 1)
    );
    assert_eq!(
        stats.stored + stats.too_large + stats.failed + stats.discarded,
        results
    );
    assert_eq!(stats.evicted, 1);
    assert_eq!(stats.entries, 2);
}
