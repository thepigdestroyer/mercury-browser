// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Passwords section of the settings window, through its `PasswordManagerUi` global.
//!
//! The list never holds a password. One is read from the store only to show or copy it (after
//! the user authenticated), to move a login to a new user name, or to export; a shown password
//! is forgotten when the login is closed, another is opened, or the window closes.

use std::path::Path;
use std::rc::{Rc, Weak};

use mercury_password_manager::{
    LoginRow, display_text, export_csv, filter_rows, import_csv, login_rows, site_label,
    update_login, write_private_file,
};
use mercury_passwords::{
    GENERATOR_MAX_LENGTH, GENERATOR_MIN_LENGTH, GeneratorOptions, LoginKey, OsRandom, Password,
    generate_password,
};
use mercury_ui::{LoginListRow, PasswordManagerUi, SettingsWindow};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use zeroize::Zeroizing;

use super::{Passwords, current};

/// What the section shows, kept between the window's callbacks.
#[derive(Default)]
pub(crate) struct SectionState {
    window: slint::Weak<SettingsWindow>,
    /// The rows shown, after the search.
    rows: Vec<LoginRow>,
    /// The login opened.
    selected: Option<LoginKey>,
}

fn ui(window: &SettingsWindow) -> PasswordManagerUi<'_> {
    window.global::<PasswordManagerUi>()
}

fn window_of(passwords: &Passwords) -> Option<SettingsWindow> {
    passwords.section.borrow().window.upgrade()
}

/// Shows `text` under the section's toolbar, if the settings window is open.
pub(super) fn set_status(text: &str) {
    if let Some(window) = current().as_deref().and_then(window_of) {
        ui(&window).set_status(text.into());
    }
}

/// Connects a newly created settings window and fills the section. Called once per window.
pub(crate) fn connect_settings_window(window: &SettingsWindow) {
    let Some(passwords) = current() else {
        return;
    };
    *passwords.section.borrow_mut() = SectionState {
        window: window.as_weak(),
        ..SectionState::default()
    };
    let ui = ui(window);
    ui.set_storage_note(
        if passwords.store.backend().is_persistent() {
            "Saved in your login keychain."
        } else {
            "This session's logins are kept only until Mercury Browser quits."
        }
        .into(),
    );

    let weak = Rc::downgrade(&passwords);
    let on = |action: fn(&Rc<Passwords>)| {
        let weak: Weak<Passwords> = weak.clone();
        move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(passwords) = weak.upgrade() {
                action(&passwords);
            }
        }
    };
    ui.on_search_edited({
        let weak = weak.clone();
        move |_| {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(passwords) = weak.upgrade() {
                refresh(&passwords);
            }
        }
    });
    ui.on_open_login({
        let weak = weak.clone();
        move |index| {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let (Some(passwords), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                open(&passwords, index);
            }
        }
    });
    ui.on_close_login(on(close));
    ui.on_show_password(on(show_password));
    ui.on_hide_password(on(hide_password));
    ui.on_copy_username(on(copy_username));
    ui.on_copy_password(on(copy_password));
    ui.on_edit_login(on(edit));
    ui.on_generate_for_edit(on(generate_for_edit));
    ui.on_save_edit(on(save_edit));
    ui.on_cancel_edit(on(cancel_edit));
    ui.on_delete_login(on(|passwords| {
        if let Some(window) = window_of(passwords) {
            ui(&window).set_confirming_delete(true);
        }
    }));
    ui.on_confirm_delete(on(confirm_delete));
    ui.on_cancel_delete(on(|passwords| {
        if let Some(window) = window_of(passwords) {
            ui(&window).set_confirming_delete(false);
        }
    }));
    ui.on_generate(on(generate));
    ui.on_copy_generated(on(copy_generated));
    ui.on_import_logins(on(choose_import));
    ui.on_export_logins(on(choose_export));
    refresh(&passwords);
}

/// The settings window closed: the grace period ends and nothing it showed is kept.
pub(crate) fn settings_window_closed() {
    if let Some(passwords) = current() {
        passwords.gate.borrow_mut().close();
        *passwords.section.borrow_mut() = SectionState::default();
    }
}

/// Lists the logins again, keeping the opened one open if it is still saved and still matches.
fn refresh(passwords: &Rc<Passwords>) {
    let Some(window) = window_of(passwords) else {
        return;
    };
    let ui = ui(&window);
    let all = match passwords.store.all() {
        Ok(all) => all,
        Err(error) => {
            log::warn!("passwords: the saved logins could not be listed: {error}");
            ui.set_status("The saved logins could not be read.".into());
            Vec::new()
        }
    };
    let rows = login_rows(&all);
    let shown = filter_rows(&rows, &ui.get_search())
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    ui.set_total(i32::try_from(rows.len()).unwrap_or(i32::MAX));
    ui.set_rows(ModelRc::new(VecModel::from(
        shown
            .iter()
            .map(|row| LoginListRow {
                site: row.site.clone().into(),
                username: row.username.clone().into(),
                kind: row.kind.clone().into(),
            })
            .collect::<Vec<_>>(),
    )));
    let mut state = passwords.section.borrow_mut();
    let selected = state
        .selected
        .as_ref()
        .and_then(|key| shown.iter().position(|row| &row.key == key));
    state.rows = shown;
    if selected.is_none() {
        state.selected = None;
        drop(state);
        show_closed(&window);
    } else {
        ui.set_selected(selected.map_or(-1, |index| i32::try_from(index).unwrap_or(-1)));
    }
}

fn show_closed(window: &SettingsWindow) {
    let ui = ui(window);
    ui.set_selected(-1);
    forget_shown_password(window);
    ui.set_editing(false);
    ui.set_edit_password(SharedString::new());
    ui.set_edit_problem(SharedString::new());
    ui.set_confirming_delete(false);
}

fn forget_shown_password(window: &SettingsWindow) {
    let ui = ui(window);
    ui.set_password_shown(false);
    ui.set_shown_password(SharedString::new());
}

fn selected(passwords: &Passwords) -> Option<LoginKey> {
    passwords.section.borrow().selected.clone()
}

fn open(passwords: &Rc<Passwords>, index: usize) {
    let Some(window) = window_of(passwords) else {
        return;
    };
    let Some(row) = passwords.section.borrow().rows.get(index).cloned() else {
        return;
    };
    show_closed(&window);
    passwords.section.borrow_mut().selected = Some(row.key.clone());
    let ui = ui(&window);
    ui.set_selected(i32::try_from(index).unwrap_or(-1));
    ui.set_detail_site(row.site.into());
    ui.set_detail_username(row.username.into());
    ui.set_detail_kind(row.kind.into());
    ui.set_status(SharedString::new());
}

fn close(passwords: &Rc<Passwords>) {
    passwords.section.borrow_mut().selected = None;
    if let Some(window) = window_of(passwords) {
        show_closed(&window);
    }
}

/// Reads the opened login's password after authentication and hands it to `then`.
fn with_password(passwords: &Rc<Passwords>, reason: &str, then: fn(&Rc<Passwords>, Password)) {
    let Some(key) = selected(passwords) else {
        return;
    };
    let weak = Rc::downgrade(passwords);
    passwords.after_authentication(reason, move |allowed| {
        let Some(passwords) = weak.upgrade().filter(|_| allowed) else {
            return;
        };
        // The user may have opened another login while the system asked.
        if selected(&passwords).as_ref() != Some(&key) {
            return;
        }
        match passwords.store.password(&key) {
            Ok(Some(password)) => then(&passwords, password),
            Ok(None) => set_status("This login is no longer saved."),
            Err(error) => {
                log::warn!("passwords: a saved password could not be read: {error}");
                set_status("Mercury Browser was not allowed to read the saved password.");
            }
        }
    });
}

fn show_password(passwords: &Rc<Passwords>) {
    with_password(passwords, "show a saved password", |passwords, password| {
        if let Some(window) = window_of(passwords) {
            let ui = ui(&window);
            ui.set_shown_password(display_text(password.expose()).into());
            ui.set_password_shown(true);
        }
    });
}

fn hide_password(passwords: &Rc<Passwords>) {
    if let Some(window) = window_of(passwords) {
        forget_shown_password(&window);
    }
}

fn copy_password(passwords: &Rc<Passwords>) {
    with_password(passwords, "copy a saved password", |passwords, password| {
        set_status(if passwords.copy_secret(password.expose()) {
            "Copied the password. It is removed from the clipboard in a minute."
        } else {
            "The password could not be copied."
        });
    });
}

fn copy_username(passwords: &Rc<Passwords>) {
    let Some(key) = selected(passwords) else {
        return;
    };
    let copied = passwords.pasteboard.write_text(&key.username, false).is_some();
    set_status(if copied {
        "Copied the user name."
    } else {
        "The user name could not be copied."
    });
}

fn edit(passwords: &Rc<Passwords>) {
    let (Some(window), Some(key)) = (window_of(passwords), selected(passwords)) else {
        return;
    };
    forget_shown_password(&window);
    let ui = ui(&window);
    ui.set_edit_username(key.username.into());
    ui.set_edit_password(SharedString::new());
    ui.set_edit_problem(SharedString::new());
    ui.set_editing(true);
}

fn cancel_edit(passwords: &Rc<Passwords>) {
    if let Some(window) = window_of(passwords) {
        let ui = ui(&window);
        ui.set_editing(false);
        ui.set_edit_password(SharedString::new());
        ui.set_edit_problem(SharedString::new());
    }
}

fn generated_password(length: i32, symbols: bool) -> Option<Password> {
    let length = usize::try_from(length)
        .unwrap_or(GENERATOR_MIN_LENGTH)
        .clamp(GENERATOR_MIN_LENGTH, GENERATOR_MAX_LENGTH);
    let options = GeneratorOptions {
        length,
        symbols,
        ..GeneratorOptions::default()
    };
    generate_password(&options, &mut OsRandom)
        .inspect_err(|error| log::warn!("passwords: no password generated: {error}"))
        .ok()
}

fn generate_for_edit(passwords: &Rc<Passwords>) {
    let Some(window) = window_of(passwords) else {
        return;
    };
    if let Some(password) = generated_password(20, true) {
        ui(&window).set_edit_password(password.expose().into());
    }
}

fn save_edit(passwords: &Rc<Passwords>) {
    let (Some(window), Some(key)) = (window_of(passwords), selected(passwords)) else {
        return;
    };
    let ui = ui(&window);
    let typed = ui.get_edit_password();
    let password = (!typed.is_empty()).then(|| Password::new(typed.as_str().to_owned()));
    match update_login(&passwords.store, &key, &ui.get_edit_username(), password) {
        Ok(new_key) => {
            passwords.section.borrow_mut().selected = Some(new_key.clone());
            ui.set_editing(false);
            ui.set_edit_password(SharedString::new());
            ui.set_edit_problem(SharedString::new());
            ui.set_detail_username(
                if new_key.username.is_empty() {
                    "No user name".to_owned()
                } else {
                    display_text(&new_key.username)
                }
                .into(),
            );
            refresh(passwords);
            ui.set_status("Saved the login.".into());
        }
        Err(error) => ui.set_edit_problem(error.message().into()),
    }
}

fn confirm_delete(passwords: &Rc<Passwords>) {
    let (Some(window), Some(key)) = (window_of(passwords), selected(passwords)) else {
        return;
    };
    let status = match passwords.store.remove(&key) {
        Ok(_) => format!("Deleted the login for {}.", site_label(&key.origin)),
        Err(error) => {
            log::warn!("passwords: a login could not be deleted: {error}");
            "The login could not be deleted.".to_owned()
        }
    };
    passwords.section.borrow_mut().selected = None;
    refresh(passwords);
    ui(&window).set_status(status.into());
}

fn generate(passwords: &Rc<Passwords>) {
    let Some(window) = window_of(passwords) else {
        return;
    };
    let ui = ui(&window);
    if let Some(password) =
        generated_password(ui.get_generator_length(), ui.get_generator_symbols())
    {
        ui.set_generated(password.expose().into());
    }
}

fn copy_generated(passwords: &Rc<Passwords>) {
    let Some(window) = window_of(passwords) else {
        return;
    };
    let generated = ui(&window).get_generated();
    if !generated.is_empty() && passwords.copy_secret(&generated) {
        set_status("Copied the generated password. It is removed from the clipboard in a minute.");
    }
}

fn choose_import(passwords: &Rc<Passwords>) {
    let Some(window) = window_of(passwords) else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        let weak = Rc::downgrade(passwords);
        super::file_dialogs::choose_import_file(
            window.window(),
            Box::new(move |path| {
                // After AppKit's callback, outside it.
                slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                    let _slint = crate::callback_scope::InSlintCallback::enter();
                    if let (Some(passwords), Some(path)) = (weak.upgrade(), path) {
                        import_file(&passwords, &path);
                    }
                });
            }),
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        set_status("Importing is not available on this platform yet.");
    }
}

/// Imports the logins of a CSV file and says what happened.
pub(super) fn import_file(passwords: &Rc<Passwords>, path: &Path) {
    let status = match std::fs::read(path) {
        Ok(bytes) => {
            let bytes = Zeroizing::new(bytes);
            match std::str::from_utf8(&bytes) {
                Ok(text) => match import_csv(&passwords.store, text) {
                    Ok(report) => report.message(),
                    Err(error) => format!(
                        "This file is not a login export from Chrome, Edge, Brave, Firefox or \
                         Safari ({error})."
                    ),
                },
                Err(_) => "This file is not a text file.".to_owned(),
            }
        }
        Err(error) => format!("The file could not be read: {error}."),
    };
    refresh(passwords);
    set_status(&status);
}

fn choose_export(passwords: &Rc<Passwords>) {
    let weak = Rc::downgrade(passwords);
    passwords.after_authentication("export your saved passwords", move |allowed| {
        let Some(passwords) = weak.upgrade().filter(|_| allowed) else {
            return;
        };
        let Some(window) = window_of(&passwords) else {
            return;
        };
        #[cfg(target_os = "macos")]
        {
            let weak = Rc::downgrade(&passwords);
            super::file_dialogs::choose_export_file(
                window.window(),
                Box::new(move |path| {
                    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                        let _slint = crate::callback_scope::InSlintCallback::enter();
                        if let (Some(passwords), Some(path)) = (weak.upgrade(), path) {
                            export_file(&passwords, &path);
                        }
                    });
                }),
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window;
            set_status("Exporting is not available on this platform yet.");
        }
    });
}

/// Writes every login to `path` (mode `0600`) and says what happened.
pub(super) fn export_file(passwords: &Rc<Passwords>, path: &Path) {
    let status = export_csv(&passwords.store)
        .and_then(|text| {
            write_private_file(path, text.as_bytes())?;
            Ok(text.lines().count().saturating_sub(1))
        })
        .map_or_else(
            |error| error.message(),
            |count| {
                format!(
                    "Exported {count} logins to {}. The file holds every password in clear \
                     text: delete it once you no longer need it.",
                    path.file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
                )
            },
        );
    set_status(&status);
}

#[cfg(test)]
mod tests {
    use i_slint_backend_testing::{AccessibleRole, ElementHandle};
    use mercury_password_manager::AuthOutcome;
    use mercury_passwords::Origin;
    use slint::Model;

    use super::*;
    use crate::passwords::tests::install_fakes;
    use crate::settings::tests::init_slint;

    fn key(origin: &str, username: &str) -> LoginKey {
        LoginKey::form(Origin::parse(origin).expect("valid"), username)
    }

    fn save(passwords: &Passwords, origin: &str, username: &str, password: &str) {
        passwords
            .store
            .save(&key(origin, username), &Password::new(password.to_owned()))
            .expect("saved");
    }

    fn section() -> (
        Rc<Passwords>,
        Rc<mercury_password_manager::testing::FakeAuthenticator>,
        Rc<mercury_password_manager::testing::FakePasteboard>,
        SettingsWindow,
    ) {
        init_slint();
        let (passwords, authenticator, pasteboard) = install_fakes();
        save(&passwords, "https://a.test", "alice", "fake-a");
        save(&passwords, "https://b.test", "bob", "fake-b");
        let window = SettingsWindow::new().expect("a window");
        window.set_section(4);
        connect_settings_window(&window);
        (passwords, authenticator, pasteboard, window)
    }

    fn element(window: &SettingsWindow, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(window, label)
            .find(|element| element.accessible_role() != Some(AccessibleRole::Text))
            .unwrap_or_else(|| panic!("no control labelled {label:?}"))
    }

    #[test]
    fn the_list_shows_every_login_and_the_search_narrows_it() {
        let (_passwords, _, _, window) = section();
        let ui = ui(&window);
        assert_eq!(ui.get_total(), 2);
        assert_eq!(ui.get_rows().row_count(), 2);

        element(&window, "Search saved logins").set_accessible_value("BOB");

        let rows = ui.get_rows();
        assert_eq!(rows.row_count(), 1);
        assert_eq!(rows.row_data(0).expect("a row").site, "b.test");
    }

    #[test]
    fn a_login_opens_from_the_keyboard_path_and_shows_no_password() {
        let (_passwords, authenticator, _, window) = section();

        element(&window, "a.test, alice").invoke_accessible_default_action();

        let ui = ui(&window);
        assert_eq!(ui.get_selected(), 0);
        assert_eq!(ui.get_detail_username(), "alice");
        assert!(!ui.get_password_shown());
        assert_eq!(ui.get_shown_password(), "");
        assert_eq!(authenticator.asked(), 0);
    }

    #[test]
    fn a_password_is_shown_only_after_the_user_authenticates() {
        let (_passwords, authenticator, _, window) = section();
        element(&window, "a.test, alice").invoke_accessible_default_action();

        element(&window, "Show the password").invoke_accessible_default_action();
        let ui = ui(&window);
        assert_eq!(
            authenticator.waiting_reasons(),
            vec!["show a saved password"]
        );
        assert!(!ui.get_password_shown(), "nothing before the answer");

        assert!(authenticator.answer(AuthOutcome::Allowed));
        assert!(ui.get_password_shown());
        assert_eq!(ui.get_shown_password(), "fake-a");

        // Within the grace period, no second prompt; closing forgets the password.
        element(&window, "Hide the password").invoke_accessible_default_action();
        element(&window, "Show the password").invoke_accessible_default_action();
        assert_eq!(authenticator.asked(), 1);
        element(&window, "a.test, alice").invoke_accessible_default_action();
        assert_eq!(ui.get_shown_password(), "");
    }

    #[test]
    fn a_refused_authentication_shows_and_copies_nothing() {
        let (_passwords, authenticator, pasteboard, window) = section();
        element(&window, "b.test, bob").invoke_accessible_default_action();

        element(&window, "Copy the password").invoke_accessible_default_action();
        authenticator.answer(AuthOutcome::Denied);
        element(&window, "Show the password").invoke_accessible_default_action();
        authenticator.answer(AuthOutcome::Unavailable);

        let ui = ui(&window);
        assert!(!ui.get_password_shown());
        assert_eq!(pasteboard.contents(), None);
        assert!(ui.get_status().contains("cannot confirm"));
    }

    #[test]
    fn copying_the_password_conceals_it_and_the_user_name_does_not_need_authentication() {
        let (_passwords, authenticator, pasteboard, window) = section();
        element(&window, "b.test, bob").invoke_accessible_default_action();

        element(&window, "Copy the user name").invoke_accessible_default_action();
        assert_eq!(pasteboard.contents(), Some(("bob".to_owned(), false)));
        assert_eq!(authenticator.asked(), 0);

        element(&window, "Copy the password").invoke_accessible_default_action();
        authenticator.answer(AuthOutcome::Allowed);
        assert_eq!(pasteboard.contents(), Some(("fake-b".to_owned(), true)));
    }

    #[test]
    fn editing_moves_the_login_and_a_taken_user_name_is_refused() {
        let (passwords, _, _, window) = section();
        save(&passwords, "https://a.test", "carol", "fake-c");
        refresh(&passwords);
        element(&window, "a.test, alice").invoke_accessible_default_action();

        element(&window, "Edit the login").invoke_accessible_default_action();
        element(&window, "New user name").set_accessible_value("carol");
        element(&window, "Save the login").invoke_accessible_default_action();
        let ui = ui(&window);
        assert_eq!(
            ui.get_edit_problem(),
            "A login with this user name is already saved for this site."
        );

        element(&window, "New user name").set_accessible_value("alice2");
        element(&window, "New password").set_accessible_value("fake-new");
        element(&window, "Save the login").invoke_accessible_default_action();

        assert!(!ui.get_editing());
        assert_eq!(ui.get_detail_username(), "alice2");
        assert_eq!(
            passwords
                .store
                .password(&key("https://a.test", "alice2"))
                .expect("readable")
                .expect("saved")
                .expose(),
            "fake-new"
        );
        assert!(
            passwords
                .store
                .password(&key("https://a.test", "alice"))
                .expect("readable")
                .is_none()
        );
    }

    #[test]
    fn deleting_asks_first() {
        let (passwords, _, _, window) = section();
        element(&window, "a.test, alice").invoke_accessible_default_action();

        element(&window, "Delete the login").invoke_accessible_default_action();
        assert_eq!(passwords.store.all().expect("listed").len(), 2);
        element(&window, "Delete the login now").invoke_accessible_default_action();

        assert_eq!(passwords.store.all().expect("listed").len(), 1);
        let ui = ui(&window);
        assert_eq!(ui.get_selected(), -1);
        assert_eq!(ui.get_status(), "Deleted the login for a.test.");
    }

    #[test]
    fn the_generator_honours_length_and_symbols() {
        let (_passwords, _, _, window) = section();
        let ui = ui(&window);
        ui.set_generator_length(32);
        ui.set_generator_symbols(false);

        element(&window, "Generate a password").invoke_accessible_default_action();

        let generated = ui.get_generated();
        assert_eq!(generated.chars().count(), 32);
        assert!(generated.chars().all(char::is_alphanumeric));
    }

    #[test]
    fn import_and_export_go_through_files_the_user_chose() {
        let (passwords, _, _, window) = section();
        let directory = tempfile::tempdir().expect("a directory");
        let import = directory.path().join("in.csv");
        std::fs::write(
            &import,
            "name,url,username,password\nc.test,https://c.test/,carol,fake-c\n",
        )
        .expect("a file");

        import_file(&passwords, &import);
        assert_eq!(ui(&window).get_status(), "Imported 1 login.");
        assert_eq!(ui(&window).get_total(), 3);

        let export = directory.path().join("out.csv");
        export_file(&passwords, &export);
        let written = std::fs::read_to_string(&export).expect("written");
        assert!(written.contains("c.test,https://c.test,carol,fake-c"));
        assert!(ui(&window).get_status().starts_with("Exported 3 logins to out.csv."));
    }

    #[test]
    fn closing_the_window_ends_the_grace_period() {
        let (passwords, authenticator, _, window) = section();
        element(&window, "a.test, alice").invoke_accessible_default_action();
        element(&window, "Show the password").invoke_accessible_default_action();
        authenticator.answer(AuthOutcome::Allowed);

        settings_window_closed();

        assert!(!passwords.gate.borrow().is_open(std::time::Instant::now()));
    }
}
