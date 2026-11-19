// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! HTTP sign-in through the dialogs lane's sign-in sheet.
//!
//! - A page can raise the sheet without a gesture, so the sheet only starts with the **user
//!   name** of a login saved for the exact origin that asked (from the in-memory list: no
//!   Keychain call). The saved password is read only when the user presses Sign in with the
//!   password field empty and that user name.
//! - Never for a proxy: a proxy's challenge carries the destination's URL, and the destination's
//!   login must not go to the proxy. Nothing is filled or offered for saving there.
//! - With the save choice ticked, the credentials are saved once the sign-in has worked (see
//!   `SignInSaves`). The user asked for that save (ticked the choice and pressed Sign in), so the
//!   job may ask (`Interaction::Allowed`): the keychain may need unlocking, or an item saved by
//!   another build of Mercury may need the user's permission. If the save still fails, the
//!   browser window says so under its toolbar, with Retry and Dismiss.
//!
//! Servo 0.8 reports no realm, so these logins are saved and looked up with an empty one.

use std::rc::Rc;
use std::time::Instant;

use mercury_password_manager::{Interaction, PendingHttpSave, site_label};
use mercury_passwords::{LoginKey, LoginKind, Origin, Password, PasswordError};
use mercury_ui::MainWindow;
use slint::ComponentHandle;
use talaria_core::{BrowserEvent, LoadState, PageRequestKind, TabId};

use super::{Passwords, current};

/// The sheet's save choice when logins are kept in the keychain.
pub(crate) const SAVE_SIGN_IN_CHOICE: &str = "Save the user name and password in Mercury Browser";
/// The same in runs that keep logins in memory only (`--profile-dir`, automated runs).
pub(crate) const REMEMBER_SIGN_IN_CHOICE: &str =
    "Remember the user name and password until Mercury Browser quits";

/// The user name of the login saved for a sign-in to `origin` (serialised,
/// `https://example.com`): the most recently changed HTTP login of that origin. Never for a
/// proxy. Reads the in-memory list only.
pub(crate) fn saved_sign_in_username(origin: &str, for_proxy: bool) -> Option<String> {
    if for_proxy {
        return None;
    }
    let passwords = current()?;
    let origin = Origin::parse(origin).ok()?;
    let logins = passwords.logins.borrow();
    logins
        .iter()
        .filter(|summary| {
            summary.key.origin == origin
                && matches!(&summary.key.kind, LoginKind::HttpAuth { realm } if realm.is_empty())
        })
        .max_by_key(|summary| summary.modified_at_ms)
        .map(|summary| summary.key.username.clone())
}

/// The user pressed Sign in with the saved user name and no password: reads the saved password
/// on the store worker (this is a gesture: macOS may ask) and calls `then` with it, or with
/// `None` if it could not be read.
pub(crate) fn read_saved_sign_in(
    origin: &str,
    username: &str,
    then: impl FnOnce(Option<String>) + 'static,
) {
    let (Some(passwords), Ok(origin)) = (current(), Origin::parse(origin)) else {
        then(None);
        return;
    };
    let key = mercury_passwords::LoginKey::http_auth(origin, "", username);
    passwords.worker.run(
        Interaction::Allowed,
        move |store| store.password(&key),
        move |result: Result<Option<Password>, PasswordError>| match result {
            Ok(Some(password)) => then(Some(password.expose().to_owned())),
            Ok(None) => then(None),
            Err(error) => {
                log::warn!("passwords: a sign-in password could not be read: {error}");
                then(None);
            }
        },
    );
}

/// The save choice for a sheet signing in to `origin`, or `None` when nothing may be offered:
/// for a proxy, an origin that cannot hold logins, or as the settings say.
pub(crate) fn save_sign_in_choice(origin: Option<&str>, for_proxy: bool) -> Option<&'static str> {
    let origin = origin.filter(|_| !for_proxy)?;
    let passwords = current()?;
    if Origin::parse(origin).is_err() || !crate::settings::password_saving_allowed(origin) {
        return None;
    }
    Some(if passwords.worker.is_persistent() {
        SAVE_SIGN_IN_CHOICE
    } else {
        REMEMBER_SIGN_IN_CHOICE
    })
}

/// The user signed in from the sheet with the save choice ticked. The caller checks that the
/// choice was offered (`save_sign_in_choice`).
pub(crate) fn sign_in_sent(tab_id: TabId, origin: &str, username: &str, password: &str) {
    if let Some(passwords) = current() {
        passwords.sign_ins.borrow_mut().sent(
            tab_id.value(),
            origin,
            username,
            Password::new(password.to_owned()),
            Instant::now(),
        );
    }
}

pub(super) fn observe_event(passwords: &Rc<Passwords>, tab_id: TabId, event: &BrowserEvent) {
    let tab = tab_id.value();
    let mut sign_ins = passwords.sign_ins.borrow_mut();
    let saved = match event {
        BrowserEvent::PageRequested(request)
            if matches!(request.kind, PageRequestKind::Authentication { .. }) =>
        {
            sign_ins.sign_in_requested(tab, &request.url);
            None
        }
        BrowserEvent::PageLoadStarted => {
            sign_ins.document_started(tab);
            None
        }
        BrowserEvent::LoadStateChanged {
            state: LoadState::Loaded,
        } => sign_ins.load_finished(tab, Instant::now()),
        BrowserEvent::LoadStateChanged {
            state: LoadState::Failed { .. },
        }
        | BrowserEvent::CloseRequested => {
            sign_ins.forget_tab(tab);
            None
        }
        _ => None,
    };
    drop(sign_ins);
    let Some(saved) = saved else {
        return;
    };
    // The settings may have changed while the sign-in was under way.
    if !crate::settings::password_saving_allowed(saved.key.origin.as_str()) {
        return;
    }
    save(passwords, saved);
}

/// A sign-in the user asked to save that could not be saved: kept until Retry or Dismiss.
pub(crate) struct FailedSave {
    key: LoginKey,
    password: Password,
}

/// Saves a sign-in the user asked to save. That request was the user's gesture, so macOS may ask
/// (unlock the keychain, allow access to an item another build saved). On failure the browser
/// window says so, with Retry and Dismiss.
fn save(passwords: &Rc<Passwords>, saved: PendingHttpSave) {
    let kept = FailedSave {
        key: saved.key.clone(),
        password: saved.password.clone(),
    };
    let weak = Rc::downgrade(passwords);
    passwords.worker.run(
        Interaction::Allowed,
        move |store| {
            store.save(&saved.key, &saved.password).map(|outcome| {
                format!(
                    "saved an HTTP sign-in for {} ({outcome:?})",
                    saved.key.origin
                )
            })
        },
        move |result: Result<String, PasswordError>| {
            let Some(passwords) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(done) => {
                    log::info!("passwords: {done}");
                    passwords.reload_logins(Interaction::Never, |_| {});
                }
                Err(error) => {
                    log::warn!("passwords: an HTTP sign-in could not be saved: {error}");
                    *passwords.failed_save.borrow_mut() = Some(kept);
                    show_save_problem(&passwords);
                }
            }
        },
    );
}

/// What the browser window says about a sign-in that could not be saved, if any.
pub(crate) fn save_problem() -> Option<String> {
    let passwords = current()?;
    let failed = passwords.failed_save.borrow();
    failed.as_ref().map(|failed| {
        format!(
            "Couldn't save the login for {}.",
            site_label(&failed.key.origin)
        )
    })
}

/// Retry: saves the failed sign-in again (the user's gesture: macOS may ask).
pub(crate) fn retry_failed_save() {
    let Some(passwords) = current() else {
        return;
    };
    let failed = passwords.failed_save.borrow_mut().take();
    show_save_problem(&passwords);
    if let Some(FailedSave { key, password }) = failed {
        save(&passwords, PendingHttpSave { key, password });
    }
}

/// Dismiss: the failed sign-in is forgotten, unsaved.
pub(crate) fn dismiss_save_problem() {
    if let Some(passwords) = current() {
        passwords.failed_save.borrow_mut().take();
        show_save_problem(&passwords);
    }
}

fn show_save_problem(passwords: &Passwords) {
    if let Some(window) = passwords.main_window.borrow().upgrade() {
        window.set_password_save_problem(save_problem().unwrap_or_default().into());
    }
}

/// Connects the browser window's notice about a sign-in that could not be saved.
pub(crate) fn connect_main_window(window: &MainWindow) {
    let Some(passwords) = current() else {
        return;
    };
    *passwords.main_window.borrow_mut() = window.as_weak();
    window.on_retry_password_save(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        retry_failed_save();
    });
    window.on_dismiss_password_save_problem(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        dismiss_save_problem();
    });
    show_save_problem(&passwords);
}

/// For the sheet's Sign in with an empty password: runs `then` once the list of saved logins
/// can be trusted. If listing failed earlier (a locked keychain at launch), it is listed again
/// first; Sign in is the user's gesture, so macOS may ask.
pub(crate) fn with_saved_sign_ins_listed(then: impl FnOnce() + 'static) {
    match current() {
        Some(passwords) => passwords.with_list_read(|_| then()),
        None => then(),
    }
}

/// Whether listing the saved logins failed (a locked keychain at launch): a sign-in sheet may
/// then have a saved login that it cannot name.
pub(crate) fn saved_sign_ins_unknown() -> bool {
    current().is_some_and(|passwords| passwords.list_unavailable.get())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mercury_passwords::LoginKey;
    use talaria_core::{PageRequest, PageRequestId};

    use super::*;
    use crate::passwords::tests::{install_fakes, save_login};

    const ORIGIN: &str = "http://127.0.0.1:8123";

    fn sign_in_request(id: u64) -> BrowserEvent {
        BrowserEvent::PageRequested(PageRequest {
            id: PageRequestId::new(id),
            url: format!("{ORIGIN}/private"),
            kind: PageRequestKind::Authentication { for_proxy: false },
            received_at: Duration::ZERO,
        })
    }

    fn loaded() -> BrowserEvent {
        BrowserEvent::LoadStateChanged {
            state: LoadState::Loaded,
        }
    }

    #[test]
    fn a_sign_in_that_worked_is_saved_and_named_on_the_next_sheet() {
        crate::settings::tests::init_slint();
        let _settings = crate::settings::SettingsHandle::start(None);
        let (passwords, _, _) = install_fakes();
        let tab = TabId::new(1);
        assert_eq!(saved_sign_in_username(ORIGIN, false), None);

        sign_in_sent(tab, ORIGIN, "alice", "fake-pw");
        observe_event(&passwords, tab, &BrowserEvent::PageLoadStarted);
        observe_event(&passwords, tab, &loaded());

        assert_eq!(
            saved_sign_in_username(ORIGIN, false).as_deref(),
            Some("alice")
        );
        let read = Rc::new(std::cell::RefCell::new(None));
        let seen = Rc::clone(&read);
        read_saved_sign_in(ORIGIN, "alice", move |password| {
            *seen.borrow_mut() = password
        });
        assert_eq!(read.borrow().as_deref(), Some("fake-pw"));
    }

    #[test]
    fn rejected_credentials_are_not_saved() {
        crate::settings::tests::init_slint();
        let _settings = crate::settings::SettingsHandle::start(None);
        let (passwords, _, _) = install_fakes();
        let tab = TabId::new(1);

        sign_in_sent(tab, ORIGIN, "alice", "fake-wrong");
        observe_event(&passwords, tab, &sign_in_request(2));
        observe_event(&passwords, tab, &BrowserEvent::PageLoadStarted);
        observe_event(&passwords, tab, &loaded());

        assert!(passwords.logins.borrow().is_empty());
    }

    #[test]
    fn never_save_and_the_offer_setting_are_honoured() {
        crate::settings::tests::init_slint();
        let settings = crate::settings::SettingsHandle::start(None);
        install_fakes();

        assert_eq!(
            save_sign_in_choice(Some(ORIGIN), false),
            Some(REMEMBER_SIGN_IN_CHOICE),
            "a memory-only run says the login lasts until quit"
        );
        assert_eq!(
            save_sign_in_choice(Some(ORIGIN), true),
            None,
            "not for a proxy"
        );
        assert_eq!(save_sign_in_choice(Some("http://example.com"), false), None);
        assert_eq!(save_sign_in_choice(None, false), None);

        settings.update(|settings| {
            settings
                .passwords
                .never_save_origins
                .push(ORIGIN.to_owned());
        });
        assert_eq!(save_sign_in_choice(Some(ORIGIN), false), None);

        settings.update(|settings| {
            settings.passwords.never_save_origins.clear();
            settings.passwords.offer_to_save = false;
        });
        assert_eq!(save_sign_in_choice(Some(ORIGIN), false), None);
    }

    #[test]
    fn the_newest_saved_login_names_the_sheet() {
        let (passwords, _, _) = install_fakes();
        save_login(
            &passwords,
            LoginKey::http_auth(Origin::parse(ORIGIN).expect("valid"), "", "bob"),
            Password::new("fake-b".to_owned()),
        );
        std::thread::sleep(Duration::from_millis(5));
        save_login(
            &passwords,
            LoginKey::http_auth(Origin::parse(ORIGIN).expect("valid"), "", "carol"),
            Password::new("fake-c".to_owned()),
        );

        assert_eq!(
            saved_sign_in_username(ORIGIN, false).as_deref(),
            Some("carol")
        );
    }
}

// passwords-ui (review): written before the fix.
#[cfg(test)]
mod worker_tests {
    use std::cell::RefCell;

    use mercury_password_manager::Interaction;
    use mercury_passwords::LoginKey;

    use super::*;
    use crate::passwords::tests::{install_deferred, save_login};

    const ORIGIN: &str = "https://example.com";

    #[test]
    fn the_saved_password_is_read_only_when_the_user_signs_in() {
        let (passwords, _, _) = install_deferred();
        save_login(
            &passwords,
            LoginKey::http_auth(Origin::parse(ORIGIN).expect("valid"), "", "alice"),
            Password::new("fake-pw".to_owned()),
        );
        assert_eq!(
            saved_sign_in_username(ORIGIN, false).as_deref(),
            Some("alice")
        );
        assert_eq!(
            saved_sign_in_username(ORIGIN, true),
            None,
            "never for a proxy"
        );
        let read = Rc::new(RefCell::new(None));
        let seen = Rc::clone(&read);

        read_saved_sign_in(ORIGIN, "alice", move |password| {
            *seen.borrow_mut() = Some(password);
        });
        assert!(read.borrow().is_none());
        assert_eq!(
            passwords.worker.last_interaction(),
            Some(Interaction::Allowed)
        );
        passwords.worker.run_queued();

        assert_eq!(read.borrow().clone().flatten().as_deref(), Some("fake-pw"));
    }

    #[test]
    fn a_sign_in_that_worked_is_saved_and_may_ask_because_the_user_asked() {
        crate::settings::tests::init_slint();
        let _settings = crate::settings::SettingsHandle::start(None);
        let (passwords, _, _) = install_deferred();
        let tab = TabId::new(1);

        sign_in_sent(tab, ORIGIN, "bob", "fake-b");
        observe_event(&passwords, tab, &BrowserEvent::PageLoadStarted);
        observe_event(
            &passwords,
            tab,
            &BrowserEvent::LoadStateChanged {
                state: LoadState::Loaded,
            },
        );

        assert_eq!(
            passwords.worker.last_interaction(),
            Some(Interaction::Allowed)
        );
        passwords.worker.run_queued();
        assert_eq!(
            saved_sign_in_username(ORIGIN, false).as_deref(),
            Some("bob")
        );
        assert_eq!(save_problem(), None);
    }
}

// passwords-ui (review round 2): written before the fix.
#[cfg(test)]
mod save_problem_tests {
    use mercury_password_manager::Interaction;

    use super::*;
    use crate::passwords::tests::install_fakes;

    #[test]
    fn a_save_that_fails_says_so_and_can_be_retried_or_dismissed() {
        crate::settings::tests::init_slint();
        let _settings = crate::settings::SettingsHandle::start(None);
        let (passwords, _, _) = install_fakes();
        let tab = TabId::new(1);

        // A control character in the user name: the store refuses it.
        sign_in_sent(tab, "https://example.com", "bob\u{1}", "fake-b");
        observe_event(&passwords, tab, &BrowserEvent::PageLoadStarted);
        observe_event(
            &passwords,
            tab,
            &BrowserEvent::LoadStateChanged {
                state: LoadState::Loaded,
            },
        );
        assert_eq!(
            save_problem().as_deref(),
            Some("Couldn't save the login for example.com.")
        );

        retry_failed_save();
        assert_eq!(
            passwords.worker.last_interaction(),
            Some(Interaction::Allowed)
        );
        assert!(save_problem().is_some(), "it failed again");

        dismiss_save_problem();
        assert_eq!(save_problem(), None);
    }
}
