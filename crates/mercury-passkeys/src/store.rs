// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::cmp::Reverse;

use crate::model::{
    MAX_CREDENTIAL_ID_BYTES, MAX_NAME_BYTES, MAX_RP_ID_BYTES, MAX_USER_HANDLE_BYTES,
    MIN_CREDENTIAL_ID_BYTES,
};
use crate::verifier::{UserVerifier, VerificationPurpose, require_verified_user};
use crate::{PasskeyBackend, PasskeyError, PasskeyKey, PasskeyRecord, PasskeySummary, SaveOutcome};

/// The longest private key accepted, in bytes of COSE_Key encoding.
pub const MAX_PRIVATE_KEY_BYTES: usize = 4096;

/// Policy on top of a backend: validation, per-site listing and deletion, newest first.
///
/// This is what the settings page (list, delete) and the CXF import use; the built-in
/// authenticator reads and writes through it too, so the same rules apply everywhere.
pub struct PasskeyStore<B: PasskeyBackend> {
    backend: B,
}

impl<B: PasskeyBackend> PasskeyStore<B> {
    /// A store that keeps its passkeys in `backend`.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// The backend itself. Intended for tests and tools; application code goes through the store
    /// so its rules apply.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Gives the backend back. Intended for tests and tools.
    pub fn into_backend(self) -> B {
        self.backend
    }

    /// Saves a passkey after checking it (see [`validate_record`]).
    pub fn save(&self, record: &PasskeyRecord) -> Result<SaveOutcome, PasskeyError> {
        validate_record(record)?;
        self.backend.put(record)
    }

    /// Every stored passkey, sorted by relying party, then newest first. No private keys.
    pub fn all(&self) -> Result<Vec<PasskeySummary>, PasskeyError> {
        let mut all = self.backend.list()?;
        sort_summaries(&mut all);
        Ok(all)
    }

    /// The passkeys of one relying party (exact match, no subdomains), newest first. No private
    /// keys.
    pub fn for_rp(&self, rp_id: &str) -> Result<Vec<PasskeySummary>, PasskeyError> {
        self.matching(rp_id, &[])
    }

    /// The passkeys of `rp_id` that a request may use: if `allow` is not empty, only those whose
    /// credential id it lists; newest first. No private keys.
    pub fn find(
        &self,
        rp_id: &str,
        allow: &[Vec<u8>],
    ) -> Result<Vec<PasskeySummary>, PasskeyError> {
        self.matching(rp_id, allow)
    }

    /// One passkey with its private key, or `None`. Any key is accepted, so every passkey
    /// [`PasskeyStore::all`] lists can be read, including one `save` would refuse (written by
    /// another tool or an older version).
    pub fn record(&self, key: &PasskeyKey) -> Result<Option<PasskeyRecord>, PasskeyError> {
        self.backend.get(key)
    }

    /// Sets the signature counter of a stored passkey; `false` if there is none.
    pub fn set_counter(&self, key: &PasskeyKey, counter: u32) -> Result<bool, PasskeyError> {
        self.backend.set_counter(key, counter)
    }

    /// Removes one passkey; `false` if it was not stored. Any key is accepted. The user must be
    /// verified by `verifier` first: if not, [`PasskeyError::NotAllowed`] is returned and nothing
    /// is removed.
    pub fn remove(
        &self,
        key: &PasskeyKey,
        verifier: &dyn UserVerifier,
    ) -> Result<bool, PasskeyError> {
        require_verified_user(
            verifier,
            VerificationPurpose::DeletePasskeys { count: Some(1) },
        )?;
        self.backend.delete(key)
    }

    /// Removes every passkey of one relying party and returns how many were removed. A passkey that
    /// cannot be removed does not stop the others; the result is then
    /// [`PasskeyError::RemoveIncomplete`]. The user must be verified by `verifier` first, as for
    /// [`PasskeyStore::remove`].
    pub fn remove_for_rp(
        &self,
        rp_id: &str,
        verifier: &dyn UserVerifier,
    ) -> Result<u64, PasskeyError> {
        let listed = self.matching(rp_id, &[])?;
        require_verified_user(
            verifier,
            VerificationPurpose::DeletePasskeys {
                count: Some(listed.len()),
            },
        )?;
        self.remove_listed(listed)
    }

    /// Removes every passkey this application stored and returns how many were removed; same rules
    /// as [`PasskeyStore::remove_for_rp`].
    pub fn remove_all(&self, verifier: &dyn UserVerifier) -> Result<u64, PasskeyError> {
        let listed = self.backend.list()?;
        require_verified_user(
            verifier,
            VerificationPurpose::DeletePasskeys {
                count: Some(listed.len()),
            },
        )?;
        self.remove_listed(listed)
    }

    fn remove_listed(&self, listed: Vec<PasskeySummary>) -> Result<u64, PasskeyError> {
        let mut removed = 0;
        let mut first_error = None;
        for summary in listed {
            match self.backend.delete(&summary.key) {
                Ok(true) => removed += 1,
                Ok(false) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            None => Ok(removed),
            Some(error) => Err(PasskeyError::RemoveIncomplete {
                removed,
                first_error: Box::new(error),
            }),
        }
    }

    fn matching(
        &self,
        rp_id: &str,
        allow: &[Vec<u8>],
    ) -> Result<Vec<PasskeySummary>, PasskeyError> {
        let mut found: Vec<PasskeySummary> = self
            .backend
            .list()?
            .into_iter()
            .filter(|summary| summary.key.rp_id == rp_id)
            .filter(|summary| allow.is_empty() || allow.contains(&summary.key.credential_id))
            .collect();
        sort_summaries(&mut found);
        Ok(found)
    }
}

fn sort_summaries(summaries: &mut [PasskeySummary]) {
    summaries.sort_by(|a, b| {
        (&a.key.rp_id, Reverse(a.created_at_ms), &a.key.credential_id).cmp(&(
            &b.key.rp_id,
            Reverse(b.created_at_ms),
            &b.key.credential_id,
        ))
    });
}

/// The rules `save` applies. Refused with [`PasskeyError::InvalidInput`]:
/// - a relying party id that is empty, longer than [`MAX_RP_ID_BYTES`], not lower-case ASCII
///   letters, digits, `-` and `.`, or with an empty label (`a..b`, a leading or trailing dot);
/// - a credential id shorter than [`MIN_CREDENTIAL_ID_BYTES`] or longer than
///   [`MAX_CREDENTIAL_ID_BYTES`];
/// - a user handle that is empty or longer than [`MAX_USER_HANDLE_BYTES`];
/// - a user name or display name longer than [`MAX_NAME_BYTES`] or with a control character;
/// - an empty private key, or one longer than [`MAX_PRIVATE_KEY_BYTES`].
pub fn validate_record(record: &PasskeyRecord) -> Result<(), PasskeyError> {
    let summary = &record.summary;
    validate_rp_id(&summary.key.rp_id)?;
    let id_length = summary.key.credential_id.len();
    if !(MIN_CREDENTIAL_ID_BYTES..=MAX_CREDENTIAL_ID_BYTES).contains(&id_length) {
        return Err(PasskeyError::InvalidInput(
            "the credential id has an unacceptable length",
        ));
    }
    if let Some(handle) = &summary.user_handle
        && (handle.is_empty() || handle.len() > MAX_USER_HANDLE_BYTES)
    {
        return Err(PasskeyError::InvalidInput(
            "the user handle has an unacceptable length",
        ));
    }
    for name in [&summary.username, &summary.display_name] {
        if name.len() > MAX_NAME_BYTES {
            return Err(PasskeyError::InvalidInput("a user name is too long"));
        }
        if name.chars().any(char::is_control) {
            return Err(PasskeyError::InvalidInput(
                "a user name contains a control character",
            ));
        }
    }
    let key_length = record.private_key.expose().len();
    if key_length == 0 || key_length > MAX_PRIVATE_KEY_BYTES {
        return Err(PasskeyError::InvalidInput(
            "the private key has an unacceptable length",
        ));
    }
    Ok(())
}

/// The rule for relying party ids, also used by the engine-facing code.
pub fn validate_rp_id(rp_id: &str) -> Result<(), PasskeyError> {
    let well_formed = !rp_id.is_empty()
        && rp_id.len() <= MAX_RP_ID_BYTES
        && rp_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
        })
        && rp_id.split('.').all(|label| !label.is_empty());
    if well_formed {
        Ok(())
    } else {
        Err(PasskeyError::InvalidInput(
            "the relying party id is not a lower-case host name",
        ))
    }
}
