// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The snapshots of a window's tabs: the store and the scheduler together.
//!
//! The scheduler decides when a picture may be taken and the store keeps the pictures under a
//! memory cap; the registry is the one value the app holds, and it keeps the two in step. The step
//! that matters is [`SnapshotRegistry::finish`]: a picture is stored only if the scheduler still
//! wants the request it answers, so the picture of a tab that was closed while it was being
//! shrunk on another thread is thrown away and never enters the store.

use std::hash::Hash;
use std::time::Duration;

use crate::downscale::Limits;
use crate::schedule::{CaptureRequest, CaptureScheduler, Skip};
use crate::store::{DEFAULT_CAP_BYTES, InsertOutcome, Snapshot, SnapshotStore};

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

/// Counters for logs and tests. Every call of [`SnapshotRegistry::finish`] adds one to exactly
/// one of `stored`, `too_large`, `failed` and `discarded`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Snapshots in the store now.
    pub entries: usize,
    /// What they cost together (pixels and overhead), never above `cap_bytes`.
    pub used_bytes: usize,
    pub cap_bytes: usize,
    /// Snapshots stored since the start.
    pub stored: u64,
    /// Snapshots that were made but cost more than the whole cap, so were not stored.
    pub too_large: u64,
    /// Captures that failed.
    pub failed: u64,
    /// Results thrown away because their request was no longer wanted.
    pub discarded: u64,
    /// Snapshots evicted to stay under the cap, whether by a new snapshot or by a lower cap.
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
    /// The counters; the live fields (`entries`, `used_bytes`, `cap_bytes`) are filled in by
    /// [`stats`](Self::stats) from the store.
    counters: Stats,
}

impl<K: Copy + Eq + Hash> SnapshotRegistry<K> {
    pub fn new(cap_bytes: usize, limits: Limits) -> Self {
        Self {
            limits,
            store: SnapshotStore::new(cap_bytes),
            scheduler: CaptureScheduler::new(),
            counters: Stats::default(),
        }
    }

    /// The size every snapshot must fit.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The app showed a frame of `key` on a surface of `surface` pixels; see
    /// [`CaptureScheduler::frame_presented`]. Runs on every presented frame.
    pub fn frame_presented(
        &mut self,
        key: K,
        surface: (u32, u32),
        now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        self.scheduler.frame_presented(key, surface, now)
    }

    /// The app is about to let another tab paint; see [`CaptureScheduler::tab_leaving`].
    pub fn tab_leaving(
        &mut self,
        key: K,
        surface: (u32, u32),
        now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        self.scheduler.tab_leaving(key, surface, now)
    }

    /// A picture is wanted now; see [`CaptureScheduler::capture_now`].
    pub fn capture_now(
        &mut self,
        key: K,
        surface: (u32, u32),
        now: Duration,
    ) -> Result<CaptureRequest<K>, Skip> {
        self.scheduler.capture_now(key, surface, now)
    }

    /// Ends `request` with its result: `Some(snapshot)` if the picture was made, `None` if the
    /// capture failed. A snapshot reaches the store only if the scheduler still wants the request
    /// (the tab was not forgotten, the request did not time out, the rendering context was not
    /// made again); otherwise it is dropped and the outcome is [`FinishOutcome::Discarded`].
    pub fn finish(
        &mut self,
        request: CaptureRequest<K>,
        snapshot: Option<Snapshot>,
        now: Duration,
    ) -> FinishOutcome {
        let Some(snapshot) = snapshot else {
            if self.scheduler.capture_failed(&request, now) {
                self.counters.failed += 1;
                return FinishOutcome::Failed;
            }
            self.counters.discarded += 1;
            return FinishOutcome::Discarded;
        };
        // Ask first: a picture the scheduler no longer wants must not touch the store.
        if !self.scheduler.capture_succeeded(&request, now) {
            self.counters.discarded += 1;
            return FinishOutcome::Discarded;
        }
        match self.store.insert(request.key(), snapshot) {
            InsertOutcome::Stored { evicted } => {
                self.counters.stored += 1;
                self.counters.evicted += evicted as u64;
                FinishOutcome::Stored { evicted }
            }
            InsertOutcome::TooLarge => {
                self.counters.too_large += 1;
                FinishOutcome::TooLarge
            }
        }
    }

    /// A tab closed: its snapshot goes, its state goes, a capture in flight for it is discarded.
    pub fn forget(&mut self, key: K) {
        self.store.remove(key);
        self.scheduler.forget(key);
    }

    /// The rendering context was torn down and made again: see the scheduler. Stored snapshots
    /// stay (they are copies in memory, not tied to the context).
    pub fn rendering_changed(&mut self) {
        self.scheduler.rendering_changed();
    }

    /// The tab's snapshot, which becomes the most recently used.
    pub fn get(&mut self, key: K) -> Option<&Snapshot> {
        self.store.get(key)
    }

    /// The tab's snapshot without changing the order.
    pub fn peek(&self, key: K) -> Option<&Snapshot> {
        self.store.peek(key)
    }

    /// Changes the memory cap (evicting at once if it is lower); returns how many were evicted.
    pub fn set_cap_bytes(&mut self, cap_bytes: usize) -> usize {
        let evicted = self.store.set_cap_bytes(cap_bytes);
        self.counters.evicted += evicted as u64;
        evicted
    }

    pub fn stats(&self) -> Stats {
        Stats {
            entries: self.store.len(),
            used_bytes: self.store.used_bytes(),
            cap_bytes: self.store.cap_bytes(),
            ..self.counters
        }
    }
}

impl<K: Copy + Eq + Hash> Default for SnapshotRegistry<K> {
    fn default() -> Self {
        Self::new(DEFAULT_CAP_BYTES, Limits::DEFAULT)
    }
}
