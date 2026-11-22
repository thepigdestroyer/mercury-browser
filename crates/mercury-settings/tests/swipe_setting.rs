// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `web.swipe_navigation`: the two-finger swipe for back and forward. On by default, stored under
//! `web`, and a file that lacks it (every file written before this setting existed) loads as on.

use std::fs;

use mercury_settings::{Loaded, Settings, SettingsStore};

fn load_text(text: &str) -> (tempfile::TempDir, Loaded<Settings>) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    fs::write(dir.path().join("settings.json"), text).expect("seed the file");
    let loaded = SettingsStore::new(dir.path())
        .load()
        .expect("loading should not fail");
    (dir, loaded)
}

fn settings_of(loaded: Loaded<Settings>) -> Settings {
    match loaded {
        Loaded::Ok { value, repairs } => {
            assert!(repairs.is_empty(), "no repair expected: {repairs:?}");
            value
        }
        other => panic!("expected the file to load cleanly, got {other:?}"),
    }
}

#[test]
fn swipe_navigation_is_on_by_default() {
    assert!(Settings::default().web.swipe_navigation);
}

#[test]
fn a_file_without_the_key_loads_as_on() {
    let (_dir, loaded) = load_text(
        r#"{ "format_version": 1, "web": { "experimental_web_features": false, "gpu_canvas": false } }"#,
    );
    let settings = settings_of(loaded);
    assert!(settings.web.swipe_navigation);
    assert!(!settings.web.gpu_canvas, "the neighbours are kept");
}

#[test]
fn swipe_navigation_false_round_trips() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = SettingsStore::new(dir.path());
    let mut settings = Settings::default();
    settings.web.swipe_navigation = false;
    store.save(&settings).expect("saving works");

    let text = fs::read_to_string(dir.path().join("settings.json")).expect("the file exists");
    assert!(text.contains("\"swipe_navigation\": false"), "{text}");
    let loaded = settings_of(store.load().expect("loading works"));
    assert!(!loaded.web.swipe_navigation);
}
