// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Security keys: the validation and answer checks of `SecurityKeyAuthenticator` against the fake
//! key, and on macOS the whole conversion path of `AuthenticatorRsDevice` against a fake transport
//! below `authenticator::AuthenticatorService`. No USB device is opened.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use mercury_passkeys::testing::{
    FakeKeyNotice, FakeKeyScript, FakePin, FakeSecurityKey, fake_assert_response,
    fake_create_response,
};
use mercury_passkeys::usb::{
    MAX_CREDENTIAL_LIST, SecurityKeyAuthenticator, SecurityKeyBlock, SecurityKeyUi,
};
use mercury_passkeys::{
    AssertRequest, AuthenticatorKind, CreateRequest, PasskeyAuthenticator, PasskeyError,
    RelyingParty, ResidentKey, UserEntity, UserVerification,
};
use zeroize::Zeroizing;

const PIN: &str = "135790";
const CREDENTIAL: [u8; 32] = [0x5a; 32];

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    SelectDevice,
    Touch,
    Pin(Option<u8>),
    PinRejected(Option<u8>),
    UvFailed(Option<u8>),
    Blocked(SecurityKeyBlock),
    Choose(usize),
    Cancelled,
}

enum PinAnswer {
    Give(&'static str),
    Cancel,
    UntilCancelled,
}

/// An interface that records what it was asked and answers from a script.
#[derive(Default)]
struct RecordingUi {
    events: Mutex<Vec<Event>>,
    pins: Mutex<VecDeque<PinAnswer>>,
    choices: Mutex<VecDeque<Option<usize>>>,
    cancelled: Mutex<bool>,
    wake: Condvar,
}

impl RecordingUi {
    fn with_pins(answers: Vec<PinAnswer>) -> Arc<Self> {
        let ui = Self::default();
        *lock(&ui.pins) = answers.into();
        Arc::new(ui)
    }

    fn with_choices(choices: Vec<Option<usize>>) -> Arc<Self> {
        let ui = Self::default();
        *lock(&ui.choices) = choices.into();
        Arc::new(ui)
    }

    fn events(&self) -> Vec<Event> {
        lock(&self.events).clone()
    }

    fn record(&self, event: Event) {
        lock(&self.events).push(event);
    }
}

impl SecurityKeyUi for RecordingUi {
    fn select_device_notice(&self) {
        self.record(Event::SelectDevice);
    }

    fn touch_notice(&self) {
        self.record(Event::Touch);
    }

    fn pin(&self, retries: Option<u8>) -> Option<Zeroizing<String>> {
        self.record(Event::Pin(retries));
        let answer = lock(&self.pins).pop_front();
        match answer {
            Some(PinAnswer::Give(pin)) => Some(Zeroizing::new(pin.to_owned())),
            Some(PinAnswer::UntilCancelled) => {
                let cancelled = lock(&self.cancelled);
                drop(
                    self.wake
                        .wait_while(cancelled, |cancelled| !*cancelled)
                        .unwrap_or_else(PoisonError::into_inner),
                );
                None
            }
            Some(PinAnswer::Cancel) | None => None,
        }
    }

    fn pin_rejected(&self, retries: Option<u8>) {
        self.record(Event::PinRejected(retries));
    }

    fn uv_failed(&self, retries: Option<u8>) {
        self.record(Event::UvFailed(retries));
    }

    fn blocked(&self, state: SecurityKeyBlock) {
        self.record(Event::Blocked(state));
    }

    fn choose_account(&self, accounts: &[UserEntity]) -> Option<usize> {
        self.record(Event::Choose(accounts.len()));
        lock(&self.choices).pop_front().flatten()
    }

    fn cancelled(&self) {
        self.record(Event::Cancelled);
        *lock(&self.cancelled) = true;
        self.wake.notify_all();
    }
}

fn user() -> UserEntity {
    UserEntity {
        id: vec![7; 16],
        name: "alice".to_owned(),
        display_name: "Alice Example".to_owned(),
    }
}

fn create_request() -> CreateRequest {
    CreateRequest {
        origin: "https://example.com".to_owned(),
        rp: RelyingParty {
            id: "example.com".to_owned(),
            name: Some("Example".to_owned()),
        },
        user: user(),
        client_data_hash: [0x11; 32],
        algorithms: vec![-7, -257],
        exclude: Vec::new(),
        resident_key: ResidentKey::Preferred,
        user_verification: UserVerification::Preferred,
        timeout_ms: Some(60_000),
    }
}

fn assert_request() -> AssertRequest {
    AssertRequest {
        origin: "https://example.com".to_owned(),
        rp_id: "example.com".to_owned(),
        client_data_hash: [0x22; 32],
        allow: vec![CREDENTIAL.to_vec()],
        user_verification: UserVerification::Preferred,
        timeout_ms: Some(60_000),
    }
}

fn authenticator(
    key: &FakeSecurityKey,
    ui: &Arc<RecordingUi>,
) -> SecurityKeyAuthenticator<FakeSecurityKey> {
    SecurityKeyAuthenticator::new(key.clone(), Arc::clone(ui) as Arc<dyn SecurityKeyUi>)
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn it_is_a_security_key_authenticator() {
    let key = FakeSecurityKey::new();
    let ui = RecordingUi::with_pins(Vec::new());
    assert_eq!(
        authenticator(&key, &ui).kind(),
        AuthenticatorKind::SecurityKey
    );
}

#[test]
fn create_reaches_the_key_unchanged_and_returns_its_answer() {
    let key = FakeSecurityKey::new();
    let expected = fake_create_response("example.com", &CREDENTIAL);
    key.push_create(FakeKeyScript::succeed(expected.clone()));
    let ui = RecordingUi::with_pins(Vec::new());

    let response = authenticator(&key, &ui)
        .make_credential(&create_request())
        .expect("create");

    assert_eq!(response, expected);
    assert_eq!(key.create_requests(), vec![create_request()]);
}

#[test]
fn assert_reaches_the_key_unchanged_and_returns_its_answer() {
    let key = FakeSecurityKey::new();
    let expected = fake_assert_response("example.com", &CREDENTIAL, Some(&[7; 16]));
    key.push_assert(FakeKeyScript::succeed(expected.clone()));
    let ui = RecordingUi::with_pins(Vec::new());

    let response = authenticator(&key, &ui)
        .get_assertion(&assert_request())
        .expect("assert");

    assert_eq!(response, expected);
    assert_eq!(key.assert_requests(), vec![assert_request()]);
}

#[test]
fn the_answer_is_marked_as_coming_from_a_security_key_whatever_the_device_said() {
    let key = FakeSecurityKey::new();
    let mut answer = fake_create_response("example.com", &CREDENTIAL);
    answer.kind = AuthenticatorKind::BuiltIn;
    key.push_create(FakeKeyScript::succeed(answer));
    let ui = RecordingUi::with_pins(Vec::new());

    let response = authenticator(&key, &ui)
        .make_credential(&create_request())
        .expect("create");
    assert_eq!(response.kind, AuthenticatorKind::SecurityKey);
}

#[test]
fn a_malformed_create_request_never_reaches_the_key() {
    let mut cases: Vec<(&str, CreateRequest)> = Vec::new();
    let mut request = create_request();
    request.rp.id = "Example.com".to_owned();
    cases.push(("upper-case rp id", request));
    let mut request = create_request();
    request.rp.id = String::new();
    cases.push(("empty rp id", request));
    let mut request = create_request();
    request.user.id = Vec::new();
    cases.push(("empty user id", request));
    let mut request = create_request();
    request.user.id = vec![1; 65];
    cases.push(("65-byte user id", request));
    let mut request = create_request();
    request.algorithms = Vec::new();
    cases.push(("no algorithms", request));
    let mut request = create_request();
    request.exclude = vec![Vec::new()];
    cases.push(("empty excluded id", request));
    let mut request = create_request();
    request.exclude = vec![vec![1; 1024]];
    cases.push(("oversized excluded id", request));
    let mut request = create_request();
    request.exclude = vec![vec![1; 16]; MAX_CREDENTIAL_LIST + 1];
    cases.push(("too many excluded ids", request));
    let mut request = create_request();
    request.user.name = "n".repeat(1025);
    cases.push(("oversized user name", request));

    let key = FakeSecurityKey::new();
    let ui = RecordingUi::with_pins(Vec::new());
    let authenticator = authenticator(&key, &ui);
    for (name, request) in cases {
        let error = authenticator
            .make_credential(&request)
            .expect_err("must be refused");
        assert!(
            matches!(error, PasskeyError::InvalidInput(_)),
            "{name}: {error}"
        );
        assert_eq!(error.dom_exception_name(), "TypeError", "{name}");
    }
    assert!(key.create_requests().is_empty(), "the key was contacted");
}

#[test]
fn a_malformed_assert_request_never_reaches_the_key() {
    let mut cases: Vec<(&str, AssertRequest)> = Vec::new();
    let mut request = assert_request();
    request.rp_id = "exa mple.com".to_owned();
    cases.push(("space in rp id", request));
    let mut request = assert_request();
    request.allow = vec![Vec::new()];
    cases.push(("empty allowed id", request));
    let mut request = assert_request();
    request.allow = vec![vec![1; 16]; MAX_CREDENTIAL_LIST + 1];
    cases.push(("too many allowed ids", request));

    let key = FakeSecurityKey::new();
    let ui = RecordingUi::with_pins(Vec::new());
    let authenticator = authenticator(&key, &ui);
    for (name, request) in cases {
        let error = authenticator
            .get_assertion(&request)
            .expect_err("must be refused");
        assert!(
            matches!(error, PasskeyError::InvalidInput(_)),
            "{name}: {error}"
        );
    }
    assert!(key.assert_requests().is_empty(), "the key was contacted");
}

#[test]
fn a_discoverable_assert_with_an_empty_allow_list_is_accepted() {
    let key = FakeSecurityKey::new();
    key.push_assert(FakeKeyScript::succeed(fake_assert_response(
        "example.com",
        &CREDENTIAL,
        Some(&[7; 16]),
    )));
    let ui = RecordingUi::with_pins(Vec::new());
    let mut request = assert_request();
    request.allow = Vec::new();

    let response = authenticator(&key, &ui)
        .get_assertion(&request)
        .expect("assert");
    assert_eq!(response.credential_id, CREDENTIAL.to_vec());
}

#[test]
fn an_answer_for_another_site_is_refused() {
    let key = FakeSecurityKey::new();
    key.push_create(FakeKeyScript::succeed(fake_create_response(
        "evil.example",
        &CREDENTIAL,
    )));
    key.push_assert(FakeKeyScript::succeed(fake_assert_response(
        "evil.example",
        &CREDENTIAL,
        None,
    )));
    let ui = RecordingUi::with_pins(Vec::new());
    let authenticator = authenticator(&key, &ui);

    assert!(matches!(
        authenticator.make_credential(&create_request()),
        Err(PasskeyError::DeviceFailure(_))
    ));
    assert!(matches!(
        authenticator.get_assertion(&assert_request()),
        Err(PasskeyError::DeviceFailure(_))
    ));
}

#[test]
fn malformed_answers_are_refused() {
    let key = FakeSecurityKey::new();
    let ui = RecordingUi::with_pins(Vec::new());
    let authenticator = authenticator(&key, &ui);

    let mut short = fake_create_response("example.com", &CREDENTIAL);
    short.authenticator_data.truncate(36);
    key.push_create(FakeKeyScript::succeed(short));
    let mut no_id = fake_create_response("example.com", &CREDENTIAL);
    no_id.credential_id = Vec::new();
    key.push_create(FakeKeyScript::succeed(no_id));
    let mut no_object = fake_create_response("example.com", &CREDENTIAL);
    no_object.attestation_object = Vec::new();
    key.push_create(FakeKeyScript::succeed(no_object));
    for _ in 0..3 {
        assert!(matches!(
            authenticator.make_credential(&create_request()),
            Err(PasskeyError::DeviceFailure(_))
        ));
    }

    let mut unsigned = fake_assert_response("example.com", &CREDENTIAL, None);
    unsigned.signature = Vec::new();
    key.push_assert(FakeKeyScript::succeed(unsigned));
    let long_handle = fake_assert_response("example.com", &CREDENTIAL, Some(&[1; 65]));
    key.push_assert(FakeKeyScript::succeed(long_handle));
    for _ in 0..2 {
        assert!(matches!(
            authenticator.get_assertion(&assert_request()),
            Err(PasskeyError::DeviceFailure(_))
        ));
    }
}

#[test]
fn an_assertion_with_a_credential_the_page_did_not_allow_is_refused() {
    let key = FakeSecurityKey::new();
    key.push_assert(FakeKeyScript::succeed(fake_assert_response(
        "example.com",
        &[0x99; 32],
        None,
    )));
    let ui = RecordingUi::with_pins(Vec::new());

    let error = authenticator(&key, &ui)
        .get_assertion(&assert_request())
        .expect_err("must be refused");
    assert!(matches!(error, PasskeyError::DeviceFailure(_)), "{error}");
}

#[test]
fn the_keys_errors_reach_the_caller_unchanged() {
    let errors = [
        PasskeyError::NoDevice,
        PasskeyError::NotAllowed,
        PasskeyError::Cancelled,
        PasskeyError::CredentialExcluded,
        PasskeyError::NoCredentials,
        PasskeyError::UnsupportedAlgorithm,
        PasskeyError::Device(0x32),
        PasskeyError::DeviceFailure("the security key failed"),
    ];
    let key = FakeSecurityKey::new();
    let ui = RecordingUi::with_pins(Vec::new());
    let authenticator = authenticator(&key, &ui);
    for error in errors {
        key.push_create(FakeKeyScript::fail(error.clone()));
        key.push_assert(FakeKeyScript::fail(error.clone()));
        assert_eq!(
            authenticator
                .make_credential(&create_request())
                .unwrap_err(),
            error
        );
        assert_eq!(
            authenticator.get_assertion(&assert_request()).unwrap_err(),
            error
        );
    }
}

#[test]
fn a_key_with_nothing_scripted_is_no_device() {
    let key = FakeSecurityKey::new();
    let ui = RecordingUi::with_pins(Vec::new());
    assert_eq!(
        authenticator(&key, &ui)
            .make_credential(&create_request())
            .unwrap_err(),
        PasskeyError::NoDevice
    );
}

#[test]
fn notices_reach_the_interface_in_order() {
    let key = FakeSecurityKey::new();
    key.push_assert(
        FakeKeyScript::succeed(fake_assert_response("example.com", &CREDENTIAL, None))
            .with_notices(&[
                FakeKeyNotice::SelectDevice,
                FakeKeyNotice::Touch,
                FakeKeyNotice::UvFailed(Some(2)),
                FakeKeyNotice::Blocked(SecurityKeyBlock::UvBlocked),
            ]),
    );
    let ui = RecordingUi::with_pins(Vec::new());

    authenticator(&key, &ui)
        .get_assertion(&assert_request())
        .expect("assert");
    assert_eq!(
        ui.events(),
        vec![
            Event::SelectDevice,
            Event::Touch,
            Event::UvFailed(Some(2)),
            Event::Blocked(SecurityKeyBlock::UvBlocked),
        ]
    );
}

#[test]
fn the_right_pin_lets_the_key_answer() {
    let key = FakeSecurityKey::new();
    key.push_create(
        FakeKeyScript::succeed(fake_create_response("example.com", &CREDENTIAL))
            .with_pin(FakePin::new(PIN, 8)),
    );
    let ui = RecordingUi::with_pins(vec![PinAnswer::Give(PIN)]);

    authenticator(&key, &ui)
        .make_credential(&create_request())
        .expect("create");
    assert_eq!(ui.events(), vec![Event::Pin(None)]);
}

#[test]
fn a_cancelled_pin_prompt_cancels_the_request() {
    let key = FakeSecurityKey::new();
    key.push_create(
        FakeKeyScript::succeed(fake_create_response("example.com", &CREDENTIAL))
            .with_pin(FakePin::new(PIN, 8)),
    );
    let ui = RecordingUi::with_pins(vec![PinAnswer::Cancel]);

    let error = authenticator(&key, &ui)
        .make_credential(&create_request())
        .unwrap_err();
    assert_eq!(error, PasskeyError::Cancelled);
    assert_eq!(error.dom_exception_name(), "AbortError");
}

#[test]
fn a_wrong_pin_is_retried_with_the_attempts_left() {
    let key = FakeSecurityKey::new();
    key.push_assert(
        FakeKeyScript::succeed(fake_assert_response("example.com", &CREDENTIAL, None))
            .with_pin(FakePin::new(PIN, 8)),
    );
    let ui = RecordingUi::with_pins(vec![
        PinAnswer::Give("000000"),
        PinAnswer::Give("111111"),
        PinAnswer::Give(PIN),
    ]);

    authenticator(&key, &ui)
        .get_assertion(&assert_request())
        .expect("assert");
    assert_eq!(
        ui.events(),
        vec![
            Event::Pin(None),
            Event::PinRejected(Some(7)),
            Event::Pin(Some(7)),
            Event::PinRejected(Some(6)),
            Event::Pin(Some(6)),
        ]
    );
}

#[test]
fn a_blocked_pin_ends_the_request_and_tells_the_interface() {
    let key = FakeSecurityKey::new();
    key.push_assert(
        FakeKeyScript::succeed(fake_assert_response("example.com", &CREDENTIAL, None))
            .with_pin(FakePin::new(PIN, 2)),
    );
    let ui = RecordingUi::with_pins(vec![
        PinAnswer::Give("000000"),
        PinAnswer::Give("111111"),
        PinAnswer::Give(PIN),
    ]);

    let error = authenticator(&key, &ui)
        .get_assertion(&assert_request())
        .unwrap_err();
    assert_eq!(error, PasskeyError::Device(0x32));
    assert_eq!(
        ui.events(),
        vec![
            Event::Pin(None),
            Event::PinRejected(Some(1)),
            Event::Pin(Some(1)),
            Event::Blocked(SecurityKeyBlock::PinBlocked),
        ],
        "the right PIN after the block must not have been asked for"
    );
}

#[test]
fn an_interface_that_cannot_ask_for_a_pin_cancels_instead_of_hanging() {
    struct Silent;
    impl SecurityKeyUi for Silent {}
    let key = FakeSecurityKey::new();
    key.push_create(
        FakeKeyScript::succeed(fake_create_response("example.com", &CREDENTIAL))
            .with_pin(FakePin::new(PIN, 8)),
    );
    let authenticator = SecurityKeyAuthenticator::new(key, Arc::new(Silent));

    assert_eq!(
        authenticator
            .make_credential(&create_request())
            .unwrap_err(),
        PasskeyError::Cancelled
    );
}

#[test]
fn a_key_nobody_touches_times_out_as_not_allowed() {
    let key = FakeSecurityKey::new();
    key.push_create(FakeKeyScript::wait_for_cancel());
    let ui = RecordingUi::with_pins(Vec::new());
    let mut request = create_request();
    request.timeout_ms = Some(50);

    let started = Instant::now();
    let error = authenticator(&key, &ui)
        .make_credential(&request)
        .unwrap_err();
    assert_eq!(error, PasskeyError::NotAllowed);
    assert!(started.elapsed() >= Duration::from_millis(50));
    assert_eq!(error.dom_exception_name(), "NotAllowedError");
}

#[test]
fn cancel_from_another_thread_ends_a_waiting_request() {
    let key = FakeSecurityKey::new();
    key.push_create(FakeKeyScript::wait_for_cancel());
    let ui = RecordingUi::with_pins(Vec::new());
    let authenticator = Arc::new(authenticator(&key, &ui));

    let worker = {
        let authenticator = Arc::clone(&authenticator);
        thread::spawn(move || authenticator.make_credential(&create_request()))
    };
    wait_until("the request to reach the key", || {
        !key.create_requests().is_empty()
    });
    authenticator.cancel();

    assert_eq!(worker.join().expect("worker"), Err(PasskeyError::Cancelled));
    assert_eq!(key.cancel_count(), 1);
    assert_eq!(ui.events(), vec![Event::Cancelled]);
}

#[test]
fn cancel_with_nothing_waiting_does_not_spoil_the_next_request() {
    let key = FakeSecurityKey::new();
    key.push_create(FakeKeyScript::succeed(fake_create_response(
        "example.com",
        &CREDENTIAL,
    )));
    let ui = RecordingUi::with_pins(Vec::new());
    let authenticator = authenticator(&key, &ui);

    authenticator.cancel();
    authenticator
        .make_credential(&create_request())
        .expect("a stale cancel must not fail the next request");
}

#[test]
fn nothing_secret_shows_in_debug_or_error_text() {
    let key = FakeSecurityKey::new();
    key.push_create(
        FakeKeyScript::succeed(fake_create_response("example.com", &CREDENTIAL))
            .with_pin(FakePin::new(PIN, 3)),
    );
    let ui = RecordingUi::with_pins(vec![PinAnswer::Give("246810"), PinAnswer::Cancel]);
    let pin = FakePin::new(PIN, 3);
    let authenticator = authenticator(&key, &ui);

    let error = authenticator
        .make_credential(&create_request())
        .unwrap_err();
    let shown = format!(
        "{key:?} {authenticator:?} {pin:?} {error} {error:?} {:?} {:?}",
        create_request(),
        user()
    );
    for secret in [PIN, "246810", "alice", "Alice Example"] {
        assert!(!shown.contains(secret), "debug output holds a secret");
    }
}

#[cfg(target_os = "macos")]
mod hid {
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Wake, Waker};

    use authenticator::authenticatorservice::{AuthenticatorService, RegisterArgs, SignArgs};
    use authenticator::crypto::COSEAlgorithm;
    use authenticator::ctap2::commands::StatusCode;
    use authenticator::ctap2::server::{ResidentKeyRequirement, UserVerificationRequirement};
    use authenticator::errors::{
        AuthenticatorError, CommandError, HIDError, PinError, U2FTokenError, UnsupportedOption,
    };
    use mercury_passkeys::testing::{
        FakeStatus, FakeTransport, FakeTransportControl, TransportScript, canned_register_result,
        canned_sign_result,
    };
    use mercury_passkeys::usb_hid::AuthenticatorRsDevice;
    use mercury_passkeys::{Attachment, COSE_ES256, Transport};
    use passkey::authenticator::{
        Authenticator, MemoryStore, UiHint, UserCheck, UserValidationMethod,
    };
    use passkey::crypto::coset::CoseKey;
    use passkey::crypto::coset::cbor::{Value, de::from_reader, ser::into_writer};
    use passkey::crypto::hash::Sha256Backend;
    use passkey::crypto::rust_crypto::{RustCryptoBackend, RustCryptoSecretKey, RustCryptoSha2};
    use passkey::crypto::{PublicKeyT, SecretKeyT};
    use passkey::types::ctap2::{Aaguid, Ctap2Error, get_assertion, make_credential};
    use passkey::types::webauthn::{
        PublicKeyCredentialDescriptor, PublicKeyCredentialParameters, PublicKeyCredentialType,
        PublicKeyCredentialUserEntity,
    };
    use passkey::types::{Bytes, Passkey};

    use super::*;

    // A software authenticator that makes the fixtures: real attestation objects, real
    // authenticator data and real signatures for the fake transport to hand back.

    struct AlwaysYes;

    #[async_trait::async_trait]
    impl UserValidationMethod for AlwaysYes {
        type PasskeyItem = Passkey;

        async fn check_user<'a>(
            &self,
            _hint: UiHint<'a, Self::PasskeyItem>,
            _presence: bool,
            _verification: bool,
        ) -> Result<UserCheck, Ctap2Error> {
            Ok(UserCheck {
                presence: true,
                verification: true,
            })
        }

        fn is_presence_enabled(&self) -> bool {
            true
        }

        fn is_verification_enabled(&self) -> Option<bool> {
            Some(true)
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        struct Unpark(thread::Thread);
        impl Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Unpark(thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = pin!(future);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
            thread::park();
        }
    }

    struct Made {
        credential_id: Vec<u8>,
        attestation_object: Vec<u8>,
        authenticator_data: Vec<u8>,
        public_cose: CoseKey,
    }

    struct Software {
        authenticator: Authenticator<MemoryStore, AlwaysYes, RustCryptoBackend>,
    }

    impl Software {
        fn new() -> Self {
            Self {
                authenticator: Authenticator::new(
                    Aaguid::new_empty(),
                    MemoryStore::new(),
                    AlwaysYes,
                    RustCryptoBackend,
                ),
            }
        }

        fn make(&mut self, rp_id: &str, user_id: &[u8]) -> Made {
            let request = make_credential::Request {
                client_data_hash: Bytes::from(vec![0x11; 32]),
                rp: make_credential::PublicKeyCredentialRpEntity {
                    id: rp_id.to_owned(),
                    name: None,
                },
                user: PublicKeyCredentialUserEntity {
                    id: Bytes::from(user_id.to_vec()),
                    name: "alice".to_owned(),
                    display_name: "Alice Example".to_owned(),
                },
                pub_key_cred_params: vec![PublicKeyCredentialParameters {
                    ty: PublicKeyCredentialType::PublicKey,
                    alg: passkey::crypto::iana::Algorithm::ES256,
                }],
                exclude_list: None,
                extensions: None,
                options: make_credential::Options {
                    rk: true,
                    up: true,
                    uv: true,
                },
                pin_auth: None,
                pin_protocol: None,
            };
            let response = block_on(self.authenticator.make_credential(request))
                .expect("the software authenticator makes a credential");
            let authenticator_data = response.auth_data.to_vec();
            let credential_id = response
                .auth_data
                .attested_credential_data
                .as_ref()
                .expect("attested credential data")
                .credential_id()
                .to_vec();
            let stored = self
                .authenticator
                .store()
                .get(credential_id.as_slice())
                .expect("the credential is stored");
            let public_cose = RustCryptoSecretKey::from_cose_key(&stored.key)
                .expect("a usable key")
                .public_key()
                .to_cose_key();
            Made {
                credential_id,
                attestation_object: response.as_webauthn_bytes().to_vec(),
                authenticator_data,
                public_cose,
            }
        }

        /// A real assertion: (authenticator data, signature, user handle).
        fn assert(
            &mut self,
            rp_id: &str,
            credential_id: &[u8],
            client_data_hash: [u8; 32],
        ) -> (Vec<u8>, Vec<u8>, Option<Vec<u8>>) {
            let request = get_assertion::Request {
                rp_id: rp_id.to_owned(),
                client_data_hash: Bytes::from(client_data_hash.to_vec()),
                allow_list: Some(vec![PublicKeyCredentialDescriptor {
                    ty: PublicKeyCredentialType::PublicKey,
                    id: Bytes::from(credential_id.to_vec()),
                    transports: None,
                }]),
                extensions: None,
                options: get_assertion::Options { up: true, uv: true },
                pin_auth: None,
                pin_protocol: None,
            };
            let response = block_on(self.authenticator.get_assertion(request))
                .expect("the software authenticator asserts");
            (
                response.auth_data.to_vec(),
                response.signature.to_vec(),
                response.user.map(|user| user.id.to_vec()),
            )
        }
    }

    fn cbor(value: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        into_writer(value, &mut out).expect("encode");
        out
    }

    fn decode(bytes: &[u8]) -> Value {
        from_reader(bytes).expect("decode")
    }

    fn map_get<'a>(value: &'a Value, key: &str) -> &'a Value {
        let Value::Map(entries) = value else {
            panic!("not a map");
        };
        entries
            .iter()
            .find(|(name, _)| matches!(name, Value::Text(text) if text == key))
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("no entry {key}"))
    }

    struct Rig {
        device: SecurityKeyAuthenticator<AuthenticatorRsDevice>,
        control: FakeTransportControl,
        ui: Arc<RecordingUi>,
    }

    fn rig_with(ui: Arc<RecordingUi>) -> Rig {
        let (transport, control) = FakeTransport::new();
        let mut service = AuthenticatorService::new().expect("service");
        service.add_transport(Box::new(transport));
        let device = SecurityKeyAuthenticator::new(
            AuthenticatorRsDevice::with_service(service),
            Arc::clone(&ui) as Arc<dyn SecurityKeyUi>,
        );
        Rig {
            device,
            control,
            ui,
        }
    }

    fn rig() -> Rig {
        rig_with(RecordingUi::with_pins(Vec::new()))
    }

    fn software_create(rp_id: &str) -> (Software, Made) {
        let mut software = Software::new();
        let made = software.make(rp_id, &[7; 16]);
        (software, made)
    }

    #[test]
    fn a_created_credential_converts_to_what_the_page_needs() {
        let (_software, made) = software_create("example.com");
        let rig = rig();
        rig.control.push_register(TransportScript::succeed(
            canned_register_result(&made.attestation_object).expect("canned result"),
        ));

        let response = rig
            .device
            .make_credential(&create_request())
            .expect("create");

        assert_eq!(response.kind, AuthenticatorKind::SecurityKey);
        assert_eq!(response.credential_id, made.credential_id);
        assert_eq!(response.attachment, Attachment::CrossPlatform);
        assert_eq!(response.transports, vec![Transport::Usb]);
        assert_eq!(response.public_key_algorithm, COSE_ES256);

        // The attestation object is `none`, an all-zero AAGUID, and carries the same authenticator
        // data as the response says (both libraries' CBOR agree).
        let object = decode(&response.attestation_object);
        assert_eq!(map_get(&object, "fmt"), &Value::Text("none".to_owned()));
        assert_eq!(map_get(&object, "attStmt"), &Value::Map(Vec::new()));
        assert_eq!(
            map_get(&object, "authData"),
            &Value::Bytes(response.authenticator_data.clone())
        );
        assert_eq!(
            response.authenticator_data, made.authenticator_data,
            "authenticator-rs must write the same authenticator data the software one made"
        );
        assert_eq!(
            response.authenticator_data[..32],
            RustCryptoSha2::sha256(b"example.com")
        );
        assert_eq!(&response.authenticator_data[37..53], &[0; 16], "AAGUID");

        // The SPKI is the one the software authenticator's own conversion gives.
        let expected_spki =
            <passkey::crypto::rust_crypto::RustCryptoPublicKey as PublicKeyT>::der_from_cose_key(
                &made.public_cose,
            )
            .expect("spki");
        assert_eq!(response.public_key_spki, Some(expected_spki));
    }

    #[test]
    fn the_attestation_of_a_real_key_is_dropped() {
        let (_software, made) = software_create("example.com");
        // Rewrite the software attestation object as a `packed` one with a made-up statement and
        // a non-zero AAGUID, the way a key with a batch certificate answers.
        let mut authenticator_data = made.authenticator_data.clone();
        authenticator_data[37..53].copy_from_slice(&[0xAA; 16]);
        let text = |value: &str| Value::Text(value.to_owned());
        let packed = cbor(&Value::Map(vec![
            (text("fmt"), text("packed")),
            (
                text("attStmt"),
                Value::Map(vec![
                    (text("alg"), Value::Integer((-7).into())),
                    (text("sig"), Value::Bytes(vec![0x30; 70])),
                    (
                        text("x5c"),
                        Value::Array(vec![Value::Bytes(vec![0x30; 40])]),
                    ),
                ]),
            ),
            (text("authData"), Value::Bytes(authenticator_data)),
        ]));
        let rig = rig();
        rig.control.push_register(TransportScript::succeed(
            canned_register_result(&packed).expect("canned result"),
        ));

        let response = rig
            .device
            .make_credential(&create_request())
            .expect("create");

        let object = decode(&response.attestation_object);
        assert_eq!(map_get(&object, "fmt"), &Value::Text("none".to_owned()));
        assert_eq!(map_get(&object, "attStmt"), &Value::Map(Vec::new()));
        assert_eq!(&response.authenticator_data[37..53], &[0; 16]);
        let shown = format!("{:x?}", response.attestation_object);
        assert!(!shown.contains("aa, aa, aa, aa"), "the batch AAGUID leaked");
    }

    #[test]
    fn a_signature_made_by_a_real_authenticator_survives_the_conversion() {
        let (mut software, made) = software_create("example.com");
        let hash = [0x22; 32];
        let (authenticator_data, signature, user_handle) =
            software.assert("example.com", &made.credential_id, hash);
        let rig = rig();
        rig.control.push_sign(TransportScript::succeed(
            canned_sign_result(
                &authenticator_data,
                &signature,
                &made.credential_id,
                user_handle
                    .as_ref()
                    .map(|id| UserEntity {
                        id: id.clone(),
                        name: "alice".to_owned(),
                        display_name: String::new(),
                    })
                    .as_ref(),
            )
            .expect("canned result"),
        ));
        let mut request = assert_request();
        request.allow = vec![made.credential_id.clone()];
        request.client_data_hash = hash;

        let response = rig.device.get_assertion(&request).expect("assert");

        assert_eq!(response.credential_id, made.credential_id);
        assert_eq!(response.signature, signature);
        assert_eq!(response.authenticator_data, authenticator_data);
        assert_eq!(response.user_handle, Some(vec![7; 16]));
        assert_eq!(response.attachment, Attachment::CrossPlatform);
        // What a relying party does: verify over authData || clientDataHash with the public key.
        let public = RustCryptoSecretKey::from_cose_key(
            &software
                .authenticator
                .store()
                .get(made.credential_id.as_slice())
                .expect("stored")
                .key,
        )
        .expect("key")
        .public_key();
        let mut signed = response.authenticator_data.clone();
        signed.extend_from_slice(&hash);
        public
            .verify(&signed, &response.signature)
            .expect("the converted assertion verifies");
    }

    #[test]
    fn a_key_that_leaves_out_the_credential_id_gets_the_one_allowed() {
        let (mut software, made) = software_create("example.com");
        let (authenticator_data, signature, _) =
            software.assert("example.com", &made.credential_id, [0x22; 32]);
        let rig = rig();
        let mut result =
            canned_sign_result(&authenticator_data, &signature, &made.credential_id, None)
                .expect("canned result");
        result.assertion.credentials = None;
        rig.control.push_sign(TransportScript::succeed(result));
        let mut request = assert_request();
        request.allow = vec![made.credential_id.clone()];

        let response = rig.device.get_assertion(&request).expect("assert");
        assert_eq!(response.credential_id, made.credential_id);
        assert_eq!(response.user_handle, None);
    }

    #[test]
    fn a_key_that_never_says_which_credential_signed_is_refused() {
        let (mut software, made) = software_create("example.com");
        let (authenticator_data, signature, _) =
            software.assert("example.com", &made.credential_id, [0x22; 32]);
        let rig = rig();
        let mut result =
            canned_sign_result(&authenticator_data, &signature, &made.credential_id, None)
                .expect("canned result");
        result.assertion.credentials = None;
        rig.control.push_sign(TransportScript::succeed(result));
        let mut request = assert_request();
        request.allow = Vec::new();

        assert!(matches!(
            rig.device.get_assertion(&request),
            Err(PasskeyError::DeviceFailure(_))
        ));
    }

    fn register_ok(rig: &Rig) {
        let (_software, made) = software_create("example.com");
        rig.control.push_register(TransportScript::succeed(
            canned_register_result(&made.attestation_object).expect("canned result"),
        ));
    }

    fn sent_register(rig: &Rig, request: &CreateRequest) -> RegisterArgs {
        register_ok(rig);
        // The software credential's id is not in the request's exclude list, so this succeeds.
        let _ = rig.device.make_credential(request);
        rig.control
            .register_args()
            .pop()
            .expect("the transport saw the call")
    }

    fn sent_sign(rig: &Rig, request: &AssertRequest) -> SignArgs {
        let _ = rig.device.get_assertion(request);
        rig.control
            .sign_args()
            .pop()
            .expect("the transport saw the call")
    }

    #[test]
    fn a_create_request_converts_to_register_arguments() {
        let rig = rig();
        let mut request = create_request();
        request.algorithms = vec![-8, -7, 9999, -257, -65535, 1];
        request.exclude = vec![vec![1; 16], vec![2; 40]];
        request.resident_key = ResidentKey::Required;
        request.user_verification = UserVerification::Required;

        let args = sent_register(&rig, &request);

        assert_eq!(args.client_data_hash, [0x11; 32]);
        assert_eq!(args.relying_party.id, "example.com");
        assert_eq!(args.relying_party.name.as_deref(), Some("Example"));
        assert_eq!(args.origin, "https://example.com");
        assert_eq!(args.user.id, vec![7; 16]);
        assert_eq!(args.user.name.as_deref(), Some("alice"));
        assert_eq!(args.user.display_name.as_deref(), Some("Alice Example"));
        // Signature algorithms the library knows, in the page's order; the rest are not offered.
        assert_eq!(
            args.pub_cred_params
                .iter()
                .map(|parameter| parameter.alg)
                .collect::<Vec<_>>(),
            vec![
                COSEAlgorithm::EDDSA,
                COSEAlgorithm::ES256,
                COSEAlgorithm::RS256
            ]
        );
        assert_eq!(
            args.exclude_list
                .iter()
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>(),
            vec![vec![1; 16], vec![2; 40]]
        );
        assert_eq!(args.resident_key_req, ResidentKeyRequirement::Required);
        assert_eq!(
            args.user_verification_req,
            UserVerificationRequirement::Required
        );
        assert!(args.pin.is_none(), "no PIN is sent before the key asks");
        assert!(!args.use_ctap1_fallback);
    }

    #[test]
    fn resident_key_and_verification_preferences_map_one_to_one() {
        for (resident, expected) in [
            (ResidentKey::Required, ResidentKeyRequirement::Required),
            (ResidentKey::Preferred, ResidentKeyRequirement::Preferred),
            (
                ResidentKey::Discouraged,
                ResidentKeyRequirement::Discouraged,
            ),
        ] {
            for (verification, wanted) in [
                (
                    UserVerification::Required,
                    UserVerificationRequirement::Required,
                ),
                (
                    UserVerification::Preferred,
                    UserVerificationRequirement::Preferred,
                ),
                (
                    UserVerification::Discouraged,
                    UserVerificationRequirement::Discouraged,
                ),
            ] {
                let rig = rig();
                let mut request = create_request();
                request.resident_key = resident;
                request.user_verification = verification;
                let args = sent_register(&rig, &request);
                assert_eq!(args.resident_key_req, expected);
                assert_eq!(args.user_verification_req, wanted);
            }
        }
    }

    #[test]
    fn a_page_that_offers_no_algorithm_a_key_can_use_never_reaches_the_key() {
        let rig = rig();
        let mut request = create_request();
        request.algorithms = vec![9999, 1, -65535];

        assert_eq!(
            rig.device.make_credential(&request).unwrap_err(),
            PasskeyError::UnsupportedAlgorithm
        );
        assert!(rig.control.register_args().is_empty());
    }

    #[test]
    fn an_assert_request_converts_to_sign_arguments() {
        let rig = rig();
        let mut request = assert_request();
        request.allow = vec![vec![3; 20], vec![4; 64]];
        request.user_verification = UserVerification::Discouraged;

        let args = sent_sign(&rig, &request);

        assert_eq!(args.client_data_hash, [0x22; 32]);
        assert_eq!(args.relying_party_id, "example.com");
        assert_eq!(args.origin, "https://example.com");
        assert_eq!(
            args.allow_list
                .iter()
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>(),
            vec![vec![3; 20], vec![4; 64]]
        );
        assert_eq!(
            args.user_verification_req,
            UserVerificationRequirement::Discouraged
        );
        assert!(args.user_presence_req, "a touch is always required");
        assert!(args.pin.is_none());
        assert!(!args.use_ctap1_fallback);
    }

    #[test]
    fn the_timeout_is_the_pages_with_a_default_and_a_cap() {
        let rig = rig();
        for (asked, used) in [
            (None, 120_000),
            (Some(0), 120_000),
            (Some(5_000), 5_000),
            (Some(300_000), 300_000),
            (Some(u32::MAX), 300_000),
        ] {
            let mut request = assert_request();
            request.timeout_ms = asked;
            let _ = rig.device.get_assertion(&request);
            assert_eq!(
                rig.control.timeouts().last().copied(),
                Some(used),
                "{asked:?}"
            );
        }
    }

    fn failure_of(error: AuthenticatorError) -> PasskeyError {
        let rig = rig();
        rig.control.push_register(TransportScript::fail(error));
        rig.device.make_credential(&create_request()).unwrap_err()
    }

    fn status(code: StatusCode) -> AuthenticatorError {
        AuthenticatorError::HIDError(HIDError::Command(CommandError::StatusCode(code, None)))
    }

    #[test]
    fn every_library_error_maps_to_the_right_passkey_error() {
        let cases: Vec<(&str, AuthenticatorError, PasskeyError)> = vec![
            (
                "cancelled",
                AuthenticatorError::CancelledByUser,
                PasskeyError::Cancelled,
            ),
            (
                "excluded",
                AuthenticatorError::CredentialExcluded,
                PasskeyError::CredentialExcluded,
            ),
            (
                "timeout",
                AuthenticatorError::U2FToken(U2FTokenError::NotAllowed),
                PasskeyError::NotAllowed,
            ),
            (
                "excluded status",
                status(StatusCode::CredentialExcluded),
                PasskeyError::CredentialExcluded,
            ),
            (
                "no credentials",
                status(StatusCode::NoCredentials),
                PasskeyError::NoCredentials,
            ),
            (
                "unsupported algorithm",
                status(StatusCode::UnsupportedAlgorithm),
                PasskeyError::UnsupportedAlgorithm,
            ),
            (
                "operation denied",
                status(StatusCode::OperationDenied),
                PasskeyError::NotAllowed,
            ),
            (
                "user action timeout",
                status(StatusCode::UserActionTimeout),
                PasskeyError::NotAllowed,
            ),
            (
                "action timeout",
                status(StatusCode::ActionTimeout),
                PasskeyError::NotAllowed,
            ),
            (
                "keepalive cancel",
                status(StatusCode::KeepaliveCancel),
                PasskeyError::Cancelled,
            ),
            (
                "pin blocked",
                status(StatusCode::PinBlocked),
                PasskeyError::Device(0x32),
            ),
            (
                "pin not set",
                status(StatusCode::PinNotSet),
                PasskeyError::Device(0x35),
            ),
            (
                "unknown status",
                status(StatusCode::Unknown(0x99)),
                PasskeyError::Device(0x99),
            ),
            (
                "pin error: blocked",
                AuthenticatorError::PinError(PinError::PinBlocked),
                PasskeyError::Device(0x32),
            ),
            (
                "pin error: auth blocked",
                AuthenticatorError::PinError(PinError::PinAuthBlocked),
                PasskeyError::Device(0x34),
            ),
            (
                "pin error: invalid",
                AuthenticatorError::PinError(PinError::InvalidPin(Some(3))),
                PasskeyError::Device(0x31),
            ),
            (
                "pin error: not set",
                AuthenticatorError::PinError(PinError::PinNotSet),
                PasskeyError::Device(0x35),
            ),
            (
                "pin error: uv blocked",
                AuthenticatorError::PinError(PinError::UvBlocked),
                PasskeyError::Device(0x3C),
            ),
            (
                "unsupported: algorithm",
                AuthenticatorError::UnsupportedOption(UnsupportedOption::PubCredParams),
                PasskeyError::UnsupportedAlgorithm,
            ),
        ];
        for (name, error, expected) in cases {
            assert_eq!(failure_of(error), expected, "{name}");
        }
        assert!(matches!(
            failure_of(AuthenticatorError::UnsupportedOption(
                UnsupportedOption::ResidentKey
            )),
            PasskeyError::UnsupportedOption(_)
        ));
        for error in [
            AuthenticatorError::Platform,
            AuthenticatorError::CryptoError,
            AuthenticatorError::U2FToken(U2FTokenError::Unknown),
            AuthenticatorError::HIDError(HIDError::DeviceNotSupported),
            AuthenticatorError::HIDError(HIDError::UnexpectedCmd(3)),
        ] {
            assert!(
                matches!(failure_of(error), PasskeyError::DeviceFailure(_)),
                "other failures are device failures"
            );
        }
    }

    #[test]
    fn the_text_of_a_library_error_is_never_kept() {
        let errors = [
            AuthenticatorError::Custom("user alice at /dev/secret-path".to_owned()),
            AuthenticatorError::InternalError("alice's key handle".to_owned()),
            AuthenticatorError::HIDError(HIDError::IO(
                Some("/dev/secret-path".into()),
                std::io::Error::other("alice"),
            )),
            AuthenticatorError::Io(std::io::Error::other("alice /dev/secret-path")),
        ];
        for error in errors {
            let mapped = failure_of(error);
            let shown = format!("{mapped} {mapped:?}");
            assert!(
                !shown.contains("alice") && !shown.contains("secret-path"),
                "{shown}"
            );
        }
    }

    #[test]
    fn no_configured_transport_means_no_device() {
        let service = AuthenticatorService::new().expect("service");
        let device = SecurityKeyAuthenticator::new(
            AuthenticatorRsDevice::with_service(service),
            RecordingUi::with_pins(Vec::new()),
        );
        assert_eq!(
            device.make_credential(&create_request()).unwrap_err(),
            PasskeyError::NoDevice
        );
        assert_eq!(
            device.get_assertion(&assert_request()).unwrap_err(),
            PasskeyError::NoDevice
        );
    }

    #[test]
    fn a_transport_that_cannot_start_is_reported() {
        let rig = rig();
        rig.control
            .reject_next_start(AuthenticatorError::NoConfiguredTransports);
        assert_eq!(
            rig.device.make_credential(&create_request()).unwrap_err(),
            PasskeyError::NoDevice
        );
    }

    #[test]
    fn touch_device_and_uv_updates_reach_the_interface() {
        let rig = rig();
        let (mut software, made) = software_create("example.com");
        let (authenticator_data, signature, _) =
            software.assert("example.com", &made.credential_id, [0x22; 32]);
        rig.control.push_sign(
            TransportScript::succeed(
                canned_sign_result(&authenticator_data, &signature, &made.credential_id, None)
                    .expect("canned result"),
            )
            .with_updates(&[
                FakeStatus::SelectDevice,
                FakeStatus::InvalidUv(Some(2)),
                FakeStatus::UvBlocked,
                FakeStatus::PinAuthBlocked,
            ])
            .with_updates_after_pin(&[FakeStatus::Touch]),
        );
        let mut request = assert_request();
        request.allow = vec![made.credential_id.clone()];

        rig.device.get_assertion(&request).expect("assert");

        assert_eq!(
            rig.ui.events(),
            vec![
                Event::SelectDevice,
                Event::UvFailed(Some(2)),
                Event::Blocked(SecurityKeyBlock::UvBlocked),
                Event::Blocked(SecurityKeyBlock::PinAuthBlocked),
                Event::Touch,
            ]
        );
    }

    #[test]
    fn a_blocked_key_tells_the_interface_before_the_request_fails() {
        let rig = rig();
        rig.control.push_register(
            TransportScript::fail(AuthenticatorError::PinError(PinError::PinNotSet))
                .with_updates(&[FakeStatus::PinNotSet]),
        );
        assert_eq!(
            rig.device.make_credential(&create_request()).unwrap_err(),
            PasskeyError::Device(0x35)
        );
        assert_eq!(
            rig.ui.events(),
            vec![Event::Blocked(SecurityKeyBlock::PinNotSet)]
        );
    }

    #[test]
    fn the_right_pin_through_the_real_conversion_code() {
        let rig = rig_with(RecordingUi::with_pins(vec![PinAnswer::Give(PIN)]));
        let (_software, made) = software_create("example.com");
        rig.control.push_register(
            TransportScript::succeed(
                canned_register_result(&made.attestation_object).expect("canned result"),
            )
            .with_pin(FakePin::new(PIN, 8))
            .with_updates_after_pin(&[FakeStatus::Touch]),
        );

        rig.device
            .make_credential(&create_request())
            .expect("create");
        assert_eq!(rig.ui.events(), vec![Event::Pin(None), Event::Touch]);
    }

    #[test]
    fn a_wrong_pin_is_retried_with_the_retries_the_key_reported() {
        let rig = rig_with(RecordingUi::with_pins(vec![
            PinAnswer::Give("000000"),
            PinAnswer::Give(PIN),
        ]));
        let (_software, made) = software_create("example.com");
        rig.control.push_register(
            TransportScript::succeed(
                canned_register_result(&made.attestation_object).expect("canned result"),
            )
            .with_pin(FakePin::new(PIN, 8)),
        );

        rig.device
            .make_credential(&create_request())
            .expect("create");
        assert_eq!(
            rig.ui.events(),
            vec![
                Event::Pin(None),
                Event::PinRejected(Some(7)),
                Event::Pin(Some(7))
            ]
        );
    }

    #[test]
    fn a_cancelled_pin_prompt_cancels_the_request_through_the_library() {
        let rig = rig_with(RecordingUi::with_pins(vec![PinAnswer::Cancel]));
        let (_software, made) = software_create("example.com");
        rig.control.push_register(
            TransportScript::succeed(
                canned_register_result(&made.attestation_object).expect("canned result"),
            )
            .with_pin(FakePin::new(PIN, 8)),
        );

        assert_eq!(
            rig.device.make_credential(&create_request()).unwrap_err(),
            PasskeyError::Cancelled
        );
    }

    #[test]
    fn a_pin_that_runs_out_of_attempts_blocks_the_key() {
        let rig = rig_with(RecordingUi::with_pins(vec![
            PinAnswer::Give("000000"),
            PinAnswer::Give("111111"),
            PinAnswer::Give(PIN),
        ]));
        let (_software, made) = software_create("example.com");
        rig.control.push_register(
            TransportScript::succeed(
                canned_register_result(&made.attestation_object).expect("canned result"),
            )
            .with_pin(FakePin::new(PIN, 2)),
        );

        assert_eq!(
            rig.device.make_credential(&create_request()).unwrap_err(),
            PasskeyError::Device(0x32)
        );
        assert_eq!(
            rig.ui.events(),
            vec![
                Event::Pin(None),
                Event::PinRejected(Some(1)),
                Event::Pin(Some(1)),
                Event::Blocked(SecurityKeyBlock::PinBlocked),
            ]
        );
    }

    #[test]
    fn the_chosen_account_is_passed_back_to_the_key() {
        let rig = rig_with(RecordingUi::with_choices(vec![Some(1)]));
        let (mut software, made) = software_create("example.com");
        let (authenticator_data, signature, _) =
            software.assert("example.com", &made.credential_id, [0x22; 32]);
        rig.control.push_sign(
            TransportScript::succeed(
                canned_sign_result(&authenticator_data, &signature, &made.credential_id, None)
                    .expect("canned result"),
            )
            .with_accounts(vec![
                UserEntity {
                    id: vec![1],
                    name: "alice".to_owned(),
                    display_name: "Alice".to_owned(),
                },
                UserEntity {
                    id: vec![2],
                    name: "bob".to_owned(),
                    display_name: "Bob".to_owned(),
                },
            ]),
        );
        let mut request = assert_request();
        request.allow = Vec::new();

        rig.device.get_assertion(&request).expect("assert");
        assert_eq!(rig.ui.events(), vec![Event::Choose(2)]);
        assert_eq!(rig.control.chosen_accounts(), vec![Some(1)]);
    }

    #[test]
    fn declining_to_choose_an_account_cancels_and_a_bad_index_is_a_decline() {
        for choice in [None, Some(5)] {
            let rig = rig_with(RecordingUi::with_choices(vec![choice]));
            let (mut software, made) = software_create("example.com");
            let (authenticator_data, signature, _) =
                software.assert("example.com", &made.credential_id, [0x22; 32]);
            rig.control.push_sign(
                TransportScript::succeed(
                    canned_sign_result(&authenticator_data, &signature, &made.credential_id, None)
                        .expect("canned result"),
                )
                .with_accounts(vec![
                    UserEntity {
                        id: vec![1],
                        name: "alice".to_owned(),
                        display_name: String::new(),
                    },
                    UserEntity {
                        id: vec![2],
                        name: "bob".to_owned(),
                        display_name: String::new(),
                    },
                ]),
            );
            let mut request = assert_request();
            request.allow = Vec::new();

            assert_eq!(
                rig.device.get_assertion(&request).unwrap_err(),
                PasskeyError::Cancelled,
                "{choice:?}"
            );
            assert_eq!(rig.control.chosen_accounts(), vec![None], "{choice:?}");
        }
    }

    #[test]
    fn a_key_nobody_touches_times_out_through_the_library() {
        let rig = rig();
        rig.control
            .push_register(TransportScript::wait_for_cancel());
        let mut request = create_request();
        request.timeout_ms = Some(80);

        let started = Instant::now();
        assert_eq!(
            rig.device.make_credential(&request).unwrap_err(),
            PasskeyError::NotAllowed
        );
        assert!(started.elapsed() >= Duration::from_millis(80));
        assert_eq!(rig.control.timeouts(), vec![80]);
        assert_eq!(rig.control.cancel_count(), 0, "a timeout is not a cancel");
    }

    #[test]
    fn cancel_from_another_thread_ends_a_waiting_request_as_cancelled() {
        let rig = rig();
        rig.control
            .push_register(TransportScript::wait_for_cancel());
        let device = Arc::new(rig.device);

        let worker = {
            let device = Arc::clone(&device);
            thread::spawn(move || device.make_credential(&create_request()))
        };
        wait_until("the request to reach the transport", || {
            !rig.control.register_args().is_empty()
        });
        device.cancel();

        // The library reports a cancelled transaction as "not allowed", as it does a timeout; the
        // device must still tell the caller it was cancelled.
        assert_eq!(worker.join().expect("worker"), Err(PasskeyError::Cancelled));
        assert_eq!(rig.control.cancel_count(), 1);
        assert_eq!(rig.ui.events(), vec![Event::Cancelled]);
    }

    #[test]
    fn cancel_while_the_pin_prompt_is_open_ends_the_request() {
        let rig = rig_with(RecordingUi::with_pins(vec![PinAnswer::UntilCancelled]));
        let (_software, made) = software_create("example.com");
        rig.control.push_register(
            TransportScript::succeed(
                canned_register_result(&made.attestation_object).expect("canned result"),
            )
            .with_pin(FakePin::new(PIN, 8)),
        );
        let device = Arc::new(rig.device);

        let worker = {
            let device = Arc::clone(&device);
            thread::spawn(move || device.make_credential(&create_request()))
        };
        wait_until("the PIN prompt", || {
            rig.ui.events().contains(&Event::Pin(None))
        });
        device.cancel();

        assert_eq!(worker.join().expect("worker"), Err(PasskeyError::Cancelled));
    }

    #[test]
    fn a_second_request_while_one_waits_is_refused_and_the_first_is_not_disturbed() {
        let rig = rig();
        rig.control
            .push_register(TransportScript::wait_for_cancel());
        let device = Arc::new(rig.device);

        let worker = {
            let device = Arc::clone(&device);
            thread::spawn(move || device.make_credential(&create_request()))
        };
        wait_until("the first request to reach the transport", || {
            !rig.control.register_args().is_empty()
        });

        let second = device.get_assertion(&assert_request());
        assert!(
            matches!(second, Err(PasskeyError::DeviceFailure(_))),
            "{second:?}"
        );
        assert!(
            rig.control.sign_args().is_empty(),
            "the busy call reached the transport"
        );

        device.cancel();
        assert_eq!(worker.join().expect("worker"), Err(PasskeyError::Cancelled));
    }

    #[test]
    fn a_cancel_before_any_request_does_not_spoil_the_next_one() {
        let rig = rig();
        rig.device.cancel();
        register_ok(&rig);
        rig.device
            .make_credential(&create_request())
            .expect("create");
    }

    #[test]
    fn nothing_secret_shows_in_the_device_debug_or_in_errors() {
        let rig = rig_with(RecordingUi::with_pins(vec![
            PinAnswer::Give("246810"),
            PinAnswer::Cancel,
        ]));
        let (_software, made) = software_create("example.com");
        rig.control.push_register(
            TransportScript::succeed(
                canned_register_result(&made.attestation_object).expect("canned result"),
            )
            .with_pin(FakePin::new(PIN, 8)),
        );
        let error = rig.device.make_credential(&create_request()).unwrap_err();
        let shown = format!("{:?} {:?} {error} {error:?}", rig.device, rig.control);
        for secret in [PIN, "246810", "alice"] {
            assert!(!shown.contains(secret), "a secret shows in debug output");
        }
        // The recorded arguments carry no PIN either.
        for args in rig.control.register_args() {
            assert!(args.pin.is_none());
        }
    }
}
