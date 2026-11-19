// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! When a snapshot is taken, and when a capture must be refused.
//!
//! The picture source is the web view's one shared rendering context: it holds the last frame
//! that was presented, whichever tab drew it, and is cleared and redrawn by the next tab's first
//! paint and by a resize. A capture is therefore only right when the context still holds the
//! tab's own last frame, which the scheduler checks from what the app tells it: the key and the
//! surface size of the frame it last presented ([`CaptureScheduler::frame_presented`]). A capture
//! for another tab, or at another size, is refused ([`Skip::WrongTab`], [`Skip::SizeChanged`]);
//! this is the guard that keeps one tab's picture from landing on another tab.
//!
//! **Warning for whoever changes the rendering path.** The guard knows only the frames the app
//! reports as presented. Any future code path that paints into the shared context without
//! showing the result through `present` (and so without calling `frame_presented`) would leave
//! the context holding a picture the scheduler has never heard of, while the guard still matches
//! the last presented frame, and a capture would then attach that picture to the wrong tab. Such
//! a path must either report the frame it painted, or call
//! [`CaptureScheduler::rendering_changed`] so that nothing is captured until the next
//! presented frame.
//!
//! Time is passed in by the caller as a [`Duration`] since an arbitrary start. It is never
//! assumed to be monotonic here: every difference is taken with `saturating_sub`, so a time that
//! goes backwards counts as "no time has passed" and never panics.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Duration;

/// A tab that stays active gets a new snapshot this often (SPEC section 6.1), once it has
/// presented a frame since the last one.
pub const PERIODIC_INTERVAL: Duration = Duration::from_secs(30);

/// A tab that stops being active is not captured again if its last snapshot is younger than this
/// (a user cycling quickly through tabs would otherwise cost a read-back per switch for nothing).
pub const MIN_REPEAT_GAP: Duration = Duration::from_secs(1);

/// A capture that has neither succeeded nor failed after this long is given up (its late result
/// is discarded) and counts as a failure, so a lost result cannot block captures for good.
pub const IN_FLIGHT_TIMEOUT: Duration = Duration::from_secs(10);

/// After a failed capture, none is started for this long.
pub const FAILURE_BACKOFF: Duration = Duration::from_secs(5);

/// After this many failures in a row (none succeeding in between) captures are switched off until
/// [`CaptureScheduler::rendering_changed`]. Reading pixels back is not available on every
/// platform (on Linux Mercury's rendering context returns nothing), and the scheduler must not
/// retry that on every frame.
pub const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Why a capture is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureReason {
    /// The tab is about to stop being the active one.
    Leaving,
    /// The tab has been active for [`PERIODIC_INTERVAL`] since its last snapshot.
    Periodic,
    /// Someone asked for it now (the Orbit view opening over the active tab).
    OnDemand,
}

/// Why no capture was started. Cheap to produce on every frame; the app logs the rare ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// No frame has been presented since the scheduler began, or since `rendering_changed`.
    NoFrame,
    /// The last presented frame belongs to another tab, so the context does not hold this tab.
    WrongTab,
    /// The context's size is not the size of the last presented frame (it was resized since), so
    /// its pixels are not the tab's picture.
    SizeChanged,
    /// The tab presented no frame since its last snapshot, so a new one would be the same picture.
    NoNewFrame,
    /// A periodic capture is not due yet.
    NotDue,
    /// The tab's last snapshot is younger than [`MIN_REPEAT_GAP`].
    TooSoon,
    /// Another capture is still in flight (at most one at a time: a full frame is large).
    Busy,
    /// A capture failed less than [`FAILURE_BACKOFF`] ago.
    Backoff,
    /// [`MAX_CONSECUTIVE_FAILURES`] captures failed in a row.
    Disabled,
}

/// A permission to read one tab's picture now. Valid until the matching `capture_succeeded` or
/// `capture_failed`, until the tab is forgotten, or until [`IN_FLIGHT_TIMEOUT`] passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRequest<K> {
    key: K,
    reason: CaptureReason,
    surface: (u32, u32),
    ticket: u64,
}

impl<K: Copy> CaptureRequest<K> {
    pub fn new(key: K, reason: CaptureReason, surface: (u32, u32), ticket: u64) -> Self {
        Self {
            key,
            reason,
            surface,
            ticket,
        }
    }

    pub fn key(&self) -> K {
        self.key
    }

    pub fn reason(&self) -> CaptureReason {
        self.reason
    }

    /// The size in pixels of the frame to read, the context's size when the request was made.
    pub fn surface(&self) -> (u32, u32) {
        self.surface
    }

    /// An id that is different for every request, for logs.
    pub fn ticket(&self) -> u64 {
        self.ticket
    }
}

/// What the scheduler remembers about one tab.
#[derive(Debug, Clone, Copy)]
struct TabState {
    /// When the tab's first frame was presented: the start of its periodic clock until it has a
    /// successful capture.
    first_frame: Duration,
    /// When the tab's last successful capture was reported.
    last_capture: Option<Duration>,
    /// Frames the tab presented since its last successful capture (or ever, if it has none).
    frames_since_capture: u64,
}

/// The last frame the app presented: what the shared rendering context holds.
#[derive(Debug, Clone, Copy)]
struct Presented<K> {
    key: K,
    surface: (u32, u32),
}

/// The one capture that has been permitted and not yet reported.
#[derive(Debug, Clone, Copy)]
struct InFlight<K> {
    key: K,
    ticket: u64,
    started: Duration,
}

/// The policy. One value per window; all time is passed in.
///
/// Contract (the tests of item 3 pin each line):
///
/// - [`frame_presented`](Self::frame_presented) records that `key`'s frame, `surface` pixels
///   large, is now what the context holds, counts it for the tab, and then decides a
///   [`CaptureReason::Periodic`] capture: `Ok` when [`PERIODIC_INTERVAL`] has passed since the
///   tab's last successful capture (or since its first presented frame if it has none) and no
///   other refusal applies.
/// - [`tab_leaving`](Self::tab_leaving) is called before the app lets another tab paint: `Ok`
///   ([`CaptureReason::Leaving`]) only if the last presented frame is `key`'s at the same
///   `surface` size and the tab presented a frame since its last capture and it is not
///   [`Skip::TooSoon`].
/// - [`capture_now`](Self::capture_now) is the same without the `TooSoon` rule
///   ([`CaptureReason::OnDemand`]).
/// - Every refusal names its [`Skip`]. The order the reasons are checked in is: `Disabled`,
///   `NoFrame`, `WrongTab`, `SizeChanged`, `NoNewFrame`, then (leaving only) `TooSoon` or (periodic
///   only) `NotDue`, then `Busy`, then `Backoff`.
/// - An `Ok` request marks a capture in flight; there is at most one. `capture_succeeded` and
///   `capture_failed` end it and return whether the request was still wanted: `false` when the
///   tab was forgotten, the request timed out or `rendering_changed` ran meanwhile, in which case
///   the result must be thrown away and the scheduler's counters are untouched.
/// - `capture_succeeded` records the tab's last capture time and clears its new-frame count and
///   the failure count. `capture_failed` counts a failure, starts the [`FAILURE_BACKOFF`] and
///   disables captures at [`MAX_CONSECUTIVE_FAILURES`].
/// - [`forget`](Self::forget) drops everything about a tab, including a capture in flight for it.
/// - [`rendering_changed`](Self::rendering_changed) forgets what the context holds and clears the
///   disabled state, the backoff and any capture in flight (the context was torn down and made
///   again, perhaps on another GPU).
///
/// How the contract was read where it leaves room:
///
/// - The in-flight timeout is applied at the start of every decision (`frame_presented`,
///   `tab_leaving`, `capture_now`) to a request strictly older than [`IN_FLIGHT_TIMEOUT`]. It is
///   not applied by `capture_succeeded` and `capture_failed`: a result that arrives before any
///   decision has noticed the timeout is still accepted (its picture was read while the guard
///   held), and one that arrives after the request was dropped is refused by its ticket.
/// - The failure count is cleared by `capture_succeeded` and `rendering_changed` only, so after
///   `rendering_changed` it takes [`MAX_CONSECUTIVE_FAILURES`] new failures to disable again.
/// - A refusal never changes the scheduler, except that an overdue request is dropped (and counted
///   as a failure) by the decision that finds it.
#[derive(Debug)]
pub struct CaptureScheduler<K> {
    tabs: HashMap<K, TabState>,
    presented: Option<Presented<K>>,
    in_flight: Option<InFlight<K>>,
    /// Failures in a row since the last success (or `rendering_changed`).
    failures: u32,
    /// When the last failure was reported, for the backoff.
    last_failure: Option<Duration>,
    next_ticket: u64,
}

impl<K: Copy + Eq + Hash> CaptureScheduler<K> {
    pub fn new() -> Self {
        Self {
            tabs: HashMap::new(),
            presented: None,
            in_flight: None,
            failures: 0,
            last_failure: None,
            next_ticket: 1,
        }
    }

    /// The number of tabs the scheduler knows (they presented a frame and were not forgotten).
    pub fn known_tabs(&self) -> usize {
        self.tabs.len()
    }

    /// The app showed a frame of tab `key` on a surface of `surface` pixels. Runs on every
    /// presented frame (60 times a second): its common path, which ends in [`Skip::NotDue`],
    /// neither allocates nor logs. Only the first frame of a tab enters it in the map.
    pub fn frame_presented(
        &mut self,
        key: K,
        surface: (u32, u32),
        now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        self.drop_overdue(now);
        self.presented = Some(Presented { key, surface });
        self.tabs
            .entry(key)
            .and_modify(|tab| tab.frames_since_capture = tab.frames_since_capture.saturating_add(1))
            .or_insert(TabState {
                first_frame: now,
                last_capture: None,
                frames_since_capture: 1,
            });
        self.decide(key, surface, now, CaptureReason::Periodic)
    }

    /// The app is about to let another tab paint, so the context still holds `key`'s last frame
    /// if `key` is the tab of the last presented frame, at `surface`.
    pub fn tab_leaving(
        &mut self,
        key: K,
        surface: (u32, u32),
        now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        self.drop_overdue(now);
        self.decide(key, surface, now, CaptureReason::Leaving)
    }

    /// A picture of the tab is wanted now, whatever its last snapshot's age.
    pub fn capture_now(
        &mut self,
        key: K,
        surface: (u32, u32),
        now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        self.drop_overdue(now);
        self.decide(key, surface, now, CaptureReason::OnDemand)
    }

    /// The capture finished and its picture was made. Returns whether the request was still
    /// wanted (see the type's contract).
    pub fn capture_succeeded(&mut self, request: &CaptureRequest<K>, now: Duration) -> bool {
        if !self.end_in_flight(request) {
            return false;
        }
        // The tab is always there while its request is in flight (`forget` ends the request);
        // if it were not, the picture would not be wanted.
        let Some(tab) = self.tabs.get_mut(&request.key()) else {
            return false;
        };
        tab.last_capture = Some(now);
        tab.frames_since_capture = 0;
        self.failures = 0;
        true
    }

    /// The capture could not read or make a picture. Returns whether the request was still wanted.
    pub fn capture_failed(&mut self, request: &CaptureRequest<K>, now: Duration) -> bool {
        if !self.end_in_flight(request) {
            return false;
        }
        self.count_failure(now);
        true
    }

    /// The tab was closed: drop everything about it, so that a late result for it is not wanted.
    pub fn forget(&mut self, key: K) {
        self.tabs.remove(&key);
        if self.presented.is_some_and(|presented| presented.key == key) {
            self.presented = None;
        }
        if self.in_flight.is_some_and(|flight| flight.key == key) {
            self.in_flight = None;
        }
    }

    /// The rendering context was torn down and made again: what it holds is unknown until the
    /// next presented frame, and the failures that came from the old one no longer count.
    pub fn rendering_changed(&mut self) {
        self.presented = None;
        self.in_flight = None;
        self.failures = 0;
        self.last_failure = None;
    }

    /// Whether a capture may start now, and if so, marks it in flight. The refusals are checked
    /// in the order of the type's contract; none of them changes anything.
    fn decide(
        &mut self,
        key: K,
        surface: (u32, u32),
        now: Duration,
        reason: CaptureReason,
    ) -> Result<CaptureRequest<K>, Skip> {
        if self.failures >= MAX_CONSECUTIVE_FAILURES {
            return Err(Skip::Disabled);
        }
        let Some(presented) = self.presented else {
            return Err(Skip::NoFrame);
        };
        if presented.key != key {
            return Err(Skip::WrongTab);
        }
        if presented.surface != surface {
            return Err(Skip::SizeChanged);
        }
        // `presented` is cleared when its tab is forgotten, so the tab is here; if it were not,
        // there would be no frame of it to capture.
        let Some(tab) = self.tabs.get(&key) else {
            return Err(Skip::NoFrame);
        };
        if tab.frames_since_capture == 0 {
            return Err(Skip::NoNewFrame);
        }
        match reason {
            CaptureReason::Leaving => {
                let too_soon = tab
                    .last_capture
                    .is_some_and(|last| now.saturating_sub(last) < MIN_REPEAT_GAP);
                if too_soon {
                    return Err(Skip::TooSoon);
                }
            }
            CaptureReason::Periodic => {
                let since = tab.last_capture.unwrap_or(tab.first_frame);
                if now.saturating_sub(since) < PERIODIC_INTERVAL {
                    return Err(Skip::NotDue);
                }
            }
            CaptureReason::OnDemand => {}
        }
        if self.in_flight.is_some() {
            return Err(Skip::Busy);
        }
        let backing_off = self
            .last_failure
            .is_some_and(|failed| now.saturating_sub(failed) < FAILURE_BACKOFF);
        if backing_off {
            return Err(Skip::Backoff);
        }

        let ticket = self.next_ticket;
        self.next_ticket = self.next_ticket.wrapping_add(1);
        self.in_flight = Some(InFlight {
            key,
            ticket,
            started: now,
        });
        Ok(CaptureRequest::new(key, reason, surface, ticket))
    }

    /// Gives up the request in flight if it is older than [`IN_FLIGHT_TIMEOUT`] and counts it as
    /// a failure from `now`. Runs at the start of every decision.
    fn drop_overdue(&mut self, now: Duration) {
        let overdue = self
            .in_flight
            .is_some_and(|flight| now.saturating_sub(flight.started) > IN_FLIGHT_TIMEOUT);
        if overdue {
            self.in_flight = None;
            self.count_failure(now);
        }
    }

    /// Ends the request in flight if `request` is it. Returns whether it was: `false` for a
    /// request of a forgotten tab, one that timed out, or one from before `rendering_changed`.
    fn end_in_flight(&mut self, request: &CaptureRequest<K>) -> bool {
        match self.in_flight {
            Some(flight) if flight.ticket == request.ticket() && flight.key == request.key() => {
                self.in_flight = None;
                true
            }
            _ => false,
        }
    }

    fn count_failure(&mut self, now: Duration) {
        self.failures = self.failures.saturating_add(1);
        self.last_failure = Some(now);
    }
}

impl<K: Copy + Eq + Hash> Default for CaptureScheduler<K> {
    fn default() -> Self {
        Self::new()
    }
}
