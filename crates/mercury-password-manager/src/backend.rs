// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::cell::Cell;

#[cfg(target_os = "macos")]
use mercury_passwords::KeychainBackend;
use mercury_passwords::{
    LoginKey, LoginSummary, MemoryBackend, Password, PasswordError, SaveOutcome, SecretBackend,
};

/// The backend the browser chose for this run: the Keychain for the user's own profile, memory
/// for other profiles, automated runs and tests.
///
/// While [`AnyBackend::set_quiet`] is on, Keychain calls run with user interaction switched off
/// (see `KeychainBackend::without_user_interaction`): one that would show a dialog fails with
/// `PasswordError::Locked`. Only the store worker sets it, on its own thread.
pub struct AnyBackend {
    kind: Kind,
    quiet: Cell<bool>,
    owner: Cell<Option<std::thread::ThreadId>>,
}

enum Kind {
    Memory(MemoryBackend),
    #[cfg(target_os = "macos")]
    Keychain(KeychainBackend),
}

impl AnyBackend {
    /// Logins that last until the browser quits.
    pub fn memory() -> Self {
        Self {
            kind: Kind::Memory(MemoryBackend::new()),
            quiet: Cell::new(false),
            owner: Cell::new(None),
        }
    }

    /// The macOS Keychain.
    #[cfg(target_os = "macos")]
    pub fn keychain(backend: KeychainBackend) -> Self {
        Self {
            kind: Kind::Keychain(backend),
            quiet: Cell::new(false),
            owner: Cell::new(None),
        }
    }

    /// Whether logins outlive this run.
    pub fn is_persistent(&self) -> bool {
        match self.kind {
            Kind::Memory(_) => false,
            #[cfg(target_os = "macos")]
            Kind::Keychain(_) => true,
        }
    }

    /// Whether calls may not ask the user now.
    pub fn is_quiet(&self) -> bool {
        self.quiet.get()
    }

    /// Makes the calling thread the only one that may call the Keychain through this backend
    /// (the store worker's thread; a test's own thread). Debug builds assert it on every call.
    pub fn bind_to_current_thread(&self) {
        self.owner.set(Some(std::thread::current().id()));
    }

    /// Whether the calling thread owns this backend.
    pub fn on_owner_thread(&self) -> bool {
        self.owner.get() == Some(std::thread::current().id())
    }

    /// Whether calls may not ask the user from now on.
    pub fn set_quiet(&self, quiet: bool) {
        self.quiet.set(quiet);
    }

    #[cfg(target_os = "macos")]
    fn keychain_call<R>(
        &self,
        operation: impl FnOnce() -> Result<R, PasswordError>,
    ) -> Result<R, PasswordError> {
        debug_assert!(
            self.on_owner_thread(),
            "every Keychain call runs on the store worker's thread"
        );
        if self.quiet.get() {
            KeychainBackend::without_user_interaction(operation)?
        } else {
            operation()
        }
    }
}

impl SecretBackend for AnyBackend {
    fn put(&self, key: &LoginKey, password: &Password) -> Result<SaveOutcome, PasswordError> {
        match &self.kind {
            Kind::Memory(backend) => backend.put(key, password),
            #[cfg(target_os = "macos")]
            Kind::Keychain(backend) => self.keychain_call(|| backend.put(key, password)),
        }
    }

    fn get(&self, key: &LoginKey) -> Result<Option<Password>, PasswordError> {
        match &self.kind {
            Kind::Memory(backend) => backend.get(key),
            #[cfg(target_os = "macos")]
            Kind::Keychain(backend) => self.keychain_call(|| backend.get(key)),
        }
    }

    fn delete(&self, key: &LoginKey) -> Result<bool, PasswordError> {
        match &self.kind {
            Kind::Memory(backend) => backend.delete(key),
            #[cfg(target_os = "macos")]
            Kind::Keychain(backend) => self.keychain_call(|| backend.delete(key)),
        }
    }

    fn list(&self) -> Result<Vec<LoginSummary>, PasswordError> {
        match &self.kind {
            Kind::Memory(backend) => backend.list(),
            #[cfg(target_os = "macos")]
            Kind::Keychain(backend) => self.keychain_call(|| backend.list()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backend_knows_the_thread_that_owns_it() {
        let backend = AnyBackend::memory();
        assert!(!backend.on_owner_thread(), "unbound");

        backend.bind_to_current_thread();
        assert!(backend.on_owner_thread());

        let elsewhere = std::thread::spawn(|| {
            let backend = AnyBackend::memory();
            backend.bind_to_current_thread();
            backend
        })
        .join()
        .expect("joined");
        assert!(!elsewhere.on_owner_thread());
    }
}
