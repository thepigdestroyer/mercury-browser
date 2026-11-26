// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The effective table for a site, and the per-site exceptions (R13 MH12).

use mercury_protection::{
    Categories, Category, ExceptionError, ExceptionList, MAX_EXCEPTIONS, Overrides,
    ProtectionLevel, ProtectionSettings, Setting, Site, SiteException,
};

use Category::*;
use Setting::{Off, Standard, Strict};

fn site(key: &str) -> Site {
    Site::from_key(key).unwrap()
}

fn plain(key: &str) -> SiteException {
    SiteException {
        site: site(key),
        shields_down: false,
        level: None,
        overrides: Overrides::default(),
        updated_at_ms: 0,
    }
}

fn flagged(key: &str) -> SiteException {
    SiteException {
        shields_down: true,
        ..plain(key)
    }
}

#[test]
fn a_site_without_an_exception_gets_the_global_table() {
    let settings = ProtectionSettings::default();
    let effective = settings.resolve(&ExceptionList::default(), Some(&site("example.com")));
    assert_eq!(effective.categories, Categories::all(Standard));
    assert_eq!(effective.level, ProtectionLevel::Standard);
    assert!(!effective.shields_down);
    assert_eq!(effective.site, Some(site("example.com")));
}

#[test]
fn shields_down_turns_every_category_off() {
    let mut list = ExceptionList::default();
    list.set(flagged("example.com")).unwrap();
    let effective = ProtectionSettings::default().resolve(&list, Some(&site("example.com")));
    assert_eq!(effective.categories, Categories::all(Off));
    assert!(effective.shields_down);
}

#[test]
fn a_site_level_replaces_the_global_level_for_that_site() {
    let mut list = ExceptionList::default();
    list.set(SiteException {
        level: Some(ProtectionLevel::Strict),
        ..plain("example.com")
    })
    .unwrap();
    let settings = ProtectionSettings::default();
    let effective = settings.resolve(&list, Some(&site("example.com")));
    assert_eq!(effective.level, ProtectionLevel::Strict);
    assert_eq!(
        effective.categories,
        ProtectionLevel::Strict.table(&settings.custom)
    );
}

#[test]
fn overrides_apply_on_top_of_the_level_and_leave_the_other_categories_alone() {
    let mut overrides = Overrides::default();
    overrides.set(CrossSiteCookies, Some(Off));
    let mut list = ExceptionList::default();
    list.set(SiteException {
        overrides,
        ..plain("example.com")
    })
    .unwrap();
    let effective = ProtectionSettings::default().resolve(&list, Some(&site("example.com")));
    assert_eq!(
        effective.categories,
        Categories::all(Standard).with(CrossSiteCookies, Off)
    );
}

#[test]
fn resolving_without_a_site_ignores_the_exceptions() {
    let mut list = ExceptionList::default();
    list.set(flagged("example.com")).unwrap();
    let effective = ProtectionSettings::default().resolve(&list, None);
    assert_eq!(effective.categories, Categories::all(Standard));
    assert!(!effective.shields_down);
    assert_eq!(effective.site, None);
}

#[test]
fn a_custom_global_level_uses_the_custom_table_and_a_site_strict_level_the_strict_one() {
    let custom = Categories::all(Standard).with(Fingerprinting, Off);
    let settings = ProtectionSettings {
        level: ProtectionLevel::Custom,
        custom,
        ..ProtectionSettings::default()
    };
    let mut list = ExceptionList::default();
    list.set(SiteException {
        level: Some(ProtectionLevel::Strict),
        ..plain("strict.example")
    })
    .unwrap();
    list.set(SiteException {
        level: Some(ProtectionLevel::Custom),
        ..plain("custom.example")
    })
    .unwrap();
    assert_eq!(
        settings
            .resolve(&list, Some(&site("strict.example")))
            .categories,
        ProtectionLevel::Strict.table(&custom)
    );
    assert_eq!(
        settings
            .resolve(&list, Some(&site("custom.example")))
            .categories,
        custom
    );
    assert_eq!(
        settings
            .resolve(&list, Some(&site("other.example")))
            .categories,
        custom
    );
}

#[test]
fn ads_are_blocked_for_third_party_requests_at_standard_and_for_all_at_strict() {
    let standard = ProtectionSettings::default().resolve(&ExceptionList::default(), None);
    assert!(!standard.blocks_ads_for(false));
    assert!(standard.blocks_ads_for(true));
    let strict = ProtectionSettings {
        level: ProtectionLevel::Strict,
        ..ProtectionSettings::default()
    }
    .resolve(&ExceptionList::default(), None);
    assert!(strict.blocks_ads_for(false));
}

#[test]
fn an_exception_with_no_flags_removes_the_record_and_returns_ok() {
    let mut list = ExceptionList::default();
    list.set(flagged("example.com")).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list.set(plain("example.com")), Ok(()));
    assert_eq!(list.len(), 0);
    assert!(list.get(&site("example.com")).is_none());
}

#[test]
fn the_list_refuses_a_new_site_past_the_cap_but_still_updates_an_existing_one() {
    let mut list = ExceptionList::default();
    for index in 0..MAX_EXCEPTIONS {
        list.set(flagged(&format!("site{index}.example"))).unwrap();
    }
    assert_eq!(list.len(), MAX_EXCEPTIONS);
    assert_eq!(
        list.set(flagged("one-more.example")),
        Err(ExceptionError::Full)
    );
    list.set(SiteException {
        updated_at_ms: 5,
        ..flagged("site0.example")
    })
    .unwrap();
    assert_eq!(list.get(&site("site0.example")).unwrap().updated_at_ms, 5);
    assert_eq!(list.len(), MAX_EXCEPTIONS);
}

#[test]
fn the_exceptions_iterate_ordered_by_site() {
    let mut list = ExceptionList::default();
    for key in ["zeta.example", "alpha.example", "mid.example"] {
        list.set(flagged(key)).unwrap();
    }
    let keys: Vec<&str> = list.iter().map(|e| e.site.as_str()).collect();
    assert_eq!(keys, ["alpha.example", "mid.example", "zeta.example"]);
}

#[test]
fn overrides_round_trip_as_text_and_count_the_pairs_they_drop() {
    let mut overrides = Overrides::default();
    overrides.set(AdsAndTrackers, Some(Off));
    overrides.set(CrossSiteCookies, Some(Strict));
    assert_eq!(
        overrides.to_text(),
        "ads_and_trackers=off,cross_site_cookies=strict"
    );
    let (back, dropped) = Overrides::from_text(&overrides.to_text());
    assert_eq!(back, overrides);
    assert_eq!(dropped, 0);

    let (partly, dropped) =
        Overrides::from_text("telemetry=off,cryptominers=hyper,fingerprinting=standard,junk");
    assert_eq!(partly.get(Fingerprinting), Some(Standard));
    assert_eq!(partly.iter().count(), 1);
    assert_eq!(dropped, 3);
    let (empty, dropped) = Overrides::from_text("");
    assert!(empty.is_empty());
    assert_eq!(dropped, 0);
}
