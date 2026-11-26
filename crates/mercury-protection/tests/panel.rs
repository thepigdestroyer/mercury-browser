// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `panel_view`: what the shield panel shows for one site (MH14, MH12).

use mercury_protection::{
    BlockEvent, Category, ExceptionList, Overrides, ProtectionSettings, ResourceKind, Setting,
    Site, SiteException, TabKey, TabProtection, panel_view,
};

fn site(key: &str) -> Site {
    Site::from_key(key).expect("a site")
}

fn exception(key: &str) -> SiteException {
    SiteException {
        site: site(key),
        shields_down: false,
        level: None,
        overrides: Overrides::default(),
        updated_at_ms: 0,
    }
}

#[test]
fn a_standard_site_lists_nine_rows_in_category_order() {
    let settings = ProtectionSettings::default();
    let effective = settings.resolve(&ExceptionList::default(), Some(&site("example.com")));

    let view = panel_view(&effective, None, None);

    assert_eq!(view.site, Some(site("example.com")));
    assert!(!view.shields_down);
    assert_eq!(view.total_blocked, 0);
    assert!(view.hosts.is_empty());
    let categories: Vec<Category> = view.categories.iter().map(|r| r.category).collect();
    assert_eq!(categories, Category::ALL.to_vec());
    for row in &view.categories {
        assert_eq!(row.setting, effective.setting(row.category));
        assert_eq!(row.count, 0);
        assert!(!row.overridden);
    }
}

#[test]
fn overridden_is_true_only_for_the_categories_the_exception_overrides() {
    let settings = ProtectionSettings::default();
    let mut list = ExceptionList::default();
    let mut e = exception("example.com");
    e.overrides
        .set(Category::CrossSiteCookies, Some(Setting::Off));
    e.overrides.set(Category::LocalNetwork, Some(Setting::Off));
    list.set(e.clone()).expect("room");
    let effective = settings.resolve(&list, Some(&site("example.com")));

    let view = panel_view(&effective, Some(&e), None);

    for row in &view.categories {
        let expected = matches!(
            row.category,
            Category::CrossSiteCookies | Category::LocalNetwork
        );
        assert_eq!(row.overridden, expected, "{:?}", row.category);
    }
    let cookies = view
        .categories
        .iter()
        .find(|r| r.category == Category::CrossSiteCookies)
        .expect("a row");
    assert_eq!(cookies.setting, Setting::Off);
}

#[test]
fn total_blocked_and_hosts_come_from_the_tab_counters() {
    let settings = ProtectionSettings::default();
    let effective = settings.resolve(&ExceptionList::default(), Some(&site("example.com")));
    let mut tabs = TabProtection::default();
    for _ in 0..3 {
        tabs.record(&BlockEvent {
            tab: TabKey(1),
            category: Category::AdsAndTrackers,
            host: "ads.example".to_owned(),
            kind: ResourceKind::Script,
        });
    }
    tabs.record(&BlockEvent {
        tab: TabKey(1),
        category: Category::Cryptominers,
        host: "mine.example".to_owned(),
        kind: ResourceKind::Script,
    });
    let counters = tabs.counters(TabKey(1));

    let view = panel_view(&effective, None, counters);

    let counted = counters.map(|c| c.total).unwrap_or(0);
    assert_eq!(view.total_blocked, counted);
    assert_eq!(view.total_blocked, 4);
    let ads = view
        .categories
        .iter()
        .find(|r| r.category == Category::AdsAndTrackers)
        .expect("a row");
    assert_eq!(ads.count, 3);
    assert_eq!(view.hosts.len(), 2);
    assert_eq!(view.hosts[0].host, "ads.example");
    assert_eq!(view.hosts[0].count, 3);
    assert_eq!(view.hosts[0].category, Category::AdsAndTrackers);
    assert_eq!(view.hosts[1].host, "mine.example");
}

#[test]
fn shields_down_shows_every_row_off() {
    let settings = ProtectionSettings::default();
    let mut list = ExceptionList::default();
    let mut e = exception("example.com");
    e.shields_down = true;
    list.set(e.clone()).expect("room");
    let effective = settings.resolve(&list, Some(&site("example.com")));

    let view = panel_view(&effective, Some(&e), None);

    assert!(view.shields_down);
    assert_eq!(view.categories.len(), Category::COUNT);
    assert!(view.categories.iter().all(|r| r.setting == Setting::Off));
}
