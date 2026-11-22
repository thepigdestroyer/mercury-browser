// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! USB security keys (EC43) behind [`PasskeyAuthenticator`].
//!
//! [`SecurityKeyAuthenticator`] checks a request the way the built-in manager does, hands it to a
//! [`SecurityKeyDevice`], and checks what comes back: the authenticator data must be for the
//! relying party asked about, and an assertion must be for a credential the page allowed. The
//! device does the CTAP2 work; on macOS [`crate::usb_hid::AuthenticatorRsDevice`] is the real one
//! and [`crate::testing::FakeSecurityKey`] the test double.
//!
//! While a key waits for the user it tells the interface through [`SecurityKeyUi`]. Those calls
//! happen on the thread that called `make_credential` or `get_assertion`, and a call that asks for
//! a PIN blocks that thread until the user has answered.

use std::sync::Arc;

use passkey::crypto::{hash::Sha256Backend, rust_crypto::RustCryptoSha2};
use zeroize::Zeroizing;

use crate::{
    AssertRequest, AssertResponse, AuthenticatorKind, CreateRequest, CreateResponse,
    MAX_CREDENTIAL_ID_BYTES, MAX_NAME_BYTES, MAX_USER_HANDLE_BYTES, PasskeyAuthenticator,
    PasskeyError, UserEntity, validate_rp_id,
};

/// How long a security key is given when the page names no timeout.
pub const DEFAULT_TIMEOUT_MS: u32 = 120_000;
/// The longest a security key is waited for, whatever the page asks.
pub const MAX_TIMEOUT_MS: u32 = 300_000;
/// The most credential ids accepted in one exclude or allow list.
pub const MAX_CREDENTIAL_LIST: usize = 1024;

/// The page's timeout as the time to wait: [`DEFAULT_TIMEOUT_MS`] if the page gave none (or zero),
/// never more than [`MAX_TIMEOUT_MS`].
pub fn effective_timeout_ms(requested: Option<u32>) -> u32 {
    match requested {
        None | Some(0) => DEFAULT_TIMEOUT_MS,
        Some(milliseconds) => milliseconds.min(MAX_TIMEOUT_MS),
    }
}

/// A state a security key can be stuck in that the user has to fix elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityKeyBlock {
    /// Too many wrong PINs: the key must be reset.
    PinBlocked,
    /// Too many wrong PINs in a row: unplug the key and plug it in again.
    PinAuthBlocked,
    /// The request needs a PIN and the key has none: set one first.
    PinNotSet,
    /// The key's own verification (a fingerprint) is blocked: use the PIN.
    UvBlocked,
}

/// What the interface does while a security key works. Every method runs on the thread that made
/// the request; none may call back into the authenticator except [`PasskeyAuthenticator::cancel`].
pub trait SecurityKeyUi: Send + Sync {
    /// Several keys are plugged in: ask the user to touch the one to use.
    fn select_device_notice(&self) {}

    /// The key waits for the user to touch it.
    fn touch_notice(&self) {}

    /// The key wants its PIN. `retries` is how many wrong PINs the key still allows, when it said
    /// so (it does not on the first prompt). Returns `None` when the user cancelled; that fails the
    /// request with [`PasskeyError::Cancelled`]. The default cancels, so an interface that cannot
    /// ask for a PIN fails cleanly instead of hanging.
    fn pin(&self, retries: Option<u8>) -> Option<Zeroizing<String>> {
        let _ = retries;
        None
    }

    /// The PIN just given was wrong; `pin` is called again right after. `retries` is how many
    /// attempts remain, when the key said so.
    fn pin_rejected(&self, retries: Option<u8>) {
        let _ = retries;
    }

    /// The key's own verification (a fingerprint) failed; the user may try again.
    fn uv_failed(&self, retries: Option<u8>) {
        let _ = retries;
    }

    /// The key cannot go on until the user fixes `state`; the request is about to fail.
    fn blocked(&self, state: SecurityKeyBlock) {
        let _ = state;
    }

    /// The key holds several accounts for the site and the page named none: returns the index of
    /// the one to use, or `None` to cancel. `accounts` carries the user entities the key reported.
    /// The default cancels rather than choose an account for the user.
    fn choose_account(&self, accounts: &[UserEntity]) -> Option<usize> {
        let _ = accounts;
        None
    }

    /// [`PasskeyAuthenticator::cancel`] was called: dismiss whatever is shown, and make a pending
    /// [`pin`](Self::pin) return `None` so the request can end.
    fn cancelled(&self) {}
}

/// A security key, or a stand-in for one, that can make and use credentials.
///
/// Both calls block until the key has answered, the user has cancelled, or the request timed out
/// (`timeout_ms` on the request; see [`effective_timeout_ms`]). They are called with requests
/// [`SecurityKeyAuthenticator`] has already validated.
pub trait SecurityKeyDevice: Send + Sync {
    /// Makes a credential on the key.
    fn make_credential(
        &self,
        request: &CreateRequest,
        ui: &dyn SecurityKeyUi,
    ) -> Result<CreateResponse, PasskeyError>;

    /// Signs an assertion with a credential on the key.
    fn get_assertion(
        &self,
        request: &AssertRequest,
        ui: &dyn SecurityKeyUi,
    ) -> Result<AssertResponse, PasskeyError>;

    /// Stops a call that is waiting; it fails with [`PasskeyError::Cancelled`]. Does nothing when
    /// no call is waiting.
    fn cancel(&self);
}

/// A [`PasskeyAuthenticator`] backed by a security key.
pub struct SecurityKeyAuthenticator<D> {
    device: D,
    ui: Arc<dyn SecurityKeyUi>,
}

impl<D: SecurityKeyDevice> SecurityKeyAuthenticator<D> {
    /// An authenticator that talks to `device` and shows its prompts through `ui`.
    pub fn new(device: D, ui: Arc<dyn SecurityKeyUi>) -> Self {
        Self { device, ui }
    }

    /// The device, for tests and for callers that keep a handle on it.
    pub fn device(&self) -> &D {
        &self.device
    }
}

impl<D> std::fmt::Debug for SecurityKeyAuthenticator<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecurityKeyAuthenticator")
            .finish_non_exhaustive()
    }
}

impl<D: SecurityKeyDevice> PasskeyAuthenticator for SecurityKeyAuthenticator<D> {
    fn kind(&self) -> AuthenticatorKind {
        AuthenticatorKind::SecurityKey
    }

    fn make_credential(&self, request: &CreateRequest) -> Result<CreateResponse, PasskeyError> {
        validate_create(request)?;
        let mut response = self.device.make_credential(request, self.ui.as_ref())?;
        check_create_response(request, &response)?;
        response.kind = AuthenticatorKind::SecurityKey;
        Ok(response)
    }

    fn get_assertion(&self, request: &AssertRequest) -> Result<AssertResponse, PasskeyError> {
        validate_assert(request)?;
        let mut response = self.device.get_assertion(request, self.ui.as_ref())?;
        check_assert_response(request, &response)?;
        response.kind = AuthenticatorKind::SecurityKey;
        Ok(response)
    }

    fn cancel(&self) {
        self.ui.cancelled();
        self.device.cancel();
    }
}

fn validate_credential_list(ids: &[Vec<u8>]) -> Result<(), PasskeyError> {
    if ids.len() > MAX_CREDENTIAL_LIST {
        return Err(PasskeyError::InvalidInput(
            "too many credential ids in the list",
        ));
    }
    if ids
        .iter()
        .any(|id| id.is_empty() || id.len() > MAX_CREDENTIAL_ID_BYTES)
    {
        return Err(PasskeyError::InvalidInput(
            "a credential id in the list has an unacceptable length",
        ));
    }
    Ok(())
}

fn validate_create(request: &CreateRequest) -> Result<(), PasskeyError> {
    validate_rp_id(&request.rp.id)?;
    if request.user.id.is_empty() || request.user.id.len() > MAX_USER_HANDLE_BYTES {
        return Err(PasskeyError::InvalidInput(
            "the user id must be 1 to 64 bytes",
        ));
    }
    if request.user.name.len() > MAX_NAME_BYTES
        || request.user.display_name.len() > MAX_NAME_BYTES
        || request
            .rp
            .name
            .as_ref()
            .is_some_and(|name| name.len() > MAX_NAME_BYTES)
    {
        return Err(PasskeyError::InvalidInput("a name is too long"));
    }
    if request.algorithms.is_empty() {
        return Err(PasskeyError::InvalidInput(
            "no signature algorithm requested",
        ));
    }
    validate_credential_list(&request.exclude)
}

fn validate_assert(request: &AssertRequest) -> Result<(), PasskeyError> {
    validate_rp_id(&request.rp_id)?;
    validate_credential_list(&request.allow)
}

/// The authenticator data starts with the SHA-256 of the relying party id and is at least the 37
/// bytes the fixed part takes.
fn authenticator_data_is_for(authenticator_data: &[u8], rp_id: &str) -> bool {
    authenticator_data.len() >= 37
        && authenticator_data[..32] == RustCryptoSha2::sha256(rp_id.as_bytes())
}

fn check_create_response(
    request: &CreateRequest,
    response: &CreateResponse,
) -> Result<(), PasskeyError> {
    if !authenticator_data_is_for(&response.authenticator_data, &request.rp.id) {
        return Err(PasskeyError::DeviceFailure(
            "the security key answered for a different site",
        ));
    }
    if response.credential_id.is_empty()
        || response.credential_id.len() > MAX_CREDENTIAL_ID_BYTES
        || response.attestation_object.is_empty()
    {
        return Err(PasskeyError::DeviceFailure(
            "the security key returned a malformed credential",
        ));
    }
    Ok(())
}

fn check_assert_response(
    request: &AssertRequest,
    response: &AssertResponse,
) -> Result<(), PasskeyError> {
    if !authenticator_data_is_for(&response.authenticator_data, &request.rp_id) {
        return Err(PasskeyError::DeviceFailure(
            "the security key answered for a different site",
        ));
    }
    if response.credential_id.is_empty()
        || response.credential_id.len() > MAX_CREDENTIAL_ID_BYTES
        || response.signature.is_empty()
        || response
            .user_handle
            .as_ref()
            .is_some_and(|handle| handle.len() > MAX_USER_HANDLE_BYTES)
    {
        return Err(PasskeyError::DeviceFailure(
            "the security key returned a malformed assertion",
        ));
    }
    if !request.allow.is_empty() && !request.allow.contains(&response.credential_id) {
        return Err(PasskeyError::DeviceFailure(
            "the security key signed with a credential the page did not allow",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_defaults_and_caps() {
        assert_eq!(effective_timeout_ms(None), 120_000);
        assert_eq!(effective_timeout_ms(Some(0)), 120_000);
        assert_eq!(effective_timeout_ms(Some(45_000)), 45_000);
        assert_eq!(effective_timeout_ms(Some(300_000)), 300_000);
        assert_eq!(effective_timeout_ms(Some(300_001)), 300_000);
        assert_eq!(effective_timeout_ms(Some(u32::MAX)), 300_000);
    }

    #[test]
    fn authenticator_data_check_needs_the_right_hash_and_length() {
        let mut data = RustCryptoSha2::sha256(b"example.com").to_vec();
        assert!(
            !authenticator_data_is_for(&data, "example.com"),
            "too short"
        );
        data.extend_from_slice(&[0x01, 0, 0, 0, 0]);
        assert!(authenticator_data_is_for(&data, "example.com"));
        assert!(!authenticator_data_is_for(&data, "example.org"));
        assert!(!authenticator_data_is_for(&[], "example.com"));
    }
}
