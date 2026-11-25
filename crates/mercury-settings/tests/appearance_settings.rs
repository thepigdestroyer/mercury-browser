// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The stored layout, palette and tab-group settings: their defaults, what loading repairs, and
//! that every value survives the store.
use std::fs;
use std::path::{Path, PathBuf};

use mercury_settings::{
    AppearanceSettings, DEFAULT_KEY_HOLD_MS, DEFAULT_KEY_SEQUENCE, DEFAULT_LAYOUT, DEFAULT_PALETTE,
    DEFAULT_SNAPSHOT_CAP_MB, DialCorner, DockStart, GroupKeySettings, KeyCapture, LAYOUT_CLASSIC,
    LAYOUT_IDS, LAYOUT_MERCURY, Loaded, MAX_KEY_HOLD_MS, MAX_KEY_SEQUENCE_LEN,
    MAX_PALETTE_ID_BYTES, MAX_QUOTED_BYTES, MAX_SNAPSHOT_CAP_MB, MIN_KEY_HOLD_MS,
    MIN_SNAPSHOT_CAP_MB, Repair, SETTINGS_FORMAT_VERSION, Settings, SettingsStore,
    TabGroupSettings, Theme,
};

const LAYOUT: &str = "appearance.layout";
const PALETTE_LIGHT: &str = "appearance.palette_light";
const PALETTE_DARK: &str = "appearance.palette_dark";
const SNAPSHOT_CAP: &str = "appearance.tab_groups.snapshot_cap_mb";
const KEY_HOLD: &str = "appearance.tab_groups.keys.hold_ms";
const KEY_SEQUENCE: &str = "appearance.tab_groups.keys.default_sequence";

/// `settings.json` as this crate wrote it before the layout, palette and tab-group settings: the
/// old golden `DEFAULT_FILE` of `settings_model.rs`, kept as it was. It is a real version 1 file
/// that is still out there on disks, and must load with the new defaults and no repairs.
const FILE_BEFORE_THE_LAYOUT_SETTINGS: &str = r#"{
  "format_version": 1,
  "startup": "new_tab",
  "homepage": "",
  "search": {
    "default_engine": "duckduckgo",
    "custom_engines": [],
    "suggestions": false
  },
  "downloads": {
    "directory": null,
    "ask_where_to_save": false
  },
  "appearance": {
    "theme": "system",
    "default_zoom_percent": 100,
    "show_bookmarks_bar": true,
    "show_home_button": false
  },
  "privacy": {
    "block_ads_and_trackers": true,
    "update_filter_lists": true,
    "https_only": false,
    "block_popups": true,
    "global_privacy_control": true,
    "clear_on_exit": {
      "history": false,
      "cookies": false,
      "cache": false
    }
  },
  "passwords": {
    "offer_to_save": true,
    "never_save_origins": []
  },
  "web": {
    "experimental_web_features": true,
    "gpu_canvas": true
  },
  "language": null
}
"#;

/// A change to the appearance settings, as a plain function so tables of them can be written.
type Change = fn(&mut AppearanceSettings);

fn profile() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn settings_path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("the profile should be readable")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// Seeds `settings.json` in a new profile with `text` and loads it.
fn load_text(text: &str) -> (tempfile::TempDir, Loaded<Settings>) {
    let dir = profile();
    fs::write(settings_path(dir.path()), text).expect("seed the file");
    let loaded = SettingsStore::new(dir.path())
        .load()
        .expect("loading should not fail");
    (dir, loaded)
}

/// Loads `text` and insists it was read: the settings and the repairs.
fn load_ok(text: &str) -> (Settings, Vec<Repair>) {
    let (_dir, loaded) = load_text(text);
    match loaded {
        Loaded::Ok { value, repairs } => (value, repairs),
        other => panic!("expected Ok, got {other:?}"),
    }
}

fn fields(repairs: &[Repair]) -> Vec<&'static str> {
    repairs.iter().map(|repair| repair.field).collect()
}

/// Defaults, with `change` applied to the appearance settings.
fn with_appearance(change: impl FnOnce(&mut AppearanceSettings)) -> Settings {
    let mut settings = Settings::default();
    change(&mut settings.appearance);
    settings
}

/// Validates defaults with `change` applied; returns the appearance settings and the repairs.
fn validate(change: impl FnOnce(&mut AppearanceSettings)) -> (AppearanceSettings, Vec<Repair>) {
    let (settings, repairs) = with_appearance(change).validated();
    (settings.appearance, repairs)
}

/// Validating `appearance` again repairs nothing and changes nothing.
fn assert_stable(appearance: &AppearanceSettings) {
    let (again, repairs) = with_appearance(|a| *a = appearance.clone()).validated();
    assert_eq!(repairs, Vec::new(), "validating twice repairs nothing more");
    assert_eq!(&again.appearance, appearance);
}

// ---------------------------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------------------------

#[test]
fn the_constants_are_the_documented_ones() {
    assert_eq!(LAYOUT_MERCURY, "mercury");
    assert_eq!(LAYOUT_CLASSIC, "classic");
    assert_eq!(LAYOUT_IDS, ["mercury", "classic"]);
    assert_eq!(DEFAULT_LAYOUT, "mercury");
    assert_eq!(DEFAULT_PALETTE, "signal");
    assert_eq!(MAX_PALETTE_ID_BYTES, 64);
    assert_eq!(
        (MIN_KEY_HOLD_MS, DEFAULT_KEY_HOLD_MS, MAX_KEY_HOLD_MS),
        (250, 400, 1000)
    );
    assert_eq!(
        (
            MIN_SNAPSHOT_CAP_MB,
            DEFAULT_SNAPSHOT_CAP_MB,
            MAX_SNAPSHOT_CAP_MB
        ),
        (32, 128, 1024)
    );
    assert_eq!(DEFAULT_KEY_SEQUENCE, "ASDFGHJKL");
    assert_eq!(MAX_KEY_SEQUENCE_LEN, 26);
}

#[test]
fn the_defaults_are_the_documented_ones() {
    let appearance = AppearanceSettings::default();

    assert_eq!(
        appearance.layout, "mercury",
        "Mercury layout is the default"
    );
    assert_eq!(appearance.palette_light, "signal");
    assert_eq!(appearance.palette_dark, "signal");

    let groups = &appearance.tab_groups;
    assert!(groups.dock, "the group dock is on in the Mercury layout");
    assert!(!groups.dock_in_classic, "Classic's dock is optional: off");
    assert_eq!(groups.dock_starts, DockStart::Open);
    assert!(groups.ring, "the hub ring is on");
    assert!(groups.orbit, "Orbit is on");
    assert!(groups.orbit_pinch, "pinch to open Orbit is on");
    assert!(groups.dial, "the corner dial is on (owner decision)");
    assert_eq!(groups.dial_corner, DialCorner::BottomRight);
    assert_eq!(groups.snapshot_cap_mb, 128);

    let keys = &groups.keys;
    assert!(keys.enabled, "the option-key group keys are on");
    assert_eq!(keys.capture, KeyCapture::OutsideTextOrHold);
    assert_eq!(keys.hold_ms, 400);
    assert!(keys.show_beside_names);
    assert_eq!(keys.default_sequence, "ASDFGHJKL");

    // The two ways to make the defaults agree.
    assert_eq!(groups, &TabGroupSettings::default());
    assert_eq!(keys, &GroupKeySettings::default());
    assert_eq!(
        Settings::default().appearance,
        appearance,
        "Settings::default uses them"
    );
}

#[test]
fn the_defaults_need_no_repairs_and_are_their_own_repaired_form() {
    let (validated, repairs) = Settings::default().validated();
    assert_eq!(repairs, Vec::new());
    assert_eq!(validated, Settings::default());

    // Every default sits inside its own rule, so repairing never changes a default.
    assert!(LAYOUT_IDS.contains(&DEFAULT_LAYOUT));
    assert!((MIN_KEY_HOLD_MS..=MAX_KEY_HOLD_MS).contains(&DEFAULT_KEY_HOLD_MS));
    assert!((MIN_SNAPSHOT_CAP_MB..=MAX_SNAPSHOT_CAP_MB).contains(&DEFAULT_SNAPSHOT_CAP_MB));
    assert!(DEFAULT_KEY_SEQUENCE.len() <= MAX_KEY_SEQUENCE_LEN);
    assert!(DEFAULT_PALETTE.len() <= MAX_PALETTE_ID_BYTES);
}

#[test]
fn the_default_sequence_is_the_home_row_of_distinct_capitals() {
    let letters: Vec<char> = DEFAULT_KEY_SEQUENCE.chars().collect();
    assert_eq!(letters, ['A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L']);
}

// ---------------------------------------------------------------------------------------------
// The layout
// ---------------------------------------------------------------------------------------------

#[test]
fn both_known_layouts_are_accepted() {
    for layout in LAYOUT_IDS {
        let (appearance, repairs) = validate(|a| a.layout = layout.to_owned());
        assert_eq!(repairs, Vec::new(), "{layout}");
        assert_eq!(appearance.layout, layout);
    }
}

#[test]
fn any_other_layout_becomes_mercury_with_one_repair() {
    for layout in [
        "sidebar",
        "compact",
        "Mercury",
        "CLASSIC",
        "",
        " ",
        "mercury ",
        " mercury",
        "mercury\n",
        "classic\0",
        "mércury",
        "../classic",
    ] {
        let (appearance, repairs) = validate(|a| a.layout = layout.to_owned());

        assert_eq!(appearance.layout, "mercury", "{layout:?}");
        assert_eq!(fields(&repairs), vec![LAYOUT], "{layout:?}");
        assert_stable(&appearance);
    }
}

#[test]
fn the_layout_repair_names_the_bad_value_and_the_replacement() {
    let (_, repairs) = validate(|a| a.layout = "sidebar".to_owned());

    assert_eq!(repairs.len(), 1);
    let reason = &repairs[0].reason;
    assert!(reason.contains("\"sidebar\""), "{reason}");
    assert!(reason.contains("using \"mercury\""), "{reason}");
}

#[test]
fn a_layout_from_a_later_version_loads_as_mercury_and_does_not_damage_the_file() {
    let text = r#"{"format_version": 1, "appearance": {"layout": "sidebar", "theme": "dark"}}"#;

    let (dir, loaded) = load_text(text);

    let Loaded::Ok { value, repairs } = loaded else {
        panic!("a layout this version does not know must not damage the file");
    };
    assert_eq!(value.appearance.layout, "mercury");
    assert_eq!(
        value.appearance.theme,
        Theme::Dark,
        "the rest of the file is kept"
    );
    assert_eq!(fields(&repairs), vec![LAYOUT], "exactly one repair");
    assert_eq!(
        names_in(dir.path()),
        vec!["settings.json"],
        "nothing was moved aside"
    );
    assert_eq!(
        fs::read_to_string(settings_path(dir.path())).expect("read"),
        text,
        "loading never rewrites the file; the application saves once on repairs"
    );
}

#[test]
fn saving_a_repaired_layout_writes_mercury() {
    let (dir, loaded) = load_text(r#"{"format_version": 1, "appearance": {"layout": "sidebar"}}"#);
    let store = SettingsStore::new(dir.path());

    store.save(loaded.value()).expect("save");

    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(settings_path(dir.path())).expect("read")).expect("json");
    assert_eq!(written["appearance"]["layout"], "mercury");
    assert_eq!(
        store.load().expect("load"),
        Loaded::Ok {
            value: Settings::default(),
            repairs: Vec::new()
        },
        "the file on disk is already repaired"
    );
}

#[test]
fn classic_is_accepted_when_loaded() {
    let (value, repairs) = load_ok(
        r#"{"format_version": 1, "appearance": {"layout": "classic",
            "tab_groups": {"dock_in_classic": true}}}"#,
    );

    assert_eq!(repairs, Vec::new());
    assert_eq!(value.appearance.layout, "classic");
    assert!(value.appearance.tab_groups.dock_in_classic);
}

// ---------------------------------------------------------------------------------------------
// The palette ids
// ---------------------------------------------------------------------------------------------

#[test]
fn well_formed_palette_ids_are_accepted_whether_or_not_they_exist() {
    let longest = format!("a{}", "-".repeat(MAX_PALETTE_ID_BYTES - 1));
    assert_eq!(longest.len(), MAX_PALETTE_ID_BYTES);
    for id in [
        "mercury-b2",
        "hermes",
        "graphite",
        "solarized-9",
        "a",
        "0",
        "9-lives",
        "a-",
        "a--b",
        "x1",
        "abcdefghijklmnopqrstuvwxyz-0123456789",
        longest.as_str(),
    ] {
        let (appearance, repairs) = validate(|a| {
            a.palette_light = id.to_owned();
            a.palette_dark = id.to_owned();
        });
        assert_eq!(repairs, Vec::new(), "{id:?}");
        assert_eq!(appearance.palette_light, id);
        assert_eq!(appearance.palette_dark, id);
    }
}

#[test]
fn a_palette_id_that_is_not_well_formed_becomes_the_default_with_one_repair() {
    let too_long = "a".repeat(MAX_PALETTE_ID_BYTES + 1);
    for id in [
        "",
        "-",
        "-mercury",
        "-0",
        "Mercury",
        "mercury-B2",
        "mercury_b2",
        "mercury b2",
        " mercury-b2",
        "mercury-b2 ",
        "mercury-b2\n",
        "mercury.b2",
        "mercury/b2",
        "../mercury",
        "a\0b",
        "é",
        "mércury",
        "日本",
        "ａ",
        too_long.as_str(),
    ] {
        let (light, light_repairs) = validate(|a| a.palette_light = id.to_owned());
        assert_eq!(light.palette_light, "signal", "{id:?}");
        assert_eq!(fields(&light_repairs), vec![PALETTE_LIGHT], "{id:?}");
        assert_eq!(light.palette_dark, "signal", "the other is untouched");
        assert_stable(&light);

        let (dark, dark_repairs) = validate(|a| a.palette_dark = id.to_owned());
        assert_eq!(dark.palette_dark, "signal", "{id:?}");
        assert_eq!(fields(&dark_repairs), vec![PALETTE_DARK], "{id:?}");
        assert_stable(&dark);
    }
}

#[test]
fn the_palette_length_limit_is_64_bytes_inclusive() {
    let exact = "z".repeat(MAX_PALETTE_ID_BYTES);
    let (appearance, repairs) = validate(|a| a.palette_light = exact.clone());
    assert_eq!(repairs, Vec::new());
    assert_eq!(appearance.palette_light, exact);

    let one_over = "z".repeat(MAX_PALETTE_ID_BYTES + 1);
    let (appearance, repairs) = validate(|a| a.palette_light = one_over.clone());
    assert_eq!(fields(&repairs), vec![PALETTE_LIGHT]);
    assert_eq!(appearance.palette_light, "signal");
}

#[test]
fn the_palettes_are_checked_one_by_one() {
    let (appearance, repairs) = validate(|a| {
        a.palette_light = "hermes".to_owned();
        a.palette_dark = "BAD".to_owned();
    });
    assert_eq!(appearance.palette_light, "hermes", "a good one is kept");
    assert_eq!(appearance.palette_dark, "signal");
    assert_eq!(fields(&repairs), vec![PALETTE_DARK]);

    let (appearance, repairs) = validate(|a| {
        a.palette_light = "".to_owned();
        a.palette_dark = "later-palette".to_owned();
    });
    assert_eq!(appearance.palette_light, "signal");
    assert_eq!(appearance.palette_dark, "later-palette");
    assert_eq!(fields(&repairs), vec![PALETTE_LIGHT]);
}

#[test]
fn an_unknown_but_well_formed_palette_survives_a_load_a_save_and_another_load() {
    // A downgrade followed by an upgrade must not lose the user's choice: the id is kept on
    // disk, and only mercury-appearance falls back when it resolves it.
    let dir = profile();
    let store = SettingsStore::new(dir.path());
    fs::write(
        settings_path(dir.path()),
        r#"{"format_version": 1, "appearance": {"palette_light": "paper-2027",
            "palette_dark": "ink-2027"}}"#,
    )
    .expect("seed");

    let Loaded::Ok { value, repairs } = store.load().expect("load") else {
        panic!("expected Ok");
    };
    assert_eq!(repairs, Vec::new());
    assert_eq!(value.appearance.palette_light, "paper-2027");
    assert_eq!(value.appearance.palette_dark, "ink-2027");

    store.save(&value).expect("save");

    assert_eq!(
        store.load().expect("load again"),
        Loaded::Ok {
            value,
            repairs: Vec::new()
        }
    );
}

#[test]
fn a_malformed_palette_id_in_a_file_is_repaired_not_damaging() {
    let (dir, loaded) = load_text(
        r#"{"format_version": 1, "appearance": {"palette_light": "Not Good", "palette_dark": "ok-2"}}"#,
    );

    let Loaded::Ok { value, repairs } = loaded else {
        panic!("a bad palette id must not damage the file");
    };
    assert_eq!(value.appearance.palette_light, "signal");
    assert_eq!(value.appearance.palette_dark, "ok-2");
    assert_eq!(fields(&repairs), vec![PALETTE_LIGHT]);
    assert_eq!(names_in(dir.path()), vec!["settings.json"]);
}

// ---------------------------------------------------------------------------------------------
// The preview cap and the key hold
// ---------------------------------------------------------------------------------------------

#[test]
fn a_snapshot_cap_outside_32_to_1024_becomes_128() {
    for (cap, expected) in [
        (0, 128),
        (1, 128),
        (31, 128),
        (32, 32),
        (33, 33),
        (128, 128),
        (1023, 1023),
        (1024, 1024),
        (1025, 128),
        (2048, 128),
        (u16::MAX, 128),
    ] {
        let (appearance, repairs) = validate(|a| a.tab_groups.snapshot_cap_mb = cap);

        assert_eq!(appearance.tab_groups.snapshot_cap_mb, expected, "{cap}");
        let expected_fields = if cap == expected {
            Vec::new()
        } else {
            vec![SNAPSHOT_CAP]
        };
        assert_eq!(fields(&repairs), expected_fields, "{cap}");
        assert_stable(&appearance);
    }
}

#[test]
fn a_snapshot_cap_repair_says_what_was_wrong() {
    let (_, repairs) = validate(|a| a.tab_groups.snapshot_cap_mb = 31);

    assert_eq!(repairs.len(), 1);
    assert_eq!(
        repairs[0].reason, "31 is outside 32-1024; using 128",
        "the zoom wording"
    );
}

#[test]
fn a_key_hold_outside_250_to_1000_becomes_400() {
    for (hold, expected) in [
        (0, 400),
        (1, 400),
        (249, 400),
        (250, 250),
        (251, 251),
        (400, 400),
        (999, 999),
        (1000, 1000),
        (1001, 400),
        (5000, 400),
        (u16::MAX, 400),
    ] {
        let (appearance, repairs) = validate(|a| a.tab_groups.keys.hold_ms = hold);

        assert_eq!(appearance.tab_groups.keys.hold_ms, expected, "{hold}");
        let expected_fields = if hold == expected {
            Vec::new()
        } else {
            vec![KEY_HOLD]
        };
        assert_eq!(fields(&repairs), expected_fields, "{hold}");
        assert_stable(&appearance);
    }
}

#[test]
fn a_key_hold_repair_says_what_was_wrong() {
    let (_, repairs) = validate(|a| a.tab_groups.keys.hold_ms = 1001);

    assert_eq!(repairs.len(), 1);
    assert_eq!(repairs[0].reason, "1001 is outside 250-1000; using 400");
}

#[test]
fn a_value_too_big_for_its_integer_type_damages_the_file_while_one_in_the_type_is_repaired() {
    // 1001 fits in a u16 and is repaired; 65536 does not, and serde refuses the whole file
    // (exactly like a zoom of 70000 today).
    let (value, repairs) = load_ok(
        r#"{"format_version": 1, "appearance": {"tab_groups": {"snapshot_cap_mb": 2000,
            "keys": {"hold_ms": 1001}}}}"#,
    );
    assert_eq!(value, Settings::default());
    assert_eq!(fields(&repairs), vec![SNAPSHOT_CAP, KEY_HOLD]);

    for body in [
        r#""appearance": {"tab_groups": {"keys": {"hold_ms": 65536}}}"#,
        r#""appearance": {"tab_groups": {"snapshot_cap_mb": 65536}}"#,
        r#""appearance": {"tab_groups": {"snapshot_cap_mb": -1}}"#,
        r#""appearance": {"tab_groups": {"keys": {"hold_ms": "400"}}}"#,
    ] {
        let (dir, loaded) = load_text(&format!(r#"{{"format_version": 1, {body}}}"#));
        assert!(
            matches!(loaded, Loaded::Damaged { .. }),
            "{body}: {loaded:?}"
        );
        assert!(!settings_path(dir.path()).exists(), "{body}");
    }
}

// ---------------------------------------------------------------------------------------------
// The default key sequence
// ---------------------------------------------------------------------------------------------

fn alphabet(upper: bool) -> String {
    if upper {
        ('A'..='Z').collect()
    } else {
        ('a'..='z').collect()
    }
}

#[test]
fn a_key_sequence_keeps_its_ascii_letters_in_capitals_without_repeats() {
    let capitals = alphabet(true);
    let thirty_distinct_characters = format!("{}ÅÄÖÜ", alphabet(false));
    assert_eq!(thirty_distinct_characters.chars().count(), 30);

    let cases: Vec<(String, String)> = [
        // Already right: no repair.
        ("ASDFGHJKL", "ASDFGHJKL"),
        ("ASDF", "ASDF"),
        ("Z", "Z"),
        ("ZYX", "ZYX"), // the order is the user's: never sorted
        (capitals.as_str(), capitals.as_str()),
        // Case is folded.
        ("asdf", "ASDF"),
        ("aSdF", "ASDF"),
        // Repeats: the first of each stays.
        ("AABB", "AB"),
        ("ABAB", "AB"),
        ("aAbB", "AB"),
        ("BAAB", "BA"),
        // Whatever is not an ASCII letter goes.
        ("A1 S", "AS"),
        ("A,S;D", "ASD"),
        ("A\nS\tD", "ASD"),
        ("ÅS", "S"),
        ("s\u{17F}", "S"), // long s: capitalises to S in Unicode, but is not ASCII
        // No letter left: the default sequence.
        ("Å", "ASDFGHJKL"),
        ("\u{212A}", "ASDFGHJKL"), // Kelvin sign: lowercases to k, but is not ASCII
        ("ａｓ", "ASDFGHJKL"),     // full-width letters
        ("123", "ASDFGHJKL"),
        (" ", "ASDFGHJKL"),
        ("\u{0}", "ASDFGHJKL"),
        ("-_-", "ASDFGHJKL"),
        // Empty stays empty (valid: new groups get no key).
        ("", ""),
        // At most MAX_KEY_SEQUENCE_LEN, which is every letter once.
        (
            format!("{}0123", alphabet(false)).as_str(),
            capitals.as_str(),
        ),
        (format!("{capitals}ABCD").as_str(), capitals.as_str()),
        (thirty_distinct_characters.as_str(), capitals.as_str()),
        (
            format!("{}{capitals}", alphabet(false)).as_str(),
            capitals.as_str(),
        ),
        ("a".repeat(100_000).as_str(), "A"),
    ]
    .into_iter()
    .map(|(input, expected)| (input.to_owned(), expected.to_owned()))
    .collect();

    for (input, expected) in cases {
        let shown: String = input.chars().take(40).collect();
        let (appearance, repairs) =
            validate(|a| a.tab_groups.keys.default_sequence = input.clone());

        assert_eq!(
            appearance.tab_groups.keys.default_sequence, expected,
            "{shown:?}"
        );
        assert!(
            appearance.tab_groups.keys.default_sequence.len() <= MAX_KEY_SEQUENCE_LEN,
            "{shown:?}"
        );
        let expected_fields = if input == expected {
            Vec::new()
        } else {
            vec![KEY_SEQUENCE]
        };
        assert_eq!(fields(&repairs), expected_fields, "{shown:?}");
        assert_stable(&appearance);
    }
}

#[test]
fn an_empty_key_sequence_is_valid() {
    let (appearance, repairs) = validate(|a| a.tab_groups.keys.default_sequence.clear());

    assert_eq!(repairs, Vec::new());
    assert_eq!(appearance.tab_groups.keys.default_sequence, "");
}

#[test]
fn a_key_sequence_repair_describes_what_changed() {
    let (_, repairs) = validate(|a| a.tab_groups.keys.default_sequence = "aabb".to_owned());
    assert_eq!(repairs.len(), 1);
    let reason = &repairs[0].reason;
    assert!(reason.contains("\"aabb\""), "{reason}");
    assert!(reason.contains("\"AB\""), "{reason}");

    let (_, repairs) = validate(|a| a.tab_groups.keys.default_sequence = "123".to_owned());
    assert_eq!(repairs.len(), 1);
    let reason = &repairs[0].reason;
    assert!(reason.contains("\"123\""), "{reason}");
    assert!(reason.contains("no ASCII letter"), "{reason}");
    assert!(reason.contains("\"ASDFGHJKL\""), "{reason}");
}

// ---------------------------------------------------------------------------------------------
// All the rules together
// ---------------------------------------------------------------------------------------------

fn everything_wrong(appearance: &mut AppearanceSettings) {
    appearance.layout = "sidebar".to_owned();
    appearance.palette_light = "Light Palette".to_owned();
    appearance.palette_dark = String::new();
    appearance.tab_groups.snapshot_cap_mb = 0;
    appearance.tab_groups.keys.hold_ms = 0;
    appearance.tab_groups.keys.default_sequence = "123".to_owned();
}

#[test]
fn repairs_come_in_field_order_at_most_one_per_field_and_leave_the_defaults() {
    let (appearance, repairs) = validate(everything_wrong);

    assert_eq!(
        fields(&repairs),
        vec![
            LAYOUT,
            PALETTE_LIGHT,
            PALETTE_DARK,
            SNAPSHOT_CAP,
            KEY_HOLD,
            KEY_SEQUENCE,
        ]
    );
    assert!(repairs.iter().all(|repair| !repair.reason.is_empty()));
    assert_eq!(appearance, AppearanceSettings::default());
    assert_stable(&appearance);
}

#[test]
fn the_appearance_repairs_follow_the_zoom_and_come_before_the_privacy_lists() {
    let mut settings = Settings {
        homepage: "javascript:void(0)".to_owned(),
        language: Some("??".to_owned()),
        ..Settings::default()
    };
    settings.appearance.default_zoom_percent = 1000;
    everything_wrong(&mut settings.appearance);
    settings.passwords.never_save_origins = vec!["nonsense".to_owned()];

    let (validated, repairs) = settings.validated();

    assert_eq!(
        fields(&repairs),
        vec![
            "homepage",
            "appearance.default_zoom_percent",
            LAYOUT,
            PALETTE_LIGHT,
            PALETTE_DARK,
            SNAPSHOT_CAP,
            KEY_HOLD,
            KEY_SEQUENCE,
            "passwords.never_save_origins",
            "language",
        ]
    );
    let (_, again) = validated.validated();
    assert_eq!(again, Vec::new(), "validating twice repairs nothing more");
}

#[test]
fn one_wrong_field_does_not_disturb_the_others() {
    let keep = AppearanceSettings {
        layout: "classic".to_owned(),
        palette_light: "paper".to_owned(),
        palette_dark: "ink".to_owned(),
        tab_groups: TabGroupSettings {
            snapshot_cap_mb: 512,
            keys: GroupKeySettings {
                hold_ms: 700,
                default_sequence: "QWERTY".to_owned(),
                ..GroupKeySettings::default()
            },
            ..TabGroupSettings::default()
        },
        ..AppearanceSettings::default()
    };
    // The field, a change that breaks it, and the change the repair makes.
    let table: [(&str, Change, Change); 6] = [
        (
            LAYOUT,
            |a| a.layout = "x".to_owned(),
            |a| a.layout = "mercury".to_owned(),
        ),
        (
            PALETTE_LIGHT,
            |a| a.palette_light = "X".to_owned(),
            |a| a.palette_light = "signal".to_owned(),
        ),
        (
            PALETTE_DARK,
            |a| a.palette_dark = "X".to_owned(),
            |a| a.palette_dark = "signal".to_owned(),
        ),
        (
            SNAPSHOT_CAP,
            |a| a.tab_groups.snapshot_cap_mb = 1,
            |a| a.tab_groups.snapshot_cap_mb = 128,
        ),
        (
            KEY_HOLD,
            |a| a.tab_groups.keys.hold_ms = 1,
            |a| a.tab_groups.keys.hold_ms = 400,
        ),
        (
            KEY_SEQUENCE,
            |a| a.tab_groups.keys.default_sequence = "1".to_owned(),
            |a| a.tab_groups.keys.default_sequence = "ASDFGHJKL".to_owned(),
        ),
    ];

    for (field, break_it, repaired) in table {
        let (appearance, repairs) = validate(|a| {
            *a = keep.clone();
            break_it(a);
        });

        assert_eq!(fields(&repairs), vec![field]);
        let mut expected = keep.clone();
        repaired(&mut expected);
        assert_eq!(appearance, expected, "{field}");
    }
}

#[test]
fn valid_non_default_values_need_no_repairs() {
    let (appearance, repairs) = validate(|a| {
        a.layout = "classic".to_owned();
        a.palette_light = "later-palette".to_owned();
        a.palette_dark = "0-dark".to_owned();
        a.tab_groups.snapshot_cap_mb = MAX_SNAPSHOT_CAP_MB;
        a.tab_groups.keys.hold_ms = MIN_KEY_HOLD_MS;
        a.tab_groups.keys.default_sequence = "QWERTY".to_owned();
        a.tab_groups.dial_corner = DialCorner::TopLeft;
        a.tab_groups.dock_starts = DockStart::Shrunk;
        a.tab_groups.keys.capture = KeyCapture::Always;
    });

    assert_eq!(repairs, Vec::new());
    assert_eq!(appearance.layout, "classic");
    assert_eq!(appearance.palette_light, "later-palette");
    assert_eq!(appearance.palette_dark, "0-dark");
    assert_eq!(appearance.tab_groups.snapshot_cap_mb, 1024);
    assert_eq!(appearance.tab_groups.keys.hold_ms, 250);
    assert_eq!(appearance.tab_groups.keys.default_sequence, "QWERTY");
}

#[test]
fn repair_messages_quote_only_a_short_piece_of_the_file() {
    let long = "x".repeat(100_000);
    let (appearance, repairs) = validate(|a| {
        a.layout = long.clone();
        a.palette_light = long.clone();
        a.palette_dark = format!("-{long}");
        a.tab_groups.keys.default_sequence = "1".repeat(100_000);
    });

    assert_eq!(
        fields(&repairs),
        vec![LAYOUT, PALETTE_LIGHT, PALETTE_DARK, KEY_SEQUENCE]
    );
    for repair in &repairs {
        assert!(
            repair.reason.len() < MAX_QUOTED_BYTES + 250,
            "{}: {} bytes",
            repair.field,
            repair.reason.len()
        );
    }
    assert_eq!(appearance, AppearanceSettings::default());

    // A letters-only sequence is quoted short too, and keeps the letters it has.
    let (appearance, repairs) = validate(|a| {
        a.tab_groups.keys.default_sequence = format!("{}{}", "ab".repeat(50_000), "1");
    });
    assert_eq!(appearance.tab_groups.keys.default_sequence, "AB");
    assert_eq!(repairs.len(), 1);
    assert!(repairs[0].reason.len() < MAX_QUOTED_BYTES + 250);
}

// ---------------------------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------------------------

/// Every stored field of the new settings, each changed from its default on its own.
fn single_changes() -> [(&'static str, Change); 17] {
    [
        ("layout", |a| a.layout = "classic".to_owned()),
        ("palette_light", |a| a.palette_light = "paper".to_owned()),
        ("palette_dark", |a| a.palette_dark = "ink".to_owned()),
        ("dock", |a| a.tab_groups.dock = false),
        ("dock_in_classic", |a| a.tab_groups.dock_in_classic = true),
        ("dock_starts", |a| {
            a.tab_groups.dock_starts = DockStart::Shrunk;
        }),
        ("ring", |a| a.tab_groups.ring = false),
        ("orbit", |a| a.tab_groups.orbit = false),
        ("orbit_pinch", |a| a.tab_groups.orbit_pinch = false),
        ("dial", |a| a.tab_groups.dial = false),
        ("dial_corner", |a| {
            a.tab_groups.dial_corner = DialCorner::TopLeft;
        }),
        ("snapshot_cap_mb", |a| a.tab_groups.snapshot_cap_mb = 512),
        ("keys.enabled", |a| a.tab_groups.keys.enabled = false),
        ("keys.capture", |a| {
            a.tab_groups.keys.capture = KeyCapture::Always;
        }),
        ("keys.hold_ms", |a| a.tab_groups.keys.hold_ms = 750),
        ("keys.show_beside_names", |a| {
            a.tab_groups.keys.show_beside_names = false;
        }),
        ("keys.default_sequence", |a| {
            a.tab_groups.keys.default_sequence = "QWERTY".to_owned();
        }),
    ]
}

fn round_trip(settings: &Settings) -> Loaded<Settings> {
    let dir = profile();
    let store = SettingsStore::new(dir.path());
    store.save(settings).expect("save");
    store.load().expect("load")
}

#[test]
fn every_stored_field_survives_a_save_and_a_load_on_its_own() {
    for (name, change) in single_changes() {
        let settings = with_appearance(change);
        assert_ne!(
            settings,
            Settings::default(),
            "{name} must differ from its default for this test to mean anything"
        );

        assert_eq!(
            round_trip(&settings),
            Loaded::Ok {
                value: settings,
                repairs: Vec::new()
            },
            "{name}"
        );
    }
}

#[test]
fn every_non_default_value_survives_a_save_and_a_load_together() {
    let settings = with_appearance(|a| {
        for (_, change) in single_changes() {
            change(a);
        }
    });
    assert_eq!(
        settings.appearance.tab_groups,
        TabGroupSettings {
            dock: false,
            dock_in_classic: true,
            dock_starts: DockStart::Shrunk,
            ring: false,
            orbit: false,
            orbit_pinch: false,
            dial: false,
            dial_corner: DialCorner::TopLeft,
            snapshot_cap_mb: 512,
            keys: GroupKeySettings {
                enabled: false,
                capture: KeyCapture::Always,
                hold_ms: 750,
                show_beside_names: false,
                default_sequence: "QWERTY".to_owned(),
            },
        },
        "single_changes() covers every field of the tab-group settings"
    );

    assert_eq!(
        round_trip(&settings),
        Loaded::Ok {
            value: settings,
            repairs: Vec::new()
        }
    );
}

#[test]
fn every_variant_of_the_closed_sets_survives_a_save_and_a_load() {
    for dock_starts in [DockStart::Open, DockStart::Shrunk] {
        for dial_corner in [
            DialCorner::TopLeft,
            DialCorner::TopRight,
            DialCorner::BottomLeft,
            DialCorner::BottomRight,
        ] {
            for capture in [
                KeyCapture::OutsideTextOrHold,
                KeyCapture::OutsideText,
                KeyCapture::Always,
            ] {
                let settings = with_appearance(|a| {
                    a.tab_groups.dock_starts = dock_starts;
                    a.tab_groups.dial_corner = dial_corner;
                    a.tab_groups.keys.capture = capture;
                });
                assert_eq!(
                    round_trip(&settings),
                    Loaded::Ok {
                        value: settings,
                        repairs: Vec::new()
                    },
                    "{dock_starts:?} {dial_corner:?} {capture:?}"
                );
            }
        }
    }
}

#[test]
fn the_closed_sets_are_written_with_their_names_forever() {
    // These strings are an API: a file written today is read by every later Mercury.
    let settings = with_appearance(|a| {
        a.tab_groups.dock_starts = DockStart::Shrunk;
        a.tab_groups.dial_corner = DialCorner::BottomLeft;
        a.tab_groups.keys.capture = KeyCapture::OutsideText;
    });
    let dir = profile();
    SettingsStore::new(dir.path())
        .save(&settings)
        .expect("save");

    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(settings_path(dir.path())).expect("read")).expect("json");
    let groups = &written["appearance"]["tab_groups"];
    assert_eq!(groups["dock_starts"], "shrunk");
    assert_eq!(groups["dial_corner"], "bottom_left");
    assert_eq!(groups["keys"]["capture"], "outside_text");
    assert_eq!(written["format_version"], SETTINGS_FORMAT_VERSION);
    assert_eq!(
        SETTINGS_FORMAT_VERSION, 1,
        "the new fields are additive: no bump"
    );

    for (variant, name) in [
        (DialCorner::TopLeft, "top_left"),
        (DialCorner::TopRight, "top_right"),
        (DialCorner::BottomLeft, "bottom_left"),
        (DialCorner::BottomRight, "bottom_right"),
    ] {
        assert_eq!(
            serde_json::to_value(variant).expect("serialise"),
            name,
            "{variant:?}"
        );
    }
    for (variant, name) in [(DockStart::Open, "open"), (DockStart::Shrunk, "shrunk")] {
        assert_eq!(serde_json::to_value(variant).expect("serialise"), name);
    }
    for (variant, name) in [
        (KeyCapture::OutsideTextOrHold, "outside_text_or_hold"),
        (KeyCapture::OutsideText, "outside_text"),
        (KeyCapture::Always, "always"),
    ] {
        assert_eq!(serde_json::to_value(variant).expect("serialise"), name);
    }
}

#[test]
fn saving_repairs_the_new_fields_before_writing() {
    let dir = profile();
    let store = SettingsStore::new(dir.path());
    let settings = with_appearance(everything_wrong);

    store.save(&settings).expect("save");

    assert_eq!(
        store.load().expect("load"),
        Loaded::Ok {
            value: Settings::default(),
            repairs: Vec::new()
        },
        "the file on disk is already repaired"
    );
}

// ---------------------------------------------------------------------------------------------
// Files written by other versions
// ---------------------------------------------------------------------------------------------

#[test]
fn a_partial_tab_groups_object_keeps_what_it_has_and_defaults_the_rest() {
    // No `format_version`: a hand-written (version 0) document.
    let (value, repairs) = load_ok(r#"{"appearance":{"tab_groups":{"dial":false}}}"#);

    let mut expected = Settings::default();
    expected.appearance.tab_groups.dial = false;
    assert_eq!(value, expected);
    assert_eq!(repairs, Vec::new());

    let (value, repairs) = load_ok(
        r#"{"format_version": 1, "appearance": {"tab_groups": {"keys": {"hold_ms": 600}}}}"#,
    );
    let mut expected = Settings::default();
    expected.appearance.tab_groups.keys.hold_ms = 600;
    assert_eq!(value, expected, "a partial `keys` keeps the other keys");
    assert_eq!(repairs, Vec::new());
}

#[test]
fn the_file_written_before_this_lane_loads_with_the_new_defaults_and_no_repairs() {
    let (dir, loaded) = load_text(FILE_BEFORE_THE_LAYOUT_SETTINGS);

    assert_eq!(
        loaded,
        Loaded::Ok {
            value: Settings::default(),
            repairs: Vec::new()
        }
    );
    assert_eq!(names_in(dir.path()), vec!["settings.json"]);
    assert_eq!(
        fs::read_to_string(settings_path(dir.path())).expect("read"),
        FILE_BEFORE_THE_LAYOUT_SETTINGS,
        "loading does not rewrite it"
    );
}

#[test]
fn the_old_file_keeps_its_own_choices_and_gains_the_new_fields_on_the_next_save() {
    let old = FILE_BEFORE_THE_LAYOUT_SETTINGS
        .replace(r#""theme": "system""#, r#""theme": "dark""#)
        .replace(
            r#""default_zoom_percent": 100"#,
            r#""default_zoom_percent": 125"#,
        )
        .replace(
            r#""show_home_button": false"#,
            r#""show_home_button": true"#,
        );
    assert_ne!(old, FILE_BEFORE_THE_LAYOUT_SETTINGS);

    let (dir, loaded) = load_text(&old);
    let Loaded::Ok { value, repairs } = loaded else {
        panic!("expected Ok, got {loaded:?}");
    };
    assert_eq!(repairs, Vec::new());
    assert_eq!(value.appearance.theme, Theme::Dark);
    assert_eq!(value.appearance.default_zoom_percent, 125);
    assert!(value.appearance.show_home_button);
    assert_eq!(value.appearance.layout, "mercury");
    assert_eq!(value.appearance.palette_light, "signal");
    assert_eq!(value.appearance.palette_dark, "signal");
    assert_eq!(value.appearance.tab_groups, TabGroupSettings::default());

    SettingsStore::new(dir.path()).save(&value).expect("save");
    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(settings_path(dir.path())).expect("read")).expect("json");
    assert_eq!(written["format_version"], 1);
    assert_eq!(written["appearance"]["layout"], "mercury");
    assert_eq!(written["appearance"]["palette_light"], "signal");
    assert_eq!(written["appearance"]["tab_groups"]["dial"], true);
    assert_eq!(written["appearance"]["theme"], "dark");
}

#[test]
fn unknown_fields_are_ignored_at_every_level_of_the_new_settings() {
    let (value, repairs) = load_ok(
        r#"{"format_version": 1, "future_top_level": [1, 2, 3],
            "appearance": {"accent": "lavender", "layout": "classic",
              "tab_groups": {"orbit": false, "future_gizmo": {"a": 1}, "dial_scale": 1.5,
                "keys": {"hold_ms": 500, "future_key": "x", "sound": null}}}}"#,
    );

    let mut expected = Settings::default();
    expected.appearance.layout = "classic".to_owned();
    expected.appearance.tab_groups.orbit = false;
    expected.appearance.tab_groups.keys.hold_ms = 500;
    assert_eq!(value, expected);
    assert_eq!(repairs, Vec::new());
}

#[test]
fn a_value_of_a_closed_set_that_this_version_does_not_know_damages_the_file() {
    // The same as `"theme": "sepia"` today, and the reason a new variant of any closed set
    // needs a format bump (see store.rs): an older reader cannot tell what it means, so it
    // moves the file aside rather than guess. Layout and palette ids are strings for the
    // opposite reason.
    let bodies = [
        r#""appearance": {"tab_groups": {"dial_corner": "centre"}}"#,
        r#""appearance": {"tab_groups": {"dock_starts": "hidden"}}"#,
        r#""appearance": {"tab_groups": {"keys": {"capture": "sometimes"}}}"#,
        r#""appearance": {"theme": "sepia"}"#,
        r#""appearance": {"layout": 3}"#,
        r#""appearance": {"tab_groups": {"dial": "yes"}}"#,
        r#""appearance": {"tab_groups": "everything"}"#,
    ];
    for body in bodies {
        let text =
            format!(r#"{{"format_version": 1, "homepage": "https://example.org/", {body}}}"#);

        let (dir, loaded) = load_text(&text);

        let Loaded::Damaged { value, backup } = loaded else {
            panic!("{body}: expected Damaged, got {loaded:?}");
        };
        assert_eq!(value, Settings::default(), "{body}");
        assert!(
            !settings_path(dir.path()).exists(),
            "{body}: the file was moved aside"
        );
        assert_eq!(
            fs::read_to_string(&backup).expect("the backup is readable"),
            text,
            "{body}"
        );
        assert_eq!(backup.parent(), Some(dir.path()));
    }
}
