// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// Mercury: (MED-1b) the state the element keeps for the underrun decision of decision D4 of lane
// med-1b: the pure `StallController` of `crates/mercury-mse` (it owns the guard, the hysteresis and the
// "re-check pending" bookkeeping, and takes the element's raw facts) and a clock for it. It lives on the
// `MediaSource` (a media element attaches one MediaSource for its whole life), so the upstream element
// struct gets no new field.

use std::time::Instant;

use servo_media::player::mse::stall_controller::StallController;

pub(crate) struct StallState {
    pub(crate) controller: StallController,
    epoch: Instant,
}

impl StallState {
    pub(crate) fn new() -> StallState {
        StallState {
            controller: StallController::new(),
            epoch: Instant::now(),
        }
    }

    /// The monotonic clock the guard's hysteresis reads, in milliseconds.
    pub(crate) fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}
