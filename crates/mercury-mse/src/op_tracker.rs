// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Which append or remove a completion event belongs to (lane med-1b, decision D3). Pure state, so it
//! is tested here without Servo; the DOM's `SourceBuffer` owns one.
//!
//! The problem: `AppendDone`, `AppendError` and `RemoveDone` travel through a `GenericCallback` and the
//! script task queue. An event that was already queued when the page calls `abort()` (or removes the
//! buffer) cannot be recalled, and if the page then appends again in the same task, the stale event
//! would end the new append early. Every accepted operation has an [`OpId`] (what `append` and
//! `remove` return); the tracker remembers the one that is pending and answers whether an event is for
//! it.
//!
//! Rules: after [`OpTracker::retire`] or a newer [`OpTracker::begin`], `finish(old)` is `false` for
//! ever (the backend never reuses an `OpId`, so "old" can never become pending again); `finish` of the
//! pending operation is `true` exactly once.

use crate::api::OpId;

/// The pending operation of one source buffer.
#[derive(Clone, Debug, Default)]
pub struct OpTracker {
    pending: Option<OpId>,
    /// Whether the pending operation is a range removal. `false` when nothing is pending.
    remove: bool,
}

impl OpTracker {
    pub fn new() -> Self {
        OpTracker::default()
    }

    /// The backend accepted an append: it is the pending operation (replacing any other).
    pub fn begin(&mut self, op: OpId) {
        self.pending = Some(op);
        self.remove = false;
    }

    /// The backend accepted a remove: it is the pending operation (replacing any other). The
    /// specification's `abort()` must not touch it (see [`OpTracker::pending_is_remove`]).
    pub fn begin_remove(&mut self, op: OpId) {
        self.pending = Some(op);
        self.remove = true;
    }

    /// `abort()`, removal of the buffer or `close()`: nothing is pending, whatever is in flight is stale.
    pub fn retire(&mut self) {
        self.pending = None;
        self.remove = false;
    }

    /// A completion event for `op` arrived. `true` when it completes the pending operation (which is
    /// then no longer pending), `false` when it is stale and must be dropped.
    pub fn finish(&mut self, op: OpId) -> bool {
        if self.pending == Some(op) {
            self.pending = None;
            self.remove = false;
            true
        } else {
            false
        }
    }

    /// The operation that is pending, if any.
    pub fn pending(&self) -> Option<OpId> {
        self.pending
    }

    /// Whether the pending operation is a range removal: the specification's range removal algorithm
    /// "is running" from `remove()` until the event that ends it was handled, whether or not the
    /// backend's worker has already finished the work.
    pub fn pending_is_remove(&self) -> bool {
        self.pending.is_some() && self.remove
    }
}
