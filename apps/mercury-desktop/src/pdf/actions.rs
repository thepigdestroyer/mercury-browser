// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The two ways to get the file out of the viewer: "Save a copy" and "Open in Preview".
//!
//! "Save a copy" is an ordinary download of the document's address (the downloads list shows it,
//! the file is quarantined like every download). "Open in Preview" hands the file to the system's
//! PDF app: a file the user already has is opened where it is; a private cache copy (which the
//! viewer deletes when the tab moves on) is first copied, under its own name, into a private
//! folder of the temporary directory. Neither runs in an automated run beyond logging what it
//! would do, and a page cannot trigger either: they are buttons.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use talaria_core::TabId;
use url::Url;

/// Copies of private files opened in another app stay this long.
const OPEN_COPY_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// The longest file name given to a copy handed to another app, in characters.
const MAX_FILE_NAME_CHARS: usize = 100;

/// What the viewer's buttons do outside the window.
pub(super) trait ExternalActions {
    /// Starts a download of the document's address for the tab. The error is for the user.
    fn save_copy(&self, tab: TabId, url: &Url) -> Result<(), String>;
    /// Opens the document in the system's PDF app. `name` is what to call a copy of a private
    /// file; the error is for the user.
    fn open_externally(&self, path: &Path, owned: bool, name: &str) -> Result<(), String>;
}

/// Actions shared by several owners (tests keep a handle to read them back).
impl<T: ExternalActions + ?Sized> ExternalActions for std::rc::Rc<T> {
    fn save_copy(&self, tab: TabId, url: &Url) -> Result<(), String> {
        (**self).save_copy(tab, url)
    }

    fn open_externally(&self, path: &Path, owned: bool, name: &str) -> Result<(), String> {
        (**self).open_externally(path, owned, name)
    }
}

/// The real thing.
pub(super) struct SystemActions {
    /// A `--screenshot` or `--background` run: nothing is downloaded to the user's folders or
    /// opened in another app.
    pub(super) automated: bool,
}

impl ExternalActions for SystemActions {
    fn save_copy(&self, tab: TabId, url: &Url) -> Result<(), String> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err("This PDF is already a file on this Mac.".to_owned());
        }
        crate::downloads::save_url(Some(tab), url.as_str(), Some(url.as_str()), None)
            .map_err(|error| error.to_string())
    }

    fn open_externally(&self, path: &Path, owned: bool, name: &str) -> Result<(), String> {
        if self.automated {
            log::info!("automated run: would open a PDF in the system's PDF app");
            return Ok(());
        }
        let target = if owned {
            copy_for_other_app(path, name)?
        } else {
            path.to_path_buf()
        };
        let program = if cfg!(target_os = "macos") {
            "/usr/bin/open"
        } else {
            "xdg-open"
        };
        let mut command = Command::new(program);
        command
            .arg(&target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| format!("Mercury could not start the PDF app: {error}"))?;
        // Reaped off the UI thread; the app itself keeps running without us.
        std::thread::spawn(move || {
            let _status = child.wait();
        });
        Ok(())
    }
}

/// Copies a private file to `<temporary directory>/mercury-browser-pdf-open/<random>/<name>`,
/// readable by the user only, and returns the copy's path.
fn copy_for_other_app(path: &Path, name: &str) -> Result<PathBuf, String> {
    let folder = open_copies_folder().join(random_name());
    create_private_folder(&folder)
        .map_err(|error| format!("Mercury could not prepare a copy of the PDF: {error}"))?;
    let target = folder.join(copy_file_name(name));
    std::fs::copy(path, &target)
        .map_err(|error| format!("Mercury could not copy the PDF: {error}"))?;
    Ok(target)
}

fn open_copies_folder() -> PathBuf {
    std::env::temp_dir().join("mercury-browser-pdf-open")
}

fn random_name() -> String {
    let mut bytes = [0_u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // No randomness: the process id and the clock still make a name nobody else is using.
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        bytes[..4].copy_from_slice(&std::process::id().to_le_bytes());
        bytes[4..].copy_from_slice(&nanos.to_le_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(unix)]
fn create_private_folder(folder: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)
}

#[cfg(not(unix))]
fn create_private_folder(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)
}

/// A name that is safe on every file system and ends in `.pdf`: path separators, colons and
/// control characters become spaces, the length is bounded, a leading dot is dropped.
pub(super) fn copy_file_name(name: &str) -> String {
    let mut cleaned = String::new();
    for character in name.chars() {
        let character = if character.is_control() || matches!(character, '/' | '\\' | ':') {
            ' '
        } else {
            character
        };
        cleaned.push(character);
    }
    let cleaned = cleaned.trim_end();
    // The last four bytes are ASCII when they are `.pdf`, so the cut is on a character boundary.
    let bytes = cleaned.as_bytes();
    let without_suffix =
        if bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".pdf") {
            &cleaned[..cleaned.len() - 4]
        } else {
            cleaned
        };
    let stem = without_suffix
        .trim_start_matches(|character: char| character == '.' || character.is_whitespace())
        .trim_end();
    let stem = if stem.is_empty() { "document" } else { stem };
    let stem = stem.chars().take(MAX_FILE_NAME_CHARS).collect::<String>();
    format!("{stem}.pdf")
}

/// Removes copies handed to other apps more than a day ago; called at start-up.
pub(super) fn sweep_old_copies() {
    sweep_folder(&open_copies_folder(), OPEN_COPY_MAX_AGE, SystemTime::now());
}

fn sweep_folder(folder: &Path, max_age: Duration, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| now.duration_since(modified).is_ok_and(|age| age >= max_age));
        if old {
            let _removed = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
pub(super) mod fake {
    use std::cell::RefCell;

    use super::*;

    /// Records what was asked and does nothing: no download, no other app.
    #[derive(Default)]
    pub(in crate::pdf) struct FakeActions {
        pub(in crate::pdf) saved: RefCell<Vec<(TabId, String)>>,
        pub(in crate::pdf) opened: RefCell<Vec<(PathBuf, bool, String)>>,
    }

    impl ExternalActions for FakeActions {
        fn save_copy(&self, tab: TabId, url: &Url) -> Result<(), String> {
            self.saved.borrow_mut().push((tab, url.to_string()));
            Ok(())
        }

        fn open_externally(&self, path: &Path, owned: bool, name: &str) -> Result<(), String> {
            self.opened
                .borrow_mut()
                .push((path.to_path_buf(), owned, name.to_owned()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_gets_a_safe_name_ending_in_pdf() {
        assert_eq!(copy_file_name("Annual report"), "Annual report.pdf");
        assert_eq!(copy_file_name("Annual report.pdf"), "Annual report.pdf");
        assert_eq!(copy_file_name("Annual report.PDF"), "Annual report.pdf");
        assert_eq!(copy_file_name("../../etc/passwd"), "etc passwd.pdf");
        assert_eq!(copy_file_name("a:b\\c\nd"), "a b c d.pdf");
        assert_eq!(copy_file_name("..hidden"), "hidden.pdf");
        assert_eq!(copy_file_name("   "), "document.pdf");
        assert_eq!(copy_file_name(".pdf"), "document.pdf");
        let long = "x".repeat(500);
        assert_eq!(
            copy_file_name(&long).chars().count(),
            MAX_FILE_NAME_CHARS + 4
        );
    }

    #[test]
    fn a_private_file_is_copied_under_its_own_name_and_readable_only_by_the_user() {
        let source = tempfile::tempdir().expect("a folder");
        let original = source.path().join("cache-1234.pdf");
        std::fs::write(&original, b"%PDF-1.4 test").expect("a file");

        let copy = copy_for_other_app(&original, "Quarterly report").expect("a copy");
        let parent = copy.parent().expect("a folder").to_path_buf();
        assert_eq!(
            copy.file_name().and_then(|name| name.to_str()),
            Some("Quarterly report.pdf")
        );
        assert_eq!(std::fs::read(&copy).expect("the copy"), b"%PDF-1.4 test");
        assert!(original.exists(), "the private file is left to its owner");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&parent)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "the folder is private: {mode:o}");
        }
        let _removed = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn old_copies_are_swept_and_new_ones_stay() {
        let folder = tempfile::tempdir().expect("a folder");
        let old = folder.path().join("old");
        let new = folder.path().join("new");
        std::fs::create_dir(&old).expect("a folder");
        std::fs::create_dir(&new).expect("a folder");
        std::fs::write(old.join("a.pdf"), b"x").expect("a file");
        std::fs::write(new.join("b.pdf"), b"x").expect("a file");

        // Two days from now, everything is old; right now, nothing is.
        sweep_folder(folder.path(), OPEN_COPY_MAX_AGE, SystemTime::now());
        assert!(old.exists() && new.exists());
        let later = SystemTime::now() + Duration::from_secs(2 * 24 * 60 * 60);
        sweep_folder(folder.path(), OPEN_COPY_MAX_AGE, later);
        assert!(!old.exists() && !new.exists());
    }

    #[test]
    fn an_automated_run_opens_nothing() {
        let actions = SystemActions { automated: true };
        assert_eq!(
            actions.open_externally(Path::new("/nonexistent/file.pdf"), false, "x"),
            Ok(())
        );
    }

    #[test]
    fn only_web_documents_can_be_saved_as_a_download() {
        let actions = SystemActions { automated: true };
        let result = actions.save_copy(
            TabId::new(1),
            &Url::parse("file:///Users/someone/a.pdf").expect("a URL"),
        );
        assert!(result.is_err());
    }
}
