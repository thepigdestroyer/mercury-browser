// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The snapshots kept in memory, least recently used first out, under a memory cap.
//!
//! GROUNDWORK STUB (lane tab-snapshots, item 2 owns this file): [`Snapshot`] and the constants are
//! final; [`SnapshotStore`] is to be written to the contracts in its comments.

use std::hash::Hash;
use std::marker::PhantomData;
use std::time::Duration;

use crate::frame::RgbaFrame;

/// The default memory cap: 128 MB (owner decision, 18 Nov 2026), read as 128,000,000 bytes so
/// that the cap can never be above the number he gave. A full-size snapshot (480 by 300, RGBA8)
/// costs 576,000 bytes plus [`ENTRY_OVERHEAD_BYTES`], so the default holds about 221 of them.
pub const DEFAULT_CAP_BYTES: usize = 128_000_000;

/// What an entry costs besides its pixels (the key, the bookkeeping and the allocation's own
/// overhead, rounded up). Counted against the cap so that thousands of tiny snapshots cannot hide
/// below it.
pub const ENTRY_OVERHEAD_BYTES: usize = 256;

/// A tab's picture and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    frame: RgbaFrame,
    captured_at: Duration,
    source_width: u32,
    source_height: u32,
}

impl Snapshot {
    /// `frame` is the downscaled picture; `captured_at` is the time it was taken (the caller's own
    /// clock, see the crate documentation); `source_size` is the size of the frame it was made
    /// from, so that a reader can tell what shape the page had.
    pub fn new(frame: RgbaFrame, captured_at: Duration, source_size: (u32, u32)) -> Self {
        Self {
            frame,
            captured_at,
            source_width: source_size.0,
            source_height: source_size.1,
        }
    }

    pub fn frame(&self) -> &RgbaFrame {
        &self.frame
    }

    pub fn captured_at(&self) -> Duration {
        self.captured_at
    }

    /// The size, in pixels, of the full frame this picture was made from.
    pub fn source_size(&self) -> (u32, u32) {
        (self.source_width, self.source_height)
    }

    /// What this snapshot counts against the cap: its pixels plus [`ENTRY_OVERHEAD_BYTES`].
    pub fn cost_bytes(&self) -> usize {
        self.frame.byte_len() + ENTRY_OVERHEAD_BYTES
    }
}

/// What [`SnapshotStore::insert`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    /// The snapshot is in the store; `evicted` older ones (not the one it replaced) made room.
    Stored { evicted: usize },
    /// The snapshot alone costs more than the whole cap, so it was not stored and nothing was
    /// evicted; an older snapshot of the same key is left where it is.
    TooLarge,
}

/// Snapshots by key, with the total cost never above the cap after a call returns.
///
/// Contract: [`insert`](Self::insert) and [`get`](Self::get) make the key the most recently used;
/// [`peek`](Self::peek) does not. Inserting for a key that is there replaces its snapshot (the
/// old cost is released first). When the total would pass the cap, the least recently used
/// entries other than the new one are removed until it fits. All operations are exact in their
/// accounting: [`used_bytes`](Self::used_bytes) is always the sum of the entries' costs.
#[derive(Debug)]
pub struct SnapshotStore<K> {
    cap_bytes: usize,
    key: PhantomData<K>,
}

impl<K: Copy + Eq + Hash> SnapshotStore<K> {
    pub fn new(cap_bytes: usize) -> Self {
        Self {
            cap_bytes,
            key: PhantomData,
        }
    }

    pub fn cap_bytes(&self) -> usize {
        self.cap_bytes
    }

    pub fn insert(&mut self, _key: K, _snapshot: Snapshot) -> InsertOutcome {
        todo!("tab-snapshots item 2: insert")
    }

    /// The snapshot of `key`, which becomes the most recently used.
    pub fn get(&mut self, _key: K) -> Option<&Snapshot> {
        todo!("tab-snapshots item 2: get")
    }

    /// The snapshot of `key` without changing the order.
    pub fn peek(&self, _key: K) -> Option<&Snapshot> {
        todo!("tab-snapshots item 2: peek")
    }

    /// Removes `key`'s snapshot; whether there was one.
    pub fn remove(&mut self, _key: K) -> bool {
        todo!("tab-snapshots item 2: remove")
    }

    /// Keeps only the keys for which `keep` is true (for example the tabs that are still open).
    /// Returns how many were removed.
    pub fn retain(&mut self, _keep: impl FnMut(&K) -> bool) -> usize {
        todo!("tab-snapshots item 2: retain")
    }

    /// Changes the cap and evicts, least recently used first, until the total fits. Returns how
    /// many were evicted. A cap of 0 empties the store.
    pub fn set_cap_bytes(&mut self, _cap_bytes: usize) -> usize {
        todo!("tab-snapshots item 2: set_cap_bytes")
    }

    pub fn clear(&mut self) {
        todo!("tab-snapshots item 2: clear")
    }

    pub fn len(&self) -> usize {
        todo!("tab-snapshots item 2: len")
    }

    pub fn is_empty(&self) -> bool {
        todo!("tab-snapshots item 2: is_empty")
    }

    /// The sum of the costs of all entries.
    pub fn used_bytes(&self) -> usize {
        todo!("tab-snapshots item 2: used_bytes")
    }

    /// The keys, the most recently used first.
    pub fn keys_most_recent_first(&self) -> Vec<K> {
        todo!("tab-snapshots item 2: keys_most_recent_first")
    }
}

impl<K: Copy + Eq + Hash> Default for SnapshotStore<K> {
    fn default() -> Self {
        Self::new(DEFAULT_CAP_BYTES)
    }
}
