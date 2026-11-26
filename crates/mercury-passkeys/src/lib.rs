// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Passkeys and security keys for Mercury Browser, with no engine in it.
//!
//! This crate is what the engine's WebAuthn surface (a later lane) and the passkey interface call.
//! It has four parts:
//!
//! - **The contract** ([`PasskeyAuthenticator`] and the request and response types in
//!   [`ctap`]): the engine checks the page's origin and relying party id, builds
//!   `clientDataJSON`, and hands an authenticator the hash to sign.
//! - **The built-in manager** ([`builtin`], EC42): a software authenticator from 1Password's
//!   `passkey` crate, with passkeys stored in the macOS Keychain ([`keychain`]), listed and deleted
//!   per site through [`PasskeyStore`], and asking the user through the [`UserVerifier`] trait the
//!   desktop app implements (Touch ID, or the account password).
//! - **Security keys** ([`usb`], EC43): USB CTAP2 keys through Mozilla's `authenticator` crate,
//!   behind the same trait, with a fake key for tests.
//! - **Import and export** ([`cxf`], EC46): the FIDO Credential Exchange Format.
//!
//! A passkey's private key is wiped from memory when dropped where the dependencies allow it, is
//! never printed (`Debug` shows `PrivateKey(…)`), and is never written in the clear: the Keychain
//! encrypts it, and the one place a key leaves the store in the clear, a CXF export, needs an
//! explicit acknowledgement and is written to a file only the user can read.

#![warn(missing_docs)]

mod backend;
pub mod builtin;
pub mod crypto;
pub mod ctap;
pub mod cxf;
mod error;
mod exec;
#[cfg(target_os = "macos")]
pub mod keychain;
mod model;
mod store;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod usb;
#[cfg(target_os = "macos")]
pub mod usb_hid;
mod verifier;

pub use backend::{MemoryBackend, PasskeyBackend};
pub use ctap::{
    AssertRequest, AssertResponse, Attachment, AuthenticatorKind, COSE_EDDSA, COSE_ES256,
    COSE_RS256, CreateRequest, CreateResponse, PasskeyAuthenticator, RelyingParty, ResidentKey,
    Transport, UserEntity, UserVerification,
};
pub use error::PasskeyError;
pub use model::AddOutcome;
pub use model::{
    MAX_CREDENTIAL_ID_BYTES, MAX_NAME_BYTES, MAX_RP_ID_BYTES, MAX_USER_HANDLE_BYTES,
    MIN_CREDENTIAL_ID_BYTES, PasskeyKey, PasskeyRecord, PasskeySummary, PrivateKey, SaveOutcome,
};
pub use store::{
    ImportOutcome, MAX_PRIVATE_KEY_BYTES, PasskeyStore, validate_record, validate_rp_id,
};
pub use verifier::{
    UserVerifier, Verification, VerificationPurpose, VerificationRequest, VerifierCapabilities,
};
