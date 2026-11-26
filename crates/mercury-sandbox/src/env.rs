// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The environment of a sandboxed content process: an allow-list, not the browser's own.
//!
//! Everything not named here is dropped, so `HOME`, `PATH`, `SSH_AUTH_SOCK`, cloud tokens and
//! every `DYLD_*` variable except the development fallback never reach the child. `MERCURY_*`
//! is a prefix on purpose: a variable of the next lane must not break silently.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

const ALLOWED_NAMES: &[&str] = &[
    "GIO_MODULE_DIR",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_RUNTIME_DIR",
    "RUST_LOG",
    "RUST_BACKTRACE",
    "LANG",
    "TZ",
    "__CF_USER_TEXT_ENCODING",
    "DYLD_FALLBACK_LIBRARY_PATH",
];

const ALLOWED_PREFIXES: &[&str] = &["MERCURY_", "GST_", "LC_"];

/// The allow-listed variables of `inherited`, sorted by name, with `GST_REGISTRY_UPDATE=no`
/// forced in: a content process never rewrites the GStreamer plugin registry.
pub fn content_environment(
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let mut kept: BTreeMap<OsString, OsString> = inherited
        .into_iter()
        .filter(|(name, _)| is_allowed(name))
        .collect();
    kept.insert(OsString::from("GST_REGISTRY_UPDATE"), OsString::from("no"));
    kept.into_iter().collect()
}

/// A command for `executable` whose environment is exactly [`content_environment`].
pub fn content_process_command(
    executable: &Path,
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
) -> Command {
    let mut command = Command::new(executable);
    command.env_clear().envs(content_environment(inherited));
    command
}

fn is_allowed(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    ALLOWED_NAMES.contains(&name)
        || ALLOWED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}
