// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use mercury_passwords::{Password, PasswordStore, SecretBackend, write_logins_csv};
use zeroize::Zeroizing;

use crate::ManagerError;

/// Every saved login in Chrome's CSV format (see `write_logins_csv`), passwords in clear text.
/// Reads every password, so on macOS the Keychain may ask the user about each one.
pub fn export_csv<B: SecretBackend>(
    store: &PasswordStore<B>,
) -> Result<Zeroizing<String>, ManagerError> {
    let summaries = store.all()?;
    let mut logins = Vec::with_capacity(summaries.len());
    for summary in summaries {
        // A login removed since it was listed is left out rather than failing the export.
        if let Some(password) = store.password(&summary.key)? {
            logins.push((summary.key, password));
        }
    }
    Ok(write_logins_csv(
        logins
            .iter()
            .map(|(key, password): &(_, Password)| (key, password)),
    ))
}

/// Writes `contents` to `path` as a file only its owner can read and write (mode `0600`).
///
/// The text goes to a new file beside `path` first, created with that mode, and is then renamed
/// over `path`; so a file the user chose to replace is replaced whole, a symbolic link at `path`
/// is replaced rather than followed, and no partly written file is left behind.
pub fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), ManagerError> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| ManagerError::File("Choose a file in a folder.".to_owned()))?;
    let name = path
        .file_name()
        .ok_or_else(|| ManagerError::File("Choose a file name.".to_owned()))?;
    let mut staged_name = std::ffi::OsString::from(".");
    staged_name.push(name);
    staged_name.push(format!(".mercury-{}.partial", std::process::id()));
    let staged = directory.join(staged_name);

    let result = write_new(&staged, contents).and_then(|()| {
        fs::rename(&staged, path)
            .map_err(|error| ManagerError::File(format!("The file could not be saved: {error}.")))
    });
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

fn write_new(path: &PathBuf, contents: &[u8]) -> Result<(), ManagerError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| ManagerError::File(format!("The file could not be created: {error}.")))?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|error| ManagerError::File(format!("The file could not be written: {error}.")))
}

#[cfg(test)]
mod tests {
    use mercury_passwords::{LoginKey, MemoryBackend, Origin};

    use super::*;

    #[test]
    fn the_export_holds_every_login_in_chromes_format() {
        let store = PasswordStore::new(MemoryBackend::new());
        let origin = Origin::parse("https://example.com").expect("valid");
        store
            .save(
                &LoginKey::form(origin.clone(), "alice"),
                &Password::new("fake-a".to_owned()),
            )
            .expect("saved");
        store
            .save(
                &LoginKey::http_auth(origin, "", "bob"),
                &Password::new("fake,b".to_owned()),
            )
            .expect("saved");

        let text = export_csv(&store).expect("exported");

        assert_eq!(
            text.as_str(),
            "name,url,username,password,note\n\
             example.com,https://example.com,alice,fake-a,\n\
             example.com,https://example.com,bob,\"fake,b\",\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_and_replaces_what_was_there() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("a directory");
        let path = directory.path().join("logins.csv");
        fs::write(&path, "old").expect("an old file");

        write_private_file(&path, b"new").expect("written");

        assert_eq!(fs::read_to_string(&path).expect("readable"), "new");
        let mode = fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            fs::read_dir(directory.path()).expect("listed").count(),
            1,
            "no staged file is left behind"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_replaced_not_followed() {
        let directory = tempfile::tempdir().expect("a directory");
        let target = directory.path().join("elsewhere.txt");
        fs::write(&target, "keep").expect("a target");
        let link = directory.path().join("logins.csv");
        std::os::unix::fs::symlink(&target, &link).expect("a link");

        write_private_file(&link, b"secret").expect("written");

        assert_eq!(fs::read_to_string(&target).expect("readable"), "keep");
        assert!(!fs::symlink_metadata(&link).expect("metadata").is_symlink());
    }

    #[test]
    fn a_missing_folder_fails_and_leaves_nothing() {
        let directory = tempfile::tempdir().expect("a directory");
        let path = directory.path().join("missing").join("logins.csv");

        assert!(write_private_file(&path, b"x").is_err());
        assert_eq!(fs::read_dir(directory.path()).expect("listed").count(), 0);
    }
}
