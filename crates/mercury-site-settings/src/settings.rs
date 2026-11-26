// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The rules, how a question about a site is answered, and the journal of what to write.
//!
//! The model is in memory. Every mutation that must reach the profile database pushes a
//! [`SiteSettingChange`] on a journal; the owner drains it with
//! [`take_changes`](SiteSettings::take_changes) and has the profile-data worker apply it.

use std::collections::BTreeMap;
use std::fmt;

use mercury_store::{SiteSettingChange, SiteSettingRow, SiteSettingsSnapshot};

use crate::handlers::HandlerTable;
use crate::pattern::Specificity;
use crate::{
    AutoplayPolicy, Axis, DataRule, Lifetime, MAX_RULES, Origin, OriginPattern, Setting,
    UnixMillis, Value,
};

/// Why a change was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The axis does not take this value (a zoom for the camera, "ask" for pop-ups, a zoom
    /// outside 30 to 500 percent, or [`DataRule::FollowGlobal`], which is the absence of a rule).
    ValueDoesNotFit,
    /// A qualifier was given for an axis that has none.
    QualifierNotAllowed,
    /// The axis needs a qualifier (a URL scheme for `external_links`).
    QualifierRequired,
    /// The qualifier is not a lower-case URL scheme.
    InvalidQualifier,
    /// The model already holds [`MAX_RULES`] rules.
    TooManyRules,
    /// The lifetime ends at or before now.
    LifetimeEnded,
    /// The scheme cannot have a handler registered (see `is_registrable_scheme`).
    SchemeNotAllowed,
    /// The handler's URL template is not acceptable.
    TemplateNotAllowed,
    /// The model already holds `MAX_HANDLERS` handlers.
    TooManyHandlers,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ValueDoesNotFit => "that value does not fit this setting",
            Self::QualifierNotAllowed => "this setting has no qualifier",
            Self::QualifierRequired => "this setting needs a qualifier",
            Self::InvalidQualifier => "the qualifier is not a lower-case URL scheme",
            Self::TooManyRules => "too many site settings",
            Self::LifetimeEnded => "the lifetime has already ended",
            Self::SchemeNotAllowed => "a handler cannot be registered for this scheme",
            Self::TemplateNotAllowed => "the handler's URL is not acceptable",
            Self::TooManyHandlers => "too many protocol handlers",
        })
    }
}

impl std::error::Error for Error {}

/// A rule as the interface reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleView {
    /// What the rule is about.
    pub pattern: OriginPattern,
    /// What it decides.
    pub axis: Axis,
    /// The URL scheme of an `external_links` rule; empty for every other axis.
    pub qualifier: String,
    /// The decision.
    pub value: Value,
    /// How long it lasts.
    pub lifetime: Lifetime,
    /// When it was made (or last changed).
    pub set_at: UnixMillis,
    /// When the site last used what the rule allowed, if recorded.
    pub last_used_at: Option<UnixMillis>,
    /// Whether auto-revoke leaves the rule alone.
    pub revoke_exempt: bool,
}

/// Where an answer came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A rule: a site's own, or the user's stored default (pattern [`OriginPattern::AllSites`]).
    Rule(RuleView),
    /// No rule: the built-in default of the axis.
    Builtin,
}

/// An answer and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// What applies.
    pub value: Value,
    /// Why.
    pub source: Source,
}

/// What [`SiteSettings::load`] did with the stored rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// Rules now in the model.
    pub loaded: usize,
    /// Timed rules that had ended; their removal is in the journal.
    pub expired: usize,
    /// Rows this build cannot use (unknown axis, unreadable pattern or value, over the cap).
    /// They stay in the file untouched.
    pub ignored: usize,
}

/// The key of a rule: the axis first, so that one axis's rules are adjacent.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RuleKey {
    pub(crate) axis: Axis,
    pub(crate) qualifier: String,
    pub(crate) pattern: OriginPattern,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) value: Value,
    pub(crate) lifetime: Lifetime,
    pub(crate) set_at: UnixMillis,
    pub(crate) last_used_at: Option<UnixMillis>,
    pub(crate) revoke_exempt: bool,
}

impl Record {
    /// Whether the rule applies at `now`. A timed rule applies from `set_at` up to, not
    /// including, its end: a clock moved back pauses it rather than extending it.
    pub(crate) fn is_live(&self, now: UnixMillis) -> bool {
        match self.lifetime {
            Lifetime::Until(end) => self.set_at <= now && now < end,
            Lifetime::Permanent | Lifetime::ThisVisit | Lifetime::Session => true,
        }
    }
}

/// A key's records: at most one that is stored (`Until`, `Permanent`) and one that is not
/// (`ThisVisit`, `Session`). The second wins while it exists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Slot {
    pub(crate) persistent: Option<Record>,
    pub(crate) ephemeral: Option<Record>,
}

impl Slot {
    pub(crate) fn live(&self, now: UnixMillis) -> Option<&Record> {
        self.ephemeral.as_ref().or_else(|| {
            self.persistent
                .as_ref()
                .filter(|record| record.is_live(now))
        })
    }
}

pub(crate) fn row_for(key: &RuleKey, record: &Record) -> SiteSettingRow {
    SiteSettingRow {
        pattern: key.pattern.to_string(),
        axis: key.axis.id().to_owned(),
        qualifier: key.qualifier.clone(),
        value: record.value.encode(),
        set_at: record.set_at,
        expires_at: match record.lifetime {
            Lifetime::Until(end) => Some(end),
            _ => None,
        },
        last_used_at: record.last_used_at,
        revoke_exempt: record.revoke_exempt,
    }
}

pub(crate) fn view_of(key: &RuleKey, record: &Record) -> RuleView {
    RuleView {
        pattern: key.pattern.clone(),
        axis: key.axis,
        qualifier: key.qualifier.clone(),
        value: record.value,
        lifetime: record.lifetime,
        set_at: record.set_at,
        last_used_at: record.last_used_at,
        revoke_exempt: record.revoke_exempt,
    }
}

/// A qualifier is valid for the axis: none for a plain axis, a lower-case URL scheme for
/// `external_links`.
pub(crate) fn check_qualifier(axis: Axis, qualifier: &str) -> Result<(), Error> {
    if !axis.takes_qualifier() {
        return if qualifier.is_empty() {
            Ok(())
        } else {
            Err(Error::QualifierNotAllowed)
        };
    }
    if qualifier.is_empty() {
        return Err(Error::QualifierRequired);
    }
    let mut bytes = qualifier.bytes();
    let starts_with_letter = bytes.next().is_some_and(|b| b.is_ascii_lowercase());
    let rest_ok = bytes
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.'));
    if starts_with_letter && rest_ok && qualifier.len() <= 32 {
        Ok(())
    } else {
        Err(Error::InvalidQualifier)
    }
}

/// Every per-site decision of the profile. See the crate documentation.
#[derive(Debug, Default)]
pub struct SiteSettings {
    pub(crate) rules: BTreeMap<RuleKey, Slot>,
    pub(crate) handlers: HandlerTable,
    pub(crate) journal: Vec<SiteSettingChange>,
    pub(crate) meta: BTreeMap<String, String>,
    pub(crate) unknown_rows: usize,
}

impl SiteSettings {
    /// A model with no rules (and nothing to write).
    pub fn new() -> Self {
        Self::default()
    }

    /// How many protocol handlers the model holds.
    pub fn handler_count(&self) -> usize {
        self.handlers.len()
    }

    /// A stored meta value, if there is one.
    pub fn meta(&self, key: &str) -> Option<&str> {
        self.meta.get(key).map(String::as_str)
    }

    /// Builds the model from the rows the store holds. Rows this build cannot use are ignored
    /// (and counted), never turned into rules; timed rules that ended are dropped and their
    /// removal is journaled.
    pub fn load(snapshot: SiteSettingsSnapshot, now: UnixMillis) -> (Self, LoadReport) {
        let mut model = Self::new();
        let mut report = LoadReport::default();
        for row in snapshot.rows {
            match model.load_row(&row, now) {
                LoadedRow::Loaded => report.loaded += 1,
                LoadedRow::Expired => report.expired += 1,
                LoadedRow::Ignored => report.ignored += 1,
            }
        }
        let (handlers, ignored_handlers) = HandlerTable::from_rows(snapshot.handlers);
        model.handlers = handlers;
        model.meta = snapshot.meta.into_iter().collect();
        model.unknown_rows = report.ignored;
        report.ignored += ignored_handlers;
        (model, report)
    }

    fn load_row(&mut self, row: &SiteSettingRow, now: UnixMillis) -> LoadedRow {
        let Some(axis) = Axis::parse(&row.axis) else {
            return LoadedRow::Ignored;
        };
        let Ok(pattern) = OriginPattern::parse(&row.pattern) else {
            return LoadedRow::Ignored;
        };
        let Some(value) = Value::decode(axis, &row.value) else {
            return LoadedRow::Ignored;
        };
        if value == Value::Data(DataRule::FollowGlobal)
            || check_qualifier(axis, &row.qualifier).is_err()
        {
            return LoadedRow::Ignored;
        }
        let key = RuleKey {
            axis,
            qualifier: row.qualifier.clone(),
            pattern,
        };
        if self.rules.contains_key(&key) || self.rules.len() >= MAX_RULES {
            return LoadedRow::Ignored;
        }
        let lifetime = match row.expires_at {
            Some(end) => Lifetime::Until(end),
            None => Lifetime::Permanent,
        };
        if matches!(lifetime, Lifetime::Until(end) if end <= now) {
            self.journal.push(SiteSettingChange::Remove {
                pattern: row.pattern.clone(),
                axis: row.axis.clone(),
                qualifier: row.qualifier.clone(),
            });
            return LoadedRow::Expired;
        }
        let record = Record {
            value,
            lifetime,
            set_at: row.set_at,
            last_used_at: row.last_used_at,
            revoke_exempt: row.revoke_exempt,
        };
        self.rules.insert(
            key,
            Slot {
                persistent: Some(record),
                ephemeral: None,
            },
        );
        LoadedRow::Loaded
    }

    // ---- queries ----

    /// What applies to `origin` for an axis without a qualifier. `origin: None` is a page with
    /// no usable origin (`about:blank`, a file): only the defaults apply.
    pub fn resolve(&self, origin: Option<&Origin>, axis: Axis, now: UnixMillis) -> Resolved {
        self.resolve_qualified(origin, axis, "", now)
    }

    /// [`resolve`](Self::resolve) for an axis with a qualifier (`external_links` and a scheme).
    pub fn resolve_qualified(
        &self,
        origin: Option<&Origin>,
        axis: Axis,
        qualifier: &str,
        now: UnixMillis,
    ) -> Resolved {
        match self.winning(origin, axis, qualifier, now, true) {
            Some((key, record, _)) => Resolved {
                value: record.value,
                source: Source::Rule(view_of(key, record)),
            },
            None => Resolved {
                value: axis.builtin_default(),
                source: Source::Builtin,
            },
        }
    }

    /// The best live rule for the question. With `with_defaults` false the user's stored default
    /// (the `AllSites` rule) is left out.
    pub(crate) fn winning(
        &self,
        origin: Option<&Origin>,
        axis: Axis,
        qualifier: &str,
        now: UnixMillis,
        with_defaults: bool,
    ) -> Option<(&RuleKey, &Record, Specificity)> {
        let lower = RuleKey {
            axis,
            qualifier: qualifier.to_owned(),
            pattern: OriginPattern::AllSites,
        };
        let mut best: Option<(&RuleKey, &Record, Specificity)> = None;
        for (key, slot) in self.rules.range(lower..) {
            if key.axis != axis || key.qualifier != qualifier {
                break;
            }
            let specificity = match origin {
                Some(origin) => key.pattern.matches(origin),
                None => (key.pattern == OriginPattern::AllSites).then_some(Specificity::AllSites),
            };
            let Some(specificity) = specificity else {
                continue;
            };
            if !with_defaults && specificity == Specificity::AllSites {
                continue;
            }
            let Some(record) = slot.live(now) else {
                continue;
            };
            if best.is_none_or(|(_, _, current)| specificity > current) {
                best = Some((key, record, specificity));
            }
        }
        best
    }

    /// The answer for a [`Permission`](ValueKind::Permission) or [`Toggle`](ValueKind::Toggle)
    /// axis. Any other axis answers `Ask`.
    pub fn permission(&self, origin: Option<&Origin>, axis: Axis, now: UnixMillis) -> Setting {
        match self.resolve(origin, axis, now).value {
            Value::Permission(setting) => setting,
            _ => Setting::Ask,
        }
    }

    /// The zoom of a page of `origin`, in percent.
    pub fn zoom_percent(&self, origin: Option<&Origin>, now: UnixMillis) -> u16 {
        match self.resolve(origin, Axis::Zoom, now).value {
            Value::Zoom(percent) => percent,
            _ => 100,
        }
    }

    /// What may play by itself on `origin`.
    pub fn autoplay(&self, origin: Option<&Origin>, now: UnixMillis) -> AutoplayPolicy {
        match self.resolve(origin, Axis::Autoplay, now).value {
            Value::Autoplay(policy) => policy,
            _ => AutoplayPolicy::BlockAudible,
        }
    }

    /// What happens to the data of `origin`'s site.
    pub fn data_rule(&self, origin: Option<&Origin>, now: UnixMillis) -> DataRule {
        match self.resolve(origin, Axis::SiteData, now).value {
            Value::Data(rule) => rule,
            _ => DataRule::FollowGlobal,
        }
    }

    /// The rule that decides each axis (and qualifier) for `origin`, leaving out the defaults:
    /// the "remembered permissions" of the site-information popover. Sorted by axis id, then
    /// qualifier.
    pub fn effective_for_origin(&self, origin: &Origin, now: UnixMillis) -> Vec<RuleView> {
        let mut views = Vec::new();
        let mut last: Option<(Axis, &str)> = None;
        for key in self.rules.keys() {
            if last == Some((key.axis, key.qualifier.as_str())) {
                continue;
            }
            last = Some((key.axis, key.qualifier.as_str()));
            if let Some((key, record, _)) =
                self.winning(Some(origin), key.axis, &key.qualifier, now, false)
            {
                views.push(view_of(key, record));
            }
        }
        views.sort_by(|a, b| (a.axis.id(), &a.qualifier).cmp(&(b.axis.id(), &b.qualifier)));
        views
    }

    /// Every live rule, sorted by pattern, then axis id, then qualifier.
    pub fn rules(&self, now: UnixMillis) -> Vec<RuleView> {
        self.collect(now, |_| true)
    }

    /// The live rules of one axis, for its "allowed" and "blocked" lists; the user's default is
    /// among them (pattern `*`).
    pub fn rules_for_axis(&self, axis: Axis, now: UnixMillis) -> Vec<RuleView> {
        self.collect(now, |key| key.axis == axis)
    }

    /// The live rules of one pattern, for a site's own page.
    pub fn rules_for_pattern(&self, pattern: &OriginPattern, now: UnixMillis) -> Vec<RuleView> {
        self.collect(now, |key| &key.pattern == pattern)
    }

    fn collect(&self, now: UnixMillis, wanted: impl Fn(&RuleKey) -> bool) -> Vec<RuleView> {
        let mut views: Vec<RuleView> = self
            .rules
            .iter()
            .filter(|(key, _)| wanted(key))
            .filter_map(|(key, slot)| slot.live(now).map(|record| view_of(key, record)))
            .collect();
        views.sort_by(|a, b| {
            (&a.pattern, a.axis.id(), &a.qualifier).cmp(&(&b.pattern, b.axis.id(), &b.qualifier))
        });
        views
    }

    /// The default of an axis: the user's stored one, else the built-in.
    pub fn default_for(&self, axis: Axis, now: UnixMillis) -> Value {
        self.resolve(None, axis, now).value
    }

    // ---- mutations ----

    /// Sets a rule for an axis without a qualifier. A rule already at the key is replaced; a
    /// stored lifetime also ends a temporary rule at the key. See [`Lifetime`].
    pub fn set(
        &mut self,
        pattern: &OriginPattern,
        axis: Axis,
        value: Value,
        lifetime: Lifetime,
        now: UnixMillis,
    ) -> Result<(), Error> {
        self.set_qualified(pattern, axis, "", value, lifetime, now)
    }

    /// [`set`](Self::set) for an axis with a qualifier.
    pub fn set_qualified(
        &mut self,
        pattern: &OriginPattern,
        axis: Axis,
        qualifier: &str,
        value: Value,
        lifetime: Lifetime,
        now: UnixMillis,
    ) -> Result<(), Error> {
        if !value.allowed_for(axis) || value == Value::Data(DataRule::FollowGlobal) {
            return Err(Error::ValueDoesNotFit);
        }
        check_qualifier(axis, qualifier)?;
        if matches!(lifetime, Lifetime::Until(end) if end <= now) {
            return Err(Error::LifetimeEnded);
        }
        let key = RuleKey {
            axis,
            qualifier: qualifier.to_owned(),
            pattern: pattern.clone(),
        };
        if !self.rules.contains_key(&key) && self.rules.len() >= MAX_RULES {
            return Err(Error::TooManyRules);
        }
        let slot = self.rules.entry(key.clone()).or_default();
        let record = Record {
            value,
            lifetime,
            set_at: now,
            last_used_at: None,
            revoke_exempt: false,
        };
        if lifetime.is_persistent() {
            let record = Record {
                revoke_exempt: slot
                    .persistent
                    .as_ref()
                    .is_some_and(|old| old.revoke_exempt),
                ..record
            };
            self.journal
                .push(SiteSettingChange::Put(row_for(&key, &record)));
            slot.persistent = Some(record);
            slot.ephemeral = None;
        } else {
            slot.ephemeral = Some(record);
        }
        Ok(())
    }

    /// Sets the user's default for an axis (pattern `*`, permanent).
    pub fn set_default(&mut self, axis: Axis, value: Value, now: UnixMillis) -> Result<(), Error> {
        self.set(
            &OriginPattern::AllSites,
            axis,
            value,
            Lifetime::Permanent,
            now,
        )
    }

    /// Removes the rule at a key, whatever its lifetime. Returns whether there was one.
    pub fn remove(&mut self, pattern: &OriginPattern, axis: Axis, qualifier: &str) -> bool {
        let key = RuleKey {
            axis,
            qualifier: qualifier.to_owned(),
            pattern: pattern.clone(),
        };
        let Some(slot) = self.rules.remove(&key) else {
            return false;
        };
        if slot.persistent.is_some() {
            self.journal.push(SiteSettingChange::Remove {
                pattern: pattern.to_string(),
                axis: axis.id().to_owned(),
                qualifier: qualifier.to_owned(),
            });
        }
        true
    }

    /// Removes every rule of a pattern ("reset this site's permissions"), including rows this
    /// build does not understand. Returns how many rules were removed.
    pub fn reset_pattern(&mut self, pattern: &OriginPattern) -> usize {
        let keys: Vec<RuleKey> = self
            .rules
            .keys()
            .filter(|key| &key.pattern == pattern)
            .cloned()
            .collect();
        for key in &keys {
            self.rules.remove(key);
        }
        if !keys.is_empty() || self.unknown_rows > 0 {
            self.journal.push(SiteSettingChange::RemovePattern {
                pattern: pattern.to_string(),
            });
        }
        keys.len()
    }

    /// Removes every rule, including rows this build does not understand. Protocol handlers
    /// and the import marker stay.
    pub fn reset_all(&mut self) {
        self.rules.clear();
        self.unknown_rows = 0;
        self.journal.push(SiteSettingChange::RemoveAll);
    }

    // ---- persistence ----

    /// Takes what must be written to the store, oldest first.
    pub fn take_changes(&mut self) -> Vec<SiteSettingChange> {
        std::mem::take(&mut self.journal)
    }

    /// Puts changes the store failed to write back, ahead of newer ones.
    pub fn restore_changes(&mut self, mut changes: Vec<SiteSettingChange>) {
        changes.append(&mut self.journal);
        self.journal = changes;
    }

    /// Whether there is anything to write.
    pub fn has_pending_changes(&self) -> bool {
        !self.journal.is_empty()
    }
}

enum LoadedRow {
    Loaded,
    Expired,
    Ignored,
}
