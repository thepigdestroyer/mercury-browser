// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Tests of the snapshot store: a map under a byte cap that drops the least recently used first.

use std::time::Duration;

use mercury_tab_snapshots::{
    DEFAULT_CAP_BYTES, ENTRY_OVERHEAD_BYTES, InsertOutcome, RgbaFrame, Snapshot, SnapshotStore,
};

/// What a 1 by 1 snapshot costs against the cap: 4 bytes of pixels plus the overhead.
const SMALL: usize = ENTRY_OVERHEAD_BYTES + 4;

/// A snapshot that costs exactly `cost` bytes (a 1 pixel wide strip, so the cost can be any
/// number of at least `SMALL` that is a multiple of 4). `id` is both the capture time in seconds
/// and the fill byte, so a test can tell which snapshot a key holds.
fn snapshot(cost: usize, id: u64) -> Snapshot {
    assert!(
        cost >= SMALL && (cost - ENTRY_OVERHEAD_BYTES).is_multiple_of(4),
        "a test snapshot costs {ENTRY_OVERHEAD_BYTES} plus a multiple of 4, not {cost}"
    );
    let height = u32::try_from((cost - ENTRY_OVERHEAD_BYTES) / 4).expect("a small strip");
    let frame = RgbaFrame::new(1, height, vec![id as u8; height as usize * 4])
        .expect("a 1 pixel wide strip");
    let snapshot = Snapshot::new(frame, Duration::from_secs(id), (1440, 900));
    assert_eq!(snapshot.cost_bytes(), cost);
    snapshot
}

/// Which snapshot `key` holds, by its id; does not touch the order.
fn id_of<K: Copy + Eq + std::hash::Hash>(store: &SnapshotStore<K>, key: K) -> Option<u64> {
    store
        .peek(key)
        .map(|snapshot| snapshot.captured_at().as_secs())
}

fn stored(evicted: usize) -> InsertOutcome {
    InsertOutcome::Stored { evicted }
}

#[test]
fn a_new_store_is_empty() {
    let store = SnapshotStore::<u32>::new(1000);

    assert!(store.is_empty());
    assert_eq!(store.len(), 0);
    assert_eq!(store.used_bytes(), 0);
    assert_eq!(store.cap_bytes(), 1000);
    assert!(store.keys_most_recent_first().is_empty());
    assert_eq!(store.peek(1), None);
}

#[test]
fn an_empty_store_has_nothing_to_get_remove_retain_or_evict() {
    let mut store = SnapshotStore::<u32>::new(1000);

    assert_eq!(store.get(1), None);
    assert!(!store.remove(1));
    assert_eq!(store.retain(|_| false), 0);
    assert_eq!(store.set_cap_bytes(500), 0);
    store.clear();

    assert!(store.is_empty());
    assert_eq!(store.used_bytes(), 0);
    assert_eq!(store.cap_bytes(), 500);
}

#[test]
fn the_default_store_has_the_default_cap() {
    assert_eq!(DEFAULT_CAP_BYTES, 128_000_000);
    assert_eq!(ENTRY_OVERHEAD_BYTES, 256);
    assert_eq!(
        SnapshotStore::<u32>::default().cap_bytes(),
        DEFAULT_CAP_BYTES
    );
}

#[test]
fn an_inserted_snapshot_can_be_peeked_and_got() {
    let mut store = SnapshotStore::new(10_000);

    assert_eq!(store.insert(7u32, snapshot(1000, 1)), stored(0));

    assert_eq!(store.len(), 1);
    assert!(!store.is_empty());
    assert_eq!(store.used_bytes(), 1000);
    assert_eq!(store.peek(7), Some(&snapshot(1000, 1)));
    assert_eq!(store.get(7), Some(&snapshot(1000, 1)));
    assert_eq!(store.peek(8), None);
    assert_eq!(store.get(8), None);
    assert_eq!(store.keys_most_recent_first(), vec![7]);
}

#[test]
fn an_entry_costs_its_pixels_plus_the_overhead() {
    let mut store = SnapshotStore::new(10_000);

    // 3 by 2 pixels: 24 bytes of pixels.
    let frame = RgbaFrame::new(3, 2, vec![9; 24]).expect("3x2");
    let snapshot = Snapshot::new(frame, Duration::ZERO, (30, 20));
    assert_eq!(store.insert(1u32, snapshot), stored(0));

    assert_eq!(store.used_bytes(), 24 + ENTRY_OVERHEAD_BYTES);
}

#[test]
fn replacing_a_key_releases_the_old_cost_exactly() {
    let mut store = SnapshotStore::new(10_000);
    store.insert(1u32, snapshot(1000, 1));
    store.insert(2, snapshot(2000, 2));
    assert_eq!(store.used_bytes(), 3000);

    // A smaller snapshot for key 1: only the new cost counts afterwards.
    assert_eq!(store.insert(1, snapshot(500, 3)), stored(0));
    assert_eq!(store.used_bytes(), 2500);
    assert_eq!(store.len(), 2);
    assert_eq!(id_of(&store, 1), Some(3));

    // A larger one.
    assert_eq!(store.insert(1, snapshot(4000, 4)), stored(0));
    assert_eq!(store.used_bytes(), 6000);
    assert_eq!(store.len(), 2);
    assert_eq!(id_of(&store, 1), Some(4));
}

#[test]
fn replacing_a_key_makes_it_the_most_recently_used() {
    let mut store = SnapshotStore::new(10_000);
    store.insert(1u32, snapshot(1000, 1));
    store.insert(2, snapshot(1000, 2));
    store.insert(3, snapshot(1000, 3));

    store.insert(1, snapshot(1000, 4));

    assert_eq!(store.keys_most_recent_first(), vec![1, 3, 2]);
}

#[test]
fn a_replacement_never_evicts_the_snapshot_it_replaces_or_counts_it_as_evicted() {
    // The cap is 2000. Key 1 holds 1000. A replacement of 1800 fits only once the old 1000 has
    // been released first: nothing else is evicted and the replaced one is not counted.
    let mut store = SnapshotStore::new(2000);
    store.insert(1u32, snapshot(1000, 1));

    assert_eq!(store.insert(1, snapshot(1800, 2)), stored(0));
    assert_eq!(store.used_bytes(), 1800);
    assert_eq!(store.keys_most_recent_first(), vec![1]);
    assert_eq!(id_of(&store, 1), Some(2));
}

#[test]
fn a_replacement_that_needs_room_evicts_the_others_least_recently_used_first() {
    let mut store = SnapshotStore::new(2000);
    store.insert(1u32, snapshot(800, 1));
    store.insert(2, snapshot(800, 2));
    assert_eq!(store.used_bytes(), 1600);

    // Replacing key 1 (800 released, 800 left) with 1500 needs key 2 to go.
    assert_eq!(store.insert(1, snapshot(1500, 3)), stored(1));

    assert_eq!(store.keys_most_recent_first(), vec![1]);
    assert_eq!(store.used_bytes(), 1500);
    assert_eq!(id_of(&store, 1), Some(3));
}

#[test]
fn the_least_recently_used_snapshot_goes_first() {
    // Room for exactly three small snapshots.
    let mut store = SnapshotStore::new(3 * SMALL);
    store.insert('a', snapshot(SMALL, 1));
    store.insert('b', snapshot(SMALL, 2));
    store.insert('c', snapshot(SMALL, 3));
    assert_eq!(store.used_bytes(), 3 * SMALL);

    // Using `a` makes `b` the oldest.
    assert!(store.get('a').is_some());
    assert_eq!(store.insert('d', snapshot(SMALL, 4)), stored(1));

    assert_eq!(store.peek('b'), None);
    assert!(store.peek('a').is_some());
    assert_eq!(store.keys_most_recent_first(), vec!['d', 'a', 'c']);
    assert_eq!(store.used_bytes(), 3 * SMALL);
}

#[test]
fn without_a_get_the_first_inserted_goes_first() {
    let mut store = SnapshotStore::new(3 * SMALL);
    store.insert('a', snapshot(SMALL, 1));
    store.insert('b', snapshot(SMALL, 2));
    store.insert('c', snapshot(SMALL, 3));

    assert_eq!(store.insert('d', snapshot(SMALL, 4)), stored(1));

    assert_eq!(store.keys_most_recent_first(), vec!['d', 'c', 'b']);
}

#[test]
fn peek_does_not_refresh_the_order() {
    let mut store = SnapshotStore::new(3 * SMALL);
    store.insert('a', snapshot(SMALL, 1));
    store.insert('b', snapshot(SMALL, 2));
    store.insert('c', snapshot(SMALL, 3));

    assert!(store.peek('a').is_some());
    assert_eq!(store.keys_most_recent_first(), vec!['c', 'b', 'a']);

    // `a` is still the oldest, so it is the one that goes.
    assert_eq!(store.insert('d', snapshot(SMALL, 4)), stored(1));
    assert_eq!(store.peek('a'), None);
    assert_eq!(store.keys_most_recent_first(), vec!['d', 'c', 'b']);
}

#[test]
fn a_get_of_a_missing_key_changes_nothing() {
    let mut store = SnapshotStore::new(10_000);
    store.insert(1u32, snapshot(1000, 1));
    store.insert(2, snapshot(1000, 2));

    assert_eq!(store.get(3), None);

    assert_eq!(store.keys_most_recent_first(), vec![2, 1]);
    assert_eq!(store.used_bytes(), 2000);
}

#[test]
fn one_big_snapshot_evicts_several_small_ones_and_says_how_many() {
    // Five small snapshots fill the cap of 1300 exactly.
    let mut store = SnapshotStore::new(5 * SMALL);
    for (id, key) in ['a', 'b', 'c', 'd', 'e'].into_iter().enumerate() {
        assert_eq!(store.insert(key, snapshot(SMALL, id as u64)), stored(0));
    }
    assert_eq!(store.used_bytes(), 1300);

    // 800 more needs the total before it to be at most 500: a, b, c and d go (1300 to 260),
    // and three would not be enough (520 is above 500).
    assert_eq!(store.insert('z', snapshot(800, 9)), stored(4));

    assert_eq!(store.keys_most_recent_first(), vec!['z', 'e']);
    assert_eq!(store.used_bytes(), 800 + SMALL);
    assert!(store.used_bytes() <= store.cap_bytes());
}

#[test]
fn a_snapshot_that_exactly_fills_the_cap_is_stored() {
    let mut store = SnapshotStore::new(1000);

    assert_eq!(store.insert(1u32, snapshot(1000, 1)), stored(0));

    assert_eq!(store.used_bytes(), 1000);
    assert_eq!(store.len(), 1);
}

#[test]
fn two_snapshots_that_together_exactly_fill_the_cap_both_stay() {
    let mut store = SnapshotStore::new(2 * SMALL);
    store.insert(1u32, snapshot(SMALL, 1));

    assert_eq!(store.insert(2, snapshot(SMALL, 2)), stored(0));

    assert_eq!(store.used_bytes(), 2 * SMALL);
    assert_eq!(store.keys_most_recent_first(), vec![2, 1]);
}

#[test]
fn one_byte_over_the_cap_evicts_the_oldest() {
    // A cost is always a multiple of 4, so make the cap 2 * SMALL - 1, which holds one.
    let mut store = SnapshotStore::new(2 * SMALL - 1);
    store.insert(1u32, snapshot(SMALL, 1));

    assert_eq!(store.insert(2, snapshot(SMALL, 2)), stored(1));

    assert_eq!(store.keys_most_recent_first(), vec![2]);
}

#[test]
fn a_snapshot_above_the_whole_cap_is_too_large_and_evicts_nothing() {
    let mut store = SnapshotStore::new(1000);
    store.insert(1u32, snapshot(400, 1));
    store.insert(2, snapshot(400, 2));

    assert_eq!(store.insert(3, snapshot(1004, 3)), InsertOutcome::TooLarge);

    assert_eq!(store.keys_most_recent_first(), vec![2, 1]);
    assert_eq!(store.used_bytes(), 800);
    assert_eq!(store.peek(3), None);
}

#[test]
fn a_too_large_snapshot_leaves_the_older_one_of_the_same_key() {
    let mut store = SnapshotStore::new(1000);
    store.insert(1u32, snapshot(400, 1));
    store.insert(2, snapshot(400, 2));

    assert_eq!(store.insert(1, snapshot(2000, 3)), InsertOutcome::TooLarge);

    // The old picture of key 1 is still there, unchanged, and still the oldest.
    assert_eq!(id_of(&store, 1), Some(1));
    assert_eq!(store.used_bytes(), 800);
    assert_eq!(store.keys_most_recent_first(), vec![2, 1]);
    assert_eq!(store.len(), 2);
}

#[test]
fn every_snapshot_is_too_large_for_a_cap_of_zero() {
    let mut store = SnapshotStore::new(0);

    assert_eq!(
        store.insert(1u32, snapshot(SMALL, 1)),
        InsertOutcome::TooLarge
    );

    assert!(store.is_empty());
    assert_eq!(store.used_bytes(), 0);
}

#[test]
fn lowering_the_cap_evicts_the_least_recently_used_first_and_counts_them() {
    let mut store = SnapshotStore::new(2000);
    store.insert(1u32, snapshot(500, 1));
    store.insert(2, snapshot(500, 2));
    store.insert(3, snapshot(500, 3));
    // Order, most recent first: 1, 3, 2.
    assert!(store.get(1).is_some());
    assert_eq!(store.keys_most_recent_first(), vec![1, 3, 2]);

    // 1500 used; 1000 holds two, so only key 2 goes.
    assert_eq!(store.set_cap_bytes(1000), 1);
    assert_eq!(store.cap_bytes(), 1000);
    assert_eq!(store.keys_most_recent_first(), vec![1, 3]);
    assert_eq!(store.used_bytes(), 1000);

    // 600 holds one: key 3 goes, the one used last stays.
    assert_eq!(store.set_cap_bytes(600), 1);
    assert_eq!(store.keys_most_recent_first(), vec![1]);
    assert_eq!(store.used_bytes(), 500);

    // A cap that is still enough evicts nothing.
    assert_eq!(store.set_cap_bytes(500), 0);
    assert_eq!(store.keys_most_recent_first(), vec![1]);
}

#[test]
fn raising_the_cap_evicts_nothing_and_makes_room() {
    let mut store = SnapshotStore::new(1000);
    store.insert(1u32, snapshot(500, 1));
    store.insert(2, snapshot(500, 2));

    assert_eq!(store.set_cap_bytes(5000), 0);
    assert_eq!(store.cap_bytes(), 5000);
    assert_eq!(store.len(), 2);

    assert_eq!(store.insert(3, snapshot(500, 3)), stored(0));
    assert_eq!(store.len(), 3);
    assert_eq!(store.used_bytes(), 1500);
}

#[test]
fn a_cap_of_zero_empties_the_store_and_refuses_new_snapshots() {
    let mut store = SnapshotStore::new(5000);
    store.insert(1u32, snapshot(500, 1));
    store.insert(2, snapshot(500, 2));
    store.insert(3, snapshot(500, 3));

    assert_eq!(store.set_cap_bytes(0), 3);

    assert!(store.is_empty());
    assert_eq!(store.used_bytes(), 0);
    assert_eq!(store.cap_bytes(), 0);
    assert_eq!(store.insert(4, snapshot(SMALL, 4)), InsertOutcome::TooLarge);

    // And raising it again works as new.
    assert_eq!(store.set_cap_bytes(1000), 0);
    assert_eq!(store.insert(4, snapshot(SMALL, 4)), stored(0));
}

#[test]
fn retain_removes_the_closed_tabs_and_updates_the_used_bytes() {
    let mut store = SnapshotStore::new(10_000);
    store.insert(1u32, snapshot(1000, 1));
    store.insert(2, snapshot(2000, 2));
    store.insert(3, snapshot(3000, 3));
    store.insert(4, snapshot(400, 4));
    assert_eq!(store.used_bytes(), 6400);

    // The tabs 2 and 4 are still open.
    let mut asked = Vec::new();
    let removed = store.retain(|key| {
        asked.push(*key);
        *key == 2 || *key == 4
    });

    assert_eq!(removed, 2);
    asked.sort_unstable();
    assert_eq!(
        asked,
        vec![1, 2, 3, 4],
        "the predicate is asked once per key"
    );
    assert_eq!(store.len(), 2);
    assert_eq!(store.used_bytes(), 2400);
    // The kept ones keep their order.
    assert_eq!(store.keys_most_recent_first(), vec![4, 2]);
    assert_eq!(store.peek(1), None);
    assert_eq!(store.peek(3), None);
}

#[test]
fn retain_that_keeps_everything_or_nothing() {
    let mut store = SnapshotStore::new(10_000);
    store.insert(1u32, snapshot(1000, 1));
    store.insert(2, snapshot(1000, 2));

    assert_eq!(store.retain(|_| true), 0);
    assert_eq!(store.used_bytes(), 2000);
    assert_eq!(store.keys_most_recent_first(), vec![2, 1]);

    assert_eq!(store.retain(|_| false), 2);
    assert!(store.is_empty());
    assert_eq!(store.used_bytes(), 0);
}

#[test]
fn remove_drops_one_snapshot_and_its_cost() {
    let mut store = SnapshotStore::new(10_000);
    store.insert(1u32, snapshot(1000, 1));
    store.insert(2, snapshot(2000, 2));
    store.insert(3, snapshot(3000, 3));

    assert!(store.remove(2));

    assert_eq!(store.used_bytes(), 4000);
    assert_eq!(store.len(), 2);
    assert_eq!(store.keys_most_recent_first(), vec![3, 1]);
    assert!(!store.remove(2), "a second remove finds nothing");
    assert_eq!(store.used_bytes(), 4000);
}

#[test]
fn clear_empties_the_store_and_keeps_the_cap() {
    let mut store = SnapshotStore::new(10_000);
    store.insert(1u32, snapshot(1000, 1));
    store.insert(2, snapshot(2000, 2));

    store.clear();

    assert!(store.is_empty());
    assert_eq!(store.len(), 0);
    assert_eq!(store.used_bytes(), 0);
    assert_eq!(store.cap_bytes(), 10_000);
    assert!(store.keys_most_recent_first().is_empty());
    assert_eq!(store.insert(1, snapshot(1000, 3)), stored(0));
    assert_eq!(store.used_bytes(), 1000);
}

#[test]
fn the_order_follows_a_mix_of_operations() {
    let mut store = SnapshotStore::new(100_000);
    store.insert(1u32, snapshot(SMALL, 1));
    store.insert(2, snapshot(SMALL, 2));
    store.insert(3, snapshot(SMALL, 3));
    assert_eq!(store.keys_most_recent_first(), vec![3, 2, 1]);

    assert!(store.get(2).is_some());
    assert_eq!(store.keys_most_recent_first(), vec![2, 3, 1]);

    assert!(store.peek(1).is_some());
    assert_eq!(store.keys_most_recent_first(), vec![2, 3, 1]);

    store.insert(4, snapshot(SMALL, 4));
    assert_eq!(store.keys_most_recent_first(), vec![4, 2, 3, 1]);

    store.insert(3, snapshot(SMALL, 5));
    assert_eq!(store.keys_most_recent_first(), vec![3, 4, 2, 1]);

    assert!(store.remove(4));
    assert_eq!(store.keys_most_recent_first(), vec![3, 2, 1]);

    assert!(store.get(1).is_some());
    assert_eq!(store.keys_most_recent_first(), vec![1, 3, 2]);

    store.insert(9, snapshot(SMALL, 6));
    assert_eq!(store.keys_most_recent_first(), vec![9, 1, 3, 2]);
}

#[test]
fn a_composite_key_works_the_same() {
    // With several windows the app's key becomes (window, tab).
    let mut store = SnapshotStore::new(2 * SMALL);
    store.insert((1u32, 1u32), snapshot(SMALL, 1));
    store.insert((2, 1), snapshot(SMALL, 2));

    assert_eq!(store.insert((1, 2), snapshot(SMALL, 3)), stored(1));

    assert_eq!(store.peek((1, 1)), None);
    assert_eq!(store.keys_most_recent_first(), vec![(1, 2), (2, 1)]);
}

#[test]
fn the_default_cap_holds_222_full_size_snapshots_and_the_223rd_evicts_the_oldest() {
    // About 128 MB of pixels while it runs.
    let full_size = |id: u32| {
        let frame = RgbaFrame::new(480, 300, vec![id as u8; 480 * 300 * 4]).expect("480x300");
        Snapshot::new(frame, Duration::from_secs(u64::from(id)), (1440, 900))
    };
    let cost = 480 * 300 * 4 + ENTRY_OVERHEAD_BYTES;
    assert_eq!(cost, 576_256);
    assert_eq!(DEFAULT_CAP_BYTES / cost, 222);

    let mut store = SnapshotStore::default();
    for id in 0..222u32 {
        assert_eq!(store.insert(id, full_size(id)), stored(0), "snapshot {id}");
    }
    assert_eq!(store.len(), 222);
    assert_eq!(store.used_bytes(), 222 * cost);
    assert!(store.used_bytes() <= DEFAULT_CAP_BYTES);
    assert!(store.peek(0).is_some());

    // The 223rd does not fit next to 222 (the slack is below one snapshot): the oldest goes.
    assert_eq!(store.insert(222, full_size(222)), stored(1));

    assert_eq!(store.len(), 222);
    assert_eq!(store.used_bytes(), 222 * cost);
    assert_eq!(store.peek(0), None);
    assert!(store.peek(1).is_some());
    assert!(store.peek(222).is_some());
    assert_eq!(store.keys_most_recent_first().first(), Some(&222));
    assert_eq!(store.keys_most_recent_first().last(), Some(&1));
}

// ---- A long pseudo-random sequence checked against a naive model ------------------------------

/// A small linear congruential generator (Knuth's MMIX constants), so that the run is the same
/// every time and needs no crate.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // The high bits are the good ones.
        self.0 >> 33
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// The obvious version of the store: a list of (key, cost, id), the least recently used first.
struct Model {
    cap: usize,
    entries: Vec<(u32, usize, u64)>,
}

impl Model {
    fn used(&self) -> usize {
        self.entries.iter().map(|entry| entry.1).sum()
    }

    fn position(&self, key: u32) -> Option<usize> {
        self.entries.iter().position(|entry| entry.0 == key)
    }

    fn insert(&mut self, key: u32, cost: usize, id: u64) -> InsertOutcome {
        if cost > self.cap {
            return InsertOutcome::TooLarge;
        }
        if let Some(index) = self.position(key) {
            self.entries.remove(index);
        }
        let mut evicted = 0;
        while self.used() + cost > self.cap {
            self.entries.remove(0);
            evicted += 1;
        }
        self.entries.push((key, cost, id));
        InsertOutcome::Stored { evicted }
    }

    fn get(&mut self, key: u32) -> Option<u64> {
        let index = self.position(key)?;
        let entry = self.entries.remove(index);
        self.entries.push(entry);
        Some(entry.2)
    }

    fn peek(&self, key: u32) -> Option<u64> {
        self.position(key).map(|index| self.entries[index].2)
    }

    fn remove(&mut self, key: u32) -> bool {
        match self.position(key) {
            Some(index) => {
                self.entries.remove(index);
                true
            }
            None => false,
        }
    }

    fn retain(&mut self, modulus: u32, remainder: u32) -> usize {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.0 % modulus != remainder);
        before - self.entries.len()
    }

    fn set_cap(&mut self, cap: usize) -> usize {
        self.cap = cap;
        let mut evicted = 0;
        while self.used() > cap {
            self.entries.remove(0);
            evicted += 1;
        }
        evicted
    }
}

/// What the run did, so that the test can tell that it exercised every path.
#[derive(Default, Debug)]
struct Seen {
    inserts: usize,
    replacements: usize,
    too_large: usize,
    evictions: usize,
    get_hits: usize,
    get_misses: usize,
    peeks: usize,
    removes: usize,
    retains: usize,
    cap_changes: usize,
    cap_evictions: usize,
    clears: usize,
    nonempty_steps: usize,
}

fn check(store: &SnapshotStore<u32>, model: &Model, step: usize, operation: &str) {
    let context = format!("step {step} after {operation}");
    let keys = store.keys_most_recent_first();

    // The sum of the real costs of what the store holds, read back one by one.
    let sum: usize = keys
        .iter()
        .map(|key| {
            store
                .peek(*key)
                .expect("a listed key is there")
                .cost_bytes()
        })
        .sum();
    assert_eq!(store.used_bytes(), sum, "used_bytes is the sum ({context})");
    assert_eq!(sum, model.used(), "the model agrees on the sum ({context})");
    assert!(
        store.used_bytes() <= store.cap_bytes(),
        "never above the cap ({context})"
    );
    assert_eq!(store.cap_bytes(), model.cap, "the cap ({context})");

    let expected: Vec<u32> = model.entries.iter().rev().map(|entry| entry.0).collect();
    assert_eq!(keys, expected, "the recency order ({context})");
    assert_eq!(store.len(), model.entries.len(), "len ({context})");
    assert_eq!(store.is_empty(), model.entries.is_empty());
    for (key, _, id) in &model.entries {
        assert_eq!(id_of(store, *key), Some(*id), "key {key} ({context})");
    }
}

#[test]
fn two_thousand_random_operations_agree_with_a_naive_model_at_every_step() {
    let mut random = Lcg(0x5EED_CAFE_F00D_1234);
    let mut store = SnapshotStore::<u32>::new(3000);
    let mut model = Model {
        cap: 3000,
        entries: Vec::new(),
    };
    let mut seen = Seen::default();

    for step in 0..2000usize {
        let key = random.below(10) as u32;
        let id = step as u64;
        let choice = random.below(100);
        let operation = match choice {
            // Insert (replaces when the key is there), costs from 260 to 1056.
            0..=44 | 95..=99 => {
                let cost = SMALL + 4 * random.below(200) as usize;
                let existed = model.position(key).is_some();
                let expected = model.insert(key, cost, id);
                seen.inserts += 1;
                if existed && matches!(expected, InsertOutcome::Stored { .. }) {
                    seen.replacements += 1;
                }
                match expected {
                    InsertOutcome::TooLarge => seen.too_large += 1,
                    InsertOutcome::Stored { evicted } => seen.evictions += evicted,
                }
                assert_eq!(
                    store.insert(key, snapshot(cost, id)),
                    expected,
                    "insert outcome at step {step}"
                );
                format!("insert {key} cost {cost}")
            }
            45..=69 => {
                let expected = model.get(key);
                match expected {
                    Some(_) => seen.get_hits += 1,
                    None => seen.get_misses += 1,
                }
                let got = store
                    .get(key)
                    .map(|snapshot| snapshot.captured_at().as_secs());
                assert_eq!(got, expected, "get at step {step}");
                format!("get {key}")
            }
            70..=77 => {
                seen.peeks += 1;
                assert_eq!(id_of(&store, key), model.peek(key), "peek at step {step}");
                format!("peek {key}")
            }
            78..=84 => {
                seen.removes += 1;
                assert_eq!(
                    store.remove(key),
                    model.remove(key),
                    "remove at step {step}"
                );
                format!("remove {key}")
            }
            85..=87 => {
                seen.retains += 1;
                let modulus = 2 + random.below(3) as u32;
                let remainder = random.below(u64::from(modulus)) as u32;
                let expected = model.retain(modulus, remainder);
                let removed = store.retain(|key| key % modulus != remainder);
                assert_eq!(removed, expected, "retain at step {step}");
                format!("retain not {remainder} mod {modulus}")
            }
            88..=93 => {
                seen.cap_changes += 1;
                // Mostly between 300 and 6300; now and then 0.
                let cap = if random.below(20) == 0 {
                    0
                } else {
                    300 + random.below(6000) as usize
                };
                let expected = model.set_cap(cap);
                seen.cap_evictions += expected;
                assert_eq!(
                    store.set_cap_bytes(cap),
                    expected,
                    "set_cap_bytes at step {step}"
                );
                format!("set_cap_bytes {cap}")
            }
            // 94: one step in a hundred empties the store.
            _ => {
                seen.clears += 1;
                store.clear();
                model.entries.clear();
                "clear".to_owned()
            }
        };
        check(&store, &model, step, &operation);
        if !store.is_empty() {
            seen.nonempty_steps += 1;
        }
    }

    // Shown with `--nocapture`.
    println!("2000 steps: {seen:?}");
    // The sequence must have used every operation, or it proves little.
    assert!(seen.inserts > 800, "{seen:?}");
    assert!(seen.replacements > 100, "{seen:?}");
    assert!(seen.too_large > 5, "{seen:?}");
    assert!(seen.evictions > 200, "{seen:?}");
    assert!(seen.get_hits > 100, "{seen:?}");
    assert!(seen.get_misses > 20, "{seen:?}");
    assert!(seen.peeks > 100, "{seen:?}");
    assert!(seen.removes > 100, "{seen:?}");
    assert!(seen.retains > 30, "{seen:?}");
    assert!(seen.cap_changes > 80, "{seen:?}");
    assert!(seen.cap_evictions > 20, "{seen:?}");
    assert!(seen.clears > 5, "{seen:?}");
    // The store was not empty for most of the run.
    assert!(seen.nonempty_steps > 1000, "{seen:?}");
}
