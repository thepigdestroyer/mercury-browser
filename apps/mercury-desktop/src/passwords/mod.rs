// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The password manager in the browser: the Passwords section of the settings window, filling a
//! saved login into a password field the user right-clicked, and filling and saving HTTP
//! sign-ins through the sign-in sheet. The logic is in `mercury-password-manager`; this module
//! connects it to the windows, the engine and the operating system.
//!
//! Where logins live: the login keychain only in a normal interactive run of the default
//! profile. Automated runs (`--screenshot`, `--background`), runs with `--profile-dir` and
//! tests keep them in memory for the run, authenticate with a refusing fake (never a system
//! prompt) and copy to an in-memory pasteboard.

mod fill;
mod section;
mod sign_in;

#[cfg(target_os = "macos")]
mod file_dialogs;
#[cfg(target_os = "macos")]
mod local_auth;
#[cfg(target_os = "macos")]
mod system_pasteboard;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use mercury_password_manager::testing::FakePasteboard;
use mercury_password_manager::{
    AnyBackend, AuthGate, AuthOutcome, Authenticator, CLIPBOARD_CLEAR_DELAY, CopiedSecret,
    Pasteboard, RefusingAuthenticator, SignInSaves,
};
use mercury_passwords::{MemoryBackend, PasswordStore};

pub(crate) use fill::{FillMenu, fill_chosen, fill_menu};
pub(crate) use section::{connect_settings_window, settings_window_closed};
pub(crate) use sign_in::{
    SAVE_SIGN_IN_CHOICE, offers_to_save_sign_in, saved_http_credentials, sign_in_sent,
};

/// Everything the password manager keeps for the run, on the UI thread.
pub(crate) struct Passwords {
    store: PasswordStore<AnyBackend>,
    authenticator: Rc<dyn Authenticator>,
    pasteboard: Rc<dyn Pasteboard>,
    gate: RefCell<AuthGate>,
    sign_ins: RefCell<SignInSaves>,
    copied: Cell<Option<CopiedSecret>>,
    clear_timer: slint::Timer,
    section: RefCell<section::SectionState>,
    fill: RefCell<Option<fill::FillOffer>>,
    http_fill: RefCell<Option<sign_in::CachedCredentials>>,
}

thread_local! {
    static PASSWORDS: RefCell<Option<Rc<Passwords>>> = const { RefCell::new(None) };
}

impl Passwords {
    fn new(
        backend: AnyBackend,
        authenticator: Rc<dyn Authenticator>,
        pasteboard: Rc<dyn Pasteboard>,
    ) -> Rc<Self> {
        Rc::new(Self {
            store: PasswordStore::new(backend),
            authenticator,
            pasteboard,
            gate: RefCell::default(),
            sign_ins: RefCell::default(),
            copied: Cell::new(None),
            clear_timer: slint::Timer::default(),
            section: RefCell::default(),
            fill: RefCell::default(),
            http_fill: RefCell::default(),
        })
    }

    /// Runs `then` with `true` once the user has authenticated (now, or within the grace
    /// period), or with `false` after showing why not.
    fn after_authentication(self: &Rc<Self>, reason: &str, then: impl FnOnce(bool) + 'static) {
        if self.gate.borrow().is_open(Instant::now()) {
            then(true);
            return;
        }
        let this = Rc::downgrade(self);
        self.authenticator.authenticate(
            reason,
            Box::new(move |outcome| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                this.gate.borrow_mut().record(outcome, Instant::now());
                if outcome != AuthOutcome::Allowed {
                    section::set_status(match outcome {
                        AuthOutcome::Denied => "Mercury Browser could not confirm that it is you.",
                        _ => {
                            "Passwords cannot be shown, copied or exported: this Mac cannot \
                             confirm that it is you."
                        }
                    });
                }
                then(outcome == AuthOutcome::Allowed);
            }),
        );
    }

    /// Copies a secret as a concealed item and removes it from the pasteboard after
    /// `CLIPBOARD_CLEAR_DELAY`, unless something else was copied since.
    fn copy_secret(self: &Rc<Self>, text: &str) -> bool {
        let Some(copied) = CopiedSecret::copy(self.pasteboard.as_ref(), text) else {
            return false;
        };
        self.copied.set(Some(copied));
        let this = Rc::downgrade(self);
        self.clear_timer
            .start(slint::TimerMode::SingleShot, CLIPBOARD_CLEAR_DELAY, move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                if let Some(this) = this.upgrade()
                    && let Some(copied) = this.copied.take()
                {
                    copied.clear_if_unchanged(this.pasteboard.as_ref());
                }
            });
        true
    }
}

/// Chooses where logins live for this run (see the module comment) and starts the manager.
/// `default_profile` is false for `--profile-dir` and for temporary profiles.
pub(crate) fn start(automated: bool, default_profile: bool) {
    #[cfg(target_os = "macos")]
    if !automated && default_profile {
        install(Passwords::new(
            AnyBackend::Keychain(mercury_passwords::KeychainBackend::login()),
            Rc::new(local_auth::LocalAuthentication),
            Rc::new(system_pasteboard::SystemPasteboard),
        ));
        return;
    }
    let _ = (automated, default_profile);
    install(in_memory(
        Rc::new(RefusingAuthenticator),
        Rc::new(FakePasteboard::default()),
    ));
    #[cfg(feature = "passwords-diagnostics")]
    if automated {
        fill::diagnostics::seed_from_environment();
    }
}

fn in_memory(authenticator: Rc<dyn Authenticator>, pasteboard: Rc<dyn Pasteboard>) -> Rc<Passwords> {
    Passwords::new(
        AnyBackend::Memory(MemoryBackend::new()),
        authenticator,
        pasteboard,
    )
}

fn install(passwords: Rc<Passwords>) {
    PASSWORDS.with(|current| *current.borrow_mut() = Some(passwords));
}

fn current() -> Option<Rc<Passwords>> {
    PASSWORDS.with(|current| current.borrow().clone())
}

/// Every event from the engine, before the controller applies it: HTTP sign-ins learn whether
/// they worked.
pub(crate) fn observe_event(
    controller: &mut talaria_core::BrowserController,
    tab_id: talaria_core::TabId,
    event: &talaria_core::BrowserEvent,
) {
    if let Some(passwords) = current() {
        sign_in::observe_event(&passwords, tab_id, event);
        #[cfg(feature = "passwords-diagnostics")]
        fill::diagnostics::observe_event(&passwords, controller, tab_id, event);
    }
    let _ = controller;
}

#[cfg(test)]
pub(crate) mod tests {
    use mercury_password_manager::testing::{FakeAuthenticator, FakePasteboard};

    use super::*;

    /// A manager with fakes for this test thread; the returned fakes are the ones it uses.
    pub(crate) fn install_fakes() -> (Rc<Passwords>, Rc<FakeAuthenticator>, Rc<FakePasteboard>) {
        let authenticator = Rc::new(FakeAuthenticator::default());
        let pasteboard = Rc::new(FakePasteboard::default());
        let passwords = in_memory(authenticator.clone(), pasteboard.clone());
        install(Rc::clone(&passwords));
        (passwords, authenticator, pasteboard)
    }

    #[test]
    fn automated_runs_keep_logins_in_memory_and_never_ask() {
        start(true, true);
        let passwords = current().expect("started");

        assert!(!passwords.store.backend().is_persistent());
        let asked = Rc::new(Cell::new(None));
        let seen = Rc::clone(&asked);
        passwords
            .authenticator
            .authenticate("test", Box::new(move |outcome| seen.set(Some(outcome))));
        assert_eq!(asked.get(), Some(AuthOutcome::Unavailable));
    }

    #[test]
    fn a_profile_dir_run_keeps_logins_in_memory_too() {
        start(false, false);

        assert!(
            !current()
                .expect("started")
                .store
                .backend()
                .is_persistent()
        );
    }

    #[test]
    fn a_copied_secret_is_cleared_after_the_delay() {
        crate::settings::tests::init_slint();
        let (passwords, _, pasteboard) = install_fakes();

        assert!(passwords.copy_secret("fake-pw"));
        assert_eq!(pasteboard.contents(), Some(("fake-pw".to_owned(), true)));
        i_slint_backend_testing::mock_elapsed_time(
            u64::try_from(CLIPBOARD_CLEAR_DELAY.as_millis()).expect("fits") + 10,
        );
        slint::platform::update_timers_and_animations();

        assert_eq!(pasteboard.contents(), None);
    }
}
