// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The level table (R13 MH13; owner decision O-18: Standard by default).

use mercury_protection::{Categories, Category, ProtectionLevel, Setting};

use Category::*;
use Setting::{Off, Standard, Strict};

#[test]
fn the_standard_level_applies_every_category_in_its_usual_form() {
    let table = ProtectionLevel::Standard.table(&Categories::all(Off));
    for category in Category::ALL {
        assert_eq!(table.get(category), Standard, "{category:?}");
    }
}

#[test]
fn the_strict_level_differs_from_standard_exactly_where_the_spec_says() {
    let table = ProtectionLevel::Strict.table(&Categories::all(Off));
    let expected = [
        (AdsAndTrackers, Strict),
        (SocialTrackers, Strict),
        (CrossSiteCookies, Strict),
        (Fingerprinting, Strict),
        (Cryptominers, Strict),
        (CookieBanners, Standard),
        (BounceTracking, Strict),
        (UrlCleaning, Strict),
        (LocalNetwork, Standard),
    ];
    for (category, setting) in expected {
        assert_eq!(table.get(category), setting, "{category:?}");
    }
}

#[test]
fn the_custom_level_is_the_users_own_table() {
    let custom = Categories::all(Standard)
        .with(AdsAndTrackers, Off)
        .with(Fingerprinting, Strict);
    assert_eq!(ProtectionLevel::Custom.table(&custom), custom);
    // The built-in levels ignore it.
    assert_eq!(
        ProtectionLevel::Standard.table(&custom),
        Categories::all(Standard)
    );
}

#[test]
fn the_categories_are_nine_in_the_panels_order_and_their_ids_round_trip() {
    assert_eq!(Category::ALL.len(), 9);
    let ids: Vec<&str> = Category::ALL.iter().map(|c| c.id()).collect();
    assert_eq!(
        ids,
        [
            "ads_and_trackers",
            "social_trackers",
            "cross_site_cookies",
            "fingerprinting",
            "cryptominers",
            "cookie_banners",
            "bounce_tracking",
            "url_cleaning",
            "local_network",
        ]
    );
    for (index, category) in Category::ALL.into_iter().enumerate() {
        assert_eq!(Category::from_id(category.id()), Some(category));
        assert_eq!(category.index(), index);
    }
    assert_eq!(Category::from_id("telemetry"), None);
}

#[test]
fn settings_and_levels_have_permanent_ids() {
    assert_eq!(
        [Off, Standard, Strict].map(Setting::id),
        ["off", "standard", "strict"]
    );
    assert_eq!(Setting::from_id("strict"), Some(Strict));
    assert_eq!(Setting::from_id("nonsense"), None);
    assert!(!Off.is_on() && Standard.is_on() && Strict.is_on());
    for level in [
        ProtectionLevel::Standard,
        ProtectionLevel::Strict,
        ProtectionLevel::Custom,
    ] {
        assert_eq!(ProtectionLevel::from_id(level.id()), Some(level));
    }
    assert_eq!(ProtectionLevel::from_id("paranoid"), None);
}

#[test]
fn a_table_changes_one_cell_at_a_time() {
    let table = Categories::all(Off).with(AdsAndTrackers, Strict);
    assert_eq!(table.get(AdsAndTrackers), Strict);
    assert_eq!(table.get(Cryptominers), Off);
}

#[test]
fn a_table_is_written_as_a_map_of_ids_and_read_back() {
    let table = Categories::all(Standard).with(UrlCleaning, Off);
    let text = serde_json::to_string(&table).unwrap();
    assert!(text.contains(r#""url_cleaning":"off""#), "{text}");
    assert_eq!(serde_json::from_str::<Categories>(&text).unwrap(), table);
}

#[test]
fn a_table_from_a_newer_build_still_loads() {
    let text = r#"{"ads_and_trackers":"hyper","telemetry":"strict","cookie_banners":"off","bounce_tracking":7}"#;
    let table: Categories = serde_json::from_str(text).unwrap();
    assert_eq!(table.get(AdsAndTrackers), Standard, "unknown setting: Standard");
    assert_eq!(table.get(CookieBanners), Off);
    assert_eq!(table.get(BounceTracking), Standard, "not a string: Standard");
    assert_eq!(table.get(Cryptominers), Standard, "missing: Standard");
}
