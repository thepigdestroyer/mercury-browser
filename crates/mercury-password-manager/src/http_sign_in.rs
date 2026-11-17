// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::time::{Duration, Instant};

use mercury_passwords::{LoginKey, Origin, Password};

/// A sign-in not confirmed within this time is forgotten, never saved.
pub const HTTP_SAVE_TIMEOUT: Duration = Duration::from_secs(120);

/// An HTTP sign-in that worked and that the user asked to save.
pub struct PendingHttpSave {
    /// The login: the sign-in's origin, an empty realm (Servo 0.8 does not report it), the user
    /// name typed.
    pub key: LoginKey,
    /// The password typed.
    pub password: Password,
}

struct Entry {
    tab: u64,
    key: LoginKey,
    password: Password,
    sent_at: Instant,
    document_started: bool,
}

/// HTTP sign-ins sent with "save" chosen, waiting to learn whether they worked.
///
/// Servo does not report success. A sign-in counts as having worked when the tab then starts a
/// new document and finishes loading it without asking for the same origin's credentials again:
/// with wrong credentials Servo fetches again, receives another 401 and asks again first. One
/// sign-in per tab is tracked; a newer one replaces it.
#[derive(Default)]
pub struct SignInSaves {
    entries: Vec<Entry>,
}

impl SignInSaves {
    /// Nothing waiting.
    pub fn new() -> Self {
        Self::default()
    }

    /// The user signed in from the sheet with "save" chosen. A URL whose origin cannot hold
    /// logins is ignored.
    pub fn sent(&mut self, tab: u64, url: &str, username: &str, password: Password, now: Instant) {
        self.forget_tab(tab);
        let Ok(origin) = Origin::parse(url) else {
            return;
        };
        self.entries.push(Entry {
            tab,
            key: LoginKey::http_auth(origin, "", username),
            password,
            sent_at: now,
            document_started: false,
        });
    }

    /// The tab asks for a sign-in again: for the same origin, the credentials were rejected.
    pub fn sign_in_requested(&mut self, tab: u64, url: &str) {
        let origin = Origin::parse(url).ok();
        self.entries
            .retain(|entry| entry.tab != tab || Some(&entry.key.origin) != origin.as_ref());
    }

    /// The tab started a new document.
    pub fn document_started(&mut self, tab: u64) {
        for entry in self.entries.iter_mut().filter(|entry| entry.tab == tab) {
            entry.document_started = true;
        }
    }

    /// The tab finished loading: a sign-in that led to a new document worked.
    pub fn load_finished(&mut self, tab: u64, now: Instant) -> Option<PendingHttpSave> {
        self.expire(now);
        let index = self
            .entries
            .iter()
            .position(|entry| entry.tab == tab && entry.document_started)?;
        let entry = self.entries.remove(index);
        Some(PendingHttpSave {
            key: entry.key,
            password: entry.password,
        })
    }

    /// The tab's load failed, it crashed or it closed: nothing is saved for it.
    pub fn forget_tab(&mut self, tab: u64) {
        self.entries.retain(|entry| entry.tab != tab);
    }

    /// Whether anything is waiting (for tests and logging).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn expire(&mut self, now: Instant) {
        self.entries
            .retain(|entry| now.saturating_duration_since(entry.sent_at) < HTTP_SAVE_TIMEOUT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:8123/private";

    fn fake() -> Password {
        Password::new("fake-pw".to_owned())
    }

    #[test]
    fn a_sign_in_that_loads_the_next_document_is_saved_with_an_empty_realm() {
        let now = Instant::now();
        let mut saves = SignInSaves::new();
        saves.sent(1, URL, "alice", fake(), now);

        assert!(saves.load_finished(1, now).is_none(), "no new document yet");
        saves.document_started(1);
        let saved = saves.load_finished(1, now).expect("worked");

        assert_eq!(
            saved.key,
            LoginKey::http_auth(Origin::parse(URL).expect("valid"), "", "alice")
        );
        assert_eq!(saved.password.expose(), "fake-pw");
        assert!(saves.is_empty());
    }

    #[test]
    fn a_second_request_for_the_same_origin_means_the_credentials_were_rejected() {
        let now = Instant::now();
        let mut saves = SignInSaves::new();
        saves.sent(1, URL, "alice", fake(), now);

        saves.sign_in_requested(1, "http://127.0.0.1:8123/private");
        saves.document_started(1);

        assert!(saves.load_finished(1, now).is_none());
    }

    #[test]
    fn another_origins_sign_in_or_another_tab_changes_nothing() {
        let now = Instant::now();
        let mut saves = SignInSaves::new();
        saves.sent(1, URL, "alice", fake(), now);

        saves.sign_in_requested(1, "https://other.test/");
        saves.sign_in_requested(2, URL);
        saves.document_started(2);
        assert!(saves.load_finished(2, now).is_none());
        saves.document_started(1);

        assert!(saves.load_finished(1, now).is_some());
    }

    #[test]
    fn failures_closes_and_old_sign_ins_save_nothing() {
        let now = Instant::now();
        let mut saves = SignInSaves::new();
        saves.sent(1, URL, "alice", fake(), now);
        saves.forget_tab(1);
        saves.document_started(1);
        assert!(saves.load_finished(1, now).is_none());

        saves.sent(1, URL, "alice", fake(), now);
        saves.document_started(1);
        assert!(saves.load_finished(1, now + HTTP_SAVE_TIMEOUT).is_none());
    }

    #[test]
    fn sites_that_cannot_hold_logins_are_never_tracked() {
        let mut saves = SignInSaves::new();

        saves.sent(1, "http://example.com/", "alice", fake(), Instant::now());

        assert!(saves.is_empty());
    }
}
