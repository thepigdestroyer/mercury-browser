// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `web.pdf`: what a link to a PDF file does. Its default, its stored names, and its own
//! tolerance: a value this version does not know reads as the default and costs nothing else.

use std::fs;

use mercury_settings::{Loaded, PdfAction, Settings, SettingsStore, StartupBehavior, Theme};

fn load_text(text: &str) -> (tempfile::TempDir, Loaded<Settings>) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    fs::write(dir.path().join("settings.json"), text).expect("seed the file");
    let loaded = SettingsStore::new(dir.path())
        .load()
        .expect("loading should not fail");
    (dir, loaded)
}

/// A file with settings besides `web.pdf` that a reset would lose, and `pdf_json` as the value of
/// `web.pdf` (`None` leaves the field out).
fn file_with_pdf(pdf_json: Option<&str>) -> String {
    let pdf = pdf_json.map_or(String::new(), |value| format!(", \"pdf\": {value}"));
    format!(
        r#"{{
  "format_version": 1,
  "startup": "restore_session",
  "homepage": "https://example.com/",
  "appearance": {{ "theme": "dark", "default_zoom_percent": 150 }},
  "web": {{ "experimental_web_features": false, "gpu_canvas": false{pdf} }}
}}"#
    )
}

/// The settings the file above must produce whatever `web.pdf` holds.
fn assert_everything_else_kept(settings: &Settings) {
    assert_eq!(settings.startup, StartupBehavior::RestoreSession);
    assert_eq!(settings.homepage, "https://example.com/");
    assert_eq!(settings.appearance.theme, Theme::Dark);
    assert_eq!(settings.appearance.default_zoom_percent, 150);
    assert!(!settings.web.experimental_web_features);
    assert!(!settings.web.gpu_canvas);
}

fn expect_ok(loaded: Loaded<Settings>) -> Settings {
    match loaded {
        Loaded::Ok { value, repairs } => {
            assert!(
                repairs.is_empty(),
                "reading web.pdf is not a repair: {repairs:?}"
            );
            value
        }
        other => panic!("expected the file to load as it is, got {other:?}"),
    }
}

#[test]
fn the_default_is_to_open_in_mercury() {
    assert_eq!(PdfAction::default(), PdfAction::OpenInMercury);
    assert_eq!(Settings::default().web.pdf, PdfAction::OpenInMercury);
}

#[test]
fn each_action_is_stored_under_its_snake_case_name() {
    for (action, name) in [
        (PdfAction::OpenInMercury, "open_in_mercury"),
        (PdfAction::OpenInSystemViewer, "open_in_system_viewer"),
        (PdfAction::Download, "download"),
    ] {
        assert_eq!(
            serde_json::to_string(&action).expect("serialise"),
            format!("\"{name}\"")
        );
        let back: PdfAction = serde_json::from_str(&format!("\"{name}\"")).expect("deserialise");
        assert_eq!(back, action);
    }
}

#[test]
fn every_action_survives_saving_and_loading() {
    for action in [
        PdfAction::OpenInMercury,
        PdfAction::OpenInSystemViewer,
        PdfAction::Download,
    ] {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = SettingsStore::new(dir.path());
        let mut settings = Settings::default();
        settings.web.pdf = action;
        store.save(&settings).expect("save");
        let text = fs::read_to_string(dir.path().join("settings.json")).expect("read");
        assert!(text.contains(&format!(
            "\"pdf\": {}",
            serde_json::to_string(&action).expect("serialise")
        )));
        let loaded = expect_ok(store.load().expect("load"));
        assert_eq!(loaded.web.pdf, action);
        assert_eq!(loaded, settings);
    }
}

#[test]
fn a_file_without_the_field_gets_the_default_and_keeps_the_rest() {
    let (_dir, loaded) = load_text(&file_with_pdf(None));
    let settings = expect_ok(loaded);
    assert_eq!(settings.web.pdf, PdfAction::OpenInMercury);
    assert_everything_else_kept(&settings);
}

#[test]
fn an_old_file_with_no_web_section_at_all_loads() {
    let (_dir, loaded) = load_text(r#"{ "format_version": 1, "homepage": "" }"#);
    assert_eq!(expect_ok(loaded).web.pdf, PdfAction::OpenInMercury);
}

#[test]
fn a_known_value_is_read_as_written() {
    for (name, expected) in [
        ("open_in_mercury", PdfAction::OpenInMercury),
        ("open_in_system_viewer", PdfAction::OpenInSystemViewer),
        ("download", PdfAction::Download),
    ] {
        let (_dir, loaded) = load_text(&file_with_pdf(Some(&format!("\"{name}\""))));
        let settings = expect_ok(loaded);
        assert_eq!(settings.web.pdf, expected);
        assert_everything_else_kept(&settings);
    }
}

#[test]
fn an_unknown_name_reads_as_the_default_without_resetting_anything_else() {
    let (dir, loaded) = load_text(&file_with_pdf(Some("\"open_in_a_newer_viewer\"")));
    let settings = expect_ok(loaded);
    assert_eq!(settings.web.pdf, PdfAction::OpenInMercury);
    assert_everything_else_kept(&settings);
    // Not "damaged": nothing was moved aside.
    let files: Vec<String> = fs::read_dir(dir.path())
        .expect("list the profile")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(files, vec!["settings.json".to_owned()], "{files:?}");
}

#[test]
fn a_value_of_the_wrong_kind_reads_as_the_default_too() {
    for junk in [
        "null",
        "true",
        "7",
        "-3",
        "1.5",
        "\"\"",
        "\"DOWNLOAD\"",
        "[\"download\", [1, 2]]",
        "{ \"action\": \"download\", \"nested\": { \"deep\": [1] } }",
    ] {
        let (_dir, loaded) = load_text(&file_with_pdf(Some(junk)));
        let settings = expect_ok(loaded);
        assert_eq!(settings.web.pdf, PdfAction::OpenInMercury, "{junk}");
        assert_everything_else_kept(&settings);
    }
}

#[test]
fn the_unknown_value_is_replaced_when_the_settings_are_saved_again() {
    let (dir, loaded) = load_text(&file_with_pdf(Some("\"someday\"")));
    let settings = expect_ok(loaded);
    let store = SettingsStore::new(dir.path());
    store.save(&settings).expect("save");
    let text = fs::read_to_string(dir.path().join("settings.json")).expect("read");
    assert!(text.contains("\"pdf\": \"open_in_mercury\""), "{text}");
    assert!(!text.contains("someday"));
}
