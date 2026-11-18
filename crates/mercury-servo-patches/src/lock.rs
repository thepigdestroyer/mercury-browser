// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The parts of `Cargo.lock` and the root `Cargo.toml` the tool reads or edits.

use crate::error::Result;
use crate::fail;
use serde::Deserialize;

pub const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

#[derive(Debug, Deserialize)]
pub struct Lock {
    #[serde(default)]
    package: Vec<LockedPackage>,
    #[serde(default)]
    patch: LockedPatches,
}

#[derive(Debug, Deserialize)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct LockedPatches {
    #[serde(default)]
    unused: Vec<LockedPackage>,
}

impl Lock {
    pub fn parse(text: &str) -> Result<Lock> {
        toml::from_str(text).map_err(|error| fail!("Cargo.lock: {error}"))
    }

    /// The checksum of `name` `version` from crates.io, if the graph uses it.
    pub fn registry_checksum(&self, name: &str, version: &str) -> Option<&str> {
        self.package
            .iter()
            .find(|p| {
                p.name == name && p.version == version && p.source.as_deref() == Some(CRATES_IO)
            })
            .and_then(|p| p.checksum.as_deref())
    }

    /// Whether the graph uses `name` `version` from a local path (no source).
    pub fn has_path_package(&self, name: &str, version: &str) -> bool {
        self.package
            .iter()
            .any(|p| p.name == name && p.version == version && p.source.is_none())
    }

    /// The versions of `name` the graph uses, wherever they come from.
    pub fn versions(&self, name: &str) -> Vec<&str> {
        self.package
            .iter()
            .filter(|p| p.name == name)
            .map(|p| p.version.as_str())
            .collect()
    }

    /// Whether Cargo recorded a `[patch]` entry for `name` as unused.
    pub fn patch_unused(&self, name: &str) -> bool {
        self.patch.unused.iter().any(|p| p.name == name)
    }
}

/// The path a root `Cargo.toml`'s `[patch.crates-io]` gives for `name`.
pub fn patch_path(cargo_toml: &str, name: &str) -> Result<Option<String>> {
    let value: toml::Table =
        toml::from_str(cargo_toml).map_err(|error| fail!("Cargo.toml: {error}"))?;
    Ok(value
        .get("patch")
        .and_then(|patch| patch.get("crates-io"))
        .and_then(|section| section.get(name))
        .and_then(|entry| entry.get("path"))
        .and_then(|path| path.as_str())
        .map(str::to_owned))
}

/// `cargo_toml` with `name = { path = "<path>" }` in `[patch.crates-io]`, the
/// entries kept in name order and everything else untouched. The section is
/// appended if there is none. An existing entry for `name` is an error.
pub fn add_patch_entry(cargo_toml: &str, name: &str, path: &str) -> Result<String> {
    if patch_path(cargo_toml, name)?.is_some() {
        return Err(fail!("Cargo.toml already patches {name}"));
    }
    let entry = format!("{name} = {{ path = \"{path}\" }}");
    let lines: Vec<&str> = cargo_toml.lines().collect();
    let Some(header) = lines
        .iter()
        .position(|line| line.trim() == "[patch.crates-io]")
    else {
        let mut out = cargo_toml.trim_end().to_owned();
        out.push_str("\n\n[patch.crates-io]\n");
        out.push_str(&entry);
        out.push('\n');
        return Ok(out);
    };
    let end = lines[header + 1..]
        .iter()
        .position(|line| line.trim().is_empty() || line.trim_start().starts_with('['))
        .map_or(lines.len(), |at| header + 1 + at);
    let key = |line: &str| {
        line.split('=')
            .next()
            .unwrap_or_default()
            .trim()
            .trim_matches('"')
            .to_owned()
    };
    let insert_at = (header + 1..end)
        .find(|&at| {
            let line = lines[at].trim_start();
            !line.starts_with('#') && key(line).as_str() > name
        })
        .unwrap_or(end);
    let mut out: Vec<&str> = Vec::with_capacity(lines.len() + 1);
    out.extend_from_slice(&lines[..insert_at]);
    out.push(&entry);
    out.extend_from_slice(&lines[insert_at..]);
    let mut text = out.join("\n");
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"
version = 4

[[package]]
name = "gaol"
version = "0.2.1"
dependencies = ["libc"]

[[package]]
name = "libc"
version = "0.2.177"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc"

[[patch.unused]]
name = "surfman"
version = "0.14.0"
"#;

    #[test]
    fn reads_checksums_path_packages_and_unused_patches() {
        let lock = Lock::parse(LOCK).unwrap();
        assert_eq!(lock.registry_checksum("libc", "0.2.177"), Some("abc"));
        assert_eq!(lock.registry_checksum("libc", "0.2.1"), None);
        assert_eq!(lock.registry_checksum("gaol", "0.2.1"), None);
        assert!(lock.has_path_package("gaol", "0.2.1"));
        assert!(!lock.has_path_package("libc", "0.2.177"));
        assert_eq!(lock.versions("libc"), ["0.2.177"]);
        assert!(lock.patch_unused("surfman"));
        assert!(!lock.patch_unused("gaol"));
    }

    const ROOT: &str = "[workspace]\nmembers = []\n\n[patch.crates-io]\n# keep sorted\ngaol = { path = \"third_party/gaol\" }\nsurfman = { path = \"third_party/surfman\" }\n\n[profile.dev]\nopt-level = 1\n";

    #[test]
    fn adds_patch_entries_in_name_order() {
        let text = add_patch_entry(ROOT, "servo-canvas", "third_party/servo-canvas").unwrap();
        assert_eq!(
            text,
            "[workspace]\nmembers = []\n\n[patch.crates-io]\n# keep sorted\ngaol = { path = \"third_party/gaol\" }\nservo-canvas = { path = \"third_party/servo-canvas\" }\nsurfman = { path = \"third_party/surfman\" }\n\n[profile.dev]\nopt-level = 1\n"
        );
        assert_eq!(
            patch_path(&text, "servo-canvas").unwrap().as_deref(),
            Some("third_party/servo-canvas")
        );
        let last = add_patch_entry(ROOT, "zzz", "third_party/zzz").unwrap();
        assert!(last.contains("surfman = { path = \"third_party/surfman\" }\nzzz = { path = \"third_party/zzz\" }\n\n[profile.dev]"));
        let first = add_patch_entry(ROOT, "a", "third_party/a").unwrap();
        assert!(first.contains("# keep sorted\na = { path"));
    }

    #[test]
    fn adds_the_section_when_missing_and_refuses_duplicates() {
        let text = add_patch_entry("[workspace]\n", "demo", "third_party/demo").unwrap();
        assert_eq!(
            text,
            "[workspace]\n\n[patch.crates-io]\ndemo = { path = \"third_party/demo\" }\n"
        );
        assert!(add_patch_entry(ROOT, "gaol", "third_party/gaol").is_err());
    }
}
