// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! CXF import and export (EC46): round trips, refusals, file handling and secret hygiene.
//!
//! Test keys are made with passkey-crypto directly, not with the built-in authenticator.

use data_encoding::{BASE64, BASE64_NOPAD, BASE64URL, BASE64URL_NOPAD, HEXLOWER, HEXUPPER};
use mercury_passkeys::cxf::{
    ExportAcknowledgement, Exported, Imported, MAX_IMPORT_BYTES, MAX_IMPORT_ITEMS, SkipReason,
    export_cxf, import_cxf, read_import_file, write_export_file,
};
use mercury_passkeys::testing::{FakeAnswer, FakeVerifier};
use mercury_passkeys::{
    COSE_ES256, MemoryBackend, PasskeyBackend, PasskeyError, PasskeyKey, PasskeyRecord,
    PasskeyStore, PasskeySummary, PrivateKey,
};
use passkey::crypto::coset::cbor::Value as Cbor;
use passkey::crypto::coset::{CborSerializable, CoseKey, Label, iana};
use passkey::crypto::rust_crypto::{RustCryptoBackend, RustCryptoSecretKey};
use passkey::crypto::{CryptoBackend, Pkcs8, PublicKeyT, SecretKeyT};
use serde_json::{Value, json};

const ACK: ExportAcknowledgement = ExportAcknowledgement::PrivateKeysInClear;
const NOW: u64 = 1_800_000_000;

// A P-256 key in PKCS#8 DER, from passkey-crypto's own test vectors, base64url without padding.
const FIXTURE_KEY: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgc_lEDIXdcT6rVqztDoy2eQT0NbSZlbUVOV43KPdavX-hRANCAASft9SPPTRzh2nq-Nq9THH83NRBPbmJMlocJ9f7iAxgnN_xbiwu0XbALavCfiw2SLHe2_ukveuwTyGGCFvaEmCw";
const FIXTURE_X: &str = "9fb7d48f3d34738769eaf8dabd4c71fcdcd4413db989325a1c27d7fb880c609c";
const FIXTURE_Y: &str = "dff16e2c2ed176c02dabc27e2c3648b1dedbfba4bdebb04f2186085bda1260b0";

/// What a test key looks like in every form it could leak in.
struct Material {
    secret: RustCryptoSecretKey,
    pkcs8: Vec<u8>,
    cose: Vec<u8>,
    scalar: Vec<u8>,
}

fn new_material() -> Material {
    let secret = RustCryptoBackend
        .generate_key(iana::Algorithm::ES256)
        .expect("generate");
    let cose_key = secret.to_cose_key();
    let scalar = scalar_of(&cose_key);
    Material {
        pkcs8: secret.to_pkcs8().expect("pkcs8"),
        cose: cose_key.to_vec().expect("cbor"),
        scalar,
        secret,
    }
}

fn scalar_of(cose: &CoseKey) -> Vec<u8> {
    cose.params
        .iter()
        .find_map(|(label, value)| match (label, value) {
            (Label::Int(-4), Cbor::Bytes(bytes)) => Some(bytes.clone()),
            _ => None,
        })
        .expect("private scalar")
}

fn record_with(
    rp_id: &str,
    credential_id: &[u8],
    username: &str,
    handle: Option<&[u8]>,
) -> (PasskeyRecord, Material) {
    let material = new_material();
    let record = PasskeyRecord {
        summary: PasskeySummary {
            key: PasskeyKey {
                rp_id: rp_id.to_owned(),
                credential_id: credential_id.to_vec(),
            },
            user_handle: handle.map(<[u8]>::to_vec),
            username: username.to_owned(),
            display_name: format!("{username} display"),
            algorithm: COSE_ES256,
            counter: None,
            created_at_ms: Some(1_700_000_000_000),
            modified_at_ms: Some(1_700_000_100_000),
        },
        private_key: PrivateKey::new(material.cose.clone()),
    };
    (record, material)
}

fn export_bytes(records: &[PasskeyRecord]) -> Exported {
    export_cxf(records, ACK, NOW, &FakeVerifier::new()).expect("export")
}

fn export_json(records: &[PasskeyRecord]) -> Value {
    serde_json::from_slice(export_bytes(records).as_bytes()).expect("json")
}

fn encodings(bytes: &[u8]) -> Vec<String> {
    vec![
        HEXLOWER.encode(bytes),
        HEXUPPER.encode(bytes),
        BASE64.encode(bytes),
        BASE64_NOPAD.encode(bytes),
        BASE64URL.encode(bytes),
        BASE64URL_NOPAD.encode(bytes),
    ]
}

fn contains_any_encoding(text: &str, secret: &[u8]) -> bool {
    encodings(secret).iter().any(|form| text.contains(form))
}

// ---- hand-built files -------------------------------------------------------------------------

fn b64(bytes: &[u8]) -> String {
    BASE64URL_NOPAD.encode(bytes)
}

fn passkey_json(rp_id: &str, id: &[u8], handle: &[u8], username: &str, key: &str) -> Value {
    json!({
        "type": "passkey",
        "credentialId": b64(id),
        "rpId": rp_id,
        "username": username,
        "userDisplayName": format!("{username} display"),
        "userHandle": b64(handle),
        "key": key,
    })
}

fn item_json(credentials: Vec<Value>) -> Value {
    json!({
        "id": b64(b"item-0123456789ab"),
        "creationAt": 1_700_000_000u64,
        "modifiedAt": 1_700_000_100u64,
        "title": "example",
        "credentials": credentials,
    })
}

fn account_json(items: Vec<Value>) -> Value {
    json!({
        "id": b64(b"account-01234567"),
        "username": "",
        "email": "",
        "collections": [],
        "items": items,
    })
}

fn doc_json(accounts: Vec<Value>) -> Value {
    json!({
        "version": {"major": 1, "minor": 0},
        "exporterRpId": "example.com",
        "exporterDisplayName": "Example",
        "timestamp": 1_700_000_000u64,
        "accounts": accounts,
    })
}

fn one_item_doc(credentials: Vec<Value>) -> Value {
    doc_json(vec![account_json(vec![item_json(credentials)])])
}

fn good_passkey(name: &str) -> Value {
    passkey_json("example.com", &[7; 16], b"handle", name, FIXTURE_KEY)
}

fn import_value(value: &Value) -> Result<Imported, PasskeyError> {
    import_cxf(&serde_json::to_vec(value).expect("serialise"))
}

fn refusal(result: Result<Imported, PasskeyError>) -> &'static str {
    match result {
        Err(PasskeyError::InvalidExchangeFile(text)) => text,
        Err(other) => panic!("wrong error: {other}"),
        Ok(imported) => panic!("accepted a file with {} records", imported.records.len()),
    }
}

fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    assert!(content.len() < 0x80);
    let mut out = vec![tag, content.len() as u8];
    out.extend(content);
    out
}

/// A well-formed PKCS#8 shell for an algorithm with the given OID bytes (not a usable key).
fn fake_pkcs8(oid: &[u8]) -> Vec<u8> {
    let mut body = der(0x02, &[0]);
    body.extend(der(0x30, &der(0x06, oid)));
    body.extend(der(0x04, &[0x04, 0x02, 1, 2]));
    der(0x30, &body)
}

// ---- round trips ------------------------------------------------------------------------------

#[test]
fn round_trip_keeps_keys_ids_names_handles_and_dates() {
    let (a, key_a) = record_with("example.com", &[1; 32], "alice", Some(b"alice-handle"));
    let (b, _) = record_with("example.com", &[2; 16], "bob", Some(&[9; 64]));
    // The same user on a second site.
    let (c, _) = record_with("example.org", &[3; 20], "alice", Some(b"alice-handle"));
    let records = [a.clone(), b.clone(), c.clone()];

    let exported = export_bytes(&records);
    assert_eq!((exported.exported, exported.skipped.len()), (3, 0));
    let imported = import_cxf(exported.as_bytes()).expect("import");

    assert!(imported.skipped.is_empty() && imported.skipped_kinds.is_empty());
    assert_eq!(imported.records.len(), 3);
    for (original, back) in records.iter().zip(&imported.records) {
        assert_eq!(back.summary, original.summary);
        assert!(back.private_key.same_as(&original.private_key));
        assert_eq!(back.summary.counter, None);
        assert_eq!(back.summary.created_at_ms, Some(1_700_000_000_000));
        assert_eq!(back.summary.modified_at_ms, Some(1_700_000_100_000));
    }

    // The imported key is the same key: same public key, and it signs for the original.
    let cose = CoseKey::from_slice(imported.records[0].private_key.expose()).expect("cose");
    let mut again = RustCryptoSecretKey::from_cose_key(&cose).expect("key");
    assert_eq!(
        again.public_key().to_cose_key(),
        key_a.secret.public_key().to_cose_key()
    );
    let message = b"authenticator data and client data hash";
    let signature = again.sign(message);
    key_a
        .secret
        .public_key()
        .verify(message, &signature)
        .expect("the original public key verifies the imported key's signature");
}

#[test]
fn an_export_the_user_refuses_writes_nothing_and_errors() {
    let (a, _) = record_with("example.com", &[1; 32], "alice", Some(b"h1"));
    let verifier = FakeVerifier::new();
    verifier.set_default_answer(FakeAnswer::Decline);
    let result = export_cxf(&[a], ACK, NOW, &verifier);
    assert!(matches!(result, Err(PasskeyError::NotAllowed)));
    assert_eq!(verifier.requests().len(), 1, "the user was asked");
}

#[test]
fn an_export_the_user_allows_holds_every_passkey() {
    let (a, _) = record_with("example.com", &[1; 32], "alice", Some(b"h1"));
    let verifier = FakeVerifier::new();
    let exported = export_cxf(&[a], ACK, NOW, &verifier).expect("export");
    assert!(!exported.as_bytes().is_empty());
    assert_eq!(verifier.requests().len(), 1, "the user was asked");
}

#[test]
fn export_writes_cxf_1_0_with_one_account_and_one_item_per_passkey() {
    let (a, _) = record_with("example.com", &[1; 32], "alice", Some(b"h1"));
    let (b, _) = record_with("example.org", &[2; 32], "bob", Some(b"h2"));
    let value = export_json(&[a, b]);

    assert_eq!(value["version"], json!({"major": 1, "minor": 0}));
    assert_eq!(
        value["exporterRpId"],
        "io.github.thepigdestroyer.mercury-browser"
    );
    assert_eq!(value["exporterDisplayName"], "Mercury Browser");
    assert_eq!(value["timestamp"], NOW);
    let accounts = value["accounts"].as_array().expect("accounts");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["username"], "");
    assert_eq!(accounts[0]["email"], "");
    let account_id = BASE64URL_NOPAD
        .decode(accounts[0]["id"].as_str().expect("id").as_bytes())
        .expect("b64url");
    assert_eq!(account_id.len(), 16);

    let items = accounts[0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    let mut ids = vec![account_id];
    for (item, (rp, name)) in items
        .iter()
        .zip([("example.com", "alice"), ("example.org", "bob")])
    {
        assert_eq!(item["title"], rp);
        assert_eq!(item["subtitle"], name);
        assert_eq!(item["scope"]["urls"], json!([format!("https://{rp}")]));
        assert_eq!(item["creationAt"], 1_700_000_000u64);
        assert_eq!(item["modifiedAt"], 1_700_000_100u64);
        let credentials = item["credentials"].as_array().expect("credentials");
        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0]["type"], "passkey");
        assert_eq!(credentials[0]["rpId"], rp);
        ids.push(
            BASE64URL_NOPAD
                .decode(item["id"].as_str().expect("id").as_bytes())
                .expect("b64url"),
        );
    }
    assert!(ids.iter().all(|id| id.len() == 16));
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3, "account and item ids are distinct");
}

#[test]
fn ids_are_fresh_for_every_export() {
    let (a, _) = record_with("example.com", &[1; 32], "alice", Some(b"h"));
    let first = export_json(std::slice::from_ref(&a));
    let second = export_json(std::slice::from_ref(&a));
    assert_ne!(first["accounts"][0]["id"], second["accounts"][0]["id"]);
    assert_ne!(
        first["accounts"][0]["items"][0]["id"],
        second["accounts"][0]["items"][0]["id"]
    );
}

#[test]
fn a_passkey_without_dates_gets_the_export_time() {
    let (mut a, _) = record_with("example.com", &[1; 32], "alice", Some(b"h"));
    a.summary.created_at_ms = None;
    a.summary.modified_at_ms = None;
    let value = export_json(&[a]);
    let item = &value["accounts"][0]["items"][0];
    assert_eq!(item["creationAt"], NOW);
    assert_eq!(item["modifiedAt"], NOW);
}

#[test]
fn an_empty_export_is_a_valid_file_that_imports_nothing() {
    let exported = export_bytes(&[]);
    assert_eq!(exported.exported, 0);
    let imported = import_cxf(exported.as_bytes()).expect("import");
    assert!(imported.records.is_empty());
}

#[test]
fn two_accounts_both_import() {
    let second = passkey_json("example.org", &[8; 16], b"other", "carol", FIXTURE_KEY);
    let value = doc_json(vec![
        account_json(vec![item_json(vec![good_passkey("alice")])]),
        account_json(vec![item_json(vec![second])]),
    ]);
    let imported = import_value(&value).expect("import");
    let names: Vec<_> = imported
        .records
        .iter()
        .map(|record| {
            (
                record.summary.key.rp_id.as_str(),
                record.summary.username.as_str(),
            )
        })
        .collect();
    assert_eq!(names, [("example.com", "alice"), ("example.org", "carol")]);
}

// ---- what is left out -------------------------------------------------------------------------

#[test]
fn export_skips_and_counts_what_cxf_cannot_carry() {
    let (good, _) = record_with("example.com", &[1; 32], "good", Some(b"h"));
    let (no_handle, _) = record_with("example.com", &[2; 32], "nohandle", None);
    let (mut counter, _) = record_with("example.com", &[3; 32], "counter", Some(b"h"));
    counter.summary.counter = Some(5);
    let (mut zero_counter, _) = record_with("example.com", &[4; 32], "zero", Some(b"h"));
    zero_counter.summary.counter = Some(0);
    let (mut other_algorithm, _) = record_with("example.com", &[5; 32], "alg", Some(b"h"));
    other_algorithm.summary.algorithm = -8;
    let (mut garbage_key, _) = record_with("example.com", &[6; 32], "garbage", Some(b"h"));
    garbage_key.private_key = PrivateKey::new(vec![0xff; 12]);
    let (mut bad_site, _) = record_with("Example.COM", &[7; 32], "site", Some(b"h"));
    bad_site.summary.key.rp_id = "Example.COM".to_owned();
    let (ed_record, _) = ed25519_record();

    let exported = export_bytes(&[
        no_handle,
        good,
        counter,
        zero_counter,
        other_algorithm,
        garbage_key,
        bad_site,
        ed_record,
    ]);
    assert_eq!(exported.exported, 2, "the good one and the zero counter");
    assert_eq!(
        exported.skipped,
        [
            SkipReason::NotDiscoverable,
            SkipReason::NonZeroCounter,
            SkipReason::UnsupportedAlgorithm,
            SkipReason::UnreadableKey,
            SkipReason::InvalidRecord,
            SkipReason::UnsupportedAlgorithm,
        ]
    );
    let imported = import_cxf(exported.as_bytes()).expect("import");
    let names: Vec<_> = imported
        .records
        .iter()
        .map(|record| record.summary.username.as_str())
        .collect();
    assert_eq!(names, ["good", "zero"]);
}

/// A stored passkey whose key is Ed25519 although its summary says ES256.
fn ed25519_record() -> (PasskeyRecord, Material) {
    let (mut record, mut material) = record_with("example.com", &[9; 32], "ed", Some(b"h"));
    let secret = RustCryptoBackend
        .generate_key(iana::Algorithm::EdDSA)
        .expect("ed25519");
    material.cose = secret.to_cose_key().to_vec().expect("cbor");
    record.private_key = PrivateKey::new(material.cose.clone());
    (record, material)
}

#[test]
fn other_credential_kinds_are_counted_by_name_and_skipped() {
    let value = one_item_doc(vec![
        json!({"type": "note", "content": {"fieldType": "string", "value": "remember"}}),
        json!({"type": "note", "content": {"fieldType": "string", "value": "again"}}),
        json!({"type": "basic-auth",
               "username": {"fieldType": "string", "value": "u"},
               "password": {"fieldType": "concealed-string", "value": "hunter2"}}),
        json!({"type": "future-credential", "shape": {"anything": [1, 2, 3]}}),
        good_passkey("alice"),
    ]);
    let imported = import_value(&value).expect("import");
    assert_eq!(imported.records.len(), 1);
    assert!(imported.skipped.is_empty());
    let kinds: Vec<_> = imported
        .skipped_kinds
        .iter()
        .map(|(name, count)| (name.as_str(), *count))
        .collect();
    assert_eq!(
        kinds,
        [("basic-auth", 1), ("future-credential", 1), ("note", 2)]
    );
}

#[test]
fn a_file_with_no_passkeys_is_not_an_error() {
    let value = one_item_doc(vec![
        json!({"type": "note", "content": {"fieldType": "string", "value": "x"}}),
    ]);
    let imported = import_value(&value).expect("import");
    assert!(imported.records.is_empty());
    assert_eq!(imported.skipped_kinds.get("note"), Some(&1));
}

#[test]
fn invented_kind_names_cannot_flood_the_result() {
    let long = "k".repeat(5000);
    let mut credentials = vec![json!({"type": long}), json!({"type": "bad\u{7}name"})];
    for index in 0..200 {
        credentials.push(json!({"type": format!("kind-{index}")}));
    }
    let imported = import_value(&one_item_doc(credentials)).expect("import");
    assert!(imported.skipped_kinds.len() <= 65);
    assert!(
        imported
            .skipped_kinds
            .keys()
            .all(|name| name.chars().count() <= 64)
    );
    assert!(
        imported
            .skipped_kinds
            .keys()
            .all(|name| !name.chars().any(char::is_control))
    );
    assert_eq!(imported.skipped_kinds.values().sum::<usize>(), 202);
}

#[test]
fn keys_of_other_algorithms_are_skipped_not_refused() {
    let ed25519 = RustCryptoBackend
        .generate_key(iana::Algorithm::EdDSA)
        .expect("ed25519")
        .to_pkcs8()
        .expect("pkcs8");
    let rsa = fake_pkcs8(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01]);
    let value = one_item_doc(vec![
        passkey_json("example.com", &[1; 16], b"h", "ed", &b64(&ed25519)),
        passkey_json("example.com", &[2; 16], b"h", "rsa", &b64(&rsa)),
        good_passkey("p256"),
    ]);
    let imported = import_value(&value).expect("import");
    assert_eq!(imported.records.len(), 1);
    assert_eq!(imported.records[0].summary.username, "p256");
    assert_eq!(
        imported.skipped,
        [
            SkipReason::UnsupportedAlgorithm,
            SkipReason::UnsupportedAlgorithm
        ]
    );
}

#[test]
fn a_skipped_passkey_still_has_its_ids_checked() {
    let ed25519 = RustCryptoBackend
        .generate_key(iana::Algorithm::EdDSA)
        .expect("ed25519")
        .to_pkcs8()
        .expect("pkcs8");
    let value = one_item_doc(vec![passkey_json(
        "Example.COM",
        &[1; 16],
        b"h",
        "ed",
        &b64(&ed25519),
    )]);
    refusal(import_value(&value));
}

#[test]
fn passkeys_with_extensions_this_crate_cannot_keep_are_skipped() {
    let mut hmac = good_passkey("hmac");
    hmac["fido2Extensions"] = json!({"hmacCredentials": {
        "algorithm": "hmac-sha256", "credWithUV": "AAAA", "credWithoutUV": "AAAA"}});
    let mut blob = passkey_json("example.com", &[2; 16], b"h", "blob", FIXTURE_KEY);
    blob["fido2Extensions"] = json!({"credBlob": "AAAA"});
    let mut payments = passkey_json("example.com", &[3; 16], b"h", "pay", FIXTURE_KEY);
    payments["fido2Extensions"] = json!({"payments": true});
    let imported = import_value(&one_item_doc(vec![hmac, blob, payments])).expect("import");
    assert_eq!(imported.records.len(), 1);
    assert_eq!(imported.records[0].summary.username, "pay");
    assert_eq!(
        imported.skipped,
        [
            SkipReason::UnsupportedExtension,
            SkipReason::UnsupportedExtension
        ]
    );
}

#[test]
fn the_same_passkey_listed_twice_is_imported_once() {
    let value = one_item_doc(vec![good_passkey("alice"), good_passkey("alice")]);
    assert_eq!(import_value(&value).expect("import").records.len(), 1);
}

#[test]
fn one_passkey_listed_twice_with_different_keys_is_refused() {
    let other = new_material();
    let value = one_item_doc(vec![
        good_passkey("alice"),
        passkey_json(
            "example.com",
            &[7; 16],
            b"handle",
            "alice",
            &b64(&other.pkcs8),
        ),
    ]);
    refusal(import_value(&value));
}

#[test]
fn an_imported_passkey_never_replaces_a_stored_one_with_the_same_ids() {
    let store = PasskeyStore::new(MemoryBackend::new());
    let (stored, _) = record_with("example.com", &[7; 16], "alice", Some(b"handle"));
    store.save(&stored).expect("save");

    let imported = import_value(&one_item_doc(vec![good_passkey("mallory")])).expect("import");
    assert_eq!(imported.records.len(), 1);
    assert!(!imported.records[0].private_key.same_as(&stored.private_key));
    let outcome = store.import(&imported.records).expect("store the import");

    assert_eq!((outcome.saved, outcome.conflicts.len()), (0, 1));
    assert_eq!(outcome.conflicts[0], stored.summary.key);
    let back = store
        .backend()
        .get(&stored.summary.key)
        .expect("get")
        .expect("the stored passkey is still there");
    assert!(back.private_key.same_as(&stored.private_key));
    assert_eq!(back.summary.username, "alice");
    assert_eq!(store.all().expect("all").len(), 1);
}

// ---- the spec's example, written by hand ------------------------------------------------------

#[test]
fn a_hand_written_file_in_the_shape_of_the_specification_example_imports() {
    let file = format!(
        r#"{{
  "version": {{ "major": 1, "minor": 7 }},
  "exporterRpId": "com.example.exporter",
  "exporterDisplayName": "Example Exporter",
  "timestamp": 1700000000,
  "futureHeaderField": {{ "ignored": true }},
  "accounts": [
    {{
      "id": "AAECAwQFBgcICQoLDA0ODw",
      "username": "alice",
      "email": "alice@example.com",
      "fullName": "Alice Example",
      "collections": [
        {{ "id": "Y29sbGVjdGlvbi0x", "title": "Personal", "items": [ {{ "item": "aXRlbS0x" }} ] }}
      ],
      "items": [
        {{
          "id": "aXRlbS0x",
          "creationAt": 1690000000,
          "modifiedAt": 1695000000,
          "title": "Example",
          "subtitle": "alice",
          "favorite": true,
          "scope": {{ "urls": ["https://example.com"], "androidApps": [] }},
          "tags": ["work"],
          "futureItemField": 12,
          "extensions": [ {{ "name": "future-extension", "value": 1 }} ],
          "credentials": [
            {{
              "type": "passkey",
              "credentialId": "Y3JlZGVudGlhbC1pZC0xMjM0NQ",
              "rpId": "example.com",
              "username": "alice",
              "userDisplayName": "Alice Example",
              "userHandle": "dXNlci1oYW5kbGU",
              "key": "{FIXTURE_KEY}",
              "futureField": "tolerated"
            }},
            {{ "type": "note", "content": {{ "fieldType": "string", "value": "recovery codes" }} }},
            {{ "type": "future-credential", "x": 1 }}
          ]
        }}
      ]
    }}
  ]
}}"#
    );
    let imported = import_cxf(file.as_bytes()).expect("import");
    assert_eq!(imported.records.len(), 1);
    let record = &imported.records[0];
    assert_eq!(record.summary.key.rp_id, "example.com");
    assert_eq!(record.summary.key.credential_id, b"credential-id-12345");
    assert_eq!(
        record.summary.user_handle.as_deref(),
        Some(&b"user-handle"[..])
    );
    assert_eq!(record.summary.username, "alice");
    assert_eq!(record.summary.display_name, "Alice Example");
    assert_eq!(record.summary.created_at_ms, Some(1_690_000_000_000));
    assert_eq!(record.summary.modified_at_ms, Some(1_695_000_000_000));
    assert_eq!(record.summary.counter, None);
    assert_eq!(record.summary.algorithm, COSE_ES256);
    assert_eq!(imported.skipped_kinds.get("note"), Some(&1));
    assert_eq!(imported.skipped_kinds.get("future-credential"), Some(&1));

    // It is the key in the file: the public point is the one in the PKCS#8.
    let cose = CoseKey::from_slice(record.private_key.expose()).expect("cose");
    let coordinate = |label| {
        cose.params
            .iter()
            .find_map(|(l, v)| match (l, v) {
                (Label::Int(n), Cbor::Bytes(bytes)) if *n == label => Some(HEXLOWER.encode(bytes)),
                _ => None,
            })
            .expect("coordinate")
    };
    assert_eq!(coordinate(-2), FIXTURE_X);
    assert_eq!(coordinate(-3), FIXTURE_Y);
}

#[test]
fn trailing_whitespace_and_padded_base64_are_accepted() {
    let mut value = one_item_doc(vec![good_passkey("alice")]);
    value["accounts"][0]["items"][0]["credentials"][0]["credentialId"] =
        json!(BASE64URL.encode(&[7; 16]));
    let mut bytes = serde_json::to_vec_pretty(&value).expect("serialise");
    bytes.extend(b"\n\n  ");
    assert_eq!(import_cxf(&bytes).expect("import").records.len(), 1);
}

// ---- refusals ---------------------------------------------------------------------------------

#[test]
fn empty_and_truncated_input_is_refused() {
    refusal(import_cxf(b""));
    let (a, _) = record_with("example.com", &[1; 32], "alice", Some(b"h"));
    let exported = export_bytes(&[a]);
    let bytes = exported.as_bytes();
    for length in [1, bytes.len() / 2, bytes.len() - 1] {
        refusal(import_cxf(&bytes[..length]));
    }
}

#[test]
fn input_that_is_not_json_or_not_utf8_is_refused() {
    refusal(import_cxf(b"not json"));
    refusal(import_cxf(b"[]"));
    refusal(import_cxf(b"{}"));
    refusal(import_cxf(b"null"));
    let mut bytes = serde_json::to_vec(&one_item_doc(vec![])).expect("serialise");
    let position = bytes.iter().position(|&b| b == b'E').expect("a letter");
    bytes[position] = 0xff;
    refusal(import_cxf(&bytes));
}

#[test]
fn trailing_garbage_is_refused() {
    let mut bytes = serde_json::to_vec(&one_item_doc(vec![good_passkey("a")])).expect("serialise");
    bytes.extend(b" x");
    refusal(import_cxf(&bytes));
    let mut twice = serde_json::to_vec(&one_item_doc(vec![])).expect("serialise");
    twice.extend(serde_json::to_vec(&one_item_doc(vec![])).expect("serialise"));
    refusal(import_cxf(&twice));
}

#[test]
fn an_unsupported_or_missing_version_is_refused() {
    for version in [
        json!({"major": 2, "minor": 0}),
        json!({"major": 0, "minor": 0}),
    ] {
        let mut value = one_item_doc(vec![good_passkey("alice")]);
        value["version"] = version;
        refusal(import_value(&value));
    }
    let mut missing = one_item_doc(vec![good_passkey("alice")]);
    missing.as_object_mut().expect("object").remove("version");
    refusal(import_value(&missing));
    let mut wrong_type = one_item_doc(vec![good_passkey("alice")]);
    wrong_type["version"] = json!("1.0");
    refusal(import_value(&wrong_type));
}

#[test]
fn oversize_input_is_refused() {
    let big = vec![b' '; MAX_IMPORT_BYTES + 1];
    assert!(refusal(import_cxf(&big)).contains("large"));
}

#[test]
fn too_many_items_are_refused_and_the_limit_itself_is_accepted() {
    let item = json!({"id": "AA", "title": "t", "credentials": []});
    let doc = |count: usize| doc_json(vec![account_json(vec![item.clone(); count])]);
    assert!(import_value(&doc(MAX_IMPORT_ITEMS)).is_ok());
    refusal(import_value(&doc(MAX_IMPORT_ITEMS + 1)));
    // The limit is for the whole file, not per account.
    let split = doc_json(vec![
        account_json(vec![item.clone(); MAX_IMPORT_ITEMS / 2 + 1]),
        account_json(vec![item.clone(); MAX_IMPORT_ITEMS / 2 + 1]),
    ]);
    refusal(import_value(&split));
}

#[test]
fn keys_that_are_not_pkcs8_are_refused() {
    let material = new_material();
    let mut truncated = BASE64URL_NOPAD.decode(FIXTURE_KEY.as_bytes()).expect("b64");
    truncated.truncate(60);
    let mut trailing = material.pkcs8.clone();
    trailing.push(0);
    let candidates: Vec<(&str, Vec<u8>)> = vec![
        ("a COSE_Key", material.cose.clone()),
        ("the bare scalar", material.scalar.clone()),
        ("random bytes", vec![0x5a; 138]),
        ("empty", Vec::new()),
        ("truncated PKCS#8", truncated),
        ("PKCS#8 with trailing bytes", trailing),
    ];
    for (what, key) in candidates {
        let value = one_item_doc(vec![passkey_json(
            "example.com",
            &[1; 16],
            b"h",
            "alice",
            &b64(&key),
        )]);
        assert!(
            matches!(
                import_value(&value),
                Err(PasskeyError::InvalidExchangeFile(_))
            ),
            "{what}"
        );
    }
}

#[test]
fn an_es256_key_that_does_not_parse_is_refused() {
    // The right PKCS#8 shell with an all-zero scalar, which is not a P-256 private key.
    let mut der = BASE64URL_NOPAD.decode(FIXTURE_KEY.as_bytes()).expect("b64");
    let start = 36;
    der[start..start + 32].fill(0);
    let value = one_item_doc(vec![passkey_json(
        "example.com",
        &[1; 16],
        b"h",
        "alice",
        &b64(&der),
    )]);
    refusal(import_value(&value));
}

#[test]
fn ids_and_names_that_break_the_store_rules_are_refused() {
    let cases = [
        passkey_json("example.com", &[1; 15], b"h", "a", FIXTURE_KEY),
        passkey_json("example.com", &vec![1; 1024], b"h", "a", FIXTURE_KEY),
        passkey_json("Example.com", &[1; 16], b"h", "a", FIXTURE_KEY),
        passkey_json("example..com", &[1; 16], b"h", "a", FIXTURE_KEY),
        passkey_json("", &[1; 16], b"h", "a", FIXTURE_KEY),
        passkey_json("example.com", &[1; 16], b"", "a", FIXTURE_KEY),
        passkey_json("example.com", &[1; 16], &[1; 65], "a", FIXTURE_KEY),
        passkey_json(
            "example.com",
            &[1; 16],
            b"h",
            &"a".repeat(1025),
            FIXTURE_KEY,
        ),
        passkey_json("example.com", &[1; 16], b"h", "a\u{0}b", FIXTURE_KEY),
        // A string field of 1 MiB.
        passkey_json(
            "example.com",
            &[1; 16],
            b"h",
            &"a".repeat(1 << 20),
            FIXTURE_KEY,
        ),
        passkey_json(&"a".repeat(1 << 20), &[1; 16], b"h", "a", FIXTURE_KEY),
    ];
    for (index, case) in cases.iter().enumerate() {
        let result = import_value(&one_item_doc(vec![case.clone()]));
        assert!(
            matches!(result, Err(PasskeyError::InvalidExchangeFile(_))),
            "case {index}"
        );
    }
}

#[test]
fn bad_base64url_is_refused() {
    for field in ["credentialId", "userHandle", "key"] {
        let mut passkey = good_passkey("alice");
        passkey[field] = json!("not base64url !!");
        refusal(import_value(&one_item_doc(vec![passkey])));
    }
}

#[test]
fn a_malformed_passkey_entry_is_refused_not_skipped_as_unknown() {
    for field in [
        "credentialId",
        "rpId",
        "username",
        "userDisplayName",
        "userHandle",
        "key",
    ] {
        let mut passkey = good_passkey("alice");
        passkey.as_object_mut().expect("object").remove(field);
        let text = refusal(import_value(&one_item_doc(vec![passkey])));
        assert!(text.contains("malformed"), "{field}: {text}");
    }
    let mut wrong_type = good_passkey("alice");
    wrong_type["rpId"] = json!(5);
    refusal(import_value(&one_item_doc(vec![wrong_type])));
}

#[test]
fn deeply_nested_json_is_refused() {
    let depth = 5000;
    let nested = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let file = format!(
        r#"{{"version":{{"major":1,"minor":0}},"exporterRpId":"x","exporterDisplayName":"x","timestamp":1,
            "accounts":[{{"id":"AA","username":"","email":"","collections":[],"items":[
              {{"id":"AA","title":"t","credentials":[{{"type":"future","deep":{nested}}}]}}]}}]}}"#
    );
    refusal(import_cxf(file.as_bytes()));
    // In a field nobody reads, serde_json skips the nesting without recursion: no stack is used
    // and nothing is kept, so the file is fine.
    let in_header = format!(
        r#"{{"version":{{"major":1,"minor":0}},"exporterRpId":"x","exporterDisplayName":"x","timestamp":1,
            "accounts":[],"extra":{nested}}}"#
    );
    assert!(
        import_cxf(in_header.as_bytes())
            .expect("skipped, not parsed")
            .records
            .is_empty()
    );
}

#[test]
fn three_good_passkeys_and_one_bad_import_nothing() {
    let value = one_item_doc(vec![
        passkey_json("example.com", &[1; 16], b"h", "a", FIXTURE_KEY),
        passkey_json("example.com", &[2; 16], b"h", "b", FIXTURE_KEY),
        passkey_json("example.com", &[3; 16], b"h", "c", FIXTURE_KEY),
        passkey_json("example.com", &[4; 15], b"h", "bad", FIXTURE_KEY),
    ]);
    refusal(import_value(&value));
    // Without the bad one all three import.
    let good = one_item_doc(vec![
        passkey_json("example.com", &[1; 16], b"h", "a", FIXTURE_KEY),
        passkey_json("example.com", &[2; 16], b"h", "b", FIXTURE_KEY),
        passkey_json("example.com", &[3; 16], b"h", "c", FIXTURE_KEY),
    ]);
    assert_eq!(import_value(&good).expect("import").records.len(), 3);
}

#[test]
fn a_bad_passkey_in_a_later_account_refuses_the_whole_file() {
    let value = doc_json(vec![
        account_json(vec![item_json(vec![good_passkey("alice")])]),
        account_json(vec![item_json(vec![passkey_json(
            "example.com",
            &[1; 3],
            b"h",
            "x",
            FIXTURE_KEY,
        )])]),
    ]);
    refusal(import_value(&value));
}

#[test]
fn refusal_texts_never_quote_the_file() {
    let canary = "CANARY-SECRET-9f2c";
    let mut cases = Vec::new();
    for field in ["credentialId", "userHandle", "key", "rpId"] {
        let mut passkey = good_passkey("alice");
        passkey[field] = json!(format!("{canary}!!"));
        cases.push(one_item_doc(vec![passkey]));
    }
    let mut version = one_item_doc(vec![good_passkey("alice")]);
    version["version"] = json!({"major": canary});
    cases.push(version);
    let mut timestamp = one_item_doc(vec![]);
    timestamp["timestamp"] = json!(canary);
    cases.push(timestamp);
    for case in cases {
        let error = import_value(&case).expect_err("refused");
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("CANARY"), "{shown}");
        assert!(!shown.contains(FIXTURE_KEY), "{shown}");
    }
}

// ---- secrets in memory and in output ----------------------------------------------------------

#[test]
fn exporting_puts_the_pkcs8_key_in_the_clear_and_debug_output_never_does() {
    let (record, material) = record_with("example.com", &[1; 32], "alice", Some(b"h"));
    let exported = export_bytes(std::slice::from_ref(&record));
    let text = std::str::from_utf8(exported.as_bytes()).expect("utf-8");
    // The control: the file does hold the key, in the CXF encoding.
    assert!(text.contains(&b64(&material.pkcs8)));

    let imported = import_cxf(exported.as_bytes()).expect("import");
    let shown = [
        format!("{exported:?}"),
        format!("{exported:#?}"),
        format!("{imported:?}"),
        format!("{imported:#?}"),
        format!("{record:?}"),
        format!("{:?}", imported.records),
    ]
    .join("\n");
    for (what, secret) in [
        ("pkcs8", &material.pkcs8),
        ("cose", &material.cose),
        ("scalar", &material.scalar),
    ] {
        assert!(
            !contains_any_encoding(&shown, secret),
            "{what} appears in Debug output"
        );
    }
    // The debug text says how much, not what.
    assert!(shown.contains("Exported"));
    assert!(shown.contains("Imported"));
}

#[test]
fn errors_and_skip_reasons_carry_no_key() {
    let material = new_material();
    let value = one_item_doc(vec![passkey_json(
        "example.com",
        &[1; 3],
        b"h",
        "alice",
        &b64(&material.pkcs8),
    )]);
    let error = import_value(&value).expect_err("refused");
    let shown = format!("{error} {error:?}");
    for secret in [&material.pkcs8, &material.cose, &material.scalar] {
        assert!(!contains_any_encoding(&shown, secret));
    }
}

// ---- files ------------------------------------------------------------------------------------

#[cfg(unix)]
mod files {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn sample_export() -> Exported {
        let (record, _) = record_with("example.com", &[1; 32], "alice", Some(b"h"));
        export_bytes(&[record])
    }

    #[test]
    fn the_export_file_is_owner_only_and_holds_the_export() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("passkeys.cxf");
        let exported = sample_export();
        write_export_file(&path, &exported).expect("write");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read(&path).expect("read"), exported.as_bytes());
        // It reads back through the import path.
        let read = read_import_file(&path).expect("read import file");
        assert_eq!(import_cxf(&read).expect("import").records.len(), 1);
    }

    #[test]
    fn an_existing_file_is_never_overwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("passkeys.cxf");
        std::fs::write(&path, b"precious").expect("write");
        let error = write_export_file(&path, &sample_export()).expect_err("refused");
        assert!(matches!(error, PasskeyError::InvalidInput(_)), "{error}");
        assert_eq!(std::fs::read(&path).expect("read"), b"precious");
    }

    #[test]
    fn a_symbolic_link_at_the_path_is_not_followed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.cxf");
        let link = dir.path().join("link.cxf");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        write_export_file(&link, &sample_export()).expect_err("refused");
        assert!(!target.exists(), "the link's target was created");
        // Also when the target exists: it is left alone.
        std::fs::write(&target, b"precious").expect("write");
        write_export_file(&link, &sample_export()).expect_err("refused");
        assert_eq!(std::fs::read(&target).expect("read"), b"precious");
    }

    #[test]
    fn a_missing_folder_is_an_os_error_and_leaves_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing").join("passkeys.cxf");
        let error = write_export_file(&path, &sample_export()).expect_err("refused");
        assert!(matches!(error, PasskeyError::Os { .. }), "{error}");
        assert!(!path.exists());
        assert!(!format!("{error}").contains("passkeys.cxf"));
    }

    #[test]
    fn an_oversize_file_is_refused_without_being_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("huge.cxf");
        let file = std::fs::File::create(&path).expect("create");
        file.set_len(MAX_IMPORT_BYTES as u64 + 1).expect("sparse");
        assert!(matches!(
            read_import_file(&path),
            Err(PasskeyError::InvalidExchangeFile(_))
        ));
    }

    #[test]
    fn a_missing_import_file_is_an_os_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = read_import_file(&dir.path().join("none.cxf")).expect_err("missing");
        assert!(matches!(error, PasskeyError::Os { .. }), "{error}");
    }
}
