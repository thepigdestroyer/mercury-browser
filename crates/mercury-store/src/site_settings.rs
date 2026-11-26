// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Per-site settings, protocol handlers and their metadata, as plain text and numbers.
//!
//! The store does not interpret the strings: patterns, axes and values are decided by
//! `mercury-site-settings`, which writes them here in batches.

use rusqlite::params;

use crate::Store;
use crate::error::{Result, StoreError};
use crate::limits::{
    MAX_HANDLER_TEMPLATE_BYTES, MAX_HANDLER_TITLE_CHARS, MAX_SITE_FIELD_BYTES,
    MAX_SITE_META_VALUE_BYTES, MAX_SITE_PATTERN_BYTES,
};

/// One row of `site_settings`: one setting for one pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteSettingRow {
    /// The canonical pattern text, `"*"` for the default.
    pub pattern: String,
    /// The stable axis id, for example `"camera"`.
    pub axis: String,
    /// `""`, or the scheme for an axis that takes a qualifier.
    pub qualifier: String,
    /// The encoded value, for example `"allow"` or `"zoom:125"`.
    pub value: String,
    /// When the value was set, in milliseconds since the Unix epoch.
    pub set_at: i64,
    /// When the value stops applying, in milliseconds since the Unix epoch; `None` if permanent.
    pub expires_at: Option<i64>,
    /// When the value last answered a request, in milliseconds since the Unix epoch.
    pub last_used_at: Option<i64>,
    /// Whether unused-permission revocation skips this row.
    pub revoke_exempt: bool,
}

/// One row of `protocol_handlers`: a web app registered for a protocol scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolHandlerRow {
    /// The protocol scheme, for example `"mailto"`.
    pub scheme: String,
    /// The origin of the web app, for example `"https://mail.a.com"`.
    pub origin: String,
    /// The URL to open, with `%s` where the protocol URL goes.
    pub url_template: String,
    /// The name shown to the user.
    pub title: String,
    /// When it was registered, in milliseconds since the Unix epoch.
    pub added_at: i64,
    /// When it last handled a link, in milliseconds since the Unix epoch.
    pub last_used_at: Option<i64>,
    /// Whether it is the handler opened for this scheme.
    pub is_default: bool,
}

/// One change to the site settings tables, applied by [`Store::apply_site_setting_changes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteSettingChange {
    /// Inserts a row, or replaces the row with the same pattern, axis and qualifier.
    Put(SiteSettingRow),
    /// Deletes one row. Deleting a row that is not there does nothing.
    Remove {
        /// The pattern of the row.
        pattern: String,
        /// The axis of the row.
        axis: String,
        /// The qualifier of the row.
        qualifier: String,
    },
    /// Deletes every row of one pattern.
    RemovePattern {
        /// The pattern whose rows go.
        pattern: String,
    },
    /// Deletes every row of `site_settings`. Handlers and meta values are kept.
    RemoveAll,
    /// Inserts a handler, or replaces the one with the same scheme and origin.
    PutHandler(ProtocolHandlerRow),
    /// Deletes one handler.
    RemoveHandler {
        /// The scheme of the handler.
        scheme: String,
        /// The origin of the handler.
        origin: String,
    },
    /// Sets a meta value, replacing any earlier value for the key.
    SetMeta {
        /// The meta key.
        key: String,
        /// The value.
        value: String,
    },
}

/// Everything stored about site settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteSettingsSnapshot {
    /// The rows, ordered by pattern, axis and qualifier.
    pub rows: Vec<SiteSettingRow>,
    /// The handlers, ordered by scheme and origin.
    pub handlers: Vec<ProtocolHandlerRow>,
    /// The meta values, ordered by key.
    pub meta: Vec<(String, String)>,
}

impl Store {
    /// Every stored row, handler and meta value.
    pub fn site_settings_snapshot(&self) -> Result<SiteSettingsSnapshot> {
        let rows = self
            .conn
            .prepare_cached(
                "SELECT pattern, axis, qualifier, value, set_at, expires_at, last_used_at,
                        revoke_exempt
                 FROM site_settings ORDER BY pattern, axis, qualifier",
            )?
            .query_map([], |row| {
                Ok(SiteSettingRow {
                    pattern: row.get(0)?,
                    axis: row.get(1)?,
                    qualifier: row.get(2)?,
                    value: row.get(3)?,
                    set_at: row.get(4)?,
                    expires_at: row.get(5)?,
                    last_used_at: row.get(6)?,
                    revoke_exempt: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let handlers = self
            .conn
            .prepare_cached(
                "SELECT scheme, origin, url_template, title, added_at, last_used_at, is_default
                 FROM protocol_handlers ORDER BY scheme, origin",
            )?
            .query_map([], |row| {
                Ok(ProtocolHandlerRow {
                    scheme: row.get(0)?,
                    origin: row.get(1)?,
                    url_template: row.get(2)?,
                    title: row.get(3)?,
                    added_at: row.get(4)?,
                    last_used_at: row.get(5)?,
                    is_default: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let meta = self
            .conn
            .prepare_cached("SELECT key, value FROM site_settings_meta ORDER BY key")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(SiteSettingsSnapshot {
            rows,
            handlers,
            meta,
        })
    }

    /// Applies `changes` in one transaction, in order. Every text field is checked before
    /// anything is written: a field that is too long refuses the whole batch with
    /// [`StoreError::InvalidArgument`] and changes nothing.
    pub fn apply_site_setting_changes(&mut self, changes: &[SiteSettingChange]) -> Result<()> {
        changes.iter().try_for_each(check_change)?;
        let tx = self.conn.transaction()?;
        for change in changes {
            match change {
                SiteSettingChange::Put(row) => {
                    tx.prepare_cached(
                        "INSERT OR REPLACE INTO site_settings
                            (pattern, axis, qualifier, value, set_at, expires_at, last_used_at,
                             revoke_exempt)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    )?
                    .execute(params![
                        row.pattern,
                        row.axis,
                        row.qualifier,
                        row.value,
                        row.set_at,
                        row.expires_at,
                        row.last_used_at,
                        row.revoke_exempt,
                    ])?;
                }
                SiteSettingChange::Remove {
                    pattern,
                    axis,
                    qualifier,
                } => {
                    tx.prepare_cached(
                        "DELETE FROM site_settings WHERE pattern = ?1 AND axis = ?2 AND qualifier = ?3",
                    )?
                    .execute(params![pattern, axis, qualifier])?;
                }
                SiteSettingChange::RemovePattern { pattern } => {
                    tx.prepare_cached("DELETE FROM site_settings WHERE pattern = ?1")?
                        .execute([pattern])?;
                }
                SiteSettingChange::RemoveAll => {
                    tx.prepare_cached("DELETE FROM site_settings")?
                        .execute([])?;
                }
                SiteSettingChange::PutHandler(row) => {
                    tx.prepare_cached(
                        "INSERT OR REPLACE INTO protocol_handlers
                            (scheme, origin, url_template, title, added_at, last_used_at, is_default)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    )?
                    .execute(params![
                        row.scheme,
                        row.origin,
                        row.url_template,
                        row.title,
                        row.added_at,
                        row.last_used_at,
                        row.is_default,
                    ])?;
                }
                SiteSettingChange::RemoveHandler { scheme, origin } => {
                    tx.prepare_cached(
                        "DELETE FROM protocol_handlers WHERE scheme = ?1 AND origin = ?2",
                    )?
                    .execute(params![scheme, origin])?;
                }
                SiteSettingChange::SetMeta { key, value } => {
                    tx.prepare_cached(
                        "INSERT OR REPLACE INTO site_settings_meta (key, value) VALUES (?1, ?2)",
                    )?
                    .execute(params![key, value])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes the rows whose `expires_at` is at or before `now_ms`, and returns how many went.
    /// Permanent rows (no `expires_at`) are never deleted.
    pub fn delete_expired_site_settings(&mut self, now_ms: i64) -> Result<usize> {
        Ok(self
            .conn
            .prepare_cached(
                "DELETE FROM site_settings WHERE expires_at IS NOT NULL AND expires_at <= ?1",
            )?
            .execute([now_ms])?)
    }
}

/// Refuses a change with a text field longer than its limit.
fn check_change(change: &SiteSettingChange) -> Result<()> {
    match change {
        SiteSettingChange::Put(row) => {
            pattern_fits(&row.pattern)?;
            field_fits(&row.axis)?;
            field_fits(&row.qualifier)?;
            field_fits(&row.value)
        }
        SiteSettingChange::Remove {
            pattern,
            axis,
            qualifier,
        } => {
            pattern_fits(pattern)?;
            field_fits(axis)?;
            field_fits(qualifier)
        }
        SiteSettingChange::RemovePattern { pattern } => pattern_fits(pattern),
        SiteSettingChange::RemoveAll => Ok(()),
        SiteSettingChange::PutHandler(row) => {
            field_fits(&row.scheme)?;
            bounded(
                &row.origin,
                MAX_SITE_PATTERN_BYTES,
                "handler origin too long",
            )?;
            bounded(
                &row.url_template,
                MAX_HANDLER_TEMPLATE_BYTES,
                "handler URL template too long",
            )?;
            if row.title.chars().count() > MAX_HANDLER_TITLE_CHARS {
                return Err(StoreError::InvalidArgument("handler title too long"));
            }
            Ok(())
        }
        SiteSettingChange::RemoveHandler { scheme, origin } => {
            field_fits(scheme)?;
            bounded(origin, MAX_SITE_PATTERN_BYTES, "handler origin too long")
        }
        SiteSettingChange::SetMeta { key, value } => {
            field_fits(key)?;
            bounded(value, MAX_SITE_META_VALUE_BYTES, "meta value too long")
        }
    }
}

fn pattern_fits(pattern: &str) -> Result<()> {
    bounded(pattern, MAX_SITE_PATTERN_BYTES, "site pattern too long")
}

fn field_fits(field: &str) -> Result<()> {
    bounded(field, MAX_SITE_FIELD_BYTES, "site setting field too long")
}

fn bounded(text: &str, max_bytes: usize, reason: &'static str) -> Result<()> {
    if text.len() > max_bytes {
        return Err(StoreError::InvalidArgument(reason));
    }
    Ok(())
}
