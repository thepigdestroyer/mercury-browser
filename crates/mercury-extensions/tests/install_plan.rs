// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `install.rs`: verify, then open, then read the manifest, then extract. Real CRX3 files built with
//! `crx3::Builder`, one real Mozilla-signed XPI, and unpacked folders.

mod package_common;

use crx3::Builder;
use crx3::aws_lc::RsaSigner;
use mercury_extensions::archive::{ArchiveError, Limits};
use mercury_extensions::crx::{CrxError, CrxPolicy};
use mercury_extensions::id::ExtensionKind;
use mercury_extensions::install::{
    InstallError, InstallOptions, InstallWarning, Trust, commit_install, plan_crx, plan_unpacked,
    plan_xpi,
};
use mercury_extensions::xpi::XpiError;
use package_common::{RawEntry, XorShift, raw_zip, temp_dir, zip_of};

const XPI: &[u8] = include_bytes!("fixtures/video-bg-play-1.8.1.xpi");
/// 2026-11-22T12:00:00Z, the day the XPI fixture was downloaded.
const FETCHED: i64 = 1_795_348_800;

const MV3: &[u8] = br#"{
  "manifest_version": 3, "name": "Test", "version": "1.2.3",
  "background": {"service_worker": "background.js"},
  "permissions": ["storage", "tabs"],
  "host_permissions": ["https://example.com/*"],
  "icons": {"16": "icons/16.png"}
}"#;

fn options() -> InstallOptions {
    InstallOptions::new(FETCHED).unwrap()
}

fn archive(manifest: &[u8], with_icon: bool, with_background: bool) -> Vec<u8> {
    let mut files: Vec<(&str, &[u8])> = vec![("manifest.json", manifest)];
    if with_background {
        files.push(("background.js", b"self.onmessage = () => {};\n"));
    }
    if with_icon {
        files.push(("icons/16.png", &[7u8; 300]));
    }
    zip_of(&files)
}

fn crx_of(zip: &[u8]) -> Vec<u8> {
    let developer = RsaSigner::generate().unwrap();
    Builder::new().signer(&developer).build(zip).unwrap()
}

#[test]
fn a_developer_signed_crx_is_planned_with_its_trust_level_and_warnings() {
    let crx = crx_of(&archive(MV3, true, true));
    let plan = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap();
    assert_eq!(plan.trust, Trust::DeveloperSigned);
    assert_eq!(plan.kind, ExtensionKind::Chrome);
    let id = plan.id.as_ref().unwrap();
    assert_eq!(id.as_str().len(), 32);
    assert!(id.as_str().bytes().all(|b| (b'a'..=b'p').contains(&b)));
    assert_eq!(plan.manifest.name, "Test");
    assert!(!plan.permission_warnings.is_empty());
    assert!(plan.warnings.is_empty());
}

#[test]
fn planning_writes_nothing_and_commit_extracts_every_file() {
    let crx = crx_of(&archive(MV3, true, true));
    let plan = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap();
    let dir = temp_dir();
    let dest = dir.path().join("ext");
    assert!(!dest.exists());
    let stats = commit_install(&plan, &dest).unwrap();
    assert_eq!(stats.files, 3);
    assert_eq!(
        std::fs::read(dest.join("background.js")).unwrap(),
        b"self.onmessage = () => {};\n"
    );
    assert!(dest.join("icons/16.png").is_file());
}

#[test]
fn a_store_policy_refuses_a_crx_with_a_developer_proof_only() {
    let crx = crx_of(&archive(MV3, true, true));
    let error = plan_crx(&crx, &CrxPolicy::store(), &options()).unwrap_err();
    assert!(matches!(error, InstallError::Crx(_)), "{error}");
}

#[test]
fn a_crx_whose_manifest_key_is_another_developers_is_refused() {
    let other = RsaSigner::generate().unwrap();
    let key = {
        use base64::Engine as _;
        use crx3::Signer;
        base64::engine::general_purpose::STANDARD.encode(other.public_key())
    };
    let manifest = format!(
        r#"{{"manifest_version":3,"name":"T","version":"1","key":"{key}","background":{{"service_worker":"background.js"}}}}"#
    );
    let crx = crx_of(&archive(manifest.as_bytes(), false, true));
    let error = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap_err();
    assert!(
        matches!(error, InstallError::Crx(CrxError::ManifestKeyMismatch)),
        "{error}"
    );
}

#[test]
fn a_missing_background_script_is_an_error_and_a_missing_icon_is_a_warning() {
    let crx = crx_of(&archive(MV3, true, false));
    let error = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap_err();
    assert!(matches!(error, InstallError::MissingEssentialFile(ref p) if p == "background.js"));

    let crx = crx_of(&archive(MV3, false, true));
    let plan = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap();
    assert_eq!(
        plan.warnings,
        vec![InstallWarning::MissingFile("icons/16.png".to_string())]
    );
}

#[test]
fn a_package_without_a_manifest_is_refused() {
    let crx = crx_of(&zip_of(&[("background.js", b"x")]));
    let error = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap_err();
    assert!(matches!(error, InstallError::NoManifest), "{error}");
}

#[test]
fn an_oversized_manifest_is_refused_before_it_is_parsed() {
    let mut big = br#"{"manifest_version":3,"name":"T","version":"1","description":""#.to_vec();
    big.extend(std::iter::repeat_n(b'a', 2 << 20));
    big.extend_from_slice(b"\"}");
    let crx = crx_of(&zip_of(&[("manifest.json", &big)]));
    assert!(plan_crx(&crx, &CrxPolicy::Developer, &options()).is_err());
}

#[test]
fn a_tampered_crx_is_refused_by_the_signature_before_the_archive_is_read() {
    let mut crx = crx_of(&archive(MV3, true, true));
    let last = crx.len() - 1;
    crx[last] ^= 0xff;
    let error = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap_err();
    assert!(matches!(error, InstallError::Crx(_)), "{error}");
}

#[test]
fn a_signed_crx_with_a_traversal_path_is_refused_and_nothing_is_written() {
    let zip = raw_zip(&[
        RawEntry::stored("manifest.json", MV3),
        RawEntry::stored("../escape.txt", b"x"),
    ]);
    let crx = crx_of(&zip);
    let error = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap_err();
    assert!(
        matches!(
            error,
            InstallError::Archive(ArchiveError::UnsafeName { .. })
        ),
        "{error}"
    );
}

#[test]
fn a_zip_bomb_inside_a_signed_crx_is_refused() {
    let mut options = options();
    options.limits = Limits {
        max_entry_bytes: 1024,
        ..Limits::default()
    };
    let zip = zip_of(&[
        ("manifest.json", MV3),
        ("background.js", b"x"),
        ("big.bin", &vec![0u8; 1 << 20]),
    ]);
    let crx = crx_of(&zip);
    assert!(plan_crx(&crx, &CrxPolicy::Developer, &options).is_err());
}

#[test]
fn a_real_mozilla_signed_xpi_is_planned_against_the_bundled_root() {
    let plan = plan_xpi(XPI, &options()).unwrap();
    assert_eq!(plan.trust, Trust::MozillaSigned);
    assert_eq!(plan.kind, ExtensionKind::Firefox);
    assert_eq!(
        plan.id.as_ref().unwrap().as_str(),
        "video-bg-play@timdream.org"
    );
    let dir = temp_dir();
    let dest = dir.path().join("xpi");
    let stats = commit_install(&plan, &dest).unwrap();
    assert!(stats.files > 0);
    assert!(dest.join("manifest.json").is_file());
}

#[test]
fn an_xpi_is_refused_when_its_chain_does_not_end_at_the_trust_anchors() {
    let mut options = options();
    options.trust = mercury_extensions::roots::TrustAnchors::from_der(&[], &[]).unwrap();
    let error = plan_xpi(XPI, &options).unwrap_err();
    assert!(matches!(error, InstallError::Xpi(_)), "{error}");
}

#[test]
fn a_changed_xpi_is_refused() {
    let mut xpi = XPI.to_vec();
    let middle = xpi.len() / 2;
    xpi[middle] ^= 0x01;
    assert!(plan_xpi(&xpi, &options()).is_err());
}

#[test]
fn an_unsigned_zip_is_not_an_xpi() {
    let zip = archive(MV3, true, true);
    let error = plan_xpi(&zip, &options()).unwrap_err();
    assert!(
        matches!(error, InstallError::Xpi(XpiError::Unsigned)),
        "{error}"
    );
}

#[test]
fn an_unpacked_folder_is_planned_in_place_without_a_signature() {
    let dir = temp_dir();
    std::fs::write(dir.path().join("manifest.json"), MV3).unwrap();
    std::fs::write(dir.path().join("background.js"), b"x").unwrap();
    let plan = plan_unpacked(dir.path(), &options()).unwrap();
    assert_eq!(plan.trust, Trust::UnpackedDev);
    assert!(plan.id.is_none());
    assert_eq!(
        plan.warnings,
        vec![InstallWarning::MissingFile("icons/16.png".to_string())]
    );
    let error = commit_install(&plan, &dir.path().join("copy")).unwrap_err();
    assert!(matches!(error, InstallError::NothingToExtract));
}

#[test]
fn an_unpacked_folder_without_a_manifest_is_refused() {
    let dir = temp_dir();
    let error = plan_unpacked(dir.path(), &options()).unwrap_err();
    assert!(matches!(error, InstallError::NoManifest), "{error}");
}

#[cfg(unix)]
#[test]
fn an_unpacked_folder_whose_manifest_is_a_link_is_refused() {
    let dir = temp_dir();
    let outside = temp_dir();
    std::fs::write(outside.path().join("manifest.json"), MV3).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("manifest.json"),
        dir.path().join("manifest.json"),
    )
    .unwrap();
    let error = plan_unpacked(dir.path(), &options()).unwrap_err();
    assert!(matches!(error, InstallError::BadFolder(_)), "{error}");
}

#[cfg(unix)]
#[test]
fn an_unpacked_background_script_that_is_a_link_out_of_the_folder_counts_as_missing() {
    let dir = temp_dir();
    let outside = temp_dir();
    std::fs::write(dir.path().join("manifest.json"), MV3).unwrap();
    std::fs::write(outside.path().join("background.js"), b"x").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("background.js"),
        dir.path().join("background.js"),
    )
    .unwrap();
    let error = plan_unpacked(dir.path(), &options()).unwrap_err();
    assert!(
        matches!(error, InstallError::MissingEssentialFile(_)),
        "{error}"
    );
}

#[test]
fn an_unpacked_path_that_is_not_a_directory_is_refused() {
    let dir = temp_dir();
    let file = dir.path().join("f");
    std::fs::write(&file, b"x").unwrap();
    assert!(plan_unpacked(&file, &options()).is_err());
    assert!(plan_unpacked(&dir.path().join("missing"), &options()).is_err());
}

#[test]
fn commit_refuses_a_destination_that_is_not_empty_and_leaves_it_alone() {
    let crx = crx_of(&archive(MV3, true, true));
    let plan = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap();
    let dir = temp_dir();
    std::fs::write(dir.path().join("mine.txt"), b"keep").unwrap();
    assert!(commit_install(&plan, dir.path()).is_err());
    assert_eq!(std::fs::read(dir.path().join("mine.txt")).unwrap(), b"keep");
    assert!(!dir.path().join("manifest.json").exists());
}

#[test]
fn the_debug_form_of_a_plan_shows_no_manifest_content() {
    let crx = crx_of(&archive(MV3, true, true));
    let plan = plan_crx(&crx, &CrxPolicy::Developer, &options()).unwrap();
    let text = format!("{plan:?}");
    assert!(!text.contains("example.com"));
    assert!(!text.contains("service_worker"));
}

#[test]
fn mutated_packages_never_panic_the_planner() {
    let crx = crx_of(&archive(MV3, true, true));
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    for _ in 0..300 {
        let mut copy = crx.clone();
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(copy.len());
            copy[at] = u8::try_from(rng.below(256)).unwrap_or(0);
        }
        if rng.below(5) == 0 {
            copy.truncate(rng.below(copy.len()));
        }
        let _ = plan_crx(&copy, &CrxPolicy::Developer, &options());
    }
    let mut rng = XorShift(0x1234_5678_9abc_def1);
    for _ in 0..100 {
        let mut copy = XPI.to_vec();
        let at = rng.below(copy.len());
        copy[at] ^= 1 << rng.below(8);
        let _ = plan_xpi(&copy, &options());
    }
}
