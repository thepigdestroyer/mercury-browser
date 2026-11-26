// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Recent searches: the terms the user searched for, newest first.

use rusqlite::{Connection, params};

use crate::Store;
use crate::error::Result;
use crate::history::ClearRange;

/// At most this many searches are kept; the oldest (by last search, then id) are dropped.
pub const MAX_RECENT_SEARCHES: u32 = 200;

/// Search terms longer than this many characters (after normalisation) are refused.
pub const MAX_SEARCH_TERM_CHARS: usize = 256;

/// [`Store::recent_searches`] lists at most this many searches, whatever it is asked for.
const LISTED_SEARCHES: u32 = 20;

const RECENT_SQL: &str = "
    SELECT term, engine_id, count, last_at FROM searches
    WHERE term_key >= ?1 AND (?2 IS NULL OR term_key < ?2)
    ORDER BY last_at DESC, id DESC
    LIMIT ?3";

/// One recent search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentSearch {
    /// The term, trimmed and with white space collapsed, as it was last typed.
    pub term: String,
    /// The search engine it was sent to, if any.
    pub engine_id: Option<String>,
    /// How many times the term was searched.
    pub count: u32,
    /// When it was last searched, in milliseconds since the Unix epoch.
    pub last_at_ms: i64,
}

impl Store {
    /// Records a search for `term` at `now_ms`. Terms that are the same ignoring case and white
    /// space count as one search: the count goes up and the latest spelling and engine are
    /// kept. Returns `false` (and stores nothing) if the term is empty, has more than
    /// [`MAX_SEARCH_TERM_CHARS`] characters, or has a control character.
    pub fn record_search(
        &mut self,
        term: &str,
        engine_id: Option<&str>,
        now_ms: i64,
    ) -> Result<bool> {
        let Some(term) = normalise(term) else {
            return Ok(false);
        };
        let tx = self.conn.transaction()?;
        tx.prepare_cached(
            "INSERT INTO searches (term_key, term, engine_id, count, first_at, last_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?4)
             ON CONFLICT (term_key) DO UPDATE SET
                 term = excluded.term,
                 engine_id = excluded.engine_id,
                 count = searches.count + 1,
                 last_at = excluded.last_at",
        )?
        .execute(params![term.to_lowercase(), term, engine_id, now_ms])?;
        tx.prepare_cached(
            "DELETE FROM searches WHERE id IN (
                 SELECT id FROM searches ORDER BY last_at DESC, id DESC LIMIT -1 OFFSET ?1)",
        )?
        .execute([i64::from(MAX_RECENT_SEARCHES)])?;
        tx.commit()?;
        Ok(true)
    }

    /// The recent searches whose term starts with `prefix` (ignoring case, and with white space
    /// normalised as in [`Store::record_search`]; an empty prefix matches all), newest first.
    /// At most 20 are returned. `%` and `_` are ordinary characters.
    pub fn recent_searches(&self, prefix: &str, limit: u32) -> Result<Vec<RecentSearch>> {
        let key = normalise(prefix).unwrap_or_default().to_lowercase();
        let end = prefix_end(&key);
        let mut stmt = self.conn.prepare_cached(RECENT_SQL)?;
        let rows = stmt
            .query_map(params![key, end, limit.min(LISTED_SEARCHES)], |row| {
                Ok(RecentSearch {
                    term: row.get(0)?,
                    engine_id: row.get(1)?,
                    count: row.get(2)?,
                    last_at_ms: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Deletes the search for `term` (ignoring case and white space). Returns `false` if there
    /// was none. The deleted row is overwritten (`secure_delete`).
    pub fn delete_recent_search(&mut self, term: &str) -> Result<bool> {
        let Some(term) = normalise(term) else {
            return Ok(false);
        };
        let removed = self
            .conn
            .prepare_cached("DELETE FROM searches WHERE term_key = ?1")?
            .execute([term.to_lowercase()])?;
        Ok(removed > 0)
    }

    /// Deletes the searches in `range` and returns how many were removed.
    pub fn clear_recent_searches(&mut self, range: ClearRange) -> Result<u64> {
        clear_searches(&self.conn, range)
    }
}

/// Deletes the searches in `range`. Shared with [`Store::clear_history`], which calls it inside
/// its own transaction.
pub(crate) fn clear_searches(conn: &Connection, range: ClearRange) -> Result<u64> {
    let removed = match range {
        ClearRange::All => conn.prepare_cached("DELETE FROM searches")?.execute([])?,
        ClearRange::Since { from_ms } => conn
            .prepare_cached("DELETE FROM searches WHERE last_at >= ?1")?
            .execute([from_ms])?,
    };
    Ok(removed as u64)
}

/// The text with its white space trimmed and collapsed, or `None` if it is empty, too long or
/// has a control character.
fn normalise(text: &str) -> Option<String> {
    let term = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = term.chars().count();
    if length == 0 || length > MAX_SEARCH_TERM_CHARS || term.chars().any(char::is_control) {
        return None;
    }
    Some(term)
}

/// The smallest key that does not start with `prefix`, for a key range scan; `None` if there is
/// no such key (an empty prefix, or one made only of the largest characters).
fn prefix_end(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = next_char(last) {
            chars.push(next);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

/// The character after `c` in code point order, skipping the surrogate range.
fn next_char(c: char) -> Option<char> {
    match c as u32 + 1 {
        0xD800 => Some('\u{E000}'),
        next => char::from_u32(next),
    }
}
