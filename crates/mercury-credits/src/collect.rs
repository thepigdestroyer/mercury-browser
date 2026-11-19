// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Deciding what to credit: for every crate of the app, which licences it is used under, which
//! texts are shown for them, and what cannot be credited at all.
//!
//! The steps for a crate that is not Mercury's own: parse its licence expression and take the
//! branches Mercury may use (`deny.toml`); find its licence files and the `[[extra]]` ones; keep
//! the texts of taken licences (a text of a licence that is not taken, such as the LGPL file of
//! `r-efi`, is left out and counted); for a taken licence that no file covers, use the standard
//! text from `LICENSES/` with a copyright line built from `credits.toml` or `Cargo.toml`.
use crate::cargo::{Metadata, Package, TreeEntry, parse_tree};
use crate::classify;
use crate::config::{Config, DenyPolicy, VendorFile, Vendored};
use crate::discover::{self, FileKind};
use crate::error::{Error, Result};
use crate::spdx::{self, Term, fold_key};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Everything `collect` needs besides the files of the repository.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    pub root: &'a Path,
    /// The output of `cargo tree`.
    pub tree: &'a str,
    pub metadata: &'a Metadata,
    pub target: &'a str,
    pub runtime_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A crate of the Mercury workspace.
    Own,
    /// A crate under `third_party/`.
    Vendored,
    /// A crate from crates.io.
    Registry,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CrateRef {
    pub name: String,
    pub version: String,
}

impl CrateRef {
    fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_owned(),
            version: version.to_owned(),
        }
    }

    pub fn label(&self) -> String {
        format!("{} {}", self.name, self.version)
    }
}

/// A crate that is credited (not Mercury's own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditedCrate {
    pub krate: CrateRef,
    pub kind: Kind,
    pub repository: Option<String>,
    /// The licence expression the crate declares.
    pub license: String,
    /// MPL-2.0 is the only taken branch, or an `AND` term: the source table lists the crate.
    pub requires_mpl: bool,
    pub vendor: Option<Vendored>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockKind {
    License,
    /// A file that says which licences apply (a bare SPDX expression, "dual-licensed under A
    /// or B", a path to another file) without being a licence text.
    Statement,
    Notice,
    Patents,
    /// A licence file whose wording the tool does not know; kept verbatim.
    Unclassified,
}

/// A crate that uses a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub krate: CrateRef,
    /// The package paths of `[[extra]]` files that hold this text.
    pub via: Vec<String>,
    /// The text is the standard text with a copyright line built by the tool.
    pub standard: bool,
    pub notes: Vec<String>,
    /// The licence the crate declares (shown for unclassified texts).
    pub declared: String,
}

/// One distinct licence text and the crates that use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// The taken licences the text covers, sorted.
    pub ids: Vec<String>,
    pub body: String,
    pub members: Vec<Member>,
}

impl Block {
    pub fn standard_only(&self) -> bool {
        self.members.iter().all(|member| member.standard)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collected {
    /// Registry and vendored crates, sorted.
    pub crates: Vec<CreditedCrate>,
    pub own: Vec<CrateRef>,
    /// Crates of the app that `[[ignore]]` leaves out, with the reason.
    pub ignored: Vec<(CrateRef, String)>,
    pub blocks: Vec<Block>,
    pub standard_text_crates: Vec<CrateRef>,
    pub workspace_version: String,
    /// The first 12 hex digits of the SHA-256 of `Cargo.lock`.
    pub lock_digest: String,
    pub notice: String,
    pub bundled: String,
    pub target: String,
    pub runtime_only: bool,
    pub report: Report,
}

impl Collected {
    pub fn registry_count(&self) -> usize {
        self.crates
            .iter()
            .filter(|krate| krate.kind == Kind::Registry)
            .count()
    }

    pub fn vendored_count(&self) -> usize {
        self.crates
            .iter()
            .filter(|krate| krate.kind == Kind::Vendored)
            .count()
    }

    /// Every credited crate, Mercury's own included.
    pub fn credited_count(&self) -> usize {
        self.crates.len() + self.own.len()
    }
}

#[derive(Default)]
struct MemberAcc {
    via: BTreeSet<String>,
    standard: bool,
    notes: BTreeSet<String>,
    declared: String,
}

#[derive(Default)]
struct BlockAcc {
    ids: BTreeSet<String>,
    members: BTreeMap<CrateRef, MemberAcc>,
}

/// The name of a licence without its exception: `Apache-2.0` for `Apache-2.0 WITH LLVM-exception`.
fn term_of_key(key: &str) -> Term {
    let mut words = key.split_whitespace();
    let id = words.next().unwrap_or_default().to_owned();
    let exception = match (words.next(), words.next()) {
        (Some(with), Some(exception)) if with.eq_ignore_ascii_case("WITH") => {
            Some(exception.to_owned())
        }
        _ => None,
    };
    Term { id, exception }
}

fn strip_email(author: &str) -> String {
    let without = match author.find('<') {
        Some(start) => &author[..start],
        None => author,
    };
    without.trim().to_owned()
}

/// The copyright line of a standard text: `[[clarify]]`, else the authors, else the crate's name.
fn copyright_line(
    package: &Package,
    clarify: Option<&crate::config::Clarify>,
    key: &str,
) -> String {
    if let Some(clarify) = clarify {
        let for_this_licence = clarify.licence.as_deref().is_none_or(|licence| {
            fold_key(licence) == fold_key(key)
                || fold_key(licence) == fold_key(&term_of_key(key).id)
        });
        if for_this_licence
            && let Some(line) = clarify
                .copyright
                .as_deref()
                .filter(|line| !line.trim().is_empty())
        {
            return line.trim().to_owned();
        }
    }
    let authors: Vec<String> = package
        .authors
        .iter()
        .map(|author| strip_email(author))
        .filter(|author| !author.is_empty())
        .collect();
    if !authors.is_empty() {
        return format!("Copyright (c) {}", authors.join(", "));
    }
    match package.repository.as_deref().filter(|url| !url.is_empty()) {
        Some(repository) => format!("Copyright (c) the {} authors ({repository})", package.name),
        None => format!("Copyright (c) the {} authors", package.name),
    }
}

struct Collector<'a> {
    root: &'a Path,
    config: Config,
    deny: DenyPolicy,
    vendor: VendorFile,
    blocks: BTreeMap<(BlockKind, String), BlockAcc>,
    report: Report,
    unclassified: BTreeSet<String>,
    left_out: BTreeMap<String, BTreeSet<String>>,
    off_list: BTreeSet<String>,
    standard_crates: BTreeSet<CrateRef>,
    native_without_extra: BTreeSet<String>,
}

impl Collector<'_> {
    fn problem(&mut self, message: String) {
        self.report.problems.push(message);
    }

    fn warn(&mut self, message: String) {
        self.report.warnings.push(message);
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        kind: BlockKind,
        ids: &[String],
        text: &str,
        krate: &CrateRef,
        declared: &str,
        via: Option<&str>,
        note: Option<&str>,
        standard: bool,
    ) {
        let body = classify::normalize(text);
        let block = self.blocks.entry((kind, body)).or_default();
        block.ids.extend(ids.iter().cloned());
        let member = block.members.entry(krate.clone()).or_default();
        member.declared = declared.to_owned();
        member.standard |= standard;
        if let Some(via) = via {
            member.via.insert(via.to_owned());
        }
        if let Some(note) = note {
            member.notes.insert(note.to_owned());
        }
    }

    /// The standard text of one licence id from `LICENSES/`, as `.txt` or `.md`.
    fn standard_file(&self, id: &str) -> Option<String> {
        ["txt", "md"].iter().find_map(|extension| {
            std::fs::read(self.root.join("LICENSES").join(format!("{id}.{extension}")))
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
    }

    /// The standard text for a taken term: the licence's, and for `WITH` the exception's too.
    /// The error names the files that are missing.
    fn standard_text(&self, key: &str) -> std::result::Result<String, String> {
        let term = term_of_key(key);
        let mut text = self
            .standard_file(&term.id)
            .ok_or_else(|| format!("LICENSES/{}.txt", term.id))?;
        if let Some(exception) = &term.exception {
            let extra = self
                .standard_file(exception)
                .ok_or_else(|| format!("LICENSES/{exception}.txt"))?;
            text.push_str("\n\n");
            text.push_str(&extra);
        }
        Ok(text)
    }

    #[allow(clippy::too_many_lines)]
    fn credit(
        &mut self,
        package: &Package,
        kind: Kind,
        vendored: Option<&Vendored>,
    ) -> Option<CreditedCrate> {
        let krate = CrateRef::new(&package.name, &package.version);
        let label = krate.label();
        let clarify = self
            .config
            .clarify_for(&package.name, &package.version)
            .cloned();
        let declared = package
            .license
            .clone()
            .or_else(|| clarify.as_ref().and_then(|clarify| clarify.licence.clone()));
        let Some(declared) = declared else {
            self.problem(format!(
                "{label}: Cargo.toml declares no `license`; name the licence in a [[clarify]] entry of credits.toml"
            ));
            return None;
        };
        let expr = match spdx::parse(&declared) {
            Ok(expr) => expr,
            Err(error) => {
                self.problem(format!("{label}: {error}"));
                return None;
            }
        };
        let taken = self.deny.taken_for(&package.name, &package.version);
        let Some(credited) = expr.credited(&taken) else {
            self.problem(format!(
                "{label}: no licence of `{declared}` is allowed by deny.toml (allow-list and exceptions)"
            ));
            return None;
        };
        let requires_mpl = expr.requires(&taken, "MPL-2.0");

        let extras = self.config.extras_for(&package.name, &package.version);
        let mut extra_paths: Vec<String> = Vec::new();
        let mut note_of_path: BTreeMap<String, String> = BTreeMap::new();
        for extra in &extras {
            for file in &extra.files {
                extra_paths.push(file.clone());
                if let Some(note) = &extra.note {
                    note_of_path.insert(discover::normalise(file), note.clone());
                }
            }
        }
        let has_extras = !extras.is_empty();
        if package.links.is_some() && !has_extras {
            self.native_without_extra.insert(label.clone());
        }
        let discovery = discover::discover(package.root(), &extra_paths);
        for missing in &discovery.missing_extras {
            self.problem(format!(
                "{label}: the [[extra]] file {missing} is not a file inside the package ({})",
                package.root().display()
            ));
        }
        for skipped in &discovery.skipped {
            self.warn(format!("{label}: {skipped}; the file is not credited"));
        }

        // Which texts are kept, and which licences they cover.
        let mut covered: BTreeSet<String> = BTreeSet::new();
        let mut unknown_accepted = false;
        for file in &discovery.files {
            let via = file.extra.then_some(file.path.as_str());
            let note = note_of_path.get(&file.path).map(String::as_str);
            match file.kind {
                FileKind::Notice => {
                    self.add(
                        BlockKind::Notice,
                        &[],
                        &file.text,
                        &krate,
                        &declared,
                        via,
                        note,
                        false,
                    );
                }
                FileKind::Patents => {
                    self.add(
                        BlockKind::Patents,
                        &[],
                        &file.text,
                        &krate,
                        &declared,
                        via,
                        note,
                        false,
                    );
                }
                FileKind::License => {
                    let found = match classify::classify(&file.text) {
                        classify::Class::Licenses(found) => found,
                        classify::Class::Statement => {
                            self.add(
                                BlockKind::Statement,
                                &[],
                                &file.text,
                                &krate,
                                &declared,
                                via,
                                note,
                                false,
                            );
                            continue;
                        }
                        classify::Class::Unknown => {
                            // A person who has read the text and listed the crate in
                            // `[accept_unclassified]` says it is the crate's licence text.
                            if self.config.accepts_unclassified(&package.name) {
                                unknown_accepted = true;
                            } else {
                                self.unclassified.insert(format!("{label}: {}", file.path));
                            }
                            self.add(
                                BlockKind::Unclassified,
                                &[],
                                &file.text,
                                &krate,
                                &declared,
                                via,
                                note,
                                false,
                            );
                            continue;
                        }
                    };
                    let kept: Vec<String> = found
                        .iter()
                        .filter_map(|id| taken.spelling(id).map(str::to_owned))
                        .collect();
                    if kept.is_empty() && file.extra {
                        // A person named this file in credits.toml because code compiled into
                        // the app carries it: it is credited whatever its licence, and the
                        // allow-list is told about it.
                        self.off_list.insert(format!(
                            "{label}: {} ({})",
                            file.path,
                            found.join(", ")
                        ));
                        self.add(
                            BlockKind::License,
                            &found,
                            &file.text,
                            &krate,
                            &declared,
                            via,
                            note,
                            false,
                        );
                        continue;
                    }
                    if kept.is_empty() {
                        self.left_out
                            .entry(found.join(", "))
                            .or_default()
                            .insert(label.clone());
                        continue;
                    }
                    covered.extend(kept.iter().map(|id| fold_key(id)));
                    self.add(
                        BlockKind::License,
                        &kept,
                        &file.text,
                        &krate,
                        &declared,
                        via,
                        note,
                        false,
                    );
                }
            }
        }

        // A term is covered by a text of its own; `A WITH exception` also by a text of `A` when
        // `A` itself is credited as an alternative, since no one then relies on the exception.
        let plain_alternatives: BTreeSet<String> = credited
            .iter()
            .filter(|key| term_of_key(key).exception.is_none())
            .map(|key| fold_key(key))
            .collect();
        let is_covered = |term: &Term| {
            unknown_accepted
                || covered.contains(&fold_key(&term.key()))
                || (term.exception.is_some()
                    && covered.contains(&fold_key(&term.id))
                    && plain_alternatives.contains(&fold_key(&term.id)))
        };
        let satisfied = expr.satisfied(&|term| taken.contains(term) && is_covered(term));
        if !satisfied {
            for key in &credited {
                if is_covered(&term_of_key(key)) {
                    continue;
                }
                match self.standard_text(key) {
                    Ok(standard) => {
                        let copyright = copyright_line(package, clarify.as_ref(), key);
                        let text = classify::with_copyright(&standard, &copyright);
                        self.add(
                            BlockKind::License,
                            std::slice::from_ref(key),
                            &text,
                            &krate,
                            &declared,
                            None,
                            None,
                            true,
                        );
                        self.standard_crates.insert(krate.clone());
                    }
                    Err(missing) => self.problem(format!(
                        "{label}: licence {key} is taken but the package has no file for it and {missing} is missing"
                    )),
                }
            }
        }

        if let Some(note) = vendored.and_then(|vendored| vendored.license_note.as_deref()) {
            // The note is shown under every text of the crate.
            for ((_, _), block) in self.blocks.iter_mut() {
                if let Some(member) = block.members.get_mut(&krate) {
                    member.notes.insert(note.to_owned());
                }
            }
        }

        Some(CreditedCrate {
            krate,
            kind,
            repository: package.repository.clone(),
            license: declared,
            requires_mpl,
            vendor: vendored.cloned(),
        })
    }
}

fn is_crates_io(source: &str) -> bool {
    matches!(
        source,
        "registry+https://github.com/rust-lang/crates.io-index" | "sparse+https://index.crates.io/"
    )
}

fn read_or_problem(root: &Path, relative: &str, report: &mut Report) -> String {
    match std::fs::read(root.join(relative)) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => {
            report.problems.push(format!(
                "{relative} cannot be read ({error}); the page needs it"
            ));
            String::new()
        }
    }
}

fn workspace_version(root: &Path, report: &mut Report) -> String {
    let read = std::fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|error| error.to_string())
        .and_then(|text| {
            text.parse::<toml::Table>()
                .map_err(|error| error.to_string())
        });
    let version = read.as_ref().ok().and_then(|table| {
        table
            .get("workspace")?
            .get("package")?
            .get("version")?
            .as_str()
            .map(str::to_owned)
    });
    version.unwrap_or_else(|| {
        let why = match read {
            Ok(_) => "has no [workspace.package] version".to_owned(),
            Err(error) => error,
        };
        report.problems.push(format!(
            "Cargo.toml: cannot find the workspace version ({why})"
        ));
        "unknown".to_owned()
    })
}

fn lock_digest(root: &Path, report: &mut Report) -> String {
    match std::fs::read(root.join("Cargo.lock")) {
        Ok(bytes) => {
            let digest = Sha256::digest(&bytes);
            digest
                .iter()
                .take(6)
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
        Err(error) => {
            report
                .problems
                .push(format!("Cargo.lock cannot be read ({error})"));
            "unknown".to_owned()
        }
    }
}

fn names_with_count(names: &BTreeSet<String>, limit: usize) -> String {
    let shown: Vec<&str> = names.iter().take(limit).map(String::as_str).collect();
    let more = names.len().saturating_sub(limit);
    if more > 0 {
        format!("{} and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

/// Works out what to credit. Problems and warnings are in `report`; an `Err` is a file the tool
/// cannot do without (`deny.toml`, `credits.toml`).
pub fn collect(inputs: &Inputs<'_>) -> Result<Collected> {
    let root = inputs.root;
    let config = Config::load(root)?;
    let deny = DenyPolicy::load(root)?;
    let vendor = VendorFile::load(root)?;
    let entries: Vec<TreeEntry> = parse_tree(inputs.tree)?;
    if entries.is_empty() {
        return Err(Error::parse("cargo tree", "the tree is empty"));
    }

    let mut collector = Collector {
        root,
        config,
        deny,
        vendor,
        blocks: BTreeMap::new(),
        report: Report::default(),
        unclassified: BTreeSet::new(),
        left_out: BTreeMap::new(),
        off_list: BTreeSet::new(),
        standard_crates: BTreeSet::new(),
        native_without_extra: BTreeSet::new(),
    };
    let workspace_version = workspace_version(root, &mut collector.report);
    let lock_digest = lock_digest(root, &mut collector.report);
    let notice = read_or_problem(root, "NOTICE", &mut collector.report);
    let bundled = read_or_problem(root, "THIRD-PARTY-NOTICES.md", &mut collector.report);

    let mut crates: Vec<CreditedCrate> = Vec::new();
    let mut own: Vec<CrateRef> = Vec::new();
    let mut ignored: Vec<(CrateRef, String)> = Vec::new();
    let mut in_app: BTreeSet<String> = BTreeSet::new();
    let mut seen_ids: BTreeSet<&str> = BTreeSet::new();

    for entry in &entries {
        let label = format!("{} {}", entry.name, entry.version);
        let Some(package) = inputs.metadata.find(entry) else {
            collector.problem(format!(
                "{label}: in the cargo tree but not in cargo metadata (the lock file and the tree disagree)"
            ));
            continue;
        };
        if !seen_ids.insert(package.id.as_str()) {
            continue;
        }
        let krate = CrateRef::new(&package.name, &package.version);
        if inputs.metadata.is_workspace_member(package) {
            own.push(krate);
            continue;
        }
        in_app.insert(package.name.clone());
        if let Some(ignore) = collector.config.ignore_for(&package.name, &package.version) {
            ignored.push((krate, ignore.reason.clone()));
            continue;
        }
        let (kind, vendored) = match package.source.as_deref() {
            None => match collector
                .vendor
                .find(&package.name, &package.version)
                .cloned()
            {
                Some(vendored) => (Kind::Vendored, Some(vendored)),
                None => {
                    collector.problem(format!(
                        "{label}: a path crate that is neither a workspace member nor listed in third_party/vendor.toml"
                    ));
                    continue;
                }
            },
            Some(source) if is_crates_io(source) => (Kind::Registry, None),
            Some(source) => {
                collector.problem(format!(
                    "{label}: comes from {source}, not crates.io; the source table cannot point to it"
                ));
                continue;
            }
        };
        if let Some(credited) = collector.credit(package, kind, vendored.as_ref()) {
            crates.push(credited);
        }
    }
    crates.sort_by(|a, b| a.krate.cmp(&b.krate));
    own.sort();
    ignored.sort();

    // Entries of credits.toml that name something that is not in the app.
    let mut stale: Vec<String> = Vec::new();
    for extra in &collector.config.extra {
        if !in_app.contains(&extra.name) {
            stale.push(format!("[[extra]] {}", extra.name));
        }
    }
    for clarify in &collector.config.clarify {
        if !in_app.contains(&clarify.name) {
            stale.push(format!("[[clarify]] {}", clarify.name));
        }
    }
    for ignore in &collector.config.ignore {
        if !in_app.contains(&ignore.name) {
            stale.push(format!("[[ignore]] {}", ignore.name));
        }
    }
    for name in &collector.config.accept_unclassified.crates {
        if !in_app.contains(name) {
            stale.push(format!("[accept_unclassified] {name}"));
        }
    }
    stale.sort();
    stale.dedup();
    if !stale.is_empty() {
        collector.warn(format!(
            "credits.toml names crates that are not in the app for {}: {}",
            inputs.target,
            stale.join(", ")
        ));
    }
    for (krate, reason) in &ignored {
        collector.warn(format!(
            "{} is not credited ([[ignore]]: {reason})",
            krate.label()
        ));
    }
    if !collector.standard_crates.is_empty() {
        let names: BTreeSet<String> = collector
            .standard_crates
            .iter()
            .map(CrateRef::label)
            .collect();
        collector.warn(format!(
            "{} crates have no licence file for a taken licence and are on a standard text with a built copyright line: {}",
            names.len(),
            names.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if !collector.unclassified.is_empty() {
        let files = collector.unclassified.clone();
        collector.warn(format!(
            "{} licence files have wording the tool does not know and are kept verbatim (add the crate to [accept_unclassified] once a person has read them): {}",
            files.len(),
            files.iter().cloned().collect::<Vec<_>>().join("; ")
        ));
    }
    if !collector.left_out.is_empty() {
        let total: usize = collector.left_out.values().map(BTreeSet::len).sum();
        let groups: Vec<String> = collector
            .left_out
            .iter()
            .map(|(licence, names)| {
                format!(
                    "{licence} ({} crates: {})",
                    names.len(),
                    names_with_count(names, 8)
                )
            })
            .collect();
        collector.warn(format!(
            "{total} licence texts left out because Mercury does not use the crate under that licence: {}",
            groups.join("; ")
        ));
    }
    if !collector.off_list.is_empty() {
        let files = collector.off_list.clone();
        collector.warn(format!(
            "{} [[extra]] files hold licences that are not on deny.toml's allow-list; they are credited because a person named them: {}",
            files.len(),
            files.iter().cloned().collect::<Vec<_>>().join("; ")
        ));
    }
    if !collector.native_without_extra.is_empty() {
        collector.warn(format!(
            "{} crates link native code (`links` in Cargo.toml) and have no [[extra]] entry; check whether they bundle other licences: {}",
            collector.native_without_extra.len(),
            names_with_count(&collector.native_without_extra, 40)
        ));
    }

    let blocks: Vec<Block> = {
        let mut blocks: Vec<Block> = collector
            .blocks
            .into_iter()
            .map(|((kind, body), acc)| Block {
                kind,
                ids: acc.ids.into_iter().collect(),
                body,
                members: acc
                    .members
                    .into_iter()
                    .map(|(krate, member)| Member {
                        krate,
                        via: member.via.into_iter().collect(),
                        standard: member.standard,
                        notes: member.notes.into_iter().collect(),
                        declared: member.declared,
                    })
                    .collect(),
            })
            .collect();
        // Texts of the same licences end up together, the shipped ones before the standard ones.
        blocks.sort_by(|a, b| {
            (a.kind, a.ids.join(", "), a.standard_only(), &a.body).cmp(&(
                b.kind,
                b.ids.join(", "),
                b.standard_only(),
                &b.body,
            ))
        });
        blocks
    };

    Ok(Collected {
        crates,
        own,
        ignored,
        blocks,
        standard_text_crates: collector.standard_crates.into_iter().collect(),
        workspace_version,
        lock_digest,
        notice,
        bundled,
        target: inputs.target.to_owned(),
        runtime_only: inputs.runtime_only,
        report: collector.report,
    })
}
