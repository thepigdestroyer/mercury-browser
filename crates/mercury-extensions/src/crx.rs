// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Chrome's CRX3 package format: the header and its signatures are verified before the archive can be
//! opened.
//!
//! [`verify_crx`] returns a [`VerifiedCrx`], the only way to read the files of a `.crx`: it owns the
//! bytes that were verified (so the file cannot change between the check and the use) and
//! [`VerifiedCrx::open`] reads the zip from the verified offset.
//!
//! What a CRX proves: the file carries a proof by the developer key whose hash is the extension id, and
//! every proof in the header signs the archive. A local file can only ever have that developer proof,
//! which anyone can produce for any archive: [`CrxPolicy::Developer`] returns `store_signed == false`
//! and the shell shows its warning. [`CrxPolicy::Store`] additionally requires a proof by a pinned
//! publisher key of a store (by default the Chrome Web Store's key). Mercury does not use the store
//! (decision D3); the pin is a policy for later.
//!
//! Differential updates (`CrOD`) are refused. `verified_contents` (Chrome's content verification) is kept
//! as bytes and not checked: a follow-up.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use crx3::{KeyHash, Policy, SplitError, VerifyError};

use crate::archive::{ArchiveError, Limits, ZipPackage};

/// What a CRX must prove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrxPolicy {
    /// The developer proof alone: the only proof a local file can have. The result is not
    /// `store_signed` (unless the file happens to carry a valid proof by the Chrome Web Store key).
    Developer,
    /// A valid proof by at least one of these publisher keys (`crx3::Policy::any_of`) as well as the
    /// developer proof.
    Store {
        /// SHA-256 hashes of the accepted publisher keys' SubjectPublicKeyInfo.
        key_hashes: Vec<KeyHash>,
    },
}

impl CrxPolicy {
    /// The Chrome Web Store's production publisher key only (`crx3::WEBSTORE_PUBLISHER_KEY_HASH`). The
    /// store's test key (`crx3::WEBSTORE_PUBLISHER_TEST_KEY_HASH`) is not accepted by default: a test
    /// key has no place in a production pin. Use [`CrxPolicy::store_with_test_key`] in tests.
    pub fn store() -> CrxPolicy {
        CrxPolicy::Store {
            key_hashes: vec![crx3::WEBSTORE_PUBLISHER_KEY_HASH],
        }
    }

    /// The production key and the store's test key.
    pub fn store_with_test_key() -> CrxPolicy {
        CrxPolicy::Store {
            key_hashes: vec![
                crx3::WEBSTORE_PUBLISHER_KEY_HASH,
                crx3::WEBSTORE_PUBLISHER_TEST_KEY_HASH,
            ],
        }
    }
}

/// Why a CRX was refused. No variant holds file contents or key bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CrxError {
    /// The file is larger than `Limits::max_package_bytes`.
    #[error("the package is {found} bytes, over the limit of {limit} bytes")]
    PackageTooLarge {
        /// The limit.
        limit: u64,
        /// The size found.
        found: u64,
    },
    /// The file does not start with `Cr24`.
    #[error("not a CRX file")]
    BadMagic,
    /// The CRX format version is not 3.
    #[error("unsupported CRX version {0}")]
    UnsupportedVersion(u32),
    /// The file ends inside the prefix or the header.
    #[error("truncated CRX file")]
    Truncated,
    /// The header is longer than `crx3::MAX_HEADER_LEN`.
    #[error("the CRX header is too large")]
    HeaderTooLarge,
    /// A differential update (`CrOD`): not supported.
    #[error("differential CRX files are not supported")]
    Differential,
    /// The header does not parse.
    #[error("the CRX header is invalid")]
    HeaderInvalid,
    /// A key in a proof cannot be used with its algorithm.
    #[error("a proof in the CRX header has an unusable key")]
    UnusableKey,
    /// There is no proof by the developer key whose hash is the extension id.
    #[error("the developer proof is missing")]
    DeveloperProofMissing,
    /// The policy asks for a store proof and the file has none.
    #[error("the file has no proof by the required publisher key")]
    StoreProofMissing,
    /// A proof does not match the archive (or the header was changed).
    #[error("a signature in the CRX header does not match the package")]
    SignatureInvalid,
    /// Reading failed.
    #[error("input/output error: {0:?}")]
    Io(std::io::ErrorKind),
    /// The manifest `key` is not valid base64.
    #[error("the manifest key is not valid base64")]
    ManifestKeyInvalid,
    /// The manifest `key` gives a different extension id than the package.
    #[error("the manifest key does not match the package id")]
    ManifestKeyMismatch,
}

/// A CRX3 file whose header proofs were verified. It owns the verified bytes.
#[derive(Clone)]
#[non_exhaustive]
pub struct VerifiedCrx {
    /// The extension id: 32 letters `a` to `p`, derived from the developer key.
    pub id: String,
    /// The developer key, DER SubjectPublicKeyInfo.
    pub public_key: Vec<u8>,
    /// Whether the file carries a valid proof by a pinned store publisher key. `false` for a developer
    /// proof alone: the shell shows the warning.
    pub store_signed: bool,
    /// A differential update. Always `false`: those are refused.
    pub diff: bool,
    /// SHA-256 of the whole file.
    pub sha256: [u8; 32],
    /// Where the zip archive starts in the file.
    pub archive_offset: u64,
    /// The compressed `verified_contents` of the header, kept as it is and not checked.
    pub verified_contents: Option<Vec<u8>>,
    bytes: Arc<[u8]>,
}

impl std::fmt::Debug for VerifiedCrx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedCrx")
            .field("id", &self.id)
            .field("store_signed", &self.store_signed)
            .field("len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl VerifiedCrx {
    /// Opens the archive of the verified file and validates its directory against `limits`.
    pub fn open(&self, limits: &Limits) -> Result<ZipPackage, ArchiveError> {
        ZipPackage::open_at(Arc::clone(&self.bytes), self.archive_offset, limits)
    }

    /// The verified bytes of the whole file.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Verifies a CRX3 file under the default [`Limits`]. See [`verify_crx_shared`].
pub fn verify_crx(bytes: &[u8], policy: &CrxPolicy) -> Result<VerifiedCrx, CrxError> {
    verify_crx_shared(Arc::from(bytes), policy, &Limits::default())
}

/// Verifies a CRX3 file held in shared bytes: the prefix and header (size, version, magic), then every
/// proof, with `crx3::verify`, which follows Chromium's `crx_file::Verify`.
pub fn verify_crx_shared(
    bytes: Arc<[u8]>,
    policy: &CrxPolicy,
    limits: &Limits,
) -> Result<VerifiedCrx, CrxError> {
    let found = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if found > limits.max_package_bytes {
        return Err(CrxError::PackageTooLarge {
            limit: limits.max_package_bytes,
            found,
        });
    }
    let parts = crx3::split(&bytes).map_err(|e| match e {
        SplitError::BadMagic => CrxError::BadMagic,
        SplitError::UnsupportedVersion(v) => CrxError::UnsupportedVersion(v),
        SplitError::Truncated => CrxError::Truncated,
        SplitError::HeaderTooLarge => CrxError::HeaderTooLarge,
        _ => CrxError::HeaderInvalid,
    })?;
    if parts.diff {
        return Err(CrxError::Differential);
    }
    let developer = crx3::verify(&bytes, &Policy::new()).map_err(|e| map_verify_error(e, false))?;
    let store_signed = match policy {
        CrxPolicy::Developer => {
            let pinned = [crx3::WEBSTORE_PUBLISHER_KEY_HASH];
            crx3::verify(&bytes, &Policy::new().any_of(&pinned)).is_ok()
        }
        CrxPolicy::Store { key_hashes } => {
            if key_hashes.is_empty() {
                return Err(CrxError::StoreProofMissing);
            }
            crx3::verify(&bytes, &Policy::new().any_of(key_hashes))
                .map_err(|e| map_verify_error(e, true))?;
            true
        }
    };
    Ok(VerifiedCrx {
        id: developer.id,
        public_key: developer.public_key,
        store_signed,
        diff: false,
        sha256: developer.file_sha256,
        archive_offset: developer.archive_offset,
        verified_contents: developer.verified_contents,
        bytes,
    })
}

fn map_verify_error(error: VerifyError, store: bool) -> CrxError {
    match error {
        VerifyError::HeaderInvalid => CrxError::HeaderInvalid,
        VerifyError::SignatureInitializationFailed => CrxError::UnusableKey,
        VerifyError::RequiredProofMissing if store => CrxError::StoreProofMissing,
        VerifyError::RequiredProofMissing => CrxError::DeveloperProofMissing,
        VerifyError::SignatureVerificationFailed => CrxError::SignatureInvalid,
        VerifyError::Io(kind) => CrxError::Io(kind),
        _ => CrxError::HeaderInvalid,
    }
}

/// Checks that the manifest's `key` (base64 SubjectPublicKeyInfo, as Chrome writes it) gives the id of
/// the package. A manifest without a `key` passes. The manifest is parsed elsewhere: pass its `key`
/// string.
pub fn check_manifest_key(crx: &VerifiedCrx, manifest_key: Option<&str>) -> Result<(), CrxError> {
    let Some(key) = manifest_key else {
        return Ok(());
    };
    let compact: String = key.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let der = STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| CrxError::ManifestKeyInvalid)?;
    if der.is_empty() {
        return Err(CrxError::ManifestKeyInvalid);
    }
    if crx3::id_from_public_key(&der) == crx.id {
        Ok(())
    } else {
        Err(CrxError::ManifestKeyMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_store_policy_pins_the_production_key_only() {
        match CrxPolicy::store() {
            CrxPolicy::Store { key_hashes } => {
                assert_eq!(key_hashes, vec![crx3::WEBSTORE_PUBLISHER_KEY_HASH]);
            }
            CrxPolicy::Developer => panic!("expected a store policy"),
        }
    }

    #[test]
    fn test_key_policy_has_both_keys() {
        match CrxPolicy::store_with_test_key() {
            CrxPolicy::Store { key_hashes } => assert_eq!(key_hashes.len(), 2),
            CrxPolicy::Developer => panic!("expected a store policy"),
        }
    }

    #[test]
    fn empty_input_is_truncated_not_a_panic() {
        assert_eq!(
            verify_crx(&[], &CrxPolicy::Developer).unwrap_err(),
            CrxError::Truncated
        );
    }
}
