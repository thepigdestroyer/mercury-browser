// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Protection exceptions, the weekly tally and bounce records (R13 MH12, MH32).
//!
//! The store holds primitives only: the model crate `mercury-protection` decides what the rows mean.

use rusqlite::{Connection, OptionalExtension, params};

use crate::Store;
use crate::error::Result;

/// Schema version 2: the protection tables (spec §6).
pub(crate) const MIGRATION_PROTECTION: &str = "
    CREATE TABLE protection_exceptions (
        site          TEXT PRIMARY KEY,
        shields_down  INTEGER NOT NULL,
        level         TEXT,
        overrides     TEXT NOT NULL,
        updated_at    INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE protection_tally (
        day       INTEGER NOT NULL,
        category  TEXT NOT NULL,
        count     INTEGER NOT NULL,
        PRIMARY KEY (day, category)
    ) STRICT;
    CREATE TABLE protection_bounce (
        site                TEXT PRIMARY KEY,
        last_activation_at  INTEGER,
        bounced_at          INTEGER
    ) STRICT;";

/// One per-site protection exception, as the table keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionExceptionRow {
    /// The site key.
    pub site: String,
    /// Whether protection is off for the site.
    pub shields_down: bool,
    /// The site's own level id, if it has one.
    pub level: Option<String>,
    /// The per-category overrides as text (`"ads=off,cookies=strict"`).
    pub overrides: String,
    /// When the exception was last written, in milliseconds since the Unix epoch.
    pub updated_at: i64,
}

/// One site's bounce-tracking record, as the table keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionBounceRow {
    /// The site key.
    pub site: String,
    /// The last user activation on the site, in milliseconds since the Unix epoch.
    pub last_activation_at: Option<i64>,
    /// When the site was seen bouncing, in milliseconds since the Unix epoch.
    pub bounced_at: Option<i64>,
}

/// One count of the weekly tally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionTallyRow {
    /// The day, as days since the Unix epoch.
    pub day: i64,
    /// The category id.
    pub category: String,
    /// How many were blocked; never negative.
    pub count: i64,
}

impl Store {
    /// Every protection exception, ordered by site.
    pub fn protection_exceptions(&self) -> Result<Vec<ProtectionExceptionRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT site, shields_down, level, overrides, updated_at
             FROM protection_exceptions ORDER BY site",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ProtectionExceptionRow {
                    site: row.get(0)?,
                    shields_down: row.get(1)?,
                    level: row.get(2)?,
                    overrides: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Stores an exception, replacing the one for the same site.
    pub fn put_protection_exception(&mut self, row: &ProtectionExceptionRow) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO protection_exceptions (site, shields_down, level, overrides, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (site) DO UPDATE SET
                    shields_down = excluded.shields_down,
                    level = excluded.level,
                    overrides = excluded.overrides,
                    updated_at = excluded.updated_at",
            )?
            .execute(params![
                row.site,
                row.shields_down,
                row.level,
                row.overrides,
                row.updated_at,
            ])?;
        Ok(())
    }

    /// Removes a site's exception. Returns whether there was one.
    pub fn remove_protection_exception(&mut self, site: &str) -> Result<bool> {
        let removed = self
            .conn
            .prepare_cached("DELETE FROM protection_exceptions WHERE site = ?1")?
            .execute([site])?;
        Ok(removed > 0)
    }

    /// Removes every exception and returns how many there were.
    pub fn clear_protection_exceptions(&mut self) -> Result<u64> {
        let removed = self
            .conn
            .prepare_cached("DELETE FROM protection_exceptions")?
            .execute([])?;
        Ok(removed as u64)
    }

    /// Adds `delta` to the count for a day and category, creating the row if needed. The count
    /// saturates at [`i64::MAX`].
    pub fn add_protection_tally(&mut self, day: i64, category: &str, delta: u64) -> Result<()> {
        let delta = i64::try_from(delta).unwrap_or(i64::MAX);
        let tx = self.conn.transaction()?;
        let current: i64 = tx
            .prepare_cached("SELECT count FROM protection_tally WHERE day = ?1 AND category = ?2")?
            .query_row(params![day, category], |row| row.get(0))
            .optional()?
            .unwrap_or(0);
        tx.prepare_cached(
            "INSERT INTO protection_tally (day, category, count) VALUES (?1, ?2, ?3)
             ON CONFLICT (day, category) DO UPDATE SET count = excluded.count",
        )?
        .execute(params![day, category, current.saturating_add(delta)])?;
        tx.commit()?;
        Ok(())
    }

    /// The tally rows for `from_day` and later, ordered by day and then category.
    pub fn protection_tally_since(&self, from_day: i64) -> Result<Vec<ProtectionTallyRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT day, category, count FROM protection_tally
             WHERE day >= ?1 ORDER BY day, category",
        )?;
        let rows = stmt
            .query_map([from_day], |row| {
                Ok(ProtectionTallyRow {
                    day: row.get(0)?,
                    category: row.get(1)?,
                    count: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Deletes the tally rows for days before `before_day`. Returns how many were deleted.
    pub fn prune_protection_tally(&mut self, before_day: i64) -> Result<u64> {
        let removed = self
            .conn
            .prepare_cached("DELETE FROM protection_tally WHERE day < ?1")?
            .execute([before_day])?;
        Ok(removed as u64)
    }

    /// Removes every tally row and returns how many there were.
    pub fn clear_protection_tally(&mut self) -> Result<u64> {
        let removed = self
            .conn
            .prepare_cached("DELETE FROM protection_tally")?
            .execute([])?;
        Ok(removed as u64)
    }

    /// Every bounce record, ordered by site.
    pub fn protection_bounce_records(&self) -> Result<Vec<ProtectionBounceRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT site, last_activation_at, bounced_at FROM protection_bounce ORDER BY site",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ProtectionBounceRow {
                    site: row.get(0)?,
                    last_activation_at: row.get(1)?,
                    bounced_at: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Replaces the whole set of bounce records in one transaction. If any row is refused, the
    /// old set stays.
    pub fn replace_protection_bounce(&mut self, rows: &[ProtectionBounceRow]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.prepare_cached("DELETE FROM protection_bounce")?
            .execute([])?;
        for row in rows {
            tx.prepare_cached(
                "INSERT INTO protection_bounce (site, last_activation_at, bounced_at)
                 VALUES (?1, ?2, ?3)",
            )?
            .execute(params![row.site, row.last_activation_at, row.bounced_at])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes every bounce record and returns how many there were.
    pub fn clear_protection_bounce(&mut self) -> Result<u64> {
        let removed = self
            .conn
            .prepare_cached("DELETE FROM protection_bounce")?
            .execute([])?;
        Ok(removed as u64)
    }
}

/// Deletes the bounce records a history clear covers: with `Some(from_ms)`, those whose activation
/// or bounce time is at or after `from_ms`; with `None`, all of them. Both times older than the
/// range keep the record.
pub(crate) fn forget_bounce(conn: &Connection, from_ms: Option<i64>) -> Result<()> {
    match from_ms {
        Some(from) => {
            conn.prepare_cached(
                "DELETE FROM protection_bounce
                 WHERE last_activation_at >= ?1 OR bounced_at >= ?1",
            )?
            .execute([from])?;
        }
        None => {
            conn.prepare_cached("DELETE FROM protection_bounce")?
                .execute([])?;
        }
    }
    Ok(())
}
