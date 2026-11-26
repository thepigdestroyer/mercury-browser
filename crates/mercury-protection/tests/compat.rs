// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The web-compatibility exception list (R13 MH31).

use mercury_protection::{Category, CompatError, CompatList, ProtectionSettings, Site};

fn site(key: &str) -> Site {
    Site::from_key(key).unwrap()
}

fn two_rules() -> &'static str {
    r#"{"version":1,"rules":[
        {"top_site":"shop.example","request_host":"cdn.payments.example","category":"cross_site_cookies","tier":"baseline","note":"checkout breaks without it"},
        {"top_site":"*","request_host":"fonts.example","category":"fingerprinting","tier":"convenience"}
    ]}"#
}

fn defaults() -> ProtectionSettings {
    ProtectionSettings::default()
}

fn with_convenience() -> ProtectionSettings {
    ProtectionSettings {
        compat_convenience: true,
        ..ProtectionSettings::default()
    }
}

fn parsed(rules: &str) -> CompatList {
    let text = format!(r#"{{"version":1,"rules":[{rules}]}}"#);
    CompatList::from_json(&text).unwrap()
}

#[test]
fn a_two_rule_list_parses() {
    assert!(CompatList::from_json(two_rules()).is_ok());
}

#[test]
fn a_matching_top_site_and_request_host_are_allowed_only_with_the_tier_enabled() {
    let list = CompatList::from_json(two_rules()).unwrap();
    let top = site("shop.example");
    assert!(list.allows(
        &defaults(),
        &top,
        "cdn.payments.example",
        Category::CrossSiteCookies
    ));
    let convenience = Category::Fingerprinting;
    assert!(!list.allows(&defaults(), &top, "fonts.example", convenience));
    assert!(list.allows(&with_convenience(), &top, "fonts.example", convenience));
}

#[test]
fn baseline_off_turns_off_its_rules() {
    let list = CompatList::from_json(two_rules()).unwrap();
    let settings = ProtectionSettings {
        compat_baseline: false,
        ..ProtectionSettings::default()
    };
    assert!(!list.allows(
        &settings,
        &site("shop.example"),
        "cdn.payments.example",
        Category::CrossSiteCookies
    ));
}

#[test]
fn a_star_top_site_matches_any_site() {
    let list = CompatList::from_json(two_rules()).unwrap();
    for top in ["news.example", "shop.example", "elsewhere.test"] {
        assert!(
            list.allows(
                &with_convenience(),
                &site(top),
                "fonts.example",
                Category::Fingerprinting
            ),
            "{top}"
        );
    }
}

#[test]
fn a_request_host_matches_itself_and_its_parent_domains_only() {
    let list = parsed(
        r#"{"top_site":"*","request_host":"a.example","category":"ads_and_trackers","tier":"baseline"}"#,
    );
    let top = site("news.test");
    let settings = defaults();
    let allowed = |host: &str| list.allows(&settings, &top, host, Category::AdsAndTrackers);
    assert!(allowed("a.example"));
    assert!(allowed("cdn.a.example"));
    assert!(allowed("x.y.a.example"));
    assert!(!allowed("xa.example"));
    assert!(!allowed("a.example.org"));
    assert!(!allowed("example"));
}

#[test]
fn the_category_must_match() {
    let list = CompatList::from_json(two_rules()).unwrap();
    assert!(!list.allows(
        &defaults(),
        &site("shop.example"),
        "cdn.payments.example",
        Category::AdsAndTrackers
    ));
}

#[test]
fn matching_ignores_case() {
    let list = parsed(
        r#"{"top_site":"Shop.Example","request_host":"CDN.Payments.Example","category":"cross_site_cookies","tier":"baseline"}"#,
    );
    assert!(list.allows(
        &defaults(),
        &site("SHOP.example"),
        "Cdn.PAYMENTS.example",
        Category::CrossSiteCookies
    ));
}

#[test]
fn an_empty_list_allows_nothing() {
    let settings = with_convenience();
    for category in Category::ALL {
        assert!(!CompatList::EMPTY.allows(
            &settings,
            &site("shop.example"),
            "cdn.payments.example",
            category
        ));
    }
}

#[test]
fn a_newer_version_is_unknown() {
    assert_eq!(
        CompatList::from_json(r#"{"version":2}"#).unwrap_err(),
        CompatError::UnknownVersion(2)
    );
}

#[test]
fn an_input_over_one_mebibyte_is_too_large() {
    let text = format!(
        "{}{{\"version\":1,\"rules\":[]}}",
        " ".repeat((1 << 20) + 1)
    );
    assert_eq!(
        CompatList::from_json(&text).unwrap_err(),
        CompatError::TooLarge
    );
}

#[test]
fn more_than_twenty_thousand_rules_is_too_many() {
    let rules = vec!["{}"; 20_001].join(",");
    let text = format!(r#"{{"version":1,"rules":[{rules}]}}"#);
    assert_eq!(
        CompatList::from_json(&text).unwrap_err(),
        CompatError::TooManyRules
    );
}

#[test]
fn a_field_of_254_bytes_is_too_long() {
    let host = "a".repeat(254);
    let rule = format!(
        r#"{{"top_site":"*","request_host":"{host}","category":"ads_and_trackers","tier":"baseline"}}"#
    );
    let text = format!(r#"{{"version":1,"rules":[{rule}]}}"#);
    assert_eq!(
        CompatList::from_json(&text).unwrap_err(),
        CompatError::FieldTooLong
    );
}

#[test]
fn an_unknown_category_is_malformed() {
    let text = r#"{"version":1,"rules":[{"top_site":"*","request_host":"a.example","category":"no_such_category","tier":"baseline"}]}"#;
    assert!(matches!(
        CompatList::from_json(text).unwrap_err(),
        CompatError::Malformed(_)
    ));
}

#[test]
fn invalid_json_is_malformed() {
    assert!(matches!(
        CompatList::from_json("{\"version\":1,").unwrap_err(),
        CompatError::Malformed(_)
    ));
}
