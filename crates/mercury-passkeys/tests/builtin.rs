// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Behaviour of the built-in authenticator, end to end on the in-memory backend: what a page gets
//! back, what the user is asked, and what is stored.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};
use std::thread::sleep;
use std::time::Duration;

use mercury_passkeys::builtin::{
    BuiltInAuthenticator, PasskeyCredentialStore, StoredPasskey, UserValidation,
};
use mercury_passkeys::crypto::MercuryCrypto;
use mercury_passkeys::testing::{FakeAnswer, FakePick, FakeVerifier};
use mercury_passkeys::{
    AddOutcome, AssertRequest, AssertResponse, Attachment, AuthenticatorKind, COSE_EDDSA,
    COSE_ES256, COSE_RS256, CreateRequest, CreateResponse, MemoryBackend, PasskeyAuthenticator,
    PasskeyBackend, PasskeyError, PasskeyKey, PasskeyRecord, PasskeyStore, PasskeySummary,
    PrivateKey, RelyingParty, ResidentKey, SaveOutcome, Transport, UserEntity, UserVerification,
    VerificationPurpose, VerifierCapabilities,
};
use passkey::authenticator::{CredentialStore, PasskeyAccessor};
use passkey::crypto::coset::cbor::{self, Value};
use passkey::crypto::coset::{CborSerializable, CoseKey, Label};
use passkey::crypto::hash::Sha256Backend;
use passkey::crypto::rust_crypto::{RustCryptoPublicKey, RustCryptoSecretKey, RustCryptoSha2};
use passkey::crypto::{PublicKeyT, SecretKeyT};
use passkey::types::ctap2::{AuthenticatorData, Flags};
use passkey::types::webauthn::{PublicKeyCredentialDescriptor, PublicKeyCredentialType};
use passkey::types::{Bytes, CredentialExtensions, Passkey, StoredHmacSecret};

const ORIGIN: &str = "https://example.com";
const RP: &str = "example.com";

struct Harness<B: PasskeyBackend + 'static = MemoryBackend> {
    store: Arc<PasskeyStore<B>>,
    verifier: Arc<FakeVerifier>,
    authenticator: BuiltInAuthenticator<B>,
}

fn harness() -> Harness {
    harness_with(FakeVerifier::new())
}

fn harness_with(verifier: FakeVerifier) -> Harness {
    harness_over(MemoryBackend::new(), verifier)
}

fn harness_over<B: PasskeyBackend + 'static>(backend: B, verifier: FakeVerifier) -> Harness<B> {
    let store = Arc::new(PasskeyStore::new(backend));
    let verifier = Arc::new(verifier);
    let authenticator = BuiltInAuthenticator::new(Arc::clone(&store), verifier.clone())
        .expect("the system RNG works");
    Harness {
        store,
        verifier,
        authenticator,
    }
}

fn hash(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn create_request(rp: &str, user_id: &[u8], name: &str) -> CreateRequest {
    CreateRequest {
        origin: format!("https://{rp}"),
        rp: RelyingParty {
            id: rp.to_owned(),
            name: Some("Example".to_owned()),
        },
        user: UserEntity {
            id: user_id.to_vec(),
            name: name.to_owned(),
            display_name: format!("{name} (display)"),
        },
        client_data_hash: hash(1),
        algorithms: vec![COSE_ES256],
        exclude: Vec::new(),
        resident_key: ResidentKey::Required,
        user_verification: UserVerification::Preferred,
        timeout_ms: None,
    }
}

fn assert_request(rp: &str, allow: Vec<Vec<u8>>) -> AssertRequest {
    AssertRequest {
        origin: format!("https://{rp}"),
        rp_id: rp.to_owned(),
        client_data_hash: hash(2),
        allow,
        user_verification: UserVerification::Preferred,
        timeout_ms: None,
    }
}

/// Creates a passkey for `user_id` at `rp` and returns the response.
fn create<B: PasskeyBackend + 'static>(
    harness: &Harness<B>,
    rp: &str,
    user_id: &[u8],
    name: &str,
) -> CreateResponse {
    harness
        .authenticator
        .make_credential(&create_request(rp, user_id, name))
        .expect("the passkey is created")
}

fn key_of(rp: &str, credential_id: &[u8]) -> PasskeyKey {
    PasskeyKey {
        rp_id: rp.to_owned(),
        credential_id: credential_id.to_vec(),
    }
}

fn auth_data(bytes: &[u8]) -> AuthenticatorData {
    AuthenticatorData::from_slice(bytes).expect("authenticator data parses")
}

fn public_key_of(response: &CreateResponse) -> CoseKey {
    auth_data(&response.authenticator_data)
        .attested_credential_data
        .expect("attested credential data")
        .key
}

fn secret_key_of<B: PasskeyBackend + 'static>(
    harness: &Harness<B>,
    rp: &str,
    credential_id: &[u8],
) -> RustCryptoSecretKey {
    let record = harness
        .store
        .record(&key_of(rp, credential_id))
        .expect("the store answers")
        .expect("the passkey is stored");
    let cose = CoseKey::from_slice(record.private_key.expose()).expect("a COSE key");
    RustCryptoSecretKey::from_cose_key(&cose).expect("an ES256 key")
}

/// Checks the assertion signature against the public key the passkey was registered with: the key
/// stored with the passkey must be the registered one, and must verify the signature over
/// `authenticatorData || clientDataHash`.
fn verify_signature<B: PasskeyBackend + 'static>(
    harness: &Harness<B>,
    rp: &str,
    registered_public_key: &CoseKey,
    response: &AssertResponse,
    client_data_hash: &[u8; 32],
) {
    let stored_public = secret_key_of(harness, rp, &response.credential_id).public_key();
    assert!(
        stored_public.to_cose_key() == *registered_public_key,
        "the stored key is the one that was registered"
    );
    let mut message = response.authenticator_data.clone();
    message.extend_from_slice(client_data_hash);
    stored_public
        .verify(&message, &response.signature)
        .expect("the signature verifies");
    let mut other = message.clone();
    other.push(0);
    assert!(
        stored_public.verify(&other, &response.signature).is_err(),
        "a different message must not verify"
    );
}

/// The private scalar `d` of a stored COSE_Key.
fn scalar_of(private_key: &PrivateKey) -> Vec<u8> {
    let cose = CoseKey::from_slice(private_key.expose()).expect("a COSE key");
    cose.params
        .iter()
        .find_map(|(label, value)| match (label, value) {
            (Label::Int(-4), Value::Bytes(bytes)) => Some(bytes.clone()),
            _ => None,
        })
        .expect("the key has a private scalar")
}

fn assert_does_not_show(text: &str, scalar: &[u8]) {
    let forms = [
        data_encoding::HEXLOWER.encode(scalar),
        data_encoding::HEXUPPER.encode(scalar),
        data_encoding::BASE64.encode(scalar),
        data_encoding::BASE64URL_NOPAD.encode(scalar),
        scalar
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    ];
    for form in forms {
        assert!(
            !text.contains(&form),
            "a Debug output shows the private key"
        );
    }
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the future was expected to complete at once"),
    }
}

fn parse_cbor(bytes: &[u8]) -> Value {
    cbor::de::from_reader(bytes).expect("valid CBOR")
}

fn map_get<'a>(value: &'a Value, name: &str) -> &'a Value {
    let Value::Map(entries) = value else {
        panic!("a CBOR map was expected");
    };
    entries
        .iter()
        .find(|(key, _)| matches!(key, Value::Text(text) if text == name))
        .map(|(_, entry)| entry)
        .unwrap_or_else(|| panic!("the map has no {name} entry"))
}

// ---- create, then assert -----------------------------------------------------------------------

#[test]
fn a_created_passkey_signs_an_assertion_that_verifies() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    assert_eq!(created.kind, AuthenticatorKind::BuiltIn);
    assert_eq!(created.attachment, Attachment::Platform);
    assert_eq!(created.transports, vec![Transport::Internal]);
    assert_eq!(created.public_key_algorithm, COSE_ES256);
    assert_eq!(created.credential_id.len(), 32);

    let request = assert_request(RP, vec![created.credential_id.clone()]);
    let asserted = h.authenticator.get_assertion(&request).expect("assertion");
    assert_eq!(asserted.kind, AuthenticatorKind::BuiltIn);
    assert_eq!(asserted.attachment, Attachment::Platform);
    assert_eq!(asserted.credential_id, created.credential_id);
    assert_eq!(asserted.user_handle.as_deref(), Some(&b"user-1"[..]));
    verify_signature(
        &h,
        RP,
        &public_key_of(&created),
        &asserted,
        &request.client_data_hash,
    );
}

#[test]
fn the_signature_covers_the_client_data_hash_it_was_given() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    let mut request = assert_request(RP, vec![created.credential_id.clone()]);
    request.client_data_hash = hash(0x5a);
    let asserted = h.authenticator.get_assertion(&request).expect("assertion");
    let public = secret_key_of(&h, RP, &created.credential_id).public_key();
    let mut message = asserted.authenticator_data.clone();
    message.extend_from_slice(&hash(0x5a));
    public
        .verify(&message, &asserted.signature)
        .expect("verifies over the hash that was given");
    let mut wrong = asserted.authenticator_data.clone();
    wrong.extend_from_slice(&hash(2));
    assert!(public.verify(&wrong, &asserted.signature).is_err());
}

#[test]
fn authenticator_data_has_the_rp_id_hash_the_flags_and_no_counter() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    let expected_hash = RustCryptoSha2::sha256(RP.as_bytes());
    // SHA-256("example.com"), computed independently with shasum.
    assert_eq!(
        data_encoding::HEXLOWER.encode(&expected_hash),
        "a379a6f6eeafb9a55e378c118034e2751e682fab9f2d30ab13d2125586ce1947"
    );

    let data = auth_data(&created.authenticator_data);
    assert_eq!(data.rp_id_hash(), expected_hash);
    assert!(data.flags.contains(Flags::UP));
    assert!(data.flags.contains(Flags::UV));
    assert!(data.flags.contains(Flags::AT));
    assert_eq!(created.authenticator_data[33..37], [0, 0, 0, 0]);
    let attested = data.attested_credential_data.expect("credential data");
    assert_eq!(attested.aaguid.0, [0u8; 16]);
    assert_eq!(attested.credential_id(), created.credential_id);

    let asserted = h
        .authenticator
        .get_assertion(&assert_request(RP, Vec::new()))
        .expect("assertion");
    let data = auth_data(&asserted.authenticator_data);
    assert_eq!(data.rp_id_hash(), expected_hash);
    assert!(data.flags.contains(Flags::UP));
    assert!(data.flags.contains(Flags::UV));
    assert!(!data.flags.contains(Flags::AT));
    assert_eq!(asserted.authenticator_data[33..37], [0, 0, 0, 0]);

    let stored = h.store.for_rp(RP).expect("list");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].counter, None);
    assert_eq!(stored[0].algorithm, COSE_ES256);
}

#[test]
fn a_new_passkey_is_backup_eligible_and_not_backed_up_on_create_and_on_get() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    assert_eq!(
        created.authenticator_data[32] & 0x08,
        0x08,
        "BE set on create"
    );
    assert_eq!(
        created.authenticator_data[32] & 0x10,
        0,
        "BS clear on create"
    );

    let request = assert_request(RP, vec![created.credential_id.clone()]);
    let asserted = h.authenticator.get_assertion(&request).expect("assertion");
    assert_eq!(
        asserted.authenticator_data[32] & 0x08,
        0x08,
        "BE set on get"
    );
    assert_eq!(asserted.authenticator_data[32] & 0x10, 0, "BS clear on get");

    let public = secret_key_of(&h, RP, &created.credential_id).public_key();
    let mut message = asserted.authenticator_data.clone();
    message.extend_from_slice(&request.client_data_hash);
    public
        .verify(&message, &asserted.signature)
        .expect("the signature covers the flags that were returned");
}

#[test]
fn the_attestation_object_is_cbor_with_format_none() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    let object = parse_cbor(&created.attestation_object);
    assert_eq!(map_get(&object, "fmt"), &Value::Text("none".into()));
    assert_eq!(map_get(&object, "attStmt"), &Value::Map(Vec::new()));
    assert_eq!(
        map_get(&object, "authData"),
        &Value::Bytes(created.authenticator_data.clone())
    );
}

#[test]
fn the_public_key_in_the_response_matches_the_stored_private_key() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    let record = h
        .store
        .record(&key_of(RP, &created.credential_id))
        .expect("store")
        .expect("stored");
    let cose = CoseKey::from_slice(record.private_key.expose()).expect("a COSE key");
    let secret = RustCryptoSecretKey::from_cose_key(&cose).expect("key");
    assert!(secret.public_key().to_cose_key() == public_key_of(&created));
    let spki = created.public_key_spki.expect("an SPKI");
    assert_eq!(
        spki,
        RustCryptoPublicKey::der_from_cose_key(&secret.public_key().to_cose_key()).expect("DER")
    );
    // An uncompressed P-256 SubjectPublicKeyInfo is 91 bytes.
    assert_eq!(spki.len(), 91);
}

#[test]
fn the_stored_passkey_keeps_what_the_page_gave() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    let stored = h.store.for_rp(RP).expect("list");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].key, key_of(RP, &created.credential_id));
    assert_eq!(stored[0].user_handle.as_deref(), Some(&b"user-1"[..]));
    assert_eq!(stored[0].username, "alice");
    assert_eq!(stored[0].display_name, "alice (display)");
}

#[test]
fn every_resident_key_preference_stores_a_listable_passkey() {
    for preference in [
        ResidentKey::Required,
        ResidentKey::Preferred,
        ResidentKey::Discouraged,
    ] {
        let h = harness();
        let mut request = create_request(RP, b"user-1", "alice");
        request.resident_key = preference;
        h.authenticator.make_credential(&request).expect("created");
        let stored = h.store.for_rp(RP).expect("list");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].user_handle.as_deref(), Some(&b"user-1"[..]));
    }
}

#[test]
fn credential_ids_differ_between_passkeys() {
    let h = harness();
    let first = create(&h, RP, b"user-1", "alice");
    let second = create(&h, RP, b"user-1", "alice");
    assert_ne!(first.credential_id, second.credential_id);
    assert_eq!(second.credential_id.len(), 32);
    assert_eq!(h.store.for_rp(RP).expect("list").len(), 2);
}

// ---- the user ----------------------------------------------------------------------------------

#[test]
fn declining_creation_stores_nothing() {
    let h = harness();
    h.verifier.push_answer(FakeAnswer::Decline);
    let error = h
        .authenticator
        .make_credential(&create_request(RP, b"user-1", "alice"))
        .expect_err("declined");
    assert_eq!(error, PasskeyError::NotAllowed);
    assert_eq!(error.dom_exception_name(), "NotAllowedError");
    assert!(h.store.all().expect("list").is_empty());
}

#[test]
fn creation_asks_the_user_about_the_site_and_the_account() {
    let h = harness();
    create(&h, RP, b"user-1", "alice");
    let requests = h.verifier.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].origin, ORIGIN);
    assert!(requests[0].require_presence);
    assert!(requests[0].require_verification);
    let VerificationPurpose::CreatePasskey { rp, user } = &requests[0].purpose else {
        panic!("a create prompt was expected");
    };
    assert_eq!(rp.id, RP);
    assert_eq!(user.id, b"user-1");
    assert_eq!(user.name, "alice");
    assert_eq!(user.display_name, "alice (display)");
}

#[test]
fn declining_an_assertion_gives_not_allowed() {
    let h = harness();
    create(&h, RP, b"user-1", "alice");
    h.verifier.push_answer(FakeAnswer::Decline);
    let error = h
        .authenticator
        .get_assertion(&assert_request(RP, Vec::new()))
        .expect_err("declined");
    assert_eq!(error, PasskeyError::NotAllowed);
}

#[test]
fn a_failed_verification_gives_not_allowed() {
    let h = harness();
    create(&h, RP, b"user-1", "alice");
    let mut request = assert_request(RP, Vec::new());
    request.user_verification = UserVerification::Required;
    h.verifier
        .push_answer(FakeAnswer::ApproveWithoutVerification);
    assert_eq!(
        h.authenticator
            .get_assertion(&request)
            .expect_err("refused"),
        PasskeyError::NotAllowed
    );
}

#[test]
fn an_assertion_asks_about_the_passkey_it_will_use() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    h.authenticator
        .get_assertion(&assert_request(RP, Vec::new()))
        .expect("assertion");
    let requests = h.verifier.requests();
    let VerificationPurpose::UsePasskey(summary) = &requests[1].purpose else {
        panic!("a use prompt was expected");
    };
    assert_eq!(summary.key, key_of(RP, &created.credential_id));
    assert_eq!(requests[1].origin, ORIGIN);
}

#[test]
fn a_verifier_error_is_passed_on_and_nothing_is_stored() {
    let h = harness();
    h.verifier
        .push_answer(FakeAnswer::Fail(PasskeyError::Cancelled));
    let error = h
        .authenticator
        .make_credential(&create_request(RP, b"user-1", "alice"))
        .expect_err("failed");
    assert_eq!(error, PasskeyError::Cancelled);
    assert_eq!(error.dom_exception_name(), "AbortError");
    assert!(h.store.all().expect("list").is_empty());

    create(&h, RP, b"user-1", "alice");
    h.verifier
        .push_answer(FakeAnswer::Fail(PasskeyError::Locked));
    assert_eq!(
        h.authenticator
            .get_assertion(&assert_request(RP, Vec::new()))
            .expect_err("failed"),
        PasskeyError::Locked
    );
}

#[test]
fn cancel_reaches_the_verifier() {
    let h = harness();
    h.authenticator.cancel();
    assert_eq!(h.verifier.cancel_calls(), 1);
}

// ---- nothing matches ---------------------------------------------------------------------------

#[test]
fn with_no_passkey_the_user_is_still_told_before_the_page_is() {
    let h = harness();
    let error = h
        .authenticator
        .get_assertion(&assert_request(RP, Vec::new()))
        .expect_err("nothing stored");
    assert_eq!(error, PasskeyError::NoCredentials);
    assert_eq!(error.dom_exception_name(), "NotAllowedError");
    let requests = h.verifier.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].purpose,
        VerificationPurpose::NoPasskeyFound {
            rp_id: RP.to_owned()
        }
    );
    assert_eq!(requests[0].origin, ORIGIN);
}

#[test]
fn if_the_user_dismisses_the_no_passkey_notice_the_answer_is_not_allowed() {
    let h = harness();
    h.verifier.push_answer(FakeAnswer::Decline);
    assert_eq!(
        h.authenticator
            .get_assertion(&assert_request(RP, Vec::new()))
            .expect_err("nothing stored"),
        PasskeyError::NotAllowed
    );
}

#[test]
fn a_passkey_for_one_site_is_not_used_for_another() {
    let h = harness();
    let created = create(&h, "a.example", b"user-1", "alice");

    let error = h
        .authenticator
        .get_assertion(&assert_request("b.example", Vec::new()))
        .expect_err("no passkey for b");
    assert_eq!(error, PasskeyError::NoCredentials);

    // Naming the other site's credential id does not help.
    let error = h
        .authenticator
        .get_assertion(&assert_request(
            "b.example",
            vec![created.credential_id.clone()],
        ))
        .expect_err("still no passkey for b");
    assert_eq!(error, PasskeyError::NoCredentials);

    // Neither does a parent or a child domain.
    for rp in ["example", "x.a.example", "sub.a.example"] {
        assert_eq!(
            h.authenticator
                .get_assertion(&assert_request(rp, Vec::new()))
                .expect_err("no passkey"),
            PasskeyError::NoCredentials
        );
    }

    // And the right site still works.
    h.authenticator
        .get_assertion(&assert_request("a.example", Vec::new()))
        .expect("assertion for a");
}

#[test]
fn the_same_user_at_two_sites_gets_two_unrelated_passkeys() {
    let h = harness();
    let at_a = create(&h, "a.example", b"user-1", "alice");
    let at_b = create(&h, "b.example", b"user-1", "alice");
    assert_ne!(
        public_key_of(&at_a).params,
        public_key_of(&at_b).params,
        "two passkeys must not share a key"
    );
    let signed = h
        .authenticator
        .get_assertion(&assert_request("b.example", Vec::new()))
        .expect("assertion");
    assert_eq!(signed.credential_id, at_b.credential_id);
    verify_signature(&h, "b.example", &public_key_of(&at_b), &signed, &hash(2));
}

// ---- several accounts, the allow list ----------------------------------------------------------

fn two_accounts(h: &Harness) -> (CreateResponse, CreateResponse) {
    let first = create(h, RP, b"user-1", "alice");
    sleep(Duration::from_millis(5));
    let second = create(h, RP, b"user-2", "bob");
    (first, second)
}

#[test]
fn two_accounts_make_the_user_pick_and_the_picked_one_signs() {
    let h = harness();
    let (alice, bob) = two_accounts(&h);
    let verifier_requests_before = h.verifier.requests().len();

    // The picker lists the newest first: bob, then alice.
    h.verifier.push_pick(FakePick::Index(1));
    let signed = h
        .authenticator
        .get_assertion(&assert_request(RP, Vec::new()))
        .expect("assertion");
    let calls = h.verifier.picker_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, ORIGIN);
    let offered: Vec<&PasskeyKey> = calls[0].1.iter().map(|summary| &summary.key).collect();
    assert_eq!(
        offered,
        vec![
            &key_of(RP, &bob.credential_id),
            &key_of(RP, &alice.credential_id)
        ]
    );
    assert_eq!(signed.credential_id, alice.credential_id);
    assert_eq!(signed.user_handle.as_deref(), Some(&b"user-1"[..]));
    verify_signature(&h, RP, &public_key_of(&alice), &signed, &hash(2));

    // After the picker the user is asked once, about the passkey they picked.
    let requests = h.verifier.requests();
    assert_eq!(requests.len(), verifier_requests_before + 1);
    let VerificationPurpose::UsePasskey(summary) = &requests[verifier_requests_before].purpose
    else {
        panic!("a use prompt was expected");
    };
    assert_eq!(summary.key, key_of(RP, &alice.credential_id));

    h.verifier.push_pick(FakePick::Index(0));
    let signed = h
        .authenticator
        .get_assertion(&assert_request(RP, Vec::new()))
        .expect("assertion");
    assert_eq!(signed.credential_id, bob.credential_id);
    verify_signature(&h, RP, &public_key_of(&bob), &signed, &hash(2));
}

#[test]
fn dismissing_the_picker_signs_nothing_and_asks_nothing_more() {
    let h = harness();
    two_accounts(&h);
    let before = h.verifier.requests().len();
    h.verifier.push_pick(FakePick::Dismiss);
    assert_eq!(
        h.authenticator
            .get_assertion(&assert_request(RP, Vec::new()))
            .expect_err("dismissed"),
        PasskeyError::NotAllowed
    );
    assert_eq!(h.verifier.requests().len(), before);
}

#[test]
fn a_picker_error_is_passed_on() {
    let h = harness();
    two_accounts(&h);
    h.verifier
        .push_pick(FakePick::Fail(PasskeyError::Cancelled));
    assert_eq!(
        h.authenticator
            .get_assertion(&assert_request(RP, Vec::new()))
            .expect_err("failed"),
        PasskeyError::Cancelled
    );
}

#[test]
fn a_pick_that_does_not_exist_is_refused() {
    let h = harness();
    two_accounts(&h);
    let before = h.verifier.requests().len();
    h.verifier.push_pick(FakePick::Index(2));
    assert!(matches!(
        h.authenticator
            .get_assertion(&assert_request(RP, Vec::new()))
            .expect_err("no such candidate"),
        PasskeyError::Internal(_)
    ));
    assert_eq!(h.verifier.requests().len(), before);
}

#[test]
fn one_passkey_needs_no_picker() {
    let h = harness();
    create(&h, RP, b"user-1", "alice");
    h.authenticator
        .get_assertion(&assert_request(RP, Vec::new()))
        .expect("assertion");
    assert!(h.verifier.picker_calls().is_empty());
}

#[test]
fn an_allow_list_selects_the_passkey_and_skips_the_picker() {
    let h = harness();
    let (alice, _bob) = two_accounts(&h);
    let request = assert_request(RP, vec![alice.credential_id.clone()]);
    let signed = h.authenticator.get_assertion(&request).expect("assertion");
    assert_eq!(signed.credential_id, alice.credential_id);
    assert!(h.verifier.picker_calls().is_empty());
    verify_signature(&h, RP, &public_key_of(&alice), &signed, &hash(2));
}

#[test]
fn an_allow_list_of_several_uses_the_newest_that_matches() {
    let h = harness();
    let (alice, bob) = two_accounts(&h);
    let request = assert_request(
        RP,
        vec![
            alice.credential_id.clone(),
            bob.credential_id.clone(),
            vec![9; 32],
        ],
    );
    let signed = h.authenticator.get_assertion(&request).expect("assertion");
    assert_eq!(signed.credential_id, bob.credential_id);
    assert!(h.verifier.picker_calls().is_empty());
}

#[test]
fn an_allow_list_naming_nothing_stored_finds_nothing() {
    let h = harness();
    two_accounts(&h);
    let request = assert_request(RP, vec![vec![9; 32]]);
    assert_eq!(
        h.authenticator
            .get_assertion(&request)
            .expect_err("unknown id"),
        PasskeyError::NoCredentials
    );
    assert!(h.verifier.picker_calls().is_empty());
}

// ---- exclude -----------------------------------------------------------------------------------

#[test]
fn an_excluded_credential_stops_creation_after_the_user_has_seen_it() {
    let h = harness();
    let existing = create(&h, RP, b"user-1", "alice");
    let before = h.verifier.requests().len();

    let mut request = create_request(RP, b"user-1", "alice");
    request.exclude = vec![existing.credential_id.clone()];
    let error = h
        .authenticator
        .make_credential(&request)
        .expect_err("excluded");
    assert_eq!(error, PasskeyError::CredentialExcluded);
    assert_eq!(error.dom_exception_name(), "InvalidStateError");

    assert_eq!(h.store.for_rp(RP).expect("list").len(), 1, "nothing new");
    let requests = h.verifier.requests();
    assert_eq!(requests.len(), before + 1, "one prompt, no creation prompt");
    assert!(requests[before].require_presence);
    let VerificationPurpose::ExcludedPasskeyFound(summary) = &requests[before].purpose else {
        panic!("an exclusion notice was expected");
    };
    assert_eq!(summary.key, key_of(RP, &existing.credential_id));
}

#[test]
fn an_excluded_credential_does_not_reach_the_page_without_the_user() {
    let h = harness();
    let existing = create(&h, RP, b"user-1", "alice");
    h.verifier.push_answer(FakeAnswer::Decline);
    let mut request = create_request(RP, b"user-1", "alice");
    request.exclude = vec![existing.credential_id.clone()];
    assert_eq!(
        h.authenticator
            .make_credential(&request)
            .expect_err("declined"),
        PasskeyError::NotAllowed
    );
}

#[test]
fn an_exclude_list_without_a_match_does_not_stop_creation() {
    let h = harness();
    let at_other_site = create(&h, "other.example", b"user-1", "alice");
    let mut request = create_request(RP, b"user-1", "alice");
    // One id that is stored for another site, and one that is stored nowhere.
    request.exclude = vec![at_other_site.credential_id.clone(), vec![7; 32]];
    h.authenticator
        .make_credential(&request)
        .expect("not excluded: the credential belongs to another site");
    assert_eq!(h.store.for_rp(RP).expect("list").len(), 1);
}

// ---- algorithms --------------------------------------------------------------------------------

#[test]
fn a_page_that_wants_only_other_algorithms_gets_unsupported_algorithm() {
    for algorithms in [
        vec![COSE_EDDSA],
        vec![COSE_RS256],
        vec![COSE_EDDSA, COSE_RS256],
        vec![-9999],
    ] {
        let h = harness();
        let mut request = create_request(RP, b"user-1", "alice");
        request.algorithms = algorithms;
        let error = h
            .authenticator
            .make_credential(&request)
            .expect_err("no common algorithm");
        assert_eq!(error, PasskeyError::UnsupportedAlgorithm);
        assert_eq!(error.dom_exception_name(), "NotSupportedError");
        assert!(h.verifier.requests().is_empty(), "the user is not asked");
        assert!(h.store.all().expect("list").is_empty());
    }
}

#[test]
fn es256_is_chosen_when_the_page_lists_it_after_others() {
    let h = harness();
    let mut request = create_request(RP, b"user-1", "alice");
    request.algorithms = vec![COSE_EDDSA, COSE_ES256, COSE_RS256];
    let created = h.authenticator.make_credential(&request).expect("created");
    assert_eq!(created.public_key_algorithm, COSE_ES256);
    assert_eq!(
        public_key_of(&created).alg,
        Some(passkey::crypto::iana::Algorithm::ES256.into())
    );
    assert_eq!(h.store.for_rp(RP).expect("list")[0].algorithm, COSE_ES256);
}

// ---- user verification -------------------------------------------------------------------------

fn without_verification() -> FakeVerifier {
    FakeVerifier::with_capabilities(VerifierCapabilities {
        presence: true,
        verification: None,
    })
}

#[test]
fn required_verification_without_a_capable_verifier_fails_before_the_user_is_asked() {
    for capabilities in [
        VerifierCapabilities {
            presence: true,
            verification: None,
        },
        VerifierCapabilities {
            presence: true,
            verification: Some(false),
        },
    ] {
        let h = harness_with(FakeVerifier::with_capabilities(capabilities));
        let mut request = create_request(RP, b"user-1", "alice");
        request.user_verification = UserVerification::Required;
        let error = h
            .authenticator
            .make_credential(&request)
            .expect_err("cannot verify");
        assert!(matches!(error, PasskeyError::UnsupportedOption(_)));
        assert_eq!(error.dom_exception_name(), "NotSupportedError");
        assert!(h.store.all().expect("list").is_empty());

        let mut request = assert_request(RP, Vec::new());
        request.user_verification = UserVerification::Required;
        assert!(matches!(
            h.authenticator
                .get_assertion(&request)
                .expect_err("cannot verify"),
            PasskeyError::UnsupportedOption(_)
        ));
        assert!(h.verifier.requests().is_empty(), "the user is not asked");
    }
}

#[test]
fn preferred_verification_is_asked_for_when_the_verifier_can_and_skipped_when_it_cannot() {
    let capable = harness();
    let created = create(&capable, RP, b"user-1", "alice");
    assert!(capable.verifier.requests()[0].require_verification);
    assert!(
        auth_data(&created.authenticator_data)
            .flags
            .contains(Flags::UV)
    );

    let incapable = harness_with(without_verification());
    let created = create(&incapable, RP, b"user-1", "alice");
    assert!(!incapable.verifier.requests()[0].require_verification);
    let flags = auth_data(&created.authenticator_data).flags;
    assert!(flags.contains(Flags::UP));
    assert!(!flags.contains(Flags::UV), "nobody verified the user");
    let signed = incapable
        .authenticator
        .get_assertion(&assert_request(RP, Vec::new()))
        .expect("assertion without verification");
    let flags = auth_data(&signed.authenticator_data).flags;
    assert!(flags.contains(Flags::UP));
    assert!(!flags.contains(Flags::UV));
}

#[test]
fn discouraged_verification_is_not_demanded_of_the_user() {
    let h = harness();
    let mut request = create_request(RP, b"user-1", "alice");
    request.user_verification = UserVerification::Discouraged;
    h.authenticator.make_credential(&request).expect("created");
    assert!(!h.verifier.requests()[0].require_verification);
    assert!(h.verifier.requests()[0].require_presence);
}

// ---- bad requests ------------------------------------------------------------------------------

#[test]
fn a_malformed_creation_request_is_refused_before_the_user_is_asked() {
    let mut cases: Vec<CreateRequest> = Vec::new();

    let mut request = create_request(RP, b"user-1", "alice");
    request.rp.id = "Example.COM".into();
    cases.push(request);

    let mut request = create_request(RP, b"user-1", "alice");
    request.rp.id = String::new();
    cases.push(request);

    cases.push(create_request(RP, b"", "alice"));
    cases.push(create_request(RP, &[1; 65], "alice"));
    cases.push(create_request(RP, b"user-1", "ali\nce"));
    cases.push(create_request(RP, b"user-1", &"x".repeat(2000)));

    let mut request = create_request(RP, b"user-1", "alice");
    request.algorithms = Vec::new();
    cases.push(request);

    for request in cases {
        let h = harness();
        let error = h
            .authenticator
            .make_credential(&request)
            .expect_err("refused");
        assert!(
            matches!(error, PasskeyError::InvalidInput(_)),
            "InvalidInput expected"
        );
        assert_eq!(error.dom_exception_name(), "TypeError");
        assert!(h.verifier.requests().is_empty(), "the user is not asked");
        assert!(h.store.all().expect("list").is_empty());
    }
}

#[test]
fn a_user_id_of_the_longest_allowed_size_is_accepted() {
    let h = harness();
    let created = create(&h, RP, &[7; 64], "alice");
    let signed = h
        .authenticator
        .get_assertion(&assert_request(RP, vec![created.credential_id]))
        .expect("assertion");
    assert_eq!(signed.user_handle, Some(vec![7; 64]));
}

#[test]
fn a_malformed_assertion_request_is_refused_before_the_user_is_asked() {
    let h = harness();
    for rp in ["", "Example.com", "a..b", "exa mple.com"] {
        let mut request = assert_request(RP, Vec::new());
        request.rp_id = rp.to_owned();
        assert!(matches!(
            h.authenticator
                .get_assertion(&request)
                .expect_err("refused"),
            PasskeyError::InvalidInput(_)
        ));
    }
    assert!(h.verifier.requests().is_empty());
}

// ---- storage failures --------------------------------------------------------------------------

/// A backend whose calls can be made to fail.
#[derive(Default)]
struct FlakyBackend {
    inner: MemoryBackend,
    fail_put: AtomicBool,
    fail_get: AtomicBool,
    fail_list: AtomicBool,
}

impl PasskeyBackend for FlakyBackend {
    fn put(&self, record: &PasskeyRecord) -> Result<SaveOutcome, PasskeyError> {
        if self.fail_put.load(Ordering::SeqCst) {
            return Err(PasskeyError::AccessDenied);
        }
        self.inner.put(record)
    }

    fn add(&self, record: &PasskeyRecord) -> Result<AddOutcome, PasskeyError> {
        if self.fail_put.load(Ordering::SeqCst) {
            return Err(PasskeyError::AccessDenied);
        }
        self.inner.add(record)
    }

    fn get(&self, key: &PasskeyKey) -> Result<Option<PasskeyRecord>, PasskeyError> {
        if self.fail_get.load(Ordering::SeqCst) {
            return Err(PasskeyError::Locked);
        }
        self.inner.get(key)
    }

    fn set_counter(&self, key: &PasskeyKey, counter: u32) -> Result<bool, PasskeyError> {
        self.inner.set_counter(key, counter)
    }

    fn delete(&self, key: &PasskeyKey) -> Result<bool, PasskeyError> {
        self.inner.delete(key)
    }

    fn list(&self) -> Result<Vec<PasskeySummary>, PasskeyError> {
        if self.fail_list.load(Ordering::SeqCst) {
            return Err(PasskeyError::Locked);
        }
        self.inner.list()
    }
}

#[test]
fn a_storage_error_while_saving_is_reported_as_it_is() {
    let h = harness_over(FlakyBackend::default(), FakeVerifier::new());
    h.store.backend().fail_put.store(true, Ordering::SeqCst);
    let error = h
        .authenticator
        .make_credential(&create_request(RP, b"user-1", "alice"))
        .expect_err("the store refuses");
    assert_eq!(error, PasskeyError::AccessDenied);
    h.store.backend().fail_put.store(false, Ordering::SeqCst);
    assert!(h.store.all().expect("list").is_empty());
}

#[test]
fn a_locked_store_while_signing_is_reported_after_the_user_approved() {
    let h = harness_over(FlakyBackend::default(), FakeVerifier::new());
    create(&h, RP, b"user-1", "alice");
    h.store.backend().fail_get.store(true, Ordering::SeqCst);
    let error = h
        .authenticator
        .get_assertion(&assert_request(RP, Vec::new()))
        .expect_err("the key cannot be read");
    assert_eq!(error, PasskeyError::Locked);
    let requests = h.verifier.requests();
    assert!(
        matches!(
            requests.last().map(|request| &request.purpose),
            Some(VerificationPurpose::UsePasskey(_))
        ),
        "the user was asked about the passkey, not told that none exists"
    );
}

#[test]
fn the_key_is_not_read_to_tell_the_user_about_an_excluded_passkey() {
    let h = harness_over(FlakyBackend::default(), FakeVerifier::new());
    let existing = create(&h, RP, b"user-1", "alice");
    h.store.backend().fail_get.store(true, Ordering::SeqCst);
    let mut request = create_request(RP, b"user-1", "alice");
    request.exclude = vec![existing.credential_id];
    assert_eq!(
        h.authenticator
            .make_credential(&request)
            .expect_err("excluded"),
        PasskeyError::CredentialExcluded
    );
}

#[test]
fn a_store_that_cannot_list_stops_the_call_without_a_prompt() {
    let h = harness_over(FlakyBackend::default(), FakeVerifier::new());
    h.store.backend().fail_list.store(true, Ordering::SeqCst);
    assert_eq!(
        h.authenticator
            .get_assertion(&assert_request(RP, Vec::new()))
            .expect_err("cannot list"),
        PasskeyError::Locked
    );
    assert!(h.verifier.requests().is_empty());
}

// ---- secrets -----------------------------------------------------------------------------------

#[test]
fn nothing_the_crate_prints_shows_the_private_key() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    let record = h
        .store
        .record(&key_of(RP, &created.credential_id))
        .expect("store")
        .expect("stored");
    let scalar = scalar_of(&record.private_key);
    assert_eq!(scalar.len(), 32);

    let signed = h
        .authenticator
        .get_assertion(&assert_request(RP, vec![created.credential_id.clone()]))
        .expect("assertion");
    let failure = h
        .authenticator
        .get_assertion(&assert_request("other.example", Vec::new()))
        .expect_err("nothing stored");

    let credential_store = PasskeyCredentialStore::new(Arc::clone(&h.store));
    let found = poll_ready(credential_store.find_credentials(None, RP, None)).expect("found");
    let validation: UserValidation<MemoryBackend> =
        UserValidation::new(h.verifier.clone(), ORIGIN, RP);

    let outputs = [
        format!("{record:?}"),
        format!("{:?}", record.private_key),
        format!("{:?}", h.authenticator),
        format!("{credential_store:?}"),
        format!("{validation:?}"),
        format!("{found:?}"),
        format!("{:?}", found[0]),
        format!("{:?}", MercuryCrypto::new()),
        format!("{:?}", h.verifier),
        format!("{:?}", h.verifier.requests()),
        format!("{:?}", h.store.all().expect("list")),
        format!("{created:?}"),
        format!("{signed:?}"),
        format!("{failure:?} {failure}"),
        format!("{:?}", create_request(RP, b"user-1", "alice")),
    ];
    for output in &outputs {
        assert_does_not_show(output, &scalar);
    }
}

#[test]
fn the_error_a_failed_call_returns_never_shows_a_name_or_an_id() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice-the-user");
    let mut request = create_request(RP, b"user-1", "alice-the-user");
    request.exclude = vec![created.credential_id.clone()];
    let error = h
        .authenticator
        .make_credential(&request)
        .expect_err("excluded");
    let shown = format!("{error:?} {error}");
    assert!(!shown.contains("alice"));
    assert!(!shown.contains(&data_encoding::HEXLOWER.encode(&created.credential_id)));
}

// ---- the credential store adapter, on its own --------------------------------------------------

fn passkey_for(rp: &str, credential_id: &[u8]) -> Passkey {
    use passkey::crypto::CryptoBackend;
    let key = MercuryCrypto::new()
        .generate_key(passkey::crypto::iana::Algorithm::ES256)
        .expect("a key");
    Passkey {
        key: key.to_cose_key(),
        credential_id: Bytes::from(credential_id),
        rp_id: rp.to_owned(),
        user_handle: Some(Bytes::from(&b"user-1"[..])),
        username: Some("alice".into()),
        user_display_name: Some("Alice".into()),
        counter: None,
        extensions: CredentialExtensions::default(),
    }
}

fn descriptor(id: &[u8], ty: PublicKeyCredentialType) -> PublicKeyCredentialDescriptor {
    PublicKeyCredentialDescriptor {
        ty,
        id: Bytes::from(id),
        transports: None,
    }
}

fn store_adapter() -> (
    Arc<PasskeyStore<MemoryBackend>>,
    PasskeyCredentialStore<MemoryBackend>,
) {
    let store = Arc::new(PasskeyStore::new(MemoryBackend::new()));
    let adapter = PasskeyCredentialStore::new(Arc::clone(&store));
    (store, adapter)
}

fn save(adapter: &mut PasskeyCredentialStore<MemoryBackend>, passkey: Passkey) {
    use passkey::types::ctap2::make_credential::{
        Options, PublicKeyCredentialRpEntity, PublicKeyCredentialUserEntity,
    };
    let rp = PublicKeyCredentialRpEntity {
        id: passkey.rp_id.clone(),
        name: None,
    };
    let user = PublicKeyCredentialUserEntity {
        id: Bytes::from(&b"user-1"[..]),
        name: None,
        display_name: None,
        icon_url: None,
    };
    poll_ready(adapter.save_credential(passkey, user, rp, Options::default())).expect("saved");
}

#[test]
fn the_adapter_refuses_a_credential_with_the_hmac_secret_extension() {
    use passkey::types::ctap2::make_credential::{
        Options, PublicKeyCredentialRpEntity, PublicKeyCredentialUserEntity,
    };
    let (store, mut adapter) = store_adapter();
    let mut passkey = passkey_for(RP, &[1; 32]);
    passkey.extensions = CredentialExtensions {
        hmac_secret: Some(StoredHmacSecret {
            cred_with_uv: vec![1; 32],
            cred_without_uv: None,
        }),
    };
    let rp = PublicKeyCredentialRpEntity {
        id: RP.into(),
        name: None,
    };
    let user = PublicKeyCredentialUserEntity {
        id: Bytes::from(&b"user-1"[..]),
        name: None,
        display_name: None,
        icon_url: None,
    };
    assert!(poll_ready(adapter.save_credential(passkey, user, rp, Options::default())).is_err());
    assert!(store.all().expect("list").is_empty());
}

#[test]
fn the_adapter_applies_the_stores_rules_when_saving() {
    use passkey::types::ctap2::make_credential::{
        Options, PublicKeyCredentialRpEntity, PublicKeyCredentialUserEntity,
    };
    let (store, mut adapter) = store_adapter();
    // A credential id of 8 bytes is below the store's minimum.
    let rp = PublicKeyCredentialRpEntity {
        id: RP.into(),
        name: None,
    };
    let user = PublicKeyCredentialUserEntity {
        id: Bytes::from(&b"user-1"[..]),
        name: None,
        display_name: None,
        icon_url: None,
    };
    let result =
        poll_ready(adapter.save_credential(passkey_for(RP, &[1; 8]), user, rp, Options::default()));
    assert!(result.is_err());
    assert!(store.all().expect("list").is_empty());
}

#[test]
fn the_adapter_finds_exact_site_matches_newest_first_and_filters_by_the_allow_list() {
    let (_store, mut adapter) = store_adapter();
    save(&mut adapter, passkey_for(RP, &[1; 32]));
    sleep(Duration::from_millis(5));
    save(&mut adapter, passkey_for(RP, &[2; 32]));
    save(&mut adapter, passkey_for("sub.example.com", &[3; 32]));

    let all = poll_ready(adapter.find_credentials(None, RP, None)).expect("found");
    let ids: Vec<&[u8]> = all.iter().map(|item| item.credential_id()).collect();
    assert_eq!(ids, vec![&[2u8; 32][..], &[1u8; 32][..]]);
    assert!(all.iter().all(|item| item.rp_id() == RP));
    assert_eq!(all[0].user_handle(), Some(&b"user-1"[..]));
    assert_eq!(all[0].username(), Some("alice"));
    assert_eq!(all[0].user_display_name(), Some("Alice"));
    assert_eq!(all[0].counter(), None);

    let allow = [
        descriptor(&[1; 32], PublicKeyCredentialType::PublicKey),
        descriptor(&[3; 32], PublicKeyCredentialType::PublicKey),
    ];
    let allowed = poll_ready(adapter.find_credentials(Some(&allow), RP, None)).expect("found");
    assert_eq!(allowed.len(), 1, "[3; 32] belongs to another site");
    assert_eq!(allowed[0].credential_id(), &[1u8; 32][..]);

    assert!(poll_ready(adapter.find_credentials(None, "example", None)).is_err());
    assert!(poll_ready(adapter.find_credentials(None, "other.example", None)).is_err());
}

#[test]
fn an_allow_list_of_unknown_credential_types_allows_nothing() {
    let (_store, mut adapter) = store_adapter();
    save(&mut adapter, passkey_for(RP, &[1; 32]));
    let allow = [descriptor(&[1; 32], PublicKeyCredentialType::Unknown)];
    assert!(poll_ready(adapter.find_credentials(Some(&allow), RP, None)).is_err());
}

#[test]
fn the_adapter_reads_the_key_only_when_asked_and_it_is_the_saved_key() {
    let (store, mut adapter) = store_adapter();
    let passkey = passkey_for(RP, &[1; 32]);
    let saved_key = passkey.key.clone();
    save(&mut adapter, passkey);
    let items = poll_ready(adapter.find_credentials(None, RP, None)).expect("found");
    let item: &StoredPasskey<MemoryBackend> = &items[0];
    assert_eq!(item.summary().key, key_of(RP, &[1; 32]));
    assert!(item.key().into_owned() == saved_key);

    // Once the passkey is gone, the key cannot be read: the item gives an unusable key.
    store
        .remove(&key_of(RP, &[1; 32]), &FakeVerifier::new())
        .expect("removed");
    assert!(item.key().into_owned() == CoseKey::default());
}

#[test]
fn the_adapter_reports_the_store_as_discoverable_only() {
    use passkey::authenticator::DiscoverabilitySupport;
    let (_store, adapter) = store_adapter();
    let info = poll_ready(adapter.get_info());
    assert!(info.discoverability == DiscoverabilitySupport::ForcedDiscoverable);
}

// ---- the key that was stored and the key that is used round-trip -------------------------------

#[test]
fn a_stored_private_key_round_trips_to_the_same_public_key() {
    let h = harness();
    let created = create(&h, RP, b"user-1", "alice");
    let record = h
        .store
        .record(&key_of(RP, &created.credential_id))
        .expect("store")
        .expect("stored");
    let cose = CoseKey::from_slice(record.private_key.expose()).expect("a COSE key");
    let secret = RustCryptoSecretKey::from_cose_key(&cose).expect("key");
    let reencoded = secret.to_cose_key();
    assert!(
        reencoded == cose,
        "decoding and encoding the key changes nothing"
    );
    assert!(secret.public_key().to_cose_key() == public_key_of(&created));
}

#[test]
fn the_manager_can_be_used_through_the_trait_object() {
    let h = harness();
    let authenticator: &dyn PasskeyAuthenticator = &h.authenticator;
    assert_eq!(authenticator.kind(), AuthenticatorKind::BuiltIn);
    let created = authenticator
        .make_credential(&create_request(RP, b"user-1", "alice"))
        .expect("created");
    authenticator
        .get_assertion(&assert_request(RP, vec![created.credential_id]))
        .expect("assertion");
}

#[test]
fn the_manager_is_usable_from_several_threads() {
    let h = Arc::new(harness());
    let handles: Vec<_> = (0..4u8)
        .map(|index| {
            let h = Arc::clone(&h);
            std::thread::spawn(move || {
                let rp = format!("site{index}.example");
                let created = create(&h, &rp, &[index + 1], "alice");
                h.authenticator
                    .get_assertion(&assert_request(&rp, vec![created.credential_id]))
                    .expect("assertion");
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("thread");
    }
    assert_eq!(h.store.all().expect("list").len(), 4);
}
