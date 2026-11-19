// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What Cargo says about the app: which crates are in it (`cargo tree`) and what each one is
//! (`cargo metadata`).
//!
//! `cargo metadata` alone is not the set: it unifies the features of the whole workspace and
//! lists crates the app does not contain. `cargo metadata --filter-platform` is not either: it
//! drops host-only build crates that `cargo tree` still lists. So the set comes from the tree and
//! the details from the unfiltered metadata.
use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What to ask Cargo.
#[derive(Debug, Clone)]
pub struct Options {
    pub package: String,
    pub target: String,
    /// Only normal edges: no build scripts and the crates they need.
    pub runtime_only: bool,
    pub manifest_path: PathBuf,
    /// The `cargo` to run (`$CARGO`, or `cargo`).
    pub cargo: PathBuf,
}

impl Options {
    /// The repository root: the directory of the workspace `Cargo.toml`.
    pub fn root(&self) -> PathBuf {
        self.manifest_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_owned)
    }
}

/// One crate of the tree: `name vX.Y.Z[ (location)][ (proc-macro)][ (*)]`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TreeEntry {
    pub name: String,
    pub version: String,
    /// The directory of a path crate (`(/work/third_party/servo)`), or a git source.
    pub location: Option<String>,
    pub proc_macro: bool,
}

impl TreeEntry {
    /// The directory of a path crate.
    pub fn path(&self) -> Option<&str> {
        self.location
            .as_deref()
            .filter(|location| location.starts_with('/') || location.get(1..3) == Some(":\\"))
    }
}

fn parse_tree_line(line: &str) -> Option<TreeEntry> {
    let (name, rest) = line.split_once(' ')?;
    let rest = rest.strip_prefix('v')?;
    let (version, mut rest) = match rest.split_once(' ') {
        Some((version, rest)) => (version, rest),
        None => (rest, ""),
    };
    if name.is_empty() || version.is_empty() {
        return None;
    }
    let mut proc_macro = false;
    loop {
        if let Some(stripped) = rest.strip_suffix("(*)") {
            rest = stripped.trim_end();
        } else if let Some(stripped) = rest.strip_suffix("(proc-macro)") {
            proc_macro = true;
            rest = stripped.trim_end();
        } else {
            break;
        }
    }
    let location = match rest {
        "" => None,
        other => {
            let inner = other.strip_prefix('(')?.strip_suffix(')')?;
            Some(inner.to_owned())
        }
    };
    Some(TreeEntry {
        name: name.to_owned(),
        version: version.to_owned(),
        location,
        proc_macro,
    })
}

/// Reads the output of `cargo tree --prefix none --format '{p}'`: every crate once, sorted.
pub fn parse_tree(text: &str) -> Result<Vec<TreeEntry>> {
    let mut entries = BTreeSet::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        // Blank lines, and the `[build-dependencies]` style headings Cargo prints in some modes.
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let entry = parse_tree_line(line).ok_or_else(|| {
            Error::parse(
                "cargo tree",
                format!("line {} is not `name vVersion ...`: {line:?}", number + 1),
            )
        })?;
        entries.insert(entry);
    }
    Ok(entries.into_iter().collect())
}

/// A package of `cargo metadata`, with only the fields the tool needs.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub license_file: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub repository: Option<String>,
    pub manifest_path: PathBuf,
    #[serde(default)]
    pub links: Option<String>,
}

impl Package {
    /// The directory of the package.
    pub fn root(&self) -> &Path {
        self.manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Metadata {
    pub packages: Vec<Package>,
    #[serde(default)]
    pub workspace_members: Vec<String>,
}

impl Metadata {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|error| Error::parse("cargo metadata", error))
    }

    pub fn is_workspace_member(&self, package: &Package) -> bool {
        self.workspace_members.iter().any(|id| *id == package.id)
    }

    /// The package a tree entry stands for: the same name and version, and for a path crate the
    /// same directory.
    pub fn find(&self, entry: &TreeEntry) -> Option<&Package> {
        let mut candidates = self
            .packages
            .iter()
            .filter(|package| package.name == entry.name && package.version == entry.version);
        match entry.path() {
            Some(path) => candidates
                .filter(|package| package.source.is_none())
                .find(|package| package.root() == Path::new(path)),
            None => candidates.find(|package| package.source.is_some()),
        }
    }
}

fn run(options: &Options, subcommand: &str, arguments: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new(&options.cargo);
    command
        .current_dir(options.root())
        .arg(subcommand)
        .arg("--locked")
        .arg("--manifest-path")
        .arg(&options.manifest_path)
        .args(arguments);
    let output = command.output().map_err(|error| {
        Error::Cargo(format!(
            "cannot run {} {subcommand}: {error}",
            options.cargo.display()
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut message = format!("`cargo {subcommand}` failed ({})", output.status);
        for line in stderr
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(12)
        {
            message.push_str("\n  ");
            message.push_str(line.trim_end());
        }
        return Err(Error::Cargo(message));
    }
    Ok(output.stdout)
}

/// `cargo tree`: the crates of the app, as text.
pub fn run_tree(options: &Options) -> Result<String> {
    let edges = if options.runtime_only {
        "normal"
    } else {
        "normal,build"
    };
    let bytes = run(
        options,
        "tree",
        &[
            "-p",
            &options.package,
            "-e",
            edges,
            "--target",
            &options.target,
            "--prefix",
            "none",
            "--format",
            "{p}",
        ],
    )?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// `cargo metadata`, unfiltered.
pub fn run_metadata(options: &Options) -> Result<Metadata> {
    let bytes = run(options, "metadata", &["--format-version", "1"])?;
    Metadata::parse(&bytes)
}
