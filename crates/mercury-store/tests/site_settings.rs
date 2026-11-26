// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
mod common;

use common::{NOW, TempDir};
use mercury_store::{
    MAX_HANDLER_TEMPLATE_BYTES, MAX_HANDLER_TITLE_CHARS, MAX_SITE_FIELD_BYTES,
    MAX_SITE_META_VALUE_BYTES, MAX_SITE_PATTERN_BYTES, ProtocolHandlerRow, SiteSettingChange,
    SiteSettingRow, Store, StoreError,
};

fn row(pattern: &str, axis: &str, qualifier: &str, value: &str) -> SiteSettingRow {
    SiteSettingRow {
        pattern: pattern.into(),
        axis: axis.into(),
        qualifier: qualifier.into(),
        value: value.into(),
        set_at: NOW,
        expires_at: None,
        last_used_at: None,
        revoke_exempt: false,
    }
}

fn put(pattern: &str, axis: &str, qualifier: &str, value: &str) -> SiteSettingChange {
    SiteSettingChange::Put(row(pattern, axis, qualifier, value))
}

fn handler(scheme: &str, origin: &str, is_default: bool) -> ProtocolHandlerRow {
    ProtocolHandlerRow {
        scheme: scheme.into(),
        origin: origin.into(),
        url_template: format!("{origin}/compose?to=%s"),
        title: "Mail".into(),
        added_at: NOW,
        last_used_at: None,
        is_default,
    }
}

fn keys(store: &Store) -> Vec<(String, String, String)> {
    store
        .site_settings_snapshot()
        .unwrap()
        .rows
        .into_iter()
        .map(|r| (r.pattern, r.axis, r.qualifier))
        .collect()
}

#[test]
fn a_new_store_has_an_empty_snapshot() {
    let store = Store::open_in_memory().unwrap();
    let snapshot = store.site_settings_snapshot().unwrap();
    assert!(snapshot.rows.is_empty());
    assert!(snapshot.handlers.is_empty());
    assert!(snapshot.meta.is_empty());
}

#[test]
fn put_then_snapshot_returns_the_row() {
    let mut store = Store::open_in_memory().unwrap();
    let full = SiteSettingRow {
        pattern: "https://a.com".into(),
        axis: "camera".into(),
        qualifier: String::new(),
        value: "allow".into(),
        set_at: NOW,
        expires_at: None,
        last_used_at: Some(NOW + 5),
        revoke_exempt: true,
    };
    let timed = SiteSettingRow {
        pattern: "site:b.com".into(),
        axis: "microphone".into(),
        qualifier: String::new(),
        value: "block".into(),
        set_at: NOW - 10,
        expires_at: Some(NOW + 1000),
        last_used_at: None,
        revoke_exempt: false,
    };
    store
        .apply_site_setting_changes(&[
            SiteSettingChange::Put(full.clone()),
            SiteSettingChange::Put(timed.clone()),
        ])
        .unwrap();
    let snapshot = store.site_settings_snapshot().unwrap();
    assert_eq!(snapshot.rows, [full, timed]);
    assert!(snapshot.handlers.is_empty());
    assert!(snapshot.meta.is_empty());
}

#[test]
fn put_twice_replaces_by_key() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .apply_site_setting_changes(&[put("https://a.com", "camera", "", "allow")])
        .unwrap();
    store
        .apply_site_setting_changes(&[put("https://a.com", "camera", "", "block")])
        .unwrap();
    let rows = store.site_settings_snapshot().unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value, "block");
}

#[test]
fn remove_deletes_one_key_remove_pattern_deletes_all_its_axes_remove_all_clears_rules_not_handlers_or_meta()
 {
    let mut store = Store::open_in_memory().unwrap();
    store
        .apply_site_setting_changes(&[
            put("https://a.com", "camera", "", "allow"),
            put("https://a.com", "microphone", "", "allow"),
            put("https://a.com", "external_links", "mailto", "allow"),
            put("https://a.com", "external_links", "tel", "allow"),
            put("site:b.com", "camera", "", "block"),
            put("*", "popups", "", "allow"),
            SiteSettingChange::PutHandler(handler("mailto", "https://mail.a.com", true)),
            SiteSettingChange::SetMeta {
                key: "legacy_import_v1".into(),
                value: "1".into(),
            },
        ])
        .unwrap();

    store
        .apply_site_setting_changes(&[SiteSettingChange::Remove {
            pattern: "https://a.com".into(),
            axis: "external_links".into(),
            qualifier: "tel".into(),
        }])
        .unwrap();
    assert_eq!(keys(&store).len(), 5);
    assert!(!keys(&store).contains(&(
        "https://a.com".into(),
        "external_links".into(),
        "tel".into()
    )));
    assert!(keys(&store).contains(&(
        "https://a.com".into(),
        "external_links".into(),
        "mailto".into()
    )));

    store
        .apply_site_setting_changes(&[SiteSettingChange::RemovePattern {
            pattern: "https://a.com".into(),
        }])
        .unwrap();
    assert_eq!(
        keys(&store),
        [
            ("*".to_string(), "popups".to_string(), String::new()),
            (
                "site:b.com".to_string(),
                "camera".to_string(),
                String::new()
            ),
        ]
    );

    store
        .apply_site_setting_changes(&[SiteSettingChange::RemoveAll])
        .unwrap();
    let snapshot = store.site_settings_snapshot().unwrap();
    assert!(snapshot.rows.is_empty());
    assert_eq!(snapshot.handlers.len(), 1);
    assert_eq!(
        snapshot.meta,
        [("legacy_import_v1".to_string(), "1".to_string())]
    );
}

#[test]
fn removing_a_row_that_is_not_there_is_not_an_error() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .apply_site_setting_changes(&[
            SiteSettingChange::Remove {
                pattern: "https://nope.com".into(),
                axis: "camera".into(),
                qualifier: String::new(),
            },
            SiteSettingChange::RemovePattern {
                pattern: "https://nope.com".into(),
            },
            SiteSettingChange::RemoveHandler {
                scheme: "mailto".into(),
                origin: "https://nope.com".into(),
            },
            SiteSettingChange::RemoveAll,
        ])
        .unwrap();
}

#[test]
fn handlers_put_replace_remove() {
    let mut store = Store::open_in_memory().unwrap();
    let first = handler("mailto", "https://mail.a.com", true);
    let second = handler("mailto", "https://mail.b.com", false);
    let tel = handler("tel", "https://mail.a.com", false);
    store
        .apply_site_setting_changes(&[
            SiteSettingChange::PutHandler(first.clone()),
            SiteSettingChange::PutHandler(second.clone()),
            SiteSettingChange::PutHandler(tel.clone()),
        ])
        .unwrap();
    assert_eq!(
        store.site_settings_snapshot().unwrap().handlers,
        [first.clone(), second.clone(), tel.clone()]
    );

    let replaced = ProtocolHandlerRow {
        url_template: "https://mail.b.com/new?u=%s".into(),
        title: "Mail B".into(),
        added_at: NOW + 1,
        last_used_at: Some(NOW + 2),
        is_default: true,
        ..second
    };
    store
        .apply_site_setting_changes(&[SiteSettingChange::PutHandler(replaced.clone())])
        .unwrap();
    let handlers = store.site_settings_snapshot().unwrap().handlers;
    assert_eq!(handlers, [first.clone(), replaced, tel.clone()]);

    store
        .apply_site_setting_changes(&[SiteSettingChange::RemoveHandler {
            scheme: "mailto".into(),
            origin: "https://mail.b.com".into(),
        }])
        .unwrap();
    assert_eq!(
        store.site_settings_snapshot().unwrap().handlers,
        [first, tel]
    );
}

#[test]
fn meta_set_overwrites() {
    let mut store = Store::open_in_memory().unwrap();
    let set = |key: &str, value: &str| SiteSettingChange::SetMeta {
        key: key.into(),
        value: value.into(),
    };
    store
        .apply_site_setting_changes(&[set("b", "1"), set("a", "1")])
        .unwrap();
    store.apply_site_setting_changes(&[set("a", "2")]).unwrap();
    assert_eq!(
        store.site_settings_snapshot().unwrap().meta,
        [
            ("a".to_string(), "2".to_string()),
            ("b".to_string(), "1".to_string())
        ]
    );
}

#[test]
fn delete_expired_removes_only_rows_with_expires_at_at_or_before_now() {
    let mut store = Store::open_in_memory().unwrap();
    let timed = |pattern: &str, expires_at: i64| {
        SiteSettingChange::Put(SiteSettingRow {
            expires_at: Some(expires_at),
            ..row(pattern, "camera", "", "allow")
        })
    };
    store
        .apply_site_setting_changes(&[
            timed("https://before.com", NOW - 1),
            timed("https://at.com", NOW),
            timed("https://after.com", NOW + 1),
            put("https://forever.com", "camera", "", "allow"),
        ])
        .unwrap();
    assert_eq!(store.delete_expired_site_settings(NOW).unwrap(), 2);
    let remaining: Vec<String> = store
        .site_settings_snapshot()
        .unwrap()
        .rows
        .into_iter()
        .map(|r| r.pattern)
        .collect();
    assert_eq!(remaining, ["https://after.com", "https://forever.com"]);
    assert_eq!(store.delete_expired_site_settings(NOW).unwrap(), 0);
}

#[test]
fn a_batch_with_one_oversized_field_changes_nothing() {
    let mut store = Store::open_in_memory().unwrap();
    let oversized = "p".repeat(MAX_SITE_PATTERN_BYTES + 1);
    let result = store.apply_site_setting_changes(&[
        put("https://a.com", "camera", "", "allow"),
        put(&oversized, "camera", "", "allow"),
    ]);
    assert!(matches!(result, Err(StoreError::InvalidArgument(_))));
    assert!(store.site_settings_snapshot().unwrap().rows.is_empty());
}

#[test]
fn every_text_field_is_bounded() {
    let mut store = Store::open_in_memory().unwrap();
    let field = |n: usize| "x".repeat(n);
    let bad: Vec<(&str, SiteSettingChange)> = vec![
        (
            "axis",
            put(
                "https://a.com",
                &field(MAX_SITE_FIELD_BYTES + 1),
                "",
                "allow",
            ),
        ),
        (
            "qualifier",
            put(
                "https://a.com",
                "camera",
                &field(MAX_SITE_FIELD_BYTES + 1),
                "allow",
            ),
        ),
        (
            "value",
            put(
                "https://a.com",
                "camera",
                "",
                &field(MAX_SITE_FIELD_BYTES + 1),
            ),
        ),
        (
            "remove pattern",
            SiteSettingChange::Remove {
                pattern: field(MAX_SITE_PATTERN_BYTES + 1),
                axis: "camera".into(),
                qualifier: String::new(),
            },
        ),
        (
            "remove axis",
            SiteSettingChange::Remove {
                pattern: "https://a.com".into(),
                axis: field(MAX_SITE_FIELD_BYTES + 1),
                qualifier: String::new(),
            },
        ),
        (
            "remove pattern only",
            SiteSettingChange::RemovePattern {
                pattern: field(MAX_SITE_PATTERN_BYTES + 1),
            },
        ),
        (
            "meta key",
            SiteSettingChange::SetMeta {
                key: field(MAX_SITE_FIELD_BYTES + 1),
                value: "1".into(),
            },
        ),
        (
            "meta value",
            SiteSettingChange::SetMeta {
                key: "k".into(),
                value: field(MAX_SITE_META_VALUE_BYTES + 1),
            },
        ),
        (
            "handler scheme",
            SiteSettingChange::PutHandler(handler(
                &field(MAX_SITE_FIELD_BYTES + 1),
                "https://a.com",
                false,
            )),
        ),
        (
            "handler origin",
            SiteSettingChange::PutHandler(handler(
                "mailto",
                &format!("https://{}", field(MAX_SITE_PATTERN_BYTES)),
                false,
            )),
        ),
        (
            "handler template",
            SiteSettingChange::PutHandler(ProtocolHandlerRow {
                url_template: field(MAX_HANDLER_TEMPLATE_BYTES + 1),
                ..handler("mailto", "https://a.com", false)
            }),
        ),
        (
            "handler title",
            SiteSettingChange::PutHandler(ProtocolHandlerRow {
                title: field(MAX_HANDLER_TITLE_CHARS + 1),
                ..handler("mailto", "https://a.com", false)
            }),
        ),
        (
            "remove handler scheme",
            SiteSettingChange::RemoveHandler {
                scheme: field(MAX_SITE_FIELD_BYTES + 1),
                origin: "https://a.com".into(),
            },
        ),
    ];
    for (what, change) in bad {
        let result = store.apply_site_setting_changes(&[change]);
        assert!(
            matches!(result, Err(StoreError::InvalidArgument(_))),
            "{what} was not refused"
        );
    }
    let snapshot = store.site_settings_snapshot().unwrap();
    assert!(snapshot.rows.is_empty() && snapshot.handlers.is_empty() && snapshot.meta.is_empty());
}

#[test]
fn fields_at_the_limit_are_accepted() {
    let mut store = Store::open_in_memory().unwrap();
    let pattern = "p".repeat(MAX_SITE_PATTERN_BYTES);
    let field = "f".repeat(MAX_SITE_FIELD_BYTES);
    store
        .apply_site_setting_changes(&[
            put(&pattern, &field, &field, &field),
            SiteSettingChange::SetMeta {
                key: field.clone(),
                value: "v".repeat(MAX_SITE_META_VALUE_BYTES),
            },
            SiteSettingChange::PutHandler(ProtocolHandlerRow {
                url_template: "t".repeat(MAX_HANDLER_TEMPLATE_BYTES),
                title: "é".repeat(MAX_HANDLER_TITLE_CHARS),
                ..handler(&field, "https://a.com", false)
            }),
        ])
        .unwrap();
    let snapshot = store.site_settings_snapshot().unwrap();
    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.handlers.len(), 1);
    assert_eq!(snapshot.meta.len(), 1);
}

#[test]
fn snapshot_rows_are_ordered_by_pattern_axis_qualifier() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .apply_site_setting_changes(&[
            put("site:b.com", "camera", "", "allow"),
            put("https://a.com", "microphone", "", "allow"),
            put("https://a.com", "external_links", "tel", "allow"),
            put("*", "popups", "", "allow"),
            put("https://a.com", "external_links", "mailto", "allow"),
            put("https://a.com", "camera", "", "allow"),
        ])
        .unwrap();
    let order: Vec<(String, String, String)> = keys(&store);
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
    assert_eq!(order.len(), 6);
    assert_eq!(order[0].0, "*");
    assert_eq!(order.last().unwrap().0, "site:b.com");
}

#[test]
fn a_failing_row_in_a_batch_rolls_back_earlier_changes() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .apply_site_setting_changes(&[put("https://keep.com", "camera", "", "allow")])
        .unwrap();
    let result = store.apply_site_setting_changes(&[
        SiteSettingChange::RemoveAll,
        put(
            &"p".repeat(MAX_SITE_PATTERN_BYTES + 1),
            "camera",
            "",
            "allow",
        ),
    ]);
    assert!(result.is_err());
    assert_eq!(keys(&store).len(), 1);
}

#[test]
fn rows_survive_reopening_the_file() {
    let dir = TempDir::new("site-settings-reopen");
    {
        let mut store = Store::open(&dir.db()).unwrap();
        store
            .apply_site_setting_changes(&[
                put("https://a.com", "camera", "", "allow"),
                SiteSettingChange::PutHandler(handler("mailto", "https://mail.a.com", true)),
                SiteSettingChange::SetMeta {
                    key: "k".into(),
                    value: "v".into(),
                },
            ])
            .unwrap();
    }
    let store = Store::open(&dir.db()).unwrap();
    let snapshot = store.site_settings_snapshot().unwrap();
    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.handlers.len(), 1);
    assert_eq!(snapshot.meta, [("k".to_string(), "v".to_string())]);
}

#[test]
fn secure_delete_leaves_removed_text_out_of_the_file() {
    let marker = "site:zzmarkerzz-secret.example";
    let dir = TempDir::new("site-settings-secure-delete");
    let mut store = Store::open(&dir.db()).unwrap();
    store
        .apply_site_setting_changes(&[put(marker, "camera", "", "allow")])
        .unwrap();
    store.purge_deleted().unwrap();
    let contains = |bytes: &[u8]| {
        bytes
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
    };
    assert!(
        contains(&std::fs::read(dir.db()).unwrap()),
        "the marker should reach the file before it is removed"
    );

    store
        .apply_site_setting_changes(&[SiteSettingChange::RemovePattern {
            pattern: marker.into(),
        }])
        .unwrap();
    store.purge_deleted().unwrap();
    assert!(!contains(&std::fs::read(dir.db()).unwrap()));
    let wal = dir.path().join("places.sqlite-wal");
    if wal.exists() {
        assert!(!contains(&std::fs::read(wal).unwrap()));
    }
}
