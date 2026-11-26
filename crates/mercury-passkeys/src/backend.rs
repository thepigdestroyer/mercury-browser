// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{AddOutcome, PasskeyError, PasskeyKey, PasskeyRecord, PasskeySummary, SaveOutcome};

/// Where passkeys live. Implementations never return a private key from `list`, and never write
/// one anywhere in the clear.
pub trait PasskeyBackend: Send + Sync {
    /// Saves `record`, creating the passkey or replacing the one with the same key. A backend that
    /// keeps creation dates sets `created_at_ms` itself and keeps it on replacement.
    fn put(&self, record: &PasskeyRecord) -> Result<SaveOutcome, PasskeyError>;
    /// Adds `record` only if no passkey has its key. An existing passkey is never changed, and
    /// the check and the write are one step.
    fn add(&self, record: &PasskeyRecord) -> Result<AddOutcome, PasskeyError>;
    /// The passkey with this key, with its private key, or `None`. On macOS this is the call that
    /// can make the system ask the user.
    fn get(&self, key: &PasskeyKey) -> Result<Option<PasskeyRecord>, PasskeyError>;
    /// Changes the signature counter of an existing passkey without reading its private key;
    /// `false` if there is no such passkey.
    fn set_counter(&self, key: &PasskeyKey, counter: u32) -> Result<bool, PasskeyError>;
    /// Deletes the passkey; `false` if there was none.
    fn delete(&self, key: &PasskeyKey) -> Result<bool, PasskeyError>;
    /// Every passkey this application stored, without private keys, in no particular order.
    fn list(&self) -> Result<Vec<PasskeySummary>, PasskeyError>;
    /// The keys of every passkey this application stored, including any whose metadata cannot be
    /// read. Items of other applications are never included.
    fn stored_keys(&self) -> Result<Vec<PasskeyKey>, PasskeyError> {
        Ok(self
            .list()?
            .into_iter()
            .map(|summary| summary.key)
            .collect())
    }
}

/// Passkeys kept in this process's memory only. For tests, and for platforms without a backend.
///
/// Private keys are wiped when they are replaced, deleted, or when the backend is dropped.
#[derive(Default)]
pub struct MemoryBackend {
    entries: Mutex<HashMap<PasskeyKey, PasskeyRecord>>,
}

impl MemoryBackend {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<PasskeyKey, PasskeyRecord>> {
        // A panic while the lock was held cannot leave a map entry half-written.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

impl PasskeyBackend for MemoryBackend {
    fn put(&self, record: &PasskeyRecord) -> Result<SaveOutcome, PasskeyError> {
        let mut entries = self.entries();
        let now = now_ms();
        let mut stored = record.clone();
        stored.summary.modified_at_ms = Some(now);
        match entries.get(&record.summary.key) {
            Some(existing) => {
                stored.summary.created_at_ms = existing.summary.created_at_ms;
                entries.insert(record.summary.key.clone(), stored);
                Ok(SaveOutcome::Replaced)
            }
            None => {
                stored.summary.created_at_ms = record.summary.created_at_ms.or(Some(now));
                entries.insert(record.summary.key.clone(), stored);
                Ok(SaveOutcome::Created)
            }
        }
    }

    fn add(&self, record: &PasskeyRecord) -> Result<AddOutcome, PasskeyError> {
        let mut entries = self.entries();
        if entries.contains_key(&record.summary.key) {
            return Ok(AddOutcome::Exists);
        }
        let now = now_ms();
        let mut stored = record.clone();
        stored.summary.modified_at_ms = Some(now);
        stored.summary.created_at_ms = record.summary.created_at_ms.or(Some(now));
        entries.insert(record.summary.key.clone(), stored);
        Ok(AddOutcome::Added)
    }

    fn get(&self, key: &PasskeyKey) -> Result<Option<PasskeyRecord>, PasskeyError> {
        Ok(self.entries().get(key).cloned())
    }

    fn set_counter(&self, key: &PasskeyKey, counter: u32) -> Result<bool, PasskeyError> {
        Ok(match self.entries().get_mut(key) {
            Some(entry) => {
                entry.summary.counter = Some(counter);
                entry.summary.modified_at_ms = Some(now_ms());
                true
            }
            None => false,
        })
    }

    fn delete(&self, key: &PasskeyKey) -> Result<bool, PasskeyError> {
        Ok(self.entries().remove(key).is_some())
    }

    fn list(&self) -> Result<Vec<PasskeySummary>, PasskeyError> {
        Ok(self
            .entries()
            .values()
            .map(|record| record.summary.clone())
            .collect())
    }
}
