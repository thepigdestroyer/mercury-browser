// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Install: verify, then open, then read the manifest, then (only when the user agrees) extract.
//!
//! The order is the security rule of this crate. A package is untrusted input, so:
//!
//! 1. its signature is verified first ([`crx::verify_crx`], [`xpi::verify_xpi`]); nothing in the
//!    archive is read before that succeeds, and the only archive handle that install code uses comes
//!    from the `Verified*` value;
//! 2. the archive directory is validated against the size caps and the path rules
//!    ([`ZipPackage`]);
//! 3. `manifest.json` is read through a byte cap and parsed ([`Manifest::parse`]); its id claims are
//!    checked against the verified id;
//! 4. the files the manifest points at are checked to exist;
//! 5. an [`InstallPlan`] is returned. It carries the trust level and the permission warnings for the
//!    prompt. **Nothing has been written to disk yet.**
//! 6. [`commit_install`] extracts the plan to a new directory after the user has agreed.
//!
//! The developer-mode unpacked directory ([`plan_unpacked`]) has no signature and no archive; it is
//! [`Trust::UnpackedDev`] and the shell shows the developer-mode warning.
//!
//! No clock, no network and no randomness here: `now` comes in through [`InstallOptions`].

use std::fs;
use std::io::Read;
use std::path::{Component, Path};

use crate::archive::{ArchiveError, ExtractStats, Limits, ZipPackage};
use crate::crx::{self, CrxError, CrxPolicy};
use crate::id::{ExtensionId, ExtensionKind, IdError};
use crate::manifest::{Manifest, ManifestError, ManifestLimits};
use crate::permissions::{ManifestPermissions, Warning, install_warnings};
use crate::roots::{RootsError, TrustAnchors};
use crate::xpi::{self, SignatureRequirement, ValidityPolicy, XpiError, XpiPolicy};

/// How far the package is trusted. The prompt and the badge in the interface follow this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// An XPI whose signature chains to Mozilla's add-ons root.
    MozillaSigned,
    /// A CRX that carries a valid proof by a pinned store publisher key.
    StoreSigned,
    /// A CRX with a developer proof only: the file is intact and the id is the developer key's, but
    /// nobody vouches for the developer. The shell shows a warning (OD-28: `.crx` drag-and-drop).
    DeveloperSigned,
    /// A directory picked in developer mode: no signature at all.
    UnpackedDev,
}

/// A problem found in a package that does not stop the install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallWarning {
    /// A file the manifest names (an icon, a popup page and so on) is not in the package.
    MissingFile(String),
    /// The manifest parse produced a warning (its text names a key, never a value).
    Manifest(String),
}

/// Why an install was refused. No variant holds file contents.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// The CRX signature or header was refused.
    #[error("the CRX was refused: {0}")]
    Crx(#[from] CrxError),
    /// The XPI signature or coverage was refused.
    #[error("the XPI was refused: {0}")]
    Xpi(#[from] XpiError),
    /// The archive was refused.
    #[error("the archive was refused: {0}")]
    Archive(#[from] ArchiveError),
    /// The manifest was refused.
    #[error("the manifest was refused: {0}")]
    Manifest(#[from] ManifestError),
    /// Mozilla's bundled certificates could not be loaded (a build problem).
    #[error("the bundled trust anchors are unusable: {0}")]
    Roots(#[from] RootsError),
    /// An id was not valid.
    #[error("the extension id is not valid: {0}")]
    Id(#[from] IdError),
    /// The package has no `manifest.json`.
    #[error("the package has no manifest.json")]
    NoManifest,
    /// The manifest names an id that is not the verified id.
    #[error("the manifest's id is not the id the signature proves")]
    IdMismatch,
    /// A file the extension cannot run without is not in the package.
    #[error("a file the manifest needs is missing from the package: {0}")]
    MissingEssentialFile(String),
    /// The unpacked directory is not usable (not a directory, a link, outside the folder, unreadable).
    #[error("the folder is not a usable unpacked extension: {0}")]
    BadFolder(&'static str),
    /// The plan has no archive to extract (an unpacked directory is used in place).
    #[error("this install has no archive to extract")]
    NothingToExtract,
}

/// What an install needs besides the bytes.
#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// The archive caps.
    pub limits: Limits,
    /// The manifest caps.
    pub manifest_limits: ManifestLimits,
    /// The time certificates are checked at, in seconds since the Unix epoch. The caller reads the
    /// clock, never this crate.
    pub now: i64,
    /// Where an XPI chain must end. Defaults to Mozilla's add-ons root; tests replace it.
    pub trust: TrustAnchors,
    /// Certificate dates. Defaults to Firefox's behaviour (see [`ValidityPolicy::IgnoreExpiry`]): the
    /// end-entity certificates Mozilla signs add-ons with are short-lived, and an add-on must not stop
    /// installing because its signing certificate has expired since.
    pub validity: ValidityPolicy,
}

impl InstallOptions {
    /// Default caps, Mozilla's roots and Firefox's validity rule.
    pub fn new(now: i64) -> Result<InstallOptions, InstallError> {
        Ok(InstallOptions {
            limits: Limits::default(),
            manifest_limits: ManifestLimits::default(),
            now,
            trust: TrustAnchors::mozilla()?,
            validity: ValidityPolicy::IgnoreExpiry,
        })
    }
}

/// A verified package, read and checked, not yet written anywhere.
pub struct InstallPlan {
    /// The extension id: proven by the signature for a package, taken from the manifest for an
    /// unpacked folder (`None` there when the manifest names none; the shell then makes one).
    pub id: Option<ExtensionId>,
    /// Chrome or Firefox flavour.
    pub kind: ExtensionKind,
    /// How far it is trusted.
    pub trust: Trust,
    /// The parsed manifest.
    pub manifest: Manifest,
    /// Things that do not stop the install.
    pub warnings: Vec<InstallWarning>,
    /// The lines of the permission prompt.
    pub permission_warnings: Vec<Warning>,
    package: Option<ZipPackage>,
}

impl std::fmt::Debug for InstallPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallPlan")
            .field("id", &self.id)
            .field("trust", &self.trust)
            .field("name", &self.manifest.name)
            .finish_non_exhaustive()
    }
}

/// Verifies a CRX3 file, reads its manifest and returns the plan. Writes nothing.
pub fn plan_crx(
    bytes: &[u8],
    policy: &CrxPolicy,
    options: &InstallOptions,
) -> Result<InstallPlan, InstallError> {
    let verified = crx::verify_crx(bytes, policy)?;
    let package = verified.open(&options.limits)?;
    let manifest = read_manifest(&package, options)?;
    crx::check_manifest_key(&verified, manifest.key.as_deref())?;
    let id = ExtensionId::parse(&verified.id)?;
    let trust = if verified.store_signed {
        Trust::StoreSigned
    } else {
        Trust::DeveloperSigned
    };
    finish(Some(id), trust, manifest, Some(package))
}

/// Verifies an XPI (COSE or PKCS#7 against the trust anchors), reads its manifest and returns the plan.
/// Writes nothing.
pub fn plan_xpi(bytes: &[u8], options: &InstallOptions) -> Result<InstallPlan, InstallError> {
    let mut policy = XpiPolicy::new(options.trust.clone(), options.now);
    policy.limits = options.limits.clone();
    policy.require = SignatureRequirement::ByManifestVersion;
    policy.validity = options.validity;
    let verified = xpi::verify_xpi(bytes, &policy)?;
    let package = verified.open(&options.limits)?;
    let manifest = read_manifest(&package, options)?;
    let id = ExtensionId::parse(&verified.addon_id)?;
    if let Some(claimed) = manifest.gecko_id()
        && claimed != &id
    {
        return Err(InstallError::IdMismatch);
    }
    finish(Some(id), Trust::MozillaSigned, manifest, Some(package))
}

/// Reads an unpacked extension folder (developer mode). Nothing is verified, nothing is copied: the
/// folder is used in place. Links, files outside the folder and oversized manifests are refused.
pub fn plan_unpacked(dir: &Path, options: &InstallOptions) -> Result<InstallPlan, InstallError> {
    let root = fs::canonicalize(dir).map_err(|_| InstallError::BadFolder("cannot open it"))?;
    if !root.is_dir() {
        return Err(InstallError::BadFolder("not a directory"));
    }
    let bytes = read_regular_file(&root, "manifest.json", options.manifest_limits.max_bytes)?
        .ok_or(InstallError::NoManifest)?;
    let manifest = Manifest::parse(&bytes, &options.manifest_limits)?;
    let mut missing = Vec::new();
    for reference in manifest.all_file_references() {
        if !folder_has_file(&root, &reference) {
            missing.push(reference);
        }
    }
    let id = manifest.key_id().or_else(|| manifest.gecko_id().cloned());
    let mut plan = finish(id, Trust::UnpackedDev, manifest, None)?;
    classify_missing(&mut plan, missing)?;
    Ok(plan)
}

/// Extracts the plan's archive to `dest`, which must not exist or be empty. Call it only after the
/// user has agreed to the prompt. Removes everything it wrote when it fails.
pub fn commit_install(plan: &InstallPlan, dest: &Path) -> Result<ExtractStats, InstallError> {
    let package = plan
        .package
        .as_ref()
        .ok_or(InstallError::NothingToExtract)?;
    Ok(package.extract_to(dest)?)
}

fn read_manifest(package: &ZipPackage, options: &InstallOptions) -> Result<Manifest, InstallError> {
    if !package.contains("manifest.json") {
        return Err(InstallError::NoManifest);
    }
    let cap = u64::try_from(options.manifest_limits.max_bytes).unwrap_or(u64::MAX);
    let bytes = package.read_file_capped("manifest.json", cap)?;
    Ok(Manifest::parse(&bytes, &options.manifest_limits)?)
}

fn finish(
    id: Option<ExtensionId>,
    trust: Trust,
    manifest: Manifest,
    package: Option<ZipPackage>,
) -> Result<InstallPlan, InstallError> {
    let mut warnings: Vec<InstallWarning> = manifest
        .warnings
        .iter()
        .map(|w| InstallWarning::Manifest(w.to_string()))
        .collect();
    let mut plan_missing = Vec::new();
    if let Some(package) = &package {
        for reference in manifest.all_file_references() {
            if !package.contains(&reference) {
                plan_missing.push(reference);
            }
        }
    }
    let permission_warnings = install_warnings(
        &ManifestPermissions::from(&manifest).warning_set(),
        manifest.kind,
    );
    let kind = manifest.kind;
    let mut plan = InstallPlan {
        id,
        kind,
        trust,
        manifest,
        warnings: Vec::new(),
        permission_warnings,
        package,
    };
    plan.warnings.append(&mut warnings);
    classify_missing(&mut plan, plan_missing)?;
    Ok(plan)
}

/// A missing file the extension cannot start without is an error; any other is a warning.
fn classify_missing(plan: &mut InstallPlan, missing: Vec<String>) -> Result<(), InstallError> {
    let essential = essential_files(&plan.manifest);
    for path in missing {
        if essential.contains(&path) {
            return Err(InstallError::MissingEssentialFile(path));
        }
        plan.warnings.push(InstallWarning::MissingFile(path));
    }
    Ok(())
}

fn essential_files(manifest: &Manifest) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(background) = &manifest.background {
        out.extend(background.service_worker.iter().cloned());
        out.extend(background.page.iter().cloned());
        out.extend(background.scripts.iter().cloned());
    }
    for script in &manifest.content_scripts {
        out.extend(script.js.iter().cloned());
        out.extend(script.css.iter().cloned());
    }
    out.extend(manifest.dnr_rule_resources.iter().map(|r| r.path.clone()));
    out
}

/// `name` below `root` as a regular file: no link on the way, not outside `root`, at most `cap` bytes.
/// `Ok(None)` when it does not exist.
fn read_regular_file(root: &Path, name: &str, cap: usize) -> Result<Option<Vec<u8>>, InstallError> {
    let Some(path) = checked_join(root, name) else {
        return Err(InstallError::BadFolder("a path leaves the folder"));
    };
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(InstallError::BadFolder("cannot read a file")),
    };
    if !meta.file_type().is_file() {
        return Err(InstallError::BadFolder(
            "manifest.json is not a regular file",
        ));
    }
    let canonical =
        fs::canonicalize(&path).map_err(|_| InstallError::BadFolder("cannot read a file"))?;
    if !canonical.starts_with(root) {
        return Err(InstallError::BadFolder("a path leaves the folder"));
    }
    let file =
        fs::File::open(&canonical).map_err(|_| InstallError::BadFolder("cannot read a file"))?;
    let mut bytes = Vec::new();
    let limit = u64::try_from(cap).unwrap_or(u64::MAX).saturating_add(1);
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| InstallError::BadFolder("cannot read a file"))?;
    if bytes.len() > cap {
        return Err(InstallError::Manifest(ManifestError::TooLarge {
            limit: cap,
        }));
    }
    Ok(Some(bytes))
}

fn folder_has_file(root: &Path, name: &str) -> bool {
    let Some(path) = checked_join(root, name) else {
        return false;
    };
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => fs::canonicalize(&path)
            .map(|canonical| canonical.starts_with(root))
            .unwrap_or(false),
        _ => false,
    }
}

/// `root` joined with a relative path of plain components only.
fn checked_join(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    let relative = Path::new(name);
    if relative.as_os_str().is_empty() {
        return None;
    }
    let mut out = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => out.push(part),
            _ => return None,
        }
    }
    Some(out)
}
