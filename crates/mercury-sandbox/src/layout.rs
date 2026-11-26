// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Where the application and its private run directory are, from what the process knows.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The four GLib user directories that `media_buffers.rs` points into one private per-run
/// directory (`<run root>/cache`, `data`, `config`, `runtime`).
const GLIB_USER_DIRECTORIES: [&str; 4] = [
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_RUNTIME_DIR",
];

/// The part of the file system the profile lets the process read as "the application": the
/// enclosing `*.app` when `executable` is `*.app/Contents/MacOS/<name>`, otherwise the
/// directory the executable is in (a development run: `target/release`, which holds `lib/`).
pub fn bundle_root(executable: &Path) -> PathBuf {
    let directory = executable.parent().unwrap_or(executable);
    if directory.ends_with("Contents/MacOS")
        && let Some(app) = directory.parent().and_then(Path::parent)
        && app.extension().is_some_and(|extension| extension == "app")
    {
        return app.to_path_buf();
    }
    directory.to_path_buf()
}

/// The private per-run directory the four GLib user directories share, or `None` when any of
/// them is missing or relative, or they do not share one parent (then nothing is writable).
pub fn run_root_from_env(get: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let mut common: Option<PathBuf> = None;
    for name in GLIB_USER_DIRECTORIES {
        let value = PathBuf::from(get(name)?);
        if !value.is_absolute() {
            return None;
        }
        let parent = value.parent()?.to_path_buf();
        match &common {
            None => common = Some(parent),
            Some(existing) if *existing == parent => {}
            Some(_) => return None,
        }
    }
    common.filter(|root| root.components().count() > 1)
}
