// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Tests of the capture scheduler: when a picture of a tab is taken, and when a capture is
//! refused (above all, a capture that could attach one tab's picture to another). All time is
//! injected; nothing here waits.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Duration;

use mercury_tab_snapshots::schedule::{
    CaptureReason, CaptureRequest, CaptureScheduler, FAILURE_BACKOFF, IN_FLIGHT_TIMEOUT,
    MAX_CONSECUTIVE_FAILURES, MIN_REPEAT_GAP, PERIODIC_INTERVAL, Skip,
};

type Tab = u32;
type Sched = CaptureScheduler<Tab>;

const A: Tab = 1;
const B: Tab = 2;
const C: Tab = 3;

/// The size of the rendering surface in most tests.
const SIZE: (u32, u32) = (1000, 600);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// The request inside an `Ok`, or a failure that names the refusal.
#[track_caller]
fn ok(result: Result<CaptureRequest<Tab>, Skip>) -> CaptureRequest<Tab> {
    match result {
        Ok(request) => request,
        Err(skip) => panic!("expected a capture request, the scheduler refused: {skip:?}"),
    }
}

/// A presented frame that must not start a capture (the periodic one is not due).
#[track_caller]
fn quiet_frame(scheduler: &mut Sched, tab: Tab, at: Duration) {
    assert_eq!(
        scheduler.frame_presented(tab, SIZE, at),
        Err(Skip::NotDue),
        "frame of tab {tab} at {at:?}"
    );
}

/// The tab leaves, a capture is started, and it fails at the same instant.
#[track_caller]
fn fail_leaving(scheduler: &mut Sched, tab: Tab, at: Duration) {
    let request = ok(scheduler.tab_leaving(tab, SIZE, at));
    assert!(scheduler.capture_failed(&request, at), "wanted at {at:?}");
}

#[test]
fn the_constants_are_the_specified_ones() {
    assert_eq!(PERIODIC_INTERVAL, secs(30));
    assert_eq!(MIN_REPEAT_GAP, secs(1));
    assert_eq!(IN_FLIGHT_TIMEOUT, secs(10));
    assert_eq!(FAILURE_BACKOFF, secs(5));
    assert_eq!(MAX_CONSECUTIVE_FAILURES, 3);
}

#[test]
fn no_frame_before_any_frame() {
    let mut scheduler = Sched::new();
    assert_eq!(scheduler.tab_leaving(A, SIZE, ms(0)), Err(Skip::NoFrame));
    assert_eq!(
        scheduler.capture_now(A, SIZE, secs(100)),
        Err(Skip::NoFrame)
    );
    assert_eq!(scheduler.known_tabs(), 0);
}

#[test]
fn the_first_frame_starts_the_periodic_clock() {
    let mut scheduler = Sched::new();
    // Frames every half second from t = 5 s: not due, however many arrive (later frames must not
    // move the clock).
    let mut at = ms(5_000);
    while at < ms(35_000) {
        quiet_frame(&mut scheduler, A, at);
        at += ms(500);
    }
    quiet_frame(&mut scheduler, A, ms(34_999));
    let request = ok(scheduler.frame_presented(A, SIZE, ms(35_000)));
    assert_eq!(request.key(), A);
    assert_eq!(request.reason(), CaptureReason::Periodic);
    assert_eq!(request.surface(), SIZE);
}

#[test]
fn busy_while_in_flight_and_the_next_periodic_counts_from_the_success() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(5_000));
    let first = ok(scheduler.frame_presented(A, SIZE, ms(35_000)));
    // Due again (there is no capture yet), but one is in flight.
    assert_eq!(
        scheduler.frame_presented(A, SIZE, ms(35_016)),
        Err(Skip::Busy)
    );
    assert_eq!(
        scheduler.frame_presented(A, SIZE, ms(40_000)),
        Err(Skip::Busy)
    );
    assert!(scheduler.capture_succeeded(&first, ms(36_000)));
    // 30 s after the success's time, not after the request's.
    quiet_frame(&mut scheduler, A, ms(36_016));
    quiet_frame(&mut scheduler, A, ms(60_000));
    quiet_frame(&mut scheduler, A, ms(65_999));
    let second = ok(scheduler.frame_presented(A, SIZE, ms(66_000)));
    assert_eq!(second.reason(), CaptureReason::Periodic);
    assert!(second.ticket() > first.ticket());
}

#[test]
fn leaving_needs_a_frame_since_the_last_capture() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(1_000));
    let request = ok(scheduler.tab_leaving(A, SIZE, ms(2_000)));
    assert_eq!(request.key(), A);
    assert_eq!(request.reason(), CaptureReason::Leaving);
    assert_eq!(request.surface(), SIZE);
    assert!(scheduler.capture_succeeded(&request, ms(2_000)));

    // No frame since the capture: the same picture again, so no capture (and this is not TooSoon).
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(2_100)),
        Err(Skip::NoNewFrame)
    );
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(10_000)),
        Err(Skip::NoNewFrame)
    );
    assert_eq!(
        scheduler.capture_now(A, SIZE, ms(10_000)),
        Err(Skip::NoNewFrame)
    );

    // One frame later it is allowed again.
    quiet_frame(&mut scheduler, A, ms(11_000));
    let again = ok(scheduler.tab_leaving(A, SIZE, ms(12_000)));
    assert_eq!(again.reason(), CaptureReason::Leaving);
}

#[test]
fn a_failed_capture_keeps_the_frames_since_the_last_good_one() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    fail_leaving(&mut scheduler, A, ms(1_000));
    // The picture is still missing, so once the backoff is over the tab may be captured again
    // without a new frame.
    ok(scheduler.tab_leaving(A, SIZE, ms(6_000)));
}

#[test]
fn leaving_a_tab_that_never_presented_is_the_wrong_tab() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    assert_eq!(
        scheduler.tab_leaving(B, SIZE, ms(1_000)),
        Err(Skip::WrongTab)
    );
    assert_eq!(
        scheduler.capture_now(B, SIZE, ms(1_000)),
        Err(Skip::WrongTab)
    );
    // Asking about a tab does not make the scheduler know it.
    assert_eq!(scheduler.known_tabs(), 1);
}

#[test]
fn only_the_tab_of_the_last_presented_frame_may_be_captured() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    quiet_frame(&mut scheduler, B, ms(100));
    // The context holds B's frame now, so A's picture cannot be read from it.
    assert_eq!(scheduler.tab_leaving(A, SIZE, ms(200)), Err(Skip::WrongTab));
    assert_eq!(scheduler.capture_now(A, SIZE, ms(200)), Err(Skip::WrongTab));
    // Back to A: now it is B that is not in the context.
    quiet_frame(&mut scheduler, A, ms(300));
    assert_eq!(scheduler.tab_leaving(B, SIZE, ms(400)), Err(Skip::WrongTab));
    ok(scheduler.tab_leaving(A, SIZE, ms(400)));
}

#[test]
fn a_resized_surface_is_refused() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(1_000));
    assert_eq!(
        scheduler.tab_leaving(A, (1000, 700), ms(2_000)),
        Err(Skip::SizeChanged)
    );
    assert_eq!(
        scheduler.capture_now(A, (1100, 600), ms(2_000)),
        Err(Skip::SizeChanged)
    );
    assert_eq!(
        scheduler.tab_leaving(A, (600, 1000), ms(2_000)),
        Err(Skip::SizeChanged)
    );
    // The same size is fine.
    ok(scheduler.tab_leaving(A, SIZE, ms(2_000)));
}

#[test]
fn a_frame_at_the_new_size_makes_the_new_size_current() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(1_000));
    assert_eq!(
        scheduler.frame_presented(A, (1000, 700), ms(2_000)),
        Err(Skip::NotDue)
    );
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(2_100)),
        Err(Skip::SizeChanged)
    );
    let request = ok(scheduler.tab_leaving(A, (1000, 700), ms(2_200)));
    assert_eq!(request.surface(), (1000, 700));
}

#[test]
fn too_soon_inside_a_second_of_the_last_capture() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    let first = ok(scheduler.tab_leaving(A, SIZE, ms(100)));
    assert!(scheduler.capture_succeeded(&first, ms(100)));
    quiet_frame(&mut scheduler, A, ms(500));
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(1_099)),
        Err(Skip::TooSoon)
    );
    let second = ok(scheduler.tab_leaving(A, SIZE, ms(1_100)));
    assert_eq!(second.reason(), CaptureReason::Leaving);
}

#[test]
fn on_demand_has_no_too_soon_but_still_needs_a_new_frame() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    let first = ok(scheduler.capture_now(A, SIZE, ms(100)));
    assert_eq!(first.reason(), CaptureReason::OnDemand);
    assert!(scheduler.capture_succeeded(&first, ms(100)));

    assert_eq!(
        scheduler.capture_now(A, SIZE, ms(200)),
        Err(Skip::NoNewFrame)
    );
    quiet_frame(&mut scheduler, A, ms(300));
    // The same state: leaving is too soon, on demand is not.
    assert_eq!(scheduler.tab_leaving(A, SIZE, ms(400)), Err(Skip::TooSoon));
    let second = ok(scheduler.capture_now(A, SIZE, ms(400)));
    assert_eq!(second.reason(), CaptureReason::OnDemand);
}

#[test]
fn a_failed_capture_backs_off_for_five_seconds() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    let request = ok(scheduler.tab_leaving(A, SIZE, ms(1_000)));
    assert!(scheduler.capture_failed(&request, ms(2_000)));

    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(2_000)),
        Err(Skip::Backoff)
    );
    // Counted from the moment the failure was reported (2 s), not from the request (1 s).
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(6_000)),
        Err(Skip::Backoff)
    );
    // A periodic frame that is not due is NotDue, not Backoff.
    quiet_frame(&mut scheduler, A, ms(4_000));
    assert_eq!(
        scheduler.capture_now(A, SIZE, ms(6_999)),
        Err(Skip::Backoff)
    );
    let again = ok(scheduler.capture_now(A, SIZE, ms(7_000)));
    assert_eq!(again.reason(), CaptureReason::OnDemand);
}

#[test]
fn three_failures_in_a_row_disable_captures_until_rendering_changed() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    fail_leaving(&mut scheduler, A, ms(1_000));
    // The backoff is over after 5 s; two failures do not disable.
    fail_leaving(&mut scheduler, A, ms(6_000));
    fail_leaving(&mut scheduler, A, ms(11_000));

    // Disabled whatever the call and however long we wait.
    for at in [ms(16_000), secs(100), secs(10_000)] {
        assert_eq!(scheduler.tab_leaving(A, SIZE, at), Err(Skip::Disabled));
        assert_eq!(scheduler.capture_now(A, SIZE, at), Err(Skip::Disabled));
        assert_eq!(scheduler.frame_presented(A, SIZE, at), Err(Skip::Disabled));
    }

    // The rendering context was made again: no frame is known, nothing is disabled or backing
    // off, and the failure count starts again from zero.
    scheduler.rendering_changed();
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, secs(10_001)),
        Err(Skip::NoFrame)
    );
    // The first frame of the new context starts the tab's periodic clock afresh (it may be a
    // partial first paint), so its periodic capture is 30 s later.
    quiet_frame(&mut scheduler, A, secs(10_002));
    quiet_frame(&mut scheduler, A, secs(10_031));
    let request = ok(scheduler.frame_presented(A, SIZE, secs(10_032)));
    assert_eq!(request.reason(), CaptureReason::Periodic);
    assert!(scheduler.capture_failed(&request, secs(10_033)));
    assert_eq!(
        scheduler.capture_now(A, SIZE, secs(10_034)),
        Err(Skip::Backoff)
    );
    ok(scheduler.capture_now(A, SIZE, secs(10_038)));
}

#[test]
fn a_success_resets_the_failure_count() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    fail_leaving(&mut scheduler, A, ms(1_000));
    fail_leaving(&mut scheduler, A, ms(6_000));
    let request = ok(scheduler.tab_leaving(A, SIZE, ms(11_000)));
    assert!(scheduler.capture_succeeded(&request, ms(11_000)));
    quiet_frame(&mut scheduler, A, ms(11_500));

    // Two more failures are again not enough (each `fail_leaving` asserts that the request was
    // allowed, so none of these is Disabled)...
    fail_leaving(&mut scheduler, A, ms(13_000));
    fail_leaving(&mut scheduler, A, ms(18_000));
    // ...and the third in a row disables.
    fail_leaving(&mut scheduler, A, ms(23_000));
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(30_000)),
        Err(Skip::Disabled)
    );
}

#[test]
fn forget_while_in_flight_discards_the_result_and_frees_the_slot() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    let request = ok(scheduler.tab_leaving(A, SIZE, ms(1_000)));
    // B paints while A's request is still out, so B's request has to wait for it.
    quiet_frame(&mut scheduler, B, ms(1_050));
    assert_eq!(scheduler.tab_leaving(B, SIZE, ms(1_060)), Err(Skip::Busy));

    scheduler.forget(A);
    assert_eq!(scheduler.known_tabs(), 1);
    // The result of the closed tab's request is not wanted, whichever way it ends, and it
    // counts for nothing (no failure, no backoff) and does not bring the tab back.
    assert!(!scheduler.capture_succeeded(&request, ms(1_100)));
    assert!(!scheduler.capture_failed(&request, ms(1_100)));
    assert_eq!(scheduler.known_tabs(), 1);

    // The slot is free and there is no backoff: B can be captured at once.
    let for_b = ok(scheduler.tab_leaving(B, SIZE, ms(1_200)));
    assert_eq!(for_b.key(), B);
    assert!(for_b.ticket() > request.ticket());
    assert!(scheduler.capture_succeeded(&for_b, ms(1_300)));
}

#[test]
fn a_late_result_after_a_timeout_is_discarded_and_leaves_a_newer_request_alone() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    let old = ok(scheduler.tab_leaving(A, SIZE, ms(1_000)));

    // 10.5 s later the old request is overdue: dropped, counted as a failure, backoff from now.
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(11_500)),
        Err(Skip::Backoff)
    );
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(16_499)),
        Err(Skip::Backoff)
    );
    let newer = ok(scheduler.tab_leaving(A, SIZE, ms(16_500)));
    assert!(newer.ticket() > old.ticket());

    // The old request's result turns up now. It is not wanted, and it changes nothing.
    assert!(!scheduler.capture_succeeded(&old, ms(17_000)));
    assert!(!scheduler.capture_failed(&old, ms(17_000)));
    // Still the newer request in flight: not a new request (Ok), not "no new frame" or "too soon"
    // (which a stale success would have caused), not a backoff.
    assert_eq!(scheduler.tab_leaving(A, SIZE, ms(17_500)), Err(Skip::Busy));
    assert!(scheduler.capture_succeeded(&newer, ms(18_000)));

    // A stale failure would have started a backoff until 22 s.
    quiet_frame(&mut scheduler, A, ms(18_100));
    let on_demand = ok(scheduler.capture_now(A, SIZE, ms(18_200)));
    assert_eq!(on_demand.reason(), CaptureReason::OnDemand);
}

#[test]
fn an_overdue_request_is_dropped_and_counted_as_a_failure() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    let request = ok(scheduler.tab_leaving(A, SIZE, ms(1_000)));

    // Nine and a bit seconds old: still in flight.
    assert_eq!(scheduler.tab_leaving(A, SIZE, ms(10_999)), Err(Skip::Busy));
    // A little over ten: dropped, counted as a failure, so the decision is a Backoff from this call.
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(11_001)),
        Err(Skip::Backoff)
    );
    assert!(!scheduler.capture_succeeded(&request, ms(11_002)));
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(16_000)),
        Err(Skip::Backoff)
    );
    ok(scheduler.tab_leaving(A, SIZE, ms(16_001)));
}

#[test]
fn an_overdue_request_is_also_dropped_by_a_presented_frame() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    ok(scheduler.tab_leaving(A, SIZE, ms(1_000)));
    // The frame itself is NotDue (periodic not due), but the decision dropped the request.
    quiet_frame(&mut scheduler, A, ms(11_001));
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(11_002)),
        Err(Skip::Backoff)
    );
}

#[test]
fn timeouts_count_towards_disabling() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    ok(scheduler.tab_leaving(A, SIZE, ms(1_000)));
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(11_001)),
        Err(Skip::Backoff)
    );
    ok(scheduler.tab_leaving(A, SIZE, ms(16_001)));
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(26_002)),
        Err(Skip::Backoff)
    );
    ok(scheduler.tab_leaving(A, SIZE, ms(31_002)));
    // The third timeout in a row: the decision that finds it is already Disabled.
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(41_003)),
        Err(Skip::Disabled)
    );
}

#[test]
fn rendering_changed_discards_a_request_in_flight_and_forgets_the_frame() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    let request = ok(scheduler.tab_leaving(A, SIZE, ms(1_000)));
    scheduler.rendering_changed();

    // The tabs are still known (their stored pictures stay valid); the request is not wanted.
    assert_eq!(scheduler.known_tabs(), 1);
    assert!(!scheduler.capture_succeeded(&request, ms(1_100)));
    assert!(!scheduler.capture_failed(&request, ms(1_100)));
    // The context holds nothing known until a frame is presented again.
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(1_200)),
        Err(Skip::NoFrame)
    );
    quiet_frame(&mut scheduler, A, ms(1_300));
    // The slot is free and the discarded result left no backoff.
    let again = ok(scheduler.tab_leaving(A, SIZE, ms(1_400)));
    assert!(again.ticket() > request.ticket());
    // The old request cannot be mistaken for the new one.
    assert!(!scheduler.capture_succeeded(&request, ms(1_500)));
    assert!(scheduler.capture_succeeded(&again, ms(1_500)));
}

#[test]
fn rendering_changed_clears_a_backoff() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    fail_leaving(&mut scheduler, A, ms(1_000));
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(2_000)),
        Err(Skip::Backoff)
    );
    scheduler.rendering_changed();
    quiet_frame(&mut scheduler, A, ms(2_100));
    ok(scheduler.tab_leaving(A, SIZE, ms(2_200)));
}

#[test]
fn a_closed_active_tab_gets_no_capture() {
    // What the engine does when the active tab is closed: the last frame was A's, the tab is
    // forgotten in the closed-tabs loop, and then the tab switch asks about A.
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(1_000));
    scheduler.forget(A);
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(1_100)),
        Err(Skip::NoFrame)
    );
    assert_eq!(
        scheduler.capture_now(A, SIZE, ms(1_100)),
        Err(Skip::NoFrame)
    );
    assert_eq!(scheduler.known_tabs(), 0);

    // The same with a request in flight when the tab is closed.
    quiet_frame(&mut scheduler, B, ms(2_000));
    let request = ok(scheduler.tab_leaving(B, SIZE, ms(2_100)));
    scheduler.forget(B);
    assert_eq!(
        scheduler.tab_leaving(B, SIZE, ms(2_200)),
        Err(Skip::NoFrame)
    );
    assert!(!scheduler.capture_succeeded(&request, ms(2_300)));

    // The tab that takes over is captured normally.
    quiet_frame(&mut scheduler, C, ms(3_000));
    ok(scheduler.tab_leaving(C, SIZE, ms(3_100)));
}

#[test]
fn forgetting_another_tab_keeps_the_frame_and_the_request() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    quiet_frame(&mut scheduler, B, ms(100));
    scheduler.forget(A);
    assert_eq!(scheduler.known_tabs(), 1);
    let request = ok(scheduler.tab_leaving(B, SIZE, ms(200)));
    scheduler.forget(C); // never seen
    scheduler.forget(A); // already gone
    assert_eq!(scheduler.known_tabs(), 1);
    assert!(scheduler.capture_succeeded(&request, ms(300)));
}

#[test]
fn rapid_switching_captures_a_tab_at_most_once_a_second() {
    let mut scheduler = Sched::new();
    let tabs = [A, B];
    let mut captured_at: [Vec<Duration>; 2] = [Vec::new(), Vec::new()];
    let mut too_soon = 0;
    let mut now = ms(0);
    let mut current = 0;
    quiet_frame(&mut scheduler, tabs[current], now);
    // 100 switches, A to B to A..., each tab active for 100 ms and presenting a frame (11 s in
    // all, so the 30 s periodic capture never gets involved).
    for _ in 0..100 {
        now += ms(100);
        match scheduler.tab_leaving(tabs[current], SIZE, now) {
            Ok(request) => {
                assert_eq!(request.key(), tabs[current]);
                captured_at[current].push(now);
                assert!(scheduler.capture_succeeded(&request, now));
            }
            Err(Skip::TooSoon) => too_soon += 1,
            Err(other) => panic!("unexpected refusal {other:?} at {now:?}"),
        }
        current = 1 - current;
        now += ms(10);
        quiet_frame(&mut scheduler, tabs[current], now);
    }

    assert!(too_soon > 0, "the one-second rule never applied");
    for (tab, times) in tabs.iter().zip(&captured_at) {
        assert!(!times.is_empty(), "tab {tab} was never captured");
        assert!(times.len() <= 12, "tab {tab} was captured {times:?}");
        for pair in times.windows(2) {
            assert!(
                pair[1] - pair[0] >= MIN_REPEAT_GAP,
                "tab {tab} captured twice within a second: {pair:?}"
            );
        }
    }
}

#[test]
fn a_tab_that_comes_back_is_not_captured_on_its_first_frame_back() {
    // Integrator finding on item 3: the periodic clock ran from the tab's last capture alone, so
    // a tab that returned after more than 30 s was captured on its first presented frame, which
    // can be a partial first paint that would replace the good picture taken when it left.
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    let leaving = ok(scheduler.tab_leaving(A, SIZE, ms(2_000)));
    assert!(scheduler.capture_succeeded(&leaving, ms(2_000)));
    quiet_frame(&mut scheduler, B, ms(2_010));

    // A returns 98 s after its last capture and presents its first frame, then more.
    quiet_frame(&mut scheduler, A, ms(100_000));
    quiet_frame(&mut scheduler, A, ms(100_016));
    quiet_frame(&mut scheduler, A, ms(129_999));
    // The clock runs from the first frame back, not from the old capture.
    let request = ok(scheduler.frame_presented(A, SIZE, ms(130_000)));
    assert_eq!(request.reason(), CaptureReason::Periodic);
    assert_eq!(request.key(), A);
}

#[test]
fn a_tab_that_never_got_a_picture_restarts_its_clock_when_it_comes_back() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    // The leaving capture of A failed, so A has no picture; B is shown, and A comes back at 50 s.
    fail_leaving(&mut scheduler, A, ms(1_000));
    quiet_frame(&mut scheduler, B, ms(1_010));
    quiet_frame(&mut scheduler, A, ms(50_000));
    quiet_frame(&mut scheduler, A, ms(79_999));
    let request = ok(scheduler.frame_presented(A, SIZE, ms(80_000)));
    assert_eq!(request.reason(), CaptureReason::Periodic);
}

#[test]
fn a_tab_that_stays_active_keeps_its_clock_when_the_frames_go_on() {
    // The restart is for a tab that becomes the presented one, not for every frame.
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    quiet_frame(&mut scheduler, B, ms(10));
    quiet_frame(&mut scheduler, A, ms(20));
    let mut at = ms(20);
    while at < ms(30_019) {
        at += ms(250);
        quiet_frame(&mut scheduler, A, at.min(ms(30_019)));
    }
    let request = ok(scheduler.frame_presented(A, SIZE, ms(30_020)));
    assert_eq!(request.reason(), CaptureReason::Periodic);
}

#[test]
fn rendering_changed_restarts_the_clock_of_the_next_frame() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    scheduler.rendering_changed();
    quiet_frame(&mut scheduler, A, ms(40_000));
    quiet_frame(&mut scheduler, A, ms(69_999));
    let request = ok(scheduler.frame_presented(A, SIZE, ms(70_000)));
    assert_eq!(request.reason(), CaptureReason::Periodic);
}
#[test]
fn known_tabs_follows_the_frames_and_forget() {
    let mut scheduler = Sched::new();
    assert_eq!(scheduler.known_tabs(), 0);
    quiet_frame(&mut scheduler, A, ms(0));
    assert_eq!(scheduler.known_tabs(), 1);
    quiet_frame(&mut scheduler, A, ms(10));
    assert_eq!(scheduler.known_tabs(), 1);
    quiet_frame(&mut scheduler, B, ms(20));
    assert_eq!(scheduler.known_tabs(), 2);
    scheduler.forget(A);
    assert_eq!(scheduler.known_tabs(), 1);
    scheduler.forget(A);
    assert_eq!(scheduler.known_tabs(), 1);
    scheduler.forget(B);
    assert_eq!(scheduler.known_tabs(), 0);
}

#[test]
fn tickets_are_unique_and_increasing() {
    let mut scheduler = Sched::new();
    let mut tickets = Vec::new();
    let mut now = ms(0);
    quiet_frame(&mut scheduler, A, now);
    for round in 0..10 {
        now += secs(2);
        if round == 4 {
            scheduler.rendering_changed();
        }
        if round == 6 {
            scheduler.forget(A);
        }
        // A frame, so that a decision after each of those events has one to work with (the
        // whole test lasts 20 s, so the periodic capture is never due).
        quiet_frame(&mut scheduler, A, now);
        now += ms(1);
        let request = ok(scheduler.tab_leaving(A, SIZE, now));
        tickets.push(request.ticket());
        assert!(scheduler.capture_succeeded(&request, now));
    }
    assert!(
        tickets.windows(2).all(|pair| pair[0] < pair[1]),
        "tickets {tickets:?}"
    );
}

#[test]
fn refusals_are_checked_in_the_documented_order() {
    // Disabled before NoFrame: forget clears the presented frame but not the disabled state.
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, ms(0));
    fail_leaving(&mut scheduler, A, ms(1_000));
    fail_leaving(&mut scheduler, A, ms(6_000));
    fail_leaving(&mut scheduler, A, ms(11_000));
    scheduler.forget(A);
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(20_000)),
        Err(Skip::Disabled)
    );

    // NoFrame before WrongTab and SizeChanged.
    let mut scheduler = Sched::new();
    assert_eq!(scheduler.tab_leaving(B, (1, 1), ms(0)), Err(Skip::NoFrame));

    // WrongTab before SizeChanged.
    quiet_frame(&mut scheduler, A, ms(0));
    assert_eq!(
        scheduler.tab_leaving(B, (1, 1), ms(100)),
        Err(Skip::WrongTab)
    );

    // SizeChanged before NoNewFrame, and NoNewFrame before TooSoon (the tab was just captured).
    let request = ok(scheduler.tab_leaving(A, SIZE, ms(1_000)));
    assert!(scheduler.capture_succeeded(&request, ms(1_000)));
    assert_eq!(
        scheduler.tab_leaving(A, (5, 5), ms(1_100)),
        Err(Skip::SizeChanged)
    );
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(1_100)),
        Err(Skip::NoNewFrame)
    );

    // TooSoon and NotDue before Busy: a request for B is in flight while A is asked about.
    quiet_frame(&mut scheduler, B, ms(1_200));
    let for_b = ok(scheduler.tab_leaving(B, SIZE, ms(1_300)));
    quiet_frame(&mut scheduler, A, ms(1_400)); // NotDue, not Busy
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(1_500)),
        Err(Skip::TooSoon)
    );
    assert_eq!(scheduler.capture_now(A, SIZE, ms(1_500)), Err(Skip::Busy));
    assert!(scheduler.capture_succeeded(&for_b, ms(1_600)));

    // Backoff comes last: every other refusal is reported during one.
    quiet_frame(&mut scheduler, C, ms(2_000));
    fail_leaving(&mut scheduler, C, ms(2_100));
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, ms(2_200)),
        Err(Skip::WrongTab)
    );
    assert_eq!(
        scheduler.tab_leaving(C, (5, 5), ms(2_200)),
        Err(Skip::SizeChanged)
    );
    quiet_frame(&mut scheduler, C, ms(2_300)); // NotDue, not Backoff
    assert_eq!(
        scheduler.tab_leaving(C, SIZE, ms(2_400)),
        Err(Skip::Backoff)
    );
}

#[test]
fn time_going_backwards_never_panics() {
    let mut scheduler = Sched::new();
    quiet_frame(&mut scheduler, A, secs(100));
    let request = ok(scheduler.tab_leaving(A, SIZE, secs(100)));

    // Earlier than the request was made: no time has passed, so it is not overdue.
    assert_eq!(
        scheduler.frame_presented(A, SIZE, secs(50)),
        Err(Skip::NotDue)
    );
    assert_eq!(scheduler.capture_now(A, SIZE, secs(10)), Err(Skip::Busy));
    // A result stamped before the request and before the stored times.
    assert!(scheduler.capture_succeeded(&request, secs(20)));

    // The last capture is at 20 s; asking at 1 s means no time has passed since it.
    quiet_frame(&mut scheduler, A, secs(3));
    assert_eq!(scheduler.tab_leaving(A, SIZE, secs(1)), Err(Skip::TooSoon));
    let on_demand = ok(scheduler.capture_now(A, SIZE, secs(1)));
    assert!(scheduler.capture_failed(&on_demand, Duration::ZERO));
    assert_eq!(
        scheduler.capture_now(A, SIZE, Duration::ZERO),
        Err(Skip::Backoff)
    );

    // The extremes of the range do not overflow either.
    let late = ok(scheduler.frame_presented(A, SIZE, Duration::MAX));
    assert_eq!(late.reason(), CaptureReason::Periodic);
    assert_eq!(
        scheduler.tab_leaving(A, SIZE, Duration::ZERO),
        Err(Skip::TooSoon)
    );
    assert!(scheduler.capture_failed(&late, Duration::ZERO));
    let last = ok(scheduler.capture_now(A, SIZE, Duration::MAX));
    assert_eq!(last.reason(), CaptureReason::OnDemand);
    assert!(last.ticket() > late.ticket());
}

// A counting allocator for the allocation test below. The counter is per thread (the tests run
// in parallel threads) and is a plain `Cell` with no destructor, so counting never allocates.
struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn count_allocation() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

fn allocations_on_this_thread() -> u64 {
    ALLOCATIONS.with(Cell::get)
}

// SAFETY: every method forwards to the system allocator with the same arguments; the counting
// only touches a thread-local `Cell`.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which `System` shares.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this allocator, so from `System` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_allocation();
        // SAFETY: `ptr` came from this allocator, so from `System` with this layout.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn the_per_frame_path_does_not_allocate() {
    // The counter really counts (so that "zero" below means something).
    let start = allocations_on_this_thread();
    let probe = std::hint::black_box(vec![0u8; 64]);
    assert!(allocations_on_this_thread() > start);
    drop(probe);

    let mut scheduler = Sched::new();
    // The first frame of each tab is allowed to allocate (the tab is entered in a map).
    quiet_frame(&mut scheduler, A, ms(0));
    quiet_frame(&mut scheduler, B, ms(1));

    let before = allocations_on_this_thread();
    // 60 frames a second for a while (under 30 s, so never due), switching between the two known
    // tabs now and then: always NotDue, and nothing allocated.
    for frame in 2..1_800u64 {
        let tab = if frame % 500 < 250 { A } else { B };
        let result = scheduler.frame_presented(tab, SIZE, ms(frame * 16));
        assert_eq!(result, Err(Skip::NotDue));
    }
    // A refused periodic capture (Busy) and a disabled scheduler are per-frame paths as well.
    // A becomes the presented tab again at 35 s, so its periodic capture is due at 65 s.
    quiet_frame(&mut scheduler, A, secs(35));
    let request = ok(scheduler.frame_presented(A, SIZE, secs(65)));
    for frame in 0..1_000u64 {
        let result = scheduler.frame_presented(A, SIZE, secs(65) + ms(frame));
        assert_eq!(result, Err(Skip::Busy));
    }
    assert!(scheduler.capture_failed(&request, secs(66)));
    fail_leaving(&mut scheduler, A, secs(71));
    fail_leaving(&mut scheduler, A, secs(77));
    for frame in 0..1_000u64 {
        let result = scheduler.frame_presented(A, SIZE, secs(85) + ms(frame));
        assert_eq!(result, Err(Skip::Disabled));
    }
    assert_eq!(allocations_on_this_thread() - before, 0);
}
