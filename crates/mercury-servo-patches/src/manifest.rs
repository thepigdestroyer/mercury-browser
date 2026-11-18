// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `third_party/vendor.toml`: every vendored crate and where it came from.

use crate::error::Result;
use crate::fail;
use serde::Deserialize;

pub const MANIFEST_PATH: &str = "third_party/vendor.toml";

const HEADER: &str = "\
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# The crates Mercury vendors in third_party/<name>/: each is the crate exactly as published on
# crates.io (`checksum`, built from `repository` at `commit`, directory `path`) plus Mercury's
# changes in third_party/patches/<name>.patch. Written by `mercury-servo-patches vendor` and
# `upgrade`; `unmarked` lists the changed files that cannot carry a `Mercury:` comment and is
# the only field edited by hand. See docs/developer-guide/vendored-patches.md.
";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    pub version: String,
    pub checksum: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub unmarked: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default, rename = "crate")]
    pub crates: Vec<Entry>,
}

fn quote(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

impl Entry {
    /// Whether the crate is part of Servo itself, so `upgrade --servo` moves it.
    pub fn is_servo(&self) -> bool {
        self.repository.as_deref().is_some_and(|url| {
            url.trim_end_matches('/').trim_end_matches(".git") == "https://github.com/servo/servo"
        })
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest> {
        let manifest: Manifest =
            toml::from_str(text).map_err(|error| fail!("{MANIFEST_PATH}: {error}"))?;
        for pair in manifest.crates.windows(2) {
            if pair[0].name >= pair[1].name {
                return Err(fail!(
                    "{MANIFEST_PATH}: crates must be listed once each, in name order ({} before {})",
                    pair[0].name,
                    pair[1].name
                ));
            }
        }
        for entry in &manifest.crates {
            let valid_name = !entry.name.is_empty()
                && entry
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !valid_name || entry.name == "patches" {
                return Err(fail!(
                    "{MANIFEST_PATH}: {:?} is not a usable crate name",
                    entry.name
                ));
            }
            if entry.checksum.len() != 64 || !entry.checksum.chars().all(|c| c.is_ascii_hexdigit())
            {
                return Err(fail!(
                    "{MANIFEST_PATH}: {}: checksum is not a SHA-256",
                    entry.name
                ));
            }
        }
        Ok(manifest)
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.crates.iter().find(|entry| entry.name == name)
    }

    /// Adds or replaces the entry for `entry.name`, keeping name order.
    pub fn upsert(&mut self, entry: Entry) {
        match self.crates.binary_search_by(|e| e.name.cmp(&entry.name)) {
            Ok(at) => self.crates[at] = entry,
            Err(at) => self.crates.insert(at, entry),
        }
    }

    pub fn render(&self) -> String {
        let mut out = HEADER.to_owned();
        for entry in &self.crates {
            out.push_str("\n[[crate]]\n");
            out.push_str(&format!("name = {}\n", quote(&entry.name)));
            out.push_str(&format!("version = {}\n", quote(&entry.version)));
            out.push_str(&format!("checksum = {}\n", quote(&entry.checksum)));
            for (key, value) in [
                ("repository", &entry.repository),
                ("commit", &entry.commit),
                ("path", &entry.path),
            ] {
                if let Some(value) = value {
                    out.push_str(&format!("{key} = {}\n", quote(value)));
                }
            }
            if !entry.unmarked.is_empty() {
                let items: Vec<String> = entry.unmarked.iter().map(|path| quote(path)).collect();
                out.push_str(&format!("unmarked = [{}]\n", items.join(", ")));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.into(),
            version: "1.0.0".into(),
            checksum: "a".repeat(64),
            repository: Some("https://github.com/servo/servo".into()),
            commit: Some("4e136d8".into()),
            path: Some("components/x".into()),
            unmarked: vec![],
        }
    }

    #[test]
    fn render_and_parse_round_trip_in_name_order() {
        let mut manifest = Manifest::default();
        manifest.upsert(entry("surfman"));
        let mut gaol = entry("gaol");
        gaol.repository = None;
        gaol.unmarked = vec!["Cargo.lock".into()];
        manifest.upsert(gaol);
        manifest.upsert(entry("servo-canvas"));
        let text = manifest.render();
        assert!(text.starts_with("# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0\n"));
        let parsed = Manifest::parse(&text).unwrap();
        assert_eq!(parsed, manifest);
        let names: Vec<_> = parsed.crates.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["gaol", "servo-canvas", "surfman"]);
        assert!(text.contains("unmarked = [\"Cargo.lock\"]\n"));
        assert!(!parsed.get("gaol").unwrap().is_servo());
        assert!(parsed.get("servo-canvas").unwrap().is_servo());
    }

    #[test]
    fn rejects_unsorted_duplicate_or_malformed_entries() {
        let mut manifest = Manifest {
            crates: vec![entry("b"), entry("a")],
        };
        assert!(
            Manifest::parse(&manifest.render())
                .unwrap_err()
                .0
                .contains("name order")
        );
        manifest.crates = vec![entry("a"), entry("a")];
        assert!(Manifest::parse(&manifest.render()).is_err());
        let mut bad = entry("a");
        bad.checksum = "xyz".into();
        manifest.crates = vec![bad];
        assert!(
            Manifest::parse(&manifest.render())
                .unwrap_err()
                .0
                .contains("SHA-256")
        );
        manifest.crates = vec![entry("../x")];
        assert!(Manifest::parse(&manifest.render()).is_err());
        assert!(Manifest::parse("[[crate]]\nname = \"a\"\nsurprise = 1\n").is_err());
    }
}
