// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `Manifest::parse`: the fixtures, every error case, and the warnings.

use mercury_extensions::id::ExtensionKind;
use mercury_extensions::manifest::{
    ContentScriptWorld, DefaultIcon, Incognito, Manifest, ManifestError, ManifestLimits,
    ManifestVersion, ManifestWarning, RunAt, WebAccessibleResources,
};
use mercury_extensions::permissions::ApiPermission;

const CHROME_MV3: &[u8] = include_bytes!("fixtures/manifest/chrome_mv3.json");
const CHROME_MV2: &[u8] = include_bytes!("fixtures/manifest/chrome_mv2.json");
const FIREFOX_MV2: &[u8] = include_bytes!("fixtures/manifest/firefox_mv2.json");
const FIREFOX_MV3: &[u8] = include_bytes!("fixtures/manifest/firefox_mv3.json");
const MINIMAL: &[u8] = include_bytes!("fixtures/manifest/minimal.json");

fn parse(json: &str) -> Result<Manifest, ManifestError> {
    Manifest::parse(json.as_bytes(), &ManifestLimits::default())
}

fn ok(json: &str) -> Manifest {
    parse(json).unwrap()
}

/// A valid manifest with `extra` members spliced in.
fn with(extra: &str) -> String {
    format!(r#"{{"manifest_version":3,"name":"T","version":"1",{extra}}}"#)
}

fn with_v2(extra: &str) -> String {
    format!(r#"{{"manifest_version":2,"name":"T","version":"1",{extra}}}"#)
}

#[test]
fn minimal_manifest_parses() {
    let m = Manifest::parse(MINIMAL, &ManifestLimits::default()).unwrap();
    assert_eq!(m.manifest_version, ManifestVersion::V3);
    assert_eq!(m.kind, ExtensionKind::Chrome);
    assert_eq!(m.name, "Minimal");
    assert!(m.warnings.is_empty());
    assert!(m.all_file_references().is_empty());
}

#[test]
fn chrome_mv3_fixture_reads_every_section() {
    let m = Manifest::parse(CHROME_MV3, &ManifestLimits::default()).unwrap();
    assert_eq!(m.name, "__MSG_extName__");
    assert_eq!(m.version, "1.2.3.4");
    assert_eq!(m.version_name.as_deref(), Some("1.2.3 beta"));
    assert_eq!(m.default_locale.as_deref(), Some("en"));
    assert_eq!(m.icons.get(&48).map(String::as_str), Some("icons/48.png"));
    assert_eq!(m.icons.get(&128).map(String::as_str), Some("icons/128.png"));
    let action = m.action.as_ref().unwrap();
    assert_eq!(action.default_popup.as_deref(), Some("popup.html"));
    assert!(matches!(action.default_icon, Some(DefaultIcon::Sized(ref s)) if s.len() == 2));
    let bg = m.background.as_ref().unwrap();
    assert_eq!(bg.service_worker.as_deref(), Some("background.js"));
    assert!(bg.module);
    let cs = &m.content_scripts[0];
    assert_eq!(cs.matches.len(), 2);
    assert_eq!(cs.exclude_matches.len(), 1);
    assert_eq!(cs.run_at, RunAt::DocumentStart);
    assert_eq!(cs.world, ContentScriptWorld::Main);
    assert!(cs.all_frames);
    assert!(m.permissions.contains(&ApiPermission::Storage));
    assert!(
        m.permissions
            .contains(&ApiPermission::Unknown("madeUpPermission".into()))
    );
    assert_eq!(m.optional_permissions.len(), 2);
    assert_eq!(m.host_permissions.len(), 2);
    assert_eq!(m.optional_host_permissions.len(), 1);
    let WebAccessibleResources::Entries(entries) = &m.web_accessible_resources else {
        panic!("MV3 entries expected");
    };
    assert_eq!(entries.len(), 2);
    assert!(entries[0].use_dynamic_url);
    assert_eq!(entries[1].extension_ids.len(), 1);
    assert!(
        m.content_security_policy
            .extension_pages
            .as_deref()
            .unwrap()
            .contains("wasm-unsafe-eval")
    );
    assert!(m.content_security_policy.sandbox.is_some());
    assert_eq!(m.dnr_rule_resources.len(), 2);
    assert!(!m.dnr_rule_resources[1].enabled);
    assert_eq!(m.options.as_ref().unwrap().open_in_tab, Some(true));
    assert_eq!(m.side_panel.as_deref(), Some("panel.html"));
    assert_eq!(m.commands.len(), 2);
    assert_eq!(
        m.commands["toggle"].description.as_deref(),
        Some("Toggle the thing")
    );
    assert_eq!(m.externally_connectable.as_ref().unwrap().ids, vec!["*"]);
    assert_eq!(m.devtools_page.as_deref(), Some("devtools.html"));
    assert_eq!(m.chrome_url_overrides["newtab"], "newtab.html");
    assert_eq!(m.managed_schema.as_deref(), Some("schema.json"));
    assert_eq!(m.omnibox_keyword.as_deref(), Some("go"));
    assert_eq!(m.incognito, Incognito::Spanning);
    assert_eq!(
        m.update_url.as_ref().unwrap().as_str(),
        "https://updates.example.com/crx"
    );
    assert_eq!(m.unknown_keys, vec!["some_vendor_key".to_owned()]);
    assert!(m.warnings.contains(&ManifestWarning::UnknownPermission));
}

#[test]
fn chrome_mv3_fixture_file_references_are_complete_and_normalised() {
    let m = Manifest::parse(CHROME_MV3, &ManifestLimits::default()).unwrap();
    let refs = m.all_file_references();
    for expected in [
        "background.js",
        "content.css",
        "content.js",
        "devtools.html",
        "icons/128.png",
        "icons/16.png",
        "icons/32.png",
        "icons/48.png",
        "newtab.html",
        "options.html",
        "panel.html",
        "popup.html",
        "rules/ads.json",
        "rules/trackers.json",
        "schema.json",
    ] {
        assert!(
            refs.iter().any(|r| r == expected),
            "{expected} missing from {refs:?}"
        );
    }
    assert!(refs.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
    assert!(
        !refs.iter().any(|r| r.contains('*')),
        "globs are not references"
    );
    assert!(
        !refs
            .iter()
            .any(|r| r.starts_with('/') || r.starts_with("./"))
    );
}

#[test]
fn chrome_mv2_fixture_splits_hosts_out_of_permissions() {
    let m = Manifest::parse(CHROME_MV2, &ManifestLimits::default()).unwrap();
    assert_eq!(m.manifest_version, ManifestVersion::V2);
    assert_eq!(m.kind, ExtensionKind::Chrome);
    assert!(m.browser_action.is_some());
    assert_eq!(m.background.as_ref().unwrap().persistent, Some(false));
    assert_eq!(m.host_permissions.len(), 2);
    assert_eq!(m.optional_host_permissions.len(), 1);
    assert!(m.permissions.contains(&ApiPermission::Tabs));
    assert!(!m.permissions.iter().any(|p| p.name().contains("://")));
    assert_eq!(m.incognito, Incognito::NotAllowed);
    assert!(
        matches!(m.web_accessible_resources, WebAccessibleResources::Legacy(ref l) if l.len() == 2)
    );
    assert!(
        m.content_security_policy
            .extension_pages
            .as_deref()
            .unwrap()
            .contains("unsafe-eval")
    );
    assert!(m.content_scripts[0].match_about_blank);
    assert_eq!(m.content_scripts[0].matches.len(), 1);
}

#[test]
fn firefox_mv2_fixture_is_firefox_and_accepts_toolkit_versions() {
    let m = Manifest::parse(FIREFOX_MV2, &ManifestLimits::default()).unwrap();
    assert_eq!(m.kind, ExtensionKind::Firefox);
    assert_eq!(m.version, "1.0.0pre1");
    assert_eq!(
        m.gecko_id().unwrap().as_str(),
        "{daf44bf7-a45e-4450-979c-91cf07434c3d}"
    );
    assert_eq!(m.declared_id(), m.gecko_id().cloned());
    assert!(
        m.warnings
            .contains(&ManifestWarning::DeprecatedApplicationsKey)
    );
    assert!(m.host_permissions.has_all_urls());
    // Blocking webRequest is fine in a Firefox manifest version 2.
    assert!(
        !m.warnings
            .contains(&ManifestWarning::BlockingWebRequestInMv3)
    );
}

#[test]
fn firefox_mv3_fixture_is_looser_about_csp_and_webrequest() {
    let m = Manifest::parse(FIREFOX_MV3, &ManifestLimits::default()).unwrap();
    assert_eq!(m.kind, ExtensionKind::Firefox);
    let gecko = m.gecko.as_ref().unwrap();
    assert_eq!(gecko.strict_min_version.as_deref(), Some("128.0"));
    assert_eq!(gecko.strict_max_version.as_deref(), Some("159.*"));
    assert_eq!(
        gecko.update_url.as_ref().unwrap().as_str(),
        "https://example.org/updates.json"
    );
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert!(m.background.as_ref().unwrap().scripts == vec!["event.js".to_owned()]);
}

#[test]
fn same_firefox_csp_is_refused_for_a_chrome_extension() {
    let json = r#"{"manifest_version":3,"name":"T","version":"1","content_security_policy":{"extension_pages":"script-src 'self' 'sha256-AbCdEf123+/='; object-src 'self'"}}"#;
    assert_eq!(
        parse(json),
        Err(ManifestError::InsecureCsp {
            directive: "script-src"
        })
    );
}

// ---- top-level shape ---------------------------------------------------------------------

#[test]
fn not_json_is_refused() {
    for bad in ["", "{", "nope", "[1]x", "{\"a\":}", "\u{1}"] {
        assert_eq!(parse(bad), Err(ManifestError::NotJson), "{bad:?}");
    }
    assert_eq!(
        Manifest::parse(&[0xFF, 0xFE, 0x00], &ManifestLimits::default()),
        Err(ManifestError::NotJson)
    );
}

#[test]
fn non_objects_are_refused() {
    for bad in ["[]", "1", "\"x\"", "null", "true"] {
        assert_eq!(parse(bad), Err(ManifestError::NotAnObject), "{bad}");
    }
}

#[test]
fn a_manifest_over_the_cap_is_refused() {
    let limits = ManifestLimits::default();
    let pad = " ".repeat(limits.max_bytes);
    let json = format!(r#"{{"manifest_version":3,"name":"T","version":"1"}}{pad}"#);
    assert!(matches!(
        Manifest::parse(json.as_bytes(), &limits),
        Err(ManifestError::TooLarge { .. })
    ));
}

#[test]
fn a_manifest_exactly_at_the_cap_still_parses() {
    let limits = ManifestLimits::default();
    let base = r#"{"manifest_version":3,"name":"T","version":"1"}"#;
    let json = format!("{base}{}", " ".repeat(limits.max_bytes - base.len()));
    assert_eq!(json.len(), limits.max_bytes);
    assert!(Manifest::parse(json.as_bytes(), &limits).is_ok());
}

#[test]
fn nesting_over_the_cap_is_refused_before_parsing() {
    let limits = ManifestLimits::default();
    let deep = format!(
        r#"{{"manifest_version":3,"name":"T","version":"1","x":{}{}}}"#,
        "[".repeat(limits.max_depth + 1),
        "]".repeat(limits.max_depth + 1)
    );
    assert!(matches!(
        Manifest::parse(deep.as_bytes(), &limits),
        Err(ManifestError::NestingTooDeep { .. })
    ));
    let bomb = "[".repeat(200_000);
    assert!(matches!(
        Manifest::parse(
            bomb.as_bytes(),
            &ManifestLimits {
                max_bytes: 1 << 20,
                ..limits
            }
        ),
        Err(ManifestError::NestingTooDeep { .. })
    ));
}

#[test]
fn nesting_at_the_cap_parses() {
    let limits = ManifestLimits::default();
    // The object itself is depth 1; the nested arrays bring it to exactly the cap.
    let inner = limits.max_depth - 1;
    let json = format!(
        r#"{{"manifest_version":3,"name":"T","version":"1","x":{}{}}}"#,
        "[".repeat(inner),
        "]".repeat(inner)
    );
    assert!(Manifest::parse(json.as_bytes(), &limits).is_ok());
}

#[test]
fn a_byte_order_mark_is_accepted() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(MINIMAL);
    assert!(Manifest::parse(&bytes, &ManifestLimits::default()).is_ok());
}

#[test]
fn comments_are_not_accepted() {
    assert_eq!(
        parse("// c\n{\"manifest_version\":3,\"name\":\"T\",\"version\":\"1\"}"),
        Err(ManifestError::NotJson)
    );
}

#[test]
fn escaped_and_odd_json_does_not_panic() {
    for json in [
        r#"{"manifest_version":3,"name":"\u0000","version":"1"}"#,
        r#"{"manifest_version":3,"name":"\ud800","version":"1"}"#,
        r#"{"manifest_version":1e999,"name":"T","version":"1"}"#,
        r#"{"manifest_version":3,"name":"T","version":"1","x":18446744073709551616123}"#,
    ] {
        let _ = parse(json);
    }
    assert_eq!(
        parse(r#"{"manifest_version":3,"name":"\u0000","version":"1"}"#),
        Err(ManifestError::InvalidField("name"))
    );
    assert_eq!(
        parse(r#"{"manifest_version":3,"name":"\ud800","version":"1"}"#),
        Err(ManifestError::NotJson)
    );
}

// ---- required keys -----------------------------------------------------------------------

#[test]
fn manifest_version_must_be_2_or_3() {
    assert_eq!(
        parse(r#"{"name":"T","version":"1"}"#),
        Err(ManifestError::MissingField("manifest_version"))
    );
    for bad in ["1", "4", "0", "\"3\"", "3.5", "null", "[]", "-3", "2.0"] {
        let json = format!(r#"{{"manifest_version":{bad},"name":"T","version":"1"}}"#);
        assert_eq!(
            parse(&json),
            Err(ManifestError::UnsupportedManifestVersion),
            "{bad}"
        );
    }
}

#[test]
fn name_is_required_and_not_blank() {
    assert_eq!(
        parse(r#"{"manifest_version":3,"version":"1"}"#),
        Err(ManifestError::MissingField("name"))
    );
    assert_eq!(
        parse(r#"{"manifest_version":3,"name":"  ","version":"1"}"#),
        Err(ManifestError::InvalidField("name"))
    );
    assert_eq!(
        parse(r#"{"manifest_version":3,"name":5,"version":"1"}"#),
        Err(ManifestError::InvalidField("name"))
    );
    let long = "n".repeat(5000);
    assert_eq!(
        parse(&format!(
            r#"{{"manifest_version":3,"name":"{long}","version":"1"}}"#
        )),
        Err(ManifestError::InvalidField("name"))
    );
}

#[test]
fn version_follows_chromes_rule() {
    assert_eq!(
        parse(r#"{"manifest_version":3,"name":"T"}"#),
        Err(ManifestError::MissingField("version"))
    );
    for good in ["1", "0.0.1", "1.2.3.4", "65535"] {
        assert!(
            parse(&format!(
                r#"{{"manifest_version":3,"name":"T","version":"{good}"}}"#
            ))
            .is_ok(),
            "{good}"
        );
    }
    for bad in [
        "",
        "1.2.3.4.5",
        "01",
        "1.02",
        "65536",
        "1..2",
        "v1",
        "1.0-beta",
        "-1",
        "1.",
        " 1",
    ] {
        assert_eq!(
            parse(&format!(
                r#"{{"manifest_version":3,"name":"T","version":"{bad}"}}"#
            )),
            Err(ManifestError::InvalidVersion),
            "{bad:?}"
        );
    }
    assert_eq!(
        parse(r#"{"manifest_version":3,"name":"T","version":1}"#),
        Err(ManifestError::InvalidField("version"))
    );
}

#[test]
fn toolkit_versions_need_a_gecko_id() {
    let firefox = r#"{"manifest_version":3,"name":"T","version":"1.0pre1","browser_specific_settings":{"gecko":{"id":"a@b.c"}}}"#;
    assert!(parse(firefox).is_ok());
    assert_eq!(
        parse(r#"{"manifest_version":3,"name":"T","version":"1.0pre1"}"#),
        Err(ManifestError::InvalidVersion)
    );
    let bad = r#"{"manifest_version":3,"name":"T","version":"1 0","browser_specific_settings":{"gecko":{"id":"a@b.c"}}}"#;
    assert_eq!(parse(bad), Err(ManifestError::InvalidVersion));
}

// ---- ids and keys ------------------------------------------------------------------------

#[test]
fn bad_gecko_ids_are_refused() {
    for bad in ["no-at-sign", "a@", "{not-a-guid}", "a b@c.d", ""] {
        let json = format!(
            r#"{{"manifest_version":3,"name":"T","version":"1","browser_specific_settings":{{"gecko":{{"id":"{bad}"}}}}}}"#
        );
        assert_eq!(parse(&json), Err(ManifestError::BadId), "{bad:?}");
    }
}

#[test]
fn manifest_key_must_be_a_base64_der_key() {
    // base64 of 30 03 02 01 05: a tiny DER sequence.
    let m = ok(&with(r#""key":"MAMCAQU=""#));
    assert!(m.key_id().is_some());
    assert_eq!(m.declared_id(), m.key_id());
    for bad in ["", "AAAA", "!!!!", "MAQCAQU="] {
        assert_eq!(
            parse(&with(&format!(r#""key":"{bad}""#))),
            Err(ManifestError::BadKey),
            "{bad:?}"
        );
    }
}

#[test]
fn gecko_update_url_must_be_http_or_https() {
    for bad in ["ftp://x/y", "javascript:alert(1)", "not a url", "file:///x"] {
        let json = format!(
            r#"{{"manifest_version":3,"name":"T","version":"1","browser_specific_settings":{{"gecko":{{"id":"a@b.c","update_url":"{bad}"}}}}}}"#
        );
        assert_eq!(
            parse(&json),
            Err(ManifestError::InvalidField("update_url")),
            "{bad}"
        );
    }
    assert_eq!(
        parse(&with(r#""update_url":"ftp://x/y""#)),
        Err(ManifestError::InvalidField("update_url"))
    );
}

#[test]
fn incognito_values_are_checked() {
    assert_eq!(
        ok(&with(r#""incognito":"split""#)).incognito,
        Incognito::Split
    );
    assert_eq!(
        ok(&with(r#""incognito":"not_allowed""#)).incognito,
        Incognito::NotAllowed
    );
    assert_eq!(
        ok(&with(r#""description":"d""#)).incognito,
        Incognito::Spanning
    );
    assert_eq!(
        parse(&with(r#""incognito":"sometimes""#)),
        Err(ManifestError::InvalidField("incognito"))
    );
}

// ---- content scripts ---------------------------------------------------------------------

#[test]
fn a_content_script_needs_matches() {
    assert_eq!(
        parse(&with(r#""content_scripts":[{"js":["a.js"]}]"#)),
        Err(ManifestError::ContentScriptWithoutMatches { index: 0 })
    );
    assert_eq!(
        parse(&with(
            r#""content_scripts":[{"matches":["https://a.com/*"]},{"matches":[],"js":["a.js"]}]"#
        )),
        Err(ManifestError::ContentScriptWithoutMatches { index: 1 })
    );
}

#[test]
fn content_script_enums_are_checked() {
    for (field, value) in [
        ("run_at", "later"),
        ("world", "isolated"),
        ("world", "OTHER"),
    ] {
        let json = with(&format!(
            r#""content_scripts":[{{"matches":["https://a.com/*"],"{field}":"{value}"}}]"#
        ));
        assert_eq!(
            parse(&json),
            Err(ManifestError::InvalidField("content_scripts")),
            "{field}={value}"
        );
    }
    let m = ok(&with(
        r#""content_scripts":[{"matches":["https://a.com/*"],"run_at":"document_end","world":"ISOLATED"}]"#,
    ));
    assert_eq!(m.content_scripts[0].run_at, RunAt::DocumentEnd);
    assert_eq!(m.content_scripts[0].world, ContentScriptWorld::Isolated);
}

#[test]
fn content_script_defaults() {
    let m = ok(&with(
        r#""content_scripts":[{"matches":["https://a.com/*"],"js":["a.js"]}]"#,
    ));
    let cs = &m.content_scripts[0];
    assert_eq!(cs.run_at, RunAt::DocumentIdle);
    assert!(!cs.all_frames && !cs.match_about_blank && !cs.match_origin_as_fallback);
    assert_eq!(cs.world, ContentScriptWorld::Isolated);
}

#[test]
fn bad_match_patterns_are_refused_in_every_place() {
    let bad = "https://example.com";
    for json in [
        with(&format!(r#""content_scripts":[{{"matches":["{bad}"]}}]"#)),
        with(&format!(
            r#""content_scripts":[{{"matches":["https://a.com/*"],"exclude_matches":["{bad}"]}}]"#
        )),
        with(&format!(r#""host_permissions":["{bad}"]"#)),
        with(&format!(r#""optional_host_permissions":["{bad}"]"#)),
        with(&format!(r#""permissions":["{bad}"]"#)),
        with(&format!(
            r#""externally_connectable":{{"matches":["{bad}"]}}"#
        )),
        with(&format!(
            r#""web_accessible_resources":[{{"resources":["a"],"matches":["{bad}"]}}]"#
        )),
    ] {
        assert!(
            matches!(parse(&json), Err(ManifestError::BadMatchPattern { .. })),
            "{json}"
        );
    }
}

#[test]
fn too_many_content_scripts_are_refused() {
    let limits = ManifestLimits {
        max_content_scripts: 2,
        ..ManifestLimits::default()
    };
    let one = r#"{"matches":["https://a.com/*"]}"#;
    let json = with(&format!(r#""content_scripts":[{one},{one},{one}]"#));
    assert_eq!(
        Manifest::parse(json.as_bytes(), &limits),
        Err(ManifestError::TooMany {
            field: "content_scripts"
        })
    );
    let json = with(&format!(r#""content_scripts":[{one},{one}]"#));
    assert!(Manifest::parse(json.as_bytes(), &limits).is_ok());
}

// ---- background --------------------------------------------------------------------------

#[test]
fn background_may_name_only_one_entry_point() {
    for json in [
        with(r#""background":{"service_worker":"a.js","scripts":["b.js"]}"#),
        with(r#""background":{"service_worker":"a.js","page":"b.html"}"#),
        with_v2(r#""background":{"scripts":["a.js"],"page":"b.html"}"#),
        with(r#""background":{"service_worker":"a.js","scripts":[],"page":"b.html"}"#),
    ] {
        assert_eq!(
            parse(&json),
            Err(ManifestError::BackgroundConflict),
            "{json}"
        );
    }
    assert!(parse(&with(r#""background":{"scripts":["a.js","b.js"]}"#)).is_ok());
}

#[test]
fn background_type_is_checked() {
    assert!(
        ok(&with(
            r#""background":{"service_worker":"a.js","type":"module"}"#
        ))
        .background
        .unwrap()
        .module
    );
    assert!(
        !ok(&with(
            r#""background":{"service_worker":"a.js","type":"classic"}"#
        ))
        .background
        .unwrap()
        .module
    );
    assert_eq!(
        parse(&with(
            r#""background":{"service_worker":"a.js","type":"esm"}"#
        )),
        Err(ManifestError::InvalidField("background"))
    );
}

// ---- paths -------------------------------------------------------------------------------

#[test]
fn unsafe_paths_are_refused_in_every_file_key() {
    // These are JSON text: `\\\\` is two backslashes in the value, `\\u0000` is a NUL.
    let evil = [
        "../evil.js",
        "a/../../b.js",
        "\\\\\\\\server\\\\share",
        "a\\\\b.js",
        "C:\\\\x.js",
        "//host/x.js",
        "a\\u0000b.js",
        "%2e%2e/x.js",
        "",
    ];
    for path in evil {
        let slots = [
            (
                "content_scripts",
                format!(r#""content_scripts":[{{"matches":["https://a.com/*"],"js":["{path}"]}}]"#),
            ),
            (
                "content_scripts",
                format!(
                    r#""content_scripts":[{{"matches":["https://a.com/*"],"css":["{path}"]}}]"#
                ),
            ),
            (
                "action",
                format!(r#""action":{{"default_popup":"{path}"}}"#),
            ),
            (
                "background",
                format!(r#""background":{{"service_worker":"{path}"}}"#),
            ),
            (
                "background",
                format!(r#""background":{{"scripts":["{path}"]}}"#),
            ),
            ("background", format!(r#""background":{{"page":"{path}"}}"#)),
            ("options_page", format!(r#""options_page":"{path}""#)),
            ("options_ui", format!(r#""options_ui":{{"page":"{path}"}}"#)),
            ("icons", format!(r#""icons":{{"16":"{path}"}}"#)),
            (
                "web_accessible_resources",
                format!(
                    r#""web_accessible_resources":[{{"resources":["{path}"],"matches":["https://a.com/*"]}}]"#
                ),
            ),
            (
                "declarative_net_request",
                format!(
                    r#""declarative_net_request":{{"rule_resources":[{{"id":"a","enabled":true,"path":"{path}"}}]}}"#
                ),
            ),
            (
                "side_panel",
                format!(r#""side_panel":{{"default_path":"{path}"}}"#),
            ),
            ("devtools_page", format!(r#""devtools_page":"{path}""#)),
            (
                "chrome_url_overrides",
                format!(r#""chrome_url_overrides":{{"newtab":"{path}"}}"#),
            ),
            (
                "storage",
                format!(r#""storage":{{"managed_schema":"{path}"}}"#),
            ),
        ];
        for (field, extra) in slots {
            let json = with(&extra);
            let err = parse(&json).expect_err(&json);
            assert_eq!(err, ManifestError::UnsafePath { field }, "{json}");
        }
    }
}

#[test]
fn web_accessible_resources_v2_paths_are_checked_too() {
    assert_eq!(
        parse(&with_v2(r#""web_accessible_resources":["../x"]"#)),
        Err(ManifestError::UnsafePath {
            field: "web_accessible_resources"
        })
    );
}

#[test]
fn over_long_and_over_deep_paths_are_refused() {
    let long = "a".repeat(513);
    assert_eq!(
        parse(&with(&format!(r#""options_page":"{long}""#))),
        Err(ManifestError::UnsafePath {
            field: "options_page"
        })
    );
    let ok512 = "a".repeat(512);
    assert!(parse(&with(&format!(r#""options_page":"{ok512}""#))).is_ok());
    let deep33 = vec!["d"; 33].join("/");
    assert_eq!(
        parse(&with(&format!(r#""options_page":"{deep33}""#))),
        Err(ManifestError::UnsafePath {
            field: "options_page"
        })
    );
    let deep32 = vec!["d"; 32].join("/");
    assert!(parse(&with(&format!(r#""options_page":"{deep32}""#))).is_ok());
}

#[test]
fn leading_slash_and_dot_segments_are_normalised() {
    let m = ok(&with(r#""options_page":"/pages/./opt.html""#));
    assert_eq!(m.options.unwrap().page, "pages/opt.html");
}

// ---- dnr, csp, permissions, warnings ----------------------------------------------------

#[test]
fn duplicate_ruleset_ids_are_refused() {
    let json = with(
        r#""declarative_net_request":{"rule_resources":[{"id":"a","enabled":true,"path":"a.json"},{"id":"a","enabled":false,"path":"b.json"}]}"#,
    );
    assert_eq!(parse(&json), Err(ManifestError::DuplicateRulesetId));
}

#[test]
fn ruleset_entries_are_validated() {
    for extra in [
        r#""declarative_net_request":{"rule_resources":[{"id":"","path":"a.json"}]}"#,
        r#""declarative_net_request":{"rule_resources":[{"path":"a.json"}]}"#,
        r#""declarative_net_request":{"rule_resources":[{"id":"a"}]}"#,
        r#""declarative_net_request":{"rule_resources":[1]}"#,
        r#""declarative_net_request":{"rule_resources":{}}"#,
    ] {
        assert_eq!(
            parse(&with(extra)),
            Err(ManifestError::InvalidField("declarative_net_request")),
            "{extra}"
        );
    }
    assert!(
        ok(&with(
            r#""declarative_net_request":{"rule_resources":[{"id":"a","path":"a.json"}]}"#
        ))
        .dnr_rule_resources[0]
            .enabled
    );
}

#[test]
fn mv3_csp_refuses_remote_code() {
    for (policy, directive) in [
        ("script-src 'self' 'unsafe-eval'", "script-src"),
        ("script-src 'self' 'unsafe-inline'", "script-src"),
        ("script-src 'self' https://cdn.example.com", "script-src"),
        ("script-src 'self' *", "script-src"),
        ("script-src 'self' data:", "script-src"),
        ("script-src 'self' blob:", "script-src"),
        ("script-src 'self' 'strict-dynamic'", "script-src"),
        ("script-src 'self' 'nonce-abc'", "script-src"),
        ("script-src-elem https://x.example", "script-src-elem"),
        ("worker-src https://x.example", "worker-src"),
        ("default-src https://x.example", "default-src"),
        ("object-src https://x.example", "object-src"),
        ("object-src 'self' 'unsafe-inline'", "object-src"),
        ("script-src 'self'; object-src *", "object-src"),
        ("SCRIPT-SRC 'SELF' 'UNSAFE-EVAL'", "script-src"),
        ("script-src 'self' http://localhost:8000", "script-src"),
    ] {
        let json = with(&format!(
            r#""content_security_policy":{{"extension_pages":"{policy}"}}"#
        ));
        assert_eq!(
            parse(&json),
            Err(ManifestError::InsecureCsp { directive }),
            "{policy}"
        );
    }
}

#[test]
fn mv3_csp_accepts_the_documented_safe_forms() {
    for policy in [
        "script-src 'self'; object-src 'self'",
        "script-src 'self' 'wasm-unsafe-eval'",
        "script-src 'self' 'inline-speculation-rules'",
        "script-src 'none'; object-src 'none'",
        "object-src 'self' 'wasm-unsafe-eval'",
        "style-src 'self' 'unsafe-inline'; img-src * data:",
        "",
    ] {
        let json = with(&format!(
            r#""content_security_policy":{{"extension_pages":"{policy}"}}"#
        ));
        assert!(parse(&json).is_ok(), "{policy}");
    }
}

#[test]
fn the_first_occurrence_of_a_csp_directive_decides() {
    let json = with(
        r#""content_security_policy":{"extension_pages":"script-src 'self'; script-src https://evil.example"}"#,
    );
    assert!(parse(&json).is_ok());
    let json = with(
        r#""content_security_policy":{"extension_pages":"script-src https://evil.example; script-src 'self'"}"#,
    );
    assert!(parse(&json).is_err());
}

#[test]
fn localhost_csp_sources_need_the_unpacked_flag() {
    let json = with(
        r#""content_security_policy":{"extension_pages":"script-src 'self' http://localhost:8000"}"#,
    );
    assert!(parse(&json).is_err());
    let limits = ManifestLimits {
        allow_localhost_csp: true,
        ..ManifestLimits::default()
    };
    assert!(Manifest::parse(json.as_bytes(), &limits).is_ok());
    let sneaky = with(
        r#""content_security_policy":{"extension_pages":"script-src 'self' http://localhost.evil.com"}"#,
    );
    assert!(Manifest::parse(sneaky.as_bytes(), &limits).is_err());
}

#[test]
fn the_isolated_world_policy_is_checked_too() {
    let json =
        with(r#""content_security_policy":{"isolated_world":"script-src 'self' 'unsafe-eval'"}"#);
    assert_eq!(
        parse(&json),
        Err(ManifestError::InsecureCsp {
            directive: "script-src"
        })
    );
}

#[test]
fn csp_has_the_right_shape_per_version() {
    assert_eq!(
        parse(&with(r#""content_security_policy":"script-src 'self'""#)),
        Err(ManifestError::InvalidField("content_security_policy"))
    );
    assert_eq!(
        parse(&with_v2(
            r#""content_security_policy":{"extension_pages":"script-src 'self'"}"#
        )),
        Err(ManifestError::InvalidField("content_security_policy"))
    );
    // MV2 policies are stored, not judged.
    let m = ok(&with_v2(
        r#""content_security_policy":"script-src 'self' https://cdn.example.com""#,
    ));
    assert!(m.content_security_policy.extension_pages.is_some());
}

#[test]
fn mv2_keys_in_mv3_are_warnings() {
    let m = ok(&with(
        r#""browser_action":{"default_title":"x"},"page_action":{},"background":{"scripts":["a.js"],"persistent":true},"permissions":["webRequest","webRequestBlocking"]"#,
    ));
    assert!(m.warnings.contains(&ManifestWarning::Mv2KeyInMv3 {
        key: "browser_action"
    }));
    assert!(
        m.warnings
            .contains(&ManifestWarning::Mv2KeyInMv3 { key: "page_action" })
    );
    assert!(
        m.warnings
            .contains(&ManifestWarning::PersistentBackgroundInMv3)
    );
    assert!(
        m.warnings
            .contains(&ManifestWarning::BlockingWebRequestInMv3)
    );
    assert!(
        m.warnings
            .contains(&ManifestWarning::PermissionNotAvailableInMv3)
    );
}

#[test]
fn mv3_keys_in_mv2_are_warnings() {
    let m = ok(&with_v2(
        r#""action":{},"host_permissions":["https://a.com/*"],"background":{"service_worker":"sw.js"}"#,
    ));
    assert!(
        m.warnings
            .contains(&ManifestWarning::Mv3KeyInMv2 { key: "action" })
    );
    assert!(m.warnings.contains(&ManifestWarning::Mv3KeyInMv2 {
        key: "host_permissions"
    }));
    assert!(m.warnings.contains(&ManifestWarning::Mv3KeyInMv2 {
        key: "background.service_worker"
    }));
    // The MV3 host key is ignored in MV2.
    assert!(m.host_permissions.is_empty());
}

#[test]
fn hosts_in_permissions_of_mv3_are_ignored_with_a_warning() {
    let m = ok(&with(r#""permissions":["storage","https://a.com/*"]"#));
    assert!(
        m.warnings
            .contains(&ManifestWarning::HostPermissionInPermissionsMv3)
    );
    assert!(m.host_permissions.is_empty());
    assert_eq!(m.permissions, vec![ApiPermission::Storage]);
}

#[test]
fn permissions_in_the_wrong_list_warn() {
    let m = ok(&with(
        r#""optional_permissions":["activeTab","storage","bookmarks"]"#,
    ));
    assert_eq!(
        m.warnings
            .iter()
            .filter(|w| **w == ManifestWarning::PermissionMisplaced)
            .count(),
        2
    );
}

#[test]
fn duplicate_permissions_collapse() {
    let m = ok(&with(r#""permissions":["storage","storage","tabs"]"#));
    assert_eq!(m.permissions.len(), 2);
}

#[test]
fn permission_entries_must_be_strings_and_bounded() {
    assert_eq!(
        parse(&with(r#""permissions":[1]"#)),
        Err(ManifestError::InvalidField("permissions"))
    );
    assert_eq!(
        parse(&with(r#""permissions":"tabs""#)),
        Err(ManifestError::InvalidField("permissions"))
    );
    let long = "p".repeat(300);
    assert_eq!(
        parse(&with(&format!(r#""permissions":["{long}"]"#))),
        Err(ManifestError::InvalidField("permissions"))
    );
    let limits = ManifestLimits {
        max_list_len: 3,
        ..ManifestLimits::default()
    };
    let json = with(r#""permissions":["a","b","c","d"]"#);
    assert_eq!(
        Manifest::parse(json.as_bytes(), &limits),
        Err(ManifestError::TooMany {
            field: "permissions"
        })
    );
}

#[test]
fn unknown_keys_are_names_only_and_capped() {
    let m = ok(&with(r#""zeta":1,"alpha":{"secret":"hunter2"}"#));
    assert_eq!(m.unknown_keys, vec!["alpha".to_owned(), "zeta".to_owned()]);
    let limits = ManifestLimits {
        max_unknown_keys: 2,
        ..ManifestLimits::default()
    };
    let json = with(r#""a":1,"b":2,"c":3,"d":4"#);
    assert_eq!(
        Manifest::parse(json.as_bytes(), &limits)
            .unwrap()
            .unknown_keys
            .len(),
        2
    );
}

#[test]
fn error_messages_never_contain_manifest_content() {
    let secret = "hunter2-secret-token";
    let cases = [
        format!(r#"{{"manifest_version":3,"name":"T","version":"{secret}"}}"#),
        format!(r#"{{"manifest_version":3,"name":"T","version":"1","key":"{secret}"}}"#),
        format!(
            r#"{{"manifest_version":3,"name":"T","version":"1","options_page":"../{secret}"}}"#
        ),
        format!(
            r#"{{"manifest_version":3,"name":"T","version":"1","host_permissions":["{secret}"]}}"#
        ),
        format!(
            r#"{{"manifest_version":3,"name":"T","version":"1","content_security_policy":{{"extension_pages":"script-src {secret}"}}}}"#
        ),
        format!(
            r#"{{"manifest_version":3,"name":"T","version":"1","browser_specific_settings":{{"gecko":{{"id":"{secret}"}}}}}}"#
        ),
        format!(r#"{{"manifest_version":3,"name":"T","version":"1","incognito":"{secret}"}}"#),
        format!(r#"{{"manifest_version":3,"name":"{secret}", broken"#),
    ];
    for json in cases {
        let err = parse(&json).expect_err(&json);
        assert!(!format!("{err} {err:?}").contains(secret), "{err:?}");
    }
}

#[test]
fn commands_are_parsed_and_bounded() {
    let m = ok(&with(
        r#""commands":{"a":{"suggested_key":"Ctrl+Q"},"b":{}}"#,
    ));
    assert_eq!(m.commands["a"].suggested_key["default"], "Ctrl+Q");
    assert!(m.commands["b"].suggested_key.is_empty());
    assert_eq!(
        parse(&with(r#""commands":{"a":1}"#)),
        Err(ManifestError::InvalidField("commands"))
    );
    let limits = ManifestLimits {
        max_commands: 1,
        ..ManifestLimits::default()
    };
    let json = with(r#""commands":{"a":{},"b":{}}"#);
    assert_eq!(
        Manifest::parse(json.as_bytes(), &limits),
        Err(ManifestError::TooMany { field: "commands" })
    );
}

#[test]
fn externally_connectable_ids_are_validated() {
    assert!(
        parse(&with(
            r#""externally_connectable":{"ids":["*","jpignaibiiemhngfjkcpokkamffknabf"]}"#
        ))
        .is_ok()
    );
    assert_eq!(
        parse(&with(r#""externally_connectable":{"ids":["nope"]}"#)),
        Err(ManifestError::BadId)
    );
}

#[test]
fn icons_keys_must_be_sizes() {
    assert_eq!(
        parse(&with(r#""icons":{"big":"a.png"}"#)),
        Err(ManifestError::InvalidField("icons"))
    );
    assert_eq!(
        parse(&with(r#""icons":[]"#)),
        Err(ManifestError::InvalidField("icons"))
    );
}

#[test]
fn default_locale_is_checked() {
    assert!(parse(&with(r#""default_locale":"pt_BR""#)).is_ok());
    assert_eq!(
        parse(&with(r#""default_locale":"../x""#)),
        Err(ManifestError::InvalidField("default_locale"))
    );
}

#[test]
fn msg_placeholders_are_kept_as_written() {
    let m = ok(&with(
        r#""description":"__MSG_desc__","action":{"default_title":"__MSG_title__"}"#,
    ));
    assert_eq!(m.description.as_deref(), Some("__MSG_desc__"));
    assert_eq!(
        m.action.unwrap().default_title.as_deref(),
        Some("__MSG_title__")
    );
}

#[test]
fn parse_is_deterministic() {
    assert_eq!(
        Manifest::parse(CHROME_MV3, &ManifestLimits::default()),
        Manifest::parse(CHROME_MV3, &ManifestLimits::default())
    );
}
