// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `crx.rs`: real CRX3 files built with `crx3::Builder` and the aws-lc signers, then mutated.

mod package_common;

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use crx3::aws_lc::{EcdsaSigner, RsaSigner};
use crx3::{Builder, MAX_HEADER_LEN, Signer, key_hash};
use mercury_extensions::archive::{ArchiveError, Limits, NameProblem};
use mercury_extensions::crx::{
    CrxError, CrxPolicy, VerifiedCrx, check_manifest_key, verify_crx, verify_crx_shared,
};
use package_common::pki::sha256;
use package_common::{RawEntry, XorShift, raw_zip, zip_of};

fn archive() -> Vec<u8> {
    zip_of(&[
        (
            "manifest.json",
            br#"{"manifest_version":3,"name":"t","version":"1"}"#,
        ),
        ("background.js", b"self.onmessage = () => {};\n"),
        ("icons/16.png", &[7u8; 300]),
    ])
}

fn developer_crx() -> (Vec<u8>, RsaSigner) {
    let developer = RsaSigner::generate().unwrap();
    let crx = Builder::new().signer(&developer).build(&archive()).unwrap();
    (crx, developer)
}

fn header_end(crx: &[u8]) -> usize {
    12 + u32::from_le_bytes([crx[8], crx[9], crx[10], crx[11]]) as usize
}

// ---- what verifies ----------------------------------------------------------------------------

#[test]
fn developer_proof_alone_verifies_and_is_not_store_signed() {
    let (crx, developer) = developer_crx();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    assert!(!verified.store_signed);
    assert!(!verified.diff);
    assert_eq!(
        verified.id,
        crx3::id_from_public_key(developer.public_key())
    );
    assert_eq!(verified.public_key, developer.public_key());
}

#[test]
fn ecdsa_developer_proof_verifies() {
    let developer = EcdsaSigner::generate().unwrap();
    let crx = Builder::new().signer(&developer).build(&archive()).unwrap();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    assert_eq!(
        verified.id,
        crx3::id_from_public_key(developer.public_key())
    );
}

#[test]
fn the_id_is_thirty_two_letters_a_to_p() {
    let (crx, _) = developer_crx();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    assert_eq!(verified.id.len(), 32);
    assert!(
        verified.id.bytes().all(|b| (b'a'..=b'p').contains(&b)),
        "{}",
        verified.id
    );
}

#[test]
fn the_verified_value_owns_the_exact_bytes_and_their_hash() {
    let (crx, _) = developer_crx();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    assert_eq!(verified.bytes(), crx.as_slice());
    assert_eq!(verified.sha256.to_vec(), sha256(&crx));
}

#[test]
fn the_verified_crx_opens_the_archive_after_the_header() {
    let (crx, _) = developer_crx();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    assert_eq!(verified.archive_offset as usize, header_end(&crx));
    let package = verified.open(&Limits::default()).unwrap();
    let names: Vec<&str> = package.file_names().collect();
    assert_eq!(
        names,
        vec!["manifest.json", "background.js", "icons/16.png"]
    );
    assert_eq!(package.read_file("icons/16.png").unwrap(), vec![7u8; 300]);
}

#[test]
fn opening_applies_the_limits_to_the_archive() {
    let (crx, _) = developer_crx();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    let tight = Limits {
        max_entries: 2,
        ..Limits::default()
    };
    assert!(matches!(
        verified.open(&tight).unwrap_err(),
        ArchiveError::TooManyEntries { limit: 2, found: 3 }
    ));
}

#[test]
fn an_archive_with_an_unsafe_name_is_signed_but_still_refused_on_open() {
    let evil = raw_zip(&[RawEntry::stored("x", b"x").raw_name(b"../evil")]);
    let developer = RsaSigner::generate().unwrap();
    let crx = Builder::new().signer(&developer).build(&evil).unwrap();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    match verified.open(&Limits::default()).unwrap_err() {
        ArchiveError::UnsafeName { reason, .. } => assert_eq!(reason, NameProblem::ParentComponent),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn two_developer_style_proofs_verify_and_the_first_signer_gives_the_id() {
    let first = RsaSigner::generate().unwrap();
    let second = EcdsaSigner::generate().unwrap();
    let crx = Builder::new()
        .signer(&first)
        .signer(&second)
        .build(&archive())
        .unwrap();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    assert_eq!(verified.id, crx3::id_from_public_key(first.public_key()));
}

#[test]
fn verified_contents_are_kept_as_bytes_and_not_checked() {
    let developer = RsaSigner::generate().unwrap();
    let crx = Builder::new()
        .signer(&developer)
        .verified_contents(b"opaque compressed bytes")
        .build(&archive())
        .unwrap();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    assert_eq!(
        verified.verified_contents.as_deref(),
        Some(&b"opaque compressed bytes"[..])
    );
}

// ---- store policy -----------------------------------------------------------------------------

fn crx_with_store_proof() -> (Vec<u8>, EcdsaSigner) {
    let developer = RsaSigner::generate().unwrap();
    let store = EcdsaSigner::generate().unwrap();
    let crx = Builder::new()
        .signer(&developer)
        .signer(&store)
        .build(&archive())
        .unwrap();
    (crx, store)
}

#[test]
fn a_store_pinned_policy_without_the_store_proof_is_refused() {
    let (crx, _) = developer_crx();
    let pinned = CrxPolicy::Store {
        key_hashes: vec![key_hash(EcdsaSigner::generate().unwrap().public_key())],
    };
    assert_eq!(
        verify_crx(&crx, &pinned).unwrap_err(),
        CrxError::StoreProofMissing
    );
}

#[test]
fn a_store_pinned_policy_with_the_store_proof_marks_the_file_store_signed() {
    let (crx, store) = crx_with_store_proof();
    let pinned = CrxPolicy::Store {
        key_hashes: vec![key_hash(store.public_key())],
    };
    let verified = verify_crx(&crx, &pinned).unwrap();
    assert!(verified.store_signed);
}

#[test]
fn any_one_of_the_pinned_store_keys_is_enough() {
    let (crx, store) = crx_with_store_proof();
    let other = EcdsaSigner::generate().unwrap();
    let pinned = CrxPolicy::Store {
        key_hashes: vec![key_hash(other.public_key()), key_hash(store.public_key())],
    };
    assert!(verify_crx(&crx, &pinned).unwrap().store_signed);
}

#[test]
fn the_developer_policy_does_not_call_an_unpinned_extra_proof_a_store_proof() {
    let (crx, _) = crx_with_store_proof();
    let verified = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    assert!(!verified.store_signed);
}

#[test]
fn the_default_store_policy_pins_only_the_production_key() {
    let (crx, store) = crx_with_store_proof();
    assert_eq!(
        verify_crx(&crx, &CrxPolicy::store()).unwrap_err(),
        CrxError::StoreProofMissing
    );
    assert_eq!(
        verify_crx(&crx, &CrxPolicy::store_with_test_key()).unwrap_err(),
        CrxError::StoreProofMissing
    );
    let _ = store;
}

#[test]
fn a_store_policy_with_no_pinned_keys_refuses_everything() {
    let (crx, _) = crx_with_store_proof();
    let empty = CrxPolicy::Store {
        key_hashes: Vec::new(),
    };
    assert_eq!(
        verify_crx(&crx, &empty).unwrap_err(),
        CrxError::StoreProofMissing
    );
}

// ---- refusals ----------------------------------------------------------------------------------

#[test]
fn wrong_magic_is_refused() {
    let (mut crx, _) = developer_crx();
    crx[0] = b'X';
    assert_eq!(
        verify_crx(&crx, &CrxPolicy::Developer).unwrap_err(),
        CrxError::BadMagic
    );
}

#[test]
fn crx_version_two_and_four_are_refused() {
    for version in [2u32, 4, 0, u32::MAX] {
        let (mut crx, _) = developer_crx();
        crx[4..8].copy_from_slice(&version.to_le_bytes());
        assert_eq!(
            verify_crx(&crx, &CrxPolicy::Developer).unwrap_err(),
            CrxError::UnsupportedVersion(version)
        );
    }
}

#[test]
fn a_differential_crx_is_refused() {
    let developer = RsaSigner::generate().unwrap();
    let crx = Builder::new()
        .signer(&developer)
        .diff(true)
        .build(&archive())
        .unwrap();
    assert_eq!(
        verify_crx(&crx, &CrxPolicy::Developer).unwrap_err(),
        CrxError::Differential
    );
}

#[test]
fn a_header_length_over_the_cap_is_refused_before_anything_is_read() {
    let (mut crx, _) = developer_crx();
    let over = u32::try_from(MAX_HEADER_LEN + 1).unwrap();
    crx[8..12].copy_from_slice(&over.to_le_bytes());
    assert_eq!(
        verify_crx(&crx, &CrxPolicy::Developer).unwrap_err(),
        CrxError::HeaderTooLarge
    );
    crx[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        verify_crx(&crx, &CrxPolicy::Developer).unwrap_err(),
        CrxError::HeaderTooLarge
    );
}

#[test]
fn a_header_length_at_the_cap_with_no_such_header_is_truncated_not_a_big_allocation() {
    let (mut crx, _) = developer_crx();
    let at = u32::try_from(MAX_HEADER_LEN).unwrap();
    crx[8..12].copy_from_slice(&at.to_le_bytes());
    assert_eq!(
        verify_crx(&crx, &CrxPolicy::Developer).unwrap_err(),
        CrxError::Truncated
    );
}

#[test]
fn a_file_over_the_package_limit_is_refused() {
    let (crx, _) = developer_crx();
    let limits = Limits {
        max_package_bytes: crx.len() as u64 - 1,
        ..Limits::default()
    };
    assert!(matches!(
        verify_crx_shared(Arc::from(crx.as_slice()), &CrxPolicy::Developer, &limits).unwrap_err(),
        CrxError::PackageTooLarge { .. }
    ));
    let exact = Limits {
        max_package_bytes: crx.len() as u64,
        ..Limits::default()
    };
    assert!(verify_crx_shared(Arc::from(crx.as_slice()), &CrxPolicy::Developer, &exact).is_ok());
}

#[test]
fn every_truncation_of_a_crx_is_refused_without_panicking() {
    let (crx, _) = developer_crx();
    for length in 0..crx.len() {
        let result = verify_crx(&crx[..length], &CrxPolicy::Developer);
        assert!(result.is_err(), "length {length} was accepted");
    }
}

#[test]
fn every_truncation_up_to_the_end_of_the_header_is_truncated_or_invalid() {
    let (crx, _) = developer_crx();
    let end = header_end(&crx);
    for length in 0..=end {
        let err = verify_crx(&crx[..length], &CrxPolicy::Developer).unwrap_err();
        assert!(
            matches!(
                err,
                CrxError::Truncated | CrxError::HeaderInvalid | CrxError::SignatureInvalid
            ),
            "length {length}: {err:?}"
        );
    }
}

#[test]
fn flipping_any_byte_of_the_archive_breaks_the_signature() {
    let (crx, _) = developer_crx();
    let start = header_end(&crx);
    for index in start..crx.len() {
        let mut bad = crx.clone();
        bad[index] ^= 0x01;
        assert!(
            verify_crx(&bad, &CrxPolicy::Developer).is_err(),
            "a flip at {index} was accepted"
        );
    }
}

#[test]
fn flipping_a_byte_of_the_header_never_changes_what_verifies() {
    let (crx, _) = developer_crx();
    let good = verify_crx(&crx, &CrxPolicy::Developer).unwrap();
    for index in 0..header_end(&crx) {
        let mut bad = crx.clone();
        bad[index] ^= 0x80;
        if let Ok(verified) = verify_crx(&bad, &CrxPolicy::Developer) {
            assert_eq!(verified.id, good.id, "flip at {index} changed the id");
            assert_eq!(verified.public_key, good.public_key);
        }
    }
}

#[test]
fn flipping_a_byte_of_a_signature_is_refused() {
    let developer = EcdsaSigner::generate().unwrap();
    let crx = Builder::new().signer(&developer).build(&archive()).unwrap();
    // The ECDSA signature is DER inside the header; flipping its tail breaks it.
    let mut refused = 0;
    let end = header_end(&crx);
    for index in end.saturating_sub(120)..end {
        let mut bad = crx.clone();
        bad[index] ^= 0x55;
        if verify_crx(&bad, &CrxPolicy::Developer).is_err() {
            refused += 1;
        }
    }
    assert!(
        refused >= 100,
        "only {refused} of the flips near the signature were refused"
    );
}

#[test]
fn a_header_with_the_zip_end_marker_is_refused() {
    let (mut crx, _) = developer_crx();
    let end = header_end(&crx);
    crx[end - 4..end].copy_from_slice(b"PK\x05\x06");
    assert!(verify_crx(&crx, &CrxPolicy::Developer).is_err());
}

#[test]
fn garbage_and_tiny_inputs_are_refused() {
    for input in [
        &b""[..],
        b"Cr24",
        b"Cr24\x03\x00\x00\x00",
        b"not a crx at all, just text",
    ] {
        assert!(verify_crx(input, &CrxPolicy::Developer).is_err());
    }
}

// ---- the manifest key -------------------------------------------------------------------------

fn verified() -> (VerifiedCrx, RsaSigner) {
    let (crx, developer) = developer_crx();
    (verify_crx(&crx, &CrxPolicy::Developer).unwrap(), developer)
}

#[test]
fn a_manifest_key_that_gives_the_package_id_passes() {
    let (verified, developer) = verified();
    let key = STANDARD.encode(developer.public_key());
    assert!(check_manifest_key(&verified, Some(&key)).is_ok());
}

#[test]
fn a_manifest_key_with_line_breaks_passes() {
    let (verified, developer) = verified();
    let key = STANDARD.encode(developer.public_key());
    let wrapped: Vec<String> = key
        .as_bytes()
        .chunks(64)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    assert!(check_manifest_key(&verified, Some(&wrapped.join("\n"))).is_ok());
}

#[test]
fn no_manifest_key_passes() {
    let (verified, _) = verified();
    assert!(check_manifest_key(&verified, None).is_ok());
}

#[test]
fn a_manifest_key_for_another_extension_is_refused() {
    let (verified, _) = verified();
    let other = RsaSigner::generate().unwrap();
    let key = STANDARD.encode(other.public_key());
    assert_eq!(
        check_manifest_key(&verified, Some(&key)).unwrap_err(),
        CrxError::ManifestKeyMismatch
    );
}

#[test]
fn a_manifest_key_that_is_not_base64_or_is_empty_is_invalid() {
    let (verified, _) = verified();
    for key in ["", "   ", "!!!not base64!!!", "AAA"] {
        let err = check_manifest_key(&verified, Some(key)).unwrap_err();
        assert!(
            matches!(
                err,
                CrxError::ManifestKeyInvalid | CrxError::ManifestKeyMismatch
            ),
            "{key:?}: {err:?}"
        );
    }
    assert_eq!(
        check_manifest_key(&verified, Some("")).unwrap_err(),
        CrxError::ManifestKeyInvalid
    );
}

// ---- no panics -------------------------------------------------------------------------------

#[test]
fn a_deterministic_mutation_loop_never_panics() {
    let (crx, _) = developer_crx();
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    for _ in 0..1500 {
        let mut bytes = crx.clone();
        match rng.below(4) {
            0 => {
                for _ in 0..=rng.below(4) {
                    let i = rng.below(bytes.len());
                    bytes[i] = rng.next() as u8;
                }
            }
            1 => bytes.truncate(rng.below(bytes.len())),
            2 => {
                let i = rng.below(bytes.len());
                bytes.insert(i, rng.next() as u8);
            }
            _ => {
                let start = rng.below(bytes.len());
                let end = (start + rng.below(20)).min(bytes.len());
                bytes.drain(start..end);
            }
        }
        if let Ok(verified) = verify_crx(&bytes, &CrxPolicy::Developer) {
            let _ = verified.open(&Limits::default());
        }
    }
}
