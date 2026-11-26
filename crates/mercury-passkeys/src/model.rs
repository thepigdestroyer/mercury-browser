// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::fmt;

use zeroize::Zeroizing;

/// The shortest credential id this crate accepts, in bytes (WebAuthn asks for at least 16).
pub const MIN_CREDENTIAL_ID_BYTES: usize = 16;
/// The longest credential id this crate accepts, in bytes (WebAuthn: at most 1023).
pub const MAX_CREDENTIAL_ID_BYTES: usize = 1023;
/// The longest user handle accepted, in bytes (WebAuthn: at most 64).
pub const MAX_USER_HANDLE_BYTES: usize = 64;
/// The longest relying party id accepted, in bytes.
pub const MAX_RP_ID_BYTES: usize = 253;
/// The longest user name or display name accepted, in bytes of UTF-8.
pub const MAX_NAME_BYTES: usize = 1024;

/// What identifies one stored passkey: the relying party id and the credential id. Two keys are the
/// same passkey exactly when they are equal.
///
/// `Debug` shows the relying party id (a site name) and only the length of the credential id.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PasskeyKey {
    /// The relying party id the passkey is bound to, lower case ASCII (an `xn--` name for
    /// internationalised hosts). Nothing here matches subdomains.
    pub rp_id: String,
    /// The credential id, as the relying party stored it.
    pub credential_id: Vec<u8>,
}

impl fmt::Debug for PasskeyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PasskeyKey")
            .field("rp_id", &self.rp_id)
            .field(
                "credential_id",
                &format_args!("<{} bytes>", self.credential_id.len()),
            )
            .finish()
    }
}

/// A stored passkey without its private key: what lists, pickers and the settings page show.
///
/// `Debug` shows the key (see [`PasskeyKey`]) and only the lengths of the user name and display
/// name, so a log line does not say who the user is.
#[derive(Clone, PartialEq, Eq)]
pub struct PasskeySummary {
    /// The relying party id and credential id.
    pub key: PasskeyKey,
    /// The relying party's identifier for the user (`user.id`); `None` for a non-discoverable
    /// passkey.
    pub user_handle: Option<Vec<u8>>,
    /// `user.name`: shown to the user to tell accounts apart. May be empty.
    pub username: String,
    /// `user.displayName`. May be empty.
    pub display_name: String,
    /// The signature algorithm as a COSE algorithm number (`-7` is ES256).
    pub algorithm: i64,
    /// The signature counter, or `None` when the passkey keeps none (the default: counters do not
    /// suit passkeys that may be exported and imported).
    pub counter: Option<u32>,
    /// When the passkey was created, milliseconds since the Unix epoch, if known.
    pub created_at_ms: Option<i64>,
    /// When the passkey was last changed, milliseconds since the Unix epoch, if known.
    pub modified_at_ms: Option<i64>,
}

impl fmt::Debug for PasskeySummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PasskeySummary")
            .field("key", &self.key)
            .field(
                "user_handle",
                &self
                    .user_handle
                    .as_ref()
                    .map(|handle| format!("<{} bytes>", handle.len())),
            )
            .field(
                "username",
                &format_args!("<redacted, {} chars>", self.username.chars().count()),
            )
            .field(
                "display_name",
                &format_args!("<redacted, {} chars>", self.display_name.chars().count()),
            )
            .field("algorithm", &self.algorithm)
            .field("counter", &self.counter)
            .field("created_at_ms", &self.created_at_ms)
            .field("modified_at_ms", &self.modified_at_ms)
            .finish()
    }
}

/// A passkey's private key in memory: the COSE_Key (RFC 9052) encoding of the key pair, wiped
/// when dropped and never printed.
///
/// `Debug` prints `PrivateKey(…)`. There is no `Display`, no serialisation and no equality, so a
/// key cannot end up in a log line or an assertion message by accident. Read the bytes with
/// [`PrivateKey::expose`] only where they are handed to the signer, the Keychain or an export.
#[derive(Clone)]
pub struct PrivateKey(Zeroizing<Vec<u8>>);

impl PrivateKey {
    /// Takes ownership of `cose_key`, the CBOR encoding of the private COSE_Key; the buffer is wiped
    /// when the key is dropped.
    pub fn new(cose_key: Vec<u8>) -> Self {
        Self(Zeroizing::new(cose_key))
    }

    /// The COSE_Key bytes.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Whether two keys hold the same bytes. The time taken depends on the lengths only.
    pub fn same_as(&self, other: &PrivateKey) -> bool {
        let (left, right) = (self.0.as_slice(), other.0.as_slice());
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .fold(0u8, |difference, (a, b)| difference | (a ^ b))
                == 0
    }
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateKey(…)")
    }
}

/// A stored passkey with its private key. Reading one from a backend is what can make macOS ask the
/// user; listing never does.
#[derive(Clone)]
pub struct PasskeyRecord {
    /// Everything but the private key.
    pub summary: PasskeySummary,
    /// The private key.
    pub private_key: PrivateKey,
}

impl fmt::Debug for PasskeyRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PasskeyRecord")
            .field("summary", &self.summary)
            .field("private_key", &self.private_key)
            .finish()
    }
}

/// What a backend did with a record it was asked to add, when it must not replace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddOutcome {
    /// There was no passkey with this key; the record was added.
    Added,
    /// A passkey with this key already exists; it was left unchanged.
    Exists,
}

/// What a backend did with a record it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// There was no passkey with this key; one was added.
    Created,
    /// A passkey with this key existed and was replaced.
    Replaced,
}
