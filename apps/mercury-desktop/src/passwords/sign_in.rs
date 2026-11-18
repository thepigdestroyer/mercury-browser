// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! HTTP sign-in through the dialogs lane's sign-in sheet: the sheet starts filled with a login
//! saved for the exact origin that asked, and with "save" chosen the credentials are saved once
//! the sign-in has worked (see `SignInSaves`). Servo 0.8 reports no realm, so these logins are
//! saved and looked up with an empty one; Mercury never answers a sign-in without the sheet.

use std::rc::Rc;
use std::time::Instant;

use mercury_passwords::{Origin, Password};
use talaria_core::{
    BrowserEvent, HttpCredentials, LoadState, PageRequestId, PageRequestKind, TabId,
};

use super::{Passwords, current};

/// The sheet's choice when saving is offered.
pub(crate) const SAVE_SIGN_IN_CHOICE: &str = "Save the user name and password in Mercury Browser";

/// The credentials found for the sign-in request on screen, so the store is read once per
/// request rather than on every redraw.
pub(crate) struct CachedCredentials {
    id: PageRequestId,
    credentials: Option<HttpCredentials>,
}

/// The saved login to fill the sheet of request `id` with: for `origin` (serialised,
/// `https://example.com`) and `realm` (`None`: Servo 0.8 does not report it), the most recently
/// changed one.
pub(crate) fn saved_http_credentials(
    id: PageRequestId,
    origin: &str,
    realm: Option<&str>,
) -> Option<HttpCredentials> {
    let passwords = current()?;
    if let Some(cached) = passwords.http_fill.borrow().as_ref()
        && cached.id == id
    {
        return cached.credentials.clone();
    }
    let credentials = look_up(&passwords, origin, realm.unwrap_or_default());
    *passwords.http_fill.borrow_mut() = Some(CachedCredentials {
        id,
        credentials: credentials.clone(),
    });
    credentials
}

fn look_up(passwords: &Passwords, origin: &str, realm: &str) -> Option<HttpCredentials> {
    let origin = Origin::parse(origin).ok()?;
    let mut logins = passwords
        .store
        .http_auth_logins_for(&origin, realm)
        .inspect_err(|error| log::warn!("passwords: sign-in logins could not be listed: {error}"))
        .ok()?;
    logins.sort_by_key(|summary| std::cmp::Reverse(summary.modified_at_ms));
    let login = logins.first()?;
    match passwords.store.password(&login.key) {
        Ok(Some(password)) => Some(HttpCredentials::new(
            login.key.username.clone(),
            password.expose().to_owned(),
        )),
        Ok(None) => None,
        Err(error) => {
            log::warn!("passwords: a sign-in password could not be read: {error}");
            None
        }
    }
}

/// Whether the sheet for a sign-in to `origin` offers to save: not for a proxy, only for an
/// origin that can hold logins, and as the settings allow.
pub(crate) fn offers_to_save_sign_in(origin: Option<&str>, for_proxy: bool) -> bool {
    let Some(origin) = origin.filter(|_| !for_proxy) else {
        return false;
    };
    current().is_some()
        && Origin::parse(origin).is_ok()
        && crate::settings::password_saving_allowed(origin)
}

/// The user signed in from the sheet with "save" chosen.
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
    if let Some(saved) = saved {
        // The settings may have changed while the sign-in was under way.
        if !crate::settings::password_saving_allowed(saved.key.origin.as_str()) {
            return;
        }
        match passwords.store.save(&saved.key, &saved.password) {
            Ok(outcome) => log::info!(
                "passwords: saved an HTTP sign-in for {} ({outcome:?})",
                saved.key.origin
            ),
            Err(error) => log::warn!("passwords: an HTTP sign-in could not be saved: {error}"),
        }
        *passwords.http_fill.borrow_mut() = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mercury_passwords::LoginKey;
    use talaria_core::PageRequest;

    use super::*;
    use crate::passwords::tests::install_fakes;

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
    fn a_sign_in_that_worked_is_saved_and_fills_the_next_sheet() {
        crate::settings::tests::init_slint();
        let _settings = crate::settings::SettingsHandle::start(None);
        let (passwords, _, _) = install_fakes();
        let tab = TabId::new(1);
        assert_eq!(
            saved_http_credentials(PageRequestId::new(1), ORIGIN, None),
            None
        );

        sign_in_sent(tab, ORIGIN, "alice", "fake-pw");
        observe_event(&passwords, tab, &BrowserEvent::PageLoadStarted);
        observe_event(&passwords, tab, &loaded());

        let saved =
            saved_http_credentials(PageRequestId::new(2), ORIGIN, None).expect("saved and found");
        assert_eq!(saved.username(), "alice");
        assert_eq!(saved.password(), "fake-pw");
        assert!(
            passwords
                .store
                .password(&LoginKey::http_auth(
                    Origin::parse(ORIGIN).expect("valid"),
                    "",
                    "alice"
                ))
                .expect("readable")
                .is_some()
        );
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

        assert!(passwords.store.all().expect("listed").is_empty());
    }

    #[test]
    fn never_save_and_the_offer_setting_are_honoured() {
        crate::settings::tests::init_slint();
        let settings = crate::settings::SettingsHandle::start(None);
        install_fakes();

        assert!(offers_to_save_sign_in(Some(ORIGIN), false));
        assert!(
            !offers_to_save_sign_in(Some(ORIGIN), true),
            "not for a proxy"
        );
        assert!(!offers_to_save_sign_in(Some("http://example.com"), false));
        assert!(!offers_to_save_sign_in(None, false));

        settings.update(|settings| {
            settings
                .passwords
                .never_save_origins
                .push(ORIGIN.to_owned());
        });
        assert!(!offers_to_save_sign_in(Some(ORIGIN), false));

        settings.update(|settings| {
            settings.passwords.never_save_origins.clear();
            settings.passwords.offer_to_save = false;
        });
        assert!(!offers_to_save_sign_in(Some(ORIGIN), false));
    }

    #[test]
    fn the_store_is_read_once_per_request() {
        let (passwords, _, _) = install_fakes();
        passwords
            .store
            .save(
                &LoginKey::http_auth(Origin::parse(ORIGIN).expect("valid"), "", "bob"),
                &Password::new("fake-b".to_owned()),
            )
            .expect("saved");
        let id = PageRequestId::new(7);
        assert!(saved_http_credentials(id, ORIGIN, None).is_some());

        passwords.store.remove_all().expect("removed");

        assert!(
            saved_http_credentials(id, ORIGIN, None).is_some(),
            "cached for the request on screen"
        );
        assert!(saved_http_credentials(PageRequestId::new(8), ORIGIN, None).is_none());
    }
}
