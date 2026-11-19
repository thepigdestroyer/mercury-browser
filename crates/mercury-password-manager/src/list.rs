// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use mercury_passwords::{LoginKey, LoginKind, LoginSummary};

use crate::display::{display_text, site_label};

/// One saved login as the list shows it. Only `key` identifies it; the rest is display text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginRow {
    /// Which login this is.
    pub key: LoginKey,
    /// The site (see [`site_label`]).
    pub site: String,
    /// The user name, safe to show; "No user name" when it is empty.
    pub username: String,
    /// "HTTP sign-in", with the realm when there is one; empty for a form login.
    pub kind: String,
}

/// The rows for the given logins, in the store's order (by site, then user name).
pub fn login_rows(summaries: &[LoginSummary]) -> Vec<LoginRow> {
    summaries
        .iter()
        .map(|summary| {
            let key = &summary.key;
            LoginRow {
                key: key.clone(),
                site: site_label(&key.origin),
                username: if key.username.is_empty() {
                    "No user name".to_owned()
                } else {
                    display_text(&key.username)
                },
                kind: match &key.kind {
                    LoginKind::Form => String::new(),
                    LoginKind::HttpAuth { realm } if realm.is_empty() => "HTTP sign-in".to_owned(),
                    LoginKind::HttpAuth { realm } => {
                        format!("HTTP sign-in – {}", display_text(realm))
                    }
                },
            }
        })
        .collect()
}

/// The rows whose site or user name contains every word of `query`, ignoring case. An empty
/// query keeps every row.
pub fn filter_rows<'a>(rows: &'a [LoginRow], query: &str) -> Vec<&'a LoginRow> {
    let words = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    rows.iter()
        .filter(|row| {
            let site = row.key.origin.as_str().to_lowercase();
            let username = row.key.username.to_lowercase();
            words
                .iter()
                .all(|word| site.contains(word.as_str()) || username.contains(word.as_str()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use mercury_passwords::Origin;

    use super::*;

    fn summary(key: LoginKey) -> LoginSummary {
        LoginSummary {
            key,
            created_at_ms: None,
            modified_at_ms: None,
        }
    }

    fn origin(text: &str) -> Origin {
        Origin::parse(text).expect("a valid origin")
    }

    fn sample() -> Vec<LoginRow> {
        login_rows(&[
            summary(LoginKey::form(origin("https://mail.example"), "Alice")),
            summary(LoginKey::form(origin("https://shop.test"), "")),
            summary(LoginKey::http_auth(
                origin("http://127.0.0.1:8123"),
                "",
                "bob",
            )),
            summary(LoginKey::http_auth(
                origin("https://intranet.test"),
                "Staff\narea",
                "carol",
            )),
        ])
    }

    #[test]
    fn rows_name_the_site_the_user_and_the_kind() {
        let rows = sample();

        assert_eq!(rows[0].site, "mail.example");
        assert_eq!(rows[0].username, "Alice");
        assert_eq!(rows[0].kind, "");
        assert_eq!(rows[1].username, "No user name");
        assert_eq!(rows[2].site, "http://127.0.0.1:8123");
        assert_eq!(rows[2].kind, "HTTP sign-in");
        assert_eq!(rows[3].kind, "HTTP sign-in – Staff\u{fffd}area");
    }

    #[test]
    fn search_matches_site_or_user_name_ignoring_case() {
        let rows = sample();
        let sites = |query: &str| {
            filter_rows(&rows, query)
                .into_iter()
                .map(|row| row.site.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(sites("").len(), 4);
        assert_eq!(sites("ALICE"), vec!["mail.example"]);
        assert_eq!(sites("shop"), vec!["shop.test"]);
        assert_eq!(sites("127.0.0.1 bob"), vec!["http://127.0.0.1:8123"]);
        assert_eq!(sites("mail bob"), Vec::<String>::new());
    }
}
