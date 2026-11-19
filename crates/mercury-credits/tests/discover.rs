// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Finding licence files in a package.
use mercury_credits::discover::{self, FileKind, Found};
use std::path::Path;

fn write(root: &Path, relative: &str, text: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn paths(files: &[Found]) -> Vec<&str> {
    let mut paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    paths.sort_unstable();
    paths
}

#[test]
fn files_named_like_a_licence_are_found_in_the_package_root() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "LICENSE",
        "LICENSE-MIT",
        "license-apache",
        "LICENCE.md",
        "COPYING",
        "UNLICENSE",
        "Copyright.txt",
        "NOTICE",
        "NOTICES.md",
        "PATENTS",
    ] {
        write(dir.path(), name, format!("text of {name}").as_bytes());
    }
    for name in [
        "README.md",
        "Cargo.toml",
        "lib.rs",
        "license.rs",
        "licenses.py",
        "Cargo.lock",
    ] {
        write(dir.path(), name, b"not a licence");
    }
    let found = discover::discover(dir.path(), &[]);
    assert_eq!(
        paths(&found.files),
        [
            "COPYING",
            "Copyright.txt",
            "LICENCE.md",
            "LICENSE",
            "LICENSE-MIT",
            "NOTICE",
            "NOTICES.md",
            "PATENTS",
            "UNLICENSE",
            "license-apache"
        ]
    );
    let kind = |name: &str| {
        found
            .files
            .iter()
            .find(|file| file.path == name)
            .unwrap()
            .kind
    };
    assert_eq!(kind("NOTICE"), FileKind::Notice);
    assert_eq!(kind("NOTICES.md"), FileKind::Notice);
    assert_eq!(kind("PATENTS"), FileKind::Patents);
    assert_eq!(kind("LICENSE"), FileKind::License);
    assert_eq!(kind("COPYING"), FileKind::License);
    assert!(found.missing_extras.is_empty() && found.skipped.is_empty());
}

#[test]
fn every_file_in_a_licenses_directory_is_a_licence_text() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "LICENSES/MIT.txt", b"mit");
    write(
        dir.path(),
        "LICENSES/LicenseRef-Slint-Royalty-free-2.0.md",
        b"slint",
    );
    write(dir.path(), "LICENSES/NOTICE", b"a notice in the directory");
    write(
        dir.path(),
        "LICENSES/nested/Deep.txt",
        b"not read: only the directory's own files",
    );
    let found = discover::discover(dir.path(), &[]);
    assert_eq!(
        paths(&found.files),
        [
            "LICENSES/LicenseRef-Slint-Royalty-free-2.0.md",
            "LICENSES/MIT.txt",
            "LICENSES/NOTICE"
        ]
    );
    let notice = found
        .files
        .iter()
        .find(|file| file.path == "LICENSES/NOTICE")
        .unwrap();
    assert_eq!(notice.kind, FileKind::Notice);
}

#[test]
fn a_lowercase_licenses_directory_is_found_too_and_not_twice() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "licenses/Apache-2.0.txt", b"apache");
    let found = discover::discover(dir.path(), &[]);
    let lowercase: Vec<String> = found
        .files
        .iter()
        .map(|file| file.path.to_lowercase())
        .collect();
    assert_eq!(
        lowercase,
        ["licenses/apache-2.0.txt"],
        "the name may be spelt either way on a case-insensitive disk"
    );
    // On a case-insensitive file system `LICENSES` is the same directory as `licenses`; the
    // text must not be listed twice either way.
    write(dir.path(), "LICENSES/MIT.txt", b"mit");
    let found = discover::discover(dir.path(), &[]);
    let mut seen = paths(&found.files);
    seen.dedup();
    assert_eq!(seen.len(), found.files.len());
}

#[test]
fn extra_paths_are_read_relative_to_the_package() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "LICENSE", b"the crate");
    write(dir.path(), "vendor/lz4/LICENSE", b"lz4 text");
    write(dir.path(), "vendor/odd/COPYING.txt", b"odd");
    write(
        dir.path(),
        "vendor/odd/readme",
        b"a name that does not look like a licence",
    );
    let extras = [
        "vendor/lz4/LICENSE".to_owned(),
        "vendor/odd/readme".to_owned(),
        "LICENSE".to_owned(),
    ];
    let found = discover::discover(dir.path(), &extras);
    assert_eq!(
        paths(&found.files),
        ["LICENSE", "vendor/lz4/LICENSE", "vendor/odd/readme"]
    );
    let lz4 = found
        .files
        .iter()
        .find(|file| file.path == "vendor/lz4/LICENSE")
        .unwrap();
    assert!(lz4.extra);
    assert_eq!(lz4.text, "lz4 text");
    let readme = found
        .files
        .iter()
        .find(|file| file.path == "vendor/odd/readme")
        .unwrap();
    assert_eq!(
        readme.kind,
        FileKind::License,
        "a named extra is a licence text whatever its name"
    );
    let root = found
        .files
        .iter()
        .find(|file| file.path == "LICENSE")
        .unwrap();
    assert!(
        !root.extra,
        "a file found in the usual place stays what it was"
    );
    assert!(found.missing_extras.is_empty());
}

#[test]
fn a_missing_extra_is_reported_not_ignored() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a/LICENSE", b"x");
    std::fs::create_dir_all(dir.path().join("a/dir")).unwrap();
    let extras = [
        "a/LICENSE".to_owned(),
        "a/NOPE".to_owned(),
        "a/dir".to_owned(),
        "../outside".to_owned(),
        "/etc/hosts".to_owned(),
    ];
    let found = discover::discover(dir.path(), &extras);
    assert_eq!(paths(&found.files), ["a/LICENSE"]);
    assert_eq!(
        found.missing_extras,
        ["a/NOPE", "a/dir", "../outside", "/etc/hosts"]
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_that_leaves_the_package_is_not_followed() {
    let outer = tempfile::tempdir().unwrap();
    let package = outer.path().join("pkg");
    std::fs::create_dir_all(&package).unwrap();
    write(outer.path(), "secret.txt", b"outside the package");
    write(outer.path(), "outside-dir/MIT.txt", b"outside directory");
    std::os::unix::fs::symlink(outer.path().join("secret.txt"), package.join("LICENSE")).unwrap();
    std::os::unix::fs::symlink("../secret.txt", package.join("COPYING")).unwrap();
    std::os::unix::fs::symlink(outer.path().join("outside-dir"), package.join("licenses")).unwrap();
    write(&package, "inside.txt", b"inside the package");
    std::os::unix::fs::symlink("inside.txt", package.join("LICENSE-MIT")).unwrap();
    let extras = ["LICENSE".to_owned()];
    let found = discover::discover(&package, &extras);
    assert_eq!(
        paths(&found.files),
        ["LICENSE-MIT"],
        "only the link that stays inside is read"
    );
    assert_eq!(found.files[0].text, "inside the package");
    assert_eq!(
        found.missing_extras,
        ["LICENSE"],
        "an extra that is a link out is missing"
    );
}

#[test]
fn bytes_that_are_not_utf8_are_read_lossily_and_binary_files_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    // Latin-1: "Copyright © 2003" with the © as the single byte 0xA9.
    write(
        dir.path(),
        "LICENSE",
        b"Copyright \xa9 2003 Jos\xe9\nPermission is granted",
    );
    write(dir.path(), "COPYING", b"binary\0data");
    write(dir.path(), "NOTICE", b"  \n\n");
    let found = discover::discover(dir.path(), &[]);
    assert_eq!(paths(&found.files), ["LICENSE"]);
    assert!(found.files[0].text.starts_with("Copyright "));
    assert!(found.files[0].text.contains("2003 Jos"));
    assert!(found.files[0].text.contains("Permission is granted"));
    assert_eq!(found.skipped.len(), 1);
    assert!(
        found.skipped[0].starts_with("COPYING"),
        "{:?}",
        found.skipped
    );
}

#[test]
fn a_package_directory_that_does_not_exist_reports_its_extras_missing() {
    let found = discover::discover(Path::new("/definitely/not/here"), &["x/LICENSE".to_owned()]);
    assert!(found.files.is_empty());
    assert_eq!(found.missing_extras, ["x/LICENSE"]);
}

#[test]
fn names_are_classified_by_prefix_in_any_case() {
    assert_eq!(discover::kind_of_name("LICENSE"), Some(FileKind::License));
    assert_eq!(
        discover::kind_of_name("Licence-APACHE"),
        Some(FileKind::License)
    );
    assert_eq!(
        discover::kind_of_name("copying.lesser"),
        Some(FileKind::License)
    );
    assert_eq!(discover::kind_of_name("UNLICENSE"), Some(FileKind::License));
    assert_eq!(discover::kind_of_name("COPYRIGHT"), Some(FileKind::License));
    assert_eq!(discover::kind_of_name("notice.txt"), Some(FileKind::Notice));
    assert_eq!(discover::kind_of_name("PATENTS"), Some(FileKind::Patents));
    assert_eq!(discover::kind_of_name("README"), None);
    assert_eq!(discover::kind_of_name("license.rs"), None);
    assert_eq!(discover::kind_of_name("not-a-license"), None);
}
