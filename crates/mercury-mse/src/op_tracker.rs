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
//! Skeleton: the signatures are committed by the lead; item 1 (`core`) implements the bodies. Rules the
//! implementation keeps: after [`OpTracker::retire`] or a newer [`OpTracker::begin`], `finish(old)` is
//! `false` for ever; `finish` of the pending operation is `true` exactly once.

use crate::api::OpId;

/// The pending operation of one source buffer.
#[derive(Clone, Debug, Default)]
pub struct OpTracker {
    pending: Option<OpId>,
}

impl OpTracker {
    pub fn new() -> Self {
        OpTracker::default()
    }

    /// The backend accepted an append or a remove: it is the pending operation (replacing any other).
    pub fn begin(&mut self, _op: OpId) {
        todo!("item 1 `core`")
    }

    /// `abort()`, removal of the buffer or `close()`: nothing is pending, whatever is in flight is stale.
    pub fn retire(&mut self) {
        todo!("item 1 `core`")
    }

    /// A completion event for `op` arrived. `true` when it completes the pending operation (which is
    /// then no longer pending), `false` when it is stale and must be dropped.
    pub fn finish(&mut self, _op: OpId) -> bool {
        todo!("item 1 `core`")
    }

    /// The operation that is pending, if any.
    pub fn pending(&self) -> Option<OpId> {
        self.pending
    }
}
