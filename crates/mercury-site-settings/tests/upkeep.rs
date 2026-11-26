// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use mercury_site_settings::{
    AUTO_REVOKE_AFTER_MS, Axis, DAY_MS, LAST_USED_GRANULARITY_MS, Lifetime, Origin, OriginPattern,
    Setting, SiteSettings, Source, UnixMillis, Value,
};
use mercury_store::{SiteSettingChange, Store};

const NOW: UnixMillis = 1_800_000_000_000;

fn origin(text: &str) -> Origin {
    Origin::parse(text).expect("origin")
}

fn pattern(text: &str) -> OriginPattern {
    OriginPattern::parse(text).expect("pattern")
}

fn allow() -> Value {
    Value::Permission(Setting::Allow)
}

fn block() -> Value {
    Value::Permission(Setting::Block)
}

fn listed_after(
    axis: Axis,
    value: Value,
    lifetime: Lifetime,
    set_at: UnixMillis,
    used_at: Option<UnixMillis>,
    exempt: bool,
) -> bool {
    let o = origin("https://www.a.com");
    let p = pattern("site:a.com");
    let mut s = SiteSettings::new();
    s.set(&p, axis, value, lifetime, set_at).expect("set");
    if let Some(used) = used_at {
        s.record_use(&o, axis, used);
    }
    if exempt {
        assert!(s.set_revoke_exempt(&p, axis, true));
    }
    !s.unused_since(NOW, AUTO_REVOKE_AFTER_MS).is_empty()
}

#[test]
fn end_visit_removes_this_visit_rules_whose_pattern_matches_the_origin_only() {
    let mut s = SiteSettings::new();
    let o = origin("https://www.a.com");
    s.set(
        &OriginPattern::for_origin(&o),
        Axis::Camera,
        allow(),
        Lifetime::ThisVisit,
        NOW,
    )
    .expect("set");
    s.set(
        &pattern("site:a.com"),
        Axis::Microphone,
        allow(),
        Lifetime::ThisVisit,
        NOW,
    )
    .expect("set");
    s.set(
        &pattern("site:b.com"),
        Axis::Camera,
        allow(),
        Lifetime::ThisVisit,
        NOW,
    )
    .expect("set");
    s.set(
        &pattern("site:a.com"),
        Axis::Location,
        allow(),
        Lifetime::Session,
        NOW,
    )
    .expect("set");

    assert_eq!(s.end_visit(&o), 2);

    let mut left: Vec<String> = s
        .rules(NOW)
        .iter()
        .map(|r| format!("{} {}", r.pattern, r.axis.id()))
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            format!("site:a.com {}", Axis::Location.id()),
            format!("site:b.com {}", Axis::Camera.id()),
        ]
    );
}

#[test]
fn end_visit_reveals_the_stored_rule_again() {
    let mut s = SiteSettings::new();
    let o = origin("https://www.a.com");
    s.set(
        &pattern("site:a.com"),
        Axis::Camera,
        block(),
        Lifetime::Permanent,
        NOW,
    )
    .expect("set");
    s.set(
        &OriginPattern::for_origin(&o),
        Axis::Camera,
        allow(),
        Lifetime::ThisVisit,
        NOW,
    )
    .expect("set");
    assert_eq!(s.permission(Some(&o), Axis::Camera, NOW), Setting::Allow);

    assert_eq!(s.end_visit(&o), 1);

    assert_eq!(s.permission(Some(&o), Axis::Camera, NOW), Setting::Block);
    match s.resolve(Some(&o), Axis::Camera, NOW).source {
        Source::Rule(view) => assert_eq!(view.pattern.to_string(), "site:a.com"),
        Source::Builtin => panic!("the stored rule should answer"),
    }
}

#[test]
fn sweep_removes_expired_persistent_rules_and_journals_removal() {
    let mut s = SiteSettings::new();
    s.set(
        &pattern("site:a.com"),
        Axis::Camera,
        allow(),
        Lifetime::Until(NOW + 1_000),
        NOW,
    )
    .expect("set");
    s.set(
        &pattern("site:b.com"),
        Axis::Camera,
        allow(),
        Lifetime::Permanent,
        NOW,
    )
    .expect("set");
    s.take_changes();

    assert_eq!(s.sweep(NOW + 999), 0);
    assert_eq!(s.sweep(NOW + 1_000), 1);
    assert_eq!(
        s.take_changes(),
        vec![SiteSettingChange::Remove {
            pattern: "site:a.com".to_owned(),
            axis: Axis::Camera.id().to_owned(),
            qualifier: String::new(),
        }]
    );
    assert_eq!(s.rules(NOW + 1_000).len(), 1);
}

#[test]
fn sweep_keeps_a_rule_whose_clock_is_behind_set_at() {
    let mut s = SiteSettings::new();
    s.set(
        &pattern("site:a.com"),
        Axis::Camera,
        allow(),
        Lifetime::Until(NOW + 1_500),
        NOW + 1_000,
    )
    .expect("set");

    assert_eq!(s.sweep(NOW), 0);
    assert_eq!(s.rules(NOW + 1_000).len(), 1);
}

#[test]
fn record_use_stamps_the_winning_stored_rule() {
    let mut s = SiteSettings::new();
    let o = origin("https://www.a.com");
    s.set(
        &pattern("site:a.com"),
        Axis::Camera,
        allow(),
        Lifetime::Permanent,
        NOW,
    )
    .expect("set");
    s.set(
        &OriginPattern::for_origin(&o),
        Axis::Camera,
        block(),
        Lifetime::Permanent,
        NOW,
    )
    .expect("set");
    s.take_changes();

    s.record_use(&o, Axis::Camera, NOW + 5);

    let changes = s.take_changes();
    let [SiteSettingChange::Put(row)] = changes.as_slice() else {
        panic!("one journal entry, got {changes:?}");
    };
    assert_eq!(row.pattern, OriginPattern::for_origin(&o).to_string());
    assert_eq!(row.last_used_at, Some(NOW + 5));
}

#[test]
fn record_use_writes_at_most_once_an_hour() {
    let mut s = SiteSettings::new();
    let o = origin("https://www.a.com");
    s.set(
        &pattern("site:a.com"),
        Axis::Camera,
        allow(),
        Lifetime::Permanent,
        NOW,
    )
    .expect("set");
    s.take_changes();

    s.record_use(&o, Axis::Camera, NOW);
    assert_eq!(s.take_changes().len(), 1);

    s.record_use(&o, Axis::Camera, NOW + LAST_USED_GRANULARITY_MS - 1);
    assert!(s.take_changes().is_empty());
    assert_eq!(s.rules(NOW)[0].last_used_at, Some(NOW));

    s.record_use(&o, Axis::Camera, NOW + LAST_USED_GRANULARITY_MS);
    assert_eq!(s.take_changes().len(), 1);
    assert_eq!(
        s.rules(NOW + LAST_USED_GRANULARITY_MS)[0].last_used_at,
        Some(NOW + LAST_USED_GRANULARITY_MS)
    );
}

#[test]
fn record_use_ignores_the_builtin_answer_but_stamps_a_stored_default() {
    let mut s = SiteSettings::new();
    let o = origin("https://www.a.com");

    s.record_use(&o, Axis::Camera, NOW);
    assert!(s.take_changes().is_empty());

    s.set_default(Axis::Popups, allow(), NOW)
        .expect("set_default");
    s.take_changes();
    s.record_use(&o, Axis::Popups, NOW);
    let changes = s.take_changes();
    let [SiteSettingChange::Put(row)] = changes.as_slice() else {
        panic!("one journal entry, got {changes:?}");
    };
    assert_eq!(row.pattern, "*");
    assert_eq!(row.last_used_at, Some(NOW));
}

#[test]
fn unused_since_lists_only_permanent_allow_rules_of_revocable_axes() {
    let old = NOW - AUTO_REVOKE_AFTER_MS - DAY_MS;
    let edge = NOW - AUTO_REVOKE_AFTER_MS;
    let recent = NOW - 89 * DAY_MS;
    let ten_days_ago = NOW - 10 * DAY_MS;

    assert!(!Axis::Zoom.auto_revocable());
    assert!(listed_after(
        Axis::Camera,
        allow(),
        Lifetime::Permanent,
        old,
        None,
        false
    ));
    assert!(listed_after(
        Axis::Camera,
        allow(),
        Lifetime::Permanent,
        edge,
        None,
        false
    ));
    assert!(!listed_after(
        Axis::Camera,
        allow(),
        Lifetime::Permanent,
        recent,
        None,
        false
    ));
    assert!(!listed_after(
        Axis::Camera,
        allow(),
        Lifetime::Permanent,
        old,
        Some(ten_days_ago),
        false
    ));
    assert!(!listed_after(
        Axis::Camera,
        block(),
        Lifetime::Permanent,
        old,
        None,
        false
    ));
    assert!(!listed_after(
        Axis::Camera,
        allow(),
        Lifetime::Until(NOW + DAY_MS),
        old,
        None,
        false
    ));
    assert!(!listed_after(
        Axis::Camera,
        allow(),
        Lifetime::ThisVisit,
        old,
        None,
        false
    ));
    assert!(!listed_after(
        Axis::Camera,
        allow(),
        Lifetime::Permanent,
        old,
        None,
        true
    ));
}

#[test]
fn revoke_unused_sets_ask_keeps_the_pattern_and_returns_the_views_as_they_were() {
    let mut s = SiteSettings::new();
    let o = origin("https://www.a.com");
    let old = NOW - AUTO_REVOKE_AFTER_MS - DAY_MS;
    s.set(
        &pattern("site:a.com"),
        Axis::Camera,
        allow(),
        Lifetime::Permanent,
        old,
    )
    .expect("set");
    s.take_changes();

    let views = s.revoke_unused(NOW, AUTO_REVOKE_AFTER_MS);

    assert_eq!(views.len(), 1);
    assert_eq!(views[0].value, allow());
    assert_eq!(views[0].set_at, old);
    let rules = s.rules(NOW);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].pattern.to_string(), "site:a.com");
    assert_eq!(rules[0].value, Value::Permission(Setting::Ask));
    assert_eq!(rules[0].set_at, NOW);
    assert_eq!(s.permission(Some(&o), Axis::Camera, NOW), Setting::Ask);
    let changes = s.take_changes();
    let [SiteSettingChange::Put(row)] = changes.as_slice() else {
        panic!("one journal entry, got {changes:?}");
    };
    assert_eq!(row.value, Value::Permission(Setting::Ask).encode());
}

#[test]
fn set_revoke_exempt_returns_false_for_a_missing_rule_and_survives_a_drain_and_reload() {
    let mut s = SiteSettings::new();
    let p = pattern("site:a.com");
    let old = NOW - AUTO_REVOKE_AFTER_MS - DAY_MS;

    assert!(!s.set_revoke_exempt(&p, Axis::Camera, true));

    s.set(&p, Axis::Camera, allow(), Lifetime::Permanent, old)
        .expect("set");
    assert!(s.set_revoke_exempt(&p, Axis::Camera, true));
    assert!(s.unused_since(NOW, AUTO_REVOKE_AFTER_MS).is_empty());

    let mut store = Store::open_in_memory().expect("store");
    store
        .apply_site_setting_changes(&s.take_changes())
        .expect("apply");
    let snapshot = store.site_settings_snapshot().expect("snapshot");
    let (reloaded, _) = SiteSettings::load(snapshot, NOW);

    assert!(reloaded.rules(NOW)[0].revoke_exempt);
    assert!(reloaded.unused_since(NOW, AUTO_REVOKE_AFTER_MS).is_empty());
}
