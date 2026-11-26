// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use mercury_settings::Settings;
use mercury_site_settings::{
    Axis, LegacyReport, LegacySources, Lifetime, Origin, OriginPattern, Setting, SiteSettings,
    UnixMillis, Value,
};
use mercury_store::{SiteSettingChange, Store};

const NOW: UnixMillis = 1_800_000_000_000;

fn sources<'a>(
    never_save: &'a [String],
    block_popups: bool,
    default_zoom_percent: u16,
) -> LegacySources<'a> {
    LegacySources {
        never_save_password_origins: never_save,
        block_popups,
        default_zoom_percent,
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn a_never_save_origin_becomes_a_permanent_block_of_save_passwords() {
    let mut s = SiteSettings::new();
    let list = strings(&["https://bank.example", "http://localhost:8080"]);

    let report = s.import_legacy(&sources(&list, true, 100), NOW);

    assert_eq!(report.never_save_imported, 2);
    let o = Origin::parse("https://bank.example").expect("origin");
    assert_eq!(
        s.permission(Some(&o), Axis::SavePasswords, NOW),
        Setting::Block
    );
    let rules = s.rules(NOW);
    assert!(
        rules
            .iter()
            .all(|r| r.lifetime == Lifetime::Permanent && r.axis == Axis::SavePasswords)
    );
}

#[test]
fn invalid_entries_are_skipped_and_counted() {
    let mut s = SiteSettings::new();
    let list = strings(&[
        "",
        "not a url",
        "ftp://x.com",
        "https://a.com/path",
        "HTTPS://A.COM",
    ]);

    let report = s.import_legacy(&sources(&list, true, 100), NOW);

    assert_eq!(report.never_save_imported, 1);
    assert_eq!(report.never_save_skipped, 4);
    let rules = s.rules(NOW);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].pattern.to_string(), "https://a.com");
}

#[test]
fn duplicates_count_once() {
    let mut s = SiteSettings::new();
    let list = strings(&["https://a.com", "https://a.com:443", "https://a.com"]);

    let report = s.import_legacy(&sources(&list, true, 100), NOW);

    assert_eq!(report.never_save_imported, 1);
    assert_eq!(report.never_save_skipped, 2);
}

#[test]
fn an_existing_rule_is_never_overwritten() {
    let mut s = SiteSettings::new();
    let pattern = OriginPattern::parse("https://bank.example").expect("pattern");
    s.set(
        &pattern,
        Axis::SavePasswords,
        Value::Permission(Setting::Allow),
        Lifetime::Permanent,
        NOW,
    )
    .expect("set");
    let list = strings(&["https://bank.example"]);

    let report = s.import_legacy(&sources(&list, true, 100), NOW);

    assert_eq!(report.never_save_imported, 0);
    assert_eq!(report.never_save_skipped, 1);
    let o = Origin::parse("https://bank.example").expect("origin");
    assert_eq!(
        s.permission(Some(&o), Axis::SavePasswords, NOW),
        Setting::Allow
    );
}

#[test]
fn block_popups_true_and_zoom_100_write_nothing() {
    let mut s = SiteSettings::new();

    let report = s.import_legacy(&sources(&[], true, 100), NOW);

    assert!(!report.popups_default_written);
    assert!(!report.zoom_default_written);
    assert!(s.rules(NOW).is_empty());
    assert_eq!(
        s.default_for(Axis::Popups, NOW),
        Axis::Popups.builtin_default()
    );
}

#[test]
fn block_popups_false_writes_a_permanent_allow_default_and_zoom_125_a_zoom_default() {
    let mut s = SiteSettings::new();

    let report = s.import_legacy(&sources(&[], false, 125), NOW);

    assert!(report.popups_default_written);
    assert!(report.zoom_default_written);
    assert_eq!(
        s.default_for(Axis::Popups, NOW),
        Value::Permission(Setting::Allow)
    );
    assert_eq!(s.default_for(Axis::Zoom, NOW), Value::Zoom(125));
}

#[test]
fn the_import_runs_once() {
    let mut s = SiteSettings::new();
    let first = strings(&["https://a.com"]);
    s.import_legacy(&sources(&first, true, 100), NOW);
    s.take_changes();

    let second = strings(&["https://b.com"]);
    let report: LegacyReport = s.import_legacy(&sources(&second, false, 150), NOW + 1);

    assert!(report.already_done);
    assert_eq!(report.never_save_imported, 0);
    assert!(!report.popups_default_written);
    assert!(!report.zoom_default_written);
    assert!(s.legacy_import_done());
    assert!(s.take_changes().is_empty());
    assert_eq!(s.rules(NOW).len(), 1);
}

#[test]
fn the_marker_reaches_the_journal_and_survives_a_reload() {
    let mut s = SiteSettings::new();
    s.import_legacy(&sources(&[], true, 100), NOW);

    let mut store = Store::open_in_memory().expect("store");
    let changes = s.take_changes();
    assert!(changes.iter().any(|c| matches!(
        c,
        SiteSettingChange::SetMeta { key, value } if key == "legacy_import_v1" && value == "1"
    )));
    store.apply_site_setting_changes(&changes).expect("apply");

    let (reloaded, _) = SiteSettings::load(store.site_settings_snapshot().expect("snap"), NOW);
    assert!(reloaded.legacy_import_done());
}

#[test]
fn a_settings_json_text_imports_end_to_end() {
    let json = r#"{
        "passwords": { "never_save_origins": ["https://bank.example", "http://localhost:8080"] },
        "privacy": { "block_popups": false },
        "appearance": { "default_zoom_percent": 110 }
    }"#;
    let parsed: Settings = serde_json::from_str(json).expect("settings json");
    let mut s = SiteSettings::new();

    let report = s.import_legacy(
        &sources(
            &parsed.passwords.never_save_origins,
            parsed.privacy.block_popups,
            parsed.appearance.default_zoom_percent,
        ),
        NOW,
    );

    assert_eq!(report.never_save_imported, 2);
    assert!(report.popups_default_written);
    assert!(report.zoom_default_written);
    assert_eq!(s.default_for(Axis::Zoom, NOW), Value::Zoom(110));
}

#[test]
fn more_than_a_thousand_origins_are_all_handled() {
    let mut s = SiteSettings::new();
    let list: Vec<String> = (0..1_000)
        .map(|i| format!("https://s{i}.example"))
        .collect();

    let report = s.import_legacy(&sources(&list, true, 100), NOW);

    assert_eq!(report.never_save_imported, 1_000);
    assert_eq!(s.rules(NOW).len(), 1_000);
}
