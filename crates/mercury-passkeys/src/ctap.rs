// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The contract between the engine and an authenticator.
//!
//! The engine (the later CRED-1 lane) checks the page's origin and relying party id, builds
//! `clientDataJSON` and hashes it, and then calls a [`PasskeyAuthenticator`] with a
//! [`CreateRequest`] or an [`AssertRequest`] that already holds the hash. The authenticator never
//! sees the page: it signs what it is given. Every implementation (the built-in manager, a USB
//! security key, later a phone) answers with the same response types, from which the engine builds
//! the page's `PublicKeyCredential`.
//!
//! The calls block until the user has answered or the authenticator has failed. They must run on a
//! worker thread, never on the thread that draws the interface.

use crate::PasskeyError;

/// COSE algorithm number of ECDSA with P-256 and SHA-256, the algorithm the built-in manager uses.
pub const COSE_ES256: i64 = -7;
/// COSE algorithm number of EdDSA.
pub const COSE_EDDSA: i64 = -8;
/// COSE algorithm number of RSASSA-PKCS1-v1_5 with SHA-256.
pub const COSE_RS256: i64 = -257;

/// Which kind of authenticator answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthenticatorKind {
    /// Mercury's own passkey manager (EC42).
    BuiltIn,
    /// A security key over USB (EC43).
    SecurityKey,
}

/// `authenticatorAttachment` of the credential the page receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attachment {
    /// Part of the device: the built-in manager.
    Platform,
    /// Removable: a security key.
    CrossPlatform,
}

/// A transport hint for `getTransports()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// USB.
    Usb,
    /// NFC.
    Nfc,
    /// Bluetooth Low Energy.
    Ble,
    /// Part of the device (the built-in manager).
    Internal,
    /// A phone through the hybrid (QR code) flow.
    Hybrid,
}

/// How much the relying party wants the user verified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UserVerification {
    /// Fail rather than skip verification.
    Required,
    /// Verify if the authenticator can.
    #[default]
    Preferred,
    /// Do not bother the user.
    Discouraged,
}

/// How much the relying party wants a discoverable credential (a passkey that can be found without
/// an allow list).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResidentKey {
    /// Fail rather than make a non-discoverable credential.
    Required,
    /// Make a discoverable credential if possible.
    Preferred,
    /// Make a non-discoverable credential.
    #[default]
    Discouraged,
}

/// `PublicKeyCredentialRpEntity`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelyingParty {
    /// The relying party id the engine checked against the page's origin.
    pub id: String,
    /// The display name the page gave, if any.
    pub name: Option<String>,
}

/// `PublicKeyCredentialUserEntity`. `Debug` shows the length of the id and of the names only.
#[derive(Clone, PartialEq, Eq)]
pub struct UserEntity {
    /// `user.id`, 1 to 64 bytes.
    pub id: Vec<u8>,
    /// `user.name`.
    pub name: String,
    /// `user.displayName`.
    pub display_name: String,
}

impl std::fmt::Debug for UserEntity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserEntity")
            .field("id", &format_args!("<{} bytes>", self.id.len()))
            .field(
                "name",
                &format_args!("<redacted, {} chars>", self.name.chars().count()),
            )
            .field(
                "display_name",
                &format_args!("<redacted, {} chars>", self.display_name.chars().count()),
            )
            .finish()
    }
}

/// `navigator.credentials.create({ publicKey })`, after the engine's checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateRequest {
    /// The page's origin, serialised (`https://example.com`), for the prompt only. The
    /// authenticator trusts `rp.id` and `client_data_hash` as given.
    pub origin: String,
    /// The relying party.
    pub rp: RelyingParty,
    /// The user account.
    pub user: UserEntity,
    /// SHA-256 of the `clientDataJSON` the engine built.
    pub client_data_hash: [u8; 32],
    /// The COSE algorithm numbers the page accepts, most preferred first.
    pub algorithms: Vec<i64>,
    /// Credential ids the page excludes: if the authenticator holds one for this relying party the
    /// operation fails with [`PasskeyError::CredentialExcluded`].
    pub exclude: Vec<Vec<u8>>,
    /// Whether a discoverable credential is wanted.
    pub resident_key: ResidentKey,
    /// Whether user verification is wanted.
    pub user_verification: UserVerification,
    /// The page's timeout in milliseconds, if it gave one. Authenticators that wait on a device
    /// use it; the built-in manager waits for the user for as long as the verifier does.
    pub timeout_ms: Option<u32>,
}

/// The authenticator's answer to a [`CreateRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateResponse {
    /// Which authenticator answered.
    pub kind: AuthenticatorKind,
    /// The new credential id (`rawId`).
    pub credential_id: Vec<u8>,
    /// The CBOR attestation object (`attestationObject`), format `none` for the built-in manager.
    pub attestation_object: Vec<u8>,
    /// The authenticator data inside the attestation object (`getAuthenticatorData()`).
    pub authenticator_data: Vec<u8>,
    /// The credential's signature algorithm, a COSE algorithm number (`getPublicKeyAlgorithm()`).
    pub public_key_algorithm: i64,
    /// The credential's public key as DER SubjectPublicKeyInfo (`getPublicKey()`), when the
    /// authenticator can give it.
    pub public_key_spki: Option<Vec<u8>>,
    /// The transports the credential can be reached over (`getTransports()`).
    pub transports: Vec<Transport>,
    /// `authenticatorAttachment`.
    pub attachment: Attachment,
}

/// `navigator.credentials.get({ publicKey })`, after the engine's checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssertRequest {
    /// The page's origin, serialised, for the prompt only.
    pub origin: String,
    /// The relying party id the engine checked against the page's origin.
    pub rp_id: String,
    /// SHA-256 of the `clientDataJSON` the engine built.
    pub client_data_hash: [u8; 32],
    /// The credential ids the page allows. Empty means any discoverable credential of the relying
    /// party (the account picker).
    pub allow: Vec<Vec<u8>>,
    /// Whether user verification is wanted.
    pub user_verification: UserVerification,
    /// The page's timeout in milliseconds, if it gave one.
    pub timeout_ms: Option<u32>,
}

/// The authenticator's answer to an [`AssertRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssertResponse {
    /// Which authenticator answered.
    pub kind: AuthenticatorKind,
    /// The credential that signed (`rawId`).
    pub credential_id: Vec<u8>,
    /// The authenticator data that was signed (`authenticatorData`).
    pub authenticator_data: Vec<u8>,
    /// The signature over the authenticator data and the client data hash (`signature`).
    pub signature: Vec<u8>,
    /// `user.id` of the credential, when it has one (`userHandle`).
    pub user_handle: Option<Vec<u8>>,
    /// `authenticatorAttachment`.
    pub attachment: Attachment,
}

/// A source of passkeys: the built-in manager, a security key, later a phone.
///
/// The engine holds a list of these and routes a request to the first that can answer (the
/// built-in manager when it holds a matching passkey, a security key when none is stored or the
/// page asks for one). Implementations block until the user has answered; see the module notes.
pub trait PasskeyAuthenticator: Send + Sync {
    /// Which kind of authenticator this is.
    fn kind(&self) -> AuthenticatorKind;

    /// Creates a credential. Errors: [`PasskeyError::NotAllowed`] if the user declined or timed
    /// out, [`PasskeyError::CredentialExcluded`], [`PasskeyError::UnsupportedAlgorithm`],
    /// [`PasskeyError::UnsupportedOption`], [`PasskeyError::InvalidInput`] for a malformed
    /// request, and the device or storage errors of the implementation.
    fn make_credential(&self, request: &CreateRequest) -> Result<CreateResponse, PasskeyError>;

    /// Signs an assertion. Errors: [`PasskeyError::NoCredentials`] if nothing matches,
    /// [`PasskeyError::NotAllowed`] if the user declined or timed out, and the device or storage
    /// errors of the implementation.
    fn get_assertion(&self, request: &AssertRequest) -> Result<AssertResponse, PasskeyError>;

    /// Asks a call that is waiting (on the user or on a device) to stop; it then fails with
    /// [`PasskeyError::Cancelled`]. Safe to call from another thread, and when nothing is waiting.
    fn cancel(&self) {}
}
