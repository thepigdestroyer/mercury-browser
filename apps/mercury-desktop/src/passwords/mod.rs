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
//! prompt) and copy to an in-memory pasteboard. (A default profile whose data folder is missing
//! or unwritable still uses the keychain.)
//!
//! **No Keychain call runs on this thread.** The store belongs to a `StoreWorker` thread; a
//! job's reply comes back through `slint::invoke_from_event_loop`, and every reply checks again
//! that what it was for (the opened login, the focused field, the sign-in request) is still
//! there. Jobs that follow a gesture in Mercury's own interface may make macOS ask the user
//! (`Interaction::Allowed`); everything else (start-up, a page's sign-in that worked) runs with
//! Keychain user interaction off (`Interaction::Never`) and fails instead of asking. The list of
//! saved logins (no passwords) is kept here, so searching, the fill menu and the sign-in sheet
//! never call the Keychain; a password is read only after the user asked for it.

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
use std::time::SystemTime;

use mercury_password_manager::testing::FakePasteboard;
use mercury_password_manager::{
    AnyBackend, AuthGate, AuthOutcome, Authenticator, CLIPBOARD_CLEAR_DELAY, CopiedSecret,
    Interaction, Pasteboard, RefusingAuthenticator, SignInSaves, StoreWorker,
};
use mercury_passwords::LoginSummary;

pub(crate) use fill::{fill_chosen, fill_menu};
pub(crate) use section::{connect_settings_window, settings_window_closed};
#[cfg(test)]
pub(crate) use sign_in::REMEMBER_SIGN_IN_CHOICE;
pub(crate) use sign_in::{
    read_saved_sign_in, save_sign_in_choice, saved_sign_in_username, sign_in_sent,
};

/// Everything the password manager keeps for the run, on the UI thread.
pub(crate) struct Passwords {
    worker: StoreWorker,
    /// Every saved login, without passwords, as last listed.
    logins: RefCell<Vec<LoginSummary>>,
    /// An automated run: the diagnostics fill may run.
    #[cfg_attr(not(feature = "passwords-diagnostics"), allow(dead_code))]
    automated: bool,
    authenticator: Rc<dyn Authenticator>,
    pasteboard: Rc<dyn Pasteboard>,
    gate: RefCell<AuthGate>,
    sign_ins: RefCell<SignInSaves>,
    copied: Cell<Option<CopiedSecret>>,
    clear_timer: slint::Timer,
    section: RefCell<section::SectionState>,
    fill: RefCell<Option<fill::FillOffer>>,
}

thread_local! {
    static PASSWORDS: RefCell<Option<Rc<Passwords>>> = const { RefCell::new(None) };
}

impl Passwords {
    fn new(
        worker: StoreWorker,
        automated: bool,
        authenticator: Rc<dyn Authenticator>,
        pasteboard: Rc<dyn Pasteboard>,
    ) -> Rc<Self> {
        Rc::new(Self {
            worker,
            logins: RefCell::default(),
            automated,
            authenticator,
            pasteboard,
            gate: RefCell::default(),
            sign_ins: RefCell::default(),
            copied: Cell::new(None),
            clear_timer: slint::Timer::default(),
            section: RefCell::default(),
            fill: RefCell::default(),
        })
    }

    /// Lists the saved logins again on the worker, then runs `then`.
    fn reload_logins(
        self: &Rc<Self>,
        interaction: Interaction,
        then: impl FnOnce(&Rc<Self>) + 'static,
    ) {
        let weak = Rc::downgrade(self);
        self.worker.run(
            interaction,
            |store| store.all(),
            move |result| {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                match result {
                    Ok(logins) => *this.logins.borrow_mut() = logins,
                    Err(error) => {
                        log::warn!("passwords: the saved logins could not be listed: {error}")
                    }
                }
                then(&this);
            },
        );
    }

    /// Runs `then` with `true` once the user has authenticated (now, or within the grace
    /// period), or with `false` after showing why not.
    fn after_authentication(self: &Rc<Self>, reason: &str, then: impl FnOnce(bool) + 'static) {
        if self.gate.borrow().is_open(SystemTime::now()) {
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
                this.gate.borrow_mut().record(outcome, SystemTime::now());
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

    /// Copies a secret as a concealed, this-Mac-only item and removes it from the pasteboard
    /// after `CLIPBOARD_CLEAR_DELAY` or at quit, unless something else was copied since.
    fn copy_secret(self: &Rc<Self>, text: &str) -> bool {
        let Some(copied) = CopiedSecret::copy(self.pasteboard.as_ref(), text) else {
            return false;
        };
        self.copied.set(Some(copied));
        let this = Rc::downgrade(self);
        self.clear_timer.start(
            slint::TimerMode::SingleShot,
            CLIPBOARD_CLEAR_DELAY,
            move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                if let Some(this) = this.upgrade() {
                    this.clear_copied_secret();
                }
            },
        );
        true
    }

    fn clear_copied_secret(&self) {
        if let Some(copied) = self.copied.take() {
            copied.clear_if_unchanged(self.pasteboard.as_ref());
        }
    }
}

/// Chooses where logins live for this run (see the module comment) and starts the manager.
/// `default_profile` is false for `--profile-dir` and for temporary profiles.
pub(crate) fn start(automated: bool, default_profile: bool) {
    #[cfg(target_os = "macos")]
    if !automated && default_profile {
        let deliver: mercury_password_manager::Deliver = std::sync::Arc::new(|id, result| {
            let _queued = slint::invoke_from_event_loop(move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                if let Some(passwords) = current() {
                    passwords.worker.finish(id, result);
                }
            });
        });
        match StoreWorker::spawn(
            AnyBackend::keychain(mercury_passwords::KeychainBackend::login()),
            deliver,
        ) {
            Ok(worker) => {
                install(Passwords::new(
                    worker,
                    automated,
                    Rc::new(local_auth::LocalAuthentication),
                    Rc::new(system_pasteboard::SystemPasteboard),
                ));
                return;
            }
            Err(error) => {
                log::error!("passwords: no store thread ({error}); logins are kept in memory");
            }
        }
    }
    let _ = default_profile;
    install(Passwords::new(
        StoreWorker::inline(AnyBackend::memory()),
        automated,
        Rc::new(RefusingAuthenticator),
        Rc::new(FakePasteboard::default()),
    ));
    #[cfg(feature = "passwords-diagnostics")]
    if automated {
        fill::diagnostics::seed_from_environment();
    }
}

fn install(passwords: Rc<Passwords>) {
    PASSWORDS.with(|current| *current.borrow_mut() = Some(Rc::clone(&passwords)));
    // Page-independent, but not a gesture: never asks (a locked keychain lists nothing until
    // the settings window opens).
    passwords.reload_logins(Interaction::Never, |_| {});
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
        fill::diagnostics::observe_event(
            &passwords,
            controller,
            tab_id,
            event,
            std::env::var(fill::diagnostics::VARIABLE).ok().as_deref(),
        );
    }
    let _ = controller;
}

/// At quit (`post_run`): a password copied less than `CLIPBOARD_CLEAR_DELAY` ago leaves the
/// pasteboard. Uses AppKit only, never Slint.
pub(crate) fn finish_run() {
    if let Some(passwords) = current() {
        passwords.clear_copied_secret();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use mercury_password_manager::testing::{FakeAuthenticator, FakePasteboard};
    use mercury_passwords::{LoginKey, Password};

    use super::*;

    type Fakes = (Rc<Passwords>, Rc<FakeAuthenticator>, Rc<FakePasteboard>);

    fn install_with(worker: StoreWorker) -> Fakes {
        let authenticator = Rc::new(FakeAuthenticator::default());
        let pasteboard = Rc::new(FakePasteboard::default());
        let passwords = Passwords::new(worker, true, authenticator.clone(), pasteboard.clone());
        install(Rc::clone(&passwords));
        passwords.worker.run_queued();
        (passwords, authenticator, pasteboard)
    }

    /// A manager with fakes for this test thread whose store jobs run at once.
    pub(crate) fn install_fakes() -> Fakes {
        install_with(StoreWorker::inline(AnyBackend::memory()))
    }

    /// The same, but store jobs wait for `passwords.worker.run_queued()`, as on the worker
    /// thread.
    pub(crate) fn install_deferred() -> Fakes {
        install_with(StoreWorker::deferred(AnyBackend::memory()))
    }

    /// Saves a login and lists the logins again, at once.
    pub(crate) fn save_login(passwords: &Rc<Passwords>, key: LoginKey, password: Password) {
        passwords.worker.run(
            Interaction::Allowed,
            move |store| store.save(&key, &password).map(|_| ()),
            |result| result.expect("saved"),
        );
        passwords.reload_logins(Interaction::Allowed, |_| {});
        passwords.worker.run_queued();
    }

    #[test]
    fn automated_runs_keep_logins_in_memory_and_never_ask() {
        start(true, true);
        let passwords = current().expect("started");

        assert!(!passwords.worker.is_persistent());
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

        assert!(!current().expect("started").worker.is_persistent());
    }

    #[test]
    fn a_copied_secret_is_cleared_after_the_delay() {
        crate::settings::tests::init_slint();
        let (passwords, _, pasteboard) = install_fakes();

        assert!(passwords.copy_secret("fake-pw"));
        assert_eq!(pasteboard.contents(), Some(("fake-pw".to_owned(), true)));
        i_slint_backend_testing::mock_elapsed_time(
            CLIPBOARD_CLEAR_DELAY + std::time::Duration::from_millis(10),
        );
        slint::platform::update_timers_and_animations();

        assert_eq!(pasteboard.contents(), None);
    }
}
// passwords-ui (review): written before the fix.
#[cfg(test)]
mod quit_tests {
    use super::tests::install_fakes;
    use super::*;

    #[test]
    fn a_copied_password_is_cleared_at_quit() {
        crate::settings::tests::init_slint();
        let (passwords, _, pasteboard) = install_fakes();
        assert!(passwords.copy_secret("fake-pw"));

        finish_run();

        assert_eq!(pasteboard.contents(), None);
    }
}
