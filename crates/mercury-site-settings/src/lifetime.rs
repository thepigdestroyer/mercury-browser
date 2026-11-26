// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! How long an answer lasts.

use crate::UnixMillis;

/// How long a rule lasts. "Allow this time" is [`ThisVisit`](Self::ThisVisit); "for 1 hour" and
/// "for 24 hours" are [`Until`](Self::Until); "on every visit" and "never" are
/// [`Permanent`](Self::Permanent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    /// Until the site's last tab closes. Never written to disk.
    ThisVisit,
    /// Until the browser quits. Never written to disk.
    Session,
    /// Until this time (milliseconds since the epoch); written to disk.
    Until(UnixMillis),
    /// Until the user changes it; written to disk.
    Permanent,
}

impl Lifetime {
    /// A timed lifetime of `ms` milliseconds from `now` (saturating).
    pub const fn for_duration(now: UnixMillis, ms: i64) -> Self {
        Self::Until(now.saturating_add(ms))
    }

    /// Whether a rule with this lifetime is written to the profile database.
    pub const fn is_persistent(&self) -> bool {
        matches!(self, Self::Until(_) | Self::Permanent)
    }
}
