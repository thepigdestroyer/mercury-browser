// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The three files the tool reads besides Cargo's own output: `crates/mercury-credits/credits.toml`
//! (what the tool cannot find by itself), `deny.toml` (which licences Mercury may use, the one
//! policy) and `third_party/vendor.toml` (where each vendored crate comes from).
use crate::error::{Error, Result};
use crate::spdx::TakenSet;
use serde::Deserialize;
use std::path::Path;

/// Where `credits.toml` is, relative to the repository root.
pub const CREDITS_TOML: &str = "crates/mercury-credits/credits.toml";

/// `[[extra]]`: licence files deeper in a package, for native code compiled into the app.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Extra {
    #[serde(rename = "crate")]
    pub name: String,
    pub version: Option<String>,
    /// Relative to the package root.
    #[serde(default)]
    pub files: Vec<String>,
    /// Shown in the page next to the texts these files hold.
    pub note: Option<String>,
}

/// `[[clarify]]`: the package has no licence file, or the wrong one.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Clarify {
    #[serde(rename = "crate")]
    pub name: String,
    pub version: Option<String>,
    /// Which standard text the copyright line belongs to.
    pub licence: Option<String>,
    pub copyright: Option<String>,
    /// Where a person read the copyright line (for the maintainers; not shown).
    pub source: Option<String>,
}

/// `[[ignore]]`: a crate in the set that is not credited.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ignore {
    #[serde(rename = "crate")]
    pub name: String,
    pub version: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcceptUnclassified {
    #[serde(default)]
    pub crates: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub extra: Vec<Extra>,
    #[serde(default)]
    pub clarify: Vec<Clarify>,
    #[serde(default)]
    pub ignore: Vec<Ignore>,
    #[serde(default)]
    pub accept_unclassified: AcceptUnclassified,
}

/// Whether an entry that names `entry_name` (and maybe one version) applies to this crate.
pub fn applies(entry_name: &str, entry_version: Option<&str>, name: &str, version: &str) -> bool {
    entry_name == name && entry_version.is_none_or(|wanted| wanted == version)
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|error| Error::parse(CREDITS_TOML, error.to_string().trim()))
    }

    /// Reads `credits.toml`; a repository without one has an empty config.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(CREDITS_TOML);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Read { path, source }),
        }
    }

    pub fn extras_for(&self, name: &str, version: &str) -> Vec<&Extra> {
        self.extra
            .iter()
            .filter(|entry| applies(&entry.name, entry.version.as_deref(), name, version))
            .collect()
    }

    pub fn clarify_for(&self, name: &str, version: &str) -> Option<&Clarify> {
        self.clarify
            .iter()
            .find(|entry| applies(&entry.name, entry.version.as_deref(), name, version))
    }

    pub fn ignore_for(&self, name: &str, version: &str) -> Option<&Ignore> {
        self.ignore
            .iter()
            .find(|entry| applies(&entry.name, entry.version.as_deref(), name, version))
    }

    pub fn accepts_unclassified(&self, name: &str) -> bool {
        self.accept_unclassified
            .crates
            .iter()
            .any(|accepted| accepted == name)
    }
}

/// An entry of `deny.toml`'s `allow` lists: a plain id, or `{ name = "...", ... }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum AllowEntry {
    Plain(String),
    Named { name: String },
}

impl AllowEntry {
    fn into_key(self) -> String {
        match self {
            Self::Plain(key) | Self::Named { name: key } => key,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawException {
    #[serde(rename = "crate")]
    name: String,
    version: Option<String>,
    #[serde(default)]
    allow: Vec<AllowEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawLicenses {
    #[serde(default)]
    allow: Vec<AllowEntry>,
    #[serde(default)]
    exceptions: Vec<RawException>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawDeny {
    #[serde(default)]
    licenses: RawLicenses,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exception {
    pub name: String,
    /// `=1.2.3` or `1.2.3` limits the exception to that version; nothing else is supported.
    pub version: Option<String>,
    pub allow: Vec<String>,
}

/// The licence policy of `deny.toml`: `[licenses] allow` and `[[licenses.exceptions]]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DenyPolicy {
    pub allow: Vec<String>,
    pub exceptions: Vec<Exception>,
}

impl DenyPolicy {
    pub fn parse(text: &str) -> Result<Self> {
        let raw: RawDeny = toml::from_str(text)
            .map_err(|error| Error::parse("deny.toml", error.to_string().trim()))?;
        Ok(Self {
            allow: raw
                .licenses
                .allow
                .into_iter()
                .map(AllowEntry::into_key)
                .collect(),
            exceptions: raw
                .licenses
                .exceptions
                .into_iter()
                .map(|exception| Exception {
                    name: exception.name,
                    version: exception.version,
                    allow: exception
                        .allow
                        .into_iter()
                        .map(AllowEntry::into_key)
                        .collect(),
                })
                .collect(),
        })
    }

    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("deny.toml");
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Read { path, source })?;
        Self::parse(&text)
    }

    /// The licences this crate may be used under: the allow-list and its own exceptions.
    pub fn taken_for(&self, name: &str, version: &str) -> TakenSet {
        let mut taken = TakenSet::new(&self.allow);
        for exception in &self.exceptions {
            let version_matches = exception
                .version
                .as_deref()
                .is_none_or(|wanted| wanted.trim_start_matches(['=', ' ']) == version);
            if exception.name == name && version_matches {
                for key in &exception.allow {
                    taken.insert(key);
                }
            }
        }
        taken
    }
}

/// One `[[crate]]` of `third_party/vendor.toml`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Vendored {
    pub name: String,
    pub version: String,
    pub repository: Option<String>,
    pub commit: Option<String>,
    pub path: Option<String>,
    pub license: Option<String>,
    pub license_note: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawVendor {
    #[serde(default, rename = "crate")]
    crates: Vec<Vendored>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VendorFile {
    pub crates: Vec<Vendored>,
}

impl VendorFile {
    pub fn parse(text: &str) -> Result<Self> {
        let raw: RawVendor = toml::from_str(text)
            .map_err(|error| Error::parse("third_party/vendor.toml", error.to_string().trim()))?;
        Ok(Self { crates: raw.crates })
    }

    /// Reads `third_party/vendor.toml`; a repository that vendors nothing has none.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("third_party/vendor.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Read { path, source }),
        }
    }

    pub fn find(&self, name: &str, version: &str) -> Option<&Vendored> {
        self.crates
            .iter()
            .find(|entry| entry.name == name && entry.version == version)
            .or_else(|| self.crates.iter().find(|entry| entry.name == name))
    }
}
