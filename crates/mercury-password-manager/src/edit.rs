// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use mercury_passwords::{LoginKey, Password, PasswordStore, SecretBackend};

use crate::ManagerError;

/// Changes a saved login's user name, password or both, and returns its key afterwards.
///
/// `password` is `None` to keep the saved one. A new user name moves the login: it is saved
/// under the new key first and the old one is removed only then, so a failure never loses it.
/// A new user name that another login of the same site and kind already has is refused.
pub fn update_login<B: SecretBackend>(
    store: &PasswordStore<B>,
    key: &LoginKey,
    username: &str,
    password: Option<Password>,
) -> Result<LoginKey, ManagerError> {
    let new_key = LoginKey {
        username: username.to_owned(),
        ..key.clone()
    };
    if new_key == *key {
        if let Some(password) = password {
            if store.password(key)?.is_none() {
                return Err(ManagerError::NotFound);
            }
            store.save(key, &password)?;
        }
        return Ok(new_key);
    }

    let saved = store.all()?;
    if saved.iter().any(|summary| summary.key == new_key) {
        return Err(ManagerError::AlreadySaved);
    }
    let password = match password {
        Some(password) => password,
        None => store.password(key)?.ok_or(ManagerError::NotFound)?,
    };
    store.save(&new_key, &password)?;
    store.remove(key)?;
    Ok(new_key)
}

#[cfg(test)]
mod tests {
    use mercury_passwords::{MemoryBackend, Origin};

    use super::*;

    fn store_with(keys: &[(&str, &str)]) -> PasswordStore<MemoryBackend> {
        let store = PasswordStore::new(MemoryBackend::new());
        for (username, password) in keys {
            store
                .save(&key(username), &Password::new((*password).to_owned()))
                .expect("saved");
        }
        store
    }

    fn key(username: &str) -> LoginKey {
        LoginKey::form(
            Origin::parse("https://example.com").expect("valid"),
            username,
        )
    }

    fn password_of(store: &PasswordStore<MemoryBackend>, username: &str) -> Option<String> {
        store
            .password(&key(username))
            .expect("readable")
            .map(|password| password.expose().to_owned())
    }

    #[test]
    fn a_new_password_replaces_the_old_one() {
        let store = store_with(&[("alice", "fake-old")]);

        let key = update_login(
            &store,
            &key("alice"),
            "alice",
            Some(Password::new("fake-new".to_owned())),
        )
        .expect("updated");

        assert_eq!(key.username, "alice");
        assert_eq!(password_of(&store, "alice").as_deref(), Some("fake-new"));
    }

    #[test]
    fn a_new_user_name_moves_the_login_with_its_password() {
        let store = store_with(&[("alice", "fake-pw")]);

        let moved = update_login(&store, &key("alice"), "alice2", None).expect("moved");

        assert_eq!(moved, key("alice2"));
        assert_eq!(password_of(&store, "alice2").as_deref(), Some("fake-pw"));
        assert_eq!(password_of(&store, "alice"), None);
    }

    #[test]
    fn a_user_name_another_login_has_is_refused() {
        let store = store_with(&[("alice", "fake-a"), ("bob", "fake-b")]);

        let result = update_login(&store, &key("alice"), "bob", None);

        assert!(matches!(result, Err(ManagerError::AlreadySaved)));
        assert_eq!(password_of(&store, "alice").as_deref(), Some("fake-a"));
        assert_eq!(password_of(&store, "bob").as_deref(), Some("fake-b"));
    }

    #[test]
    fn an_empty_password_is_refused_and_nothing_changes() {
        let store = store_with(&[("alice", "fake-pw")]);

        let result = update_login(
            &store,
            &key("alice"),
            "alice",
            Some(Password::new(String::new())),
        );

        assert!(matches!(result, Err(ManagerError::Store(_))));
        assert_eq!(password_of(&store, "alice").as_deref(), Some("fake-pw"));
    }

    #[test]
    fn a_login_removed_meanwhile_is_not_recreated() {
        let store = store_with(&[]);

        let result = update_login(
            &store,
            &key("alice"),
            "alice",
            Some(Password::new("fake".to_owned())),
        );

        assert!(matches!(result, Err(ManagerError::NotFound)));
        assert!(store.all().expect("listed").is_empty());
    }
}
