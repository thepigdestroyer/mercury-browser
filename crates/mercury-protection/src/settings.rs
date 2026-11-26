// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The user's global protection settings (MH13).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::level::{Categories, Category, ProtectionLevel, Setting};

/// What happens to a public page's request for loopback or the local network (R14c D6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LnaPolicy {
    /// Refuse, and show the block in the panel with an "Allow on this site" button.
    #[default]
    Block,
    /// Ask the user (needs the dialogs lane's permission sheet).
    Ask,
}

/// The global protection settings, as the profile file keeps them. A field this build does not understand
/// reads as its default, so a profile written by a newer build still loads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtectionSettings {
    /// The level for every site without a level of its own.
    #[serde(deserialize_with = "lenient")]
    pub level: ProtectionLevel,
    /// The user's own table; used when `level` is [`ProtectionLevel::Custom`].
    pub custom: Categories,
    /// Global Privacy Control: the header and `navigator.globalPrivacyControl`. Global only (P6).
    pub gpc: bool,
    /// The baseline web-compatibility list (MH31, tier 1).
    pub compat_baseline: bool,
    /// The convenience web-compatibility list (MH31, tier 2).
    pub compat_convenience: bool,
    /// What happens to a public page's request for the local network.
    #[serde(deserialize_with = "lenient")]
    pub lna: LnaPolicy,
}

impl Default for ProtectionSettings {
    fn default() -> Self {
        Self {
            level: ProtectionLevel::Standard,
            custom: Categories::all(Setting::Standard),
            gpc: true,
            compat_baseline: true,
            compat_convenience: false,
            lna: LnaPolicy::Block,
        }
    }
}

impl ProtectionSettings {
    /// The settings a pre-0.3 `mercury-settings` file means: its two blocking switches and GPC. Both blocking
    /// switches on is the default; otherwise the switched-off categories go into a `Custom` table.
    pub fn from_legacy(
        block_ads_and_trackers: bool,
        block_third_party_cookies: bool,
        gpc: bool,
    ) -> Self {
        let mut custom = Categories::all(Setting::Standard);
        if !block_ads_and_trackers {
            custom = custom.with(Category::AdsAndTrackers, Setting::Off);
        }
        if !block_third_party_cookies {
            custom = custom.with(Category::CrossSiteCookies, Setting::Off);
        }
        let level = if custom == Categories::all(Setting::Standard) {
            ProtectionLevel::Standard
        } else {
            ProtectionLevel::Custom
        };
        Self {
            level,
            custom,
            gpc,
            ..Self::default()
        }
    }
}

/// Reads a value this build knows, or the type's default for one it does not.
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}
