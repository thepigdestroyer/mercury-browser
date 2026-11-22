// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The end-to-end proof that a passkey's private key is never written to a plain file: a passkey
//! made through the built-in authenticator on a real (temporary) Keychain file, then every file
//! under the temporary directory searched for the key in every spelling it could have.
#![cfg(target_os = "macos")]

mod common;

use std::path::Path;
use std::sync::Arc;

use common::keychain::TempKeychain;
use data_encoding::{BASE64, BASE64URL, BASE64URL_NOPAD, HEXLOWER, HEXUPPER};
use mercury_passkeys::builtin::BuiltInAuthenticator;
use mercury_passkeys::testing::FakeVerifier;
use mercury_passkeys::{
    CreateRequest, PasskeyAuthenticator, PasskeyStore, RelyingParty, ResidentKey, UserEntity,
    UserVerification,
};
use passkey::crypto::coset::{CborSerializable, CoseKey, Label, cbor::Value};
use passkey::crypto::rust_crypto::RustCryptoSecretKey;
use passkey::crypto::{Pkcs8, SecretKeyT};

/// The `d` parameter of an EC2 COSE key (RFC 9053): the private scalar.
const COSE_EC2_D: i64 = -4;

fn scalar_of(cose_key: &[u8]) -> Vec<u8> {
    let key = CoseKey::from_slice(cose_key).expect("a COSE key");
    key.params
        .iter()
        .find_map(|(label, value)| match (label, value) {
            (Label::Int(COSE_EC2_D), Value::Bytes(bytes)) => Some(bytes.clone()),
            _ => None,
        })
        .expect("an EC2 private key has a d parameter")
}

fn spellings(secret: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("raw", secret.to_vec()),
        ("hex lower", HEXLOWER.encode(secret).into_bytes()),
        ("hex upper", HEXUPPER.encode(secret).into_bytes()),
        ("base64", BASE64.encode(secret).into_bytes()),
        ("base64url", BASE64URL.encode(secret).into_bytes()),
        (
            "base64url no pad",
            BASE64URL_NOPAD.encode(secret).into_bytes(),
        ),
    ]
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn files_under(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a readable directory") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            files_under(&path, found);
        } else if path.is_file() {
            found.push(path);
        }
    }
}

fn create_request() -> CreateRequest {
    CreateRequest {
        origin: "https://example.com".to_owned(),
        rp: RelyingParty {
            id: "example.com".to_owned(),
            name: Some("Example".to_owned()),
        },
        user: UserEntity {
            id: b"user-handle-1".to_vec(),
            name: "alice".to_owned(),
            display_name: "Alice".to_owned(),
        },
        client_data_hash: [7; 32],
        algorithms: vec![-7],
        exclude: Vec::new(),
        resident_key: ResidentKey::Required,
        user_verification: UserVerification::Preferred,
        timeout_ms: None,
    }
}

#[test]
fn a_passkey_made_on_the_keychain_leaves_its_private_key_in_no_file() {
    let (guard, backend): (TempKeychain, _) = common::keychain::temp_keychain();
    let store = Arc::new(PasskeyStore::new(backend));
    let verifier = Arc::new(FakeVerifier::new());
    let authenticator =
        BuiltInAuthenticator::new(Arc::clone(&store), verifier).expect("the system RNG works");

    let created = authenticator
        .make_credential(&create_request())
        .expect("the passkey is made");

    // What the Keychain holds, read back through the store (this is the only legitimate read).
    let listed = store.all().expect("listing works");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key.credential_id, created.credential_id);
    let record = store
        .record(&listed[0].key)
        .expect("reading works")
        .expect("the passkey is stored");
    let cose_bytes = record.private_key.expose().to_vec();
    let scalar = scalar_of(&cose_bytes);
    assert_eq!(scalar.len(), 32);
    let secret_key =
        RustCryptoSecretKey::from_cose_key(&CoseKey::from_slice(&cose_bytes).expect("COSE"))
            .expect("a P-256 key");
    let pkcs8 = secret_key.to_pkcs8().expect("PKCS#8");

    let mut needles = Vec::new();
    for (name, bytes) in [
        ("scalar", &scalar),
        ("COSE_Key", &cose_bytes),
        ("PKCS#8", &pkcs8),
    ] {
        for (form, text) in spellings(bytes) {
            needles.push((format!("{name} ({form})"), text));
        }
    }

    // Also run an assertion, so the key has been read and used once.
    authenticator
        .get_assertion(&mercury_passkeys::AssertRequest {
            origin: "https://example.com".to_owned(),
            rp_id: "example.com".to_owned(),
            client_data_hash: [9; 32],
            allow: Vec::new(),
            user_verification: UserVerification::Preferred,
            timeout_ms: None,
        })
        .expect("the passkey signs");

    let mut files = Vec::new();
    files_under(guard.dir(), &mut files);
    assert!(!files.is_empty(), "the keychain file exists");
    let mut saw_site_name = false;
    for path in &files {
        let bytes = std::fs::read(path).expect("a readable file");
        saw_site_name |= contains(&bytes, b"example.com");
        for (name, needle) in &needles {
            assert!(
                !contains(&bytes, needle),
                "a file in the keychain's directory holds the private key as {name}"
            );
        }
    }
    assert!(
        saw_site_name,
        "the scan must be able to see the attributes it is allowed to find"
    );
}

#[test]
fn the_only_export_of_a_key_in_the_clear_needs_an_explicit_acknowledgement() {
    use mercury_passkeys::cxf::{ExportAcknowledgement, export_cxf};

    let (_guard, backend) = common::keychain::temp_keychain();
    let store = Arc::new(PasskeyStore::new(backend));
    let authenticator =
        BuiltInAuthenticator::new(Arc::clone(&store), Arc::new(FakeVerifier::new()))
            .expect("the system RNG works");
    authenticator
        .make_credential(&create_request())
        .expect("the passkey is made");
    let listed = store.all().expect("listing works");
    let record = store
        .record(&listed[0].key)
        .expect("reading works")
        .expect("stored");
    let cose_bytes = record.private_key.expose().to_vec();
    let scalar = scalar_of(&cose_bytes);

    // `export_cxf` cannot be called without naming the acknowledgement: the argument has no
    // default and no `From` conversion.
    let exported = export_cxf(
        &[record],
        ExportAcknowledgement::PrivateKeysInClear,
        1_800_000_000,
    )
    .expect("the export works");
    // The one legitimate place for the key in the clear: PKCS#8 as base64url text. The raw scalar
    // bytes themselves never appear.
    assert_eq!(exported.exported, 1);
    assert!(!contains(exported.as_bytes(), &scalar));
    let pkcs8 =
        RustCryptoSecretKey::from_cose_key(&CoseKey::from_slice(&cose_bytes).expect("COSE"))
            .expect("a P-256 key")
            .to_pkcs8()
            .expect("PKCS#8");
    assert!(contains(
        exported.as_bytes(),
        BASE64URL_NOPAD.encode(&pkcs8).as_bytes()
    ));
}
