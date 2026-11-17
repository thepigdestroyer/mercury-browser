// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use mercury_passwords::{LoginSummary, Origin, PasswordStore, SecretBackend};

/// The form logins that may be offered for filling into the top-level page at `page_url`: those
/// saved for exactly its origin. A page that cannot hold logins (plain HTTP other than loopback,
/// `file:`, `data:`, `about:`) gets none, and so does a store that cannot be listed. Offering is
/// all this does: a login is filled only after the user picks it in the browser's own menu.
pub fn fill_candidates<B: SecretBackend>(
    store: &PasswordStore<B>,
    page_url: &str,
) -> (Option<Origin>, Vec<LoginSummary>) {
    let Ok(origin) = Origin::parse(page_url) else {
        return (None, Vec::new());
    };
    let logins = store.logins_for(&origin).unwrap_or_default();
    (Some(origin), logins)
}

#[cfg(test)]
mod tests {
    use mercury_passwords::{LoginKey, MemoryBackend, Password};

    use super::*;

    fn store() -> PasswordStore<MemoryBackend> {
        let store = PasswordStore::new(MemoryBackend::new());
        for (origin, username) in [
            ("https://example.com", "bob"),
            ("https://example.com", "alice"),
            ("https://www.example.com", "carol"),
            ("https://example.com:8443", "dave"),
        ] {
            store
                .save(
                    &LoginKey::form(Origin::parse(origin).expect("valid"), username),
                    &Password::new("fake".to_owned()),
                )
                .expect("saved");
        }
        store
            .save(
                &LoginKey::http_auth(
                    Origin::parse("https://example.com").expect("valid"),
                    "",
                    "erin",
                ),
                &Password::new("fake".to_owned()),
            )
            .expect("saved");
        store
    }

    fn usernames(found: &[LoginSummary]) -> Vec<&str> {
        found
            .iter()
            .map(|summary| summary.key.username.as_str())
            .collect()
    }

    #[test]
    fn only_form_logins_of_exactly_the_pages_origin_are_offered() {
        let (origin, found) = fill_candidates(&store(), "https://example.com/login?next=/");

        assert_eq!(
            origin.map(|origin| origin.as_str().to_owned()).as_deref(),
            Some("https://example.com")
        );
        assert_eq!(usernames(&found), vec!["alice", "bob"]);
    }

    #[test]
    fn pages_that_cannot_hold_logins_are_offered_nothing() {
        for url in [
            "http://example.com/",
            "file:///Users/me/login.html",
            "about:blank",
            "data:text/html,<input type=password>",
            "",
        ] {
            let (origin, found) = fill_candidates(&store(), url);
            assert!(origin.is_none() && found.is_empty(), "{url}");
        }
    }
}
