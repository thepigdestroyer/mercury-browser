// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

/// Everything that can go wrong in this crate.
///
/// No variant ever carries a private key, a credential id, a user name, a user handle, a relying
/// party id, a PIN, or a field read from a file: reasons are fixed texts, and operating-system
/// errors carry only their status code and the system's own message for that code. A value of this
/// type is safe to log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PasskeyError {
    /// A request, record or file is not acceptable.
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    /// The user declined, did not verify, or the operation timed out.
    #[error("the user did not allow the operation, or it timed out")]
    NotAllowed,
    /// The operation was cancelled by the caller.
    #[error("the operation was cancelled")]
    Cancelled,
    /// No passkey matches the request.
    #[error("no matching passkey")]
    NoCredentials,
    /// The authenticator already holds a credential the request excludes.
    #[error("a passkey for this account is already registered on this authenticator")]
    CredentialExcluded,
    /// None of the requested signature algorithms is supported.
    #[error("none of the requested algorithms is supported")]
    UnsupportedAlgorithm,
    /// The request needs something this authenticator cannot do.
    #[error("unsupported option: {0}")]
    UnsupportedOption(&'static str),
    /// The store is locked, or the operation needs the user's approval and user interaction is not
    /// allowed in this process.
    #[error(
        "the passkey store is locked, or the operation needs the user and user interaction is not allowed"
    )]
    Locked,
    /// The user, or the system on the user's behalf, refused access to the item.
    #[error("access to the stored passkey was denied")]
    AccessDenied,
    /// An item that had to exist does not.
    #[error("the stored passkey was not found")]
    NotFound,
    /// A passkey with the same relying party and credential id is already stored, and it was
    /// left unchanged.
    #[error("a passkey with this relying party and credential id is already stored")]
    AlreadyExists,
    /// This platform has no such backend.
    #[error("this platform does not support this operation")]
    Unsupported,
    /// A stored item exists but its contents could not be decoded.
    #[error("a stored passkey could not be decoded")]
    CorruptItem,
    /// The operating system's random number generator failed.
    #[error("the operating system's random number generator failed")]
    RandomUnavailable,
    /// A bulk removal removed some passkeys but could not remove at least one; it went on with the
    /// rest.
    #[error("removed {removed} passkeys, but not all of them; the first error was: {first_error}")]
    RemoveIncomplete {
        /// How many passkeys were removed.
        removed: u64,
        /// The first error met.
        first_error: Box<PasskeyError>,
    },
    /// Any other operating-system error.
    #[error("operating system error {code}: {message}")]
    Os {
        /// The status code (an `OSStatus` on macOS).
        code: i32,
        /// The system's description of the status code.
        message: String,
    },
    /// No security key is connected, or none answered.
    #[error("no security key was found")]
    NoDevice,
    /// The security key answered with a CTAP error status.
    #[error("the security key reported error status {0:#04x}")]
    Device(u8),
    /// The security key or its driver failed in some other way.
    #[error("the security key failed: {0}")]
    DeviceFailure(&'static str),
    /// A credential exchange file is not acceptable.
    #[error("not a valid credential exchange file: {0}")]
    InvalidExchangeFile(&'static str),
    /// A bug or an unexpected answer from a dependency.
    #[error("internal error: {0}")]
    Internal(&'static str),
}

impl PasskeyError {
    /// The name of the `DOMException` a page should see for this error, following the WebAuthn
    /// Level 3 rules: a user who declines or a missing credential is `NotAllowedError` (a page
    /// must not learn which), an excluded credential is `InvalidStateError`, and so on.
    pub fn dom_exception_name(&self) -> &'static str {
        match self {
            Self::NotAllowed
            | Self::NoCredentials
            | Self::Locked
            | Self::AccessDenied
            | Self::NoDevice => "NotAllowedError",
            Self::Cancelled => "AbortError",
            Self::CredentialExcluded => "InvalidStateError",
            Self::UnsupportedAlgorithm | Self::UnsupportedOption(_) | Self::Unsupported => {
                "NotSupportedError"
            }
            Self::InvalidInput(_) => "TypeError",
            Self::NotFound
            | Self::AlreadyExists
            | Self::CorruptItem
            | Self::RandomUnavailable
            | Self::RemoveIncomplete { .. }
            | Self::Os { .. }
            | Self::Device(_)
            | Self::DeviceFailure(_)
            | Self::InvalidExchangeFile(_)
            | Self::Internal(_) => "UnknownError",
        }
    }
}
