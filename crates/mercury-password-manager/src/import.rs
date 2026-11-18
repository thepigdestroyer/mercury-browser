// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::collections::HashSet;

use mercury_passwords::{
    CsvSkipped, LoginKey, Origin, PasswordStore, SecretBackend, parse_logins_csv,
};

use crate::ManagerError;

/// What an import did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Logins saved.
    pub added: usize,
    /// Logins already saved (or listed twice in the file); the saved one was kept.
    pub already_saved: usize,
    /// Logins the store refused or could not write.
    pub failed: usize,
    /// Rows that were not logins, by line, with a reason that never quotes the file.
    pub skipped: Vec<CsvSkipped>,
}

impl ImportReport {
    /// A summary for the user.
    pub fn message(&self) -> String {
        let mut parts = vec![match self.added {
            1 => "Imported 1 login.".to_owned(),
            count => format!("Imported {count} logins."),
        }];
        if self.already_saved > 0 {
            parts.push(format!(
                "{} already saved, kept as they were.",
                self.already_saved
            ));
        }
        if self.failed > 0 {
            parts.push(format!("{} could not be saved.", self.failed));
        }
        if !self.skipped.is_empty() {
            let lines = self
                .skipped
                .iter()
                .take(5)
                .map(|skipped| skipped.line.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let more = if self.skipped.len() > 5 { ", …" } else { "" };
            parts.push(format!(
                "{} rows were skipped (lines {lines}{more}).",
                self.skipped.len()
            ));
        }
        parts.join(" ")
    }
}

/// Saves the logins of a CSV file exported by Chrome, Edge, Brave, Firefox or Safari.
///
/// A login that is already saved keeps its saved password: an import never overwrites, and it
/// never reads a saved password (which on macOS could make the Keychain ask the user). The caller
/// should wipe `text` afterwards; it holds passwords in clear text.
pub fn import_csv<B: SecretBackend>(
    store: &PasswordStore<B>,
    text: &str,
) -> Result<ImportReport, ManagerError> {
    let parsed = parse_logins_csv(text)?;
    let mut known = store
        .all()?
        .into_iter()
        .map(|summary| summary.key)
        .collect::<HashSet<_>>();
    let mut report = ImportReport {
        skipped: parsed.skipped,
        ..ImportReport::default()
    };
    for login in parsed.logins {
        let Ok(origin) = Origin::parse(&login.url) else {
            report.failed += 1;
            continue;
        };
        let key = match login.http_realm {
            Some(realm) => LoginKey::http_auth(origin, realm, login.username),
            None => LoginKey::form(origin, login.username),
        };
        if known.contains(&key) {
            report.already_saved += 1;
            continue;
        }
        match store.save(&key, &login.password) {
            Ok(_) => {
                report.added += 1;
                known.insert(key);
            }
            Err(_) => report.failed += 1,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use mercury_passwords::{MemoryBackend, Password};

    use super::*;

    fn key(origin: &str, username: &str) -> LoginKey {
        LoginKey::form(Origin::parse(origin).expect("valid"), username)
    }

    #[test]
    fn new_logins_are_saved_and_saved_ones_kept() {
        let store = PasswordStore::new(MemoryBackend::new());
        store
            .save(
                &key("https://a.test", "alice"),
                &Password::new("fake-kept".to_owned()),
            )
            .expect("saved");
        let csv = "name,url,username,password\n\
                   a.test,https://a.test/login,alice,fake-other\n\
                   b.test,https://b.test/,bob,fake-b\n\
                   b.test,https://b.test/,bob,fake-b-again\n\
                   c.test,http://c.test/,carol,fake-c\n";

        let report = import_csv(&store, csv).expect("imported");

        assert_eq!(report.added, 1);
        assert_eq!(report.already_saved, 2);
        assert_eq!(report.failed, 0);
        assert_eq!(
            report.skipped.len(),
            1,
            "plain http is not a site for logins"
        );
        assert_eq!(report.skipped[0].line, 5);
        assert_eq!(
            store
                .password(&key("https://a.test", "alice"))
                .expect("readable")
                .expect("saved")
                .expose(),
            "fake-kept"
        );
        assert_eq!(
            store
                .password(&key("https://b.test", "bob"))
                .expect("readable")
                .expect("saved")
                .expose(),
            "fake-b"
        );
        assert_eq!(
            report.message(),
            "Imported 1 login. 2 already saved, kept as they were. 1 rows were skipped (lines 5)."
        );
    }

    #[test]
    fn firefox_realms_become_http_sign_ins() {
        let store = PasswordStore::new(MemoryBackend::new());
        let csv = "url,username,password,httpRealm,formActionOrigin,guid,timeCreated,timeLastUsed,timePasswordChanged\n\
                   https://intranet.test,dave,fake-d,Staff,,{1},1,1,1\n";

        let report = import_csv(&store, csv).expect("imported");

        assert_eq!(report.added, 1);
        let saved = store.all().expect("listed");
        assert_eq!(
            saved[0].key,
            LoginKey::http_auth(
                Origin::parse("https://intranet.test").expect("valid"),
                "Staff",
                "dave"
            )
        );
    }

    #[test]
    fn a_file_that_is_not_a_login_export_is_an_error() {
        let store = PasswordStore::new(MemoryBackend::new());

        assert!(import_csv(&store, "a,b,c\n1,2,3\n").is_err());
    }
}
