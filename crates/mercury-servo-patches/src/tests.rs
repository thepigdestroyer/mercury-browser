// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The commands end to end, on a throwaway repository with fixture archives.

use crate::archive::fixture::pack;
use crate::archive::sha256_hex;
use crate::commands::{Outcome, Repo, UpgradeTarget, workspace_problems};
use crate::manifest::Manifest;
use crate::registry::Registry;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const LIB_V1: &str = "pub fn one() -> u32 {\n    1\n}\n\npub fn two() -> u32 {\n    2\n}\n\npub fn three() -> u32 {\n    3\n}\n";
const LIB_V2: &str = "pub fn one() -> u32 {\n    1\n}\n\npub fn two() -> u32 {\n    2\n}\n\npub fn three() -> u32 {\n    3 // upstream\n}\n";

fn cargo_toml(name: &str, version: &str, repository: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"{version}\"\nrepository = \"{repository}\"\nlicense = \"MPL-2.0\"\n"
    )
}

fn vcs_info(commit: &str) -> String {
    format!(
        "{{\n  \"git\": {{\n    \"sha1\": \"{commit}\"\n  }},\n  \"path_in_vcs\": \"components/demo\"\n}}"
    )
}

struct Fixture {
    _dir: TempDir,
    repo: Repo,
    v1: Vec<u8>,
    v2: Vec<u8>,
}

fn root_cargo_toml() -> &'static str {
    "[workspace]\nmembers = []\n\n[patch.crates-io]\nother = { path = \"elsewhere/other\" }\n"
}

fn lock(entries: &[(&str, &str, Option<&str>)]) -> String {
    let mut text = String::from("version = 4\n");
    for (name, version, checksum) in entries {
        text.push_str(&format!(
            "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"
        ));
        if let Some(checksum) = checksum {
            text.push_str(&format!(
                "source = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{checksum}\"\n"
            ));
        }
    }
    text
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir_all(root.join("third_party")).unwrap();
    let servo = "https://github.com/servo/servo";
    let v1 = pack(
        "demo",
        "1.0.0",
        &[
            ("Cargo.toml", &cargo_toml("demo", "1.0.0", servo)),
            (".cargo_vcs_info.json", &vcs_info("1111")),
            ("src/lib.rs", LIB_V1),
            ("src/old.rs", "pub fn old() {}\n"),
            ("logo.bin", "\0binary\0"),
        ],
    );
    let v2 = pack(
        "demo",
        "1.1.0",
        &[
            ("Cargo.toml", &cargo_toml("demo", "1.1.0", servo)),
            (".cargo_vcs_info.json", &vcs_info("2222")),
            ("src/lib.rs", LIB_V2),
            ("logo.bin", "\0binary 2\0"),
        ],
    );
    let gaol = pack(
        "gaol",
        "0.2.1",
        &[(
            "Cargo.toml",
            &cargo_toml("gaol", "0.2.1", "https://github.com/servo/gaol"),
        )],
    );
    let cache = dir
        .path()
        .join("cargo-home/registry/cache/index.crates.io-0000");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("demo-1.0.0.crate"), &v1).unwrap();
    fs::write(cache.join("demo-1.1.0.crate"), &v2).unwrap();
    fs::write(cache.join("gaol-0.2.1.crate"), &gaol).unwrap();
    fs::write(root.join("Cargo.toml"), root_cargo_toml()).unwrap();
    fs::write(
        root.join("Cargo.lock"),
        lock(&[
            ("demo", "1.0.0", Some(&sha256_hex(&v1))),
            ("demo", "1.1.0", Some(&sha256_hex(&v2))),
            ("gaol", "0.2.1", Some(&sha256_hex(&gaol))),
        ]),
    )
    .unwrap();
    let repo = Repo {
        registry: Registry {
            cargo_home: Some(dir.path().join("cargo-home")),
            cache_dir: root.join("target/mercury-servo-patches/crates"),
            network: false,
        },
        cargo: None,
        git: false,
        root,
    };
    Fixture {
        _dir: dir,
        repo,
        v1,
        v2,
    }
}

impl Fixture {
    fn root(&self) -> &Path {
        &self.repo.root
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root().join(path)).unwrap()
    }

    fn write(&self, path: &str, contents: &str) {
        fs::write(self.root().join(path), contents).unwrap();
    }

    /// What `cargo` does to Cargo.lock once `[patch.crates-io]` takes a crate from a path.
    fn lock_takes_from_path(&self, demo_version: &str) {
        let other = if demo_version == "1.0.0" {
            "1.1.0"
        } else {
            "1.0.0"
        };
        let checksum = sha256_hex(if other == "1.0.0" { &self.v1 } else { &self.v2 });
        self.write(
            "Cargo.lock",
            &lock(&[
                ("demo", demo_version, None),
                ("demo", other, Some(&checksum)),
            ]),
        );
    }

    fn vendor_demo(&self) {
        self.repo.vendor("demo@1.0.0", &mut Vec::new()).unwrap();
        self.lock_takes_from_path("1.0.0");
    }

    fn check(&self) -> Result<String, String> {
        let mut out = Vec::new();
        self.repo
            .check(&mut out)
            .map(|()| String::from_utf8(out).unwrap())
            .map_err(|error| error.0)
    }

    fn export(&self) {
        self.repo.export(&mut Vec::new()).unwrap();
    }
}

#[test]
fn vendor_unpacks_the_crate_unchanged_and_records_it() {
    let f = fixture();
    let mut out = Vec::new();
    f.repo.vendor("demo@1.0.0", &mut out).unwrap();

    assert_eq!(f.read("third_party/demo/src/lib.rs"), LIB_V1);
    assert_eq!(
        fs::read(f.root().join("third_party/demo/logo.bin")).unwrap(),
        b"\0binary\0"
    );
    assert!(!f.root().join("third_party/demo/.cargo-ok").exists());

    let manifest = Manifest::parse(&f.read("third_party/vendor.toml")).unwrap();
    let entry = manifest.get("demo").unwrap();
    assert_eq!(entry.version, "1.0.0");
    assert_eq!(entry.checksum, sha256_hex(&f.v1));
    assert_eq!(
        entry.repository.as_deref(),
        Some("https://github.com/servo/servo")
    );
    assert_eq!(entry.commit.as_deref(), Some("1111"));
    assert_eq!(entry.path.as_deref(), Some("components/demo"));

    assert_eq!(
        f.read("Cargo.toml"),
        "[workspace]\nmembers = []\n\n[patch.crates-io]\ndemo = { path = \"third_party/demo\" }\nother = { path = \"elsewhere/other\" }\n"
    );
    let notes = f.read("third_party/demo/MERCURY.md");
    assert!(notes.starts_with("# Patched demo 1.0.0\n"), "{notes}");
    assert!(notes.contains("`MPL-2.0` in its `Cargo.toml`"));
    assert!(
        notes.contains("https://github.com/servo/servo at commit\n`1111`, path `components/demo`")
    );
    let patch = f.read("third_party/patches/demo.patch");
    assert!(
        patch.starts_with("# Mercury's changes to demo 1.0.0"),
        "{patch}"
    );
    assert!(!patch.contains("diff --git"));
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("vendored demo 1.0.0")
    );

    f.lock_takes_from_path("1.0.0");
    assert_eq!(
        f.check().unwrap(),
        "ok  demo 1.0.0 (0 files changed)\n1 vendored crates: no findings\n"
    );
}

#[test]
fn vendor_refuses_crates_outside_the_graph_and_twice() {
    let f = fixture();
    let error = f.repo.vendor("demo@9.0.0", &mut Vec::new()).unwrap_err();
    assert!(error.0.contains("it has 1.0.0, 1.1.0"), "{error}");
    assert!(f.repo.vendor("demo", &mut Vec::new()).is_err());
    f.repo.vendor("demo@1.0.0", &mut Vec::new()).unwrap();
    assert!(
        f.repo
            .vendor("demo@1.0.0", &mut Vec::new())
            .unwrap_err()
            .0
            .contains("already vendored")
    );
}

#[test]
fn vendor_refuses_an_archive_that_does_not_match_the_lock() {
    let f = fixture();
    let cache = f
        ._dir
        .path()
        .join("cargo-home/registry/cache/index.crates.io-0000");
    fs::write(cache.join("demo-1.0.0.crate"), &f.v2).unwrap();
    let error = f.repo.vendor("demo@1.0.0", &mut Vec::new()).unwrap_err();
    assert!(error.0.contains("expected SHA-256"), "{error}");
    assert!(!f.root().join("third_party/demo").exists());
}

#[test]
fn check_follows_edits_markers_and_exports() {
    let f = fixture();
    f.vendor_demo();

    // An edit that is not exported.
    let edited = LIB_V1.replace("    2\n", "    // Mercury: two is twenty-two.\n    22\n");
    f.write("third_party/demo/src/lib.rs", &edited);
    let error = f.check().unwrap_err();
    assert!(error.contains("not the published crate plus"), "{error}");
    assert!(error.contains("src/lib.rs"), "{error}");

    let mut diff = Vec::new();
    f.repo.diff("demo", &mut diff).unwrap();
    let diff = String::from_utf8(diff).unwrap();
    assert!(
        diff.starts_with(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n"
        ),
        "{diff}"
    );
    assert!(
        diff.contains("-    2\n+    // Mercury: two is twenty-two.\n+    22\n"),
        "{diff}"
    );

    f.export();
    assert!(f.read("third_party/patches/demo.patch").contains(&diff));
    assert_eq!(
        f.check().unwrap(),
        "ok  demo 1.0.0 (1 files changed)\n1 vendored crates: no findings\n"
    );

    // Losing the marker fails even once exported.
    f.write(
        "third_party/demo/src/lib.rs",
        &LIB_V1.replace("    2\n", "    22\n"),
    );
    f.export();
    let error = f.check().unwrap_err();
    assert!(
        error.contains("demo/src/lib.rs: changed without a `Mercury:` comment"),
        "{error}"
    );

    // A marker that was already upstream does not count: only added lines do.
    f.write(
        "third_party/demo/src/old.rs",
        "// Mercury: placed by hand\npub fn old() {}\n",
    );
    f.export();
    let error = f.check().unwrap_err();
    assert!(
        error.contains("demo/src/lib.rs: changed without"),
        "{error}"
    );
    assert!(!error.contains("src/old.rs"), "{error}");

    // Deleting a file needs no marker; creating one does.
    f.write("third_party/demo/src/lib.rs", &edited);
    fs::remove_file(f.root().join("third_party/demo/src/old.rs")).unwrap();
    f.write("third_party/demo/src/new.rs", "pub fn new() {}\n");
    f.export();
    let error = f.check().unwrap_err();
    assert_eq!(
        error,
        "demo/src/new.rs: changed without a `Mercury:` comment in the changed lines"
    );
}

#[test]
fn unmarked_files_are_exempt_and_must_be_changed() {
    let f = fixture();
    f.vendor_demo();
    f.write("third_party/demo/Cargo.lock", "# generated\n");
    f.export();
    assert!(
        f.check()
            .unwrap_err()
            .contains("demo/Cargo.lock: changed without")
    );

    let manifest_text = f.read("third_party/vendor.toml");
    let mut manifest = Manifest::parse(&manifest_text).unwrap();
    manifest.crates[0].unmarked = vec!["Cargo.lock".into(), "src/lib.rs".into()];
    f.write("third_party/vendor.toml", &manifest.render());
    let error = f.check().unwrap_err();
    assert_eq!(
        error,
        "third_party/vendor.toml: demo lists src/lib.rs as unmarked, but the patch does not change it"
    );
}

#[test]
fn check_finds_conflict_markers_strays_and_broken_wiring() {
    let f = fixture();
    f.vendor_demo();
    f.write(
        "third_party/demo/src/lib.rs",
        &format!("// Mercury: merged\n<<<<<<< ours\n{LIB_V1}"),
    );
    f.export();
    assert!(
        f.check()
            .unwrap_err()
            .contains("demo/src/lib.rs: unresolved conflict markers")
    );
    f.write("third_party/demo/src/lib.rs", LIB_V1);
    f.export();
    f.check().unwrap();

    fs::create_dir_all(f.root().join("third_party/stray")).unwrap();
    f.write("third_party/patches/gone.patch", "");
    f.write("Cargo.toml", root_cargo_toml());
    f.write(
        "Cargo.lock",
        &lock(&[("demo", "1.0.0", Some(&sha256_hex(&f.v1)))]),
    );
    let error = f.check().unwrap_err();
    for expected in [
        "third_party/stray is not in third_party/vendor.toml",
        "third_party/patches/gone.patch belongs to no vendored crate",
        "Cargo.toml has no [patch.crates-io] entry for demo",
        "Cargo.lock does not take demo 1.0.0 from third_party/demo",
    ] {
        assert!(
            error.contains(expected),
            "{expected:?} missing from:\n{error}"
        );
    }
}

#[test]
fn check_fails_when_the_published_archive_is_not_the_recorded_one() {
    let f = fixture();
    f.vendor_demo();
    let mut manifest = Manifest::parse(&f.read("third_party/vendor.toml")).unwrap();
    manifest.crates[0].checksum = sha256_hex(b"something else");
    f.write("third_party/vendor.toml", &manifest.render());
    assert!(
        f.repo
            .check(&mut Vec::new())
            .unwrap_err()
            .0
            .contains("expected SHA-256")
    );
}

#[test]
fn check_reports_every_problem_even_when_one_crate_or_cargo_fails() {
    let mut f = fixture();
    f.repo.vendor("gaol@0.2.1", &mut Vec::new()).unwrap();
    f.vendor_demo();
    let mut manifest = Manifest::parse(&f.read("third_party/vendor.toml")).unwrap();
    manifest.crates[1].checksum = sha256_hex(b"something else");
    f.write("third_party/vendor.toml", &manifest.render());
    f.write("third_party/demo/src/lib.rs", "// edited\n");
    f.repo.cargo = Some(f.root().join("no-such-cargo"));
    let error = f.check().unwrap_err();
    for expected in [
        "demo: the directory is not the published crate plus",
        "gaol 0.2.1: the cached archive does not have the expected SHA-256",
        "cannot run",
    ] {
        assert!(
            error.contains(expected),
            "{expected:?} missing from:\n{error}"
        );
    }
}

#[test]
fn export_removes_patches_of_crates_no_longer_vendored() {
    let f = fixture();
    f.vendor_demo();
    f.write("third_party/patches/old-crate.patch", "");
    let mut out = Vec::new();
    f.repo.export(&mut out).unwrap();
    assert!(
        !f.root()
            .join("third_party/patches/old-crate.patch")
            .exists()
    );
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("demo.patch (0 files changed)"), "{out}");
    assert!(out.contains("removed"), "{out}");
}

#[test]
fn upgrade_merges_mercury_changes_and_reports_each_file() {
    let f = fixture();
    f.vendor_demo();
    // A change upstream leaves alone, one upstream deletes the file under, and a new file.
    f.write(
        "third_party/demo/src/lib.rs",
        &LIB_V1.replace("    1\n", "    // Mercury: one is eleven.\n    11\n"),
    );
    f.write(
        "third_party/demo/src/old.rs",
        "// Mercury: kept\npub fn old() {}\n",
    );
    f.write("third_party/demo/src/extra.rs", "// Mercury: ours\n");
    f.export();
    f.check().unwrap();

    let mut out = Vec::new();
    let report = f
        .repo
        .upgrade(UpgradeTarget::Crate("demo@1.1.0".into()), &mut out)
        .unwrap();
    let out = String::from_utf8(out).unwrap();
    assert_eq!(report.conflicts(), 0, "{out}");
    let files = &report.crates[0].files;
    assert_eq!(
        files,
        &[
            ("src/extra.rs".to_owned(), Outcome::Applied),
            ("src/lib.rs".to_owned(), Outcome::Applied),
            ("src/old.rs".to_owned(), Outcome::Gone),
        ]
    );
    assert!(out.contains("demo 1.0.0 -> 1.1.0\n"), "{out}");
    assert!(out.contains("  src/old.rs: GONE"), "{out}");

    let lib = f.read("third_party/demo/src/lib.rs");
    assert!(
        lib.contains("    // Mercury: one is eleven.\n    11\n"),
        "{lib}"
    );
    assert!(lib.contains("3 // upstream"), "{lib}");
    assert!(!f.root().join("third_party/demo/src/old.rs").exists());
    assert_eq!(
        f.read("third_party/demo/src/extra.rs"),
        "// Mercury: ours\n"
    );
    assert_eq!(
        fs::read(f.root().join("third_party/demo/logo.bin")).unwrap(),
        b"\0binary 2\0"
    );
    assert!(f.root().join("third_party/demo/MERCURY.md").exists());

    let manifest = Manifest::parse(&f.read("third_party/vendor.toml")).unwrap();
    let entry = manifest.get("demo").unwrap();
    assert_eq!(
        (entry.version.as_str(), entry.commit.as_deref()),
        ("1.1.0", Some("2222"))
    );
    assert_eq!(entry.checksum, sha256_hex(&f.v2));

    // The old patch no longer describes the tree until it is exported again.
    f.lock_takes_from_path("1.1.0");
    assert!(
        f.check()
            .unwrap_err()
            .contains("run `mercury-servo-patches export`")
    );
    f.export();
    f.check().unwrap();
}

#[test]
fn upgrade_writes_conflict_markers_where_both_sides_changed() {
    let f = fixture();
    f.vendor_demo();
    f.write(
        "third_party/demo/src/lib.rs",
        &LIB_V1.replace(
            "    3\n",
            "    // Mercury: three is thirty-three.\n    33\n",
        ),
    );
    fs::remove_file(f.root().join("third_party/demo/src/old.rs")).unwrap();
    f.export();
    let mut out = Vec::new();
    let report = f
        .repo
        .upgrade(UpgradeTarget::Crate("demo@1.1.0".into()), &mut out)
        .unwrap();
    assert_eq!(report.conflicts(), 1);
    assert_eq!(
        report.crates[0].files,
        [
            ("src/lib.rs".to_owned(), Outcome::Conflict),
            ("src/old.rs".to_owned(), Outcome::Kept),
        ]
    );
    let lib = f.read("third_party/demo/src/lib.rs");
    assert!(lib.contains("<<<<<<<") && lib.contains(">>>>>>>"), "{lib}");
    assert!(lib.contains("33") && lib.contains("3 // upstream"), "{lib}");

    f.lock_takes_from_path("1.1.0");
    f.export();
    assert!(
        f.check()
            .unwrap_err()
            .contains("unresolved conflict markers")
    );
}

#[test]
fn upgrade_refuses_unexported_changes_and_servo_picks_servo_crates() {
    let f = fixture();
    f.repo.vendor("gaol@0.2.1", &mut Vec::new()).unwrap();
    f.vendor_demo();
    f.write("third_party/demo/src/lib.rs", "// Mercury: not exported\n");
    let error = f
        .repo
        .upgrade(UpgradeTarget::Servo("1.1.0".into()), &mut Vec::new())
        .unwrap_err();
    assert!(error.0.contains("not exported"), "{error}");
    assert_eq!(
        f.read("third_party/demo/src/lib.rs"),
        "// Mercury: not exported\n"
    );

    f.export();
    let report = f
        .repo
        .upgrade(UpgradeTarget::Servo("1.1.0".into()), &mut Vec::new())
        .unwrap();
    let moved: Vec<_> = report
        .crates
        .iter()
        .map(|upgraded| upgraded.name.as_str())
        .collect();
    assert_eq!(moved, ["demo"]);
    let manifest = Manifest::parse(&f.read("third_party/vendor.toml")).unwrap();
    assert_eq!(manifest.get("gaol").unwrap().version, "0.2.1");
}

#[test]
fn workspace_problems_catch_members_and_path_dependencies_in_third_party() {
    let root = Path::new("/repo");
    let metadata = serde_json::json!({
        "packages": [
            {
                "name": "talaria-servo",
                "manifest_path": "/repo/crates/talaria-servo/Cargo.toml",
                "dependencies": [
                    { "name": "servo", "path": null },
                    { "name": "talaria-core", "path": "/repo/crates/talaria-core" }
                ]
            }
        ]
    });
    assert!(workspace_problems(&metadata, root).is_empty());
    let metadata = serde_json::json!({
        "packages": [
            {
                "name": "talaria-servo",
                "manifest_path": "/repo/crates/talaria-servo/Cargo.toml",
                "dependencies": [ { "name": "servo-canvas", "path": "/repo/third_party/servo-canvas" } ]
            },
            {
                "name": "gaol",
                "manifest_path": "/repo/third_party/gaol/Cargo.toml",
                "dependencies": []
            }
        ]
    });
    let problems = workspace_problems(&metadata, root);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("depends on servo-canvas by path"));
    assert!(problems[1].contains("gaol in third_party/ is a workspace member"));
}
