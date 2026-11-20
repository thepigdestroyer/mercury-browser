// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Looking at a data file before it ships: [`check`] says whether a file is fit to ship,
//! [`guard_refresh`] is the same question asked of a freshly built list before it may replace the
//! file in the tree, and [`diff`] / [`render_diff`] show what a refresh changed.
//!
//! Servo fails open on a bad file (it logs one line and uses an empty list), so every way a refresh
//! can go wrong silently has a guard here that turns it into a refusal with a sentence a
//! maintainer can act on: a truncated download (too few entries), a renamed key in Chromium's file
//! (almost no entry includes subdomains, or the long-standing entries are gone), a list that
//! shrank without anyone meaning it.

use std::cmp::Ordering;

use super::fstmap::Preload;
use super::manifest::{Date, Manifest, sha256_hex};
use super::{CANARIES, DEFAULT_MIN_ENTRIES, Entry, MIN_INCLUDE_PERCENT};

/// The size limits a list must meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Fewer entries than this is a problem.
    pub min_entries: usize,
    /// A smaller share of entries (in whole percent) that include subdomains is a problem.
    pub min_include_percent: usize,
    /// A list retrieved more days ago than this is a problem; `None` means age never fails.
    pub max_age_days: Option<i64>,
}

impl Default for Limits {
    /// [`DEFAULT_MIN_ENTRIES`], [`MIN_INCLUDE_PERCENT`] and no age limit.
    fn default() -> Limits {
        Limits {
            min_entries: DEFAULT_MIN_ENTRIES,
            min_include_percent: MIN_INCLUDE_PERCENT,
            max_age_days: None,
        }
    }
}

/// The result of [`check`]: what is wrong, and what is worth knowing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Reasons the file must not ship. Any problem fails `check` (exit status 1).
    pub problems: Vec<String>,
    /// Facts and warnings that do not fail it.
    pub notes: Vec<String>,
}

impl Verdict {
    /// Whether there is no problem.
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// `12345` as `12,345`.
fn thousands(number: usize) -> String {
    let digits = number.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (position, digit) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// A count of tenths of a percent as `12.3`.
fn tenths(tenths_of_percent: u64) -> String {
    format!("{}.{}", tenths_of_percent / 10, tenths_of_percent % 10)
}

/// `part` of `whole` in percent with one decimal, rounded to nearest (`0.0` of nothing).
fn share_text(part: usize, whole: usize) -> String {
    if whole == 0 {
        return tenths(0);
    }
    let (part, whole) = (part as u64, whole as u64);
    tenths((part * 1_000 + whole / 2) / whole)
}

/// Whether fewer than `min_percent` percent of the entries include subdomains. Whole numbers
/// only, so that exactly the minimum passes and one entry fewer does not.
fn include_share_too_low(preload: &Preload, min_percent: usize) -> bool {
    (preload.include_subdomains_count() as u64) * 100
        < (min_percent as u64) * (preload.len() as u64)
}

fn days(count: i64) -> String {
    if count == 1 {
        "1 day".to_string()
    } else {
        format!("{count} days")
    }
}

/// One sentence for each canary host that is missing or listed without `include_subdomains`.
fn canary_problems(preload: &Preload) -> Vec<String> {
    let mut problems = Vec::new();
    for canary in CANARIES {
        match preload.get(canary) {
            Some(true) => {}
            Some(false) => problems.push(format!(
                "the long-standing entry {canary} is listed without include_subdomains (a list \
                 that lost its flag was parsed wrongly or damaged)"
            )),
            None => problems.push(format!(
                "the long-standing entry {canary} is missing (a list without it was truncated \
                 or parsed wrongly)"
            )),
        }
    }
    problems
}

/// Checks the bytes of a data file the way a maintainer wants it checked before a release.
///
/// A file that cannot be opened (not an `fst` map, no checksum, a damaged byte, a key that is not
/// a plain lower-case host name) gives exactly one problem and nothing else, because nothing
/// further can be said about it. Otherwise the **problems** are, in this order: the values do
/// not follow `2 * rank + include_subdomains`; fewer entries than `limits.min_entries`; a smaller
/// include-subdomains share than `limits.min_include_percent`; a canary host missing or without
/// the flag; the manifest (when given) disagreeing with the file about the entries, the
/// include-subdomains count or the SHA-256; and, when `limits.max_age_days` is set, a list that is
/// older than that or whose age is unknown (a limit that cannot be shown to be met is not met).
///
/// The **notes** are the counts, the SHA-256, the age ("N days old", "age unknown", or a warning
/// that the retrieved date is in the future) and, when the file is not what `build` would write
/// from its own entries, that it is not byte-identical to a rebuild (valid for Servo all the same).
pub fn check(bytes: &[u8], manifest: Option<&Manifest>, today: Date, limits: &Limits) -> Verdict {
    let preload = match Preload::open(bytes.to_vec()) {
        Ok(preload) => preload,
        Err(error) => {
            return Verdict {
                problems: vec![format!(
                    "the file cannot be used as an HSTS preload list: {error}"
                )],
                notes: Vec::new(),
            };
        }
    };
    let mut verdict = Verdict::default();
    let entries = preload.len();
    let included = preload.include_subdomains_count();
    let sha256 = sha256_hex(bytes);

    // Problems about the data itself.
    if let Some(violation) = preload.id_scheme_violation() {
        verdict.problems.push(format!(
            "the values do not follow the scheme this tool writes: {violation}"
        ));
    }
    if entries < limits.min_entries {
        verdict.problems.push(format!(
            "the list has {} entries, fewer than the minimum of {}",
            thousands(entries),
            thousands(limits.min_entries)
        ));
    }
    if include_share_too_low(&preload, limits.min_include_percent) {
        verdict.problems.push(format!(
            "only {} % of the entries include subdomains, less than the minimum of {} %",
            share_text(included, entries),
            limits.min_include_percent
        ));
    }
    verdict.problems.extend(canary_problems(&preload));

    // Problems about the manifest that goes with it.
    if let Some(manifest) = manifest {
        if manifest.entries != entries {
            verdict.problems.push(format!(
                "the manifest says the list has {} entries, but the file has {}",
                thousands(manifest.entries),
                thousands(entries)
            ));
        }
        if manifest.include_subdomains != included {
            verdict.problems.push(format!(
                "the manifest says {} entries include subdomains, but the file has {}",
                thousands(manifest.include_subdomains),
                thousands(included)
            ));
        }
        if !manifest.output_sha256.eq_ignore_ascii_case(&sha256) {
            verdict.problems.push(format!(
                "the manifest's output_sha256 ({}) is not the SHA-256 of the file ({sha256})",
                manifest.output_sha256
            ));
        }
    }

    // Notes.
    verdict.notes.push(format!(
        "{} entries, {} of them include subdomains ({} %)",
        thousands(entries),
        thousands(included),
        share_text(included, entries)
    ));
    verdict.notes.push(format!("SHA-256 {sha256}"));
    match manifest.and_then(|manifest| manifest.source.retrieved) {
        Some(retrieved) => {
            let age = today.days_since(retrieved);
            if age < 0 {
                verdict.notes.push(format!(
                    "retrieved date is in the future ({retrieved}, today is {today}), so the age \
                     cannot be trusted"
                ));
            } else {
                verdict
                    .notes
                    .push(format!("{} old (retrieved {retrieved})", days(age)));
            }
            if let Some(limit) = limits.max_age_days
                && age > limit
            {
                verdict.problems.push(format!(
                    "the list was retrieved {retrieved}, {} ago, longer than the limit of {}",
                    days(age),
                    days(limit)
                ));
            }
        }
        None => {
            verdict.notes.push(if manifest.is_some() {
                "age unknown (the manifest records no retrieval date)".to_string()
            } else {
                "age unknown (no manifest)".to_string()
            });
            if let Some(limit) = limits.max_age_days {
                verdict.problems.push(format!(
                    "the age of the list is unknown, so it cannot be shown to be within the limit \
                     of {}",
                    days(limit)
                ));
            }
        }
    }
    if !preload.rebuilds_identically(bytes) {
        verdict.notes.push(
            "not byte-identical to a rebuild from its own entries (Servo reads it all the same: \
             it only needs the parity of each value)"
                .to_string(),
        );
    }
    verdict
}

/// The rule `build` applies before it may replace a data file: the first of these that `new`
/// breaks is returned as a sentence with the numbers and the way out. In this order: fewer entries
/// than `limits.min_entries`; a smaller include-subdomains share than `limits.min_include_percent`;
/// a canary host missing or without the flag; and, when `old` is the file being replaced, a list
/// that is more than `max_shrink_percent` percent smaller than `old` (exactly that share passes,
/// one entry more fails). `allow_shrink` waives the last rule and nothing else. `limits.max_age_days`
/// plays no part: a list built just now is as old as the input says.
pub fn guard_refresh(
    old: Option<&Preload>,
    new: &Preload,
    limits: &Limits,
    max_shrink_percent: usize,
    allow_shrink: bool,
) -> Result<(), String> {
    if new.len() < limits.min_entries {
        return Err(format!(
            "the new list has only {} entries, fewer than the minimum of {}; the input may be \
             truncated or Chromium's file may have changed shape (pass --min-entries to lower \
             the minimum only if that is intended)",
            thousands(new.len()),
            thousands(limits.min_entries)
        ));
    }
    if include_share_too_low(new, limits.min_include_percent) {
        return Err(format!(
            "only {} % of the new list's entries include subdomains, less than the minimum of \
             {} %; Chromium's file may have changed shape (for example a renamed \
             include_subdomains key), so the input should be looked at before it is used",
            share_text(new.include_subdomains_count(), new.len()),
            limits.min_include_percent
        ));
    }
    if let Some(problem) = canary_problems(new).into_iter().next() {
        return Err(format!("{problem}; look at the input before using it"));
    }
    if let Some(old) = old
        && !allow_shrink
    {
        let (old_len, new_len) = (old.len() as u64, new.len() as u64);
        let lost = old_len.saturating_sub(new_len);
        if lost * 100 > (max_shrink_percent as u64) * old_len {
            // Rounded up, so that the percentage shown is always above the limit it broke.
            let lost_tenths = (lost * 1_000).div_ceil(old_len);
            return Err(format!(
                "the new list has {} % fewer entries ({} instead of {}) than the file it would \
                 replace (limit {max_shrink_percent} %); pass --allow-shrink if that is intended",
                tenths(lost_tenths),
                thousands(new.len()),
                thousands(old.len())
            ));
        }
    }
    Ok(())
}

/// What changed between two lists, each part in key order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// Hosts in the new list that the old one did not have.
    pub added: Vec<Entry>,
    /// Hosts the old list had that the new one lacks (as they were in the old list).
    pub removed: Vec<Entry>,
    /// Hosts in both lists whose include-subdomains flag differs (as they are in the new list).
    pub flag_changed: Vec<Entry>,
}

impl Diff {
    /// Whether the two lists have the same hosts with the same flags.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.flag_changed.is_empty()
    }
}

/// Compares two lists in one walk over both in key order.
pub fn diff(old: &Preload, new: &Preload) -> Diff {
    let old = old.entries();
    let new = new.entries();
    let mut result = Diff::default();
    let (mut from_old, mut from_new) = (0, 0);
    while from_old < old.len() && from_new < new.len() {
        let (before, after) = (&old[from_old], &new[from_new]);
        match before.host.cmp(&after.host) {
            Ordering::Less => {
                result.removed.push(before.clone());
                from_old += 1;
            }
            Ordering::Greater => {
                result.added.push(after.clone());
                from_new += 1;
            }
            Ordering::Equal => {
                if before.include_subdomains != after.include_subdomains {
                    result.flag_changed.push(after.clone());
                }
                from_old += 1;
                from_new += 1;
            }
        }
    }
    result.removed.extend_from_slice(&old[from_old..]);
    result.added.extend_from_slice(&new[from_new..]);
    result
}

/// The diff as text: the line `added 12 / removed 3 / changed 1`, then each kind in turn
/// (added `+`, removed `-`, changed `~`) with up to `limit` lines (all of them when `None`),
/// then `... and N more added` (or `removed`, `changed`) for the rest. Every line ends with a
/// newline.
pub fn render_diff(diff: &Diff, limit: Option<usize>) -> String {
    let mut text = format!(
        "added {} / removed {} / changed {}\n",
        diff.added.len(),
        diff.removed.len(),
        diff.flag_changed.len()
    );
    render_section(&mut text, &diff.added, limit, "added", |entry| {
        let flag = if entry.include_subdomains {
            "includes subdomains"
        } else {
            "host only"
        };
        format!("+ {} ({flag})", entry.host)
    });
    render_section(&mut text, &diff.removed, limit, "removed", |entry| {
        let flag = if entry.include_subdomains {
            "included subdomains"
        } else {
            "host only"
        };
        format!("- {} ({flag})", entry.host)
    });
    render_section(&mut text, &diff.flag_changed, limit, "changed", |entry| {
        let flag = if entry.include_subdomains {
            "now includes subdomains"
        } else {
            "now host only"
        };
        format!("~ {} ({flag})", entry.host)
    });
    text
}

fn render_section(
    text: &mut String,
    entries: &[Entry],
    limit: Option<usize>,
    word: &str,
    line: impl Fn(&Entry) -> String,
) {
    let shown = limit.map_or(entries.len(), |limit| limit.min(entries.len()));
    for entry in &entries[..shown] {
        text.push_str(&line(entry));
        text.push('\n');
    }
    if shown < entries.len() {
        text.push_str(&format!("... and {} more {word}\n", entries.len() - shown));
    }
}
