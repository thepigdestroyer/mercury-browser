// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The snapshots kept in memory, least recently used first out, under a memory cap.
//!
//! The cap counts bytes, not entries (a snapshot is its pixels plus [`ENTRY_OVERHEAD_BYTES`]), so
//! the `lru` crate, which counts entries, does not fit. There are at most a few hundred entries
//! (one per open tab), so the store is a `HashMap` with a "last used" tick on every entry. A tick
//! comes from a counter that goes up by one on every use, so the ticks are all different and the
//! smallest one is the least recently used entry. Eviction scans for it, which costs one pass over
//! a few hundred entries and only when something has to go. That is simple, exact, and needs no
//! linked list.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Duration;

use crate::frame::RgbaFrame;

/// The default memory cap: 128 MB (owner decision, 18 Nov 2026), read as 128,000,000 bytes so
/// that the cap can never be above the number he gave. A full-size snapshot (480 by 300, RGBA8)
/// costs 576,000 bytes plus [`ENTRY_OVERHEAD_BYTES`], so the default holds 222 of them.
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

/// One stored snapshot and when it was last used.
#[derive(Debug)]
struct Entry {
    snapshot: Snapshot,
    /// The value of the store's tick counter at the last `insert` or `get` of this entry.
    last_used: u64,
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
    /// The sum of `snapshot.cost_bytes()` over `entries`, kept exact by every method that adds or
    /// removes an entry.
    used_bytes: usize,
    entries: HashMap<K, Entry>,
    /// The tick the next use will get. A `u64` that goes up by one per use cannot wrap in
    /// practice (centuries at a billion uses a second), so the order is never confused.
    next_tick: u64,
}

impl<K: Copy + Eq + Hash> SnapshotStore<K> {
    pub fn new(cap_bytes: usize) -> Self {
        Self {
            cap_bytes,
            used_bytes: 0,
            entries: HashMap::new(),
            next_tick: 0,
        }
    }

    pub fn cap_bytes(&self) -> usize {
        self.cap_bytes
    }

    /// Stores `snapshot` for `key` as the most recently used, replacing the key's older snapshot
    /// and evicting the least recently used others if the total would pass the cap.
    pub fn insert(&mut self, key: K, snapshot: Snapshot) -> InsertOutcome {
        let cost = snapshot.cost_bytes();
        if cost > self.cap_bytes {
            // Nothing changes: not even an older snapshot of this key is touched.
            return InsertOutcome::TooLarge;
        }
        // The old snapshot of this key (if any) is released first, so that it is neither counted
        // twice nor evicted as if it were another tab's. The new entry is not in the map yet, so
        // the eviction below can never remove it.
        self.remove(key);
        // `cost <= cap_bytes` here, so the subtraction does not wrap, and `used + cost <= cap`
        // is the same as `used <= cap - cost`.
        let evicted = self.evict_until_used_is_at_most(self.cap_bytes - cost);
        let last_used = self.take_tick();
        self.used_bytes += cost;
        self.entries.insert(
            key,
            Entry {
                snapshot,
                last_used,
            },
        );
        InsertOutcome::Stored { evicted }
    }

    /// The snapshot of `key`, which becomes the most recently used.
    pub fn get(&mut self, key: K) -> Option<&Snapshot> {
        // `entries` and `next_tick` are separate fields, so both can be used at once.
        let entry = self.entries.get_mut(&key)?;
        entry.last_used = self.next_tick;
        self.next_tick += 1;
        Some(&entry.snapshot)
    }

    /// The snapshot of `key` without changing the order.
    pub fn peek(&self, key: K) -> Option<&Snapshot> {
        self.entries.get(&key).map(|entry| &entry.snapshot)
    }

    /// Removes `key`'s snapshot; whether there was one.
    pub fn remove(&mut self, key: K) -> bool {
        match self.entries.remove(&key) {
            Some(entry) => {
                self.used_bytes -= entry.snapshot.cost_bytes();
                true
            }
            None => false,
        }
    }

    /// Keeps only the keys for which `keep` is true (for example the tabs that are still open).
    /// Returns how many were removed.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) -> usize {
        let before = self.entries.len();
        let mut freed = 0;
        self.entries.retain(|key, entry| {
            let kept = keep(key);
            if !kept {
                freed += entry.snapshot.cost_bytes();
            }
            kept
        });
        self.used_bytes -= freed;
        before - self.entries.len()
    }

    /// Changes the cap and evicts, least recently used first, until the total fits. Returns how
    /// many were evicted. A cap of 0 empties the store.
    pub fn set_cap_bytes(&mut self, cap_bytes: usize) -> usize {
        self.cap_bytes = cap_bytes;
        self.evict_until_used_is_at_most(cap_bytes)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.used_bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The sum of the costs of all entries.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// The keys, the most recently used first.
    pub fn keys_most_recent_first(&self) -> Vec<K> {
        let mut by_use: Vec<(u64, K)> = self
            .entries
            .iter()
            .map(|(key, entry)| (entry.last_used, *key))
            .collect();
        // The ticks are all different, so the order is total and an unstable sort is exact.
        by_use.sort_unstable_by_key(|(tick, _)| std::cmp::Reverse(*tick));
        by_use.into_iter().map(|(_, key)| key).collect()
    }

    /// A new tick, higher than every one given before.
    fn take_tick(&mut self) -> u64 {
        let tick = self.next_tick;
        self.next_tick += 1;
        tick
    }

    /// Removes the entry with the smallest tick; whether there was one.
    fn evict_least_recently_used(&mut self) -> bool {
        let least_recently_used = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| *key);
        match least_recently_used {
            Some(key) => self.remove(key),
            None => false,
        }
    }

    /// Evicts, least recently used first, until the total is at most `limit`. Returns how many
    /// went. An empty store has a total of 0, so this always ends.
    fn evict_until_used_is_at_most(&mut self, limit: usize) -> usize {
        let mut evicted = 0;
        while self.used_bytes > limit && self.evict_least_recently_used() {
            evicted += 1;
        }
        evicted
    }
}

impl<K: Copy + Eq + Hash> Default for SnapshotStore<K> {
    fn default() -> Self {
        Self::new(DEFAULT_CAP_BYTES)
    }
}
