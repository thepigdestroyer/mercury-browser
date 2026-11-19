// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The viewer's memory: a byte-bounded, least-recently-used store of rendered tiles, a second
//! small store of low-resolution page placeholders, and a per-page store for text and links.
//!
//! Nothing here reads a clock: "least recently used" is a counter that grows by one at every
//! touch, so the order is deterministic and tests need no sleeps. The stores never decide what
//! is on screen: the caller passes a `pinned` test to [`ByteLru::evict_to_budget`], and an entry
//! it pins is never evicted, even when that leaves the store over its budget (the page the person
//! is reading must stay drawn).
//!
//! Every budget of the viewer is a constant in [`budget`], in one place, so the lead can lower
//! them without reading the logic.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;

use crate::layout::TILE_SIDE;
use crate::types::{Rotation, Tile, TileRequest};

/// The viewer's limits on memory and on what one selection or find may hold, in one place.
pub mod budget {
    /// Rendered tiles of one document, in bytes of pixels (96 MiB).
    pub const TILE_BYTES: usize = 96 * 1024 * 1024;
    /// Low-resolution page placeholders of one document, in bytes of pixels (16 MiB).
    pub const PLACEHOLDER_BYTES: usize = 16 * 1024 * 1024;
    /// The widest a placeholder is, in device pixels.
    pub const PLACEHOLDER_MAX_WIDTH: u32 = 160;
    /// The share of [`TILE_BYTES`] and [`PLACEHOLDER_BYTES`] that the tiles and pages wanted
    /// beyond the screen may take. The rest is room for the tiles the person scrolled past, so
    /// that asking for what the margin needs never evicts what the margin needs.
    pub const PREFETCH_SHARE_PERCENT: usize = 75;
    /// The text of the pages kept for selection, in characters (about 40 MB).
    pub const TEXT_CHARS: usize = 2_000_000;
    /// The links of the pages kept, in links (a page with none counts as one).
    pub const LINK_ENTRIES: usize = 100_000;
    /// The most tile requests remembered as failed (they are not asked for again until the zoom
    /// changes); on overflow the whole list is forgotten.
    pub const FAILED_REQUESTS: usize = 4_096;
    /// The most bytes `selected_text()` returns (1 MiB); a longer selection is cut there.
    pub const SELECTED_TEXT_BYTES: usize = 1024 * 1024;
    /// The most pages, counted from the start of a selection, whose text is read for copying.
    pub const SELECTED_PAGES: u32 = 256;
    /// The most hits a find keeps; later hits are dropped.
    pub const FIND_HITS: usize = 10_000;
}

/// One tile of one page at one zoom: the page, the scale in thousandths of a device pixel per
/// point, the rotation, and the tile's column and row in the page's 512-pixel grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub page: u32,
    pub scale_milli: u32,
    pub rotation: Rotation,
    pub col: u32,
    pub row: u32,
}

impl TileKey {
    /// The key of a request. The viewer only asks for scales that are whole thousandths and for
    /// tiles on the 512-pixel grid, so a request and its key correspond one to one.
    pub fn from_request(request: &TileRequest) -> Self {
        Self {
            page: request.page,
            scale_milli: scale_to_milli(request.scale),
            rotation: request.rotation,
            col: request.x / TILE_SIDE,
            row: request.y / TILE_SIDE,
        }
    }
}

/// A scale in whole thousandths (rounded; out-of-range and NaN values saturate).
pub fn scale_to_milli(scale: f32) -> u32 {
    (scale * 1000.0).round() as u32
}

/// A placeholder is the whole page at a low resolution: one per page and rotation.
pub type PlaceholderKey = (u32, Rotation);

/// The tile requests the model has asked for and has no answer to yet.
#[derive(Debug, Default)]
pub struct PendingTiles {
    requests: HashMap<TileKey, TileRequest>,
}

impl PendingTiles {
    /// Records a request; false when that tile was already pending.
    pub fn insert(&mut self, key: TileKey, request: TileRequest) -> bool {
        self.requests.insert(key, request).is_none()
    }

    pub fn contains(&self, key: &TileKey) -> bool {
        self.requests.contains_key(key)
    }

    /// Forgets a request (its answer came, or it was cancelled).
    pub fn remove(&mut self, key: &TileKey) -> Option<TileRequest> {
        self.requests.remove(key)
    }

    /// Removes and returns the pending requests whose key is not in `wanted`, in a fixed order
    /// (by page, scale, rotation, row and column) so that plans are reproducible.
    pub fn take_unwanted(&mut self, wanted: &HashSet<TileKey>) -> Vec<TileRequest> {
        let mut stale: Vec<(TileKey, TileRequest)> = self
            .requests
            .iter()
            .filter(|(key, _)| !wanted.contains(key))
            .map(|(key, request)| (*key, *request))
            .collect();
        stale.sort_by_key(|(key, _)| {
            (
                key.page,
                key.scale_milli,
                key.rotation.degrees(),
                key.row,
                key.col,
            )
        });
        for (key, _) in &stale {
            self.requests.remove(key);
        }
        stale.into_iter().map(|(_, request)| request).collect()
    }

    pub fn clear(&mut self) {
        self.requests.clear();
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}

/// The tiles the helper could not render: not asked for again until the zoom changes, so a
/// tile that always fails cannot make a request loop. At most [`budget::FAILED_REQUESTS`] are
/// remembered; on overflow the list starts again.
#[derive(Debug, Default)]
pub struct FailedTiles {
    keys: HashSet<TileKey>,
}

impl FailedTiles {
    pub fn remember(&mut self, key: TileKey) {
        if self.keys.len() >= budget::FAILED_REQUESTS {
            self.keys.clear();
        }
        self.keys.insert(key);
    }

    pub fn contains(&self, key: &TileKey) -> bool {
        self.keys.contains(key)
    }

    pub fn clear(&mut self) {
        self.keys.clear();
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

struct Entry {
    tile: Arc<Tile>,
    bytes: usize,
    used: u64,
}

/// A byte-bounded store of tiles, least recently used first out.
pub struct ByteLru<K> {
    budget: usize,
    bytes: usize,
    clock: u64,
    entries: HashMap<K, Entry>,
}

impl<K: Copy + Eq + Hash> ByteLru<K> {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            bytes: 0,
            clock: 0,
            entries: HashMap::new(),
        }
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// The tile, marked as used now.
    pub fn get(&mut self, key: &K) -> Option<Arc<Tile>> {
        let now = self.tick();
        let entry = self.entries.get_mut(key)?;
        entry.used = now;
        Some(Arc::clone(&entry.tile))
    }

    /// The tile without marking it used (for code that only reads, such as drawing).
    pub fn peek(&self, key: &K) -> Option<&Arc<Tile>> {
        self.entries.get(key).map(|entry| &entry.tile)
    }

    /// Marks the tile as used now; false when it is not stored.
    pub fn touch(&mut self, key: &K) -> bool {
        let now = self.tick();
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.used = now;
                true
            }
            None => false,
        }
    }

    pub fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// Stores a tile as the most recently used one, replacing (and returning) an older tile of
    /// the same key. It does not evict: call [`Self::evict_to_budget`] afterwards.
    pub fn insert(&mut self, key: K, tile: Arc<Tile>) -> Option<Arc<Tile>> {
        let used = self.tick();
        let bytes = tile.rgba.len();
        let old = self.entries.insert(key, Entry { tile, bytes, used });
        self.bytes += bytes;
        old.map(|entry| {
            self.bytes -= entry.bytes;
            entry.tile
        })
    }

    pub fn remove(&mut self, key: &K) -> Option<Arc<Tile>> {
        let entry = self.entries.remove(key)?;
        self.bytes -= entry.bytes;
        Some(entry.tile)
    }

    /// Evicts the least recently used tiles that `pinned` does not protect until the store is
    /// within its budget, or nothing evictable is left. Returns the evicted keys, oldest first.
    pub fn evict_to_budget(&mut self, pinned: impl Fn(&K) -> bool) -> Vec<K> {
        if self.bytes <= self.budget {
            return Vec::new();
        }
        let mut candidates: Vec<(u64, K)> = self
            .entries
            .iter()
            .filter(|(key, _)| !pinned(key))
            .map(|(key, entry)| (entry.used, *key))
            .collect();
        candidates.sort_unstable_by_key(|(used, _)| *used);
        let mut evicted = Vec::new();
        for (_, key) in candidates {
            if self.bytes <= self.budget {
                break;
            }
            self.remove(&key);
            evicted.push(key);
        }
        evicted
    }

    /// Drops every tile for which `keep` is false; returns how many went.
    pub fn retain(&mut self, keep: impl Fn(&K) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|key, _| keep(key));
        self.bytes = self.entries.values().map(|entry| entry.bytes).sum();
        before - self.entries.len()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// The bytes of pixels held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The keys in no particular order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.keys()
    }
}

/// A store of one value per page (text, links) bounded by a cost the caller defines (characters
/// or links), least recently used first out.
pub struct PageStore<T> {
    budget: usize,
    cost: usize,
    clock: u64,
    entries: HashMap<u32, (T, usize, u64)>,
}

impl<T> PageStore<T> {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            cost: 0,
            clock: 0,
            entries: HashMap::new(),
        }
    }

    pub fn get(&self, page: u32) -> Option<&T> {
        self.entries.get(&page).map(|(value, _, _)| value)
    }

    pub fn contains(&self, page: u32) -> bool {
        self.entries.contains_key(&page)
    }

    /// Marks the page as used now.
    pub fn touch(&mut self, page: u32) {
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(&page) {
            entry.2 = self.clock;
        }
    }

    /// Stores (or replaces) the value of a page as the most recently used one.
    pub fn insert(&mut self, page: u32, value: T, cost: usize) {
        self.clock += 1;
        if let Some((_, old_cost, _)) = self.entries.insert(page, (value, cost, self.clock)) {
            self.cost -= old_cost;
        }
        self.cost += cost;
    }

    /// Evicts the least recently used pages that `keep` does not protect until the cost is
    /// within the budget or nothing evictable is left; returns the pages that went.
    pub fn evict(&mut self, keep: impl Fn(u32) -> bool) -> Vec<u32> {
        if self.cost <= self.budget {
            return Vec::new();
        }
        let mut candidates: Vec<(u64, u32)> = self
            .entries
            .iter()
            .filter(|(page, _)| !keep(**page))
            .map(|(page, (_, _, used))| (*used, *page))
            .collect();
        candidates.sort_unstable();
        let mut evicted = Vec::new();
        for (_, page) in candidates {
            if self.cost <= self.budget {
                break;
            }
            if let Some((_, cost, _)) = self.entries.remove(&page) {
                self.cost -= cost;
            }
            evicted.push(page);
        }
        evicted
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.cost = 0;
    }

    pub fn cost(&self) -> usize {
        self.cost
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(bytes: usize) -> Arc<Tile> {
        Arc::new(Tile {
            width: 1,
            height: u32::try_from(bytes / 4).expect("small"),
            rgba: vec![255; bytes],
        })
    }

    #[test]
    fn a_key_follows_the_request_it_comes_from() {
        let request = TileRequest {
            page: 7,
            scale: 1.25,
            rotation: Rotation::Deg90,
            x: 1024,
            y: 512,
            width: 100,
            height: 100,
        };
        assert_eq!(
            TileKey::from_request(&request),
            TileKey {
                page: 7,
                scale_milli: 1250,
                rotation: Rotation::Deg90,
                col: 2,
                row: 1,
            }
        );
        assert_eq!(scale_to_milli(0.261), 261);
        assert_eq!(scale_to_milli(f32::NAN), 0);
    }

    #[test]
    fn the_budgets_are_the_ones_of_the_brief() {
        assert_eq!(budget::TILE_BYTES, 96 * 1024 * 1024);
        assert_eq!(budget::PLACEHOLDER_BYTES, 16 * 1024 * 1024);
        assert_eq!(budget::PLACEHOLDER_MAX_WIDTH, 160);
    }

    #[test]
    fn insert_counts_bytes_and_replacing_does_not_leak_them() {
        let mut cache = ByteLru::new(1000);
        cache.insert(1_u32, tile(400));
        cache.insert(2, tile(300));
        assert_eq!((cache.bytes(), cache.len()), (700, 2));
        let old = cache.insert(1, tile(100));
        assert_eq!(old.map(|tile| tile.rgba.len()), Some(400));
        assert_eq!(cache.bytes(), 400);
        assert_eq!(cache.remove(&2).map(|tile| tile.rgba.len()), Some(300));
        assert_eq!(cache.bytes(), 100);
        assert!(cache.remove(&2).is_none());
    }

    #[test]
    fn the_least_recently_used_tile_goes_first() {
        let mut cache = ByteLru::new(1000);
        for key in 1..=4_u32 {
            cache.insert(key, tile(300));
        }
        // 1200 bytes: over by 200. Use 1 so that 2 is now the oldest.
        assert!(cache.touch(&1));
        assert_eq!(cache.evict_to_budget(|_| false), vec![2]);
        assert!(cache.contains(&1) && cache.contains(&3) && cache.contains(&4));
        assert_eq!(cache.bytes(), 900);
    }

    #[test]
    fn peek_does_not_change_the_order_but_get_does() {
        let mut cache = ByteLru::new(600);
        cache.insert(1_u32, tile(300));
        cache.insert(2, tile(300));
        cache.insert(3, tile(300));
        assert!(cache.peek(&1).is_some());
        assert_eq!(cache.evict_to_budget(|_| false), vec![1]);
        cache.insert(4, tile(300));
        assert!(cache.get(&2).is_some());
        assert_eq!(cache.evict_to_budget(|_| false), vec![3]);
    }

    #[test]
    fn a_pinned_tile_is_never_evicted_even_over_budget() {
        let mut cache = ByteLru::new(500);
        for key in 1..=4_u32 {
            cache.insert(key, tile(300));
        }
        // 1 and 2 are the oldest but on screen; 3 goes, then 4 (newest, not pinned).
        let evicted = cache.evict_to_budget(|key| *key <= 2);
        assert_eq!(evicted, vec![3, 4]);
        assert_eq!(cache.bytes(), 600, "the pinned tiles stay, over the budget");
        // Nothing evictable: a second call changes nothing.
        assert!(cache.evict_to_budget(|key| *key <= 2).is_empty());
    }

    #[test]
    fn eviction_stops_as_soon_as_the_budget_is_met() {
        let mut cache = ByteLru::new(1000);
        for key in 1..=5_u32 {
            cache.insert(key, tile(300));
        }
        assert_eq!(cache.evict_to_budget(|_| false), vec![1, 2]);
        assert_eq!(cache.bytes(), 900);
    }

    #[test]
    fn retain_recounts_the_bytes() {
        let mut cache = ByteLru::new(10_000);
        for key in 1..=4_u32 {
            cache.insert(key, tile(100 * key as usize));
        }
        assert_eq!(cache.retain(|key| key % 2 == 0), 2);
        assert_eq!(cache.bytes(), 200 + 400);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.bytes(), 0);
    }

    #[test]
    fn the_page_store_evicts_by_cost_and_keeps_what_is_wanted() {
        let mut store = PageStore::new(10);
        store.insert(1, "a", 4);
        store.insert(2, "b", 4);
        store.insert(3, "c", 4);
        store.touch(1);
        // Over by 2: page 2 is the oldest.
        assert_eq!(store.evict(|_| false), vec![2]);
        assert_eq!(store.cost(), 8);
        store.insert(4, "d", 4);
        // Over again; 1 is wanted, so 3 goes.
        assert_eq!(store.evict(|page| page == 1), vec![3]);
        assert!(store.contains(1) && store.contains(4) && !store.contains(3));
        assert_eq!(store.get(4), Some(&"d"));
        store.insert(4, "e", 1);
        assert_eq!(store.cost(), 5);
    }

    fn request(page: u32, x: u32) -> TileRequest {
        TileRequest {
            page,
            scale: 1.0,
            rotation: Rotation::Deg0,
            x,
            y: 0,
            width: 512,
            height: 512,
        }
    }

    #[test]
    fn a_pending_request_is_recorded_once_and_the_unwanted_come_out_in_a_fixed_order() {
        let mut pending = PendingTiles::default();
        let all = [request(3, 512), request(1, 0), request(3, 0), request(2, 0)];
        for one in &all {
            assert!(pending.insert(TileKey::from_request(one), *one));
        }
        assert!(
            !pending.insert(TileKey::from_request(&all[0]), all[0]),
            "the same tile again"
        );
        assert_eq!(pending.len(), 4);
        let wanted: HashSet<TileKey> = [TileKey::from_request(&all[3])].into_iter().collect();
        let stale = pending.take_unwanted(&wanted);
        // By page, then column: 1, 3 (column 0), 3 (column 1).
        let order: Vec<(u32, u32)> = stale.iter().map(|one| (one.page, one.x)).collect();
        assert_eq!(order, vec![(1, 0), (3, 0), (3, 512)]);
        assert_eq!(pending.len(), 1);
        assert!(pending.contains(&TileKey::from_request(&all[3])));
        assert!(
            pending
                .remove(&TileKey::from_request(&all[3]))
                .is_some_and(|one| one.page == 2)
        );
        assert!(pending.is_empty());
        pending.insert(TileKey::from_request(&all[1]), all[1]);
        pending.clear();
        assert!(pending.is_empty());
    }

    #[test]
    fn failed_tiles_are_remembered_up_to_a_limit_then_forgotten_together() {
        let mut failed = FailedTiles::default();
        let key = |index: u32| TileKey::from_request(&request(index, 0));
        failed.remember(key(0));
        assert!(failed.contains(&key(0)) && !failed.contains(&key(1)));
        for index in 1..budget::FAILED_REQUESTS as u32 {
            failed.remember(key(index));
        }
        assert_eq!(failed.len(), budget::FAILED_REQUESTS);
        // One more than the limit: the list starts again with just this one.
        failed.remember(key(budget::FAILED_REQUESTS as u32));
        assert_eq!(failed.len(), 1);
        assert!(!failed.contains(&key(0)));
        failed.clear();
        assert!(failed.is_empty());
    }
}
