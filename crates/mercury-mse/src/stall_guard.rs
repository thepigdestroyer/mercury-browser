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
//! Skeleton: the signatures are committed by the lead; item 1 (`core`) implements the body and writes
//! the exact rule and its constants in these docs. The rule: [`StallAction::EnterStall`] when playing,
//! not seeking, not ended and the readiness is at most `HaveCurrentData`; [`StallAction::LeaveStall`]
//! when the readiness is `HaveFutureData` or more (or the source has ended), at least 250 ms after the
//! last transition; a `seeking` input clears the stall without an action; the same state again gives
//! [`StallAction::None`].

use crate::api::ElementReadyState;

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
    #[allow(dead_code)] // read by item 1 `core`
    last_transition_ms: Option<u64>,
}

impl StallGuard {
    pub fn new() -> Self {
        StallGuard::default()
    }

    /// Takes the element's state and says what to do.
    pub fn update(&mut self, _input: StallInput) -> StallAction {
        todo!("item 1 `core`")
    }

    /// `true` while the pipeline is paused by the guard.
    pub fn is_stalled(&self) -> bool {
        self.stalled
    }
}
