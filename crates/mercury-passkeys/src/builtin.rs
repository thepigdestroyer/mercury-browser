// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The built-in passkey manager (EC42): a software authenticator from 1Password's `passkey` crate,
//! with passkeys in a [`PasskeyStore`] and the user asked through a [`UserVerifier`].
//!
//! [`BuiltInAuthenticator`] implements [`PasskeyAuthenticator`]. For each call it builds a
//! passkey-rs `Authenticator` over three adapters in this module and drives it with the blocking
//! executor: [`PasskeyCredentialStore`] (storage), [`UserValidation`] (the user) and
//! [`crate::crypto::MercuryCrypto`] (ES256 and the operating system RNG).
//!
//! Things this module does that passkey-rs does not, on purpose:
//!
//! - **The exclude list.** passkey-rs 0.6 asks the user about an excluded credential and then goes
//!   on to create a new one anyway (its own test is named "Excluded id gets ignored"). The manager
//!   therefore checks the exclude list itself, before passkey-rs is called, and fails with
//!   [`PasskeyError::CredentialExcluded`] once the user has acknowledged the prompt.
//! - **The account picker.** With two or more passkeys for a site and no allow list, the user
//!   picks one through [`UserVerifier::choose_passkey`] before anything is signed.
//! - **Errors keep their meaning.** passkey-rs reports only CTAP status codes; a locked Keychain or
//!   a cancelled prompt is remembered on the side and returned instead of the status code.
//!
//! A private key is read from the store only when passkey-rs asks for it, which is after the user
//! has approved the sign-in.

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use passkey::authenticator::{
    Authenticator, CredentialIdLength, CredentialStore, DiscoverabilitySupport, PasskeyAccessor,
    StoreInfo, UiHint, UserCheck, UserValidationMethod,
};
use passkey::crypto::PublicKeyT;
use passkey::crypto::coset::cbor::{self, Value};
use passkey::crypto::coset::{AsCborValue, CborSerializable, CoseKey, RegisteredLabelWithPrivate};
use passkey::crypto::rust_crypto::RustCryptoPublicKey;
use passkey::crypto::{coset::iana::EnumI64, iana};
use passkey::types::ctap2::{
    Aaguid, Ctap2Error, StatusCode, get_assertion as ctap_get, make_credential as ctap_make,
};
use passkey::types::webauthn::{
    AuthenticatorTransport, PublicKeyCredentialDescriptor, PublicKeyCredentialParameters,
    PublicKeyCredentialType, PublicKeyCredentialUserEntity,
};
use passkey::types::{Bytes, CredentialExtensions, Passkey};
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{MercuryCrypto, probe_os_rng, unsynced_backup_flags};
use crate::exec::block_on;
use crate::{
    AssertRequest, AssertResponse, Attachment, AuthenticatorKind, COSE_ES256, CreateRequest,
    CreateResponse, MAX_USER_HANDLE_BYTES, MIN_CREDENTIAL_ID_BYTES, PasskeyAuthenticator,
    PasskeyBackend, PasskeyError, PasskeyKey, PasskeyRecord, PasskeyStore, PasskeySummary,
    PrivateKey, RelyingParty, Transport, UserEntity, UserVerification, UserVerifier, Verification,
    VerificationPurpose, VerificationRequest, VerifierCapabilities, validate_record,
    validate_rp_id,
};

/// The length of the credential ids the built-in authenticator makes, in bytes.
const CREDENTIAL_ID_BYTES: u8 = 32;
/// Room for the CBOR of a COSE_Key with a P-256 private key, so the buffer is never reallocated
/// (a reallocation would leave an unwiped copy behind).
const KEY_BUFFER_BYTES: usize = 512;

/// Errors that passkey-rs cannot carry (it only has CTAP status codes) are put here by the
/// adapters, and read by [`BuiltInAuthenticator`] when the call has failed. The first one wins.
#[derive(Clone, Default)]
struct ErrorSlot(Arc<Mutex<Option<PasskeyError>>>);

impl ErrorSlot {
    fn record(&self, error: PasskeyError) {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        slot.get_or_insert(error);
    }

    fn take(&self) -> Option<PasskeyError> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// Records `error` and gives the status code to return to passkey-rs.
    fn fail(&self, error: PasskeyError) -> StatusCode {
        self.record(error);
        Ctap2Error::Other.into()
    }
}

/// Overwrites every byte string and text string inside `value` with zeros.
fn wipe_value(value: &mut Value) {
    match value {
        // As a slice: zeroing keeps the length, so the structure stays valid and testable.
        Value::Bytes(bytes) => bytes.as_mut_slice().zeroize(),
        Value::Text(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(wipe_value),
        Value::Map(entries) => entries.iter_mut().for_each(|(key, entry)| {
            wipe_value(key);
            wipe_value(entry);
        }),
        Value::Tag(_, inner) => wipe_value(inner),
        _ => {}
    }
}

/// Wipes the parameters (the private scalar among them) of a COSE_Key before it is dropped.
fn wipe_cose_key(key: &mut CoseKey) {
    key.params
        .iter_mut()
        .for_each(|(_, value)| wipe_value(value));
}

/// The CBOR encoding of `key`, in a buffer that is wiped when dropped. The intermediate copies this
/// function makes are wiped too.
fn encode_private_key(key: &CoseKey) -> Result<PrivateKey, PasskeyError> {
    let mut value = key
        .clone()
        .to_cbor_value()
        .map_err(|_| PasskeyError::Internal("a passkey key could not be encoded"))?;
    let mut encoded = Zeroizing::new(Vec::with_capacity(KEY_BUFFER_BYTES));
    let written = cbor::ser::into_writer(&value, &mut *encoded);
    wipe_value(&mut value);
    written.map_err(|_| PasskeyError::Internal("a passkey key could not be encoded"))?;
    Ok(PrivateKey::new(std::mem::take(&mut *encoded)))
}

/// What passkey-rs' CTAP status codes mean for a caller of this crate.
fn error_from_status(status: StatusCode) -> PasskeyError {
    let StatusCode::Known(error) = status else {
        return PasskeyError::Internal("the authenticator failed");
    };
    match error {
        Ctap2Error::OperationDenied
        | Ctap2Error::NotAllowed
        | Ctap2Error::Timeout
        | Ctap2Error::ActionTimeout
        | Ctap2Error::UserActionTimeout => PasskeyError::NotAllowed,
        Ctap2Error::KeepAliveCancel => PasskeyError::Cancelled,
        Ctap2Error::NoCredentials => PasskeyError::NoCredentials,
        Ctap2Error::CredentialExcluded => PasskeyError::CredentialExcluded,
        Ctap2Error::UnsupportedAlgorithm => PasskeyError::UnsupportedAlgorithm,
        Ctap2Error::UnsupportedOption => {
            PasskeyError::UnsupportedOption("the option is not supported")
        }
        Ctap2Error::InvalidOption | Ctap2Error::InvalidParameter | Ctap2Error::MissingParameter => {
            PasskeyError::InvalidInput("the authenticator refused the request")
        }
        Ctap2Error::InvalidCredential => PasskeyError::CorruptItem,
        _ => PasskeyError::Internal("the authenticator failed"),
    }
}

/// A passkey found in the store, as passkey-rs sees it.
///
/// It holds the passkey's summary and a handle to the store. The private key is read from the store
/// when passkey-rs asks for it ([`PasskeyAccessor::key`]), which it does only after the user has
/// approved the sign-in, and is not kept: the item holds no key material between calls.
///
/// `Debug` shows the summary, which hides the user's names (see [`PasskeySummary`]).
pub struct StoredPasskey<B: PasskeyBackend> {
    summary: PasskeySummary,
    store: Arc<PasskeyStore<B>>,
    problems: ErrorSlot,
}

impl<B: PasskeyBackend> StoredPasskey<B> {
    /// The passkey without its private key.
    pub fn summary(&self) -> &PasskeySummary {
        &self.summary
    }

    fn load_key(&self) -> Option<CoseKey> {
        let record = match self.store.record(&self.summary.key) {
            Ok(Some(record)) => record,
            Ok(None) => {
                self.problems.record(PasskeyError::NotFound);
                return None;
            }
            Err(error) => {
                self.problems.record(error);
                return None;
            }
        };
        match CoseKey::from_slice(record.private_key.expose()) {
            Ok(key) => Some(key),
            Err(_) => {
                self.problems.record(PasskeyError::CorruptItem);
                None
            }
        }
    }
}

impl<B: PasskeyBackend> fmt::Debug for StoredPasskey<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredPasskey")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

impl<B: PasskeyBackend> PasskeyAccessor for StoredPasskey<B> {
    /// The private COSE_Key, read from the store now. If it cannot be read, an empty key is
    /// returned (passkey-rs then fails to sign with it, as it has no algorithm) and the reason is
    /// remembered for [`BuiltInAuthenticator`] to report.
    fn key(&self) -> Cow<'_, CoseKey> {
        Cow::Owned(self.load_key().unwrap_or_default())
    }

    fn credential_id(&self) -> &[u8] {
        &self.summary.key.credential_id
    }

    fn rp_id(&self) -> &str {
        &self.summary.key.rp_id
    }

    fn user_handle(&self) -> Option<&[u8]> {
        self.summary.user_handle.as_deref()
    }

    fn username(&self) -> Option<&str> {
        Some(&self.summary.username)
    }

    fn user_display_name(&self) -> Option<&str> {
        Some(&self.summary.display_name)
    }

    fn counter(&self) -> Option<u32> {
        self.summary.counter
    }

    fn set_counter(&mut self, counter: u32) {
        self.summary.counter = Some(counter);
    }

    fn extensions(&self) -> Cow<'_, CredentialExtensions> {
        Cow::Owned(CredentialExtensions::default())
    }
}

/// passkey-rs' `CredentialStore` over a [`PasskeyStore`].
///
/// Every passkey is discoverable (it keeps its user handle and names, so it can be listed and
/// shown in the account picker). `find_credentials` matches the relying party exactly, filters by
/// the allow list, and returns the newest passkey first. `save_credential` goes through
/// [`PasskeyStore::save`], so its rules apply, and refuses credentials with an `hmac-secret`
/// extension. A passkey is never replaced by creating another for the same account: both stay,
/// newest first, until the user deletes one.
pub struct PasskeyCredentialStore<B: PasskeyBackend> {
    store: Arc<PasskeyStore<B>>,
    problems: ErrorSlot,
}

impl<B: PasskeyBackend> PasskeyCredentialStore<B> {
    /// An adapter over `store`.
    pub fn new(store: Arc<PasskeyStore<B>>) -> Self {
        Self {
            store,
            problems: ErrorSlot::default(),
        }
    }

    fn sharing(store: Arc<PasskeyStore<B>>, problems: ErrorSlot) -> Self {
        Self { store, problems }
    }

    fn save(&self, passkey: &Passkey) -> Result<(), PasskeyError> {
        if passkey.extensions.hmac_secret.is_some() {
            return Err(PasskeyError::UnsupportedOption(
                "the hmac-secret extension is not supported",
            ));
        }
        let algorithm = match &passkey.key.alg {
            Some(RegisteredLabelWithPrivate::Assigned(algorithm)) => algorithm.to_i64(),
            _ => return Err(PasskeyError::InvalidInput("the key has no algorithm")),
        };
        let record = PasskeyRecord {
            summary: PasskeySummary {
                key: PasskeyKey {
                    rp_id: passkey.rp_id.clone(),
                    credential_id: passkey.credential_id.to_vec(),
                },
                user_handle: passkey.user_handle.as_ref().map(|handle| handle.to_vec()),
                username: passkey.username.clone().unwrap_or_default(),
                display_name: passkey.user_display_name.clone().unwrap_or_default(),
                algorithm,
                counter: passkey.counter,
                created_at_ms: None,
                modified_at_ms: None,
            },
            private_key: encode_private_key(&passkey.key)?,
        };
        self.store.save(&record).map(|_| ())
    }
}

impl<B: PasskeyBackend> fmt::Debug for PasskeyCredentialStore<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PasskeyCredentialStore")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<B: PasskeyBackend + 'static> CredentialStore for PasskeyCredentialStore<B> {
    type PasskeyItem = StoredPasskey<B>;

    async fn find_credentials(
        &self,
        ids: Option<&[PublicKeyCredentialDescriptor]>,
        rp_id: &str,
        _user_handle: Option<&[u8]>,
    ) -> Result<Vec<Self::PasskeyItem>, StatusCode> {
        let restricted = ids.is_some_and(|list| !list.is_empty());
        let allow: Vec<Vec<u8>> = ids
            .into_iter()
            .flatten()
            .filter(|descriptor| descriptor.is_known())
            .map(|descriptor| descriptor.id.to_vec())
            .collect();
        // A list of only unknown credential types allows nothing; it must not turn into "any".
        if restricted && allow.is_empty() {
            return Err(Ctap2Error::NoCredentials.into());
        }
        let found = self
            .store
            .find(rp_id, &allow)
            .map_err(|error| self.problems.fail(error))?;
        if found.is_empty() {
            return Err(Ctap2Error::NoCredentials.into());
        }
        Ok(found
            .into_iter()
            .map(|summary| StoredPasskey {
                summary,
                store: Arc::clone(&self.store),
                problems: self.problems.clone(),
            })
            .collect())
    }

    async fn save_credential(
        &mut self,
        mut passkey: Passkey,
        _user: ctap_make::PublicKeyCredentialUserEntity,
        _rp: ctap_make::PublicKeyCredentialRpEntity,
        _options: ctap_make::Options,
    ) -> Result<(), StatusCode> {
        let saved = self.save(&passkey);
        wipe_cose_key(&mut passkey.key);
        saved.map_err(|error| self.problems.fail(error))
    }

    async fn update_credential(&mut self, passkey: &Self::PasskeyItem) -> Result<(), StatusCode> {
        let Some(counter) = passkey.summary.counter else {
            return Ok(());
        };
        match self.store.set_counter(&passkey.summary.key, counter) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Ctap2Error::NoCredentials.into()),
            Err(error) => Err(self.problems.fail(error)),
        }
    }

    async fn get_info(&self) -> StoreInfo {
        StoreInfo {
            discoverability: DiscoverabilitySupport::ForcedDiscoverable,
        }
    }
}

/// passkey-rs' `UserValidationMethod` over a [`UserVerifier`], for one operation at one site.
///
/// passkey-rs tells it what is about to happen (create a passkey, use one, tell the user that one
/// exists or that none matches); it turns that into a [`VerificationRequest`] naming the page's
/// origin, and the user's answer back into passkey-rs' terms. If the verifier fails with an error,
/// the error is remembered for [`BuiltInAuthenticator`] to return.
pub struct UserValidation<B: PasskeyBackend> {
    verifier: Arc<dyn UserVerifier>,
    capabilities: VerifierCapabilities,
    origin: String,
    rp_id: String,
    problems: ErrorSlot,
    backend: PhantomData<fn() -> B>,
}

impl<B: PasskeyBackend> UserValidation<B> {
    /// An adapter that asks `verifier` on behalf of the page at `origin`, whose relying party id is
    /// `rp_id`.
    pub fn new(verifier: Arc<dyn UserVerifier>, origin: &str, rp_id: &str) -> Self {
        let capabilities = verifier.capabilities();
        Self::sharing(verifier, capabilities, origin, rp_id, ErrorSlot::default())
    }

    fn sharing(
        verifier: Arc<dyn UserVerifier>,
        capabilities: VerifierCapabilities,
        origin: &str,
        rp_id: &str,
        problems: ErrorSlot,
    ) -> Self {
        Self {
            verifier,
            capabilities,
            origin: origin.to_owned(),
            rp_id: rp_id.to_owned(),
            problems,
            backend: PhantomData,
        }
    }
}

impl<B: PasskeyBackend> fmt::Debug for UserValidation<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserValidation")
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<B: PasskeyBackend + 'static> UserValidationMethod for UserValidation<B> {
    type PasskeyItem = StoredPasskey<B>;

    async fn check_user<'a>(
        &self,
        hint: UiHint<'a, Self::PasskeyItem>,
        presence: bool,
        verification: bool,
    ) -> Result<UserCheck, Ctap2Error> {
        let purpose = match hint {
            UiHint::InformExcludedCredentialFound(item) => {
                VerificationPurpose::ExcludedPasskeyFound(item.summary.clone())
            }
            UiHint::InformNoCredentialsFound => VerificationPurpose::NoPasskeyFound {
                rp_id: self.rp_id.clone(),
            },
            UiHint::RequestNewCredential(user, rp) => VerificationPurpose::CreatePasskey {
                rp: RelyingParty {
                    id: rp.id.clone(),
                    name: rp.name.clone(),
                },
                user: UserEntity {
                    id: user.id.to_vec(),
                    name: user.name.clone().unwrap_or_default(),
                    display_name: user.display_name.clone().unwrap_or_default(),
                },
            },
            UiHint::RequestExistingCredential(item) => {
                VerificationPurpose::UsePasskey(item.summary.clone())
            }
        };
        let request = VerificationRequest {
            origin: self.origin.clone(),
            purpose,
            require_presence: presence,
            require_verification: verification,
        };
        match self.verifier.verify(&request) {
            Ok(Verification {
                presence,
                verification,
            }) => Ok(UserCheck {
                presence,
                verification,
            }),
            Err(error) => {
                self.problems.record(error);
                Err(Ctap2Error::OperationDenied)
            }
        }
    }

    fn is_presence_enabled(&self) -> bool {
        self.capabilities.presence
    }

    fn is_verification_enabled(&self) -> Option<bool> {
        self.capabilities.verification
    }
}

/// Mercury's own passkey manager (EC42): creates and uses passkeys kept in a [`PasskeyStore`],
/// asking the user through a [`UserVerifier`].
///
/// - A passkey is never created or used without a `verify` call that returned presence.
/// - ES256 only; the credential id is 32 random bytes from the operating system; the AAGUID is
///   all zeros and the attestation format is `none`; passkeys have no signature counter.
/// - The authenticator data carries passkey-rs' default flags (backup eligible and backed up) in
///   addition to user presence and, when the user was verified, user verification.
/// - [`UserVerification::Required`] needs a verifier whose capabilities say it can verify the user
///   (`verification == Some(true)`); otherwise the call fails with
///   [`PasskeyError::UnsupportedOption`] before the user is asked anything.
///
/// The calls block until the user has answered; run them on a worker thread.
pub struct BuiltInAuthenticator<B: PasskeyBackend + 'static> {
    store: Arc<PasskeyStore<B>>,
    verifier: Arc<dyn UserVerifier>,
    capabilities: VerifierCapabilities,
}

impl<B: PasskeyBackend + 'static> BuiltInAuthenticator<B> {
    /// A manager over `store` that asks `verifier`. Reads the verifier's capabilities once.
    ///
    /// Fails with [`PasskeyError::RandomUnavailable`] if the operating system's random number
    /// generator does not answer: keys and credential ids need it.
    pub fn new(
        store: Arc<PasskeyStore<B>>,
        verifier: Arc<dyn UserVerifier>,
    ) -> Result<Self, PasskeyError> {
        probe_os_rng()?;
        let capabilities = verifier.capabilities();
        Ok(Self {
            store,
            verifier,
            capabilities,
        })
    }

    /// Whether user verification should be asked for, given what the page wants and what the
    /// verifier can do.
    fn wants_verification(&self, wanted: UserVerification) -> Result<bool, PasskeyError> {
        let capable = self.capabilities.verification == Some(true);
        match wanted {
            UserVerification::Required if !capable => Err(PasskeyError::UnsupportedOption(
                "user verification is not available",
            )),
            UserVerification::Required => Ok(true),
            UserVerification::Preferred => Ok(capable),
            UserVerification::Discouraged => Ok(false),
        }
    }

    fn authenticator(
        &self,
        origin: &str,
        rp_id: &str,
        problems: &ErrorSlot,
    ) -> Authenticator<PasskeyCredentialStore<B>, UserValidation<B>, MercuryCrypto> {
        let mut authenticator = Authenticator::new(
            Aaguid::new_empty(),
            PasskeyCredentialStore::sharing(Arc::clone(&self.store), problems.clone()),
            UserValidation::sharing(
                Arc::clone(&self.verifier),
                self.capabilities,
                origin,
                rp_id,
                problems.clone(),
            ),
            MercuryCrypto::new(),
        )
        .transports(vec![AuthenticatorTransport::Internal]);
        authenticator.set_make_credential_id_length(CredentialIdLength::from(CREDENTIAL_ID_BYTES));
        authenticator
    }

    /// Checks a creation request before the user is asked anything. Returns whether user
    /// verification is wanted.
    fn check_create(&self, request: &CreateRequest) -> Result<bool, PasskeyError> {
        validate_rp_id(&request.rp.id)?;
        if request.user.id.is_empty() || request.user.id.len() > MAX_USER_HANDLE_BYTES {
            return Err(PasskeyError::InvalidInput(
                "the user id has an unacceptable length",
            ));
        }
        if request.algorithms.is_empty() {
            return Err(PasskeyError::InvalidInput(
                "no signature algorithm was requested",
            ));
        }
        // The names are saved with the passkey. Check them now with the rules `save` will apply, so
        // that a name the store would refuse does not come up after the user has approved.
        validate_record(&PasskeyRecord {
            summary: PasskeySummary {
                key: PasskeyKey {
                    rp_id: request.rp.id.clone(),
                    credential_id: vec![0; MIN_CREDENTIAL_ID_BYTES],
                },
                user_handle: Some(request.user.id.clone()),
                username: request.user.name.clone(),
                display_name: request.user.display_name.clone(),
                algorithm: COSE_ES256,
                counter: None,
                created_at_ms: None,
                modified_at_ms: None,
            },
            private_key: PrivateKey::new(vec![0]),
        })?;
        if !request.algorithms.contains(&COSE_ES256) {
            return Err(PasskeyError::UnsupportedAlgorithm);
        }
        self.wants_verification(request.user_verification)
    }

    /// Fails with [`PasskeyError::CredentialExcluded`] if the store holds a passkey of this
    /// relying party that the request excludes, once the user has acknowledged it. (passkey-rs
    /// does not do this: see the module notes.)
    fn refuse_excluded(&self, request: &CreateRequest) -> Result<(), PasskeyError> {
        if request.exclude.is_empty() {
            return Ok(());
        }
        let Some(excluded) = self
            .store
            .find(&request.rp.id, &request.exclude)?
            .into_iter()
            .next()
        else {
            return Ok(());
        };
        let answer = self.verifier.verify(&VerificationRequest {
            origin: request.origin.clone(),
            purpose: VerificationPurpose::ExcludedPasskeyFound(excluded),
            require_presence: true,
            require_verification: false,
        })?;
        if answer.presence {
            Err(PasskeyError::CredentialExcluded)
        } else {
            Err(PasskeyError::NotAllowed)
        }
    }

    /// Lets the user pick when several passkeys match and the page did not name one. Returns the
    /// allow list to give passkey-rs.
    fn choose(&self, request: &AssertRequest) -> Result<Vec<Vec<u8>>, PasskeyError> {
        if !request.allow.is_empty() {
            return Ok(request.allow.clone());
        }
        let candidates = self.store.for_rp(&request.rp_id)?;
        if candidates.len() < 2 {
            return Ok(Vec::new());
        }
        let Some(index) = self.verifier.choose_passkey(&request.origin, &candidates)? else {
            return Err(PasskeyError::NotAllowed);
        };
        let chosen = candidates.get(index).ok_or(PasskeyError::Internal(
            "the verifier chose a passkey that does not exist",
        ))?;
        Ok(vec![chosen.key.credential_id.clone()])
    }
}

impl<B: PasskeyBackend + 'static> fmt::Debug for BuiltInAuthenticator<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuiltInAuthenticator")
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

fn descriptor(id: &[u8]) -> PublicKeyCredentialDescriptor {
    PublicKeyCredentialDescriptor {
        ty: PublicKeyCredentialType::PublicKey,
        id: Bytes::from(id),
        transports: None,
    }
}

impl<B: PasskeyBackend + 'static> PasskeyAuthenticator for BuiltInAuthenticator<B> {
    fn kind(&self) -> AuthenticatorKind {
        AuthenticatorKind::BuiltIn
    }

    fn make_credential(&self, request: &CreateRequest) -> Result<CreateResponse, PasskeyError> {
        let verify_user = self.check_create(request)?;
        self.refuse_excluded(request)?;

        let problems = ErrorSlot::default();
        let mut authenticator = self.authenticator(&request.origin, &request.rp.id, &problems);
        let ctap_request = ctap_make::Request {
            client_data_hash: Bytes::from(request.client_data_hash.as_slice()),
            rp: ctap_make::PublicKeyCredentialRpEntity {
                id: request.rp.id.clone(),
                name: request.rp.name.clone(),
            },
            user: PublicKeyCredentialUserEntity {
                id: Bytes::from(request.user.id.as_slice()),
                name: request.user.name.clone(),
                display_name: request.user.display_name.clone(),
            },
            pub_key_cred_params: request
                .algorithms
                .iter()
                .filter_map(|number| iana::Algorithm::from_i64(*number))
                .map(|alg| PublicKeyCredentialParameters {
                    ty: PublicKeyCredentialType::PublicKey,
                    alg,
                })
                .collect(),
            // Checked above: passkey-rs would prompt about an excluded credential and then ignore it.
            exclude_list: None,
            extensions: None,
            options: ctap_make::Options {
                rk: request.resident_key != crate::ResidentKey::Discouraged,
                up: true,
                uv: verify_user,
            },
            pin_auth: None,
            pin_protocol: None,
        };
        let mut response = block_on(authenticator.make_credential(ctap_request))
            .map_err(|status| problems.take().unwrap_or_else(|| error_from_status(status)))?;
        response.auth_data.flags = unsynced_backup_flags(response.auth_data.flags);

        let attested =
            response
                .auth_data
                .attested_credential_data
                .as_ref()
                .ok_or(PasskeyError::Internal(
                    "the authenticator returned no credential data",
                ))?;
        Ok(CreateResponse {
            kind: AuthenticatorKind::BuiltIn,
            credential_id: attested.credential_id().to_vec(),
            attestation_object: response.as_webauthn_bytes().to_vec(),
            authenticator_data: response.auth_data.to_vec(),
            public_key_algorithm: COSE_ES256,
            public_key_spki: RustCryptoPublicKey::der_from_cose_key(&attested.key).ok(),
            transports: vec![Transport::Internal],
            attachment: Attachment::Platform,
        })
    }

    fn get_assertion(&self, request: &AssertRequest) -> Result<AssertResponse, PasskeyError> {
        validate_rp_id(&request.rp_id)?;
        let verify_user = self.wants_verification(request.user_verification)?;
        // With nothing stored for the site, passkey-rs still asks the user (to tell them so) before
        // it fails: the page must not learn that nothing matched without the user's involvement.
        let allow = self.choose(request)?;

        let problems = ErrorSlot::default();
        let mut authenticator = self.authenticator(&request.origin, &request.rp_id, &problems);
        let ctap_request = ctap_get::Request {
            rp_id: request.rp_id.clone(),
            client_data_hash: Bytes::from(request.client_data_hash.as_slice()),
            allow_list: (!allow.is_empty())
                .then(|| allow.iter().map(|id| descriptor(id)).collect()),
            extensions: None,
            options: ctap_get::Options {
                up: true,
                uv: verify_user,
            },
            pin_auth: None,
            pin_protocol: None,
        };
        let mut response = block_on(authenticator.get_assertion(ctap_request))
            .map_err(|status| problems.take().unwrap_or_else(|| error_from_status(status)))?;
        response.auth_data.flags = unsynced_backup_flags(response.auth_data.flags);

        let credential = response.credential.as_ref().ok_or(PasskeyError::Internal(
            "the authenticator returned no credential",
        ))?;
        Ok(AssertResponse {
            kind: AuthenticatorKind::BuiltIn,
            credential_id: credential.id.to_vec(),
            authenticator_data: response.auth_data.to_vec(),
            signature: response.signature.to_vec(),
            user_handle: response.user.as_ref().map(|user| user.id.to_vec()),
            attachment: Attachment::Platform,
        })
    }

    fn cancel(&self) {
        self.verifier.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_recorded_error_wins() {
        let slot = ErrorSlot::default();
        slot.record(PasskeyError::Locked);
        slot.record(PasskeyError::NotFound);
        assert_eq!(slot.take(), Some(PasskeyError::Locked));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn status_codes_map_to_the_errors_a_page_may_see() {
        let cases = [
            (Ctap2Error::OperationDenied, PasskeyError::NotAllowed),
            (Ctap2Error::NoCredentials, PasskeyError::NoCredentials),
            (
                Ctap2Error::CredentialExcluded,
                PasskeyError::CredentialExcluded,
            ),
            (
                Ctap2Error::UnsupportedAlgorithm,
                PasskeyError::UnsupportedAlgorithm,
            ),
            (Ctap2Error::KeepAliveCancel, PasskeyError::Cancelled),
            (Ctap2Error::InvalidCredential, PasskeyError::CorruptItem),
        ];
        for (status, expected) in cases {
            assert_eq!(error_from_status(status.into()), expected);
        }
        assert!(matches!(
            error_from_status(Ctap2Error::UnsupportedOption.into()),
            PasskeyError::UnsupportedOption(_)
        ));
        assert!(matches!(
            error_from_status(Ctap2Error::Other.into()),
            PasskeyError::Internal(_)
        ));
        assert!(matches!(
            error_from_status(StatusCode::from(0x08u8)),
            PasskeyError::Internal(_)
        ));
    }

    #[test]
    fn wiping_a_value_clears_nested_byte_strings() {
        let mut value = Value::Map(vec![(
            Value::Integer(1.into()),
            Value::Array(vec![
                Value::Bytes(vec![7; 8]),
                Value::Tag(5, Box::new(Value::Bytes(vec![9; 4]))),
            ]),
        )]);
        wipe_value(&mut value);
        let Value::Map(entries) = &value else {
            panic!("a map");
        };
        let Value::Array(items) = &entries[0].1 else {
            panic!("an array");
        };
        assert_eq!(items[0], Value::Bytes(vec![0; 8]));
        let Value::Tag(_, inner) = &items[1] else {
            panic!("a tag");
        };
        assert_eq!(**inner, Value::Bytes(vec![0; 4]));
    }

    #[test]
    fn wiping_a_cose_key_zeroes_its_private_scalar() {
        let mut key = passkey::crypto::coset::CoseKeyBuilder::new_ec2_priv_key(
            iana::EllipticCurve::P_256,
            vec![1; 32],
            vec![2; 32],
            vec![3; 32],
        )
        .build();
        wipe_cose_key(&mut key);
        let bytes: Vec<&Vec<u8>> = key
            .params
            .iter()
            .filter_map(|(_, value)| match value {
                Value::Bytes(bytes) => Some(bytes),
                _ => None,
            })
            .collect();
        assert_eq!(bytes.len(), 3, "x, y and d are still there");
        assert!(
            bytes
                .iter()
                .all(|b| b.len() == 32 && b.iter().all(|byte| *byte == 0))
        );
    }

    #[test]
    fn an_encoded_key_decodes_to_the_same_key() {
        let key = passkey::crypto::coset::CoseKeyBuilder::new_ec2_priv_key(
            iana::EllipticCurve::P_256,
            vec![1; 32],
            vec![2; 32],
            vec![3; 32],
        )
        .algorithm(iana::Algorithm::ES256)
        .build();
        let encoded = encode_private_key(&key).expect("encoding");
        assert_eq!(
            CoseKey::from_slice(encoded.expose()).expect("decoding"),
            key
        );
    }
}
