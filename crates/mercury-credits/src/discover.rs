// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Finding the licence files of a package: in its root, in `LICENSES/` or `licenses/`, and at the
//! paths `credits.toml` names for native code deeper in the package.
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

/// Names that start a licence file, in any case and with any suffix.
const PREFIXES: &[&str] = &[
    "LICENSE",
    "LICENCE",
    "COPYING",
    "UNLICENSE",
    "NOTICE",
    "COPYRIGHT",
    "PATENTS",
];

/// Source files that happen to start like a licence file (`license.rs`).
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "c", "h", "cc", "cpp", "py", "sh", "js", "toml", "lock",
];

/// Texts larger than this are not read (a licence is a few kilobytes).
const MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileKind {
    License,
    Notice,
    Patents,
}

/// A licence file read from a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Relative to the package root, with `/`.
    pub path: String,
    pub kind: FileKind,
    pub text: String,
    /// Named by an `[[extra]]` entry rather than found in the usual places.
    pub extra: bool,
}

#[derive(Debug, Default)]
pub struct Discovery {
    pub files: Vec<Found>,
    /// `[[extra]]` paths that are not a regular file inside the package.
    pub missing_extras: Vec<String>,
    /// Things found but not read (too large, not text), for a warning.
    pub skipped: Vec<String>,
}

/// The kind of file a name stands for, if it is a licence file name.
pub fn kind_of_name(name: &str) -> Option<FileKind> {
    let upper = name.to_ascii_uppercase();
    if !PREFIXES.iter().any(|prefix| upper.starts_with(prefix)) {
        return None;
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if extension.is_some_and(|extension| SOURCE_EXTENSIONS.contains(&extension.as_str())) {
        return None;
    }
    Some(if upper.starts_with("NOTICE") {
        FileKind::Notice
    } else if upper.starts_with("PATENTS") {
        FileKind::Patents
    } else {
        FileKind::License
    })
}

/// A regular file at `path`, reached without leaving `package_root` (a symlink that points
/// outside the package is not followed). `package_root` must be canonical.
fn regular_file_inside(path: &Path, package_root: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.is_file() {
        // A file in a directory that is a link out of the package is outside too.
        return path
            .canonicalize()
            .is_ok_and(|real| real.starts_with(package_root));
    }
    if metadata.file_type().is_symlink() {
        return path
            .canonicalize()
            .is_ok_and(|real| real.starts_with(package_root) && real.is_file());
    }
    false
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    let size = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .len();
    if size > MAX_BYTES {
        return Err(format!("{size} bytes is too large to be a licence"));
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if bytes.contains(&0) {
        return Err("it is not text".to_owned());
    }
    // Some licence files are Latin-1; the lossy form keeps everything that is ASCII.
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok((!text.trim().is_empty()).then_some(text))
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Lists the regular files of `directory`, sorted by name.
fn list(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    paths
}

/// The licence files of the package at `package_root`, plus the `extras` (paths relative to it).
pub fn discover(package_root: &Path, extras: &[String]) -> Discovery {
    let mut discovery = Discovery::default();
    let Ok(canonical_root) = package_root.canonicalize() else {
        discovery.missing_extras.extend(extras.iter().cloned());
        return discovery;
    };
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();

    let mut take = |discovery: &mut Discovery, path: &Path, kind: FileKind, extra: bool| {
        let Ok(real) = path.canonicalize() else {
            return;
        };
        if !seen.insert(real) {
            return;
        }
        let name = relative(path, package_root);
        match read_text(path) {
            Ok(Some(text)) => discovery.files.push(Found {
                path: name,
                kind,
                text,
                extra,
            }),
            Ok(None) => {}
            Err(reason) => discovery.skipped.push(format!("{name}: {reason}")),
        }
    };

    // The package root: files named like a licence.
    for path in list(package_root) {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(kind) = kind_of_name(name)
            && regular_file_inside(&path, &canonical_root)
        {
            take(&mut discovery, &path, kind, false);
        }
    }
    // `LICENSES/` (REUSE) and `licenses/`: every file is a licence text, whatever its name
    // (`MIT.txt`, `LicenseRef-Slint-Royalty-free-2.0.md`).
    for directory in ["LICENSES", "licenses"] {
        let directory = package_root.join(directory);
        let inside = directory
            .canonicalize()
            .is_ok_and(|real| real.starts_with(&canonical_root) && real.is_dir());
        if !inside {
            continue;
        }
        for path in list(&directory) {
            if !regular_file_inside(&path, &canonical_root) {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let kind = kind_of_name(name).unwrap_or(FileKind::License);
            take(&mut discovery, &path, kind, false);
        }
    }
    // The paths `credits.toml` names.
    for extra in extras {
        let relative_path = Path::new(extra);
        let escapes = relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)));
        let path = package_root.join(relative_path);
        if escapes || !regular_file_inside(&path, &canonical_root) {
            discovery.missing_extras.push(extra.clone());
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let kind = kind_of_name(name).unwrap_or(FileKind::License);
        // A file already found in the usual places stays as it was (`take` ignores it).
        take(&mut discovery, &path, kind, true);
    }
    discovery
}
