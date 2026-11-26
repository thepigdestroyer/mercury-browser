// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Persistence through `mercury-store`: exceptions, the weekly tally, bounce records (in-memory store).

use mercury_protection::persist::{
    clear_all, delete_exception, flush_tally, load_bounce, load_exceptions, load_tally,
    save_bounce, save_exception,
};
use mercury_protection::{
    BounceConfig, BounceTracker, Category, NavKind, Overrides, ProtectionLevel, Setting, Site,
    SiteException, TabKey, WeeklyTally,
};
use mercury_store::{ProtectionExceptionRow, Store};

const DAY: i64 = 86_400_000;
const T0: i64 = 1_000_000_000;

fn site(key: &str) -> Site {
    Site::from_key(key).unwrap()
}

fn exception(key: &str) -> SiteException {
    SiteException {
        site: site(key),
        shields_down: false,
        level: None,
        overrides: Overrides::default(),
        updated_at_ms: T0,
    }
}

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn raw_exceptions(store: &Store) -> Vec<ProtectionExceptionRow> {
    store.protection_exceptions().unwrap()
}

#[test]
fn exceptions_round_trip_with_level_overrides_and_shields_down() {
    let mut store = store();
    let mut strict = exception("example.com");
    strict.level = Some(ProtectionLevel::Strict);
    strict
        .overrides
        .set(Category::CrossSiteCookies, Some(Setting::Off));
    let mut down = exception("bank.example");
    down.shields_down = true;
    save_exception(&mut store, &strict).unwrap();
    save_exception(&mut store, &down).unwrap();

    let loaded = load_exceptions(&store).unwrap();

    assert_eq!(loaded.skipped, 0);
    assert_eq!(loaded.value.get(&site("example.com")), Some(&strict));
    assert_eq!(loaded.value.get(&site("bank.example")), Some(&down));
}

#[test]
fn deleting_an_exception_removes_it_and_reports_whether_it_existed() {
    let mut store = store();
    let mut e = exception("example.com");
    e.shields_down = true;
    save_exception(&mut store, &e).unwrap();

    assert!(delete_exception(&mut store, &site("example.com")).unwrap());
    assert!(!delete_exception(&mut store, &site("example.com")).unwrap());
    assert!(load_exceptions(&store).unwrap().value.is_empty());
}

#[test]
fn an_exception_that_changes_nothing_is_removed_not_stored() {
    let mut store = store();
    let mut e = exception("example.com");
    e.shields_down = true;
    save_exception(&mut store, &e).unwrap();

    save_exception(&mut store, &exception("example.com")).unwrap();

    assert!(raw_exceptions(&store).is_empty());
}

#[test]
fn a_row_with_an_unknown_level_is_skipped_and_left_in_the_store() {
    let mut store = store();
    store
        .put_protection_exception(&ProtectionExceptionRow {
            site: "future.example".into(),
            shields_down: false,
            level: Some("hyper".into()),
            overrides: String::new(),
            updated_at: T0,
        })
        .unwrap();

    let loaded = load_exceptions(&store).unwrap();

    assert!(loaded.value.is_empty());
    assert_eq!(loaded.skipped, 1);
    assert_eq!(raw_exceptions(&store).len(), 1);
}

#[test]
fn a_row_with_a_site_that_is_not_a_site_is_skipped() {
    let mut store = store();
    store
        .put_protection_exception(&ProtectionExceptionRow {
            site: "   ".into(),
            shields_down: true,
            level: None,
            overrides: String::new(),
            updated_at: T0,
        })
        .unwrap();

    let loaded = load_exceptions(&store).unwrap();

    assert!(loaded.value.is_empty());
    assert_eq!(loaded.skipped, 1);
}

#[test]
fn an_unknown_override_pair_is_dropped_and_the_rest_of_the_row_kept() {
    let mut store = store();
    store
        .put_protection_exception(&ProtectionExceptionRow {
            site: "example.com".into(),
            shields_down: false,
            level: Some("strict".into()),
            overrides: "telemetry=off,cross_site_cookies=off".into(),
            updated_at: T0,
        })
        .unwrap();

    let loaded = load_exceptions(&store).unwrap();
    let e = loaded.value.get(&site("example.com")).unwrap();

    assert_eq!(loaded.skipped, 1);
    assert_eq!(e.level, Some(ProtectionLevel::Strict));
    assert_eq!(
        e.overrides.get(Category::CrossSiteCookies),
        Some(Setting::Off)
    );
}

#[test]
fn loading_does_not_rewrite_the_stored_rows() {
    let mut store = store();
    let bad = ProtectionExceptionRow {
        site: "future.example".into(),
        shields_down: false,
        level: Some("hyper".into()),
        overrides: "telemetry=off".into(),
        updated_at: T0,
    };
    store.put_protection_exception(&bad).unwrap();
    let before = raw_exceptions(&store);

    load_exceptions(&store).unwrap();
    load_exceptions(&store).unwrap();

    assert_eq!(raw_exceptions(&store), before);
}

#[test]
fn flushing_a_tally_twice_writes_each_delta_once() {
    let mut store = store();
    let mut tally = WeeklyTally::default();
    tally.record(T0, Category::AdsAndTrackers, 3);
    tally.record(T0, Category::AdsAndTrackers, 2);

    flush_tally(&mut store, &mut tally, T0).unwrap();
    flush_tally(&mut store, &mut tally, T0).unwrap();

    let rows = store
        .protection_tally_since(WeeklyTally::day_of(T0))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].count, 5);
}

#[test]
fn the_tally_after_a_flush_equals_the_in_memory_week() {
    let mut store = store();
    let mut tally = WeeklyTally::default();
    tally.record(T0, Category::AdsAndTrackers, 4);
    tally.record(T0 - DAY, Category::Fingerprinting, 7);
    tally.record(T0 - 3 * DAY, Category::Cryptominers, 1);

    flush_tally(&mut store, &mut tally, T0).unwrap();
    let loaded = load_tally(&store, T0).unwrap();

    assert_eq!(loaded.skipped, 0);
    assert_eq!(loaded.value.week(T0), tally.week(T0));
}

#[test]
fn a_tally_row_with_an_unknown_category_is_skipped() {
    let mut store = store();
    store
        .add_protection_tally(WeeklyTally::day_of(T0), "telemetry", 9)
        .unwrap();
    store
        .add_protection_tally(WeeklyTally::day_of(T0), "ads_and_trackers", 2)
        .unwrap();

    let loaded = load_tally(&store, T0).unwrap();

    assert_eq!(loaded.skipped, 1);
    assert_eq!(loaded.value.week(T0).total, 2);
}

#[test]
fn days_older_than_the_kept_window_are_pruned_by_a_flush() {
    let mut store = store();
    let mut tally = WeeklyTally::default();
    tally.record(T0 - 40 * DAY, Category::AdsAndTrackers, 6);
    flush_tally(&mut store, &mut tally, T0 - 40 * DAY).unwrap();
    assert_eq!(
        store
            .protection_tally_since(i64::MIN)
            .unwrap()
            .iter()
            .map(|row| row.count)
            .sum::<i64>(),
        6
    );

    tally.record(T0, Category::Fingerprinting, 1);
    flush_tally(&mut store, &mut tally, T0).unwrap();

    let rows = store.protection_tally_since(i64::MIN).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].category, "fingerprinting");
}

#[test]
fn a_flush_with_no_dirty_rows_writes_nothing() {
    let mut store = store();
    let mut tally = WeeklyTally::default();

    flush_tally(&mut store, &mut tally, T0).unwrap();

    assert!(store.protection_tally_since(i64::MIN).unwrap().is_empty());
}

#[test]
fn bounce_records_round_trip_and_a_due_site_is_still_due_after_reload() {
    let cfg = BounceConfig::for_setting(Setting::Standard);
    let mut tracker = BounceTracker::new(cfg);
    let tab = TabKey(1);
    tracker.navigated(tab, Some(site("a.example")), NavKind::User, T0);
    tracker.navigated(tab, Some(site("b.example")), NavKind::Redirect, T0);
    tracker.storage_accessed(tab, &site("b.example"));
    tracker.navigated(tab, Some(site("c.example")), NavKind::Redirect, T0);
    tracker.tab_closed(tab, T0);
    tracker.user_activated(&site("x.example"), T0 + 1);
    let mut store = store();

    save_bounce(&mut store, &tracker).unwrap();
    let loaded = load_bounce(&store, cfg).unwrap();

    assert_eq!(loaded.skipped, 0);
    assert_eq!(loaded.value.records(), tracker.records());
    assert_eq!(
        loaded.value.due(T0 + 3_600_000, &|_: &Site| false),
        vec![site("b.example")]
    );
}

#[test]
fn clear_all_empties_the_three_tables() {
    let mut store = store();
    let mut e = exception("example.com");
    e.shields_down = true;
    save_exception(&mut store, &e).unwrap();
    let mut tally = WeeklyTally::default();
    tally.record(T0, Category::AdsAndTrackers, 1);
    flush_tally(&mut store, &mut tally, T0).unwrap();
    let mut tracker = BounceTracker::new(BounceConfig::for_setting(Setting::Standard));
    tracker.user_activated(&site("x.example"), T0);
    save_bounce(&mut store, &tracker).unwrap();

    clear_all(&mut store).unwrap();

    assert!(raw_exceptions(&store).is_empty());
    assert!(store.protection_tally_since(i64::MIN).unwrap().is_empty());
    assert!(store.protection_bounce_records().unwrap().is_empty());
}
