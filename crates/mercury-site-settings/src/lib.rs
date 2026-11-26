// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the user decided per site, as data.
//!
//! A library with no engine, no Slint and no clock. It holds origin patterns ([`OriginPattern`]),
//! the things a site can be allowed or blocked from ([`Axis`]) with their values ([`Value`]) and
//! lifetimes ([`Lifetime`]).
//!
//! Every method whose answer depends on the time takes `now` (milliseconds since the Unix epoch,
//! [`UnixMillis`]); nothing here reads the clock.
//!
//! - [`Origin`] and [`OriginPattern`]: what a rule can be about, and how rules rank
//!   ([`Specificity`]).
//! - [`Axis`], [`Value`], [`Setting`]: the closed set of per-site decisions and their values.
//! - [`Lifetime`]: how long an answer lasts.

#![warn(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod axis;
mod lifetime;
mod origin;
mod pattern;
mod upkeep;
mod value;

pub use axis::Axis;
pub use lifetime::Lifetime;
pub use origin::Origin;
pub use pattern::{OriginPattern, PatternError, Specificity};
pub use value::{AutoplayPolicy, DataRule, Setting, Value, ValueKind};

/// Milliseconds since the Unix epoch, as `mercury-store` keeps times.
pub type UnixMillis = i64;

/// One hour in milliseconds.
pub const HOUR_MS: i64 = 3_600_000;
/// One day in milliseconds.
pub const DAY_MS: i64 = 86_400_000;
/// A `Permanent` allowed sensitive permission unused for this long is reset to "ask" (Chrome's
/// period: about three months).
pub const AUTO_REVOKE_AFTER_MS: i64 = 90 * DAY_MS;
/// `record_use` writes a rule's last-used time at most this often.
pub const LAST_USED_GRANULARITY_MS: i64 = HOUR_MS;
/// The most rules the model holds; `load` ignores the rest and `set` refuses a new one.
pub const MAX_RULES: usize = 50_000;
/// The most protocol handlers the model holds.
pub const MAX_HANDLERS: usize = 2_000;
/// The smallest zoom a site can have, in percent (equal to `mercury-settings`').
pub const MIN_ZOOM_PERCENT: u16 = 30;
/// The largest zoom a site can have, in percent (equal to `mercury-settings`').
pub const MAX_ZOOM_PERCENT: u16 = 500;
