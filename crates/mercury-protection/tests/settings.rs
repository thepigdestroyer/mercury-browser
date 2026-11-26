// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The global protection settings and their profile file (R13 MH13).

use mercury_protection::{
    Categories, Category, LnaPolicy, ProtectionLevel, ProtectionSettings, Setting,
};

use Category::*;
use Setting::{Off, Standard};

#[test]
fn the_default_settings_are_standard_with_gpc_and_the_baseline_list_on() {
    let settings = ProtectionSettings::default();
    assert_eq!(settings.level, ProtectionLevel::Standard);
    assert_eq!(settings.custom, Categories::all(Standard));
    assert!(settings.gpc);
    assert!(settings.compat_baseline);
    assert!(!settings.compat_convenience);
    assert_eq!(settings.lna, LnaPolicy::Block);
}

#[test]
fn an_empty_profile_reads_as_the_default() {
    let settings: ProtectionSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(settings, ProtectionSettings::default());
}

#[test]
fn a_level_from_a_newer_build_reads_as_standard_and_the_other_fields_are_kept() {
    let settings: ProtectionSettings =
        serde_json::from_str(r#"{"level":"future_level","gpc":false}"#).unwrap();
    assert_eq!(settings.level, ProtectionLevel::Standard);
    assert!(!settings.gpc);
    assert!(settings.compat_baseline);
}

#[test]
fn an_unknown_setting_in_the_custom_table_reads_as_standard() {
    let settings: ProtectionSettings =
        serde_json::from_str(r#"{"custom":{"ads_and_trackers":"hyper"}}"#).unwrap();
    assert_eq!(settings.custom.get(AdsAndTrackers), Standard);
}

#[test]
fn the_legacy_switches_map_to_a_custom_table() {
    let settings = ProtectionSettings::from_legacy(false, true, true);
    assert_eq!(settings.level, ProtectionLevel::Custom);
    assert_eq!(
        settings.custom,
        Categories::all(Standard).with(AdsAndTrackers, Off)
    );
    assert!(settings.gpc);
}

#[test]
fn both_legacy_blocking_switches_on_is_the_default() {
    assert_eq!(
        ProtectionSettings::from_legacy(true, true, true),
        ProtectionSettings::default()
    );
}

#[test]
fn turning_off_third_party_cookie_blocking_keeps_the_legacy_gpc_flag() {
    let settings = ProtectionSettings::from_legacy(true, false, false);
    assert_eq!(settings.level, ProtectionLevel::Custom);
    assert_eq!(settings.custom.get(CrossSiteCookies), Off);
    assert_eq!(settings.custom.get(AdsAndTrackers), Standard);
    assert!(!settings.gpc);
}
