// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! USB security keys over CTAP2 and HID, through Mozilla's `authenticator` crate (macOS only).
//!
//! [`AuthenticatorRsDevice`] turns a [`CreateRequest`] or [`AssertRequest`] into the arguments of
//! `authenticator::AuthenticatorService`, runs the call, forwards what the key reports (touch, PIN,
//! wrong PIN, blocked states, which device) to a [`SecurityKeyUi`], and turns the key's answer or
//! its error back into this crate's types.
//!
//! Attestation is always dropped: the attestation object handed back has format `none` and an
//! all-zero AAGUID, which is what a WebAuthn client does for `attestation: "none"`, the default.
//! Mercury does not pass a key's batch attestation certificate to a site.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use authenticator::authenticatorservice::{AuthenticatorService, RegisterArgs, SignArgs};
use authenticator::crypto::COSEAlgorithm;
use authenticator::ctap2::commands::StatusCode;
use authenticator::ctap2::server::{
    AuthenticationExtensionsClientInputs, PublicKeyCredentialDescriptor,
    PublicKeyCredentialParameters, PublicKeyCredentialUserEntity, RelyingParty,
    ResidentKeyRequirement, UserVerificationRequirement,
};
use authenticator::errors::{
    AuthenticatorError, CommandError, HIDError, PinError, U2FTokenError, UnsupportedOption,
};
use authenticator::statecallback::StateCallback;
use authenticator::{Pin, RegisterResult, SignResult, StatusPinUv, StatusUpdate};
use passkey::crypto::coset::cbor::{Value, ser::into_writer};

use crate::usb::{SecurityKeyBlock, SecurityKeyDevice, SecurityKeyUi, effective_timeout_ms};
use crate::{
    AssertRequest, AssertResponse, Attachment, AuthenticatorKind, CreateRequest, CreateResponse,
    PasskeyError, ResidentKey, Transport, UserEntity, UserVerification,
};

/// How often a waiting call looks at its channels and at the clock.
const POLL: Duration = Duration::from_millis(20);
/// How long past its own timeout a call waits for the transport to report before it gives up on it.
const WATCHDOG_GRACE: Duration = Duration::from_secs(10);
/// How long after a cancel a call waits for the transport to report before it returns anyway.
const CANCEL_GRACE: Duration = Duration::from_secs(3);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A [`SecurityKeyDevice`] that talks to USB security keys through `authenticator-rs`.
///
/// One call at a time: while a request is waiting for the user, another fails at once with
/// [`PasskeyError::DeviceFailure`]. [`SecurityKeyDevice::cancel`] ends the call in progress. If the
/// call is waiting for a PIN, the interface must first make its pending [`SecurityKeyUi::pin`]
/// return `None` ([`SecurityKeyUi::cancelled`] is for that), because the library cannot be stopped
/// while it waits for the answer.
pub struct AuthenticatorRsDevice {
    service: Mutex<AuthenticatorService>,
    busy: Mutex<()>,
    active: Mutex<Option<Arc<AtomicBool>>>,
}

impl std::fmt::Debug for AuthenticatorRsDevice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthenticatorRsDevice")
            .finish_non_exhaustive()
    }
}

struct ActiveReset<'a>(&'a Mutex<Option<Arc<AtomicBool>>>);

impl Drop for ActiveReset<'_> {
    fn drop(&mut self) {
        *lock(self.0) = None;
    }
}

impl AuthenticatorRsDevice {
    /// The real thing: every USB transport `authenticator-rs` finds on this platform. Opens no
    /// device; the keys are looked for when a request starts.
    pub fn new() -> Result<Self, PasskeyError> {
        let mut service = AuthenticatorService::new()
            .map_err(|_| PasskeyError::DeviceFailure("the security key service could not start"))?;
        service.add_detected_transports();
        Ok(Self::with_service(service))
    }

    /// A device over a service the caller built, for tests that add their own transport.
    pub fn with_service(service: AuthenticatorService) -> Self {
        Self {
            service: Mutex::new(service),
            busy: Mutex::new(()),
            active: Mutex::new(None),
        }
    }

    fn drive<T: Send + 'static>(
        &self,
        timeout_ms: u32,
        ui: &dyn SecurityKeyUi,
        start: impl FnOnce(
            &mut AuthenticatorService,
            u64,
            Sender<StatusUpdate>,
            StateCallback<authenticator::Result<T>>,
        ) -> authenticator::Result<()>,
    ) -> Result<T, PasskeyError> {
        let _busy = match self.busy.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => {
                return Err(PasskeyError::DeviceFailure(
                    "the security key is busy with another request",
                ));
            }
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        *lock(&self.active) = Some(Arc::clone(&cancelled));
        let _active = ActiveReset(&self.active);

        let (status_sender, status_receiver) = channel();
        let (result_sender, result_receiver) = channel();
        let callback = StateCallback::new(Box::new(move |result| {
            let _ = result_sender.send(result);
        }));
        let timeout = u64::from(timeout_ms);
        let started = start(&mut lock(&self.service), timeout, status_sender, callback);
        if let Err(error) = started {
            let _ = lock(&self.service).cancel();
            return Err(map_error(error));
        }

        let deadline = Instant::now() + Duration::from_millis(timeout) + WATCHDOG_GRACE;
        let mut cancel_seen = None;
        let mut status_open = true;
        loop {
            let finished = if status_open {
                match status_receiver.recv_timeout(POLL) {
                    Ok(update) => handle_update(update, ui),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => status_open = false,
                }
                result_receiver.try_recv()
            } else {
                result_receiver
                    .recv_timeout(POLL)
                    .map_err(|error| match error {
                        RecvTimeoutError::Timeout => TryRecvError::Empty,
                        RecvTimeoutError::Disconnected => TryRecvError::Disconnected,
                    })
            };
            match finished {
                Ok(result) => {
                    while let Ok(update) = status_receiver.try_recv() {
                        handle_update(update, ui);
                    }
                    return match result {
                        Ok(value) => Ok(value),
                        Err(_) if cancelled.load(Ordering::SeqCst) => Err(PasskeyError::Cancelled),
                        Err(error) => Err(map_error(error)),
                    };
                }
                Err(TryRecvError::Disconnected) => {
                    return Err(PasskeyError::DeviceFailure(
                        "the security key service stopped without an answer",
                    ));
                }
                Err(TryRecvError::Empty) => {}
            }
            if cancelled.load(Ordering::SeqCst) {
                let since = *cancel_seen.get_or_insert_with(Instant::now);
                if since.elapsed() > CANCEL_GRACE {
                    return Err(PasskeyError::Cancelled);
                }
            }
            if Instant::now() > deadline {
                let _ = lock(&self.service).cancel();
                return Err(PasskeyError::NotAllowed);
            }
        }
    }
}

impl SecurityKeyDevice for AuthenticatorRsDevice {
    fn make_credential(
        &self,
        request: &CreateRequest,
        ui: &dyn SecurityKeyUi,
    ) -> Result<CreateResponse, PasskeyError> {
        let args = register_args(request)?;
        let timeout = effective_timeout_ms(request.timeout_ms);
        let result = self.drive(timeout, ui, |service, timeout, status, callback| {
            service.register(timeout, args, status, callback)
        })?;
        create_response(result)
    }

    fn get_assertion(
        &self,
        request: &AssertRequest,
        ui: &dyn SecurityKeyUi,
    ) -> Result<AssertResponse, PasskeyError> {
        let args = sign_args(request);
        let timeout = effective_timeout_ms(request.timeout_ms);
        let result = self.drive(timeout, ui, |service, timeout, status, callback| {
            service.sign(timeout, args, status, callback)
        })?;
        assert_response(result, request)
    }

    fn cancel(&self) {
        let active = lock(&self.active).clone();
        if let Some(cancelled) = active {
            cancelled.store(true, Ordering::SeqCst);
            let _ = lock(&self.service).cancel();
        }
    }
}

/// Passes one status update of the key to the interface.
fn handle_update(update: StatusUpdate, ui: &dyn SecurityKeyUi) {
    match update {
        StatusUpdate::PresenceRequired => ui.touch_notice(),
        StatusUpdate::SelectDeviceNotice => ui.select_device_notice(),
        StatusUpdate::PinUvError(state) => match state {
            StatusPinUv::PinRequired(reply) => answer_pin(ui, reply, None, false),
            StatusPinUv::InvalidPin(reply, retries) => answer_pin(ui, reply, retries, true),
            StatusPinUv::InvalidUv(retries) => ui.uv_failed(retries),
            StatusPinUv::PinBlocked => ui.blocked(SecurityKeyBlock::PinBlocked),
            StatusPinUv::PinAuthBlocked => ui.blocked(SecurityKeyBlock::PinAuthBlocked),
            StatusPinUv::PinNotSet => ui.blocked(SecurityKeyBlock::PinNotSet),
            StatusPinUv::UvBlocked => ui.blocked(SecurityKeyBlock::UvBlocked),
            // These answer a request to set a PIN, which Mercury never makes.
            StatusPinUv::PinIsTooShort | StatusPinUv::PinIsTooLong(_) => {}
        },
        StatusUpdate::SelectResultNotice(reply, users) => {
            let accounts: Vec<UserEntity> = users
                .into_iter()
                .map(|user| UserEntity {
                    id: user.id,
                    name: user.name.unwrap_or_default(),
                    display_name: user.display_name.unwrap_or_default(),
                })
                .collect();
            let choice = ui
                .choose_account(&accounts)
                .filter(|&index| index < accounts.len());
            let _ = reply.send(choice);
        }
        StatusUpdate::InteractiveManagement(_) => {}
    }
}

/// Asks the interface for the PIN and gives it to the key. Dropping `reply` is how the library is
/// told the user cancelled.
fn answer_pin(ui: &dyn SecurityKeyUi, reply: Sender<Pin>, retries: Option<u8>, rejected: bool) {
    if rejected {
        ui.pin_rejected(retries);
    }
    if let Some(pin) = ui.pin(retries) {
        let _ = reply.send(Pin::new(pin.as_str()));
    }
}

/// The signature algorithms handed to a key; anything else a page lists is not offered.
fn cose_algorithm(number: i64) -> Option<COSEAlgorithm> {
    let algorithm = COSEAlgorithm::try_from(number).ok()?;
    matches!(
        algorithm,
        COSEAlgorithm::ES256
            | COSEAlgorithm::EDDSA
            | COSEAlgorithm::RS256
            | COSEAlgorithm::ES384
            | COSEAlgorithm::ES512
            | COSEAlgorithm::PS256
            | COSEAlgorithm::PS384
            | COSEAlgorithm::PS512
            | COSEAlgorithm::RS384
            | COSEAlgorithm::RS512
            | COSEAlgorithm::ES256K
    )
    .then_some(algorithm)
}

fn descriptors(ids: &[Vec<u8>]) -> Vec<PublicKeyCredentialDescriptor> {
    ids.iter()
        .map(|id| PublicKeyCredentialDescriptor {
            id: id.clone(),
            transports: Vec::new(),
        })
        .collect()
}

fn user_verification(wanted: UserVerification) -> UserVerificationRequirement {
    match wanted {
        UserVerification::Required => UserVerificationRequirement::Required,
        UserVerification::Preferred => UserVerificationRequirement::Preferred,
        UserVerification::Discouraged => UserVerificationRequirement::Discouraged,
    }
}

fn register_args(request: &CreateRequest) -> Result<RegisterArgs, PasskeyError> {
    let pub_cred_params: Vec<PublicKeyCredentialParameters> = request
        .algorithms
        .iter()
        .filter_map(|&number| cose_algorithm(number))
        .map(|alg| PublicKeyCredentialParameters { alg })
        .collect();
    if pub_cred_params.is_empty() {
        return Err(PasskeyError::UnsupportedAlgorithm);
    }
    Ok(RegisterArgs {
        client_data_hash: request.client_data_hash,
        relying_party: RelyingParty {
            id: request.rp.id.clone(),
            name: request.rp.name.clone(),
        },
        origin: request.origin.clone(),
        user: PublicKeyCredentialUserEntity {
            id: request.user.id.clone(),
            name: Some(request.user.name.clone()),
            display_name: Some(request.user.display_name.clone()),
        },
        pub_cred_params,
        exclude_list: descriptors(&request.exclude),
        user_verification_req: user_verification(request.user_verification),
        resident_key_req: match request.resident_key {
            ResidentKey::Required => ResidentKeyRequirement::Required,
            ResidentKey::Preferred => ResidentKeyRequirement::Preferred,
            ResidentKey::Discouraged => ResidentKeyRequirement::Discouraged,
        },
        extensions: AuthenticationExtensionsClientInputs::default(),
        pin: None,
        // The library speaks CTAP1 by itself to a key that has no CTAP2.
        use_ctap1_fallback: false,
    })
}

fn sign_args(request: &AssertRequest) -> SignArgs {
    SignArgs {
        client_data_hash: request.client_data_hash,
        origin: request.origin.clone(),
        relying_party_id: request.rp_id.clone(),
        allow_list: descriptors(&request.allow),
        user_verification_req: user_verification(request.user_verification),
        user_presence_req: true,
        extensions: AuthenticationExtensionsClientInputs::default(),
        pin: None,
        use_ctap1_fallback: false,
    }
}

fn cbor_error() -> PasskeyError {
    PasskeyError::DeviceFailure("the security key's answer could not be encoded")
}

/// The bytes the value serialises to as a CBOR byte string (the form `authenticator-rs` uses for
/// authenticator data).
fn serialised_bytes<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, PasskeyError> {
    match Value::serialized(value) {
        Ok(Value::Bytes(bytes)) => Ok(bytes),
        _ => Err(cbor_error()),
    }
}

fn create_response(result: RegisterResult) -> Result<CreateResponse, PasskeyError> {
    let mut attestation = result.att_obj;
    attestation.anonymize();
    let (credential_id, public_key_algorithm, public_key_spki) =
        match &attestation.auth_data.credential_data {
            Some(credential) => (
                credential.credential_id.clone(),
                credential.credential_public_key.alg as i64,
                credential.credential_public_key.der_spki().ok(),
            ),
            None => {
                return Err(PasskeyError::DeviceFailure(
                    "the security key returned no credential",
                ));
            }
        };
    let authenticator_data = serialised_bytes(&attestation.auth_data)?;
    let mut attestation_object = Vec::new();
    into_writer(&attestation, &mut attestation_object).map_err(|_| cbor_error())?;
    Ok(CreateResponse {
        kind: AuthenticatorKind::SecurityKey,
        credential_id,
        attestation_object,
        authenticator_data,
        public_key_algorithm,
        public_key_spki,
        transports: vec![Transport::Usb],
        attachment: Attachment::CrossPlatform,
    })
}

fn assert_response(
    result: SignResult,
    request: &AssertRequest,
) -> Result<AssertResponse, PasskeyError> {
    let assertion = result.assertion;
    let credential_id = match assertion.credentials {
        Some(descriptor) => descriptor.id,
        // CTAP 2.0 keys may leave it out when the allow list had one entry.
        None if request.allow.len() == 1 => request.allow[0].clone(),
        None => {
            return Err(PasskeyError::DeviceFailure(
                "the security key did not say which credential signed",
            ));
        }
    };
    Ok(AssertResponse {
        kind: AuthenticatorKind::SecurityKey,
        credential_id,
        authenticator_data: serialised_bytes(&assertion.auth_data)?,
        signature: assertion.signature,
        user_handle: assertion
            .user
            .map(|user| user.id)
            .filter(|id| !id.is_empty()),
        attachment: Attachment::CrossPlatform,
    })
}

/// The CTAP status byte of a status code.
fn status_byte(code: &StatusCode) -> u8 {
    match code {
        StatusCode::OK => 0x00,
        StatusCode::InvalidCommand => 0x01,
        StatusCode::InvalidParameter => 0x02,
        StatusCode::InvalidLength => 0x03,
        StatusCode::InvalidSeq => 0x04,
        StatusCode::Timeout => 0x05,
        StatusCode::ChannelBusy => 0x06,
        StatusCode::LockRequired => 0x0A,
        StatusCode::InvalidChannel => 0x0B,
        StatusCode::CBORUnexpectedType => 0x11,
        StatusCode::InvalidCBOR => 0x12,
        StatusCode::MissingParameter => 0x14,
        StatusCode::LimitExceeded => 0x15,
        StatusCode::UnsupportedExtension => 0x16,
        StatusCode::CredentialExcluded => 0x19,
        StatusCode::Processing => 0x21,
        StatusCode::InvalidCredential => 0x22,
        StatusCode::UserActionPending => 0x23,
        StatusCode::OperationPending => 0x24,
        StatusCode::NoOperations => 0x25,
        StatusCode::UnsupportedAlgorithm => 0x26,
        StatusCode::OperationDenied => 0x27,
        StatusCode::KeyStoreFull => 0x28,
        StatusCode::NoOperationPending => 0x2A,
        StatusCode::UnsupportedOption => 0x2B,
        StatusCode::InvalidOption => 0x2C,
        StatusCode::KeepaliveCancel => 0x2D,
        StatusCode::NoCredentials => 0x2E,
        StatusCode::UserActionTimeout => 0x2F,
        StatusCode::NotAllowed => 0x30,
        StatusCode::PinInvalid => 0x31,
        StatusCode::PinBlocked => 0x32,
        StatusCode::PinAuthInvalid => 0x33,
        StatusCode::PinAuthBlocked => 0x34,
        StatusCode::PinNotSet => 0x35,
        StatusCode::PinRequired => 0x36,
        StatusCode::PinPolicyViolation => 0x37,
        StatusCode::PinTokenExpired => 0x38,
        StatusCode::RequestTooLarge => 0x39,
        StatusCode::ActionTimeout => 0x3A,
        StatusCode::UpRequired => 0x3B,
        StatusCode::UvBlocked => 0x3C,
        StatusCode::IntegrityFailure => 0x3D,
        StatusCode::InvalidSubcommand => 0x3E,
        StatusCode::UvInvalid => 0x3F,
        StatusCode::UnauthorizedPermission => 0x40,
        StatusCode::Other => 0x7F,
        StatusCode::Unknown(byte) => *byte,
    }
}

/// What a CTAP status byte means to the page. The ones a page must see as a refusal or a timeout
/// become [`PasskeyError::NotAllowed`]; the rest stay [`PasskeyError::Device`] for the interface.
fn from_status(byte: u8) -> PasskeyError {
    match byte {
        0x19 => PasskeyError::CredentialExcluded,
        0x2E => PasskeyError::NoCredentials,
        0x26 => PasskeyError::UnsupportedAlgorithm,
        0x2D => PasskeyError::Cancelled,
        0x05 | 0x27 | 0x2F | 0x3A => PasskeyError::NotAllowed,
        0x2B => {
            PasskeyError::UnsupportedOption("the security key does not support a requested option")
        }
        other => PasskeyError::Device(other),
    }
}

fn map_pin_error(error: &PinError) -> PasskeyError {
    match error {
        PinError::PinRequired => PasskeyError::Device(0x36),
        PinError::PinIsTooShort | PinError::PinIsTooLong(_) => PasskeyError::Device(0x37),
        PinError::InvalidPin(_) => PasskeyError::Device(0x31),
        PinError::InvalidUv(_) => PasskeyError::Device(0x3F),
        PinError::PinAuthBlocked => PasskeyError::Device(0x34),
        PinError::PinBlocked => PasskeyError::Device(0x32),
        PinError::PinNotSet => PasskeyError::Device(0x35),
        PinError::UvBlocked => PasskeyError::Device(0x3C),
        PinError::PinAuthInvalid => PasskeyError::Device(0x33),
        PinError::Crypto(_) => {
            PasskeyError::DeviceFailure("the security key's PIN protocol failed")
        }
    }
}

fn map_hid_error(error: &HIDError) -> PasskeyError {
    match error {
        HIDError::Command(CommandError::StatusCode(code, _)) => from_status(status_byte(code)),
        HIDError::Command(_) => {
            PasskeyError::DeviceFailure("the security key sent an answer that could not be read")
        }
        HIDError::IO(..) => {
            PasskeyError::DeviceFailure("communication with the security key failed")
        }
        HIDError::DeviceNotSupported
        | HIDError::UnsupportedCommand
        | HIDError::UnexpectedVersion => {
            PasskeyError::DeviceFailure("this security key is not supported")
        }
        _ => PasskeyError::DeviceFailure("the security key did not answer as expected"),
    }
}

/// Maps an error of the library to this crate's, keeping nothing from its text: that can hold
/// device paths or request data.
fn map_error(error: AuthenticatorError) -> PasskeyError {
    match error {
        AuthenticatorError::NoConfiguredTransports => PasskeyError::NoDevice,
        AuthenticatorError::CancelledByUser => PasskeyError::Cancelled,
        AuthenticatorError::CredentialExcluded => PasskeyError::CredentialExcluded,
        AuthenticatorError::U2FToken(U2FTokenError::NotAllowed) => PasskeyError::NotAllowed,
        AuthenticatorError::U2FToken(U2FTokenError::NotSupported) => {
            PasskeyError::UnsupportedOption("the security key does not support the request")
        }
        AuthenticatorError::U2FToken(_) => {
            PasskeyError::DeviceFailure("the security key reported an error")
        }
        AuthenticatorError::HIDError(error) => map_hid_error(&error),
        AuthenticatorError::PinError(error) => map_pin_error(&error),
        AuthenticatorError::UnsupportedOption(option) => match option {
            UnsupportedOption::PubCredParams => PasskeyError::UnsupportedAlgorithm,
            UnsupportedOption::ResidentKey => PasskeyError::UnsupportedOption(
                "the security key cannot store a discoverable credential",
            ),
            UnsupportedOption::UserVerification => {
                PasskeyError::UnsupportedOption("the security key cannot verify the user")
            }
            UnsupportedOption::EmptyAllowList => PasskeyError::UnsupportedOption(
                "the security key needs the page to name the credential",
            ),
            UnsupportedOption::CredProtect | UnsupportedOption::MaxPinLength => {
                PasskeyError::UnsupportedOption(
                    "the security key does not support a requested option",
                )
            }
        },
        AuthenticatorError::InvalidRelyingPartyInput => {
            PasskeyError::InvalidInput("the relying party was refused by the security key library")
        }
        AuthenticatorError::Io(_)
        | AuthenticatorError::Platform
        | AuthenticatorError::InternalError(_)
        | AuthenticatorError::Custom(_)
        | AuthenticatorError::VersionMismatch(..)
        | AuthenticatorError::CryptoError => {
            PasskeyError::DeviceFailure("the security key support of this system failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_byte_survives_the_round_trip_the_library_defines() {
        for byte in 0..=u8::MAX {
            let code = StatusCode::from(byte);
            assert_eq!(status_byte(&code), byte, "status {byte:#04x}");
        }
    }

    #[test]
    fn status_bytes_map_to_what_a_page_may_learn() {
        assert_eq!(from_status(0x19), PasskeyError::CredentialExcluded);
        assert_eq!(from_status(0x2E), PasskeyError::NoCredentials);
        assert_eq!(from_status(0x26), PasskeyError::UnsupportedAlgorithm);
        for refusal in [0x05, 0x27, 0x2F, 0x3A] {
            assert_eq!(from_status(refusal), PasskeyError::NotAllowed);
        }
        assert_eq!(from_status(0x32), PasskeyError::Device(0x32));
        assert_eq!(from_status(0x7F), PasskeyError::Device(0x7F));
    }
}
