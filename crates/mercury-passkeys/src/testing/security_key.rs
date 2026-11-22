// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Test doubles for security keys.
//!
//! [`FakeSecurityKey`] stands in for a whole [`SecurityKeyDevice`]: it answers with scripted
//! responses or errors, walks the [`SecurityKeyUi`] through a PIN flow like a real key, and records
//! the requests it saw. On macOS [`FakeTransport`] sits one level lower, below
//! `authenticator::AuthenticatorService`, so the real conversion code in
//! [`crate::usb_hid::AuthenticatorRsDevice`] runs against it. No USB device is ever opened.
//!
//! The responses these doubles hand back are shaped like real ones but are not signed by any
//! key: [`fake_create_response`] and [`fake_assert_response`] carry a placeholder signature and
//! the public key of the P-256 generator point.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use passkey::crypto::coset::cbor::{Value, ser::into_writer};
use passkey::crypto::{hash::Sha256Backend, rust_crypto::RustCryptoSha2};
use zeroize::Zeroizing;

use crate::usb::{SecurityKeyBlock, SecurityKeyDevice, SecurityKeyUi, effective_timeout_ms};
use crate::{
    AssertRequest, AssertResponse, Attachment, AuthenticatorKind, COSE_ES256, CreateRequest,
    CreateResponse, PasskeyError, Transport, UserEntity,
};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A PIN a fake key demands, and how many wrong attempts it allows. `Debug` shows neither.
#[derive(Clone)]
pub struct FakePin {
    pin: Zeroizing<String>,
    retries: u8,
}

impl FakePin {
    /// A key whose PIN is `pin` and that blocks itself after `retries` wrong attempts.
    pub fn new(pin: &str, retries: u8) -> Self {
        Self {
            pin: Zeroizing::new(pin.to_owned()),
            retries,
        }
    }

    /// The PIN, for the fake's own comparison.
    #[cfg(target_os = "macos")]
    fn matches(&self, given: &[u8]) -> bool {
        self.pin.as_bytes() == given
    }

    fn attempts(&self) -> u8 {
        self.retries
    }
}

impl std::fmt::Debug for FakePin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FakePin(<redacted>)")
    }
}

/// An interface call a fake key makes before it answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeKeyNotice {
    /// [`SecurityKeyUi::select_device_notice`].
    SelectDevice,
    /// [`SecurityKeyUi::touch_notice`].
    Touch,
    /// [`SecurityKeyUi::uv_failed`] with this many retries.
    UvFailed(Option<u8>),
    /// [`SecurityKeyUi::blocked`] with this state.
    Blocked(SecurityKeyBlock),
}

/// How a scripted call of a [`FakeSecurityKey`] ends.
#[derive(Debug)]
pub enum FakeKeyOutcome<T> {
    /// Answer at once with this.
    Respond(Result<T, PasskeyError>),
    /// Wait like a key nobody touches: until [`SecurityKeyDevice::cancel`] (then
    /// [`PasskeyError::Cancelled`]) or the request's timeout (then [`PasskeyError::NotAllowed`]).
    WaitForCancel,
}

/// One scripted call of a [`FakeSecurityKey`].
pub struct FakeKeyScript<T> {
    notices: Vec<FakeKeyNotice>,
    pin: Option<FakePin>,
    outcome: FakeKeyOutcome<T>,
}

impl<T> FakeKeyScript<T> {
    /// A call that answers with `response`.
    pub fn succeed(response: T) -> Self {
        Self::new(FakeKeyOutcome::Respond(Ok(response)))
    }

    /// A call that fails with `error`.
    pub fn fail(error: PasskeyError) -> Self {
        Self::new(FakeKeyOutcome::Respond(Err(error)))
    }

    /// A call that waits for a cancel or the timeout.
    pub fn wait_for_cancel() -> Self {
        Self::new(FakeKeyOutcome::WaitForCancel)
    }

    fn new(outcome: FakeKeyOutcome<T>) -> Self {
        Self {
            notices: Vec::new(),
            pin: None,
            outcome,
        }
    }

    /// Makes the key call the interface with these notices, in order, before anything else.
    pub fn with_notices(mut self, notices: &[FakeKeyNotice]) -> Self {
        self.notices = notices.to_vec();
        self
    }

    /// Makes the key ask for a PIN, after the notices, and answer only once the right one is given.
    pub fn with_pin(mut self, pin: FakePin) -> Self {
        self.pin = Some(pin);
        self
    }
}

#[derive(Default)]
struct KeyScripts {
    creates: VecDeque<FakeKeyScript<CreateResponse>>,
    asserts: VecDeque<FakeKeyScript<AssertResponse>>,
    create_requests: Vec<CreateRequest>,
    assert_requests: Vec<AssertRequest>,
    cancels: usize,
}

#[derive(Default)]
struct KeyWait {
    active: bool,
    cancelled: bool,
}

#[derive(Default)]
struct KeyShared {
    scripts: Mutex<KeyScripts>,
    wait: Mutex<KeyWait>,
    wake: Condvar,
}

/// A scripted [`SecurityKeyDevice`]. Clones share one script and one record, so a test keeps a
/// clone, hands the other to a [`crate::usb::SecurityKeyAuthenticator`] and reads back what the
/// key was asked.
#[derive(Clone, Default)]
pub struct FakeSecurityKey {
    shared: Arc<KeyShared>,
}

impl std::fmt::Debug for FakeSecurityKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FakeSecurityKey")
            .finish_non_exhaustive()
    }
}

/// Marks a call as waiting, from before its request is recorded until it returns, so a cancel
/// that follows seeing the request is never lost.
struct ActiveGuard<'a>(&'a KeyShared);

impl<'a> ActiveGuard<'a> {
    fn begin(shared: &'a KeyShared) -> Self {
        let mut wait = lock(&shared.wait);
        wait.active = true;
        wait.cancelled = false;
        Self(shared)
    }
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        lock(&self.0.wait).active = false;
    }
}

impl FakeSecurityKey {
    /// A key with nothing scripted: every call fails with [`PasskeyError::NoDevice`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Scripts the next `make_credential` call.
    pub fn push_create(&self, script: FakeKeyScript<CreateResponse>) {
        lock(&self.shared.scripts).creates.push_back(script);
    }

    /// Scripts the next `get_assertion` call.
    pub fn push_assert(&self, script: FakeKeyScript<AssertResponse>) {
        lock(&self.shared.scripts).asserts.push_back(script);
    }

    /// The `make_credential` requests the key received, oldest first.
    pub fn create_requests(&self) -> Vec<CreateRequest> {
        lock(&self.shared.scripts).create_requests.clone()
    }

    /// The `get_assertion` requests the key received, oldest first.
    pub fn assert_requests(&self) -> Vec<AssertRequest> {
        lock(&self.shared.scripts).assert_requests.clone()
    }

    /// How many times [`SecurityKeyDevice::cancel`] was called.
    pub fn cancel_count(&self) -> usize {
        lock(&self.shared.scripts).cancels
    }

    fn run<T>(
        &self,
        script: FakeKeyScript<T>,
        timeout_ms: Option<u32>,
        ui: &dyn SecurityKeyUi,
        _active: ActiveGuard<'_>,
    ) -> Result<T, PasskeyError> {
        for notice in &script.notices {
            match *notice {
                FakeKeyNotice::SelectDevice => ui.select_device_notice(),
                FakeKeyNotice::Touch => ui.touch_notice(),
                FakeKeyNotice::UvFailed(retries) => ui.uv_failed(retries),
                FakeKeyNotice::Blocked(state) => ui.blocked(state),
            }
        }
        if let Some(pin) = &script.pin {
            let mut remaining = pin.attempts();
            let mut asked = None;
            loop {
                let Some(given) = ui.pin(asked) else {
                    return Err(PasskeyError::Cancelled);
                };
                if given.as_str() == pin.pin.as_str() {
                    break;
                }
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    ui.blocked(SecurityKeyBlock::PinBlocked);
                    return Err(PasskeyError::Device(0x32));
                }
                ui.pin_rejected(Some(remaining));
                asked = Some(remaining);
            }
        }
        match script.outcome {
            FakeKeyOutcome::Respond(result) => result,
            FakeKeyOutcome::WaitForCancel => {
                let timeout = Duration::from_millis(u64::from(effective_timeout_ms(timeout_ms)));
                let wait = lock(&self.shared.wait);
                let (wait, _) = self
                    .shared
                    .wake
                    .wait_timeout_while(wait, timeout, |state| !state.cancelled)
                    .unwrap_or_else(PoisonError::into_inner);
                if wait.cancelled {
                    Err(PasskeyError::Cancelled)
                } else {
                    Err(PasskeyError::NotAllowed)
                }
            }
        }
    }
}

impl SecurityKeyDevice for FakeSecurityKey {
    fn make_credential(
        &self,
        request: &CreateRequest,
        ui: &dyn SecurityKeyUi,
    ) -> Result<CreateResponse, PasskeyError> {
        let active = ActiveGuard::begin(&self.shared);
        let script = {
            let mut scripts = lock(&self.shared.scripts);
            scripts.create_requests.push(request.clone());
            scripts.creates.pop_front()
        };
        match script {
            Some(script) => self.run(script, request.timeout_ms, ui, active),
            None => Err(PasskeyError::NoDevice),
        }
    }

    fn get_assertion(
        &self,
        request: &AssertRequest,
        ui: &dyn SecurityKeyUi,
    ) -> Result<AssertResponse, PasskeyError> {
        let active = ActiveGuard::begin(&self.shared);
        let script = {
            let mut scripts = lock(&self.shared.scripts);
            scripts.assert_requests.push(request.clone());
            scripts.asserts.pop_front()
        };
        match script {
            Some(script) => self.run(script, request.timeout_ms, ui, active),
            None => Err(PasskeyError::NoDevice),
        }
    }

    fn cancel(&self) {
        lock(&self.shared.scripts).cancels += 1;
        let mut wait = lock(&self.shared.wait);
        if wait.active {
            wait.cancelled = true;
            self.shared.wake.notify_all();
        }
    }
}

/// x and y of the P-256 generator point: a valid public key whose private key (1) is public, used
/// in the fake credentials so no real key is ever implied.
const GENERATOR_X: [u8; 32] = [
    0x6b, 0x17, 0xd1, 0xf2, 0xe1, 0x2c, 0x42, 0x47, 0xf8, 0xbc, 0xe6, 0xe5, 0x63, 0xa4, 0x40, 0xf2,
    0x77, 0x03, 0x7d, 0x81, 0x2d, 0xeb, 0x33, 0xa0, 0xf4, 0xa1, 0x39, 0x45, 0xd8, 0x98, 0xc2, 0x96,
];
const GENERATOR_Y: [u8; 32] = [
    0x4f, 0xe3, 0x42, 0xe2, 0xfe, 0x1a, 0x7f, 0x9b, 0x8e, 0xe7, 0xeb, 0x4a, 0x7c, 0x0f, 0x9e, 0x16,
    0x2b, 0xce, 0x33, 0x57, 0x6b, 0x31, 0x5e, 0xce, 0xcb, 0xb6, 0x40, 0x68, 0x37, 0xbf, 0x51, 0xf5,
];

fn cbor_bytes(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    // Writing a CBOR value into a Vec cannot fail.
    let _ = into_writer(value, &mut out);
    out
}

fn fake_cose_public_key() -> Vec<u8> {
    let integer = |n: i64| Value::Integer(n.into());
    cbor_bytes(&Value::Map(vec![
        (integer(1), integer(2)),
        (integer(3), integer(COSE_ES256)),
        (integer(-1), integer(1)),
        (integer(-2), Value::Bytes(GENERATOR_X.to_vec())),
        (integer(-3), Value::Bytes(GENERATOR_Y.to_vec())),
    ]))
}

/// A well-formed [`CreateResponse`] for `rp_id` and `credential_id`: attestation format `none`,
/// an all-zero AAGUID, flags user present and attested, and a placeholder public key. Its
/// authenticator data starts with SHA-256 of `rp_id`, as a real key's does. Nothing in it is
/// signed.
pub fn fake_create_response(rp_id: &str, credential_id: &[u8]) -> CreateResponse {
    let mut authenticator_data = RustCryptoSha2::sha256(rp_id.as_bytes()).to_vec();
    authenticator_data.push(0x41);
    authenticator_data.extend_from_slice(&0_u32.to_be_bytes());
    authenticator_data.extend_from_slice(&[0; 16]);
    authenticator_data.extend_from_slice(
        &u16::try_from(credential_id.len())
            .unwrap_or(u16::MAX)
            .to_be_bytes(),
    );
    authenticator_data.extend_from_slice(credential_id);
    authenticator_data.extend_from_slice(&fake_cose_public_key());
    let text = |value: &str| Value::Text(value.to_owned());
    let attestation_object = cbor_bytes(&Value::Map(vec![
        (text("fmt"), text("none")),
        (text("attStmt"), Value::Map(Vec::new())),
        (text("authData"), Value::Bytes(authenticator_data.clone())),
    ]));
    CreateResponse {
        kind: AuthenticatorKind::SecurityKey,
        credential_id: credential_id.to_vec(),
        attestation_object,
        authenticator_data,
        public_key_algorithm: COSE_ES256,
        public_key_spki: None,
        transports: vec![Transport::Usb],
        attachment: Attachment::CrossPlatform,
    }
}

/// A well-formed [`AssertResponse`] for `rp_id` and `credential_id`: flags user present, counter 1,
/// and a placeholder signature. Its authenticator data starts with SHA-256 of `rp_id`.
pub fn fake_assert_response(
    rp_id: &str,
    credential_id: &[u8],
    user_handle: Option<&[u8]>,
) -> AssertResponse {
    let mut authenticator_data = RustCryptoSha2::sha256(rp_id.as_bytes()).to_vec();
    authenticator_data.push(0x01);
    authenticator_data.extend_from_slice(&1_u32.to_be_bytes());
    AssertResponse {
        kind: AuthenticatorKind::SecurityKey,
        credential_id: credential_id.to_vec(),
        authenticator_data,
        signature: vec![0x30; 64],
        user_handle: user_handle.map(<[u8]>::to_vec),
        attachment: Attachment::CrossPlatform,
    }
}

/// Convenience for tests: a [`UserEntity`] with these names.
pub fn fake_user(id: &[u8], name: &str, display_name: &str) -> UserEntity {
    UserEntity {
        id: id.to_vec(),
        name: name.to_owned(),
        display_name: display_name.to_owned(),
    }
}

#[cfg(target_os = "macos")]
pub use transport::*;

#[cfg(target_os = "macos")]
mod transport {
    use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
    use std::thread::JoinHandle;

    use authenticator::authenticatorservice::{AuthenticatorTransport, RegisterArgs, SignArgs};
    use authenticator::ctap2::attestation::AuthenticatorData;
    use authenticator::ctap2::server::{
        AuthenticatorAttachment, PublicKeyCredentialDescriptor, PublicKeyCredentialUserEntity,
    };
    use authenticator::errors::{AuthenticatorError, PinError, U2FTokenError};
    use authenticator::statecallback::StateCallback;
    use authenticator::{
        Assertion, AttestationObject, GetAssertionResult, MakeCredentialsResult, ManageResult, Pin,
        RegisterResult, ResetResult, SignResult, StatusPinUv, StatusUpdate,
    };
    use passkey::crypto::coset::cbor::{Value, de::from_reader};

    use super::*;

    /// Builds the [`RegisterResult`] a key would return, from the CBOR attestation object a
    /// software authenticator made. Fails with [`PasskeyError::InvalidInput`] when
    /// `authenticator-rs` cannot read it.
    pub fn canned_register_result(
        attestation_object: &[u8],
    ) -> Result<RegisterResult, PasskeyError> {
        let att_obj: AttestationObject = from_reader(attestation_object)
            .map_err(|_| PasskeyError::InvalidInput("not an attestation object"))?;
        Ok(MakeCredentialsResult {
            att_obj,
            attachment: AuthenticatorAttachment::CrossPlatform,
            extensions: Default::default(),
        })
    }

    /// Builds the [`SignResult`] a key would return: `authenticator_data` and `signature` as a
    /// software authenticator made them, for `credential_id`, with the account `user` if the key
    /// reports one. Fails with [`PasskeyError::InvalidInput`] when `authenticator-rs` cannot read
    /// the authenticator data.
    pub fn canned_sign_result(
        authenticator_data: &[u8],
        signature: &[u8],
        credential_id: &[u8],
        user: Option<&UserEntity>,
    ) -> Result<SignResult, PasskeyError> {
        let encoded = cbor_bytes(&Value::Bytes(authenticator_data.to_vec()));
        let auth_data: AuthenticatorData = from_reader(encoded.as_slice())
            .map_err(|_| PasskeyError::InvalidInput("not authenticator data"))?;
        Ok(GetAssertionResult {
            assertion: Assertion {
                credentials: Some(PublicKeyCredentialDescriptor {
                    id: credential_id.to_vec(),
                    transports: Vec::new(),
                }),
                auth_data,
                signature: signature.to_vec(),
                user: user.map(rs_user),
            },
            attachment: AuthenticatorAttachment::CrossPlatform,
            extensions: Default::default(),
        })
    }

    fn rs_user(user: &UserEntity) -> PublicKeyCredentialUserEntity {
        PublicKeyCredentialUserEntity {
            id: user.id.clone(),
            name: Some(user.name.clone()),
            display_name: Some(user.display_name.clone()),
        }
    }

    /// A status update a fake transport sends that needs no answer.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum FakeStatus {
        /// `StatusUpdate::SelectDeviceNotice`.
        SelectDevice,
        /// `StatusUpdate::PresenceRequired`.
        Touch,
        /// `StatusPinUv::InvalidUv` with this many retries.
        InvalidUv(Option<u8>),
        /// `StatusPinUv::PinBlocked`.
        PinBlocked,
        /// `StatusPinUv::PinAuthBlocked`.
        PinAuthBlocked,
        /// `StatusPinUv::PinNotSet`.
        PinNotSet,
        /// `StatusPinUv::UvBlocked`.
        UvBlocked,
    }

    impl FakeStatus {
        fn update(self) -> StatusUpdate {
            match self {
                Self::SelectDevice => StatusUpdate::SelectDeviceNotice,
                Self::Touch => StatusUpdate::PresenceRequired,
                Self::InvalidUv(retries) => {
                    StatusUpdate::PinUvError(StatusPinUv::InvalidUv(retries))
                }
                Self::PinBlocked => StatusUpdate::PinUvError(StatusPinUv::PinBlocked),
                Self::PinAuthBlocked => StatusUpdate::PinUvError(StatusPinUv::PinAuthBlocked),
                Self::PinNotSet => StatusUpdate::PinUvError(StatusPinUv::PinNotSet),
                Self::UvBlocked => StatusUpdate::PinUvError(StatusPinUv::UvBlocked),
            }
        }
    }

    /// How a scripted transport call ends.
    pub enum FakeTransportOutcome<T> {
        /// Call back with this result.
        Success(T),
        /// Call back with this error.
        Fail(AuthenticatorError),
        /// Wait like a transaction nobody touches: until the transport is cancelled or the
        /// call's timeout passes, then call back with `U2FToken(NotAllowed)`, which is what the
        /// real transport reports for both.
        WaitForCancel,
    }

    /// One scripted `register` or `sign` call of a [`FakeTransport`].
    pub struct TransportScript<T> {
        before: Vec<FakeStatus>,
        accounts: Option<Vec<UserEntity>>,
        pin: Option<FakePin>,
        after: Vec<FakeStatus>,
        outcome: FakeTransportOutcome<T>,
    }

    impl<T> TransportScript<T> {
        /// A call that ends with `result`.
        pub fn succeed(result: T) -> Self {
            Self::new(FakeTransportOutcome::Success(result))
        }

        /// A call that ends with `error`.
        pub fn fail(error: AuthenticatorError) -> Self {
            Self::new(FakeTransportOutcome::Fail(error))
        }

        /// A call that waits for a cancel or the timeout.
        pub fn wait_for_cancel() -> Self {
            Self::new(FakeTransportOutcome::WaitForCancel)
        }

        fn new(outcome: FakeTransportOutcome<T>) -> Self {
            Self {
                before: Vec::new(),
                accounts: None,
                pin: None,
                after: Vec::new(),
                outcome,
            }
        }

        /// Sends these status updates first.
        pub fn with_updates(mut self, updates: &[FakeStatus]) -> Self {
            self.before = updates.to_vec();
            self
        }

        /// After the PIN, sends these status updates.
        pub fn with_updates_after_pin(mut self, updates: &[FakeStatus]) -> Self {
            self.after = updates.to_vec();
            self
        }

        /// Demands a PIN the way a CTAP2 key does: `PinRequired`, then `InvalidPin` with the
        /// attempts left after each wrong PIN, then `PinBlocked` and a `PinError` when none are
        /// left. A dropped PIN channel is a cancel (`CancelledByUser`).
        pub fn with_pin(mut self, pin: FakePin) -> Self {
            self.pin = Some(pin);
            self
        }

        /// Reports these accounts (`SelectResultNotice`) after the PIN and waits for the choice.
        pub fn with_accounts(mut self, accounts: Vec<UserEntity>) -> Self {
            self.accounts = Some(accounts);
            self
        }
    }

    #[derive(Default)]
    struct CancelSignal {
        cancelled: Mutex<bool>,
        wake: Condvar,
    }

    impl CancelSignal {
        fn is_cancelled(&self) -> bool {
            *lock(&self.cancelled)
        }

        fn cancel(&self) {
            *lock(&self.cancelled) = true;
            self.wake.notify_all();
        }

        /// Waits up to `timeout` for a cancel; true if one came.
        fn wait(&self, timeout: Duration) -> bool {
            let cancelled = lock(&self.cancelled);
            let (cancelled, _) = self
                .wake
                .wait_timeout_while(cancelled, timeout, |cancelled| !*cancelled)
                .unwrap_or_else(PoisonError::into_inner);
            *cancelled
        }
    }

    #[derive(Default)]
    struct TransportState {
        registers: VecDeque<TransportScript<RegisterResult>>,
        signs: VecDeque<TransportScript<SignResult>>,
        rejections: VecDeque<AuthenticatorError>,
        register_args: Vec<RegisterArgs>,
        sign_args: Vec<SignArgs>,
        timeouts: Vec<u64>,
        chosen: Vec<Option<usize>>,
        cancels: usize,
        signal: Option<Arc<CancelSignal>>,
        worker: Option<JoinHandle<()>>,
    }

    #[derive(Default)]
    struct TransportShared {
        state: Mutex<TransportState>,
    }

    /// The test's handle on a [`FakeTransport`]: scripts it and reads back what it was asked.
    /// Clones share the same transport.
    #[derive(Clone)]
    pub struct FakeTransportControl {
        shared: Arc<TransportShared>,
    }

    impl std::fmt::Debug for FakeTransportControl {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("FakeTransportControl")
                .finish_non_exhaustive()
        }
    }

    impl FakeTransportControl {
        /// Scripts the next `register` call.
        pub fn push_register(&self, script: TransportScript<RegisterResult>) {
            lock(&self.shared.state).registers.push_back(script);
        }

        /// Scripts the next `sign` call.
        pub fn push_sign(&self, script: TransportScript<SignResult>) {
            lock(&self.shared.state).signs.push_back(script);
        }

        /// Makes the next `register` or `sign` call fail at once, from the call itself, as a
        /// transport that cannot start does.
        pub fn reject_next_start(&self, error: AuthenticatorError) {
            lock(&self.shared.state).rejections.push_back(error);
        }

        /// The arguments of every `register` call, oldest first.
        pub fn register_args(&self) -> Vec<RegisterArgs> {
            lock(&self.shared.state).register_args.clone()
        }

        /// The arguments of every `sign` call, oldest first.
        pub fn sign_args(&self) -> Vec<SignArgs> {
            lock(&self.shared.state).sign_args.clone()
        }

        /// The timeout, in milliseconds, of every call, oldest first.
        pub fn timeouts(&self) -> Vec<u64> {
            lock(&self.shared.state).timeouts.clone()
        }

        /// The account index the user chose in every `SelectResultNotice`, oldest first
        /// (`None` for a cancel).
        pub fn chosen_accounts(&self) -> Vec<Option<usize>> {
            lock(&self.shared.state).chosen.clone()
        }

        /// How many times the service cancelled the transport.
        pub fn cancel_count(&self) -> usize {
            lock(&self.shared.state).cancels
        }
    }

    /// A stand-in for the USB transport, to be added to an `AuthenticatorService` with
    /// `add_transport`. Like the real one it returns from `register` and `sign` at once and
    /// reports through the status channel and the callback from its own thread, and `cancel`
    /// waits for that thread to finish.
    pub struct FakeTransport {
        shared: Arc<TransportShared>,
    }

    impl FakeTransport {
        /// A transport and the handle that scripts it.
        pub fn new() -> (Self, FakeTransportControl) {
            let shared = Arc::new(TransportShared::default());
            (
                Self {
                    shared: Arc::clone(&shared),
                },
                FakeTransportControl { shared },
            )
        }

        fn stop_worker(&self) -> bool {
            let (signal, worker) = {
                let mut state = lock(&self.shared.state);
                (state.signal.take(), state.worker.take())
            };
            if let Some(signal) = &signal {
                signal.cancel();
            }
            if let Some(worker) = worker {
                let _ = worker.join();
            }
            signal.is_some()
        }

        fn start<T: Send + 'static>(
            &self,
            script: TransportScript<T>,
            timeout: u64,
            status: Sender<StatusUpdate>,
            callback: StateCallback<authenticator::Result<T>>,
        ) {
            self.stop_worker();
            let signal = Arc::new(CancelSignal::default());
            let shared = Arc::clone(&self.shared);
            let worker_signal = Arc::clone(&signal);
            let worker = std::thread::spawn(move || {
                run_script(&shared, script, timeout, &status, &callback, &worker_signal);
            });
            let mut state = lock(&self.shared.state);
            state.signal = Some(signal);
            state.worker = Some(worker);
        }
    }

    fn send(status: &Sender<StatusUpdate>, update: StatusUpdate) {
        let _ = status.send(update);
    }

    fn run_script<T>(
        shared: &TransportShared,
        script: TransportScript<T>,
        timeout: u64,
        status: &Sender<StatusUpdate>,
        callback: &StateCallback<authenticator::Result<T>>,
        signal: &CancelSignal,
    ) {
        let timed_out = || {
            callback.call(Err(AuthenticatorError::U2FToken(U2FTokenError::NotAllowed)));
        };
        for update in &script.before {
            send(status, update.update());
        }
        if let Some(pin) = &script.pin {
            let mut remaining = pin.attempts();
            let mut invalid_with = None;
            loop {
                let (reply, answer) = channel::<Pin>();
                let update = match invalid_with {
                    None => StatusPinUv::PinRequired(reply),
                    Some(retries) => StatusPinUv::InvalidPin(reply, Some(retries)),
                };
                send(status, StatusUpdate::PinUvError(update));
                let given = loop {
                    match answer.recv_timeout(Duration::from_millis(10)) {
                        Ok(given) => break given,
                        Err(RecvTimeoutError::Timeout) if signal.is_cancelled() => {
                            timed_out();
                            return;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => {
                            callback.call(Err(AuthenticatorError::CancelledByUser));
                            return;
                        }
                    }
                };
                if pin.matches(given.as_bytes()) {
                    break;
                }
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    send(status, StatusUpdate::PinUvError(StatusPinUv::PinBlocked));
                    callback.call(Err(AuthenticatorError::PinError(PinError::PinBlocked)));
                    return;
                }
                invalid_with = Some(remaining);
            }
        }
        for update in &script.after {
            send(status, update.update());
        }
        if let Some(accounts) = &script.accounts {
            let (reply, answer) = channel::<Option<usize>>();
            send(
                status,
                StatusUpdate::SelectResultNotice(reply, accounts.iter().map(rs_user).collect()),
            );
            let choice = answer.recv().ok().flatten();
            lock(&shared.state).chosen.push(choice);
            if choice.is_none() {
                callback.call(Err(AuthenticatorError::CancelledByUser));
                return;
            }
        }
        match script.outcome {
            FakeTransportOutcome::Success(result) => callback.call(Ok(result)),
            FakeTransportOutcome::Fail(error) => callback.call(Err(error)),
            FakeTransportOutcome::WaitForCancel => {
                signal.wait(Duration::from_millis(timeout));
                timed_out();
            }
        }
    }

    impl AuthenticatorTransport for FakeTransport {
        fn register(
            &mut self,
            timeout: u64,
            ctap_args: RegisterArgs,
            status: Sender<StatusUpdate>,
            callback: StateCallback<authenticator::Result<RegisterResult>>,
        ) -> authenticator::Result<()> {
            let script = {
                let mut state = lock(&self.shared.state);
                state.register_args.push(ctap_args);
                state.timeouts.push(timeout);
                if let Some(error) = state.rejections.pop_front() {
                    return Err(error);
                }
                state.registers.pop_front()
            };
            match script {
                Some(script) => self.start(script, timeout, status, callback),
                None => callback.call(Err(AuthenticatorError::Platform)),
            }
            Ok(())
        }

        fn sign(
            &mut self,
            timeout: u64,
            ctap_args: SignArgs,
            status: Sender<StatusUpdate>,
            callback: StateCallback<authenticator::Result<SignResult>>,
        ) -> authenticator::Result<()> {
            let script = {
                let mut state = lock(&self.shared.state);
                state.sign_args.push(ctap_args);
                state.timeouts.push(timeout);
                if let Some(error) = state.rejections.pop_front() {
                    return Err(error);
                }
                state.signs.pop_front()
            };
            match script {
                Some(script) => self.start(script, timeout, status, callback),
                None => callback.call(Err(AuthenticatorError::Platform)),
            }
            Ok(())
        }

        fn cancel(&mut self) -> authenticator::Result<()> {
            lock(&self.shared.state).cancels += 1;
            self.stop_worker();
            Ok(())
        }

        fn reset(
            &mut self,
            _timeout: u64,
            _status: Sender<StatusUpdate>,
            _callback: StateCallback<authenticator::Result<ResetResult>>,
        ) -> authenticator::Result<()> {
            Err(AuthenticatorError::Platform)
        }

        fn set_pin(
            &mut self,
            _timeout: u64,
            _new_pin: Pin,
            _status: Sender<StatusUpdate>,
            _callback: StateCallback<authenticator::Result<ResetResult>>,
        ) -> authenticator::Result<()> {
            Err(AuthenticatorError::Platform)
        }

        fn manage(
            &mut self,
            _timeout: u64,
            _status: Sender<StatusUpdate>,
            _callback: StateCallback<authenticator::Result<ManageResult>>,
        ) -> authenticator::Result<()> {
            Err(AuthenticatorError::Platform)
        }
    }
}
