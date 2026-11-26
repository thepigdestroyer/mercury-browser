// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The one-time import of the lists that exist today: the "never save" origins, the pop-up
//! blocking switch and the default zoom. It runs once per profile, and never overwrites a rule
//! the user has already set.

use mercury_store::SiteSettingChange;

use crate::axis::Axis;
use crate::lifetime::Lifetime;
use crate::origin::Origin;
use crate::pattern::OriginPattern;
use crate::settings::{RuleKey, SiteSettings};
use crate::value::{Setting, Value};
use crate::{MAX_ZOOM_PERCENT, MIN_ZOOM_PERCENT, UnixMillis};

const DONE_KEY: &str = "legacy_import_v1";

/// What the current settings model stores, read from `mercury-settings` once at upgrade.
pub struct LegacySources<'a> {
    /// `passwords.never_save_origins`.
    pub never_save_password_origins: &'a [String],
    /// `privacy.block_popups`.
    pub block_popups: bool,
    /// `appearance.default_zoom_percent`.
    pub default_zoom_percent: u16,
}

/// What [`SiteSettings::import_legacy`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LegacyReport {
    /// The import had already run, so nothing was done.
    pub already_done: bool,
    /// Never-save origins written as a permanent block of save-passwords.
    pub never_save_imported: usize,
    /// Never-save entries skipped: not an origin, duplicated, or already set by the user.
    pub never_save_skipped: usize,
    /// A permanent pop-up default was written (only when pop-ups are allowed, as the default is
    /// blocked).
    pub popups_default_written: bool,
    /// A zoom default was written (only when it is not the built-in 100%).
    pub zoom_default_written: bool,
}

impl SiteSettings {
    /// Imports `sources` once. The first call marks the profile done; later calls return
    /// `already_done` and change nothing.
    pub fn import_legacy(&mut self, sources: &LegacySources<'_>, now: UnixMillis) -> LegacyReport {
        if self.legacy_import_done() {
            return LegacyReport {
                already_done: true,
                ..LegacyReport::default()
            };
        }
        let mut report = LegacyReport::default();
        let mut seen = std::collections::BTreeSet::new();
        for text in sources.never_save_password_origins {
            let Ok(origin) = Origin::parse(text) else {
                report.never_save_skipped += 1;
                continue;
            };
            if !seen.insert(origin.clone()) {
                report.never_save_skipped += 1;
                continue;
            }
            let pattern = OriginPattern::for_origin(&origin);
            if self.has_record(&pattern, Axis::SavePasswords, now) {
                report.never_save_skipped += 1;
                continue;
            }
            if self
                .set(
                    &pattern,
                    Axis::SavePasswords,
                    Value::Permission(Setting::Block),
                    Lifetime::Permanent,
                    now,
                )
                .is_ok()
            {
                report.never_save_imported += 1;
            } else {
                report.never_save_skipped += 1;
            }
        }
        if !sources.block_popups {
            report.popups_default_written = self
                .set_default(Axis::Popups, Value::Permission(Setting::Allow), now)
                .is_ok();
        }
        if sources.default_zoom_percent != 100 {
            let zoom = sources
                .default_zoom_percent
                .clamp(MIN_ZOOM_PERCENT, MAX_ZOOM_PERCENT);
            report.zoom_default_written =
                self.set_default(Axis::Zoom, Value::Zoom(zoom), now).is_ok();
        }
        self.meta.insert(DONE_KEY.to_owned(), "1".to_owned());
        self.journal.push(SiteSettingChange::SetMeta {
            key: DONE_KEY.to_owned(),
            value: "1".to_owned(),
        });
        report
    }

    /// Whether [`import_legacy`](Self::import_legacy) has already run for this profile.
    pub fn legacy_import_done(&self) -> bool {
        self.meta.get(DONE_KEY).is_some_and(|v| v == "1")
    }

    /// Whether a live record is already set for `pattern` and `axis` with no qualifier.
    fn has_record(&self, pattern: &OriginPattern, axis: Axis, now: UnixMillis) -> bool {
        let key = RuleKey {
            axis,
            qualifier: String::new(),
            pattern: pattern.clone(),
        };
        self.rules.get(&key).is_some_and(|slot| {
            slot.persistent.as_ref().is_some_and(|r| r.is_live(now))
                || slot.ephemeral.as_ref().is_some_and(|r| r.is_live(now))
        })
    }
}
