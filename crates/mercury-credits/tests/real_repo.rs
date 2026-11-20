// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The real `deny.toml`, `third_party/vendor.toml` and `credits.toml` of this repository, read by
//! the tool's own parsers.
use mercury_credits::config::{Config, DenyPolicy, VendorFile};
use mercury_credits::spdx;
use std::path::{Component, Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn deny_toml_gives_an_allow_list_with_the_permissive_licences_and_no_copyleft() {
    let deny = DenyPolicy::load(&root()).unwrap();
    for expected in [
        "MIT",
        "Apache-2.0",
        "MPL-2.0",
        "BSD-3-Clause",
        "ISC",
        "Zlib",
        "Unicode-3.0",
        "Apache-2.0 WITH LLVM-exception",
    ] {
        assert!(
            deny.allow.iter().any(|id| id == expected),
            "{expected} in {:?}",
            deny.allow
        );
    }
    for copyleft in deny.allow.iter() {
        assert!(!copyleft.contains("GPL"), "{copyleft} is copyleft");
    }
    // Every entry is one term: `Apache-2.0 WITH LLVM-exception` counts as one.
    for entry in &deny.allow {
        let expr = spdx::parse(entry).unwrap();
        assert_eq!(expr.terms().len(), 1, "{entry}");
    }
}

#[test]
fn slint_gets_its_royalty_free_exception_and_only_that_branch_is_credited() {
    let deny = DenyPolicy::load(&root()).unwrap();
    let slint = deny.taken_for("slint", "1.17.1");
    assert!(slint.contains_key("LicenseRef-Slint-Royalty-free-2.0"));
    assert!(
        slint.contains_key("MIT"),
        "the global allow-list still applies"
    );
    let other = deny.taken_for("serde", "1.0.0");
    assert!(!other.contains_key("LicenseRef-Slint-Royalty-free-2.0"));

    let expression = spdx::parse(
        "GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0",
    )
    .unwrap();
    assert_eq!(
        expression.credited(&slint),
        Some(vec!["LicenseRef-Slint-Royalty-free-2.0".to_owned()])
    );
    assert_eq!(expression.credited(&other), None);
    for name in [
        "slint",
        "slint-build",
        "slint-macros",
        "i-slint-backend-selector",
        "i-slint-backend-winit",
        "i-slint-common",
        "i-slint-compiler",
        "i-slint-core",
        "i-slint-core-macros",
        "i-slint-renderer-femtovg",
    ] {
        assert!(
            deny.taken_for(name, "1.17.1")
                .contains_key("LicenseRef-Slint-Royalty-free-2.0"),
            "{name} has the exception"
        );
    }
}

#[test]
fn vendor_toml_lists_servo_and_slints_winit_backend_with_its_licence_note() {
    let vendor = VendorFile::load(&root()).unwrap();
    let servo = vendor.find("servo", "0.8.0").expect("servo is vendored");
    assert_eq!(
        servo.repository.as_deref(),
        Some("https://github.com/servo/servo")
    );
    assert_eq!(
        servo.commit.as_ref().map(String::len),
        Some(40),
        "a full commit hash"
    );
    assert_eq!(servo.license.as_deref(), Some("MPL-2.0"));

    let winit = vendor
        .find("i-slint-backend-winit", "1.17.1")
        .expect("the winit backend is vendored");
    let note = winit.license_note.as_deref().expect("its licence note");
    assert!(note.contains("LicenseRef-Slint-Royalty-free-2.0"), "{note}");
    assert_eq!(
        winit.repository.as_deref(),
        Some("https://github.com/slint-ui/slint")
    );

    assert!(vendor.find("gaol", "0.2.1").is_some());
    assert!(
        vendor.crates.len() >= 20,
        "{} vendored crates",
        vendor.crates.len()
    );
    for krate in &vendor.crates {
        assert!(
            krate
                .repository
                .as_deref()
                .is_some_and(|url| url.starts_with("https://")),
            "{} has a repository",
            krate.name
        );
        assert!(krate.commit.is_some(), "{} has a commit", krate.name);
    }
}

#[test]
fn credits_toml_parses_and_names_only_relative_paths_inside_packages() {
    let config = Config::load(&root()).unwrap();
    for extra in &config.extra {
        for file in &extra.files {
            let path = Path::new(file);
            assert!(path.is_relative(), "{}: {file}", extra.name);
            assert!(
                !path
                    .components()
                    .any(|part| matches!(part, Component::ParentDir)),
                "{}: {file}",
                extra.name
            );
        }
    }
    for clarify in &config.clarify {
        if let Some(licence) = &clarify.licence {
            spdx::parse(licence).unwrap();
        }
    }
    for ignore in &config.ignore {
        assert!(
            !ignore.reason.trim().is_empty(),
            "{} needs a reason",
            ignore.name
        );
    }
}

#[test]
fn the_workspace_has_the_version_and_lock_file_the_page_shows() {
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let table: toml::Table = manifest.parse().unwrap();
    let version = table["workspace"]["package"]["version"].as_str().unwrap();
    assert!(version.split('.').count() >= 3, "{version}");
    assert!(root().join("Cargo.lock").is_file());
    assert!(root().join("NOTICE").is_file());
    assert!(root().join("THIRD-PARTY-NOTICES.md").is_file());
}
