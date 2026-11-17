// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use mercury_passwords::PasswordError;

/// What can go wrong in the password manager. Like [`PasswordError`], no variant carries a
/// password, a user name or a field from a file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManagerError {
    /// The store refused or failed.
    #[error(transparent)]
    Store(#[from] PasswordError),
    /// Another login for the same site already has this user name.
    #[error("a login with this user name is already saved for this site")]
    AlreadySaved,
    /// The login is no longer saved (another program may have removed it).
    #[error("the login is no longer saved")]
    NotFound,
    /// A file could not be read or written.
    #[error("{0}")]
    File(String),
}

impl ManagerError {
    /// A sentence for the user.
    pub fn message(&self) -> String {
        match self {
            Self::Store(PasswordError::Locked | PasswordError::AccessDenied) => {
                "Mercury Browser was not allowed to read the saved login.".to_owned()
            }
            Self::Store(PasswordError::InvalidInput(reason)) => {
                let mut text = reason.to_string();
                if let Some(first) = text.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                format!("{text}.")
            }
            Self::Store(PasswordError::InvalidOrigin(_)) => {
                "Logins can only be saved for HTTPS sites.".to_owned()
            }
            Self::Store(_) => "The password store could not be used.".to_owned(),
            Self::AlreadySaved => {
                "A login with this user name is already saved for this site.".to_owned()
            }
            Self::NotFound => "This login is no longer saved.".to_owned(),
            Self::File(reason) => reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_sentences_without_details() {
        assert_eq!(
            ManagerError::Store(PasswordError::InvalidInput("the password is empty")).message(),
            "The password is empty."
        );
        assert_eq!(
            ManagerError::Store(PasswordError::Locked).message(),
            "Mercury Browser was not allowed to read the saved login."
        );
        assert_eq!(
            ManagerError::Store(PasswordError::Os {
                code: -1,
                message: "x".to_owned()
            })
            .message(),
            "The password store could not be used."
        );
    }
}
