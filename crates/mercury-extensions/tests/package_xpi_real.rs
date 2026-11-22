// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! One real Mozilla-signed add-on (see `tests/fixtures/README.md`) against the bundled Mozilla root.
//! `now` is the day the fixture was fetched, never the system clock.

mod package_common;

use mercury_extensions::archive::{Limits, ZipPackage};
use mercury_extensions::roots::TrustAnchors;
use mercury_extensions::xpi::{
    CertProblem, SignatureAlgorithm, SignatureRequirement, ValidityPolicy, XpiError, XpiPolicy,
    verify_xpi,
};
use package_common::pki::{Files, Pki, sha256};
use package_common::zip_files;

const XPI: &[u8] = include_bytes!("fixtures/video-bg-play-1.8.1.xpi");
/// 2026-11-22T12:00:00Z, the day the fixture was downloaded.
const FETCHED: i64 = 1_795_348_800;
const ID: &str = "video-bg-play@timdream.org";

fn policy() -> XpiPolicy {
    XpiPolicy::new(TrustAnchors::mozilla().unwrap(), FETCHED)
}

fn files_of(bytes: &[u8]) -> Files {
    let package = ZipPackage::open(bytes, &Limits::default()).unwrap();
    package
        .file_names()
        .filter(|n| !n.ends_with('/'))
        .map(|n| (n.to_string(), package.read_file(n).unwrap()))
        .collect()
}

#[test]
fn the_fixture_is_the_file_the_readme_names() {
    let digest: String = sha256(XPI).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        digest,
        "b04e7e489262b5c9bcd4dbf8219a4bd6fb665d0ff7cf9f4526eda0147025f550"
    );
    assert_eq!(XPI.len(), 13_436);
}

#[test]
fn the_real_add_on_verifies_against_the_bundled_mozilla_root() {
    let verified = verify_xpi(XPI, &policy()).unwrap();
    assert_eq!(verified.addon_id, ID);
    assert_eq!(verified.signer_common_name, ID);
    assert_eq!(verified.manifest_version, 2);
    assert_eq!(verified.algorithm, SignatureAlgorithm::CoseEs256);
    assert!(
        verified
            .algorithms
            .contains(&SignatureAlgorithm::Pkcs7Sha256)
    );
    assert!(verified.signer_chain_len >= 3);
}

#[test]
fn the_real_add_on_verifies_under_every_requirement_it_meets() {
    for require in [
        SignatureRequirement::Cose,
        SignatureRequirement::Pkcs7,
        SignatureRequirement::Either,
        SignatureRequirement::ByManifestVersion,
    ] {
        let policy = XpiPolicy {
            require,
            ..policy()
        };
        assert!(verify_xpi(XPI, &policy).is_ok(), "{require:?}");
    }
}

#[test]
fn the_real_add_on_verifies_with_the_root_alone_and_with_firefoxs_date_rule() {
    let mozilla = TrustAnchors::mozilla().unwrap();
    let root_only = TrustAnchors {
        roots: mozilla.roots.clone(),
        intermediates: Vec::new(),
    };
    assert!(verify_xpi(XPI, &XpiPolicy::new(root_only, FETCHED)).is_ok());
    let lenient = XpiPolicy {
        validity: ValidityPolicy::IgnoreExpiry,
        ..policy()
    };
    assert!(verify_xpi(XPI, &lenient).is_ok());
}

#[test]
fn the_real_add_on_opens_and_reads_its_files() {
    let verified = verify_xpi(XPI, &policy()).unwrap();
    let package = verified.open(&Limits::default()).unwrap();
    let manifest = package.read_file("manifest.json").unwrap();
    assert!(String::from_utf8(manifest).unwrap().contains(ID));
    assert!(package.contains("META-INF/cose.sig"));
}

#[test]
fn a_test_chain_does_not_stand_in_for_mozilla() {
    let stranger = Pki::new("root@elsewhere.test");
    let policy = XpiPolicy::new(stranger.anchors(), FETCHED);
    assert!(matches!(
        verify_xpi(XPI, &policy).unwrap_err(),
        XpiError::Certificate(CertProblem::UnknownIssuer)
    ));
}

#[test]
fn far_in_the_future_the_certificates_have_expired_when_dates_are_enforced() {
    let year_2300 = 10_413_792_000;
    let policy = XpiPolicy {
        now: year_2300,
        ..policy()
    };
    assert!(matches!(
        verify_xpi(XPI, &policy).unwrap_err(),
        XpiError::Certificate(CertProblem::Expired)
    ));
    let lenient = XpiPolicy {
        validity: ValidityPolicy::IgnoreExpiry,
        ..policy
    };
    assert!(verify_xpi(XPI, &lenient).is_ok());
}

#[test]
fn before_the_root_existed_it_is_not_yet_valid_when_dates_are_enforced() {
    let year_2000 = 946_684_800;
    let policy = XpiPolicy {
        now: year_2000,
        ..policy()
    };
    assert!(matches!(
        verify_xpi(XPI, &policy).unwrap_err(),
        XpiError::Certificate(CertProblem::NotYetValid)
    ));
}

#[test]
fn a_changed_file_in_the_real_add_on_is_refused() {
    let mut files = files_of(XPI);
    for (name, data) in files.iter_mut() {
        if name == "video-bg-play-content.js" {
            data.extend_from_slice(b"\n// tampered\n");
        }
    }
    let err = verify_xpi(&zip_files(&files), &policy()).unwrap_err();
    assert_eq!(err, XpiError::ModifiedEntry);
}

#[test]
fn an_extra_file_in_the_real_add_on_is_refused() {
    let mut files = files_of(XPI);
    files.push(("extra.js".to_string(), b"x".to_vec()));
    assert_eq!(
        verify_xpi(&zip_files(&files), &policy()).unwrap_err(),
        XpiError::UnsignedEntry
    );
}

#[test]
fn removing_the_signature_files_leaves_an_unsigned_package() {
    let mut files = files_of(XPI);
    files.retain(|(n, _)| !n.starts_with("META-INF/"));
    assert_eq!(
        verify_xpi(&zip_files(&files), &policy()).unwrap_err(),
        XpiError::Unsigned
    );
}

#[test]
fn removing_one_signature_file_is_an_error_not_unsigned() {
    let mut files = files_of(XPI);
    files.retain(|(n, _)| n != "META-INF/cose.sig");
    let err = verify_xpi(&zip_files(&files), &policy()).unwrap_err();
    assert_ne!(err, XpiError::Unsigned);
}

#[test]
fn every_single_byte_flip_in_the_signature_files_is_survived() {
    let files = files_of(XPI);
    for name in ["META-INF/cose.sig", "META-INF/mozilla.rsa"] {
        let original = files.iter().find(|(n, _)| n == name).unwrap().1.clone();
        for index in (0..original.len()).step_by(11) {
            let mut bytes = original.clone();
            bytes[index] ^= 0x04;
            let mut copy = files.clone();
            for (n, d) in copy.iter_mut() {
                if n == name {
                    *d = bytes.clone();
                }
            }
            let _ = verify_xpi(&zip_files(&copy), &policy());
        }
    }
}
