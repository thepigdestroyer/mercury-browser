// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Reading the output of `cargo tree` and `cargo metadata`.
use mercury_credits::cargo::{self, Metadata, Options, TreeEntry};
use std::path::{Path, PathBuf};

const TREE: &str = "\
mercury-desktop v0.2.0 (/work/apps/mercury-desktop)
bytemuck_derive v1.12.1 (proc-macro)
proc-macro2 v1.0.107
proc-macro2 v1.0.107 (*)
quote v1.0.47 (*)
servo v0.8.0 (/work/third_party/servo)
servo v0.8.0 (/work/third_party/servo) (*)
toml v0.9.12+spec-1.1.0
toml v1.2.1+spec-1.1.0 (*)
zerocopy-derive v0.8.63 (proc-macro) (*)
";

fn names(entries: &[TreeEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| format!("{} {}", entry.name, entry.version))
        .collect()
}

#[test]
fn the_tree_lists_each_crate_once_whatever_marks_follow() {
    let entries = cargo::parse_tree(TREE).unwrap();
    assert_eq!(
        names(&entries),
        [
            "bytemuck_derive 1.12.1",
            "mercury-desktop 0.2.0",
            "proc-macro2 1.0.107",
            "quote 1.0.47",
            "servo 0.8.0",
            "toml 0.9.12+spec-1.1.0",
            "toml 1.2.1+spec-1.1.0",
            "zerocopy-derive 0.8.63",
        ]
    );
}

#[test]
fn proc_macro_path_and_build_metadata_are_understood() {
    let entries = cargo::parse_tree(TREE).unwrap();
    let find = |name: &str| entries.iter().find(|entry| entry.name == name).unwrap();
    assert!(find("bytemuck_derive").proc_macro);
    assert!(
        find("zerocopy-derive").proc_macro,
        "a (proc-macro) before the (*)"
    );
    assert!(
        !find("proc-macro2").proc_macro,
        "a name that contains the word is not a marker"
    );
    assert_eq!(find("servo").path(), Some("/work/third_party/servo"));
    assert_eq!(
        find("servo").location.as_deref(),
        Some("/work/third_party/servo")
    );
    assert_eq!(find("quote").location, None);
    assert_eq!(find("quote").path(), None);
    let versions: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.name == "toml")
        .map(|entry| entry.version.as_str())
        .collect();
    assert_eq!(versions, ["0.9.12+spec-1.1.0", "1.2.1+spec-1.1.0"]);
}

#[test]
fn a_path_with_spaces_and_a_git_source_are_kept_as_locations() {
    let entries = cargo::parse_tree(
        "mine v1.0.0 (/Users/some one/my (copy)/crates/mine)\nfrom-git v0.1.0 (https://example.org/repo#abc123)\n",
    )
    .unwrap();
    assert_eq!(entries[0].name, "from-git");
    assert_eq!(entries[0].path(), None);
    assert_eq!(
        entries[0].location.as_deref(),
        Some("https://example.org/repo#abc123")
    );
    assert_eq!(
        entries[1].path(),
        Some("/Users/some one/my (copy)/crates/mine")
    );
}

#[test]
fn headings_and_blank_lines_are_skipped_and_nonsense_is_an_error() {
    let entries = cargo::parse_tree("\n[build-dependencies]\nfoo v1.0.0\n\n").unwrap();
    assert_eq!(names(&entries), ["foo 1.0.0"]);
    assert!(cargo::parse_tree("").unwrap().is_empty());
    for bad in [
        "foo",
        "foo 1.0.0",
        "foo v",
        "foo v1.0.0 (unclosed",
        "foo v1.0.0 extra",
    ] {
        let error = cargo::parse_tree(bad).unwrap_err().to_string();
        assert!(error.contains("cargo tree"), "{bad}: {error}");
    }
}

fn metadata_json() -> String {
    r#"{
  "packages": [
    {"id": "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0", "name": "serde", "version": "1.0.0",
     "source": "registry+https://github.com/rust-lang/crates.io-index", "license": "MIT OR Apache-2.0", "license_file": null,
     "authors": ["David <d@example.org>"], "repository": "https://github.com/serde-rs/serde",
     "manifest_path": "/reg/serde-1.0.0/Cargo.toml", "links": null, "description": "ignored", "dependencies": [{"name": "x"}]},
    {"id": "path+file:///work/third_party/servo#0.8.0", "name": "servo", "version": "0.8.0", "source": null,
     "license": "MPL-2.0", "authors": [], "repository": null, "manifest_path": "/work/third_party/servo/Cargo.toml"},
    {"id": "registry+https://github.com/rust-lang/crates.io-index#servo@0.8.0", "name": "servo", "version": "0.8.0",
     "source": "registry+https://github.com/rust-lang/crates.io-index", "license": "MPL-2.0", "authors": [],
     "manifest_path": "/reg/servo-0.8.0/Cargo.toml"},
    {"id": "path+file:///work/apps/mercury-desktop#0.2.0", "name": "mercury-desktop", "version": "0.2.0", "source": null,
     "license": null, "license_file": "LICENSE", "authors": [], "manifest_path": "/work/apps/mercury-desktop/Cargo.toml", "links": "native"}
  ],
  "workspace_members": ["path+file:///work/apps/mercury-desktop#0.2.0"],
  "resolve": null,
  "target_directory": "/work/target"
}"#
    .to_owned()
}

#[test]
fn metadata_keeps_only_the_fields_the_tool_needs() {
    let metadata = Metadata::parse(metadata_json().as_bytes()).unwrap();
    assert_eq!(metadata.packages.len(), 4);
    let serde = &metadata.packages[0];
    assert_eq!(serde.license.as_deref(), Some("MIT OR Apache-2.0"));
    assert_eq!(serde.authors, ["David <d@example.org>"]);
    assert_eq!(
        serde.repository.as_deref(),
        Some("https://github.com/serde-rs/serde")
    );
    assert_eq!(serde.root(), Path::new("/reg/serde-1.0.0"));
    let desktop = &metadata.packages[3];
    assert_eq!(desktop.license, None);
    assert_eq!(desktop.license_file.as_deref(), Some("LICENSE"));
    assert_eq!(desktop.links.as_deref(), Some("native"));
    assert!(metadata.is_workspace_member(desktop));
    assert!(!metadata.is_workspace_member(serde));
    assert!(Metadata::parse(b"not json").is_err());
}

#[test]
fn entries_are_matched_by_name_version_and_for_path_crates_by_directory() {
    let metadata = Metadata::parse(metadata_json().as_bytes()).unwrap();
    let entries = cargo::parse_tree(
        "serde v1.0.0\nservo v0.8.0 (/work/third_party/servo)\nservo v0.8.0\nserde v9.9.9\nservo v0.8.0 (/elsewhere/servo)\n",
    )
    .unwrap();
    let find = |name: &str, version: &str, path: Option<&str>| {
        let entry = entries
            .iter()
            .find(|entry| entry.name == name && entry.version == version && entry.path() == path)
            .unwrap();
        metadata.find(entry).map(|package| package.id.clone())
    };
    assert_eq!(
        find("serde", "1.0.0", None).as_deref(),
        Some("registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0")
    );
    assert_eq!(
        find("servo", "0.8.0", Some("/work/third_party/servo")).as_deref(),
        Some("path+file:///work/third_party/servo#0.8.0"),
        "the path crate, not the registry crate of the same name and version"
    );
    assert_eq!(
        find("servo", "0.8.0", None).as_deref(),
        Some("registry+https://github.com/rust-lang/crates.io-index#servo@0.8.0")
    );
    assert_eq!(find("serde", "9.9.9", None), None);
    assert_eq!(find("servo", "0.8.0", Some("/elsewhere/servo")), None);
}

#[test]
fn the_root_is_the_directory_of_the_manifest() {
    let options = |manifest: &str| Options {
        package: "p".into(),
        target: "t".into(),
        runtime_only: false,
        manifest_path: PathBuf::from(manifest),
        cargo: PathBuf::from("cargo"),
    };
    assert_eq!(options("/work/Cargo.toml").root(), Path::new("/work"));
    assert_eq!(options("Cargo.toml").root(), Path::new("."));
}
