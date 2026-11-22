// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The operation state machine of a DOM `SourceBuffer` (med-1b review fix, findings 1 and 5a): which
//! append or remove is pending, `updating`, and which events the DOM must queue for each transition.
//! Pure state: the DOM's `SourceBuffer` owns one of these instead of a `Cell<bool>` and an
//! [`OpTracker`], and maps the returned [`SbEvent`]s to `queue_event`; everything it decides is tested
//! here without Servo.
//!
//! What it mirrors (nothing else is decided here):
//!
//! | DOM call | method | spec text |
//! |---|---|---|
//! | `appendBuffer()` accepted | [`SourceBufferOps::begin_append`] | prepare append / buffer append steps 3, 4 |
//! | `remove()` accepted | [`SourceBufferOps::begin_remove`] | range removal steps 3, 4 |
//! | `AppendDone` / `RemoveDone` | [`SourceBufferOps::finished`] | append steps 7 to 9, range removal steps 6, 7 |
//! | `AppendError` | [`SourceBufferOps::failed`] | append error steps 2 to 4 |
//! | `abort()` | [`SourceBufferOps::check_abort`], [`SourceBufferOps::abort`] | `abort()` steps 3 and 4 |
//! | `removeSourceBuffer()` | [`SourceBufferOps::abort_pending`] | step 2 |
//! | `MediaSource` close | [`SourceBufferOps::detach`] | the source buffers leave `sourceBuffers` |
//!
//! `UpdateStart` is part of [`SbEvent`] and is returned by `begin_append` and `begin_remove`, so the DOM
//! has one mapping from events to tasks and cannot forget the `updatestart` task nor queue it before
//! `updating` is true. The page-visible `updating` attribute is [`SourceBufferOps::updating`]: it is true
//! exactly when an operation is pending (`updating() == pending().is_some()`, always), and an operation
//! stays pending until its completion event was *handled by the DOM* (or it was retired), not until the
//! backend's worker finished it. That difference is finding 1: a removal that the worker has already
//! finished is still "running" for the page until `RemoveDone` is handled, so `abort()` must throw.

use std::fmt;

use crate::api::OpId;
use crate::op_tracker::OpTracker;

/// An event the DOM queues at the `SourceBuffer` (a task that fires an event of that name), in the order
/// returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SbEvent {
    UpdateStart,
    Update,
    UpdateEnd,
    Error,
    Abort,
}

impl SbEvent {
    /// The event's type string, as the page sees it.
    pub fn name(self) -> &'static str {
        match self {
            SbEvent::UpdateStart => "updatestart",
            SbEvent::Update => "update",
            SbEvent::UpdateEnd => "updateend",
            SbEvent::Error => "error",
            SbEvent::Abort => "abort",
        }
    }
}

/// `abort()` must throw `InvalidStateError` with this reason (step 3: the range removal algorithm is
/// running). Nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbortRefused {
    pub reason: &'static str,
}

impl fmt::Display for AbortRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason)
    }
}

impl std::error::Error for AbortRefused {}

/// The pending operation and `updating` of one source buffer, with the events of every transition.
#[derive(Clone, Debug, Default)]
pub struct SourceBufferOps {
    tracker: OpTracker,
}

impl SourceBufferOps {
    pub fn new() -> Self {
        SourceBufferOps::default()
    }

    /// The `updating` attribute.
    pub fn updating(&self) -> bool {
        self.tracker.pending().is_some()
    }

    /// The operation whose completion event the buffer waits for.
    pub fn pending(&self) -> Option<OpId> {
        self.tracker.pending()
    }

    /// Whether the pending operation is a range removal.
    pub fn pending_is_remove(&self) -> bool {
        self.tracker.pending_is_remove()
    }

    /// The backend accepted an append: `updating` becomes true. The DOM has checked `updating` is false
    /// first (`appendBuffer()`'s prepare append); a call while an operation is pending replaces it, as
    /// [`OpTracker::begin`] does.
    #[must_use = "queue the returned event (updatestart)"]
    pub fn begin_append(&mut self, op: OpId) -> SbEvent {
        self.tracker.begin(op);
        SbEvent::UpdateStart
    }

    /// The backend accepted a remove: `updating` becomes true and the range removal algorithm is running
    /// until [`SourceBufferOps::finished`] handles its `RemoveDone`.
    #[must_use = "queue the returned event (updatestart)"]
    pub fn begin_remove(&mut self, op: OpId) -> SbEvent {
        self.tracker.begin_remove(op);
        SbEvent::UpdateStart
    }

    /// Whether `abort()` may run now, without changing anything: step 3, "if the range removal algorithm
    /// is running, throw an InvalidStateError". Call it before the backend's `abort` (which also resets
    /// the parser), so a refused `abort()` has no effect at all.
    pub fn check_abort(&self) -> Result<(), AbortRefused> {
        if self.pending_is_remove() {
            return Err(AbortRefused {
                reason: "the range removal algorithm is running",
            });
        }
        Ok(())
    }

    /// `abort()` steps 3 and 4: refused while a removal is pending; with an append pending it is retired
    /// (its completion event, queued or not, is stale from now on) and `abort` and `updateend` are
    /// returned; with nothing pending nothing happens. The caller makes steps 1 and 2 (removed, not
    /// open) first and runs the parser reset and the append window reset afterwards.
    pub fn abort(&mut self) -> Result<Vec<SbEvent>, AbortRefused> {
        self.check_abort()?;
        Ok(self.abort_pending())
    }

    /// `removeSourceBuffer()` step 2 (also what a removal does to a buffer that is updating, a removal
    /// included): retire the pending operation, and if there was one return `abort` and `updateend`.
    pub fn abort_pending(&mut self) -> Vec<SbEvent> {
        let was_updating = self.updating();
        self.tracker.retire();
        if was_updating {
            vec![SbEvent::Abort, SbEvent::UpdateEnd]
        } else {
            Vec::new()
        }
    }

    /// The buffer leaves its media source silently (the MediaSource's close steps and the last step of
    /// `removeSourceBuffer`, after [`SourceBufferOps::abort_pending`]): retire, `updating` false, no
    /// events.
    pub fn detach(&mut self) {
        self.tracker.retire();
    }

    /// `AppendDone` or `RemoveDone` for `op`: when it is the pending operation, `updating` becomes false
    /// and `update` and `updateend` are returned; a stale event (retired by `abort()` or a removal, or an
    /// older operation) returns nothing and changes nothing.
    pub fn finished(&mut self, op: OpId) -> Vec<SbEvent> {
        if self.tracker.finish(op) {
            vec![SbEvent::Update, SbEvent::UpdateEnd]
        } else {
            Vec::new()
        }
    }

    /// `AppendError` for `op`: like [`SourceBufferOps::finished`] with `error` instead of `update`.
    pub fn failed(&mut self, op: OpId) -> Vec<SbEvent> {
        if self.tracker.finish(op) {
            vec![SbEvent::Error, SbEvent::UpdateEnd]
        } else {
            Vec::new()
        }
    }
}
