// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A crate's files in memory, read from or written to a directory.

use crate::error::{Context, Result};
use crate::fail;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Mercury's notes in a vendored directory; never part of the crate or its patch.
pub const NOTES_FILE: &str = "MERCURY.md";

/// Build output that may appear at the top of a vendored directory and is not
/// part of it (`cargo test --manifest-path third_party/gaol/Cargo.toml` without
/// `--target-dir`, for example).
const SKIPPED_DIRECTORY: &str = "target";

/// Files by path relative to the crate root, with `/` separators.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tree {
    pub files: BTreeMap<String, Vec<u8>>,
    /// The paths whose mode has an execute bit.
    pub executable: BTreeSet<String>,
}

/// Fails unless `path` is a plain relative path: no empty, `.` or `..`
/// component, no leading `/` and no backslash.
pub fn check_relative_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(fail!("unsafe or empty path {path:?}"));
    }
    Ok(())
}

impl Tree {
    /// Reads every file under `dir`, except [`NOTES_FILE`] and a top-level
    /// `target/` directory. A symbolic link is an error: the crates.io archive
    /// format cannot contain one, so it cannot be part of a vendored crate.
    pub fn read(dir: &Path) -> Result<Tree> {
        let mut tree = Tree::default();
        read_into(dir, "", &mut tree)?;
        Ok(tree)
    }

    /// Writes every file into `dir`, creating directories as needed. Existing
    /// files are overwritten; nothing is removed.
    pub fn write(&self, dir: &Path) -> Result<()> {
        for (path, data) in &self.files {
            check_relative_path(path)?;
            let target = dir.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).context(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&target, data).context(|| format!("writing {}", target.display()))?;
            set_executable(&target, self.executable.contains(path))?;
        }
        Ok(())
    }
}

fn read_into(dir: &Path, prefix: &str, tree: &mut Tree) -> Result<()> {
    let mut entries = fs::read_dir(dir)
        .context(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .context(|| format!("reading {}", dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| fail!("{}: file name {name:?} is not UTF-8", dir.display()))?;
        let path = format!("{prefix}{name}");
        if prefix.is_empty() && (name == NOTES_FILE || name == SKIPPED_DIRECTORY) {
            continue;
        }
        let file_type = entry
            .file_type()
            .context(|| format!("reading {}", entry.path().display()))?;
        if file_type.is_dir() {
            read_into(&entry.path(), &format!("{path}/"), tree)?;
        } else if file_type.is_file() {
            let data =
                fs::read(entry.path()).context(|| format!("reading {}", entry.path().display()))?;
            if is_executable(&entry.path())? {
                tree.executable.insert(path.clone());
            }
            tree.files.insert(path, data);
        } else {
            return Err(fail!(
                "{} is not a regular file or directory",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)
        .context(|| format!("reading {}", path.display()))?
        .permissions()
        .mode();
    Ok(mode & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> Result<bool> {
    Ok(false)
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .context(|| format!("setting the mode of {}", path.display()))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_must_stay_inside() {
        for bad in ["", "/etc/passwd", "a/../b", "a//b", "./a", "a\\b", "a/"] {
            assert!(check_relative_path(bad).is_err(), "{bad:?} accepted");
        }
        check_relative_path("src/lib.rs").unwrap();
        check_relative_path(".cargo_vcs_info.json").unwrap();
    }

    #[test]
    fn write_then_read_round_trips_without_notes_and_build_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = Tree::default();
        tree.files
            .insert("src/lib.rs".into(), b"fn f() {}\n".to_vec());
        tree.files.insert("run.sh".into(), b"#!/bin/sh\n".to_vec());
        tree.executable.insert("run.sh".into());
        tree.write(dir.path()).unwrap();
        fs::write(dir.path().join(NOTES_FILE), "# notes\n").unwrap();
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        fs::write(dir.path().join("target/debug/x"), "x").unwrap();
        // Only a top-level `target` is build output.
        fs::create_dir_all(dir.path().join("src/target")).unwrap();
        fs::write(dir.path().join("src/target/keep.rs"), "").unwrap();
        tree.files.insert("src/target/keep.rs".into(), Vec::new());

        let read = Tree::read(dir.path()).unwrap();
        if cfg!(unix) {
            assert_eq!(read, tree);
        } else {
            assert_eq!(read.files, tree.files);
        }
    }
}
