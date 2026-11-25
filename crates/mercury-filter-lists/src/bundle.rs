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

/// The repository directory that holds the licence texts, relative to the repository root.
pub const LICENCES_DIR: &str = "LICENSES";
/// The notice written beside the installed lists.
pub const NOTICE_FILE: &str = "NOTICE.md";

/// The notice for the lists of a manifest: each list's name, credit, licence and source.
pub fn notice(manifest: &crate::manifest::Manifest) -> String {
    let mut text = String::from(
        "# Filter lists\n\n\
         These lists are data read by Mercury Browser's ad and tracker blocker. They are not part \
         of Mercury Browser's code: each is redistributed unmodified under its own licence, whose \
         text is in `LICENSES/` beside this file, and newer versions are fetched from the source \
         given.\n\n",
    );
    for list in &manifest.lists {
        text.push_str(&format!(
            "- **{}** (`{}`): by {}; licence {} (`LICENSES/{}.txt`); source {}\n",
            list.name, list.file, list.credit, list.licence, list.licence, list.source_url
        ));
    }
    text
}

/// The licences the lists of a manifest are under, each once, in order.
fn licences_of(manifest: &crate::manifest::Manifest) -> Vec<&str> {
    let mut licences: Vec<&str> = Vec::new();
    for list in &manifest.lists {
        if !licences.contains(&list.licence.as_str()) {
            licences.push(&list.licence);
        }
    }
    licences
}

/// Checks `source`, then replaces `destination` with a directory holding exactly the manifest,
/// the lists it names, `NOTICE.md` and `LICENSES/<licence>.txt` for each licence, copied from
/// `licences`. Nothing is written if a check fails or a licence text is missing. Returns how
/// many lists were installed.
pub fn install(source: &Path, licences: &Path, destination: &Path) -> Result<usize, String> {
    let files = check_dir(source)?;
    let manifest = read_manifest(source).map_err(|error| error.to_string())?;
    let needed = licences_of(&manifest);
    for licence in &needed {
        let text = licences.join(format!("{licence}.txt"));
        if !text.is_file() {
            return Err(format!(
                "no licence text for {licence} ({} is missing)",
                text.display()
            ));
        }
    }
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
        std::fs::create_dir(staging.join(LICENCES_DIR))
            .map_err(|error| format!("cannot create the licence directory: {error}"))?;
        for licence in &needed {
            let name = format!("{licence}.txt");
            std::fs::copy(licences.join(&name), staging.join(LICENCES_DIR).join(&name))
                .map_err(|error| format!("cannot copy {name}: {error}"))?;
        }
        std::fs::write(staging.join(NOTICE_FILE), notice(&manifest))
            .map_err(|error| format!("cannot write {NOTICE_FILE}: {error}"))?;
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

    fn repository_licences() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../LICENSES")
    }

    /// A licence directory with the test lists' licence.
    fn licences(directory: &Path) -> PathBuf {
        let licences = directory.join("LICENSES");
        std::fs::create_dir_all(&licences).unwrap();
        std::fs::write(licences.join("CC0-1.0.txt"), "CC0 text").unwrap();
        licences
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

        assert_eq!(
            install(&source, &licences(directory.path()), &destination),
            Ok(2)
        );

        assert_eq!(
            names(&destination),
            [
                "LICENSES",
                "NOTICE.md",
                "ads.txt",
                "manifest.json",
                "more.txt"
            ]
        );
        assert_eq!(names(&destination.join("LICENSES")), ["CC0-1.0.txt"]);
        assert_eq!(names(&directory.path().join("out")), ["filter-lists"]);
    }

    #[test]
    fn a_failed_check_installs_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        bundle(&source, &[("ads", "||ads.example^\n")]);
        std::fs::write(source.join("ads.txt"), "tampered").unwrap();
        let destination = directory.path().join("out/filter-lists");

        assert!(install(&source, &licences(directory.path()), &destination).is_err());

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
            install(&source, &licences(directory.path()), &destination)
                .unwrap_err()
                .contains("not a directory")
        );
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "a file");
    }

    #[test]
    fn a_missing_licence_text_installs_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        bundle(&source, &[("ads", "||ads.example^\n")]);
        let empty = directory.path().join("no-licences");
        std::fs::create_dir_all(&empty).unwrap();
        let destination = directory.path().join("out/filter-lists");

        let problem = install(&source, &empty, &destination).unwrap_err();

        assert!(problem.contains("CC0-1.0"), "{problem}");
        assert!(!destination.exists());
    }

    #[test]
    fn the_repository_lists_install_with_their_licences_and_notice() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("filter-lists");

        assert_eq!(
            install(&repository_lists(), &repository_licences(), &destination),
            Ok(5)
        );

        assert_eq!(
            names(&destination.join("LICENSES")),
            ["CC-BY-SA-3.0.txt", "MPL-2.0.txt"]
        );
        let notice = std::fs::read_to_string(destination.join(NOTICE_FILE)).unwrap();
        for expected in [
            "EasyList",
            "The EasyList authors (https://easylist.to/)",
            "CC-BY-SA-3.0",
            "https://easylist.to/easylist/easylist.txt",
            "Brave Specific",
            "MPL-2.0",
            "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-specific.txt",
            "LICENSES/CC-BY-SA-3.0.txt",
        ] {
            assert!(
                notice.contains(expected),
                "{expected} missing from:\n{notice}"
            );
        }
    }

    #[test]
    fn the_notice_names_every_list_its_credit_licence_and_source() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = bundle(directory.path(), &[("ads", "||a^\n"), ("more", "||b^\n")]);

        let text = notice(&manifest);

        for list in &manifest.lists {
            assert!(text.contains(&list.name));
            assert!(text.contains(list.source_url.as_str()));
        }
        assert!(text.contains("Test lists"));
        assert!(text.contains("LICENSES/CC0-1.0.txt"));
    }

    /// Minor 5 of the review: nothing compiles the lists into a program any more.
    #[test]
    fn talaria_servo_compiles_no_list_into_the_program() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../talaria-servo");
        assert!(!crate_dir.join("build.rs").exists());
        for entry in std::fs::read_dir(crate_dir.join("src")).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            // The one file that is compiled in is Mercury's HSTS preload list (`resources.rs`), which
            // is not a filter list.
            for line in text.lines().filter(|line| line.contains("include_bytes!")) {
                assert!(line.contains("hsts_preload"), "{}: {line}", path.display());
            }
            assert!(!text.contains("adblock-engine.dat"), "{}", path.display());
        }
    }
}
