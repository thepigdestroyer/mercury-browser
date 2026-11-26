// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The web-compatibility exception list (R13 MH31).
//!
//! A rule lets one category's requests through on one top-level site, for a request host and its subdomains. The
//! list ships as data; this module parses it, refuses oversized or malformed input, and matches requests.

use serde::{Deserialize, Serialize};

use crate::level::Category;
use crate::settings::ProtectionSettings;
use crate::site::Site;

/// The largest list text read (1 MiB).
const MAX_BYTES: usize = 1 << 20;

/// The most rules read.
const MAX_RULES: usize = 20_000;

/// The version this build reads.
const VERSION: u32 = 1;

/// Which switch enables a rule: the baseline list or the convenience list (Q5: baseline on, convenience off).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatTier {
    /// Enabled by [`ProtectionSettings::compat_baseline`].
    Baseline,
    /// Enabled by [`ProtectionSettings::compat_convenience`].
    Convenience,
}

/// One exception: requests of `category` to `request_host` are allowed on `top_site`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatRule {
    /// A site key, or `*` for any site. Lower case.
    pub top_site: String,
    /// A host or a parent domain of it. Lower case.
    pub request_host: String,
    /// The category the rule lets through.
    pub category: Category,
    /// The switch that enables the rule.
    pub tier: CompatTier,
    /// Why the rule exists, for the maintainers.
    #[serde(default)]
    pub note: String,
}

/// Why a list was not read. The caller keeps [`CompatList::EMPTY`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CompatError {
    /// The text is longer than 1 MiB.
    #[error("the compatibility list is too large")]
    TooLarge,
    /// The list has more than 20,000 rules.
    #[error("the compatibility list has too many rules")]
    TooManyRules,
    /// A text field is longer than 253 bytes.
    #[error("a compatibility list field is too long")]
    FieldTooLong,
    /// The version is not one this build reads.
    #[error("unknown compatibility list version {0}")]
    UnknownVersion(u32),
    /// The text is not a valid list.
    #[error("malformed compatibility list: {0}")]
    Malformed(String),
}

/// The web-compatibility list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompatList(Vec<CompatRule>);

#[derive(Deserialize)]
struct Document {
    version: u32,
    #[serde(default)]
    rules: Vec<serde_json::Value>,
}

impl CompatList {
    /// The list with no rules: it allows nothing.
    pub const EMPTY: CompatList = CompatList(Vec::new());

    /// Reads `{"version":1,"rules":[...]}`. Rules are lower-cased here. Any error leaves the caller with
    /// [`CompatList::EMPTY`].
    pub fn from_json(text: &str) -> Result<CompatList, CompatError> {
        if text.len() > MAX_BYTES {
            return Err(CompatError::TooLarge);
        }
        let document: Document = serde_json::from_str(text)
            .map_err(|error| CompatError::Malformed(error.to_string()))?;
        if document.version != VERSION {
            return Err(CompatError::UnknownVersion(document.version));
        }
        if document.rules.len() > MAX_RULES {
            return Err(CompatError::TooManyRules);
        }
        let mut rules = Vec::with_capacity(document.rules.len());
        for value in document.rules {
            let rule: CompatRule = serde_json::from_value(value)
                .map_err(|error| CompatError::Malformed(error.to_string()))?;
            rules.push(normalise(rule)?);
        }
        Ok(CompatList(rules))
    }

    /// Whether a request of `category` to `request_host` is allowed on the top-level site `top`. A rule counts
    /// only when its tier is enabled in `settings`.
    pub fn allows(
        &self,
        settings: &ProtectionSettings,
        top: &Site,
        request_host: &str,
        category: Category,
    ) -> bool {
        let host = request_host
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        self.0.iter().any(|rule| {
            rule.category == category
                && tier_enabled(settings, rule.tier)
                && (rule.top_site == "*" || rule.top_site == top.as_str())
                && host_matches(&host, &rule.request_host)
        })
    }
}

fn normalise(mut rule: CompatRule) -> Result<CompatRule, CompatError> {
    let fields = [&rule.top_site, &rule.request_host, &rule.note];
    if fields.iter().any(|field| field.len() > Site::MAX_LEN) {
        return Err(CompatError::FieldTooLong);
    }
    rule.top_site = rule.top_site.trim().to_ascii_lowercase();
    rule.request_host = rule
        .request_host
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if rule.top_site.is_empty() || rule.request_host.is_empty() {
        return Err(CompatError::Malformed("an empty site or host".to_owned()));
    }
    Ok(rule)
}

fn tier_enabled(settings: &ProtectionSettings, tier: CompatTier) -> bool {
    match tier {
        CompatTier::Baseline => settings.compat_baseline,
        CompatTier::Convenience => settings.compat_convenience,
    }
}

/// Whether `host` is `rule` or one of its subdomains. `xa.example` is not a subdomain of `a.example`.
fn host_matches(host: &str, rule: &str) -> bool {
    host.strip_suffix(rule)
        .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('.'))
}
