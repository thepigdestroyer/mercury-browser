// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// Mercury: (MED-1b) the state the element keeps for the underrun decision of decision D4 of lane
// med-1b: the pure `StallGuard` of `crates/mercury-mse`, a clock for its hysteresis, and whether a
// delayed re-check is already queued. It lives on the `MediaSource` (a media element attaches one
// MediaSource for its whole life), so the upstream element struct gets no new field.

use std::time::Instant;

use servo_media::player::mse::StallGuard;

pub(crate) struct StallState {
    pub(crate) guard: StallGuard,
    /// A re-check task is queued (the guard wanted to change state and had to wait out its hysteresis).
    pub(crate) recheck_pending: bool,
    epoch: Instant,
}

impl StallState {
    pub(crate) fn new() -> StallState {
        StallState {
            guard: StallGuard::new(),
            recheck_pending: false,
            epoch: Instant::now(),
        }
    }

    /// The monotonic clock the guard's hysteresis reads, in milliseconds.
    pub(crate) fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}
