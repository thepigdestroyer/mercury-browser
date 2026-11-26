// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The bounce-tracking state machine (MH32). The tests drive it with a fake clock: the tracker never reads real time.

use mercury_protection::{BounceConfig, BounceTracker, NavKind, Setting, Site, SiteRecord, TabKey};

const T0: i64 = 1_000_000;
const HOUR: i64 = 3_600_000;
const DAYS_45: i64 = 45 * 86_400_000;

const TAB: TabKey = TabKey(1);
const OTHER_TAB: TabKey = TabKey(2);

/// A fake clock: it only adds milliseconds to a fixed origin.
struct Clock(i64);

impl Clock {
    fn at(&self, ms: i64) -> i64 {
        self.0 + ms
    }
}

fn site(key: &str) -> Site {
    Site::from_key(key).unwrap()
}

fn tracker() -> BounceTracker {
    BounceTracker::new(BounceConfig::for_setting(Setting::Standard))
}

fn due(tracker: &BounceTracker, now_ms: i64) -> Vec<Site> {
    tracker.due(now_ms, &|_: &Site| false)
}

fn record(key: &str, last_activation_ms: Option<i64>, bounced_at_ms: Option<i64>) -> SiteRecord {
    SiteRecord {
        site: site(key),
        last_activation_ms,
        bounced_at_ms,
    }
}

/// Runs `a → b → c` on `tab` with `b` storing data, and closes the chain at `at`.
fn bounce_b(tracker: &mut BounceTracker, tab: TabKey, at: i64) {
    tracker.navigated(tab, Some(site("a.example")), NavKind::User, at - 2);
    tracker.navigated(tab, Some(site("b.example")), NavKind::Redirect, at - 1);
    tracker.storage_accessed(tab, &site("b.example"));
    tracker.navigated(tab, Some(site("c.example")), NavKind::Redirect, at);
    tracker.tab_closed(tab, at);
}

#[test]
fn a_redirect_hop_that_stored_data_is_due_one_hour_after_the_chain_closes() {
    let clock = Clock(T0);
    let mut t = tracker();
    t.navigated(TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(TAB, Some(site("b.example")), NavKind::Redirect, T0);
    t.storage_accessed(TAB, &site("b.example"));
    t.navigated(TAB, Some(site("c.example")), NavKind::Redirect, T0);
    t.tab_closed(TAB, T0);

    assert_eq!(t.records(), vec![record("b.example", None, Some(T0))]);
    assert!(due(&t, clock.at(HOUR - 1)).is_empty());
    assert_eq!(due(&t, clock.at(HOUR)), vec![site("b.example")]);
}

#[test]
fn a_hop_that_stored_nothing_and_a_one_hop_chain_are_never_due() {
    let mut t = tracker();
    t.navigated(TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(TAB, Some(site("b.example")), NavKind::Redirect, T0);
    t.navigated(TAB, Some(site("c.example")), NavKind::Redirect, T0);
    t.tab_closed(TAB, T0);
    t.navigated(OTHER_TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(OTHER_TAB, Some(site("b.example")), NavKind::Redirect, T0);
    t.tab_closed(OTHER_TAB, T0);

    assert!(t.records().is_empty());
    assert!(due(&t, T0 + DAYS_45).is_empty());
}

#[test]
fn a_script_hop_within_the_client_redirect_window_continues_the_chain() {
    let mut t = tracker();
    t.navigated(TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(TAB, Some(site("b.example")), NavKind::Script, T0);
    t.storage_accessed(TAB, &site("b.example"));
    t.navigated(TAB, Some(site("c.example")), NavKind::Script, T0 + 10_000);
    t.tab_closed(TAB, T0 + 10_000);

    assert_eq!(due(&t, T0 + 10_000 + HOUR), vec![site("b.example")]);
}

#[test]
fn a_script_hop_later_than_the_window_closes_the_chain_and_b_is_not_a_bounce() {
    let mut t = tracker();
    t.navigated(TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(TAB, Some(site("b.example")), NavKind::Script, T0);
    t.storage_accessed(TAB, &site("b.example"));
    t.navigated(TAB, Some(site("c.example")), NavKind::Script, T0 + 10_001);
    t.tab_closed(TAB, T0 + 10_001);

    assert!(t.records().is_empty());
    assert!(due(&t, T0 + DAYS_45).is_empty());
}

#[test]
fn an_activation_before_the_chain_closes_protects_the_site() {
    let mut t = tracker();
    t.navigated(TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(TAB, Some(site("b.example")), NavKind::Redirect, T0);
    t.storage_accessed(TAB, &site("b.example"));
    t.user_activated(&site("b.example"), T0 + 1_000);
    t.navigated(TAB, Some(site("c.example")), NavKind::Redirect, T0 + 2_000);
    t.tab_closed(TAB, T0 + 2_000);

    assert_eq!(
        t.records(),
        vec![record("b.example", Some(T0 + 1_000), None)]
    );
    assert!(due(&t, T0 + DAYS_45).is_empty());
}

#[test]
fn an_activation_after_the_bounce_clears_it() {
    let mut t = tracker();
    bounce_b(&mut t, TAB, T0);
    assert_eq!(due(&t, T0 + HOUR), vec![site("b.example")]);

    t.user_activated(&site("b.example"), T0 + 100);

    assert!(due(&t, T0 + HOUR + 1_000).is_empty());
    assert_eq!(t.records(), vec![record("b.example", Some(T0 + 100), None)]);
}

#[test]
fn an_activation_older_than_the_lifetime_does_not_protect_and_one_just_inside_it_does() {
    let mut old = tracker();
    old.user_activated(&site("b.example"), T0);
    bounce_b(&mut old, TAB, T0 + DAYS_45 + 1);
    assert_eq!(due(&old, T0 + DAYS_45 + 1 + HOUR), vec![site("b.example")]);

    let mut recent = tracker();
    recent.user_activated(&site("b.example"), T0);
    bounce_b(&mut recent, TAB, T0 + DAYS_45 - 1);
    assert!(due(&recent, T0 + DAYS_45 - 1 + HOUR).is_empty());
    assert_eq!(recent.records(), vec![record("b.example", Some(T0), None)]);
}

#[test]
fn a_chain_that_returns_to_its_first_site_still_bounces_the_middle_one() {
    let mut t = tracker();
    t.navigated(TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(TAB, Some(site("b.example")), NavKind::Redirect, T0);
    t.storage_accessed(TAB, &site("b.example"));
    t.navigated(TAB, Some(site("a.example")), NavKind::Redirect, T0);
    t.tab_closed(TAB, T0);

    assert_eq!(due(&t, T0 + HOUR), vec![site("b.example")]);
    assert_eq!(t.records().len(), 1);
}

#[test]
fn same_site_hops_record_nothing() {
    let mut t = tracker();
    t.navigated(TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(TAB, Some(site("a.example")), NavKind::Redirect, T0);
    t.storage_accessed(TAB, &site("a.example"));
    t.navigated(TAB, Some(site("a.example")), NavKind::Redirect, T0);
    t.tab_closed(TAB, T0);
    t.navigated(OTHER_TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(OTHER_TAB, Some(site("a.example")), NavKind::Redirect, T0);
    t.storage_accessed(OTHER_TAB, &site("a.example"));
    t.tab_closed(OTHER_TAB, T0);

    assert!(t.records().is_empty());
    assert!(due(&t, T0 + DAYS_45).is_empty());
}

#[test]
fn an_exempt_site_is_not_due_until_it_is_not_exempt() {
    let mut t = tracker();
    bounce_b(&mut t, TAB, T0);
    let b = site("b.example");

    assert!(t.due(T0 + HOUR, &|s: &Site| *s == b).is_empty());
    assert_eq!(due(&t, T0 + HOUR), vec![b]);
}

#[test]
fn a_purged_site_is_not_due_even_later() {
    let mut t = tracker();
    bounce_b(&mut t, TAB, T0);
    t.purged(&site("b.example"));

    assert!(due(&t, T0 + DAYS_45).is_empty());
    assert!(t.records().iter().all(|r| r.bounced_at_ms.is_none()));
}

#[test]
fn closing_an_unknown_tab_does_nothing() {
    let mut t = tracker();
    t.tab_closed(TabKey(99), T0);

    assert!(t.records().is_empty());
}

#[test]
fn a_navigation_without_a_site_closes_the_chain_and_starts_none() {
    let mut t = tracker();
    t.navigated(TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(TAB, Some(site("b.example")), NavKind::Redirect, T0);
    t.storage_accessed(TAB, &site("b.example"));
    t.navigated(TAB, Some(site("c.example")), NavKind::Redirect, T0);
    t.navigated(TAB, None, NavKind::User, T0 + 5);
    t.storage_accessed(TAB, &site("c.example"));
    t.tab_closed(TAB, T0 + 6);

    assert_eq!(t.records(), vec![record("b.example", None, Some(T0 + 5))]);
    assert_eq!(due(&t, T0 + 5 + HOUR), vec![site("b.example")]);
}

#[test]
fn two_tabs_interleaved_do_not_mix_their_chains() {
    let mut t = tracker();
    t.navigated(TAB, Some(site("a.example")), NavKind::User, T0);
    t.navigated(OTHER_TAB, Some(site("x.example")), NavKind::User, T0);
    t.navigated(TAB, Some(site("b.example")), NavKind::Redirect, T0 + 1);
    t.navigated(
        OTHER_TAB,
        Some(site("d.example")),
        NavKind::Redirect,
        T0 + 1,
    );
    t.storage_accessed(OTHER_TAB, &site("b.example"));
    t.storage_accessed(TAB, &site("b.example"));
    t.navigated(TAB, Some(site("c.example")), NavKind::Redirect, T0 + 2);
    t.navigated(
        OTHER_TAB,
        Some(site("y.example")),
        NavKind::Redirect,
        T0 + 2,
    );
    t.tab_closed(OTHER_TAB, T0 + 3);

    assert!(t.records().is_empty());

    t.tab_closed(TAB, T0 + 4);

    assert_eq!(due(&t, T0 + 4 + HOUR), vec![site("b.example")]);
    assert_eq!(t.records().len(), 1);
}

#[test]
fn a_clock_that_goes_backwards_neither_purges_nor_panics() {
    let mut t = tracker();
    bounce_b(&mut t, TAB, T0);
    assert!(due(&t, T0 - 5).is_empty());

    bounce_b(&mut t, OTHER_TAB, T0 - 500);

    assert_eq!(t.records(), vec![record("b.example", None, Some(T0 - 500))]);
    assert!(due(&t, T0 - 5).is_empty());
    assert_eq!(due(&t, T0 - 500 + HOUR), vec![site("b.example")]);
}

#[test]
fn a_hundred_hop_chain_keeps_at_most_max_chain_hops() {
    let cfg = BounceConfig::for_setting(Setting::Standard);
    let mut t = BounceTracker::new(cfg);
    t.navigated(TAB, Some(site("h0.example")), NavKind::User, T0);
    for i in 1..=100 {
        let key = format!("h{i}.example");
        t.navigated(TAB, Some(site(&key)), NavKind::Redirect, T0);
        t.storage_accessed(TAB, &site(&key));
    }
    t.tab_closed(TAB, T0);

    assert!(t.records().len() <= cfg.max_chain);
    assert!(!t.records().is_empty());
}

#[test]
fn ten_thousand_activations_keep_at_most_max_sites_records_and_keep_pending_bounces() {
    let cfg = BounceConfig::for_setting(Setting::Standard);
    let mut t = BounceTracker::new(cfg);
    bounce_b(&mut t, TAB, T0);
    for i in 0..10_000 {
        let key = format!("s{i}.example");
        t.user_activated(&site(&key), T0 + 1 + i);
    }

    assert!(t.records().len() <= cfg.max_sites);
    assert!(
        t.records()
            .iter()
            .any(|r| r.site == site("b.example") && r.bounced_at_ms == Some(T0))
    );

    t.prune(T0 + 20_000 + DAYS_45);

    assert_eq!(t.records(), vec![record("b.example", None, Some(T0))]);
    assert_eq!(due(&t, T0 + HOUR), vec![site("b.example")]);
}

#[test]
fn records_round_trip_through_from_records() {
    let cfg = BounceConfig::for_setting(Setting::Standard);
    let mut t = BounceTracker::new(cfg);
    t.user_activated(&site("x.example"), T0);
    bounce_b(&mut t, TAB, T0 + 5);

    let rebuilt = BounceTracker::from_records(cfg, t.records());

    assert_eq!(rebuilt.records(), t.records());
    assert_eq!(due(&rebuilt, T0 + 5 + HOUR), vec![site("b.example")]);
}

#[test]
fn the_grace_period_follows_the_setting() {
    assert_eq!(
        BounceConfig::for_setting(Setting::Strict).grace_period_ms,
        600_000
    );
    assert_eq!(
        BounceConfig::for_setting(Setting::Standard).grace_period_ms,
        HOUR
    );
    assert_eq!(
        BounceConfig::for_setting(Setting::Off),
        BounceConfig::for_setting(Setting::Standard)
    );

    let mut strict = BounceTracker::new(BounceConfig::for_setting(Setting::Strict));
    bounce_b(&mut strict, TAB, T0);
    assert!(due(&strict, T0 + 599_999).is_empty());
    assert_eq!(due(&strict, T0 + 600_000), vec![site("b.example")]);
}
