// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! When a snapshot is taken, and when a capture must be refused.
//!
//! GROUNDWORK STUB (lane tab-snapshots, item 3 owns this file): the constants and the data types
//! are final; [`CaptureScheduler`] is to be written to the contracts in its comments.
//!
//! The picture source is the web view's one shared rendering context: it holds the last frame
//! that was presented, whichever tab drew it, and is cleared and redrawn by the next tab's first
//! paint and by a resize. A capture is therefore only right when the context still holds the
//! tab's own last frame, which the scheduler checks from what the app tells it: the key and the
//! surface size of the frame it last presented.

use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;
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
#[derive(Debug)]
pub struct CaptureScheduler<K> {
    tabs: HashMap<K, PhantomData<K>>,
    next_ticket: u64,
}

impl<K: Copy + Eq + Hash> CaptureScheduler<K> {
    pub fn new() -> Self {
        Self {
            tabs: HashMap::new(),
            next_ticket: 1,
        }
    }

    /// The number of tabs the scheduler knows (they presented a frame and were not forgotten).
    pub fn known_tabs(&self) -> usize {
        self.tabs.len()
    }

    pub fn frame_presented(
        &mut self,
        _key: K,
        _surface: (u32, u32),
        _now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        let _ = self.next_ticket;
        todo!("tab-snapshots item 3: frame_presented")
    }

    pub fn tab_leaving(
        &mut self,
        _key: K,
        _surface: (u32, u32),
        _now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        todo!("tab-snapshots item 3: tab_leaving")
    }

    pub fn capture_now(
        &mut self,
        _key: K,
        _surface: (u32, u32),
        _now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        todo!("tab-snapshots item 3: capture_now")
    }

    /// The capture finished and its picture was made. Returns whether the request was still
    /// wanted (see the type's contract).
    pub fn capture_succeeded(&mut self, _request: &CaptureRequest<K>, _now: Duration) -> bool {
        todo!("tab-snapshots item 3: capture_succeeded")
    }

    /// The capture could not read or make a picture. Returns whether the request was still wanted.
    pub fn capture_failed(&mut self, _request: &CaptureRequest<K>, _now: Duration) -> bool {
        todo!("tab-snapshots item 3: capture_failed")
    }

    pub fn forget(&mut self, _key: K) {
        todo!("tab-snapshots item 3: forget")
    }

    pub fn rendering_changed(&mut self) {
        todo!("tab-snapshots item 3: rendering_changed")
    }
}

impl<K: Copy + Eq + Hash> Default for CaptureScheduler<K> {
    fn default() -> Self {
        Self::new()
    }
}
