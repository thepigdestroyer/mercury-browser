// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Keeping the rules honest: ending a visit, expiring timed rules, stamping use and revoking
//! permissions nobody has used.

use mercury_store::SiteSettingChange;

use crate::axis::Axis;
use crate::lifetime::Lifetime;
use crate::origin::Origin;
use crate::pattern::OriginPattern;
use crate::settings::{RuleKey, RuleView, SiteSettings, Slot, row_for, view_of};
use crate::value::{Setting, Value};
use crate::{LAST_USED_GRANULARITY_MS, UnixMillis};

impl SiteSettings {
    /// Ends a visit to `origin`: removes its `ThisVisit` rules (those whose pattern matches the
    /// origin, including `site:` and `*.` patterns), and returns how many went. Stored rules
    /// beneath them answer again.
    pub fn end_visit(&mut self, origin: &Origin) -> usize {
        let keys: Vec<RuleKey> = self
            .rules
            .iter()
            .filter(|(key, slot)| {
                slot.ephemeral
                    .as_ref()
                    .is_some_and(|record| record.lifetime == Lifetime::ThisVisit)
                    && key.pattern.matches(origin).is_some()
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &keys {
            if let Some(slot) = self.rules.get_mut(key) {
                slot.ephemeral = None;
            }
            self.drop_if_empty(key);
        }
        keys.len()
    }

    /// Deletes timed persistent rules whose end has passed at `now`, journals each removal, and
    /// returns how many went. A rule whose `set_at` is after `now` (the clock moved back) stays.
    pub fn sweep(&mut self, now: UnixMillis) -> usize {
        let expired: Vec<RuleKey> = self
            .rules
            .iter()
            .filter(|(_, slot)| {
                slot.persistent.as_ref().is_some_and(|record| {
                    record.set_at <= now
                        && matches!(record.lifetime, Lifetime::Until(end) if now >= end)
                })
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &expired {
            if let Some(slot) = self.rules.get_mut(key) {
                slot.persistent = None;
            }
            self.drop_if_empty(key);
            self.journal.push(SiteSettingChange::Remove {
                pattern: key.pattern.to_string(),
                axis: key.axis.id().to_owned(),
                qualifier: key.qualifier.clone(),
            });
        }
        expired.len()
    }

    /// Records that `origin` used `axis` now, stamping the stored rule that answered.
    pub fn record_use(&mut self, origin: &Origin, axis: Axis, now: UnixMillis) {
        self.record_use_qualified(origin, axis, "", now);
    }

    /// Like [`record_use`](Self::record_use) for an axis with a qualifier.
    pub fn record_use_qualified(
        &mut self,
        origin: &Origin,
        axis: Axis,
        qualifier: &str,
        now: UnixMillis,
    ) {
        let Some(key) = self.winning_stored_key(origin, axis, qualifier, now) else {
            return;
        };
        let Some(record) = self
            .rules
            .get_mut(&key)
            .and_then(|slot| slot.persistent.as_mut())
        else {
            return;
        };
        if record
            .last_used_at
            .is_some_and(|used| now.saturating_sub(used) < LAST_USED_GRANULARITY_MS)
        {
            return;
        }
        record.last_used_at = Some(now);
        let row = row_for(&key, record);
        self.journal.push(SiteSettingChange::Put(row));
    }

    /// Sets whether unused-permission revocation skips the rule. Returns `false` when there is
    /// no persistent rule for the pattern, axis and empty qualifier.
    pub fn set_revoke_exempt(&mut self, pattern: &OriginPattern, axis: Axis, exempt: bool) -> bool {
        let key = RuleKey {
            axis,
            qualifier: String::new(),
            pattern: pattern.clone(),
        };
        let Some(record) = self
            .rules
            .get_mut(&key)
            .and_then(|slot| slot.persistent.as_mut())
        else {
            return false;
        };
        record.revoke_exempt = exempt;
        let row = row_for(&key, record);
        self.journal.push(SiteSettingChange::Put(row));
        true
    }

    /// The permanent allowed rules of revocable axes whose last use (or set time, if never used)
    /// is at least `older_than_ms` before `now`, and that are not exempt.
    pub fn unused_since(&self, now: UnixMillis, older_than_ms: i64) -> Vec<RuleView> {
        self.rules
            .iter()
            .filter_map(|(key, slot)| {
                let record = slot.persistent.as_ref()?;
                let idle_since = record.last_used_at.unwrap_or(record.set_at);
                let stale = now.saturating_sub(idle_since) >= older_than_ms;
                let eligible = key.axis.auto_revocable()
                    && record.lifetime == Lifetime::Permanent
                    && record.value == Value::Permission(Setting::Allow)
                    && !record.revoke_exempt
                    && record.is_live(now);
                (stale && eligible).then(|| view_of(key, record))
            })
            .collect()
    }

    /// Resets each rule [`unused_since`](Self::unused_since) lists to "ask", keeping its pattern,
    /// and returns the listed views as they were before the reset.
    pub fn revoke_unused(&mut self, now: UnixMillis, older_than_ms: i64) -> Vec<RuleView> {
        let listed = self.unused_since(now, older_than_ms);
        for item in &listed {
            let key = RuleKey {
                axis: item.axis,
                qualifier: item.qualifier.clone(),
                pattern: item.pattern.clone(),
            };
            let Some(record) = self
                .rules
                .get_mut(&key)
                .and_then(|slot| slot.persistent.as_mut())
            else {
                continue;
            };
            record.value = Value::Permission(Setting::Ask);
            record.set_at = now;
            let row = row_for(&key, record);
            self.journal.push(SiteSettingChange::Put(row));
        }
        listed
    }

    /// The key of the stored rule that answers `origin` for `axis` and `qualifier` at `now`, or
    /// `None` when the answer is an ephemeral rule or the built-in default.
    fn winning_stored_key(
        &self,
        origin: &Origin,
        axis: Axis,
        qualifier: &str,
        now: UnixMillis,
    ) -> Option<RuleKey> {
        let (key, _, _) = self.winning(Some(origin), axis, qualifier, now, true)?;
        let slot: &Slot = self.rules.get(key)?;
        if slot.ephemeral.as_ref().is_some_and(|r| r.is_live(now)) {
            return None;
        }
        Some(key.clone())
    }

    fn drop_if_empty(&mut self, key: &RuleKey) {
        let empty = self
            .rules
            .get(key)
            .is_some_and(|slot| slot.persistent.is_none() && slot.ephemeral.is_none());
        if empty {
            self.rules.remove(key);
        }
    }
}
