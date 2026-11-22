// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The underrun controller of a MediaSource element, from raw element facts (med-1b review fix, findings 5
//! and 11). It wraps the [`StallGuard`]: the element hands over what it knows ([`ElementSnapshot`]), the
//! controller builds the guard's input itself, keeps the guard and the "a re-check is already pending"
//! bookkeeping, and answers with what to do ([`StallStep`]). The element keeps only the player calls and
//! the timer, so every decision of the underrun logic is tested here (`tests/stall_controller.rs`).
//!
//! What the controller computes from the raw facts:
//!
//! * `playing` = not paused, no error, network state not EMPTY, readyState not HAVE_NOTHING, not at the
//!   end of playback: what the element's old `media_source_check_stall` derived;
//! * `ended` = the MediaSource is ended **and the position has reached the end of the data**
//!   (`position >= highest buffered end`). The guard's `ended` means exactly this: an ended source with a
//!   hole in the middle of its data is still a stall at the hole (finding 11);
//! * the pipeline change: `Pause` on `EnterStall`; `Resume` on `LeaveStall` and when the guard let go
//!   silently (a seek, or the page paused during the stall), meaning "the hold is released, evaluate
//!   `update_media_state`" (which plays only a potentially playing element); otherwise `Keep`;
//! * the re-check: when the guard wants to change state and its 250 ms hysteresis refuses, nothing may be
//!   left waiting for the next position update (the pipeline is paused during a stall, so none comes).
//!   The step then carries `recheck_in_ms` (the rest of the hysteresis plus a small margin), once; further
//!   steps carry none until the element's timer has fired and told the controller ([`StallController::recheck_fired`]).

use crate::api::{ElementReadyState, TimeRange, ns_to_secs};
use crate::stall_guard::{StallAction, StallGuard, StallInput};

/// Added to the rest of the hysteresis so that the re-check lands after it, not on its last millisecond.
pub const RECHECK_MARGIN_MS: u64 = 10;

/// The raw facts of the media element at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementSnapshot {
    /// The playback position, seconds.
    pub position: f64,
    /// The readiness the backend computes for `position` (`element_ready_state`).
    pub backend_ready: ElementReadyState,
    /// The element's own `readyState` attribute.
    pub element_ready: ElementReadyState,
    /// The page's `paused` attribute.
    pub paused: bool,
    /// The element has an error.
    pub has_error: bool,
    /// `networkState` is `NETWORK_EMPTY`.
    pub network_empty: bool,
    /// The element has ended playback (looping included).
    pub ended_playback: bool,
    /// A seek is in progress.
    pub seeking: bool,
    /// The MediaSource's readyState is "ended".
    pub source_ended: bool,
    /// The end of the last buffered range of the element (`None`: nothing buffered).
    pub highest_buffered_end: Option<f64>,
    /// A monotonic clock, milliseconds.
    pub now_ms: u64,
}

/// What to do with the pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineChange {
    /// Pause it (a stall began).
    Pause,
    /// The hold is released: re-evaluate `update_media_state` (it resumes only a potentially playing
    /// element).
    Resume,
    /// Leave it.
    Keep,
}

/// The controller's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StallStep {
    /// What the guard decided (the element fires nothing for it: `waiting` and `playing` come from the
    /// readyState change).
    pub action: StallAction,
    pub pipeline: PipelineChange,
    /// Call [`StallController::recheck_fired`] and step again after this many milliseconds.
    pub recheck_in_ms: Option<u64>,
}

/// The stall state of one MediaSource element.
#[derive(Clone, Debug, Default)]
pub struct StallController {
    guard: StallGuard,
    recheck_pending: bool,
}

impl StallController {
    pub fn new() -> Self {
        StallController::default()
    }

    /// Takes the element's facts and says what to do.
    pub fn step(&mut self, snapshot: ElementSnapshot) -> StallStep {
        let playing = !snapshot.paused
            && !snapshot.has_error
            && !snapshot.network_empty
            && snapshot.element_ready != ElementReadyState::HaveNothing
            && !snapshot.ended_playback;
        let ended = snapshot.source_ended
            && snapshot
                .highest_buffered_end
                .is_none_or(|end| snapshot.position >= end);
        let was_stalled = self.guard.is_stalled();
        let action = self.guard.update(StallInput {
            position: snapshot.position,
            ready: snapshot.backend_ready,
            playing,
            seeking: snapshot.seeking,
            ended,
            now_ms: snapshot.now_ms,
        });
        let is_stalled = self.guard.is_stalled();

        let pipeline = match action {
            StallAction::EnterStall => PipelineChange::Pause,
            StallAction::LeaveStall => PipelineChange::Resume,
            StallAction::None if was_stalled && !is_stalled => PipelineChange::Resume,
            StallAction::None => PipelineChange::Keep,
        };

        let wants_change = if is_stalled {
            snapshot.backend_ready >= ElementReadyState::HaveFutureData || ended
        } else {
            playing
                && !snapshot.seeking
                && !ended
                && snapshot.backend_ready <= ElementReadyState::HaveCurrentData
        };
        let recheck_in_ms = if action == StallAction::None && wants_change && !self.recheck_pending
        {
            self.recheck_pending = true;
            Some(
                self.guard
                    .hysteresis_left_ms(snapshot.now_ms)
                    .saturating_add(RECHECK_MARGIN_MS),
            )
        } else {
            None
        };

        StallStep {
            action,
            pipeline,
            recheck_in_ms,
        }
    }

    /// The element's timer for a `recheck_in_ms` has fired (or could not be started): the next step may
    /// ask for a new one.
    pub fn recheck_fired(&mut self) {
        self.recheck_pending = false;
    }

    /// `true` while the pipeline is paused by the controller.
    pub fn is_stalled(&self) -> bool {
        self.guard.is_stalled()
    }
}

/// The end of the last range, seconds: the element's "highest buffered end" for [`ElementSnapshot`].
pub fn highest_buffered_end(ranges: &[TimeRange]) -> Option<f64> {
    ranges.iter().map(|r| r.end).max().map(ns_to_secs)
}
