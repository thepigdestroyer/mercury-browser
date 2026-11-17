// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#[cfg(target_os = "macos")]
use mercury_passwords::KeychainBackend;
use mercury_passwords::{
    LoginKey, LoginSummary, MemoryBackend, Password, PasswordError, SaveOutcome, SecretBackend,
};

/// The backend the browser chose for this run: the Keychain for the user's own profile, memory
/// for temporary profiles, automated runs and tests.
pub enum AnyBackend {
    /// Logins that last until the browser quits.
    Memory(MemoryBackend),
    /// The macOS Keychain.
    #[cfg(target_os = "macos")]
    Keychain(KeychainBackend),
}

impl AnyBackend {
    /// Whether logins outlive this run.
    pub fn is_persistent(&self) -> bool {
        match self {
            Self::Memory(_) => false,
            #[cfg(target_os = "macos")]
            Self::Keychain(_) => true,
        }
    }
}

impl SecretBackend for AnyBackend {
    fn put(&self, key: &LoginKey, password: &Password) -> Result<SaveOutcome, PasswordError> {
        match self {
            Self::Memory(backend) => backend.put(key, password),
            #[cfg(target_os = "macos")]
            Self::Keychain(backend) => backend.put(key, password),
        }
    }

    fn get(&self, key: &LoginKey) -> Result<Option<Password>, PasswordError> {
        match self {
            Self::Memory(backend) => backend.get(key),
            #[cfg(target_os = "macos")]
            Self::Keychain(backend) => backend.get(key),
        }
    }

    fn delete(&self, key: &LoginKey) -> Result<bool, PasswordError> {
        match self {
            Self::Memory(backend) => backend.delete(key),
            #[cfg(target_os = "macos")]
            Self::Keychain(backend) => backend.delete(key),
        }
    }

    fn list(&self) -> Result<Vec<LoginSummary>, PasswordError> {
        match self {
            Self::Memory(backend) => backend.list(),
            #[cfg(target_os = "macos")]
            Self::Keychain(backend) => backend.list(),
        }
    }
}
