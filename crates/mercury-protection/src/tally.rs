// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The weekly tally: blocks counted per UTC day and category, for the shield panel's week (MH14).

use std::collections::BTreeMap;

use crate::level::Category;

/// How many days are kept: today and the 34 before it.
pub const TALLY_DAYS_KEPT: i64 = 35;

const MS_PER_DAY: i64 = 86_400_000;

/// A change to one (day, category) count since the last [`WeeklyTally::take_dirty`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TallyDelta {
    /// The UTC day, as [`WeeklyTally::day_of`] counts it.
    pub day: i64,
    /// The category.
    pub category: Category,
    /// How much the count went up.
    pub delta: u64,
}

/// The week shown on the panel: seven days ending today.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeekView {
    /// Every block in the week.
    pub total: u64,
    /// Blocks per category in the week, in [`Category::ALL`] order.
    pub by_category: [u64; Category::COUNT],
    /// Blocks per day, oldest first; the last entry is today.
    pub per_day: [u64; 7],
}

/// Blocks per UTC day and category. Writes are remembered until [`WeeklyTally::take_dirty`] takes them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeeklyTally {
    days: BTreeMap<i64, [u64; Category::COUNT]>,
    dirty: BTreeMap<(i64, Category), u64>,
}

impl WeeklyTally {
    /// The UTC day of a time in milliseconds since the epoch. Negative times count backwards from day -1.
    pub fn day_of(ms: i64) -> i64 {
        ms.div_euclid(MS_PER_DAY)
    }

    /// Counts `n` blocks of a category at `now_ms`.
    pub fn record(&mut self, now_ms: i64, c: Category, n: u64) {
        let day = Self::day_of(now_ms);
        let counts = self.days.entry(day).or_insert([0; Category::COUNT]);
        add(&mut counts[c.index()], n);
        let pending = self.dirty.entry((day, c)).or_insert(0);
        add(pending, n);
    }

    /// The seven days ending at `now_ms`. Days after today are not counted.
    pub fn week(&self, now_ms: i64) -> WeekView {
        let today = Self::day_of(now_ms);
        let mut view = WeekView {
            total: 0,
            by_category: [0; Category::COUNT],
            per_day: [0; 7],
        };
        for (slot, offset) in view.per_day.iter_mut().zip((0..7).rev()) {
            let Some(counts) = self.days.get(&(today - offset)) else {
                continue;
            };
            for (category, &n) in counts.iter().enumerate() {
                add(&mut view.by_category[category], n);
                add(&mut view.total, n);
                add(slot, n);
            }
        }
        view
    }

    /// Drops the days older than [`TALLY_DAYS_KEPT`] before `now_ms`.
    pub fn prune(&mut self, now_ms: i64) {
        let oldest = Self::day_of(now_ms) - (TALLY_DAYS_KEPT - 1);
        self.days.retain(|day, _| *day >= oldest);
    }

    /// The changes since the last call, each (day, category) once. Empty when nothing changed.
    pub fn take_dirty(&mut self) -> Vec<TallyDelta> {
        std::mem::take(&mut self.dirty)
            .into_iter()
            .map(|((day, category), delta)| TallyDelta {
                day,
                category,
                delta,
            })
            .collect()
    }

    /// A tally from stored rows. The rows are not marked dirty: they came from the store.
    pub fn from_rows(rows: impl IntoIterator<Item = (i64, Category, u64)>) -> Self {
        let mut tally = Self::default();
        for (day, category, n) in rows {
            let counts = tally.days.entry(day).or_insert([0; Category::COUNT]);
            add(&mut counts[category.index()], n);
        }
        tally
    }

    /// Empties the tally, including the changes not yet taken.
    pub fn clear(&mut self) {
        self.days.clear();
        self.dirty.clear();
    }
}

fn add(slot: &mut u64, n: u64) {
    *slot = slot.saturating_add(n);
}
