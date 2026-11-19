// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Filling a saved login into the password field the user right-clicked.
//!
//! The page context menu gets "Fill login…" only when Servo opened it in the same frame as the
//! focused password field (`BrowserState::password_field_menu`) and logins are saved for the
//! exact origin of the tab's top-level page (from the in-memory list: no Keychain call).
//! "Fill login…" turns the menu into a list of those logins; picking one (a click or Return in
//! the browser's own menu: the user gesture) reads its password on the store worker. When the
//! keychain answers, the browser checks again that the same field of the active tab still has
//! focus and the page still has that origin, and only then queues one
//! `BrowserCommand::FillLogin`. The script fills only if the top-level document has that origin
//! and the focused element is one of its own password inputs: never a field in another frame or
//! origin (focus inside any frame makes the frame element the active element).

use std::cell::RefCell;
use std::rc::Rc;

use mercury_password_manager::{Interaction, display_text, fill_candidates, site_label};
use mercury_passwords::{LoginKey, Origin, Password, PasswordError};
use talaria_core::{BrowserCommand, BrowserController, ControlId, LoginFill, TabId};

use super::current;

/// The logins a context menu offers, for the field it was opened over.
pub(crate) struct FillOffer {
    tab_id: TabId,
    field: ControlId,
    origin: Origin,
    logins: Vec<LoginKey>,
}

/// What the context menu shows for a fill: the site, then one row per user name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FillMenu {
    pub(crate) heading: String,
    pub(crate) usernames: Vec<String>,
}

/// For a context menu opened over the focused password `field` of the tab's top-level page at
/// `page_url`: the logins to offer, or `None` (nothing saved, or a page that cannot hold
/// logins). Any older offer is dropped first; this one is kept until the next menu.
pub(crate) fn fill_menu(tab_id: TabId, field: ControlId, page_url: &str) -> Option<FillMenu> {
    let passwords = current()?;
    *passwords.fill.borrow_mut() = None;
    let (origin, logins) = fill_candidates(&passwords.logins.borrow(), page_url);
    let origin = origin?;
    // The list could not be read (a locked keychain at launch): the user's right-click is a
    // gesture, so it is read again (macOS may ask); the next menu offers what it finds.
    if passwords.list_unavailable.get() {
        passwords.reload_logins(Interaction::Allowed, |_| {});
    }
    if logins.is_empty() {
        return None;
    }
    let menu = FillMenu {
        heading: format!("Fill a login for {}", site_label(&origin)),
        usernames: logins
            .iter()
            .map(|summary| {
                if summary.key.username.is_empty() {
                    "Password only (no user name)".to_owned()
                } else {
                    display_text(&summary.key.username)
                }
            })
            .collect(),
    };
    *passwords.fill.borrow_mut() = Some(FillOffer {
        tab_id,
        field,
        origin,
        logins: logins.into_iter().map(|summary| summary.key).collect(),
    });
    Some(menu)
}

/// Whether the offered field of the active tab still has focus and its page still has the
/// offered origin.
fn still_offered(
    controller: &BrowserController,
    tab_id: TabId,
    field: ControlId,
    origin: &Origin,
) -> bool {
    let state = controller.active_state();
    controller.active_tab_id() == tab_id
        && state.input_method_id() == Some(field)
        && Origin::parse(state.url()).ok().as_ref() == Some(origin)
}

/// The user picked login `index` of the last offer. Reads its password on the store worker
/// and fills it if, when the keychain answers, the offered field of the active tab still has
/// focus and the page still has the offered origin; then calls `queued` (to wake the engine).
pub(crate) fn fill_chosen(
    controller: &Rc<RefCell<BrowserController>>,
    index: usize,
    queued: impl FnOnce() + 'static,
) {
    let Some(passwords) = current() else {
        return;
    };
    let Some(offer) = passwords.fill.borrow_mut().take() else {
        return;
    };
    let Some(key) = offer.logins.get(index).cloned() else {
        return;
    };
    if !still_offered(
        &controller.borrow(),
        offer.tab_id,
        offer.field,
        &offer.origin,
    ) {
        log::info!("passwords: not filling: the field lost focus or the page changed");
        return;
    }
    let controller = Rc::clone(controller);
    let username = key.username.clone();
    passwords.worker.run(
        Interaction::Allowed,
        move |store| store.password(&key),
        move |result: Result<Option<Password>, PasswordError>| {
            let password = match result {
                Ok(Some(password)) => password,
                Ok(None) => {
                    log::info!("passwords: not filling: the login is no longer saved");
                    return;
                }
                Err(error) => {
                    log::warn!("passwords: not filling: the password could not be read: {error}");
                    return;
                }
            };
            if !still_offered(
                &controller.borrow(),
                offer.tab_id,
                offer.field,
                &offer.origin,
            ) {
                log::info!(
                    "passwords: not filling: the field lost focus while the keychain answered"
                );
                return;
            }
            controller.borrow_mut().queue_command_for(
                offer.tab_id,
                BrowserCommand::FillLogin(LoginFill::new(
                    offer.origin.as_str().to_owned(),
                    username,
                    password.expose().to_owned(),
                )),
            );
            queued();
        },
    );
}

/// `MERCURY_PASSWORDS_DIAG_FILL=<origin> <user name> <password>` (automated runs of a build with
/// the `passwords-diagnostics` feature only): saves that login in the run's memory store, and
/// fills the tab's focused field as if the user had picked the login each time a page of the
/// tab finishes loading. Lets `--screenshot` runs check the fill script without input.
#[cfg(feature = "passwords-diagnostics")]
pub(super) mod diagnostics {
    use talaria_core::{BrowserEvent, LoadState};

    use super::*;
    use crate::passwords::Passwords;

    pub(crate) const VARIABLE: &str = "MERCURY_PASSWORDS_DIAG_FILL";

    pub(crate) fn seed_from_environment() {
        let Some(passwords) = current() else {
            return;
        };
        let Ok(value) = std::env::var(VARIABLE) else {
            return;
        };
        let mut parts = value.splitn(3, ' ');
        let (Some(origin), Some(username), Some(password)) =
            (parts.next(), parts.next(), parts.next())
        else {
            eprintln!("mercury-passwords-diag: {VARIABLE} needs '<origin> <user> <password>'");
            return;
        };
        let Ok(origin) = Origin::parse(origin) else {
            eprintln!("mercury-passwords-diag: not an origin for logins");
            return;
        };
        let key = LoginKey::form(origin, username);
        let password = Password::new(password.to_owned());
        passwords.worker.run(
            Interaction::Never,
            move |store| store.save(&key, &password).is_ok(),
            |saved| eprintln!("mercury-passwords-diag: seeded one login: {saved}"),
        );
        passwords.reload_logins(Interaction::Never, |_| {});
    }

    /// `variable`: the value of `VARIABLE`, if set. Only automated runs act on it.
    pub(crate) fn observe_event(
        passwords: &Rc<Passwords>,
        controller: &mut BrowserController,
        tab_id: TabId,
        event: &BrowserEvent,
        variable: Option<&str>,
    ) {
        if !passwords.automated
            || variable.is_none()
            || *event
                != (BrowserEvent::LoadStateChanged {
                    state: LoadState::Loaded,
                })
        {
            return;
        }
        let Some(tab) = controller.tabs().find(|tab| tab.id() == tab_id) else {
            return;
        };
        let (origin, logins) = fill_candidates(&passwords.logins.borrow(), tab.state().url());
        let (Some(origin), Some(login)) = (origin, logins.first()) else {
            eprintln!("mercury-passwords-diag: no saved login for this page");
            return;
        };
        // Memory store only (automated run): the job runs at once.
        let key = login.key.clone();
        let read = Rc::new(RefCell::new(None));
        let seen = Rc::clone(&read);
        passwords.worker.run(
            Interaction::Never,
            move |store| store.password(&key).ok().flatten(),
            move |password| *seen.borrow_mut() = password,
        );
        let Some(password) = read.take() else {
            return;
        };
        eprintln!("mercury-passwords-diag: filling on {origin}");
        controller.queue_command_for(
            tab_id,
            BrowserCommand::FillLogin(LoginFill::new(
                origin.as_str().to_owned(),
                login.key.username.clone(),
                password.expose().to_owned(),
            )),
        );
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::passwords::tests::{install_fakes, save_login};

        #[test]
        fn the_diagnostics_fill_never_runs_outside_automated_runs() {
            let (passwords, _, _) = install_fakes();
            let alice = || LoginKey::form(Origin::parse("https://a.test").expect("valid"), "alice");
            save_login(&passwords, alice(), Password::new("fake".to_owned()));
            let interactive = Passwords::new(
                mercury_password_manager::StoreWorker::inline(
                    mercury_password_manager::AnyBackend::memory(),
                ),
                false,
                Rc::new(mercury_password_manager::RefusingAuthenticator),
                Rc::new(mercury_password_manager::testing::FakePasteboard::default()),
            );
            save_login(&interactive, alice(), Password::new("fake".to_owned()));
            let mut controller = BrowserController::default();
            let tab_id = controller.active_tab_id();
            controller.handle_event(
                tab_id,
                BrowserEvent::AddressChanged {
                    url: "https://a.test/".to_owned(),
                },
            );
            let loaded = BrowserEvent::LoadStateChanged {
                state: LoadState::Loaded,
            };

            observe_event(&interactive, &mut controller, tab_id, &loaded, Some("x"));
            assert!(controller.next_command().is_none());

            observe_event(&passwords, &mut controller, tab_id, &loaded, Some("x"));
            assert!(controller.next_command().is_some());
        }
    }
}

#[cfg(test)]
mod tests {
    use mercury_passwords::Password;
    use talaria_core::{BrowserEvent, ControlRect, ControlRequest, TabCommand};

    use super::*;
    use crate::passwords::tests::{install_fakes, save_login};

    const RECT: ControlRect = ControlRect {
        x: 0,
        y: 0,
        width: 10,
        height: 10,
    };

    fn controller_with_password_menu(url: &str) -> (Rc<RefCell<BrowserController>>, TabId) {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        for event in [
            BrowserEvent::AddressChanged {
                url: url.to_owned(),
            },
            BrowserEvent::InputMethodShown {
                id: ControlId::new(1),
                rect: RECT,
            },
            BrowserEvent::ControlShown {
                id: ControlId::new(2),
                request: ControlRequest::ContextMenu {
                    rect: RECT,
                    entries: Vec::new(),
                    link_url: None,
                    image_url: None,
                },
            },
            BrowserEvent::PasswordFieldMenu {
                menu: ControlId::new(2),
                field: ControlId::new(1),
            },
        ] {
            controller.handle_event(tab_id, event);
        }
        (Rc::new(RefCell::new(controller)), tab_id)
    }

    fn save(username: &str) {
        save_login(
            &current().expect("installed"),
            LoginKey::form(
                Origin::parse("https://example.com").expect("valid"),
                username,
            ),
            Password::new(format!("fake-{username}")),
        );
    }

    fn fills(controller: &Rc<RefCell<BrowserController>>) -> Vec<String> {
        std::iter::from_fn(|| controller.borrow_mut().next_command())
            .map(TabCommand::into_command)
            .filter_map(|command| match command {
                BrowserCommand::FillLogin(fill) => Some(fill.script()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_menu_offers_the_sites_logins_and_a_pick_fills_the_chosen_one() {
        install_fakes();
        save("bob");
        save("alice");
        let (controller, tab_id) = controller_with_password_menu("https://example.com/login");
        let field = controller
            .borrow()
            .active_state()
            .password_field_menu()
            .expect("a password menu");

        let menu = fill_menu(tab_id, field, "https://example.com/login").expect("offered");
        assert_eq!(menu.heading, "Fill a login for example.com");
        assert_eq!(menu.usernames, vec!["alice", "bob"]);
        // The engine's menu is answered first, as the menu closes.
        let menu_id = ControlId::new(2);
        assert!(
            controller
                .borrow_mut()
                .answer_control(menu_id, talaria_core::ControlAnswer::Dismiss)
        );

        fill_chosen(&controller, 1, || {});

        let scripts = fills(&controller);
        assert_eq!(scripts.len(), 1);
        assert!(scripts[0].ends_with(r#"("https://example.com", "bob", "fake-bob");"#));
    }

    #[test]
    fn nothing_is_offered_without_saved_logins_or_on_a_page_that_cannot_hold_them() {
        install_fakes();
        save("alice");

        assert_eq!(
            fill_menu(TabId::new(1), ControlId::new(1), "https://other.test/"),
            None
        );
        assert_eq!(
            fill_menu(TabId::new(1), ControlId::new(1), "http://example.com/"),
            None
        );
    }

    #[test]
    fn nothing_is_filled_once_the_field_lost_focus_or_the_page_moved() {
        install_fakes();
        save("alice");

        let (controller, tab_id) = controller_with_password_menu("https://example.com/login");
        fill_menu(tab_id, ControlId::new(1), "https://example.com/login").expect("offered");
        controller.borrow_mut().handle_event(
            tab_id,
            BrowserEvent::ControlHidden {
                id: ControlId::new(1),
            },
        );
        fill_chosen(&controller, 0, || {});
        assert!(fills(&controller).is_empty());

        let (controller, tab_id) = controller_with_password_menu("https://example.com/login");
        fill_menu(tab_id, ControlId::new(1), "https://example.com/login").expect("offered");
        controller.borrow_mut().handle_event(
            tab_id,
            BrowserEvent::AddressChanged {
                url: "https://evil.test/".to_owned(),
            },
        );
        fill_chosen(&controller, 0, || {});
        assert!(fills(&controller).is_empty());
    }

    #[test]
    fn an_offer_is_used_at_most_once() {
        install_fakes();
        save("alice");
        let (controller, tab_id) = controller_with_password_menu("https://example.com/login");
        fill_menu(tab_id, ControlId::new(1), "https://example.com/login").expect("offered");

        fill_chosen(&controller, 0, || {});
        fill_chosen(&controller, 0, || {});

        assert_eq!(fills(&controller).len(), 1);
    }
}

// passwords-ui (review): written before the fix.
#[cfg(test)]
mod worker_tests {
    use mercury_passwords::Password;
    use talaria_core::{BrowserEvent, ControlRect, ControlRequest, TabCommand};

    use super::*;
    use crate::passwords::tests::{install_deferred, install_fakes, save_login};

    const RECT: ControlRect = ControlRect {
        x: 0,
        y: 0,
        width: 10,
        height: 10,
    };

    fn controller(url: &str) -> (Rc<RefCell<BrowserController>>, TabId) {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        for event in [
            BrowserEvent::AddressChanged {
                url: url.to_owned(),
            },
            BrowserEvent::InputMethodShown {
                id: ControlId::new(1),
                rect: RECT,
            },
            BrowserEvent::ControlShown {
                id: ControlId::new(2),
                request: ControlRequest::ContextMenu {
                    rect: RECT,
                    entries: Vec::new(),
                    link_url: None,
                    image_url: None,
                },
            },
        ] {
            controller.handle_event(tab_id, event);
        }
        (Rc::new(RefCell::new(controller)), tab_id)
    }

    fn alice() -> LoginKey {
        LoginKey::form(
            Origin::parse("https://example.com").expect("valid"),
            "alice",
        )
    }

    fn fills(controller: &Rc<RefCell<BrowserController>>) -> usize {
        std::iter::from_fn(|| controller.borrow_mut().next_command())
            .map(TabCommand::into_command)
            .filter(|command| matches!(command, BrowserCommand::FillLogin(_)))
            .count()
    }

    #[test]
    fn a_field_that_lost_focus_while_the_keychain_answered_gets_nothing() {
        let (passwords, _, _) = install_deferred();
        save_login(&passwords, alice(), Password::new("fake".to_owned()));
        let (controller, tab_id) = controller("https://example.com/login");
        fill_menu(tab_id, ControlId::new(1), "https://example.com/login").expect("offered");
        let queued = Rc::new(std::cell::Cell::new(false));
        let seen = Rc::clone(&queued);

        fill_chosen(&controller, 0, move || seen.set(true));
        assert_eq!(fills(&controller), 0, "nothing before the keychain answers");
        controller.borrow_mut().handle_event(
            tab_id,
            BrowserEvent::ControlHidden {
                id: ControlId::new(1),
            },
        );
        passwords.worker.run_queued();

        assert_eq!(fills(&controller), 0);
        assert!(!queued.get());
    }

    #[test]
    fn a_menu_without_logins_clears_an_older_offer() {
        let (passwords, _, _) = install_fakes();
        save_login(&passwords, alice(), Password::new("fake".to_owned()));
        fill_menu(TabId::new(1), ControlId::new(1), "https://example.com/").expect("offered");

        assert!(fill_menu(TabId::new(1), ControlId::new(1), "about:blank").is_none());

        assert!(passwords.fill.borrow().is_none());
    }
}

// passwords-ui (review round 2): written before the fix.
#[cfg(test)]
mod locked_list_tests {
    use mercury_password_manager::Interaction;

    use super::*;
    use crate::passwords::tests::install_deferred;

    #[test]
    fn a_list_that_could_not_be_read_is_read_again_on_the_next_right_click() {
        let (passwords, _, _) = install_deferred();
        passwords.list_unavailable.set(true);

        assert!(fill_menu(TabId::new(1), ControlId::new(1), "https://example.com/").is_none());

        assert_eq!(passwords.worker.pending(), 1);
        assert_eq!(
            passwords.worker.last_interaction(),
            Some(Interaction::Allowed)
        );
        passwords.worker.run_queued();
        assert!(!passwords.list_unavailable.get());
    }
}
