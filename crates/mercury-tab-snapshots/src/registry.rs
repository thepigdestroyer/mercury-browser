// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The snapshots of a window's tabs: the store and the scheduler together.
//!
//! GROUNDWORK STUB (lane tab-snapshots, item 4 owns this file, after items 2 and 3 have landed): the data
//! types are final; [`SnapshotRegistry`] is to be written to the contracts in its comments.

use std::hash::Hash;
use std::time::Duration;

use crate::downscale::Limits;
use crate::schedule::{CaptureRequest, CaptureScheduler, Skip};
use crate::store::{DEFAULT_CAP_BYTES, Snapshot, SnapshotStore};

/// What [`SnapshotRegistry::finish`] did with a capture's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishOutcome {
    /// The snapshot is stored; `evicted` older ones made room.
    Stored { evicted: usize },
    /// The snapshot is larger than the whole cap and was not stored.
    TooLarge,
    /// The request was no longer wanted (the tab closed, or the request timed out); the result
    /// was thrown away.
    Discarded,
    /// The capture failed (`None` was passed); nothing is stored.
    Failed,
}

/// Counters for logs and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub entries: usize,
    pub used_bytes: usize,
    pub cap_bytes: usize,
    /// Snapshots stored since the start.
    pub stored: u64,
    /// Captures that failed.
    pub failed: u64,
    /// Results thrown away because their request was no longer wanted.
    pub discarded: u64,
    /// Snapshots evicted to stay under the cap.
    pub evicted: u64,
}

/// The one value the app holds per window.
///
/// Contract: [`frame_presented`](Self::frame_presented), [`tab_leaving`](Self::tab_leaving) and
/// [`capture_now`](Self::capture_now) are the scheduler's, unchanged. [`finish`](Self::finish)
/// ends a request: with `Some(snapshot)` it asks the scheduler (`capture_succeeded`) whether the
/// request is still wanted, and only then stores the snapshot (a closed tab's picture must never
/// enter the store); with `None` it records the failure (`capture_failed`).
/// [`forget`](Self::forget) removes the tab from both. [`get`](Self::get) and
/// [`peek`](Self::peek) are the store's. The caller shrinks the frame itself (on a worker thread)
/// with [`limits`](Self::limits).
#[derive(Debug)]
pub struct SnapshotRegistry<K> {
    limits: Limits,
    store: SnapshotStore<K>,
    scheduler: CaptureScheduler<K>,
    stats: Stats,
}

impl<K: Copy + Eq + Hash> SnapshotRegistry<K> {
    pub fn new(cap_bytes: usize, limits: Limits) -> Self {
        Self {
            limits,
            store: SnapshotStore::new(cap_bytes),
            scheduler: CaptureScheduler::new(),
            stats: Stats::default(),
        }
    }

    /// The size every snapshot must fit.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn frame_presented(
        &mut self,
        _key: K,
        _surface: (u32, u32),
        _now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        let _ = &self.scheduler;
        todo!("tab-snapshots item 4: SnapshotRegistry::frame_presented")
    }

    pub fn tab_leaving(
        &mut self,
        _key: K,
        _surface: (u32, u32),
        _now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        todo!("tab-snapshots item 4: SnapshotRegistry::tab_leaving")
    }

    pub fn capture_now(
        &mut self,
        _key: K,
        _surface: (u32, u32),
        _now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        todo!("tab-snapshots item 4: SnapshotRegistry::capture_now")
    }

    pub fn finish(
        &mut self,
        _request: CaptureRequest<K>,
        _snapshot: Option<Snapshot>,
        _now: Duration,
    ) -> FinishOutcome {
        todo!("tab-snapshots item 4: SnapshotRegistry::finish")
    }

    /// A tab closed: its snapshot goes, its state goes, a capture in flight for it is discarded.
    pub fn forget(&mut self, _key: K) {
        todo!("tab-snapshots item 4: SnapshotRegistry::forget")
    }

    /// The rendering context was torn down and made again: see the scheduler. Stored snapshots
    /// stay (they are copies in memory, not tied to the context).
    pub fn rendering_changed(&mut self) {
        todo!("tab-snapshots item 4: SnapshotRegistry::rendering_changed")
    }

    /// The tab's snapshot, which becomes the most recently used.
    pub fn get(&mut self, _key: K) -> Option<&Snapshot> {
        todo!("tab-snapshots item 4: SnapshotRegistry::get")
    }

    /// The tab's snapshot without changing the order.
    pub fn peek(&self, _key: K) -> Option<&Snapshot> {
        todo!("tab-snapshots item 4: SnapshotRegistry::peek")
    }

    /// Changes the memory cap (evicting at once if it is lower); returns how many were evicted.
    pub fn set_cap_bytes(&mut self, _cap_bytes: usize) -> usize {
        let _ = &self.store;
        todo!("tab-snapshots item 4: SnapshotRegistry::set_cap_bytes")
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }
}

impl<K: Copy + Eq + Hash> Default for SnapshotRegistry<K> {
    fn default() -> Self {
        Self::new(DEFAULT_CAP_BYTES, Limits::DEFAULT)
    }
}
