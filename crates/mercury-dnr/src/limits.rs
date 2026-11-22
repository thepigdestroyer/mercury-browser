// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The limits on rule counts, regex size and rule size, and the checks against them.
//!
//! The rule-count numbers are Chrome's (<https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest>,
//! "Rule limits"); the size caps are Mercury's own. Everything is in one [`Limits`] struct so a caller (or
//! a test) can lower it. "Safe" rules are `block`, `allow`, `allowAllRequests` and `upgradeScheme`; the
//! "unsafe" ones (`redirect`, `modifyHeaders`) have a smaller dynamic quota (5,000) that also counts
//! against the 30,000 total.
//!
//! Static and dynamic counts are combined by the caller: [`RulesetStats`] describes one ruleset and
//! [`check_extension_budget`] checks everything an extension has enabled.

use std::fmt;

use thiserror::Error;

/// Rules every extension may enable across its static rulesets (`GUARANTEED_MINIMUM_STATIC_RULES`).
pub const GUARANTEED_STATIC_RULES: usize = 30_000;
/// Rules the browser lets all extensions enable together on top of the guarantees.
pub const GLOBAL_STATIC_RULES: usize = 330_000;
/// Static rulesets an extension may enable at once (`MAX_NUMBER_OF_ENABLED_STATIC_RULESETS`).
pub const MAX_ENABLED_STATIC_RULESETS: usize = 50;
/// Static rulesets an extension may declare.
pub const MAX_STATIC_RULESETS: usize = 100;
/// Dynamic rules (`MAX_NUMBER_OF_DYNAMIC_RULES`).
pub const MAX_DYNAMIC_RULES: usize = 30_000;
/// Dynamic rules that are not safe (`MAX_NUMBER_OF_UNSAFE_DYNAMIC_RULES`).
pub const MAX_UNSAFE_DYNAMIC_RULES: usize = 5_000;
/// Session rules (`MAX_NUMBER_OF_SESSION_RULES`).
pub const MAX_SESSION_RULES: usize = 5_000;
/// Regular-expression rules per kind of ruleset (`MAX_NUMBER_OF_REGEX_RULES`).
pub const MAX_REGEX_RULES: usize = 1_000;

/// Every limit this crate enforces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// See [`GUARANTEED_STATIC_RULES`].
    pub guaranteed_static_rules: usize,
    /// See [`GLOBAL_STATIC_RULES`].
    pub global_static_rules: usize,
    /// See [`MAX_ENABLED_STATIC_RULESETS`].
    pub max_enabled_static_rulesets: usize,
    /// See [`MAX_STATIC_RULESETS`].
    pub max_static_rulesets: usize,
    /// See [`MAX_DYNAMIC_RULES`].
    pub max_dynamic_rules: usize,
    /// See [`MAX_UNSAFE_DYNAMIC_RULES`].
    pub max_unsafe_dynamic_rules: usize,
    /// See [`MAX_SESSION_RULES`].
    pub max_session_rules: usize,
    /// See [`MAX_REGEX_RULES`].
    pub max_regex_rules: usize,
    /// The JSON of one ruleset, in bytes (Mercury's cap: 16 MiB).
    pub max_ruleset_bytes: usize,
    /// One rule once serialised, in bytes (Mercury's cap: 16 KiB).
    pub max_rule_bytes: usize,
    /// Entries in one list of domains, types, methods or tab ids (Mercury's cap: 10,000).
    pub max_list_entries: usize,
    /// Bytes in a `urlFilter` or `regexFilter` (Mercury's cap: 2,048).
    pub max_filter_bytes: usize,
    /// The compiled size of one regex, in bytes (Mercury's cap: 2 MiB); handed to
    /// `RegexBuilder::size_limit`.
    pub regex_size_limit: usize,
    /// The lazy-DFA cache of one regex, in bytes (Mercury's cap: 2 MiB); handed to
    /// `RegexBuilder::dfa_size_limit`.
    pub regex_dfa_size_limit: usize,
    /// Header operations (request and response together) in one rule (Mercury's cap: 100).
    pub max_header_operations: usize,
    /// Nesting depth of the JSON (Mercury's cap: 64).
    pub max_json_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            guaranteed_static_rules: GUARANTEED_STATIC_RULES,
            global_static_rules: GLOBAL_STATIC_RULES,
            max_enabled_static_rulesets: MAX_ENABLED_STATIC_RULESETS,
            max_static_rulesets: MAX_STATIC_RULESETS,
            max_dynamic_rules: MAX_DYNAMIC_RULES,
            max_unsafe_dynamic_rules: MAX_UNSAFE_DYNAMIC_RULES,
            max_session_rules: MAX_SESSION_RULES,
            max_regex_rules: MAX_REGEX_RULES,
            max_ruleset_bytes: 16 * 1024 * 1024,
            max_rule_bytes: 16 * 1024,
            max_list_entries: 10_000,
            max_filter_bytes: 2_048,
            regex_size_limit: 2 * 1024 * 1024,
            regex_dfa_size_limit: 2 * 1024 * 1024,
            max_header_operations: 100,
            max_json_depth: 64,
        }
    }
}

/// What a count in [`LimitError::TooManyRules`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountedItem {
    /// Static rulesets declared by the extension.
    StaticRulesets,
    /// Static rulesets enabled at once.
    EnabledStaticRulesets,
    /// Rules in the enabled static rulesets (or in one static ruleset, against the global pool).
    StaticRules,
    /// Regex rules in the enabled static rulesets.
    StaticRegexRules,
    /// Dynamic rules.
    DynamicRules,
    /// Dynamic rules that are not safe.
    UnsafeDynamicRules,
    /// Regex rules in the dynamic ruleset.
    DynamicRegexRules,
    /// Session rules.
    SessionRules,
    /// Regex rules in the session ruleset.
    SessionRegexRules,
}

impl fmt::Display for CountedItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CountedItem::StaticRulesets => "static rulesets",
            CountedItem::EnabledStaticRulesets => "enabled static rulesets",
            CountedItem::StaticRules => "static rules",
            CountedItem::StaticRegexRules => "static regex rules",
            CountedItem::DynamicRules => "dynamic rules",
            CountedItem::UnsafeDynamicRules => "unsafe dynamic rules",
            CountedItem::DynamicRegexRules => "dynamic regex rules",
            CountedItem::SessionRules => "session rules",
            CountedItem::SessionRegexRules => "session regex rules",
        })
    }
}

/// A limit was exceeded. No variant carries the offending content, only sizes and names.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LimitError {
    /// More rules (or rulesets) than allowed.
    #[error("too many {item}: {found} (limit {limit})")]
    TooManyRules {
        /// What was counted.
        item: CountedItem,
        /// The limit.
        limit: usize,
        /// The count found.
        found: usize,
    },
    /// The ruleset JSON is too large.
    #[error("ruleset is {found} bytes (limit {limit})")]
    RulesetTooLarge {
        /// The limit in bytes.
        limit: usize,
        /// The size found.
        found: usize,
    },
    /// One rule is too large once serialised.
    #[error("a rule is {found} bytes serialised (limit {limit})")]
    RuleTooLarge {
        /// The limit in bytes.
        limit: usize,
        /// The size found.
        found: usize,
    },
    /// A list has too many entries.
    #[error("{field} has {found} entries (limit {limit})")]
    ListTooLong {
        /// The JSON field.
        field: &'static str,
        /// The limit.
        limit: usize,
        /// The length found.
        found: usize,
    },
    /// A `urlFilter` or `regexFilter` is too long.
    #[error("{field} is {found} bytes (limit {limit})")]
    FilterTooLong {
        /// The JSON field.
        field: &'static str,
        /// The limit in bytes.
        limit: usize,
        /// The length found.
        found: usize,
    },
    /// A rule has too many header operations.
    #[error("{found} header operations in one rule (limit {limit})")]
    TooManyHeaderOperations {
        /// The limit.
        limit: usize,
        /// The count found.
        found: usize,
    },
    /// The JSON is nested too deeply.
    #[error("JSON nested deeper than {limit}")]
    NestingTooDeep {
        /// The limit.
        limit: usize,
    },
}

/// Counts for one parsed ruleset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RulesetStats {
    /// All rules.
    pub rules: usize,
    /// Rules that are not safe (`redirect`, `modifyHeaders`).
    pub unsafe_rules: usize,
    /// Rules with a `regexFilter`.
    pub regex_rules: usize,
}

/// Everything an extension has enabled, for [`check_extension_budget`].
#[derive(Debug, Clone, Copy)]
pub struct ExtensionBudget<'a> {
    /// Static rulesets the manifest declares (enabled or not).
    pub declared_static_rulesets: usize,
    /// Stats of each enabled static ruleset.
    pub enabled_static: &'a [RulesetStats],
    /// Stats of the dynamic ruleset.
    pub dynamic: RulesetStats,
    /// Stats of the session ruleset.
    pub session: RulesetStats,
    /// Rules this extension may enable beyond [`Limits::guaranteed_static_rules`], taken from the global
    /// pool (see [`global_pool_remaining`]); `0` when the pool is exhausted.
    pub extra_static_rules: usize,
}

/// Checks everything an extension has enabled against the limits. The first failing limit is returned.
pub fn check_extension_budget(
    limits: &Limits,
    budget: &ExtensionBudget<'_>,
) -> Result<(), LimitError> {
    check_count(
        CountedItem::StaticRulesets,
        limits.max_static_rulesets,
        budget.declared_static_rulesets,
    )?;
    check_count(
        CountedItem::EnabledStaticRulesets,
        limits.max_enabled_static_rulesets,
        budget.enabled_static.len(),
    )?;
    let static_rules = budget
        .enabled_static
        .iter()
        .fold(0usize, |acc, s| acc.saturating_add(s.rules));
    check_count(
        CountedItem::StaticRules,
        limits
            .guaranteed_static_rules
            .saturating_add(budget.extra_static_rules),
        static_rules,
    )?;
    let static_regex = budget
        .enabled_static
        .iter()
        .fold(0usize, |acc, s| acc.saturating_add(s.regex_rules));
    check_count(
        CountedItem::StaticRegexRules,
        limits.max_regex_rules,
        static_regex,
    )?;
    check_count(
        CountedItem::DynamicRules,
        limits.max_dynamic_rules,
        budget.dynamic.rules,
    )?;
    check_count(
        CountedItem::UnsafeDynamicRules,
        limits.max_unsafe_dynamic_rules,
        budget.dynamic.unsafe_rules,
    )?;
    check_count(
        CountedItem::DynamicRegexRules,
        limits.max_regex_rules,
        budget.dynamic.regex_rules,
    )?;
    check_count(
        CountedItem::SessionRules,
        limits.max_session_rules,
        budget.session.rules,
    )?;
    check_count(
        CountedItem::SessionRegexRules,
        limits.max_regex_rules,
        budget.session.regex_rules,
    )?;
    Ok(())
}

/// How many rules are left in the global static pool when all extensions together use `in_use` rules
/// beyond their guarantees.
pub fn global_pool_remaining(limits: &Limits, in_use: usize) -> usize {
    limits.global_static_rules.saturating_sub(in_use)
}

/// `Ok` when `found <= limit`.
pub(crate) fn check_count(item: CountedItem, limit: usize, found: usize) -> Result<(), LimitError> {
    if found > limit {
        Err(LimitError::TooManyRules { item, limit, found })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(rules: usize) -> RulesetStats {
        RulesetStats {
            rules,
            ..RulesetStats::default()
        }
    }

    fn budget<'a>(enabled: &'a [RulesetStats]) -> ExtensionBudget<'a> {
        ExtensionBudget {
            declared_static_rulesets: enabled.len(),
            enabled_static: enabled,
            dynamic: RulesetStats::default(),
            session: RulesetStats::default(),
            extra_static_rules: 0,
        }
    }

    #[test]
    fn defaults_are_chromes_numbers() {
        let l = Limits::default();
        assert_eq!(l.guaranteed_static_rules, 30_000);
        assert_eq!(l.global_static_rules, 330_000);
        assert_eq!(l.max_enabled_static_rulesets, 50);
        assert_eq!(l.max_static_rulesets, 100);
        assert_eq!(l.max_dynamic_rules, 30_000);
        assert_eq!(l.max_unsafe_dynamic_rules, 5_000);
        assert_eq!(l.max_session_rules, 5_000);
        assert_eq!(l.max_regex_rules, 1_000);
    }

    #[test]
    fn defaults_are_mercurys_caps() {
        let l = Limits::default();
        assert_eq!(l.max_ruleset_bytes, 16 * 1024 * 1024);
        assert_eq!(l.max_rule_bytes, 16 * 1024);
        assert_eq!(l.max_list_entries, 10_000);
        assert_eq!(l.max_filter_bytes, 2_048);
        assert_eq!(l.regex_size_limit, 2 * 1024 * 1024);
        assert_eq!(l.regex_dfa_size_limit, 2 * 1024 * 1024);
        assert_eq!(l.max_header_operations, 100);
        assert_eq!(l.max_json_depth, 64);
    }

    #[test]
    fn static_rules_at_the_guarantee_pass_and_one_over_fails() {
        let l = Limits::default();
        assert!(check_extension_budget(&l, &budget(&[stats(30_000)])).is_ok());
        assert_eq!(
            check_extension_budget(&l, &budget(&[stats(30_001)])),
            Err(LimitError::TooManyRules {
                item: CountedItem::StaticRules,
                limit: 30_000,
                found: 30_001
            })
        );
    }

    #[test]
    fn static_rules_sum_across_rulesets() {
        let l = Limits::default();
        assert!(check_extension_budget(&l, &budget(&[stats(15_000), stats(15_000)])).is_ok());
        assert!(check_extension_budget(&l, &budget(&[stats(15_000), stats(15_001)])).is_err());
    }

    #[test]
    fn extra_static_rules_from_the_global_pool_raise_the_static_cap() {
        let l = Limits::default();
        let enabled = [stats(40_000)];
        let mut b = budget(&enabled);
        assert!(check_extension_budget(&l, &b).is_err());
        b.extra_static_rules = 10_000;
        assert!(check_extension_budget(&l, &b).is_ok());
        b.extra_static_rules = 9_999;
        assert!(check_extension_budget(&l, &b).is_err());
    }

    #[test]
    fn enabled_static_rulesets_edge() {
        let l = Limits::default();
        let fifty = vec![stats(1); 50];
        assert!(check_extension_budget(&l, &budget(&fifty)).is_ok());
        let fifty_one = vec![stats(1); 51];
        assert_eq!(
            check_extension_budget(&l, &budget(&fifty_one)),
            Err(LimitError::TooManyRules {
                item: CountedItem::EnabledStaticRulesets,
                limit: 50,
                found: 51
            })
        );
    }

    #[test]
    fn declared_static_rulesets_edge() {
        let l = Limits::default();
        let mut b = budget(&[]);
        b.declared_static_rulesets = 100;
        assert!(check_extension_budget(&l, &b).is_ok());
        b.declared_static_rulesets = 101;
        assert_eq!(
            check_extension_budget(&l, &b),
            Err(LimitError::TooManyRules {
                item: CountedItem::StaticRulesets,
                limit: 100,
                found: 101
            })
        );
    }

    #[test]
    fn static_regex_rules_edge() {
        let l = Limits::default();
        let ok = [RulesetStats {
            rules: 1_000,
            unsafe_rules: 0,
            regex_rules: 1_000,
        }];
        assert!(check_extension_budget(&l, &budget(&ok)).is_ok());
        let over = [RulesetStats {
            rules: 1_001,
            unsafe_rules: 0,
            regex_rules: 1_001,
        }];
        assert_eq!(
            check_extension_budget(&l, &budget(&over)),
            Err(LimitError::TooManyRules {
                item: CountedItem::StaticRegexRules,
                limit: 1_000,
                found: 1_001
            })
        );
    }

    #[test]
    fn dynamic_rules_edge() {
        let l = Limits::default();
        let mut b = budget(&[]);
        b.dynamic = stats(30_000);
        assert!(check_extension_budget(&l, &b).is_ok());
        b.dynamic = stats(30_001);
        assert_eq!(
            check_extension_budget(&l, &b),
            Err(LimitError::TooManyRules {
                item: CountedItem::DynamicRules,
                limit: 30_000,
                found: 30_001
            })
        );
    }

    #[test]
    fn unsafe_dynamic_rules_edge() {
        let l = Limits::default();
        let mut b = budget(&[]);
        b.dynamic = RulesetStats {
            rules: 5_000,
            unsafe_rules: 5_000,
            regex_rules: 0,
        };
        assert!(check_extension_budget(&l, &b).is_ok());
        b.dynamic.unsafe_rules = 5_001;
        b.dynamic.rules = 5_001;
        assert_eq!(
            check_extension_budget(&l, &b),
            Err(LimitError::TooManyRules {
                item: CountedItem::UnsafeDynamicRules,
                limit: 5_000,
                found: 5_001
            })
        );
    }

    #[test]
    fn session_rules_edge() {
        let l = Limits::default();
        let mut b = budget(&[]);
        b.session = stats(5_000);
        assert!(check_extension_budget(&l, &b).is_ok());
        b.session = stats(5_001);
        assert_eq!(
            check_extension_budget(&l, &b),
            Err(LimitError::TooManyRules {
                item: CountedItem::SessionRules,
                limit: 5_000,
                found: 5_001
            })
        );
    }

    #[test]
    fn dynamic_and_session_regex_rules_are_counted_separately() {
        let l = Limits::default();
        let mut b = budget(&[]);
        let regex_1000 = RulesetStats {
            rules: 1_000,
            unsafe_rules: 0,
            regex_rules: 1_000,
        };
        b.dynamic = regex_1000;
        b.session = regex_1000;
        assert!(check_extension_budget(&l, &b).is_ok());
        b.dynamic.regex_rules = 1_001;
        b.dynamic.rules = 1_001;
        assert!(matches!(
            check_extension_budget(&l, &b),
            Err(LimitError::TooManyRules {
                item: CountedItem::DynamicRegexRules,
                ..
            })
        ));
        b.dynamic = RulesetStats::default();
        b.session.regex_rules = 1_001;
        b.session.rules = 1_001;
        assert!(matches!(
            check_extension_budget(&l, &b),
            Err(LimitError::TooManyRules {
                item: CountedItem::SessionRegexRules,
                ..
            })
        ));
    }

    #[test]
    fn sums_saturate_instead_of_overflowing() {
        let l = Limits::default();
        let huge = [stats(usize::MAX), stats(usize::MAX)];
        assert!(check_extension_budget(&l, &budget(&huge)).is_err());
    }

    #[test]
    fn global_pool_never_underflows() {
        let l = Limits::default();
        assert_eq!(global_pool_remaining(&l, 0), 330_000);
        assert_eq!(global_pool_remaining(&l, 330_000), 0);
        assert_eq!(global_pool_remaining(&l, 400_000), 0);
    }

    #[test]
    fn limit_errors_print_no_content() {
        let e = LimitError::TooManyRules {
            item: CountedItem::SessionRules,
            limit: 5,
            found: 6,
        };
        assert_eq!(e.to_string(), "too many session rules: 6 (limit 5)");
    }
}
