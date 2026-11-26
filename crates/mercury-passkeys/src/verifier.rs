// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The user-verification hook of the built-in manager.
//!
//! Before the built-in manager creates or uses a passkey it asks a [`UserVerifier`]. The desktop
//! app implements the trait later: a sheet that names the site and account, then Touch ID, with the
//! account password as the fallback (`LAPolicy.deviceOwnerAuthentication`). This crate has no
//! such implementation; tests use `testing::FakeVerifier`.
//!
//! The methods block until the user has answered, on the worker thread the engine calls from.

use crate::{PasskeyError, PasskeySummary, RelyingParty, UserEntity};

/// What the user is being asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerificationPurpose {
    /// Create a new passkey for this account at this site.
    CreatePasskey {
        /// The site.
        rp: RelyingParty,
        /// The account.
        user: UserEntity,
    },
    /// Sign in with this stored passkey.
    UsePasskey(PasskeySummary),
    /// Tell the user that a passkey for this account is already stored here, so no new one is made.
    /// The passkey's details are in the summary.
    ExcludedPasskeyFound(PasskeySummary),
    /// Tell the user that no stored passkey matches the site's request.
    NoPasskeyFound {
        /// The site's relying party id.
        rp_id: String,
    },
    /// Export stored passkeys, private keys included, to a file. The user must be verified.
    ExportPasskeys {
        /// How many passkeys the export holds.
        count: usize,
    },
    /// Remove stored passkeys. The user must be verified.
    DeletePasskeys {
        /// How many passkeys would be removed, if known.
        count: Option<usize>,
    },
}

/// One question for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationRequest {
    /// The page's origin, serialised, shown so the user can see which site asks.
    pub origin: String,
    /// What the user is asked to allow.
    pub purpose: VerificationPurpose,
    /// Whether the user must show they are present (every sign-in and creation: a click on the
    /// prompt).
    pub require_presence: bool,
    /// Whether the user must be verified (Touch ID or the account password).
    pub require_verification: bool,
}

/// The user's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verification {
    /// The user was present and agreed.
    pub presence: bool,
    /// The user was verified (biometrics or password).
    pub verification: bool,
}

/// What a verifier can do. Decided once, when the built-in manager is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifierCapabilities {
    /// Whether it can check that the user is present (can show a prompt).
    pub presence: bool,
    /// `Some(true)`: it can verify the user and is set up to (Touch ID enrolled, or an account
    /// password). `Some(false)`: it can, but is not set up. `None`: it cannot verify.
    pub verification: Option<bool>,
}

/// Asks the user. Implemented by the desktop app; faked in tests.
pub trait UserVerifier: Send + Sync {
    /// What this verifier can do.
    fn capabilities(&self) -> VerifierCapabilities;

    /// Shows `request` and waits. `Ok` with `presence: false` or `verification: false` means the
    /// user declined or failed; the manager turns that into [`PasskeyError::NotAllowed`] when the
    /// request required it. `Err` is for a verifier that failed for another reason; the manager
    /// passes it on.
    fn verify(&self, request: &VerificationRequest) -> Result<Verification, PasskeyError>;

    /// Lets the user pick one of several stored passkeys of a site (the account picker). Called
    /// only with two or more candidates, before [`UserVerifier::verify`]. Returns the index into
    /// `candidates`, or `None` if the user dismissed the picker (the manager then fails with
    /// [`PasskeyError::NotAllowed`]).
    fn choose_passkey(
        &self,
        origin: &str,
        candidates: &[PasskeySummary],
    ) -> Result<Option<usize>, PasskeyError>;

    /// Asks a [`UserVerifier::verify`] or [`UserVerifier::choose_passkey`] that is waiting to give
    /// up. They then return `Err(PasskeyError::Cancelled)`. Safe to call from another thread.
    fn cancel(&self) {}
}

/// Asks the user to allow `purpose` and fails with [`PasskeyError::NotAllowed`] unless the user is
/// present and verified. Used before an export or a removal, which have no site, so the origin is
/// empty.
pub(crate) fn require_verified_user(
    verifier: &dyn UserVerifier,
    purpose: VerificationPurpose,
) -> Result<(), PasskeyError> {
    let request = VerificationRequest {
        origin: String::new(),
        purpose,
        require_presence: true,
        require_verification: true,
    };
    let answer = verifier.verify(&request)?;
    if answer.presence && answer.verification {
        Ok(())
    } else {
        Err(PasskeyError::NotAllowed)
    }
}
