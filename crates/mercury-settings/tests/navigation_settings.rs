// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `navigation`: the ⌘-click, new-tab focus and quit-warning switches. Their defaults, and that
//! an old file, a partial object or an object with keys this version does not know all load
//! without a repair and without resetting anything else.

use std::fs;

use mercury_settings::{Loaded, NavigationSettings, Settings, SettingsStore, StartupBehavior};

fn load_text(text: &str) -> (tempfile::TempDir, Loaded<Settings>) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    fs::write(dir.path().join("settings.json"), text).expect("seed the file");
    let loaded = SettingsStore::new(dir.path())
        .load()
        .expect("loading should not fail");
    (dir, loaded)
}

fn expect_ok(loaded: Loaded<Settings>) -> Settings {
    match loaded {
        Loaded::Ok { value, repairs } => {
            assert!(
                repairs.is_empty(),
                "reading navigation is not a repair: {repairs:?}"
            );
            value
        }
        other => panic!("expected the file to load as it is, got {other:?}"),
    }
}

#[test]
fn defaults_are_cmd_click_on_new_tab_not_active_warn_on() {
    let navigation = NavigationSettings::default();
    assert!(navigation.command_click_opens_tab);
    assert!(!navigation.new_tab_becomes_active);
    assert!(navigation.warn_before_quitting);
    assert_eq!(Settings::default().navigation, navigation);
}

#[test]
fn an_old_settings_file_without_the_object_reads_the_defaults() {
    let (_dir, loaded) = load_text(
        r#"{
  "format_version": 1,
  "startup": "restore_session",
  "homepage": "https://example.com/",
  "appearance": { "theme": "dark", "default_zoom_percent": 150 },
  "web": { "experimental_web_features": false, "gpu_canvas": false }
}"#,
    );
    let settings = expect_ok(loaded);
    assert_eq!(settings.navigation, NavigationSettings::default());
    assert_eq!(settings.startup, StartupBehavior::RestoreSession);
    assert_eq!(settings.homepage, "https://example.com/");
    assert_eq!(settings.appearance.default_zoom_percent, 150);
}

#[test]
fn a_partial_object_keeps_the_rest_default() {
    let (_dir, loaded) = load_text(
        r#"{"format_version": 1, "homepage": "https://example.com/",
            "navigation": {"warn_before_quitting": false}}"#,
    );
    let settings = expect_ok(loaded);
    assert_eq!(
        settings.navigation,
        NavigationSettings {
            warn_before_quitting: false,
            ..NavigationSettings::default()
        }
    );
    assert!(settings.navigation.command_click_opens_tab);
    assert!(!settings.navigation.new_tab_becomes_active);
    assert_eq!(settings.homepage, "https://example.com/");
}

#[test]
fn unknown_keys_in_the_object_are_ignored() {
    let (_dir, loaded) = load_text(
        r#"{"format_version": 1,
            "navigation": {"command_click_opens_tab": false, "swipe_to_go_back": true,
                           "nested": {"deep": [1, 2]}}}"#,
    );
    let settings = expect_ok(loaded);
    assert_eq!(
        settings.navigation,
        NavigationSettings {
            command_click_opens_tab: false,
            ..NavigationSettings::default()
        }
    );
}

#[test]
fn round_trip_through_the_store() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = SettingsStore::new(dir.path());
    let settings = Settings {
        navigation: NavigationSettings {
            command_click_opens_tab: false,
            new_tab_becomes_active: true,
            warn_before_quitting: false,
        },
        ..Settings::default()
    };
    store.save(&settings).expect("save");
    let text = fs::read_to_string(dir.path().join("settings.json")).expect("read");
    assert!(text.contains("\"navigation\""), "{text}");
    assert!(text.contains("\"new_tab_becomes_active\": true"), "{text}");
    let loaded = expect_ok(store.load().expect("load"));
    assert_eq!(loaded, settings);
}

#[test]
fn validated_leaves_it_alone() {
    let settings = Settings {
        navigation: NavigationSettings {
            command_click_opens_tab: false,
            new_tab_becomes_active: true,
            warn_before_quitting: false,
        },
        ..Settings::default()
    };
    let (validated, repairs) = settings.clone().validated();
    assert!(repairs.is_empty(), "{repairs:?}");
    assert_eq!(validated.navigation, settings.navigation);
}
