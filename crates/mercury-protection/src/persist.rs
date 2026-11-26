// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Persistence of exceptions, the weekly tally and bounce records through `mercury-store` (R13 MH12, MH14, MH32).
//!
//! Loading never rewrites a row. A row this build cannot read (an unknown level, a site that is not a [`Site`]) is
//! left in the store and counted in [`Loaded::skipped`]; an unknown override pair is dropped from an otherwise good
//! row and counted the same way.

use mercury_store::{
    ProtectionBounceRow, ProtectionExceptionRow, ProtectionTallyRow, Store, StoreError,
};

use crate::bounce::{BounceConfig, BounceTracker, SiteRecord};
use crate::exceptions::{ExceptionList, Overrides, SiteException};
use crate::level::{Category, ProtectionLevel};
use crate::site::Site;
use crate::tally::{TALLY_DAYS_KEPT, WeeklyTally};

/// A value read from the store, and how many stored rows were skipped to read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded<T> {
    /// The value.
    pub value: T,
    /// Rows or pairs this build could not read.
    pub skipped: usize,
}

/// Reads every exception. Rows that are not valid are skipped and counted; the store is not changed.
pub fn load_exceptions(store: &Store) -> Result<Loaded<ExceptionList>, StoreError> {
    let mut list = ExceptionList::default();
    let mut skipped = 0;
    for row in store.protection_exceptions()? {
        match exception_from_row(&row) {
            Some((exception, dropped)) => {
                skipped += dropped;
                if list.set(exception).is_err() {
                    skipped += 1;
                }
            }
            None => skipped += 1,
        }
    }
    Ok(Loaded {
        value: list,
        skipped,
    })
}

/// Stores one exception, replacing the one for its site. An exception that changes nothing removes the row.
pub fn save_exception(store: &mut Store, e: &SiteException) -> Result<(), StoreError> {
    if e.shields_down || e.level.is_some() || !e.overrides.is_empty() {
        store.put_protection_exception(&exception_to_row(e))
    } else {
        store
            .remove_protection_exception(e.site.as_str())
            .map(|_| ())
    }
}

/// Removes a site's exception. Returns whether there was one.
pub fn delete_exception(store: &mut Store, site: &Site) -> Result<bool, StoreError> {
    store.remove_protection_exception(site.as_str())
}

/// Writes the tally's changes since the last flush, and prunes the days that are too old to show.
pub fn flush_tally(
    store: &mut Store,
    tally: &mut WeeklyTally,
    now_ms: i64,
) -> Result<(), StoreError> {
    for delta in tally.take_dirty() {
        store.add_protection_tally(delta.day, delta.category.id(), delta.delta)?;
    }
    store.prune_protection_tally(WeeklyTally::day_of(now_ms) - (TALLY_DAYS_KEPT - 1))?;
    Ok(())
}

/// Reads the tally of the days still shown at `now_ms`. Rows with an unknown category are skipped and counted.
pub fn load_tally(store: &Store, now_ms: i64) -> Result<Loaded<WeeklyTally>, StoreError> {
    let from_day = WeeklyTally::day_of(now_ms) - (TALLY_DAYS_KEPT - 1);
    let mut rows = Vec::new();
    let mut skipped = 0;
    for row in store.protection_tally_since(from_day)? {
        match tally_from_row(&row) {
            Some(parsed) => rows.push(parsed),
            None => skipped += 1,
        }
    }
    Ok(Loaded {
        value: WeeklyTally::from_rows(rows),
        skipped,
    })
}

/// Reads the bounce records. The records are kept as stored; a site that is not a [`Site`] is skipped and counted.
pub fn load_bounce(store: &Store, cfg: BounceConfig) -> Result<Loaded<BounceTracker>, StoreError> {
    let mut records = Vec::new();
    let mut skipped = 0;
    for row in store.protection_bounce_records()? {
        match bounce_from_row(&row) {
            Some(record) => records.push(record),
            None => skipped += 1,
        }
    }
    Ok(Loaded {
        value: BounceTracker::from_records(cfg, records),
        skipped,
    })
}

/// Replaces the stored bounce records with the tracker's.
pub fn save_bounce(store: &mut Store, tracker: &BounceTracker) -> Result<(), StoreError> {
    let rows: Vec<ProtectionBounceRow> = tracker.records().iter().map(bounce_to_row).collect();
    store.replace_protection_bounce(&rows)
}

/// Removes the exceptions, the tally and the bounce records: the "reset protections" action.
pub fn clear_all(store: &mut Store) -> Result<(), StoreError> {
    store.clear_protection_exceptions()?;
    store.clear_protection_tally()?;
    store.clear_protection_bounce()?;
    Ok(())
}

fn exception_to_row(e: &SiteException) -> ProtectionExceptionRow {
    ProtectionExceptionRow {
        site: e.site.as_str().to_owned(),
        shields_down: e.shields_down,
        level: e.level.map(|level| level.id().to_owned()),
        overrides: e.overrides.to_text(),
        updated_at: e.updated_at_ms,
    }
}

/// The exception a row holds, and how many override pairs were dropped. `None` when the row itself is not valid.
fn exception_from_row(row: &ProtectionExceptionRow) -> Option<(SiteException, usize)> {
    let site = Site::from_key(&row.site)?;
    let level = match row.level.as_deref() {
        Some(id) => Some(ProtectionLevel::from_id(id)?),
        None => None,
    };
    let (overrides, dropped) = Overrides::from_text(&row.overrides);
    Some((
        SiteException {
            site,
            shields_down: row.shields_down,
            level,
            overrides,
            updated_at_ms: row.updated_at,
        },
        dropped,
    ))
}

fn tally_from_row(row: &ProtectionTallyRow) -> Option<(i64, Category, u64)> {
    let category = Category::from_id(&row.category)?;
    let count = u64::try_from(row.count).ok()?;
    Some((row.day, category, count))
}

fn bounce_to_row(record: &SiteRecord) -> ProtectionBounceRow {
    ProtectionBounceRow {
        site: record.site.as_str().to_owned(),
        last_activation_at: record.last_activation_ms,
        bounced_at: record.bounced_at_ms,
    }
}

fn bounce_from_row(row: &ProtectionBounceRow) -> Option<SiteRecord> {
    Some(SiteRecord {
        site: Site::from_key(&row.site)?,
        last_activation_ms: row.last_activation_at,
        bounced_at_ms: row.bounced_at,
    })
}
