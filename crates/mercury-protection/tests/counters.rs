// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `TabProtection`: per-tab counts and the list of blocked hosts (MH14).

use mercury_protection::{
    BlockEvent, Category, MAX_BLOCKED_HOSTS, ResourceKind, TabKey, TabProtection,
};

fn event(tab: u64, category: Category, host: &str) -> BlockEvent {
    BlockEvent {
        tab: TabKey(tab),
        category,
        host: host.to_owned(),
        kind: ResourceKind::Script,
    }
}

fn hosts_of(p: &TabProtection, tab: u64) -> Vec<(String, Category, u64)> {
    p.counters(TabKey(tab))
        .expect("the tab has counters")
        .hosts()
        .map(|h| (h.host.to_owned(), h.category, h.count))
        .collect()
}

#[test]
fn two_tabs_never_share_counts() {
    let mut p = TabProtection::default();
    p.record(&event(1, Category::AdsAndTrackers, "ads.example"));
    p.record(&event(1, Category::AdsAndTrackers, "ads.example"));
    p.record(&event(2, Category::Cryptominers, "mine.example"));

    let one = p.counters(TabKey(1)).expect("tab 1");
    let two = p.counters(TabKey(2)).expect("tab 2");
    assert_eq!(one.total, 2);
    assert_eq!(two.total, 1);
    assert_eq!(one.by_category[Category::Cryptominers.index()], 0);
    assert_eq!(two.by_category[Category::AdsAndTrackers.index()], 0);
}

#[test]
fn record_increments_total_and_by_category() {
    let mut p = TabProtection::default();
    p.record(&event(1, Category::AdsAndTrackers, "ads.example"));
    p.record(&event(1, Category::AdsAndTrackers, "ads.example"));
    p.record(&event(1, Category::Cryptominers, "mine.example"));

    let c = p.counters(TabKey(1)).expect("tab 1");
    assert_eq!(c.total, 3);
    assert_eq!(c.by_category[Category::AdsAndTrackers.index()], 2);
    assert_eq!(c.by_category[Category::Cryptominers.index()], 1);
    assert_eq!(c.by_category.iter().sum::<u64>(), 3);
}

#[test]
fn host_list_keeps_first_seen_order_and_merges_the_same_host_and_category() {
    let mut p = TabProtection::default();
    p.record(&event(1, Category::AdsAndTrackers, "a.example"));
    p.record(&event(1, Category::SocialTrackers, "b.example"));
    p.record(&event(1, Category::AdsAndTrackers, "a.example"));
    p.record(&event(1, Category::Cryptominers, "a.example"));

    assert_eq!(
        hosts_of(&p, 1),
        vec![
            ("a.example".to_owned(), Category::AdsAndTrackers, 2),
            ("b.example".to_owned(), Category::SocialTrackers, 1),
            ("a.example".to_owned(), Category::Cryptominers, 1),
        ]
    );
}

#[test]
fn host_list_is_capped_while_totals_keep_counting() {
    assert_eq!(MAX_BLOCKED_HOSTS, 200);
    let mut p = TabProtection::default();
    for i in 0..250 {
        p.record(&event(
            1,
            Category::AdsAndTrackers,
            &format!("h{i}.example"),
        ));
    }

    let c = p.counters(TabKey(1)).expect("tab 1");
    assert_eq!(c.hosts().count(), 200);
    assert_eq!(c.total, 250);
    assert_eq!(c.by_category[Category::AdsAndTrackers.index()], 250);
    assert_eq!(c.hosts().next().map(|h| h.host), Some("h0.example"));
}

#[test]
fn document_change_zeroes_one_tab_only() {
    let mut p = TabProtection::default();
    p.record(&event(1, Category::AdsAndTrackers, "ads.example"));
    p.record(&event(2, Category::AdsAndTrackers, "ads.example"));

    p.document_changed(TabKey(1));

    let zeroed = p.counters(TabKey(1)).expect("tab 1 is still tracked");
    assert_eq!(zeroed.total, 0);
    assert_eq!(zeroed.hosts().count(), 0);
    assert_eq!(p.counters(TabKey(2)).map(|c| c.total), Some(1));
    p.document_changed(TabKey(9));
    assert!(p.counters(TabKey(9)).is_none());
}

#[test]
fn close_drops_the_tab() {
    let mut p = TabProtection::default();
    p.record(&event(1, Category::AdsAndTrackers, "ads.example"));
    p.record(&event(2, Category::AdsAndTrackers, "ads.example"));

    p.close(TabKey(1));

    assert!(p.counters(TabKey(1)).is_none());
    assert_eq!(p.counters(TabKey(2)).map(|c| c.total), Some(1));
}

#[test]
fn tabs_are_capped_and_the_least_recently_updated_is_evicted() {
    let mut p = TabProtection::default();
    for tab in 0..512 {
        p.record(&event(tab, Category::AdsAndTrackers, "ads.example"));
    }
    p.record(&event(0, Category::AdsAndTrackers, "ads.example"));
    p.record(&event(512, Category::AdsAndTrackers, "ads.example"));

    assert!(
        p.counters(TabKey(1)).is_none(),
        "tab 1 is the least recently updated"
    );
    assert!(p.counters(TabKey(0)).is_some());
    assert!(p.counters(TabKey(512)).is_some());
    let kept = (0..=512)
        .filter(|&tab| p.counters(TabKey(tab)).is_some())
        .count();
    assert_eq!(kept, 512);
}

#[test]
fn hosts_are_lower_cased_and_truncated_to_253_bytes() {
    let mut p = TabProtection::default();
    p.record(&event(1, Category::AdsAndTrackers, "WWW.Example.COM"));
    p.record(&event(1, Category::AdsAndTrackers, &"B".repeat(300)));

    let hosts = hosts_of(&p, 1);
    assert_eq!(hosts[0].0, "www.example.com");
    assert_eq!(hosts[1].0, "b".repeat(253));
    assert_eq!(hosts[1].0.len(), 253);
}
