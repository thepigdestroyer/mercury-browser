// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use mercury_site_settings::is_registrable_scheme;
use mercury_site_settings::{Error, MAX_HANDLERS, Origin, SiteSettings, UnixMillis};
use mercury_store::{SiteSettingChange, Store};

const NOW: UnixMillis = 1_800_000_000_000;

fn origin(text: &str) -> Origin {
    Origin::parse(text).expect("origin")
}

#[test]
fn is_registrable_scheme_follows_the_safelist_and_web_plus_rule() {
    for scheme in [
        "mailto",
        "tel",
        "webcal",
        "magnet",
        "web+music",
        "matrix",
        "sms",
        "xmpp",
    ] {
        assert!(is_registrable_scheme(scheme), "{scheme} should be allowed");
    }
    for scheme in [
        "http",
        "https",
        "javascript",
        "data",
        "file",
        "blob",
        "mercury",
        "web+",
        "web+Music1",
        "web+a b",
        "",
        "chrome",
        "about",
    ] {
        assert!(!is_registrable_scheme(scheme), "{scheme} should be refused");
    }
}

#[test]
fn register_accepts_a_same_origin_https_template_with_percent_s() {
    let mut s = SiteSettings::new();
    s.register_handler(
        "mailto",
        &origin("https://mail.a.com"),
        "https://mail.a.com/compose?to=%s",
        "Mail",
        NOW,
    )
    .expect("register");
    assert_eq!(s.handlers_for("mailto").len(), 1);
}

#[test]
fn register_refuses_a_bad_template_or_scheme() {
    let mut s = SiteSettings::new();
    let mail = origin("https://mail.a.com");
    let refused = [
        (
            "mailto",
            "https://other.com/compose?to=%s",
            Error::TemplateNotAllowed,
        ),
        (
            "mailto",
            "https://mail.a.com/compose",
            Error::TemplateNotAllowed,
        ),
        (
            "mailto",
            "http://mail.a.com/compose?to=%s",
            Error::TemplateNotAllowed,
        ),
        ("mailto", "javascript:alert(%s)", Error::TemplateNotAllowed),
        (
            "mailto",
            "https://user:pw@mail.a.com/?to=%s",
            Error::TemplateNotAllowed,
        ),
        (
            "javascript",
            "https://mail.a.com/?to=%s",
            Error::SchemeNotAllowed,
        ),
    ];
    for (scheme, template, want) in refused {
        assert_eq!(
            s.register_handler(scheme, &mail, template, "Mail", NOW),
            Err(want),
            "{scheme} {template}"
        );
    }
    let too_long = format!("https://mail.a.com/?to=%s{}", "x".repeat(2048));
    assert_eq!(
        s.register_handler("mailto", &mail, &too_long, "Mail", NOW),
        Err(Error::TemplateNotAllowed)
    );
    assert!(s.handlers_for("mailto").is_empty());
}

#[test]
fn a_loopback_http_template_is_accepted_for_any_origin() {
    let mut s = SiteSettings::new();
    s.register_handler(
        "mailto",
        &origin("https://mail.a.com"),
        "http://localhost:3000/x?u=%s",
        "Local",
        NOW,
    )
    .expect("register");
    assert_eq!(s.handlers_for("mailto").len(), 1);
}

#[test]
fn the_first_handler_for_a_scheme_becomes_the_default_and_a_second_does_not() {
    let mut s = SiteSettings::new();
    let a = origin("https://mail.a.com");
    let b = origin("https://mail.b.com");
    s.register_handler("mailto", &a, "https://mail.a.com/?to=%s", "A", NOW)
        .expect("a");
    s.register_handler("mailto", &b, "https://mail.b.com/?to=%s", "B", NOW + 1)
        .expect("b");

    let handlers = s.handlers_for("mailto");
    assert_eq!(handlers[0].origin, a);
    assert!(handlers[0].is_default);
    assert!(!handlers[1].is_default);
    assert_eq!(s.default_handler("mailto").expect("default").origin, a);
}

#[test]
fn set_default_handler_moves_the_flag_so_exactly_one_is_default() {
    let mut s = SiteSettings::new();
    let a = origin("https://mail.a.com");
    let b = origin("https://mail.b.com");
    s.register_handler("mailto", &a, "https://mail.a.com/?to=%s", "A", NOW)
        .expect("a");
    s.register_handler("mailto", &b, "https://mail.b.com/?to=%s", "B", NOW + 1)
        .expect("b");

    assert!(s.set_default_handler("mailto", &b));
    assert!(!s.set_default_handler("mailto", &origin("https://c.com")));

    let defaults: Vec<_> = s
        .handlers_for("mailto")
        .into_iter()
        .filter(|h| h.is_default)
        .collect();
    assert_eq!(defaults.len(), 1);
    assert_eq!(defaults[0].origin, b);
}

#[test]
fn removing_the_default_leaves_no_default_until_the_user_picks() {
    let mut s = SiteSettings::new();
    let a = origin("https://mail.a.com");
    let b = origin("https://mail.b.com");
    s.register_handler("mailto", &a, "https://mail.a.com/?to=%s", "A", NOW)
        .expect("a");
    s.register_handler("mailto", &b, "https://mail.b.com/?to=%s", "B", NOW + 1)
        .expect("b");

    assert!(s.remove_handler("mailto", &a));
    assert!(s.default_handler("mailto").is_none());
    assert_eq!(s.handlers_for("mailto").len(), 1);
}

#[test]
fn re_registering_updates_template_and_title_and_keeps_added_at_and_default() {
    let mut s = SiteSettings::new();
    let a = origin("https://mail.a.com");
    s.register_handler("mailto", &a, "https://mail.a.com/?to=%s", "A", NOW)
        .expect("a");

    s.register_handler(
        "mailto",
        &a,
        "https://mail.a.com/new?to=%s",
        "A renamed",
        NOW + 9,
    )
    .expect("again");

    let handlers = s.handlers_for("mailto");
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0].url_template, "https://mail.a.com/new?to=%s");
    assert_eq!(handlers[0].title, "A renamed");
    assert_eq!(handlers[0].added_at, NOW);
    assert!(handlers[0].is_default);
}

#[test]
fn handlers_for_lists_the_default_first_then_by_added_at() {
    let mut s = SiteSettings::new();
    let a = origin("https://mail.a.com");
    let b = origin("https://mail.b.com");
    let c = origin("https://mail.c.com");
    s.register_handler("mailto", &a, "https://mail.a.com/?to=%s", "A", NOW + 2)
        .expect("a");
    s.register_handler("mailto", &b, "https://mail.b.com/?to=%s", "B", NOW + 1)
        .expect("b");
    s.register_handler("mailto", &c, "https://mail.c.com/?to=%s", "C", NOW)
        .expect("c");
    assert!(s.set_default_handler("mailto", &a));

    let order: Vec<_> = s
        .handlers_for("mailto")
        .into_iter()
        .map(|h| h.origin)
        .collect();
    assert_eq!(order, [a, c, b]);
}

#[test]
fn handler_changes_are_journaled() {
    let mut s = SiteSettings::new();
    let a = origin("https://mail.a.com");
    let b = origin("https://mail.b.com");
    s.register_handler("mailto", &a, "https://mail.a.com/?to=%s", "A", NOW)
        .expect("a");
    s.register_handler("mailto", &b, "https://mail.b.com/?to=%s", "B", NOW)
        .expect("b");
    s.take_changes();

    assert!(s.set_default_handler("mailto", &b));
    let changes = s.take_changes();
    assert_eq!(
        changes.len(),
        2,
        "one PutHandler per changed row: {changes:?}"
    );
    assert!(
        changes
            .iter()
            .all(|c| matches!(c, SiteSettingChange::PutHandler(_)))
    );

    assert!(s.remove_handler("mailto", &a));
    assert_eq!(
        s.take_changes(),
        vec![SiteSettingChange::RemoveHandler {
            scheme: "mailto".to_owned(),
            origin: a.to_string(),
        }]
    );
}

#[test]
fn a_drain_and_reload_through_the_store_gives_equal_handlers() {
    let mut s = SiteSettings::new();
    let a = origin("https://mail.a.com");
    let b = origin("https://mail.b.com");
    s.register_handler("mailto", &a, "https://mail.a.com/?to=%s", "A", NOW)
        .expect("a");
    s.register_handler("mailto", &b, "https://mail.b.com/?to=%s", "B", NOW + 1)
        .expect("b");
    s.set_default_handler("mailto", &b);
    s.record_handler_use("mailto", &b, NOW + 5);

    let mut store = Store::open_in_memory().expect("store");
    store
        .apply_site_setting_changes(&s.take_changes())
        .expect("apply");
    let (reloaded, _) = SiteSettings::load(store.site_settings_snapshot().expect("snap"), NOW);

    assert_eq!(reloaded.handlers_for("mailto"), s.handlers_for("mailto"));
}

#[test]
fn the_handler_cap_refuses_a_new_one_but_not_a_re_registration() {
    let mut s = SiteSettings::new();
    for i in 0..MAX_HANDLERS {
        let host = format!("app{i}.a.com");
        let o = origin(&format!("https://{host}"));
        s.register_handler("mailto", &o, &format!("https://{host}/?to=%s"), "App", NOW)
            .expect("under the cap");
    }
    let extra = origin("https://extra.a.com");
    assert_eq!(
        s.register_handler("mailto", &extra, "https://extra.a.com/?to=%s", "X", NOW),
        Err(Error::TooManyHandlers)
    );
    let first = origin("https://app0.a.com");
    s.register_handler("mailto", &first, "https://app0.a.com/?to=%s", "Again", NOW)
        .expect("re-registration at the cap");
}

#[test]
fn stored_rows_that_no_longer_validate_are_ignored_and_kept() {
    use mercury_store::ProtocolHandlerRow;
    let mut store = Store::open_in_memory().expect("store");
    store
        .apply_site_setting_changes(&[
            SiteSettingChange::PutHandler(ProtocolHandlerRow {
                scheme: "javascript".to_owned(),
                origin: "https://mail.a.com".to_owned(),
                url_template: "https://mail.a.com/?to=%s".to_owned(),
                title: "Old".to_owned(),
                added_at: NOW,
                last_used_at: None,
                is_default: true,
            }),
            SiteSettingChange::PutHandler(ProtocolHandlerRow {
                scheme: "mailto".to_owned(),
                origin: "https://mail.a.com".to_owned(),
                url_template: "https://evil.com/?to=%s".to_owned(),
                title: "Bad".to_owned(),
                added_at: NOW,
                last_used_at: None,
                is_default: true,
            }),
        ])
        .expect("apply");

    let (s, report) = SiteSettings::load(store.site_settings_snapshot().expect("snap"), NOW);
    assert!(s.handlers_for("mailto").is_empty());
    assert_eq!(report.ignored, 2);
    assert_eq!(
        store.site_settings_snapshot().expect("snap").handlers.len(),
        2
    );
}
