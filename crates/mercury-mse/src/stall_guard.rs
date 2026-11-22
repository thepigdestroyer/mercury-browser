// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The underrun decision of a MediaSource element (lane med-1b, decision D4). Pure state, so it is
//! tested here without GStreamer or Servo; the element (`dom/mse/mediaelement.rs`) calls it on every
//! position update (100 ms for MediaSource players) and on every change of the buffered data, and acts
//! on the answer through the player: pause the pipeline and fire `waiting`, or resume it and fire
//! `playing`. The page-visible `paused` attribute is never touched.
//!
//! Why the DOM and not the pipeline: `GstPlay` raises no buffering message for an `appsrc` source, and a
//! feed-side "dry" signal cannot tell when the pipeline has played out the data it already holds. The
//! position and the buffered ranges are what the specification's readiness rules use.
//!
//! ## The rule
//!
//! [`StallGuard::update`] looks at one input and applies these steps in order:
//!
//! 1. `seeking`: any stall is cleared **without an action** and the hysteresis clock is forgotten (a
//!    seek starts a new situation; the element's seek handling owns the pipeline). Answer `None`.
//! 2. Stalled and not `playing` (the page paused during the stall): the stall is cleared **without an
//!    action**; the pipeline is already paused, and the page's own `play()` resumes it. Answer `None`.
//! 3. Stalled: [`StallAction::LeaveStall`] when `ended` or `ready >= HaveFutureData`, but only
//!    [`HYSTERESIS_MS`] or more after the last transition. Otherwise `None`.
//! 4. Not stalled: [`StallAction::EnterStall`] when `playing && !ended && ready <= HaveCurrentData`,
//!    but only [`HYSTERESIS_MS`] or more after the last transition (the first one is never delayed).
//!    Otherwise `None`.
//!
//! Round 1's readiness rule (D13) makes `HaveFutureData` need at least max(100 ms, two frame durations)
//! of data ahead of the position, and the position is reported every 100 ms for MediaSource players, so
//! the guard enters the stall while some data is still ahead, not after the clock ran out. Resume uses
//! `HaveFutureData`, not `HaveCurrentData`, so a trickle of data does not make playback stutter.
//!
//! Consequences the caller relies on: the same state twice gives `None`; `EnterStall` and `LeaveStall`
//! alternate between seeks; two answers other than `None` are never closer than [`HYSTERESIS_MS`] unless
//! a seek or a pause came between; a stall never starts while paused, seeking or ended. The guard clears
//! a stall silently in steps 1 and 2, so the element compares [`StallGuard::is_stalled`] before and
//! after the call: `true` then `false` with answer `None` means the guard let go and the element drops
//! its `stalled_by_mse` flag (it resumes nothing).
//!
//! `position` is part of the input for the element's logs and for later rules; the current rule uses
//! only the readiness, which already is a function of the position. `now_ms` may be any monotonic
//! clock; a clock that steps back is treated as no time having passed.

use crate::api::ElementReadyState;

/// The least time between two transitions, milliseconds (decision D4).
pub const HYSTERESIS_MS: u64 = 250;

/// What the element tells the guard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StallInput {
    /// The playback position, seconds.
    pub position: f64,
    /// The readiness the backend computed for `position` (`element_ready_state`).
    pub ready: ElementReadyState,
    /// The page wants playback (not paused, not ended) and the pipeline is meant to run.
    pub playing: bool,
    /// A seek is in progress.
    pub seeking: bool,
    /// The MediaSource has ended and the data reaches its end.
    pub ended: bool,
    /// A monotonic clock, milliseconds (the hysteresis needs one; tests pass their own).
    pub now_ms: u64,
}

/// What the element does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StallAction {
    None,
    /// Fire `waiting`, pause the pipeline, remember that MSE paused it.
    EnterStall,
    /// Resume the pipeline MSE paused, fire `playing`.
    LeaveStall,
}

/// The stall state of one MediaSource element.
#[derive(Clone, Debug, Default)]
pub struct StallGuard {
    stalled: bool,
    last_transition_ms: Option<u64>,
}

impl StallGuard {
    pub fn new() -> Self {
        StallGuard::default()
    }

    /// Takes the element's state and says what to do.
    pub fn update(&mut self, input: StallInput) -> StallAction {
        if input.seeking {
            self.stalled = false;
            self.last_transition_ms = None;
            return StallAction::None;
        }
        if self.stalled && !input.playing {
            self.stalled = false;
            return StallAction::None;
        }
        let settled = self
            .last_transition_ms
            .is_none_or(|last| input.now_ms.saturating_sub(last) >= HYSTERESIS_MS);
        if !settled {
            return StallAction::None;
        }
        if self.stalled {
            if input.ended || input.ready >= ElementReadyState::HaveFutureData {
                self.stalled = false;
                self.last_transition_ms = Some(input.now_ms);
                return StallAction::LeaveStall;
            }
        } else if input.playing && !input.ended && input.ready <= ElementReadyState::HaveCurrentData
        {
            self.stalled = true;
            self.last_transition_ms = Some(input.now_ms);
            return StallAction::EnterStall;
        }
        StallAction::None
    }

    /// Milliseconds of the hysteresis still to wait at `now_ms` (0 when no transition was made yet or
    /// the 250 ms have passed): how long the [`crate::stall_controller::StallController`] asks the element
    /// to wait before it looks again.
    pub fn hysteresis_left_ms(&self, now_ms: u64) -> u64 {
        self.last_transition_ms.map_or(0, |last| {
            HYSTERESIS_MS.saturating_sub(now_ms.saturating_sub(last))
        })
    }

    /// `true` while the pipeline is paused by the guard.
    pub fn is_stalled(&self) -> bool {
        self.stalled
    }
}
