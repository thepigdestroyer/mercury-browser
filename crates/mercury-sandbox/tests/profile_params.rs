// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The parameters of the content profile: values reach the kernel unchanged, bad paths are
//! refused, and the run root and the bundle are derived the same way every time.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use mercury_sandbox::{
    ProfileError, ProfileInputs, bundle_root, content_profile, report_profile, run_root_from_env,
};

fn inputs() -> ProfileInputs {
    ProfileInputs::new(
        PathBuf::from("/Applications/Mercury.app"),
        PathBuf::from("/Applications/Mercury.app/Contents/MacOS/mercury-browser"),
        PathBuf::from("/Users/alice"),
    )
}

fn environment(pairs: &[(&str, String)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), OsString::from(value)))
        .collect();
    move |name: &str| map.get(name).cloned()
}

fn xdg_under(root: &str) -> Vec<(&'static str, String)> {
    vec![
        ("XDG_CACHE_HOME", format!("{root}/cache")),
        ("XDG_DATA_HOME", format!("{root}/data")),
        ("XDG_CONFIG_HOME", format!("{root}/config")),
        ("XDG_RUNTIME_DIR", format!("{root}/runtime")),
    ]
}

fn set(vars: &mut [(&'static str, String)], name: &str, value: &str) {
    for (key, current) in vars.iter_mut() {
        if *key == name {
            *current = value.to_owned();
        }
    }
}

#[test]
fn hostile_path_values_are_unchanged_parameters() {
    let home = "/Users/a \"b\" (c)\nd/é";
    let mut inputs = inputs();
    inputs.home = PathBuf::from(home);
    let profile = content_profile(&inputs).unwrap();
    assert_eq!(profile.parameter("HOMEDIR"), Some(home));
    assert!(!profile.text.contains(home));
}

#[test]
fn relative_dotdot_empty_nul_and_root_are_refused() {
    let mut relative = inputs();
    relative.home = PathBuf::from("Users/alice");
    assert_eq!(
        content_profile(&relative),
        Err(ProfileError::NotAbsolute("HOMEDIR"))
    );

    let mut dotdot = inputs();
    dotdot.bundle_path = PathBuf::from("/Applications/../etc");
    assert_eq!(
        content_profile(&dotdot),
        Err(ProfileError::ParentComponent("BUNDLE_PATH"))
    );

    let mut empty = inputs();
    empty.home = PathBuf::new();
    assert_eq!(content_profile(&empty), Err(ProfileError::Empty("HOMEDIR")));

    let mut nul = inputs();
    nul.home = PathBuf::from("/Users/a\0b");
    assert_eq!(content_profile(&nul), Err(ProfileError::Nul("HOMEDIR")));

    let mut root = inputs();
    root.bundle_path = PathBuf::from("/");
    assert_eq!(
        content_profile(&root),
        Err(ProfileError::Root("BUNDLE_PATH"))
    );

    let mut root_home = inputs();
    root_home.home = PathBuf::from("/");
    assert_eq!(
        content_profile(&root_home),
        Err(ProfileError::Root("HOMEDIR"))
    );

    let mut run_root_relative = inputs();
    run_root_relative.run_root = Some(PathBuf::from("run"));
    assert_eq!(
        content_profile(&run_root_relative),
        Err(ProfileError::NotAbsolute("RUN_ROOT"))
    );
}

#[test]
fn run_root_is_optional() {
    let profile = content_profile(&inputs()).unwrap();
    assert_eq!(profile.parameter("RUN_ROOT"), None);
    assert!(
        profile
            .parameters
            .iter()
            .all(|(name, _)| name != "RUN_ROOT")
    );
}

#[test]
fn booleans_are_exactly_true_or_false() {
    let mut inputs = inputs();
    inputs.audio_output = true;
    inputs.media_decode = false;
    inputs.capture = true;
    inputs.logging = false;
    let profile = content_profile(&inputs).unwrap();
    assert_eq!(profile.parameter("ALLOW_AUDIO_OUT"), Some("TRUE"));
    assert_eq!(profile.parameter("ALLOW_MEDIA_DECODE"), Some("FALSE"));
    assert_eq!(profile.parameter("ALLOW_CAPTURE"), Some("TRUE"));
    assert_eq!(profile.parameter("ENABLE_LOGGING"), Some("FALSE"));
    for (name, value) in &profile.parameters {
        if name.starts_with("ALLOW_") || name.starts_with("ENABLE_") {
            assert!(value == "TRUE" || value == "FALSE", "{name} is {value}");
        }
    }
}

#[test]
fn bundle_root_of_an_app_and_of_a_dev_binary() {
    assert_eq!(
        bundle_root(Path::new(
            "/Applications/Mercury.app/Contents/MacOS/mercury-browser"
        )),
        PathBuf::from("/Applications/Mercury.app")
    );
    assert_eq!(
        bundle_root(Path::new(
            "/Users/dev/mercury/target/release/mercury-browser"
        )),
        PathBuf::from("/Users/dev/mercury/target/release")
    );
}

#[test]
fn run_root_needs_all_four_xdg_variables_under_one_parent() {
    let all = xdg_under("/var/folders/x/run");
    assert_eq!(
        run_root_from_env(&environment(&all)),
        Some(PathBuf::from("/var/folders/x/run"))
    );

    let missing: Vec<_> = all
        .iter()
        .filter(|(key, _)| *key != "XDG_RUNTIME_DIR")
        .cloned()
        .collect();
    assert_eq!(run_root_from_env(&environment(&missing)), None);

    let mut other_parent = xdg_under("/var/folders/x/run");
    set(
        &mut other_parent,
        "XDG_RUNTIME_DIR",
        "/var/folders/x/other/runtime",
    );
    assert_eq!(run_root_from_env(&environment(&other_parent)), None);

    let mut relative = xdg_under("/var/folders/x/run");
    set(&mut relative, "XDG_CACHE_HOME", "run/cache");
    assert_eq!(run_root_from_env(&environment(&relative)), None);

    let at_root = xdg_under("");
    assert_eq!(run_root_from_env(&environment(&at_root)), None);
}

#[test]
fn report_profile_has_no_parameters() {
    let profile = report_profile();
    assert!(profile.parameters.is_empty());
    assert!(profile.text.contains("(allow default (with report))"));
}
