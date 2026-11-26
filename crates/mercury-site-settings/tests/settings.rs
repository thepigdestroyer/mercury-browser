// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Rules, how a question about a site is answered, lists, loading and the journal.

use mercury_site_settings::{
    AutoplayPolicy, Axis, DataRule, Error, HOUR_MS, Lifetime, LoadReport, Origin, OriginPattern,
    Setting, SiteSettings, Source, Value,
};
use mercury_store::{ProtocolHandlerRow, SiteSettingChange, SiteSettingRow, SiteSettingsSnapshot};

const NOW: i64 = 1_700_000_000_000;

fn origin(text: &str) -> Origin {
    Origin::parse(text).unwrap_or_else(|error| panic!("{text}: {error:?}"))
}

fn pattern(text: &str) -> OriginPattern {
    OriginPattern::parse(text).unwrap_or_else(|error| panic!("{text}: {error:?}"))
}

fn allow() -> Value {
    Value::Permission(Setting::Allow)
}

fn block() -> Value {
    Value::Permission(Setting::Block)
}

fn row(pattern: &str, axis: &str, value: &str) -> SiteSettingRow {
    SiteSettingRow {
        pattern: pattern.into(),
        axis: axis.into(),
        qualifier: String::new(),
        value: value.into(),
        set_at: NOW - 1_000,
        expires_at: None,
        last_used_at: None,
        revoke_exempt: false,
    }
}

fn snapshot(rows: Vec<SiteSettingRow>) -> SiteSettingsSnapshot {
    SiteSettingsSnapshot {
        rows,
        handlers: Vec::new(),
        meta: Vec::new(),
    }
}

#[test]
fn an_empty_model_answers_with_the_builtin_defaults() {
    let model = SiteSettings::new();
    let site = origin("https://example.com");
    let resolved = model.resolve(Some(&site), Axis::Camera, NOW);
    assert_eq!(resolved.value, Value::Permission(Setting::Ask));
    assert_eq!(resolved.source, Source::Builtin);
    assert_eq!(model.zoom_percent(Some(&site), NOW), 100);
    assert_eq!(
        model.autoplay(Some(&site), NOW),
        AutoplayPolicy::BlockAudible
    );
    assert_eq!(model.data_rule(Some(&site), NOW), DataRule::FollowGlobal);
    assert_eq!(
        model.permission(Some(&site), Axis::Popups, NOW),
        Setting::Block
    );
    assert_eq!(model.permission(None, Axis::Camera, NOW), Setting::Ask);
}

#[test]
fn the_more_specific_rule_wins() {
    let mut model = SiteSettings::new();
    model
        .set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("site rule");
    model
        .set(
            &pattern("*.a.example.com"),
            Axis::Camera,
            block(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("subdomain rule");
    model
        .set(
            &pattern("https://x.a.example.com"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("origin rule");

    assert_eq!(
        model.permission(Some(&origin("https://b.example.com")), Axis::Camera, NOW),
        Setting::Allow
    );
    assert_eq!(
        model.permission(Some(&origin("https://y.a.example.com")), Axis::Camera, NOW),
        Setting::Block
    );
    assert_eq!(
        model.permission(Some(&origin("https://x.a.example.com")), Axis::Camera, NOW),
        Setting::Allow
    );
}

#[test]
fn the_user_default_applies_until_a_site_has_its_own_rule() {
    let mut model = SiteSettings::new();
    model
        .set_default(Axis::Camera, block(), NOW)
        .expect("default");
    let other = origin("https://other.org");
    assert_eq!(model.default_for(Axis::Camera, NOW), block());
    assert_eq!(
        model.permission(Some(&other), Axis::Camera, NOW),
        Setting::Block
    );

    model
        .set(
            &pattern("site:other.org"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("site rule");
    assert_eq!(
        model.permission(Some(&other), Axis::Camera, NOW),
        Setting::Allow
    );
}

#[test]
fn the_resolution_reports_the_rule_it_came_from() {
    let mut model = SiteSettings::new();
    model
        .set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("site rule");
    let resolved = model.resolve(Some(&origin("https://example.com")), Axis::Camera, NOW);
    match resolved.source {
        Source::Rule(view) => {
            assert_eq!(view.pattern, pattern("site:example.com"));
            assert_eq!(view.axis, Axis::Camera);
            assert_eq!(view.value, allow());
            assert_eq!(view.set_at, NOW);
        }
        Source::Builtin => panic!("expected a rule"),
    }
}

#[test]
fn a_timed_rule_applies_up_to_its_end_and_no_further() {
    let mut model = SiteSettings::new();
    let site = origin("https://example.com");
    model
        .set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::for_duration(NOW, HOUR_MS),
            NOW,
        )
        .expect("timed rule");
    assert_eq!(
        model.permission(Some(&site), Axis::Camera, NOW + HOUR_MS - 1),
        Setting::Allow
    );
    assert_eq!(
        model.permission(Some(&site), Axis::Camera, NOW + HOUR_MS),
        Setting::Ask
    );
    assert_eq!(model.rules(NOW + HOUR_MS).len(), 0);
}

#[test]
fn a_this_visit_rule_is_never_written() {
    let mut model = SiteSettings::new();
    model
        .set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::ThisVisit,
            NOW,
        )
        .expect("this-visit rule");
    assert_eq!(
        model.permission(Some(&origin("https://example.com")), Axis::Camera, NOW),
        Setting::Allow
    );
    assert!(!model.has_pending_changes());
}

#[test]
fn a_permanent_rule_is_journaled_once_per_change() {
    let mut model = SiteSettings::new();
    model
        .set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("rule");
    let changes = model.take_changes();
    assert_eq!(changes.len(), 1);
    match &changes[0] {
        SiteSettingChange::Put(row) => {
            assert_eq!(row.pattern, "site:example.com");
            assert_eq!(row.axis, "camera");
            assert_eq!(row.value, "allow");
            assert_eq!(row.set_at, NOW);
            assert_eq!(row.expires_at, None);
        }
        other => panic!("unexpected change {other:?}"),
    }
    assert!(!model.has_pending_changes());
}

#[test]
fn a_timed_rule_is_journaled_with_its_end() {
    let mut model = SiteSettings::new();
    let end = NOW + HOUR_MS;
    model
        .set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::Until(end),
            NOW,
        )
        .expect("timed rule");
    match model.take_changes().as_slice() {
        [SiteSettingChange::Put(row)] => assert_eq!(row.expires_at, Some(end)),
        other => panic!("unexpected changes {other:?}"),
    }
}

#[test]
fn set_refuses_a_value_the_axis_does_not_take() {
    let mut model = SiteSettings::new();
    let site = pattern("site:example.com");
    assert_eq!(
        model.set(
            &site,
            Axis::Camera,
            Value::Zoom(100),
            Lifetime::Permanent,
            NOW
        ),
        Err(Error::ValueDoesNotFit)
    );
    assert_eq!(
        model.set(
            &site,
            Axis::Popups,
            Value::Permission(Setting::Ask),
            Lifetime::Permanent,
            NOW
        ),
        Err(Error::ValueDoesNotFit)
    );
    assert_eq!(
        model.set(
            &site,
            Axis::Zoom,
            Value::Zoom(600),
            Lifetime::Permanent,
            NOW
        ),
        Err(Error::ValueDoesNotFit)
    );
    assert_eq!(
        model.set(
            &site,
            Axis::SiteData,
            Value::Data(DataRule::FollowGlobal),
            Lifetime::Permanent,
            NOW
        ),
        Err(Error::ValueDoesNotFit)
    );
    assert!(!model.has_pending_changes());
    assert!(model.rules(NOW).is_empty());
}

#[test]
fn set_refuses_a_lifetime_that_has_already_ended() {
    let mut model = SiteSettings::new();
    assert_eq!(
        model.set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::Until(NOW),
            NOW
        ),
        Err(Error::LifetimeEnded)
    );
}

#[test]
fn an_external_link_rule_needs_a_scheme_and_other_axes_refuse_one() {
    let mut model = SiteSettings::new();
    let site = pattern("site:example.com");
    assert_eq!(
        model.set(
            &site,
            Axis::ExternalLinks,
            allow(),
            Lifetime::Permanent,
            NOW
        ),
        Err(Error::QualifierRequired)
    );
    assert_eq!(
        model.set_qualified(
            &site,
            Axis::Camera,
            "mailto",
            allow(),
            Lifetime::Permanent,
            NOW
        ),
        Err(Error::QualifierNotAllowed)
    );
    assert_eq!(
        model.set_qualified(
            &site,
            Axis::ExternalLinks,
            "Mailto",
            allow(),
            Lifetime::Permanent,
            NOW
        ),
        Err(Error::InvalidQualifier)
    );
    model
        .set_qualified(
            &site,
            Axis::ExternalLinks,
            "mailto",
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("mailto rule");
    let site_origin = origin("https://example.com");
    assert_eq!(
        model
            .resolve_qualified(Some(&site_origin), Axis::ExternalLinks, "mailto", NOW)
            .value,
        allow()
    );
    assert_eq!(
        model
            .resolve_qualified(Some(&site_origin), Axis::ExternalLinks, "tel", NOW)
            .source,
        Source::Builtin
    );
}

#[test]
fn remove_reports_whether_there_was_a_rule_and_journals_only_stored_ones() {
    let mut model = SiteSettings::new();
    let site = pattern("site:example.com");
    model
        .set(&site, Axis::Camera, allow(), Lifetime::ThisVisit, NOW)
        .expect("visit rule");
    assert!(model.remove(&site, Axis::Camera, ""));
    assert!(!model.remove(&site, Axis::Camera, ""));
    assert!(!model.has_pending_changes());

    model
        .set(&site, Axis::Camera, allow(), Lifetime::Permanent, NOW)
        .expect("permanent rule");
    model.take_changes();
    assert!(model.remove(&site, Axis::Camera, ""));
    assert_eq!(
        model.take_changes(),
        vec![SiteSettingChange::Remove {
            pattern: "site:example.com".into(),
            axis: "camera".into(),
            qualifier: String::new(),
        }]
    );
}

#[test]
fn reset_pattern_removes_only_that_pattern_and_journals_it() {
    let mut model = SiteSettings::new();
    let site = pattern("site:example.com");
    model
        .set(&site, Axis::Camera, allow(), Lifetime::Permanent, NOW)
        .expect("camera");
    model
        .set(&site, Axis::Microphone, block(), Lifetime::Permanent, NOW)
        .expect("microphone");
    model
        .set(
            &pattern("site:other.org"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("other");
    model.take_changes();

    assert_eq!(model.reset_pattern(&site), 2);
    assert_eq!(model.rules(NOW).len(), 1);
    assert_eq!(
        model.take_changes(),
        vec![SiteSettingChange::RemovePattern {
            pattern: "site:example.com".into(),
        }]
    );
    assert_eq!(model.reset_pattern(&site), 0);
    assert!(!model.has_pending_changes());
}

#[test]
fn reset_all_clears_every_rule_including_the_default() {
    let mut model = SiteSettings::new();
    model
        .set_default(Axis::Camera, block(), NOW)
        .expect("default");
    model
        .set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("site");
    model.take_changes();
    model.reset_all();
    assert!(model.rules(NOW).is_empty());
    assert_eq!(model.take_changes(), vec![SiteSettingChange::RemoveAll]);
}

#[test]
fn effective_for_origin_lists_the_rules_that_apply_to_it_without_the_default() {
    let mut model = SiteSettings::new();
    model
        .set_default(Axis::Camera, block(), NOW)
        .expect("default");
    model
        .set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("site");
    model
        .set(
            &pattern("site:example.com"),
            Axis::Zoom,
            Value::Zoom(125),
            Lifetime::Permanent,
            NOW,
        )
        .expect("zoom");
    model
        .set(
            &pattern("site:other.org"),
            Axis::Microphone,
            block(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("elsewhere");

    let views = model.effective_for_origin(&origin("https://example.com"), NOW);
    let axes: Vec<Axis> = views.iter().map(|view| view.axis).collect();
    assert_eq!(axes, vec![Axis::Camera, Axis::Zoom]);
    assert_eq!(views[0].value, allow());
    assert_eq!(views[1].value, Value::Zoom(125));
}

#[test]
fn rules_for_axis_include_the_default_under_the_all_sites_pattern() {
    let mut model = SiteSettings::new();
    model
        .set_default(Axis::Camera, block(), NOW)
        .expect("default");
    model
        .set(
            &pattern("site:example.com"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("site");
    model
        .set(
            &pattern("site:example.com"),
            Axis::Microphone,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("microphone");

    let camera = model.rules_for_axis(Axis::Camera, NOW);
    let patterns: Vec<String> = camera.iter().map(|view| view.pattern.to_string()).collect();
    assert_eq!(
        patterns,
        vec!["*".to_owned(), "site:example.com".to_owned()]
    );
    assert_eq!(
        model
            .rules_for_pattern(&pattern("site:example.com"), NOW)
            .len(),
        2
    );
}

#[test]
fn load_builds_rules_and_counts_what_it_cannot_use() {
    let mut expiring = row("site:old.org", "camera", "allow");
    expiring.expires_at = Some(NOW - 1);
    let mut snapshot = snapshot(vec![
        row("site:example.com", "camera", "allow"),
        row("*", "zoom", "zoom:125"),
        row("site:example.com", "teleport", "allow"),
        row("site:example.com", "popups", "ask"),
        row("not a pattern", "camera", "allow"),
        expiring,
    ]);
    snapshot.meta.push(("import_done".into(), "1".into()));
    snapshot.handlers.push(ProtocolHandlerRow {
        scheme: "mailto".into(),
        origin: "https://mail.a.com".into(),
        url_template: "https://mail.a.com/?to=%s".into(),
        title: "Mail".into(),
        added_at: NOW,
        last_used_at: None,
        is_default: true,
    });

    let (mut model, report) = SiteSettings::load(snapshot, NOW);
    assert_eq!(
        report,
        LoadReport {
            loaded: 2,
            expired: 1,
            ignored: 3,
        }
    );
    assert_eq!(
        model.permission(Some(&origin("https://example.com")), Axis::Camera, NOW),
        Setting::Allow
    );
    assert_eq!(
        model.zoom_percent(Some(&origin("https://anywhere.net")), NOW),
        125
    );
    assert_eq!(model.handler_count(), 1);
    assert_eq!(model.meta("import_done"), Some("1"));
    assert_eq!(
        model.take_changes(),
        vec![SiteSettingChange::Remove {
            pattern: "site:old.org".into(),
            axis: "camera".into(),
            qualifier: String::new(),
        }]
    );
}

#[test]
fn load_leaves_unusable_rows_in_the_file_and_never_turns_them_into_rules() {
    let bad_value = row("site:example.com", "camera", "zoom:125");
    let (model, report) = SiteSettings::load(snapshot(vec![bad_value]), NOW);
    assert_eq!(report.ignored, 1);
    assert!(model.rules(NOW).is_empty());
    assert!(!model.has_pending_changes());
}

#[test]
fn reset_pattern_also_removes_rows_this_build_cannot_read() {
    let (mut model, report) = SiteSettings::load(
        snapshot(vec![row("site:example.com", "teleport", "allow")]),
        NOW,
    );
    assert_eq!(report.ignored, 1);
    assert_eq!(model.reset_pattern(&pattern("site:example.com")), 0);
    assert_eq!(
        model.take_changes(),
        vec![SiteSettingChange::RemovePattern {
            pattern: "site:example.com".into(),
        }]
    );
}

#[test]
fn restored_changes_go_back_ahead_of_newer_ones() {
    let mut model = SiteSettings::new();
    let first = SiteSettingChange::RemoveAll;
    model
        .set(
            &pattern("site:a.com"),
            Axis::Camera,
            allow(),
            Lifetime::Permanent,
            NOW,
        )
        .expect("rule");
    model.restore_changes(vec![first.clone()]);
    let changes = model.take_changes();
    assert_eq!(changes[0], first);
    assert!(matches!(changes[1], SiteSettingChange::Put(_)));
}
