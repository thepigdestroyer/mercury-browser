// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Where `mercury-import` puts its private copies of another browser's databases while it
//! reads them: a directory of this profile's own, owner-only (0700), emptied at every start.
//!
//! - The default profile's copies go under Mercury's cache directory
//!   (`~/Library/Caches/<bundle id>/import-copies/Default` on macOS), which backups skip.
//! - Any other profile (a `--profile-dir`, or the temporary profile of an automated run) keeps
//!   them inside the profile, in `import-copies`; a temporary profile's copies go with it.
//!
//! Each read removes its copy when it ends. A read that is still running when the browser quits
//! is told to stop and is not waited for; whatever it leaves, and whatever a crash leaves, is
//! removed at the next start, while this run holds the profile's lock, so no other run can be
//! using the directory.

use std::io;
use std::path::{Path, PathBuf};

const AREA_NAME: &str = "import-copies";

/// Mercury's cache directory, as the media buffers use it on macOS; `None` if the platform
/// names none.
pub(crate) fn mercury_cache_dir() -> Option<PathBuf> {
    let platform = dirs::cache_dir()?;
    #[cfg(target_os = "macos")]
    return Some(crate::media_isolation::mercury_cache_dir(Some(platform)));
    #[cfg(not(target_os = "macos"))]
    return Some(platform.join(crate::profile::APP_DIR_NAME));
}

/// The copy area of the profile at `profile_root`; `default_profile` when it is the user's
/// default profile, whose copies go under `cache_dir` (Mercury's cache directory) if known.
pub(crate) fn area_for(
    profile_root: &Path,
    default_profile: bool,
    cache_dir: Option<&Path>,
) -> PathBuf {
    match (default_profile, cache_dir) {
        (true, Some(cache_dir)) => cache_dir.join(AREA_NAME).join("Default"),
        _ => profile_root.join(AREA_NAME),
    }
}

/// Creates the copy area if needed, owner-only, and removes everything in it (what an earlier
/// run that was killed or quit while reading left behind). A symbolic link where the area
/// belongs is removed, never followed.
pub(crate) fn prepare(area: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(area) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            for entry in std::fs::read_dir(area)? {
                let path = entry?.path();
                let kind = std::fs::symlink_metadata(&path)?.file_type();
                if kind.is_dir() {
                    std::fs::remove_dir_all(&path)?;
                } else {
                    std::fs::remove_file(&path)?;
                }
            }
        }
        Ok(_) => {
            std::fs::remove_file(area)?;
            create(area)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => create(area)?,
        Err(error) => return Err(error),
    }
    make_private(area)
}

fn create(area: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(area)
}

#[cfg(unix)]
fn make_private(area: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(area, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn make_private(_area: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_profile_copies_under_the_cache_and_others_inside_the_profile() {
        let cache = Path::new("/cache/org.example.mercury");
        assert_eq!(
            area_for(Path::new("/profiles/a"), true, Some(cache)),
            cache.join("import-copies/Default")
        );
        assert_eq!(
            area_for(Path::new("/profiles/a"), false, Some(cache)),
            Path::new("/profiles/a/import-copies")
        );
        assert_eq!(
            area_for(Path::new("/profiles/a"), true, None),
            Path::new("/profiles/a/import-copies")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_area_is_created_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let area = root.path().join("cache/import-copies/Default");

        prepare(&area).unwrap();

        let mode = std::fs::metadata(&area).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn what_a_run_that_quit_while_reading_left_is_removed_at_the_next_start() {
        let profile = tempfile::tempdir().unwrap();
        let area = area_for(profile.path(), false, None);
        prepare(&area).unwrap();
        // A read still copying when the browser quit (it is not waited for).
        let leftover = area.join("mercury-import-AbC123");
        std::fs::create_dir(&leftover).unwrap();
        std::fs::write(
            leftover.join("snapshot.sqlite"),
            b"a copy of someone's history",
        )
        .unwrap();
        std::fs::write(area.join("stray"), b"x").unwrap();
        let note = crate::import_ui::marker::Unfinished {
            browser: "Firefox".into(),
            name: "Default".into(),
            path: "/fixtures/Firefox/a".into(),
            bookmarks: true,
            history: true,
        };
        crate::import_ui::marker::add(profile.path(), &note).unwrap();

        prepare(&area).unwrap();

        assert_eq!(std::fs::read_dir(&area).unwrap().count(), 0);
        assert_eq!(
            crate::import_ui::marker::load(profile.path()),
            vec![note],
            "the note of the unfinished import is kept"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_the_area_is_removed_and_its_target_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = root.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("keep"), b"not ours").unwrap();
        let area = root.path().join("import-copies");
        std::os::unix::fs::symlink(&elsewhere, &area).unwrap();

        prepare(&area).unwrap();

        assert!(std::fs::symlink_metadata(&area).unwrap().is_dir());
        assert!(elsewhere.join("keep").exists());
    }
}
