// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Checking and installing the bundled lists for packaging (the `filter-lists` tool): the
//! manifest is read with the same rules as at run time, every list must match its SHA-256, and
//! an installation holds exactly the manifest and the lists it names.

use std::path::{Path, PathBuf};

use crate::engine::read_manifest;
use crate::manifest::MANIFEST_FILE;
use crate::sha256_hex;

/// The repository directory that holds the bundled lists, relative to the repository root.
pub const SOURCE_DIR: &str = "crates/talaria-servo/resources/adblock";

/// Checks the manifest in `directory` and every list it names; returns the list files.
pub fn check_dir(directory: &Path) -> Result<Vec<String>, String> {
    let manifest = read_manifest(directory).map_err(|error| error.to_string())?;
    let mut files = Vec::with_capacity(manifest.lists.len());
    for list in &manifest.lists {
        let path = directory.join(&list.file);
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let digest = sha256_hex(&bytes);
        if digest != list.sha256 {
            return Err(format!(
                "{}: SHA-256 {digest} does not match the manifest's {}",
                list.file, list.sha256
            ));
        }
        files.push(list.file.clone());
    }
    Ok(files)
}

/// Checks `source`, then replaces `destination` with a directory holding exactly the manifest
/// and the lists it names. Nothing is written if the check fails. Returns how many lists were
/// installed.
pub fn install(source: &Path, destination: &Path) -> Result<usize, String> {
    let files = check_dir(source)?;
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if let Ok(metadata) = std::fs::symlink_metadata(destination)
        && !metadata.is_dir()
    {
        return Err(format!(
            "{} exists and is not a directory",
            destination.display()
        ));
    }
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let staging = staging_path(parent, destination);
    let result = (|| {
        std::fs::create_dir(&staging)
            .map_err(|error| format!("cannot create {}: {error}", staging.display()))?;
        for name in std::iter::once(MANIFEST_FILE).chain(files.iter().map(String::as_str)) {
            std::fs::copy(source.join(name), staging.join(name))
                .map_err(|error| format!("cannot copy {name}: {error}"))?;
        }
        check_dir(&staging)?;
        if destination.exists() {
            std::fs::remove_dir_all(destination)
                .map_err(|error| format!("cannot replace {}: {error}", destination.display()))?;
        }
        std::fs::rename(&staging, destination)
            .map_err(|error| format!("cannot move into {}: {error}", destination.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result.map(|()| files.len())
}

fn staging_path(parent: &Path, destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .map_or_else(|| "filter-lists".into(), |name| name.to_string_lossy());
    parent.join(format!(".{name}.staging-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::tests::bundle;

    fn repository_lists() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../talaria-servo/resources/adblock")
    }

    fn names(directory: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn the_repository_lists_match_their_manifest() {
        let files = check_dir(&repository_lists()).expect("valid");
        assert_eq!(files.len(), 5);
        assert!(
            !files.iter().any(|file| file.contains("ublock")),
            "no GPL-only list"
        );
    }

    #[test]
    fn a_damaged_or_missing_list_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        bundle(directory.path(), &[("ads", "||ads.example^\n")]);
        std::fs::write(
            directory.path().join("ads.txt"),
            "||ads.example^\n||extra^\n",
        )
        .unwrap();
        assert!(check_dir(directory.path()).unwrap_err().contains("SHA-256"));

        std::fs::remove_file(directory.path().join("ads.txt")).unwrap();
        assert!(
            check_dir(directory.path())
                .unwrap_err()
                .contains("cannot read")
        );

        std::fs::remove_file(directory.path().join(MANIFEST_FILE)).unwrap();
        assert!(check_dir(directory.path()).is_err());
    }

    #[test]
    fn an_installation_holds_exactly_the_manifest_and_its_lists() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        bundle(
            &source,
            &[("ads", "||ads.example^\n"), ("more", "||more.example^\n")],
        );
        std::fs::write(source.join("README.md"), "not shipped").unwrap();
        let destination = directory.path().join("out/filter-lists");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("stale.txt"), "old").unwrap();

        assert_eq!(install(&source, &destination), Ok(2));

        assert_eq!(
            names(&destination),
            ["ads.txt", "manifest.json", "more.txt"]
        );
        assert_eq!(names(&directory.path().join("out")), ["filter-lists"]);
    }

    #[test]
    fn a_failed_check_installs_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        bundle(&source, &[("ads", "||ads.example^\n")]);
        std::fs::write(source.join("ads.txt"), "tampered").unwrap();
        let destination = directory.path().join("out/filter-lists");

        assert!(install(&source, &destination).is_err());

        assert!(!directory.path().join("out").exists());
    }

    #[test]
    fn a_destination_that_is_a_file_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        bundle(&source, &[("ads", "||ads.example^\n")]);
        let destination = directory.path().join("filter-lists");
        std::fs::write(&destination, "a file").unwrap();

        assert!(
            install(&source, &destination)
                .unwrap_err()
                .contains("not a directory")
        );
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "a file");
    }
}
