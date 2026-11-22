// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `xpi.rs` against a test PKI: there is no Mozilla signing key, so every package here is signed by a
//! root, an intermediate and an end entity made in the test. The real Mozilla add-on is in
//! `package_xpi_real.rs`.

mod package_common;

use mercury_extensions::archive::Limits;
use mercury_extensions::roots::TrustAnchors;
use mercury_extensions::xpi::{
    CertProblem, SignatureAlgorithm, SignatureRequirement, ValidityPolicy, VerifiedXpi, XpiError,
    XpiPolicy, expected_common_name, verify_xpi,
};
use package_common::pki::*;
use package_common::{XorShift, zip_files};
use rcgen::{ExtendedKeyUsagePurpose, KeyUsagePurpose};

const ID: &str = "test@mercury.test";

fn pki() -> Pki {
    Pki::new(ID)
}

fn policy(pki: &Pki) -> XpiPolicy {
    XpiPolicy::new(pki.anchors(), now())
}

fn run(files: &Files, policy: &XpiPolicy) -> Result<VerifiedXpi, XpiError> {
    verify_xpi(&zip_files(files), policy)
}

fn err(files: &Files, policy: &XpiPolicy) -> XpiError {
    run(files, policy).unwrap_err()
}

fn cose_files(pki: &Pki) -> Files {
    signed_files(pki, 3, &SignSpec::cose())
}

fn pkcs7_files(pki: &Pki) -> Files {
    signed_files(pki, 2, &SignSpec::pkcs7())
}

fn either(pki: &Pki) -> XpiPolicy {
    XpiPolicy {
        require: SignatureRequirement::Either,
        ..policy(pki)
    }
}

/// The package signed by COSE with `manifest` as `cose.manifest`.
fn cose_with_manifest(pki: &Pki, manifest: &str) -> Files {
    let mut files = base_files(&pki.cn, 3);
    let sig = cose_sig(manifest.as_bytes(), pki, &CoseOptions::default());
    files.push((
        "META-INF/cose.manifest".to_string(),
        manifest.as_bytes().to_vec(),
    ));
    files.push(("META-INF/cose.sig".to_string(), sig));
    files
}

fn digest_lines(data: &[u8]) -> String {
    format!("SHA256-Digest: {}\n", b64(&sha256(data)))
}

// ---- COSE: the good paths ------------------------------------------------------------------------

#[test]
fn a_cose_signed_package_verifies() {
    let pki = pki();
    let verified = run(&cose_files(&pki), &policy(&pki)).unwrap();
    assert_eq!(verified.addon_id, ID);
    assert_eq!(verified.algorithm, SignatureAlgorithm::CoseEs256);
    assert_eq!(verified.algorithms, vec![SignatureAlgorithm::CoseEs256]);
    assert_eq!(verified.signer_chain_len, 3);
    assert_eq!(verified.signer_common_name, ID);
    assert_eq!(verified.manifest_version, 3);
}

#[test]
fn the_verified_xpi_owns_the_bytes_and_opens_the_archive() {
    let pki = pki();
    let bytes = zip_files(&cose_files(&pki));
    let verified = verify_xpi(&bytes, &policy(&pki)).unwrap();
    assert_eq!(verified.bytes(), bytes.as_slice());
    assert_eq!(verified.sha256.to_vec(), sha256(&bytes));
    let package = verified.open(&Limits::default()).unwrap();
    assert!(package.contains("manifest.json"));
    assert_eq!(
        package.read_file("background.js").unwrap(),
        b"console.log('hi');\n"
    );
}

#[test]
fn an_untagged_cose_sign_verifies() {
    let pki = pki();
    let spec = SignSpec {
        cose: Some(CoseOptions {
            tagged: false,
            ..Default::default()
        }),
        pkcs7: None,
    };
    assert!(run(&signed_files(&pki, 3, &spec), &policy(&pki)).is_ok());
}

#[test]
fn a_standard_x5chain_in_the_signature_headers_verifies() {
    let pki = pki();
    for layout in [
        CoseLayout::X5ChainUnprotected,
        CoseLayout::X5ChainBodyProtected,
    ] {
        let spec = SignSpec {
            cose: Some(CoseOptions {
                layout,
                ..Default::default()
            }),
            pkcs7: None,
        };
        let verified = run(&signed_files(&pki, 3, &spec), &policy(&pki)).unwrap();
        assert_eq!(verified.signer_chain_len, 3, "{layout:?}");
    }
}

#[test]
fn a_known_intermediate_completes_a_chain_the_signature_leaves_short() {
    let pki = pki();
    let spec = SignSpec {
        cose: Some(CoseOptions {
            include_intermediate: false,
            ..Default::default()
        }),
        pkcs7: None,
    };
    let files = signed_files(&pki, 3, &spec);
    let known = XpiPolicy::new(pki.anchors_with_intermediate(), now());
    assert!(run(&files, &known).is_ok());
}

#[test]
fn es384_with_a_p384_key_verifies() {
    let pki = build_pki(&PkiSpec::new(ID).kind(KeyKind::Ec384));
    let spec = SignSpec {
        cose: Some(CoseOptions {
            alg_label: -35,
            ..Default::default()
        }),
        pkcs7: None,
    };
    let verified = run(&signed_files(&pki, 3, &spec), &policy(&pki)).unwrap();
    assert_eq!(verified.algorithm, SignatureAlgorithm::CoseEs384);
}

#[test]
fn a_cose_sign_with_two_good_signatures_verifies() {
    let pki = pki();
    let spec = SignSpec {
        cose: Some(CoseOptions {
            signature_count: 2,
            ..Default::default()
        }),
        pkcs7: None,
    };
    let verified = run(&signed_files(&pki, 3, &spec), &policy(&pki)).unwrap();
    assert_eq!(verified.algorithms.len(), 2);
}

#[test]
fn the_id_may_come_from_applications_gecko() {
    let pki = pki();
    let mut files = base_files(ID, 2);
    replace_file(
        &mut files,
        "manifest.json",
        format!(r#"{{"manifest_version":2,"name":"t","version":"1","applications":{{"gecko":{{"id":"{ID}"}}}}}}"#)
            .as_bytes(),
    );
    let signed = sign_package(files, &pki, &SignSpec::cose());
    assert_eq!(run(&signed, &policy(&pki)).unwrap().addon_id, ID);
}

#[test]
fn a_manifest_without_an_id_takes_the_certificate_name() {
    let pki = pki();
    let mut files = base_files(ID, 3);
    replace_file(
        &mut files,
        "manifest.json",
        br#"{"manifest_version":3,"name":"t","version":"1"}"#,
    );
    let signed = sign_package(files, &pki, &SignSpec::cose());
    assert_eq!(run(&signed, &policy(&pki)).unwrap().addon_id, ID);
}

#[test]
fn a_manifest_without_an_id_and_a_name_that_is_not_an_id_is_refused() {
    let pki = Pki::new("Not An Id");
    let mut files = base_files(ID, 3);
    replace_file(
        &mut files,
        "manifest.json",
        br#"{"manifest_version":3,"name":"t","version":"1"}"#,
    );
    let signed = sign_package(files, &pki, &SignSpec::cose());
    assert_eq!(err(&signed, &policy(&pki)), XpiError::InvalidAddonId);
}

#[test]
fn an_id_over_64_characters_is_matched_against_its_sha256_hex() {
    let id = format!("{}@long.example", "a".repeat(70));
    let pki = Pki::new(&expected_common_name(&id));
    let signed = sign_package(base_files(&id, 3), &pki, &SignSpec::cose());
    assert_eq!(run(&signed, &policy(&pki)).unwrap().addon_id, id);
    let wrong = Pki::new(&id);
    let signed = sign_package(base_files(&id, 3), &wrong, &SignSpec::cose());
    assert_eq!(err(&signed, &policy(&wrong)), XpiError::SignerMismatch);
}

// ---- COSE: refusals --------------------------------------------------------------------------------

#[test]
fn a_certificate_for_another_id_is_refused() {
    let pki = Pki::new("someone-else@example.org");
    let signed = sign_package(base_files(ID, 3), &pki, &SignSpec::cose());
    assert_eq!(err(&signed, &policy(&pki)), XpiError::SignerMismatch);
}

#[test]
fn a_missing_intermediate_is_refused() {
    let pki = pki();
    let spec = SignSpec {
        cose: Some(CoseOptions {
            include_intermediate: false,
            ..Default::default()
        }),
        pkcs7: None,
    };
    assert_eq!(
        err(&signed_files(&pki, 3, &spec), &policy(&pki)),
        XpiError::Certificate(CertProblem::UnknownIssuer)
    );
}

#[test]
fn a_chain_that_is_not_in_the_anchors_is_refused() {
    let signer = pki();
    let stranger = Pki::new("root@elsewhere.test");
    let wrong_anchors = XpiPolicy::new(stranger.anchors(), now());
    // The stranger's root has the same subject name, so the issuer is found but its key does not match.
    assert!(matches!(
        err(&cose_files(&signer), &wrong_anchors),
        XpiError::Certificate(CertProblem::UnknownIssuer | CertProblem::BadCertificateSignature)
    ));
    let renamed = Pki::new("root@elsewhere.test");
    let unrelated = XpiPolicy::new(
        TrustAnchors::from_der(std::slice::from_ref(&renamed.ee_der), &[]).unwrap(),
        now(),
    );
    assert_eq!(
        err(&cose_files(&signer), &unrelated),
        XpiError::Certificate(CertProblem::UnknownIssuer)
    );
}

#[test]
fn mozillas_real_root_does_not_trust_a_test_chain() {
    let signer = pki();
    let mozilla = XpiPolicy::new(TrustAnchors::mozilla().unwrap(), now());
    assert!(matches!(
        err(&cose_files(&signer), &mozilla),
        XpiError::Certificate(_)
    ));
}

#[test]
fn an_expired_signer_certificate_is_refused_when_dates_are_enforced() {
    let mut spec = PkiSpec::new(ID);
    spec.ee_validity = ((2020, 1, 1), (2025, 1, 1));
    let pki = build_pki(&spec);
    assert_eq!(
        err(&cose_files(&pki), &policy(&pki)),
        XpiError::Certificate(CertProblem::Expired)
    );
}

#[test]
fn a_not_yet_valid_signer_certificate_is_refused_when_dates_are_enforced() {
    let mut spec = PkiSpec::new(ID);
    spec.ee_validity = ((2027, 1, 1), (2030, 1, 1));
    let pki = build_pki(&spec);
    assert_eq!(
        err(&cose_files(&pki), &policy(&pki)),
        XpiError::Certificate(CertProblem::NotYetValid)
    );
}

#[test]
fn expired_and_not_yet_valid_intermediates_and_roots_are_refused() {
    for (set, expected) in [
        (0, CertProblem::Expired),
        (1, CertProblem::Expired),
        (2, CertProblem::NotYetValid),
        (3, CertProblem::NotYetValid),
    ] {
        let mut spec = PkiSpec::new(ID);
        match set {
            0 => spec.inter_validity = ((2020, 1, 1), (2025, 1, 1)),
            1 => spec.root_validity = ((2020, 1, 1), (2025, 1, 1)),
            2 => spec.inter_validity = ((2027, 1, 1), (2080, 1, 1)),
            _ => spec.root_validity = ((2027, 1, 1), (2090, 1, 1)),
        }
        let pki = build_pki(&spec);
        assert_eq!(
            err(&cose_files(&pki), &policy(&pki)),
            XpiError::Certificate(expected),
            "case {set}"
        );
    }
}

#[test]
fn firefoxs_behaviour_ignores_certificate_dates() {
    let mut spec = PkiSpec::new(ID);
    spec.ee_validity = ((2020, 1, 1), (2025, 1, 1));
    let pki = build_pki(&spec);
    let lenient = XpiPolicy {
        validity: ValidityPolicy::IgnoreExpiry,
        ..policy(&pki)
    };
    assert!(run(&cose_files(&pki), &lenient).is_ok());
    let mut early = PkiSpec::new(ID);
    early.ee_validity = ((2027, 1, 1), (2030, 1, 1));
    let pki = build_pki(&early);
    let lenient = XpiPolicy {
        validity: ValidityPolicy::IgnoreExpiry,
        ..policy(&pki)
    };
    assert!(run(&cose_files(&pki), &lenient).is_ok());
}

#[test]
fn certificate_validity_is_checked_at_the_policy_time_not_the_clock() {
    let pki = pki();
    let files = cose_files(&pki);
    let before = XpiPolicy {
        now: date_unix(2024, 1, 1),
        ..policy(&pki)
    };
    assert_eq!(
        err(&files, &before),
        XpiError::Certificate(CertProblem::NotYetValid)
    );
    let inside = XpiPolicy {
        now: date_unix(2029, 12, 31),
        ..policy(&pki)
    };
    assert!(run(&files, &inside).is_ok());
    let after = XpiPolicy {
        now: date_unix(2030, 1, 2),
        ..policy(&pki)
    };
    assert_eq!(
        err(&files, &after),
        XpiError::Certificate(CertProblem::Expired)
    );
}

fn date_unix(y: i32, m: u8, d: u8) -> i64 {
    rcgen::date_time_ymd(y, m, d).unix_timestamp()
}

#[test]
fn a_changed_file_is_refused() {
    let pki = pki();
    let mut files = cose_files(&pki);
    replace_file(&mut files, "background.js", b"evil();\n");
    assert_eq!(err(&files, &policy(&pki)), XpiError::ModifiedEntry);
}

#[test]
fn an_extra_file_is_refused() {
    let pki = pki();
    let mut files = cose_files(&pki);
    files.push(("extra.js".to_string(), b"x".to_vec()));
    assert_eq!(err(&files, &policy(&pki)), XpiError::UnsignedEntry);
}

#[test]
fn a_missing_file_is_refused() {
    let pki = pki();
    let mut files = cose_files(&pki);
    remove_file(&mut files, "background.js");
    assert_eq!(err(&files, &policy(&pki)), XpiError::MissingEntry);
}

#[test]
fn a_changed_manifest_json_is_refused() {
    let pki = pki();
    let mut files = cose_files(&pki);
    replace_file(
        &mut files,
        "manifest.json",
        &manifest_json("evil@example.org", 3),
    );
    assert_eq!(err(&files, &policy(&pki)), XpiError::ModifiedEntry);
}

#[test]
fn a_changed_cose_manifest_breaks_the_signature() {
    let pki = pki();
    let mut files = cose_files(&pki);
    let mut manifest = file(&files, "META-INF/cose.manifest").to_vec();
    manifest.extend_from_slice(b"\n");
    replace_file(&mut files, "META-INF/cose.manifest", &manifest);
    assert_eq!(err(&files, &policy(&pki)), XpiError::BadSignature);
}

#[test]
fn a_flipped_signature_byte_is_refused() {
    let pki = pki();
    let mut files = cose_files(&pki);
    let mut sig = file(&files, "META-INF/cose.sig").to_vec();
    let last = sig.len() - 1;
    sig[last] ^= 0x01;
    replace_file(&mut files, "META-INF/cose.sig", &sig);
    assert_eq!(err(&files, &policy(&pki)), XpiError::BadSignature);
}

#[test]
fn every_truncation_of_cose_sig_is_refused() {
    let pki = pki();
    let files = cose_files(&pki);
    let sig = file(&files, "META-INF/cose.sig").to_vec();
    let policy = policy(&pki);
    let mut step = 0;
    for length in 0..sig.len() {
        step += 1;
        if length > 40 && step % 9 != 0 {
            continue;
        }
        let mut copy = files.clone();
        replace_file(&mut copy, "META-INF/cose.sig", &sig[..length]);
        assert!(run(&copy, &policy).is_err(), "length {length}");
    }
}

#[test]
fn cose_sig_garbage_is_malformed() {
    let pki = pki();
    for garbage in [
        &b""[..],
        b"\x00",
        b"\xd8\x62",
        b"not cbor at all",
        &[0xFFu8; 64],
    ] {
        let mut files = cose_files(&pki);
        replace_file(&mut files, "META-INF/cose.sig", garbage);
        assert_eq!(err(&files, &policy(&pki)), XpiError::MalformedCose);
    }
}

#[test]
fn unsupported_cose_algorithms_are_refused() {
    let pki = pki();
    for label in [-37i64, -8, -257, 5, 0] {
        let spec = SignSpec {
            cose: Some(CoseOptions {
                alg_label: label,
                ..Default::default()
            }),
            pkcs7: None,
        };
        assert_eq!(
            err(&signed_files(&pki, 3, &spec), &policy(&pki)),
            XpiError::UnsupportedAlgorithm,
            "alg {label}"
        );
    }
    let spec = SignSpec {
        cose: Some(CoseOptions {
            omit_alg: true,
            ..Default::default()
        }),
        pkcs7: None,
    };
    assert_eq!(
        err(&signed_files(&pki, 3, &spec), &policy(&pki)),
        XpiError::UnsupportedAlgorithm
    );
}

#[test]
fn an_algorithm_that_does_not_match_the_key_curve_is_refused() {
    let pki = pki();
    let spec = SignSpec {
        cose: Some(CoseOptions {
            alg_label: -35,
            ..Default::default()
        }),
        pkcs7: None,
    };
    assert!(run(&signed_files(&pki, 3, &spec), &policy(&pki)).is_err());
}

#[test]
fn a_signature_over_another_payload_is_refused() {
    let pki = pki();
    let spec = SignSpec {
        cose: Some(CoseOptions {
            payload_override: Some(b"other".to_vec()),
            ..Default::default()
        }),
        pkcs7: None,
    };
    assert_eq!(
        err(&signed_files(&pki, 3, &spec), &policy(&pki)),
        XpiError::BadSignature
    );
}

#[test]
fn an_embedded_payload_is_refused() {
    let pki = pki();
    let spec = SignSpec {
        cose: Some(CoseOptions {
            embed_payload: true,
            ..Default::default()
        }),
        pkcs7: None,
    };
    assert_eq!(
        err(&signed_files(&pki, 3, &spec), &policy(&pki)),
        XpiError::MalformedCose
    );
}

#[test]
fn a_chain_of_a_thousand_certificates_is_refused() {
    let pki = pki();
    let spec = SignSpec {
        cose: Some(CoseOptions {
            extra_certs: vec![pki.inter_der.clone(); 1000],
            ..Default::default()
        }),
        pkcs7: None,
    };
    assert_eq!(
        err(&signed_files(&pki, 3, &spec), &policy(&pki)),
        XpiError::Certificate(CertProblem::TooManyCertificates)
    );
}

#[test]
fn eight_certificates_are_accepted_and_nine_are_refused() {
    let pki = pki();
    // The signer's, the intermediate's, and the extras: 2 + extras.
    for (extras, ok) in [(6usize, true), (7, false)] {
        let spec = SignSpec {
            cose: Some(CoseOptions {
                extra_certs: vec![pki.inter_der.clone(); extras],
                ..Default::default()
            }),
            pkcs7: None,
        };
        let result = run(&signed_files(&pki, 3, &spec), &policy(&pki));
        assert_eq!(
            result.is_ok(),
            ok,
            "{extras} extra certificates: {result:?}"
        );
    }
}

#[test]
fn a_signature_file_over_one_mebibyte_is_refused_and_one_mebibyte_is_read() {
    let pki = pki();
    let noise = |len: usize| {
        let mut rng = XorShift(0x1234_5678_9ABC_DEF1);
        (0..len).map(|_| rng.next() as u8).collect::<Vec<u8>>()
    };
    let mut files = cose_files(&pki);
    replace_file(&mut files, "META-INF/cose.sig", &noise(1024 * 1024));
    assert_eq!(err(&files, &policy(&pki)), XpiError::MalformedCose);
    replace_file(&mut files, "META-INF/cose.sig", &noise(1024 * 1024 + 1));
    assert_eq!(err(&files, &policy(&pki)), XpiError::SignatureFileTooLarge);
}

#[test]
fn a_fifty_mebibyte_signature_file_is_refused_without_reading_it() {
    let pki = pki();
    let mut rng = XorShift(0xDEAD_BEEF_CAFE_F00D);
    let big: Vec<u8> = (0..50 * 1024 * 1024).map(|_| rng.next() as u8).collect();
    let mut files = cose_files(&pki);
    replace_file(&mut files, "META-INF/cose.sig", &big);
    assert_eq!(err(&files, &policy(&pki)), XpiError::SignatureFileTooLarge);
}

// ---- manifests -------------------------------------------------------------------------------------

fn good_manifest(pki: &Pki) -> String {
    jar_manifest(&base_files(&pki.cn, 3))
}

#[test]
fn a_hand_written_manifest_with_lowercase_attributes_verifies() {
    let pki = pki();
    let files = base_files(&pki.cn, 3);
    let mut text = String::from("Manifest-Version: 1.0\n\n");
    for (name, data) in &files {
        text.push_str(&format!(
            "name: {name}\nsha256-digest: {}\n\n",
            b64(&sha256(data))
        ));
    }
    assert!(run(&cose_with_manifest(&pki, &text), &policy(&pki)).is_ok());
}

#[test]
fn a_duplicated_manifest_section_is_refused() {
    let pki = pki();
    let files = base_files(&pki.cn, 3);
    let mut text = good_manifest(&pki);
    let (name, data) = &files[1];
    text.push_str(&format!("Name: {name}\n{}\n", digest_lines(data)));
    assert_eq!(
        err(&cose_with_manifest(&pki, &text), &policy(&pki)),
        XpiError::MalformedManifest
    );
}

#[test]
fn a_line_injection_in_the_manifest_is_refused() {
    let pki = pki();
    let files = base_files(&pki.cn, 3);
    let (name, data) = &files[1];
    // Two names in one section, a second digest in one section, and a `Magic` attribute.
    let two_names = format!(
        "Manifest-Version: 1.0\n\nName: {name}\nName: evil.js\n{}\n",
        digest_lines(data)
    );
    let two_digests = format!(
        "Manifest-Version: 1.0\n\nName: {name}\n{}{}\n",
        digest_lines(data),
        digest_lines(b"other")
    );
    let magic = format!(
        "Manifest-Version: 1.0\n\nName: {name}\n{}Magic: yes\n\n",
        digest_lines(data)
    );
    for text in [two_names, two_digests, magic] {
        assert_eq!(
            err(&cose_with_manifest(&pki, &text), &policy(&pki)),
            XpiError::MalformedManifest
        );
    }
}

#[test]
fn a_manifest_path_with_dot_dot_names_nothing_and_is_refused() {
    let pki = pki();
    let mut text = good_manifest(&pki);
    text.push_str(&format!("Name: ../outside.js\n{}\n", digest_lines(b"x")));
    assert_eq!(
        err(&cose_with_manifest(&pki, &text), &policy(&pki)),
        XpiError::MissingEntry
    );
}

#[test]
fn a_manifest_that_lists_a_signature_file_is_refused() {
    let pki = pki();
    let mut text = good_manifest(&pki);
    text.push_str(&format!(
        "Name: META-INF/cose.sig\n{}\n",
        digest_lines(b"x")
    ));
    assert_eq!(
        err(&cose_with_manifest(&pki, &text), &policy(&pki)),
        XpiError::MissingEntry
    );
}

#[test]
fn md5_and_sha1_only_digests_are_not_read_for_cose() {
    let pki = pki();
    let files = base_files(&pki.cn, 3);
    let mut md5 = String::from("Manifest-Version: 1.0\n\n");
    let mut sha1_only = String::from("Manifest-Version: 1.0\n\n");
    for (name, data) in &files {
        md5.push_str(&format!(
            "Name: {name}\nMD5-Digest: {}\n\n",
            b64(&sha1(data)[..16])
        ));
        sha1_only.push_str(&format!(
            "Name: {name}\nSHA1-Digest: {}\n\n",
            b64(&sha1(data))
        ));
    }
    for text in [md5, sha1_only] {
        assert_eq!(
            err(&cose_with_manifest(&pki, &text), &policy(&pki)),
            XpiError::MalformedManifest
        );
    }
}

#[test]
fn a_manifest_with_a_wrong_digest_length_is_refused() {
    let pki = pki();
    let files = base_files(&pki.cn, 3);
    let mut text = String::from("Manifest-Version: 1.0\n\n");
    for (name, data) in &files {
        text.push_str(&format!(
            "Name: {name}\nSHA256-Digest: {}\n\n",
            b64(&sha256(data)[..16])
        ));
    }
    assert_eq!(
        err(&cose_with_manifest(&pki, &text), &policy(&pki)),
        XpiError::MalformedManifest
    );
}

#[test]
fn a_manifest_without_the_version_line_is_refused() {
    let pki = pki();
    let text = good_manifest(&pki).replacen("Manifest-Version: 1.0\n", "", 1);
    assert_eq!(
        err(&cose_with_manifest(&pki, &text), &policy(&pki)),
        XpiError::MalformedManifest
    );
}

#[test]
fn a_manifest_with_a_nul_byte_is_refused() {
    let pki = pki();
    let mut text = good_manifest(&pki);
    text.push('\0');
    assert_eq!(
        err(&cose_with_manifest(&pki, &text), &policy(&pki)),
        XpiError::MalformedManifest
    );
}

#[test]
fn a_long_file_name_is_wrapped_over_continuation_lines_and_verifies() {
    let pki = pki();
    let name = format!("{}/{}.js", "d".repeat(100), "f".repeat(100));
    let mut files = base_files(&pki.cn, 3);
    files.push((name, b"long();\n".to_vec()));
    let signed = sign_package(files, &pki, &SignSpec::cose());
    assert!(run(&signed, &policy(&pki)).is_ok());
}

// ---- manifest.json ---------------------------------------------------------------------------------

#[test]
fn a_package_without_manifest_json_is_refused() {
    let pki = pki();
    let mut files = base_files(&pki.cn, 3);
    remove_file(&mut files, "manifest.json");
    let signed = sign_package(files, &pki, &SignSpec::cose());
    assert_eq!(err(&signed, &policy(&pki)), XpiError::ManifestUnusable);
}

#[test]
fn manifest_json_that_is_not_json_or_has_a_bad_version_is_refused() {
    let pki = pki();
    for bad in [
        &b"not json"[..],
        br#"{"manifest_version":4}"#,
        br#"{"manifest_version":"3"}"#,
        br#"{"name":"no version"}"#,
        b"[]",
    ] {
        let mut files = base_files(&pki.cn, 3);
        replace_file(&mut files, "manifest.json", bad);
        let signed = sign_package(files, &pki, &SignSpec::cose());
        assert_eq!(err(&signed, &policy(&pki)), XpiError::ManifestUnusable);
    }
}

#[test]
fn manifest_json_nested_over_64_levels_is_refused() {
    let pki = pki();
    let mut files = base_files(&pki.cn, 3);
    let nest = |n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n));
    let doc = |n: usize| format!(r#"{{"manifest_version":3,"x":{}}}"#, nest(n));
    replace_file(&mut files, "manifest.json", doc(62).as_bytes());
    let signed = sign_package(files.clone(), &pki, &SignSpec::cose());
    assert!(matches!(
        run(&signed, &policy(&pki)),
        Ok(_) | Err(XpiError::InvalidAddonId)
    ));
    replace_file(&mut files, "manifest.json", doc(64).as_bytes());
    let signed = sign_package(files, &pki, &SignSpec::cose());
    assert_eq!(err(&signed, &policy(&pki)), XpiError::ManifestUnusable);
}

#[test]
fn manifest_json_over_one_mebibyte_is_refused() {
    let pki = pki();
    let mut files = base_files(&pki.cn, 3);
    let mut rng = XorShift(7);
    let filler: String = (0..1024 * 1024)
        .map(|_| char::from(b'a' + (rng.next() % 26) as u8))
        .collect();
    let doc = format!(r#"{{"manifest_version":3,"x":"{filler}"}}"#);
    replace_file(&mut files, "manifest.json", doc.as_bytes());
    let signed = sign_package(files, &pki, &SignSpec::cose());
    assert_eq!(err(&signed, &policy(&pki)), XpiError::ManifestUnusable);
}

// ---- certificate rules ---------------------------------------------------------------------------

fn bad_cert(edit: impl FnOnce(&mut PkiSpec)) -> XpiError {
    let mut spec = PkiSpec::new(ID);
    edit(&mut spec);
    let pki = build_pki(&spec);
    err(&cose_files(&pki), &policy(&pki))
}

#[test]
fn a_signer_that_is_a_ca_is_refused() {
    assert_eq!(
        bad_cert(|s| s.ee_is_ca = true),
        XpiError::Certificate(CertProblem::NotEndEntity)
    );
}

#[test]
fn a_signer_without_code_signing_usage_is_refused() {
    assert_eq!(
        bad_cert(|s| s.ee_ekus = vec![ExtendedKeyUsagePurpose::ServerAuth]),
        XpiError::Certificate(CertProblem::ExtendedKeyUsageNotAllowed)
    );
}

#[test]
fn a_signer_with_any_extended_usage_or_none_is_accepted() {
    let any = PkiSpec {
        ee_ekus: vec![ExtendedKeyUsagePurpose::Any],
        ..PkiSpec::new(ID)
    };
    let pki = build_pki(&any);
    assert!(run(&cose_files(&pki), &policy(&pki)).is_ok());
    let none = PkiSpec {
        ee_ekus: Vec::new(),
        ..PkiSpec::new(ID)
    };
    let pki = build_pki(&none);
    assert!(run(&cose_files(&pki), &policy(&pki)).is_ok());
}

#[test]
fn a_signer_whose_key_usage_forbids_signing_is_refused() {
    assert_eq!(
        bad_cert(|s| s.ee_key_usages = vec![KeyUsagePurpose::KeyEncipherment]),
        XpiError::Certificate(CertProblem::KeyUsageNotAllowed)
    );
}

#[test]
fn a_signer_with_an_unknown_critical_extension_is_refused() {
    assert_eq!(
        bad_cert(|s| s.ee_critical_unknown_extension = true),
        XpiError::Certificate(CertProblem::UnknownCriticalExtension)
    );
}

#[test]
fn an_issuer_that_is_not_a_ca_is_refused() {
    assert_eq!(
        bad_cert(|s| s.inter_is_ca = false),
        XpiError::Certificate(CertProblem::NotACa)
    );
}

#[test]
fn an_issuer_that_may_not_sign_certificates_is_refused() {
    assert_eq!(
        bad_cert(|s| s.inter_key_usages = vec![KeyUsagePurpose::DigitalSignature]),
        XpiError::Certificate(CertProblem::KeyUsageNotAllowed)
    );
}

#[test]
fn an_issuer_restricted_to_another_purpose_is_refused() {
    assert_eq!(
        bad_cert(|s| s.inter_ekus = vec![ExtendedKeyUsagePurpose::ServerAuth]),
        XpiError::Certificate(CertProblem::ExtendedKeyUsageNotAllowed)
    );
}

#[test]
fn a_path_length_of_zero_still_allows_one_end_entity() {
    let spec = PkiSpec {
        inter_path_len: Some(0),
        ..PkiSpec::new(ID)
    };
    let pki = build_pki(&spec);
    assert!(run(&cose_files(&pki), &policy(&pki)).is_ok());
}

#[test]
fn an_rsa_signer_certificate_signs_pkcs7() {
    let pki = build_pki(&PkiSpec::new(ID).kind(KeyKind::Rsa));
    let verified = run(&pkcs7_files(&pki), &either(&pki)).unwrap();
    assert_eq!(verified.algorithm, SignatureAlgorithm::Pkcs7Sha256);
}

// ---- PKCS#7 --------------------------------------------------------------------------------------

fn pkcs7_policy(pki: &Pki) -> XpiPolicy {
    XpiPolicy {
        require: SignatureRequirement::Pkcs7,
        ..policy(pki)
    }
}

fn signed_pkcs7(pki: &Pki, options: Pkcs7Options) -> Files {
    let spec = SignSpec {
        cose: None,
        pkcs7: Some(options),
    };
    signed_files(pki, 2, &spec)
}

#[test]
fn a_pkcs7_signed_package_verifies() {
    let pki = pki();
    let verified = run(&pkcs7_files(&pki), &pkcs7_policy(&pki)).unwrap();
    assert_eq!(verified.addon_id, ID);
    assert_eq!(verified.algorithm, SignatureAlgorithm::Pkcs7Sha256);
    assert_eq!(verified.signer_chain_len, 3);
}

#[test]
fn a_pkcs7_package_with_an_rsa_signer_verifies() {
    let pki = build_pki(&PkiSpec::new(ID).kind(KeyKind::Rsa));
    assert!(run(&pkcs7_files(&pki), &pkcs7_policy(&pki)).is_ok());
}

#[test]
fn a_pkcs7_package_with_a_sha1_digest_verifies_and_says_so() {
    let pki = build_pki(&PkiSpec::new(ID).kind(KeyKind::Rsa));
    let files = signed_pkcs7(
        &pki,
        Pkcs7Options {
            digest: Pkcs7Digest::Sha1,
            ..Default::default()
        },
    );
    let verified = run(&files, &pkcs7_policy(&pki)).unwrap();
    assert_eq!(verified.algorithm, SignatureAlgorithm::Pkcs7Sha1);
}

#[test]
fn a_pkcs7_signature_without_signed_attributes_verifies() {
    let pki = pki();
    let files = signed_pkcs7(
        &pki,
        Pkcs7Options {
            signed_attributes: false,
            ..Default::default()
        },
    );
    assert!(run(&files, &pkcs7_policy(&pki)).is_ok());
}

#[test]
fn a_pkcs7_chain_completed_by_a_known_intermediate_verifies() {
    let pki = pki();
    let files = signed_pkcs7(
        &pki,
        Pkcs7Options {
            include_intermediate: false,
            ..Default::default()
        },
    );
    assert_eq!(
        err(&files, &pkcs7_policy(&pki)),
        XpiError::Certificate(CertProblem::UnknownIssuer)
    );
    let known = XpiPolicy {
        require: SignatureRequirement::Pkcs7,
        ..XpiPolicy::new(pki.anchors_with_intermediate(), now())
    };
    assert!(run(&files, &known).is_ok());
}

#[test]
fn a_tampered_message_digest_attribute_is_refused() {
    let pki = pki();
    let files = signed_pkcs7(
        &pki,
        Pkcs7Options {
            tamper_message_digest: true,
            ..Default::default()
        },
    );
    assert_eq!(err(&files, &pkcs7_policy(&pki)), XpiError::BadSignature);
}

#[test]
fn a_wrong_content_type_attribute_is_refused() {
    let pki = pki();
    let files = signed_pkcs7(
        &pki,
        Pkcs7Options {
            wrong_content_type: true,
            ..Default::default()
        },
    );
    assert_eq!(err(&files, &pkcs7_policy(&pki)), XpiError::BadSignature);
}

#[test]
fn a_signature_over_other_content_is_refused() {
    let pki = pki();
    let files = signed_pkcs7(
        &pki,
        Pkcs7Options {
            sign_other_content: true,
            ..Default::default()
        },
    );
    assert_eq!(err(&files, &pkcs7_policy(&pki)), XpiError::BadSignature);
}

#[test]
fn a_changed_file_in_a_pkcs7_package_is_refused() {
    let pki = pki();
    let mut files = pkcs7_files(&pki);
    replace_file(&mut files, "background.js", b"evil();\n");
    assert_eq!(err(&files, &pkcs7_policy(&pki)), XpiError::ModifiedEntry);
}

#[test]
fn an_extra_or_missing_file_in_a_pkcs7_package_is_refused() {
    let pki = pki();
    let mut extra = pkcs7_files(&pki);
    extra.push(("extra.js".to_string(), b"x".to_vec()));
    assert_eq!(err(&extra, &pkcs7_policy(&pki)), XpiError::UnsignedEntry);
    let mut missing = pkcs7_files(&pki);
    remove_file(&mut missing, "background.js");
    assert_eq!(err(&missing, &pkcs7_policy(&pki)), XpiError::MissingEntry);
}

#[test]
fn a_changed_signature_file_breaks_the_pkcs7_signature() {
    let pki = pki();
    let mut files = pkcs7_files(&pki);
    let mut sf = file(&files, "META-INF/mozilla.sf").to_vec();
    sf.extend_from_slice(b"\n");
    replace_file(&mut files, "META-INF/mozilla.sf", &sf);
    assert_eq!(err(&files, &pkcs7_policy(&pki)), XpiError::BadSignature);
}

#[test]
fn a_changed_manifest_does_not_match_the_signature_file_digest() {
    let pki = pki();
    let mut files = pkcs7_files(&pki);
    let mut mf = file(&files, "META-INF/manifest.mf").to_vec();
    mf.extend_from_slice(b"\n");
    replace_file(&mut files, "META-INF/manifest.mf", &mf);
    assert_eq!(
        err(&files, &pkcs7_policy(&pki)),
        XpiError::ManifestDigestMismatch
    );
}

#[test]
fn a_pkcs7_package_missing_its_signature_file_or_manifest_is_incomplete() {
    let pki = pki();
    let mut no_sf = pkcs7_files(&pki);
    remove_file(&mut no_sf, "META-INF/mozilla.sf");
    assert_eq!(
        err(&no_sf, &pkcs7_policy(&pki)),
        XpiError::IncompleteSignature
    );
    let mut no_mf = pkcs7_files(&pki);
    remove_file(&mut no_mf, "META-INF/manifest.mf");
    assert_eq!(
        err(&no_mf, &pkcs7_policy(&pki)),
        XpiError::IncompleteSignature
    );
}

#[test]
fn two_pkcs7_signature_blocks_are_ambiguous() {
    let pki = pki();
    let mut files = pkcs7_files(&pki);
    let rsa = file(&files, "META-INF/mozilla.rsa").to_vec();
    files.push(("META-INF/other.rsa".to_string(), rsa));
    assert_eq!(err(&files, &pkcs7_policy(&pki)), XpiError::MalformedPkcs7);
}

#[test]
fn pkcs7_garbage_is_malformed() {
    let pki = pki();
    for garbage in [&b""[..], b"\x30\x00", b"hello", &[0u8; 100]] {
        let mut files = pkcs7_files(&pki);
        replace_file(&mut files, "META-INF/mozilla.rsa", garbage);
        assert_eq!(err(&files, &pkcs7_policy(&pki)), XpiError::MalformedPkcs7);
    }
}

#[test]
fn every_truncation_of_the_pkcs7_block_is_refused() {
    let pki = pki();
    let files = pkcs7_files(&pki);
    let rsa = file(&files, "META-INF/mozilla.rsa").to_vec();
    let policy = pkcs7_policy(&pki);
    for length in (0..rsa.len()).step_by(7) {
        let mut copy = files.clone();
        replace_file(&mut copy, "META-INF/mozilla.rsa", &rsa[..length]);
        assert!(run(&copy, &policy).is_err(), "length {length}");
    }
}

#[test]
fn a_pkcs7_block_over_one_mebibyte_is_refused() {
    let pki = pki();
    let mut rng = XorShift(99);
    let big: Vec<u8> = (0..1024 * 1024 + 1).map(|_| rng.next() as u8).collect();
    let mut files = pkcs7_files(&pki);
    replace_file(&mut files, "META-INF/mozilla.rsa", &big);
    assert_eq!(
        err(&files, &pkcs7_policy(&pki)),
        XpiError::SignatureFileTooLarge
    );
}

// ---- policy -----------------------------------------------------------------------------------------

#[test]
fn a_package_without_signature_files_is_unsigned() {
    let pki = pki();
    assert_eq!(err(&base_files(ID, 3), &policy(&pki)), XpiError::Unsigned);
}

#[test]
fn stray_signature_files_without_a_signature_are_incomplete_not_unsigned() {
    let pki = pki();
    let mut only_manifest = base_files(ID, 3);
    only_manifest.push((
        "META-INF/cose.manifest".to_string(),
        b"Manifest-Version: 1.0\n".to_vec(),
    ));
    assert_eq!(
        err(&only_manifest, &policy(&pki)),
        XpiError::IncompleteSignature
    );
    let mut only_sf = base_files(ID, 3);
    only_sf.push((
        "META-INF/mozilla.sf".to_string(),
        b"Signature-Version: 1.0\n".to_vec(),
    ));
    assert_eq!(err(&only_sf, &policy(&pki)), XpiError::IncompleteSignature);
}

#[test]
fn cose_without_its_manifest_is_incomplete() {
    let pki = pki();
    let mut files = cose_files(&pki);
    remove_file(&mut files, "META-INF/cose.manifest");
    assert_eq!(err(&files, &policy(&pki)), XpiError::IncompleteSignature);
}

#[test]
fn cose_required_refuses_a_pkcs7_only_package() {
    let pki = pki();
    assert_eq!(
        err(&pkcs7_files(&pki), &policy(&pki)),
        XpiError::CoseRequired
    );
}

#[test]
fn pkcs7_required_refuses_a_cose_only_package() {
    let pki = pki();
    assert_eq!(
        err(&cose_files(&pki), &pkcs7_policy(&pki)),
        XpiError::Pkcs7Required
    );
}

#[test]
fn either_accepts_cose_or_pkcs7() {
    let pki = pki();
    assert!(run(&cose_files(&pki), &either(&pki)).is_ok());
    assert!(run(&pkcs7_files(&pki), &either(&pki)).is_ok());
}

#[test]
fn by_manifest_version_requires_cose_for_version_3_and_allows_pkcs7_for_version_2() {
    let pki = pki();
    let by_version = XpiPolicy {
        require: SignatureRequirement::ByManifestVersion,
        ..policy(&pki)
    };
    let v3_pkcs7 = sign_package(base_files(ID, 3), &pki, &SignSpec::pkcs7());
    assert_eq!(err(&v3_pkcs7, &by_version), XpiError::CoseRequired);
    let v3_cose = sign_package(base_files(ID, 3), &pki, &SignSpec::cose());
    assert!(run(&v3_cose, &by_version).is_ok());
    let v2_pkcs7 = sign_package(base_files(ID, 2), &pki, &SignSpec::pkcs7());
    assert!(run(&v2_pkcs7, &by_version).is_ok());
    let v2_cose = sign_package(base_files(ID, 2), &pki, &SignSpec::cose());
    assert!(run(&v2_cose, &by_version).is_ok());
}

#[test]
fn a_package_with_both_signatures_verifies_both_and_leads_with_cose() {
    let pki = pki();
    let files = signed_files(&pki, 2, &SignSpec::both());
    let verified = run(&files, &policy(&pki)).unwrap();
    assert_eq!(
        verified.algorithms,
        vec![
            SignatureAlgorithm::CoseEs256,
            SignatureAlgorithm::Pkcs7Sha256
        ]
    );
    assert_eq!(verified.algorithm, SignatureAlgorithm::CoseEs256);
}

#[test]
fn a_broken_pkcs7_block_fails_even_when_only_cose_is_required() {
    let pki = pki();
    let mut files = signed_files(&pki, 2, &SignSpec::both());
    let mut rsa = file(&files, "META-INF/mozilla.rsa").to_vec();
    let last = rsa.len() - 1;
    rsa[last] ^= 0xFF;
    replace_file(&mut files, "META-INF/mozilla.rsa", &rsa);
    assert!(run(&files, &policy(&pki)).is_err());
}

#[test]
fn a_broken_cose_signature_fails_even_when_only_pkcs7_is_required() {
    let pki = pki();
    let mut files = signed_files(&pki, 2, &SignSpec::both());
    // The PKCS#7 manifest covers cose.sig, so change it and re-sign the PKCS#7 part around the change.
    let mut sig = file(&files, "META-INF/cose.sig").to_vec();
    let last = sig.len() - 1;
    sig[last] ^= 0x01;
    remove_file(&mut files, "META-INF/mozilla.rsa");
    remove_file(&mut files, "META-INF/mozilla.sf");
    remove_file(&mut files, "META-INF/manifest.mf");
    replace_file(&mut files, "META-INF/cose.sig", &sig);
    let files = sign_package(files, &pki, &SignSpec::pkcs7());
    assert_eq!(err(&files, &pkcs7_policy(&pki)), XpiError::BadSignature);
}

#[test]
fn a_package_that_uses_the_cose_files_without_covering_them_in_pkcs7_is_refused() {
    let pki = pki();
    let mut files = signed_files(&pki, 2, &SignSpec::both());
    remove_file(&mut files, "META-INF/cose.sig");
    assert!(run(&files, &policy(&pki)).is_err());
}

// ---- no panics ------------------------------------------------------------------------------------

#[test]
fn a_mutation_loop_over_the_zip_bytes_never_panics() {
    let pki = pki();
    let base = zip_files(&signed_files(&pki, 2, &SignSpec::both()));
    let policy = policy(&pki);
    let mut rng = XorShift(0xA5A5_5A5A_0F0F_F0F0);
    for _ in 0..600 {
        let mut bytes = base.clone();
        for _ in 0..=rng.below(3) {
            let i = rng.below(bytes.len());
            bytes[i] ^= 1 << rng.below(8);
        }
        if rng.below(5) == 0 {
            bytes.truncate(rng.below(bytes.len()));
        }
        let _ = verify_xpi(&bytes, &policy);
    }
}

#[test]
fn a_mutation_loop_over_each_signature_file_never_panics() {
    let pki = pki();
    let files = signed_files(&pki, 2, &SignSpec::both());
    let policy = policy(&pki);
    let mut rng = XorShift(0x0123_4567_89AB_CDEF);
    for name in [
        "META-INF/cose.sig",
        "META-INF/cose.manifest",
        "META-INF/mozilla.rsa",
        "META-INF/mozilla.sf",
        "META-INF/manifest.mf",
    ] {
        let original = file(&files, name).to_vec();
        for _ in 0..250 {
            let mut bytes = original.clone();
            match rng.below(3) {
                0 => {
                    for _ in 0..=rng.below(3) {
                        let i = rng.below(bytes.len());
                        bytes[i] = rng.next() as u8;
                    }
                }
                1 => bytes.truncate(rng.below(bytes.len())),
                _ => {
                    let i = rng.below(bytes.len());
                    bytes.insert(i, rng.next() as u8);
                }
            }
            let mut copy = files.clone();
            replace_file(&mut copy, name, &bytes);
            let _ = run(&copy, &policy);
        }
    }
}
