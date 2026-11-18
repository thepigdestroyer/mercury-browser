// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The tool's commands: `vendor`, `diff`, `export`, `check` and `upgrade`.

use crate::archive::unpack;
use crate::error::{Context, Error, Result};
use crate::fail;
use crate::lock::{self, Lock};
use crate::manifest::{Entry, MANIFEST_PATH, Manifest};
use crate::patch::{self, FilePatch, Kind};
use crate::registry::Registry;
use crate::tree::{NOTES_FILE, Tree};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The marker every file Mercury changes must carry in one of its added lines.
pub const MARKER: &str = "Mercury:";
pub const PATCHES_DIR: &str = "third_party/patches";
const THIRD_PARTY: &str = "third_party";
const CONFLICT_MARKERS: [&str; 3] = ["<<<<<<<", "|||||||", ">>>>>>>"];

/// A repository checkout and how to reach the outside world from it.
pub struct Repo {
    pub root: PathBuf,
    pub registry: Registry,
    /// The `cargo` program, or `None` to never run it (tests).
    pub cargo: Option<PathBuf>,
    /// Whether to ask `git` about ignored files.
    pub git: bool,
}

/// `name@version`.
pub fn parse_spec(spec: &str) -> Result<(String, String)> {
    match spec.split_once('@') {
        Some((name, version)) if !name.is_empty() && !version.is_empty() => {
            Ok((name.to_owned(), version.to_owned()))
        }
        _ => Err(fail!("expected <crate>@<version>, got {spec:?}")),
    }
}

fn write_out(out: &mut dyn Write, text: &str) -> Result<()> {
    out.write_all(text.as_bytes())
        .context(|| "writing the output".to_owned())
}

/// Where a crate came from, from its `.cargo_vcs_info.json` and `Cargo.toml`.
struct Source {
    repository: Option<String>,
    commit: Option<String>,
    path: Option<String>,
    license: Option<String>,
}

fn source_of(tree: &Tree, name: &str) -> Result<Source> {
    let manifest: toml::Table = tree
        .files
        .get("Cargo.toml")
        .ok_or_else(|| fail!("{name}: the archive has no Cargo.toml"))
        .and_then(|bytes| {
            toml::from_str(&String::from_utf8_lossy(bytes))
                .map_err(|error| fail!("{name}: Cargo.toml: {error}"))
        })?;
    let package = manifest.get("package");
    let field = |key: &str| {
        package
            .and_then(|package| package.get(key))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    };
    let vcs: Option<serde_json::Value> = tree
        .files
        .get(".cargo_vcs_info.json")
        .map(|bytes| serde_json::from_slice(bytes))
        .transpose()
        .map_err(|error| fail!("{name}: .cargo_vcs_info.json: {error}"))?;
    let vcs_field = |pointer: &str| {
        vcs.as_ref()
            .and_then(|vcs| vcs.pointer(pointer))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    };
    Ok(Source {
        repository: field("repository"),
        commit: vcs_field("/git/sha1"),
        path: vcs_field("/path_in_vcs"),
        license: field("license"),
    })
}

/// The `[package]` name and version a crate's `Cargo.toml` declares.
fn package_id(tree: &Tree) -> Option<(String, String)> {
    let manifest: toml::Table =
        toml::from_str(&String::from_utf8_lossy(tree.files.get("Cargo.toml")?)).ok()?;
    let package = manifest.get("package")?;
    Some((
        package.get("name")?.as_str()?.to_owned(),
        package.get("version")?.as_str()?.to_owned(),
    ))
}

fn preamble(entry: &Entry) -> String {
    format!(
        "Mercury's changes to {} {}, against the crate as published on crates.io\n\
         (SHA-256 {}). Written by `mercury-servo-patches export`; do not edit by hand.\n\
         The changed files stay under the licence of {}.",
        entry.name, entry.version, entry.checksum, entry.name
    )
}

fn notes_skeleton(entry: &Entry, license: Option<&str>) -> String {
    let source = match (&entry.repository, &entry.commit) {
        (Some(repository), Some(commit)) => format!(
            ", built from {repository} at commit\n`{commit}`{}",
            entry
                .path
                .as_deref()
                .filter(|path| !path.is_empty())
                .map(|path| format!(", path `{path}`"))
                .unwrap_or_default()
        ),
        (Some(repository), None) => format!(", from {repository}"),
        _ => String::new(),
    };
    format!(
        "# Patched {name} {version}\n\
         \n\
         This directory vendors `{name}` {version} under its own licence ({licence}).\n\
         It is the crate exactly as published on crates.io (checksum\n\
         `{checksum}`){source},\n\
         plus Mercury's changes in `third_party/patches/{name}.patch`.\n\
         `third_party/vendor.toml` records where it came from.\n\
         \n\
         `mercury-servo-patches diff {name}` shows the changes. Each changed file carries\n\
         a `Mercury:` comment at every change; `make servo-patches-check` fails without one.\n\
         \n\
         ## Changes\n\
         \n\
         None yet. For each change, say what it does, why Mercury needs it, the upstream\n\
         file and line it changes, how it is tested and when it can be dropped.\n\
         \n\
         ## Removing this directory\n\
         \n\
         Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in\n\
         `third_party/vendor.toml`, `third_party/patches/{name}.patch` and this directory\n\
         once Mercury no longer needs to change the crate.\n",
        name = entry.name,
        version = entry.version,
        checksum = entry.checksum,
        licence = license
            .map(|license| format!("`{license}` in its `Cargo.toml`"))
            .unwrap_or_else(|| "see its `Cargo.toml`".to_owned()),
    )
}

/// What `upgrade` did with one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Mercury's change applied to the new version without conflict.
    Applied,
    /// Upstream did not change the file; Mercury's version was kept.
    Kept,
    /// Upstream now has exactly Mercury's version.
    Absorbed,
    /// Both changed it in ways that overlap; the file has conflict markers.
    Conflict,
    /// Mercury deletes the file but upstream changed it; upstream's file was kept.
    DeletedChangedUpstream,
    /// Mercury changed a file the new version no longer has.
    Gone,
}

impl Outcome {
    fn describe(&self) -> &'static str {
        match self {
            Outcome::Applied => "applied cleanly",
            Outcome::Kept => "applied cleanly (unchanged upstream)",
            Outcome::Absorbed => "already upstream (Mercury's change is no longer needed)",
            Outcome::Conflict => "CONFLICT (conflict markers written)",
            Outcome::DeletedChangedUpstream => {
                "CONFLICT (Mercury deletes it, upstream changed it; upstream's file kept)"
            }
            Outcome::Gone => "GONE (upstream removed the file Mercury changes)",
        }
    }

    pub fn is_conflict(&self) -> bool {
        matches!(self, Outcome::Conflict | Outcome::DeletedChangedUpstream)
    }
}

/// The result of an upgrade: per crate, per changed file, what happened.
#[derive(Debug, Default)]
pub struct UpgradeReport {
    pub crates: Vec<UpgradedCrate>,
}

/// One crate moved by `upgrade`.
#[derive(Debug)]
pub struct UpgradedCrate {
    pub name: String,
    pub from: String,
    pub to: String,
    /// What happened to each file Mercury changes.
    pub files: Vec<(String, Outcome)>,
}

impl UpgradeReport {
    pub fn conflicts(&self) -> usize {
        self.crates
            .iter()
            .flat_map(|upgraded| &upgraded.files)
            .filter(|(_, outcome)| outcome.is_conflict())
            .count()
    }
}

/// Merges Mercury's changes (`ours`, made against `old`) into `new`.
pub fn merge_trees(
    old: &Tree,
    ours: &Tree,
    new: &Tree,
    changes: &[FilePatch],
) -> (Tree, Vec<(String, Outcome)>) {
    let mut merged = new.clone();
    let mut report = Vec::new();
    for change in changes {
        let path = &change.path;
        let old_file = old.files.get(path);
        let our_file = ours.files.get(path);
        let new_file = new.files.get(path);
        let outcome = match (change.kind, new_file) {
            (Kind::Modified, None) => Outcome::Gone,
            (Kind::Modified, Some(new_file)) => {
                let (old_file, our_file) = (
                    old_file.expect("patched file"),
                    our_file.expect("patched file"),
                );
                if old_file == new_file {
                    merged.files.insert(path.clone(), our_file.clone());
                    Outcome::Kept
                } else if our_file == new_file {
                    Outcome::Absorbed
                } else {
                    match diffy::merge_bytes(old_file, our_file, new_file) {
                        Ok(text) => {
                            merged.files.insert(path.clone(), text);
                            Outcome::Applied
                        }
                        Err(text) => {
                            merged.files.insert(path.clone(), text);
                            Outcome::Conflict
                        }
                    }
                }
            }
            (Kind::Added, None) => {
                merged
                    .files
                    .insert(path.clone(), our_file.expect("added file").clone());
                Outcome::Applied
            }
            (Kind::Added, Some(new_file)) => {
                let our_file = our_file.expect("added file");
                if our_file == new_file {
                    Outcome::Absorbed
                } else {
                    let text =
                        diffy::merge_bytes(b"", our_file, new_file).unwrap_or_else(|text| text);
                    merged.files.insert(path.clone(), text);
                    Outcome::Conflict
                }
            }
            (Kind::Deleted, None) => Outcome::Kept,
            (Kind::Deleted, Some(new_file)) => {
                if old_file == Some(new_file) {
                    merged.files.remove(path);
                    merged.executable.remove(path);
                    Outcome::Kept
                } else {
                    Outcome::DeletedChangedUpstream
                }
            }
        };
        report.push((path.clone(), outcome));
    }
    (merged, report)
}

/// Problems that would let `cargo fmt --all`, clippy or the build treat a
/// vendored crate as Mercury's own code, from `cargo metadata --no-deps`.
///
/// `cargo fmt --all` formats the workspace members and every dependency a
/// member declares with `path = …`, recursively. A `[patch.crates-io]` entry
/// is not such a declaration, so a vendored crate is left alone as long as it
/// is not a member and no member depends on it by path. Clippy lints only
/// members (`--workspace`).
pub fn workspace_problems(metadata: &serde_json::Value, root: &Path) -> Vec<String> {
    let third_party = root.join(THIRD_PARTY);
    let inside = |path: &str| Path::new(path).starts_with(&third_party);
    let mut problems = Vec::new();
    for package in metadata["packages"].as_array().into_iter().flatten() {
        let name = package["name"].as_str().unwrap_or("?");
        if package["manifest_path"].as_str().is_some_and(inside) {
            problems.push(format!(
                "{name} in third_party/ is a workspace member; cargo fmt --all and clippy would change or lint it (add it to `workspace.exclude`)"
            ));
        }
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            if dependency["path"].as_str().is_some_and(inside) {
                problems.push(format!(
                    "{name} depends on {} by path in third_party/; cargo fmt --all would format it (depend on the crates.io version, which [patch.crates-io] redirects)",
                    dependency["name"].as_str().unwrap_or("?")
                ));
            }
        }
    }
    problems
}

impl Repo {
    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn crate_dir(&self, name: &str) -> PathBuf {
        self.root.join(THIRD_PARTY).join(name)
    }

    fn patch_file(&self, name: &str) -> PathBuf {
        self.root.join(PATCHES_DIR).join(format!("{name}.patch"))
    }

    fn read_text(&self, relative: &str) -> Result<String> {
        let path = self.path(relative);
        fs::read_to_string(&path).context(|| format!("reading {}", path.display()))
    }

    pub fn manifest(&self) -> Result<Manifest> {
        let path = self.path(MANIFEST_PATH);
        if !path.exists() {
            return Ok(Manifest::default());
        }
        Manifest::parse(&self.read_text(MANIFEST_PATH)?)
    }

    fn save_manifest(&self, manifest: &Manifest) -> Result<()> {
        let path = self.path(MANIFEST_PATH);
        fs::write(&path, manifest.render()).context(|| format!("writing {}", path.display()))
    }

    fn lock(&self) -> Result<Lock> {
        Lock::parse(&self.read_text("Cargo.lock")?)
    }

    fn entry(&self, manifest: &Manifest, name: &str) -> Result<Entry> {
        manifest
            .get(name)
            .cloned()
            .ok_or_else(|| fail!("{name} is not vendored (not in {MANIFEST_PATH})"))
    }

    fn pristine(&self, name: &str, version: &str, checksum: &str) -> Result<Tree> {
        let archive = self.registry.archive(name, version, checksum)?;
        unpack(&archive, name, version)
    }

    fn run_cargo(&self, args: &[&str]) -> Result<()> {
        let Some(cargo) = &self.cargo else {
            return Ok(());
        };
        let output = Command::new(cargo)
            .args(args)
            .current_dir(&self.root)
            .output()
            .context(|| format!("running cargo {}", args.join(" ")))?;
        if !output.status.success() {
            return Err(fail!(
                "cargo {} failed:\n{}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }

    fn ignored_files(&self, name: &str) -> Result<Vec<String>> {
        if !self.git {
            return Ok(Vec::new());
        }
        let dir = format!("{THIRD_PARTY}/{name}");
        let output = Command::new("git")
            .args([
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "--",
                &dir,
            ])
            .current_dir(&self.root)
            .output()
            .context(|| "running git ls-files".to_owned())?;
        if !output.status.success() {
            return Ok(Vec::new());
        }
        let build_output = format!("{dir}/target/");
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.starts_with(&build_output))
            .map(str::to_owned)
            .collect())
    }

    /// Mercury's changes to `name`: pristine archive against the directory.
    fn changes(&self, entry: &Entry) -> Result<(Tree, Tree, Vec<FilePatch>)> {
        let pristine = self.pristine(&entry.name, &entry.version, &entry.checksum)?;
        let dir = self.crate_dir(&entry.name);
        if !dir.is_dir() {
            return Err(fail!("{} does not exist", dir.display()));
        }
        let tree = Tree::read(&dir)?;
        let changes = patch::diff_trees(&pristine, &tree)
            .map_err(|error| fail!("{}: {error}", entry.name))?;
        Ok((pristine, tree, changes))
    }

    fn write_patch(&self, entry: &Entry, changes: &[FilePatch]) -> Result<PathBuf> {
        let path = self.patch_file(&entry.name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&path, patch::render(&preamble(entry), changes))
            .context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    /// `vendor <crate>@<version>`.
    pub fn vendor(&self, spec: &str, out: &mut dyn Write) -> Result<()> {
        let (name, version) = parse_spec(spec)?;
        let mut manifest = self.manifest()?;
        if manifest.get(&name).is_some() {
            return Err(fail!(
                "{name} is already vendored; use `upgrade {name}@{version}` to move it"
            ));
        }
        let dir = self.crate_dir(&name);
        if dir.exists() {
            return Err(fail!("{} already exists", dir.display()));
        }
        let lock = self.lock()?;
        let Some(checksum) = lock.registry_checksum(&name, &version).map(str::to_owned) else {
            let versions = lock.versions(&name);
            return Err(fail!(
                "Cargo.lock has no {name} {version} from crates.io ({}); only crates the browser uses can be vendored",
                if versions.is_empty() {
                    "no version at all".to_owned()
                } else {
                    format!("it has {}", versions.join(", "))
                }
            ));
        };
        if !self.registry.is_cached(&name, &version) {
            write_out(out, "running cargo fetch --locked\n")?;
            self.run_cargo(&["fetch", "--locked"])?;
        }
        let pristine = self.pristine(&name, &version, &checksum)?;
        let source = source_of(&pristine, &name)?;
        pristine.write(&dir)?;

        let entry = Entry {
            name: name.clone(),
            version: version.clone(),
            checksum,
            repository: source.repository,
            commit: source.commit,
            path: source.path,
            unmarked: Vec::new(),
        };
        manifest.upsert(entry.clone());
        self.save_manifest(&manifest)?;

        let cargo_toml = self.read_text("Cargo.toml")?;
        let updated = lock::add_patch_entry(&cargo_toml, &name, &format!("{THIRD_PARTY}/{name}"))?;
        fs::write(self.path("Cargo.toml"), updated).context(|| "writing Cargo.toml".to_owned())?;
        fs::write(
            dir.join(NOTES_FILE),
            notes_skeleton(&entry, source.license.as_deref()),
        )
        .context(|| format!("writing {}", dir.join(NOTES_FILE).display()))?;
        let patch_path = self.write_patch(&entry, &[])?;

        // Resolving the graph again rewrites Cargo.lock to take the crate from
        // its path; nothing is compiled.
        self.run_cargo(&["metadata", "--format-version", "1", "--offline"])?;
        if self.cargo.is_some() && !self.lock()?.has_path_package(&name, &version) {
            return Err(fail!(
                "Cargo.lock still takes {name} {version} from crates.io after the patch; run `cargo update -p {name}@{version}` and check the [patch.crates-io] entry"
            ));
        }

        write_out(
            out,
            &format!(
                "vendored {name} {version} into {dir}\n\
                 recorded it in {MANIFEST_PATH}, added it to [patch.crates-io] in Cargo.toml,\n\
                 wrote {notes} and {patch}\n",
                dir = dir.display(),
                notes = dir.join(NOTES_FILE).display(),
                patch = patch_path.display(),
            ),
        )?;
        let ignored = self.ignored_files(&name)?;
        if !ignored.is_empty() {
            write_out(
                out,
                &format!(
                    "the crate's own .gitignore hides {} of its files from git; add them with\n  git add -f -- third_party/{name}\n",
                    ignored.len()
                ),
            )?;
        }
        Ok(())
    }

    /// `diff <crate>`: Mercury's changes, from the directory as it is now.
    pub fn diff(&self, name: &str, out: &mut dyn Write) -> Result<()> {
        let entry = self.entry(&self.manifest()?, name)?;
        let (_, _, changes) = self.changes(&entry)?;
        out.write_all(&patch::render("", &changes))
            .context(|| "writing the output".to_owned())
    }

    /// `export`: rewrites every `third_party/patches/<crate>.patch` and
    /// removes patch files of crates that are no longer vendored.
    pub fn export(&self, out: &mut dyn Write) -> Result<()> {
        let manifest = self.manifest()?;
        for entry in &manifest.crates {
            let (_, _, changes) = self.changes(entry)?;
            let path = self.write_patch(entry, &changes)?;
            write_out(
                out,
                &format!(
                    "wrote {} ({} files changed)\n",
                    path.display(),
                    changes.len()
                ),
            )?;
        }
        let names: BTreeSet<String> = manifest
            .crates
            .iter()
            .map(|e| format!("{}.patch", e.name))
            .collect();
        if let Ok(entries) = fs::read_dir(self.path(PATCHES_DIR)) {
            for entry in entries.flatten() {
                let file = entry.file_name().to_string_lossy().into_owned();
                if file.ends_with(".patch") && !names.contains(&file) {
                    fs::remove_file(entry.path())
                        .context(|| format!("removing {}", entry.path().display()))?;
                    write_out(out, &format!("removed {}\n", entry.path().display()))?;
                }
            }
        }
        Ok(())
    }

    /// The problems `check` finds with one crate.
    fn check_crate(&self, entry: &Entry, lock: &Lock, cargo_toml: &str) -> Result<Vec<String>> {
        let name = &entry.name;
        let mut problems = Vec::new();
        let expected_path = format!("{THIRD_PARTY}/{name}");
        match lock::patch_path(cargo_toml, name)? {
            Some(path) if path.trim_end_matches('/') == expected_path => {}
            Some(path) => problems.push(format!(
                "Cargo.toml patches {name} with {path}, not {expected_path}"
            )),
            None => problems.push(format!(
                "Cargo.toml has no [patch.crates-io] entry for {name}"
            )),
        }
        if lock.patch_unused(name) {
            problems.push(format!("Cargo.lock lists the {name} patch as unused"));
        }
        if !lock.has_path_package(name, &entry.version) {
            problems.push(format!(
                "Cargo.lock does not take {name} {} from {expected_path} (it has {}); run `cargo update -p {name}`",
                entry.version,
                lock.versions(name).join(", ")
            ));
        }
        let dir = self.crate_dir(name);
        if !dir.is_dir() {
            problems.push(format!("{expected_path} does not exist"));
            return Ok(problems);
        }
        if !dir.join(NOTES_FILE).is_file() {
            problems.push(format!("{expected_path}/{NOTES_FILE} is missing"));
        }
        for file in self.ignored_files(name)? {
            problems.push(format!(
                "{file} is ignored by git, so the committed crate lacks it: git add -f -- {file}"
            ));
        }

        let pristine = self.pristine(name, &entry.version, &entry.checksum)?;
        let tree = Tree::read(&dir)?;
        if package_id(&tree) != Some((name.clone(), entry.version.clone())) {
            problems.push(format!(
                "{expected_path}/Cargo.toml does not declare {name} {}",
                entry.version
            ));
        }
        let patch_path = self.patch_file(name);
        let Ok(patch_bytes) = fs::read(&patch_path) else {
            problems.push(format!(
                "{} is missing; run `mercury-servo-patches export`",
                patch_path.display()
            ));
            return Ok(problems);
        };
        let changes = match patch::parse(&patch_bytes) {
            Ok(changes) => changes,
            Err(error) => {
                problems.push(format!("{}: {error}", patch_path.display()));
                return Ok(problems);
            }
        };
        match patch::apply(&pristine, &changes) {
            Err(error) => problems.push(format!(
                "{name}: the exported patch does not apply to the published crate ({error}); fix the directory and run `mercury-servo-patches export`"
            )),
            Ok(patched) => {
                let mut paths: BTreeSet<&String> = patched.files.keys().collect();
                paths.extend(tree.files.keys());
                let differing: Vec<&str> = paths
                    .into_iter()
                    .filter(|path| patched.files.get(*path) != tree.files.get(*path))
                    .map(String::as_str)
                    .collect();
                if !differing.is_empty() {
                    problems.push(format!(
                        "{name}: the directory is not the published crate plus {}; run `mercury-servo-patches export` (or undo the edit): {}",
                        patch_path.display(),
                        differing.join(", ")
                    ));
                }
            }
        }
        let changed: BTreeSet<&str> = changes.iter().map(|change| change.path.as_str()).collect();
        for path in &entry.unmarked {
            if !changed.contains(path.as_str()) {
                problems.push(format!(
                    "{MANIFEST_PATH}: {name} lists {path} as unmarked, but the patch does not change it"
                ));
            }
        }
        for change in &changes {
            if change.kind == Kind::Deleted {
                continue;
            }
            let added = change.added_lines()?;
            let added: Vec<String> = added
                .iter()
                .map(|line| String::from_utf8_lossy(line).into_owned())
                .collect();
            if added.iter().any(|line| {
                CONFLICT_MARKERS
                    .iter()
                    .any(|marker| line.starts_with(marker))
            }) {
                problems.push(format!(
                    "{name}/{}: unresolved conflict markers",
                    change.path
                ));
            }
            if !entry.unmarked.contains(&change.path)
                && !added.iter().any(|line| line.contains(MARKER))
            {
                problems.push(format!(
                    "{name}/{}: changed without a `{MARKER}` comment in the changed lines",
                    change.path
                ));
            }
        }
        Ok(problems)
    }

    /// `check`: every vendored crate is the published crate plus its exported
    /// patch, recorded consistently in the manifest, `Cargo.toml` and `Cargo.lock`.
    pub fn check(&self, out: &mut dyn Write) -> Result<()> {
        let manifest = self.manifest()?;
        let lock = self.lock()?;
        let cargo_toml = self.read_text("Cargo.toml")?;
        let mut problems = Vec::new();

        let vendored: BTreeSet<&str> = manifest.crates.iter().map(|e| e.name.as_str()).collect();
        if let Ok(entries) = fs::read_dir(self.path(THIRD_PARTY)) {
            let mut dirs: Vec<String> = entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            dirs.sort();
            for dir in dirs {
                if dir != "patches" && !vendored.contains(dir.as_str()) {
                    problems.push(format!(
                        "third_party/{dir} is not in {MANIFEST_PATH}; vendor crates with `mercury-servo-patches vendor`"
                    ));
                }
            }
        }
        if let Ok(entries) = fs::read_dir(self.path(PATCHES_DIR)) {
            for entry in entries.flatten() {
                let file = entry.file_name().to_string_lossy().into_owned();
                let stem = file.strip_suffix(".patch");
                if !stem.is_some_and(|stem| vendored.contains(stem)) {
                    problems.push(format!("{PATCHES_DIR}/{file} belongs to no vendored crate"));
                }
            }
        }

        for entry in &manifest.crates {
            let found = self
                .check_crate(entry, &lock, &cargo_toml)
                .unwrap_or_else(|error| vec![error.0]);
            if found.is_empty() {
                let changed =
                    patch::parse(&fs::read(self.patch_file(&entry.name)).unwrap_or_default())
                        .map(|changes| changes.len())
                        .unwrap_or_default();
                write_out(
                    out,
                    &format!(
                        "ok  {} {} ({changed} files changed)\n",
                        entry.name, entry.version
                    ),
                )?;
            }
            problems.extend(found);
        }

        if let Some(cargo) = &self.cargo {
            let output = Command::new(cargo)
                .args([
                    "metadata",
                    "--format-version",
                    "1",
                    "--no-deps",
                    "--locked",
                    "--offline",
                ])
                .current_dir(&self.root)
                .output();
            match output {
                Err(error) => problems.push(format!(
                    "cannot run {} metadata ({error}); is cargo on PATH?",
                    cargo.display()
                )),
                Ok(output) if !output.status.success() => problems.push(format!(
                    "cargo metadata --locked failed (is Cargo.lock up to date?): {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )),
                Ok(output) => match serde_json::from_slice(&output.stdout) {
                    Err(error) => problems.push(format!("cargo metadata: {error}")),
                    Ok(metadata) => {
                        let root =
                            fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
                        problems.extend(workspace_problems(&metadata, &root));
                    }
                },
            }
        }

        if problems.is_empty() {
            write_out(
                out,
                &format!("{} vendored crates: no findings\n", manifest.crates.len()),
            )?;
            Ok(())
        } else {
            Err(Error(problems.join("\n")))
        }
    }

    /// `upgrade <crate>@<version>` or, with `servo`, every crate from Servo's repository.
    pub fn upgrade(&self, target: UpgradeTarget, out: &mut dyn Write) -> Result<UpgradeReport> {
        let mut manifest = self.manifest()?;
        let targets: Vec<(Entry, String)> = match &target {
            UpgradeTarget::Crate(spec) => {
                let (name, version) = parse_spec(spec)?;
                vec![(self.entry(&manifest, &name)?, version)]
            }
            UpgradeTarget::Servo(version) => {
                let servo: Vec<_> = manifest
                    .crates
                    .iter()
                    .filter(|entry| entry.is_servo())
                    .map(|entry| (entry.clone(), version.clone()))
                    .collect();
                if servo.is_empty() {
                    return Err(fail!(
                        "no vendored crate comes from https://github.com/servo/servo"
                    ));
                }
                servo
            }
        };

        // Check them all before changing anything.
        let lock = self.lock()?;
        let mut prepared = Vec::new();
        for (entry, version) in targets {
            if entry.version == version {
                write_out(out, &format!("{} is already {version}\n", entry.name))?;
                continue;
            }
            let old = self.pristine(&entry.name, &entry.version, &entry.checksum)?;
            let tree = Tree::read(&self.crate_dir(&entry.name))?;
            let patch_bytes = fs::read(self.patch_file(&entry.name))
                .context(|| format!("reading {}", self.patch_file(&entry.name).display()))?;
            let changes = patch::parse(&patch_bytes)?;
            let ours = patch::apply(&old, &changes)?;
            if ours.files != tree.files {
                return Err(fail!(
                    "{}: the directory has changes that are not exported; run `mercury-servo-patches export` and commit first",
                    entry.name
                ));
            }
            let checksum = match lock.registry_checksum(&entry.name, &version) {
                Some(checksum) => checksum.to_owned(),
                None => self.registry.index_checksum(&entry.name, &version)?,
            };
            let new = self.pristine(&entry.name, &version, &checksum)?;
            prepared.push((entry, version, checksum, old, ours, new, changes));
        }

        let mut report = UpgradeReport::default();
        for (entry, version, checksum, old, ours, new, changes) in prepared {
            let (merged, files) = merge_trees(&old, &ours, &new, &changes);
            let dir = self.crate_dir(&entry.name);
            for item in fs::read_dir(&dir)
                .context(|| format!("reading {}", dir.display()))?
                .flatten()
            {
                let file = item.file_name();
                if file == NOTES_FILE || file == "target" {
                    continue;
                }
                let path = item.path();
                if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                }
                .context(|| format!("removing {}", path.display()))?;
            }
            merged.write(&dir)?;
            let source = source_of(&new, &entry.name)?;
            manifest.upsert(Entry {
                version: version.clone(),
                checksum,
                repository: source.repository.or(entry.repository.clone()),
                commit: source.commit,
                path: source.path,
                ..entry.clone()
            });

            write_out(
                out,
                &format!("{} {} -> {version}\n", entry.name, entry.version),
            )?;
            if files.is_empty() {
                write_out(out, "  no Mercury changes\n")?;
            }
            for (path, outcome) in &files {
                write_out(out, &format!("  {path}: {}\n", outcome.describe()))?;
            }
            report.crates.push(UpgradedCrate {
                name: entry.name.clone(),
                from: entry.version.clone(),
                to: version,
                files,
            });
        }
        self.save_manifest(&manifest)?;

        if !report.crates.is_empty() {
            let servo_hint = match &target {
                UpgradeTarget::Servo(version) => {
                    format!("  - set the root Cargo.toml's `servo` requirement to \"={version}\"\n")
                }
                UpgradeTarget::Crate(_) => String::new(),
            };
            let names: Vec<&str> = report
                .crates
                .iter()
                .map(|upgraded| upgraded.name.as_str())
                .collect();
            write_out(
                out,
                &format!(
                    "next:\n\
                     {servo_hint}  - resolve the conflicts (search for <<<<<<<) and check each GONE file\n\
                     \x20 - run `mercury-servo-patches export` and update each MERCURY.md\n\
                     \x20 - run `cargo update {}` and `make servo-patches-check`\n",
                    names
                        .iter()
                        .map(|name| format!("-p {name}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            )?;
        }
        Ok(report)
    }
}

pub enum UpgradeTarget {
    Crate(String),
    Servo(String),
}
